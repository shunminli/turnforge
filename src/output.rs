//! CLI-only output ownership. No blocking stdout worker or detached task.
use std::{
    fs::File,
    io::{self, Write},
    os::{
        fd::{AsRawFd, RawFd},
        unix::fs::{FileTypeExt, MetadataExt},
    },
    time::Duration,
};

use nix::fcntl::{FcntlArg, OFlag, fcntl};
use tokio::{io::unix::AsyncFd, sync::mpsc::Receiver};
use turnforge::{CancellationToken, DebugAction, Event, message::Message, model::ModelDelta};

use crate::learning::{self, Lesson};

pub enum OutputItem {
    Event(Event),
    Notice(String),
}

pub enum DisplayMode {
    Json,
    Text,
    Lab {
        lesson: Option<Lesson>,
        automatic: bool,
    },
}

pub const LAB_HELP: &str =
    "操作：Enter/n 下一步 | c 继续 | i 完整现场 | p 暂停 | q 取消 | l 学习路线 | h 帮助\n";

pub async fn forward(
    mut events: Receiver<OutputItem>,
    mode: DisplayMode,
    cancel: &CancellationToken,
) -> io::Result<()> {
    let mut output = Output::stdout()?;
    while let Some(item) = events.recv().await {
        let bytes = encode(item, &mode)?;
        if bytes.is_empty() {
            continue;
        }
        let write = output.write_all(&bytes);
        tokio::pin!(write);
        // A slow sink gets a bounded opportunity to drain. After cancellation
        // we still try to deliver terminal events, but never wait indefinitely.
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => tokio::time::timeout(Duration::from_millis(100), &mut write).await,
            result = tokio::time::timeout(Duration::from_secs(2), &mut write) => result,
        };
        match result {
            Ok(result) => result?,
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "stdout stalled; output may be incomplete",
                ));
            }
        }
    }
    Ok(())
}

fn encode(item: OutputItem, mode: &DisplayMode) -> io::Result<Vec<u8>> {
    let event = match item {
        OutputItem::Notice(text) => return Ok(text.into_bytes()),
        OutputItem::Event(event) => event,
    };
    Ok(match mode {
        DisplayMode::Lab { lesson, automatic } => {
            lab_text(event, *lesson, *automatic)?.into_bytes()
        }
        DisplayMode::Json => {
            let mut bytes = serde_json::to_vec(&event).map_err(io::Error::other)?;
            bytes.push(b'\n');
            bytes
        }
        DisplayMode::Text => match event {
            Event::ModelDelta {
                delta: ModelDelta::Text { text },
            } => text.into_bytes(),
            Event::RunFinished { .. } => vec![b'\n'],
            Event::DebugPaused { snapshot } => {
                let id = snapshot.pause_id;
                format!(
                        "\n[debug paused #{id}]\n{}\nCommands: step {id} | continue {id} | inspect {id} | pause | cancel\n",
                        serde_json::to_string_pretty(&snapshot).map_err(io::Error::other)?
                    )
                    .into_bytes()
            }
            Event::DebugSnapshot { snapshot } => {
                let id = snapshot.pause_id;
                format!(
                        "\n[debug snapshot #{id}]\n{}\nCommands: step {id} | continue {id} | inspect {id} | pause | cancel\n",
                        serde_json::to_string_pretty(&snapshot).map_err(io::Error::other)?
                    )
                    .into_bytes()
            }
            Event::DebugResumed { pause_id } => {
                format!("\n[debug resumed #{pause_id}]\n").into_bytes()
            }
            Event::DebugCommandRejected { command, reason } => format!(
                "\n[debug rejected] {}: {reason}\n",
                serde_json::to_string(&command).map_err(io::Error::other)?
            )
            .into_bytes(),
            _ => Vec::new(),
        },
    })
}

fn preview(value: &impl serde::Serialize) -> io::Result<String> {
    let text = serde_json::to_string(value).map_err(io::Error::other)?;
    let mut chars = text.chars();
    let mut short: String = chars.by_ref().take(1200).collect();
    if chars.next().is_some() {
        short.push_str(" …（预览截断，输入 i 查看完整现场）");
    }
    Ok(short)
}

fn lab_text(event: Event, lesson: Option<Lesson>, automatic: bool) -> io::Result<String> {
    Ok(match event {
        Event::StepStarted { step } => format!("\n[model step {step}] 请求本地模型…\n"),
        Event::ModelDelta {
            delta: ModelDelta::Text { text },
        } => text,
        Event::ToolStarted { call } => {
            format!("\n[tool] {} {}\n", call.name, preview(&call.arguments)?)
        }
        Event::MessageCommitted {
            message: Message::Tool { call_id, output },
        } => {
            format!("\n[result] {call_id}: {}\n", preview(&output)?)
        }
        Event::DebugPaused { snapshot } => {
            let next = match &snapshot.next {
                DebugAction::Model { step } => format!("请求模型（第 {step} 轮）"),
                DebugAction::Tool { call, .. } => {
                    format!("执行工具 {} {}", call.name, preview(&call.arguments)?)
                }
                DebugAction::Finish { outcome } => format!("结束本轮：{outcome:?}"),
            };
            let mut text = format!(
                "\n[lab paused #{}]\n位置：{:?}，模型轮次 {}\n下一步：{next}\n已提交 {} 条消息；待执行 {} 个工具\n",
                snapshot.pause_id,
                snapshot.point,
                snapshot.step,
                snapshot.messages.len(),
                snapshot.pending_calls.len()
            );
            if let Some(lesson) = lesson {
                text.push_str(&learning::hint(lesson, &snapshot));
                text.push('\n');
            }
            text.push_str(if automatic {
                "[auto] 自动单步推进\n"
            } else {
                LAB_HELP
            });
            text
        }
        Event::DebugSnapshot { snapshot } => format!(
            "\n[lab snapshot #{}]\n{}\n{LAB_HELP}",
            snapshot.pause_id,
            serde_json::to_string_pretty(&snapshot).map_err(io::Error::other)?
        ),
        Event::DebugResumed { pause_id } => format!("[lab resumed #{pause_id}]\n"),
        Event::DebugCommandRejected { reason, .. } => {
            format!("[lab rejected] {reason}；请等待当前暂停提示后操作。\n")
        }
        Event::RunFinished { outcome } => format!("\n[run] {outcome:?}\n"),
        _ => String::new(),
    })
}

enum Output {
    Pollable(AsyncFd<Descriptor>),
    File(File),
}

impl Output {
    fn stdout() -> io::Result<Self> {
        let file = File::from(nix::unistd::dup(io::stdout()).map_err(io::Error::from)?);
        // epoll/kqueue do not portably support regular files. Local file output
        // uses ordinary writes; network/special filesystem stalls are out of scope.
        let metadata = file.metadata()?;
        let dev_null = metadata.file_type().is_char_device()
            && metadata.rdev() == std::fs::metadata("/dev/null")?.rdev();
        if metadata.is_file() || dev_null {
            return Ok(Self::File(file));
        }
        let flags =
            OFlag::from_bits_truncate(fcntl(&file, FcntlArg::F_GETFL).map_err(io::Error::from)?);
        fcntl(&file, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK)).map_err(io::Error::from)?;
        Ok(Self::Pollable(AsyncFd::new(Descriptor { file, flags })?))
    }

    async fn write_all(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        match self {
            Self::File(file) => file.write_all(bytes),
            Self::Pollable(fd) => {
                while !bytes.is_empty() {
                    let mut ready = fd.writable().await?;
                    match ready.try_io(|fd| {
                        nix::unistd::write(&fd.get_ref().file, bytes).map_err(io::Error::from)
                    }) {
                        Ok(Ok(0)) => return Err(io::ErrorKind::WriteZero.into()),
                        Ok(Ok(count)) => bytes = &bytes[count..],
                        Ok(Err(error)) => return Err(error),
                        Err(_) => continue,
                    }
                }
                Ok(())
            }
        }
    }
}

struct Descriptor {
    file: File,
    flags: OFlag,
}
impl AsRawFd for Descriptor {
    fn as_raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }
}
impl Drop for Descriptor {
    fn drop(&mut self) {
        // dup shares the open-file description; restore the host's original flags.
        let _ = fcntl(&self.file, FcntlArg::F_SETFL(self.flags));
    }
}
