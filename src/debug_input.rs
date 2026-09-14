//! CLI-only command input. The host owns and stops this future with the run.
use std::{
    fs::File,
    io::{self, IsTerminal},
    os::{
        fd::{AsRawFd, RawFd},
        unix::fs::{FileTypeExt, MetadataExt},
    },
    sync::atomic::{AtomicU64, Ordering},
};

use nix::fcntl::{FcntlArg, OFlag, fcntl};
use tokio::io::unix::AsyncFd;
use turnforge::DebugCommand;

const MAX_LINE_BYTES: usize = 4096;

pub enum ControlCommand {
    Debug(DebugCommand),
    Help,
    Learn,
    NotPaused,
}

pub struct DebugInput {
    source: Source,
    buffer: [u8; 1024],
    consumed: usize,
    filled: usize,
    line: Vec<u8>,
    buffer_pause: u64,
    line_pause: Option<u64>,
}

impl DebugInput {
    pub fn stdin() -> io::Result<Self> {
        let file = File::from(nix::unistd::dup(io::stdin()).map_err(io::Error::from)?);
        let metadata = file.metadata()?;
        let kind = metadata.file_type();
        let dev_null =
            kind.is_char_device() && metadata.rdev() == std::fs::metadata("/dev/null")?.rdev();
        let source = if dev_null {
            Source::Eof
        } else {
            // Regular files are not portably pollable. Do not hide blocking
            // input in a worker that would outlive a completed or cancelled run.
            if !kind.is_fifo() && !kind.is_socket() && !file.is_terminal() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "debug stdin must be a terminal, pipe or socket; use a pipe instead of regular-file redirection",
                ));
            }
            let flags = OFlag::from_bits_truncate(
                fcntl(&file, FcntlArg::F_GETFL).map_err(io::Error::from)?,
            );
            fcntl(&file, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK)).map_err(io::Error::from)?;
            Source::Pollable(AsyncFd::new(Descriptor { file, flags })?)
        };
        Ok(Self {
            source,
            buffer: [0; 1024],
            consumed: 0,
            filled: 0,
            line: Vec::new(),
            buffer_pause: 0,
            line_pause: None,
        })
    }

    pub async fn next_command(
        &mut self,
        lab_pause: Option<&AtomicU64>,
    ) -> io::Result<Option<ControlCommand>> {
        loop {
            while self.consumed < self.filled {
                let byte = self.buffer[self.consumed];
                self.consumed += 1;
                // Pin every line to the pause observed when its first byte was
                // read. Buffered shortcuts must not acquire a later pause ID.
                self.line_pause.get_or_insert(self.buffer_pause);
                if byte == b'\n' {
                    return self.parse_line(lab_pause.is_some()).map(Some);
                }
                if self.line.len() == MAX_LINE_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "debug command exceeds 4 KiB",
                    ));
                }
                self.line.push(byte);
            }
            self.filled = self.source.read(&mut self.buffer).await?;
            self.buffer_pause = lab_pause.map_or(0, |id| id.load(Ordering::Acquire));
            self.consumed = 0;
            if self.filled == 0 {
                return if self.line.is_empty() {
                    Ok(None)
                } else {
                    self.parse_line(lab_pause.is_some()).map(Some)
                };
            }
        }
    }

    fn parse_line(&mut self, lab: bool) -> io::Result<ControlCommand> {
        let line = std::str::from_utf8(&self.line).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "debug command must be UTF-8")
        })?;
        let line = line.trim();
        let command = if lab {
            match line {
                "" | "n" | "next" | "step" => {
                    self.at_pause(|pause_id| DebugCommand::Step { pause_id })
                }
                "c" | "continue" => self.at_pause(|pause_id| DebugCommand::Continue { pause_id }),
                "i" | "inspect" => self.at_pause(|pause_id| DebugCommand::Inspect { pause_id }),
                "p" => ControlCommand::Debug(DebugCommand::Pause {}),
                "q" => ControlCommand::Debug(DebugCommand::Cancel {}),
                "h" | "help" | "?" => ControlCommand::Help,
                "l" | "learn" => ControlCommand::Learn,
                _ => ControlCommand::Debug(parse_debug(line)?),
            }
        } else {
            ControlCommand::Debug(parse_debug(line)?)
        };
        self.line.clear();
        self.line_pause = None;
        Ok(command)
    }

    fn at_pause(&self, command: impl FnOnce(u64) -> DebugCommand) -> ControlCommand {
        match self.line_pause {
            Some(id) if id != 0 => ControlCommand::Debug(command(id)),
            _ => ControlCommand::NotPaused,
        }
    }
}

fn parse_debug(line: &str) -> io::Result<DebugCommand> {
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid debug command: expected step ID, continue ID, inspect ID, pause, cancel, or a JSON command",
        )
    };
    let command = if line.starts_with('{') {
        // DebugCommand itself owns the strict, deny-unknown-fields schema.
        serde_json::from_str(line).map_err(|_| invalid())?
    } else {
        let mut parts = line.split_whitespace();
        let command = parts.next().ok_or_else(invalid)?;
        match command {
            "pause" if parts.next().is_none() => DebugCommand::Pause {},
            "cancel" if parts.next().is_none() => DebugCommand::Cancel {},
            "step" | "continue" | "inspect" => {
                let id = parts.next().ok_or_else(invalid)?;
                if !id.bytes().all(|byte| byte.is_ascii_digit()) || parts.next().is_some() {
                    return Err(invalid());
                }
                let pause_id = id.parse::<u64>().map_err(|_| invalid())?;
                match command {
                    "step" => DebugCommand::Step { pause_id },
                    "continue" => DebugCommand::Continue { pause_id },
                    "inspect" => DebugCommand::Inspect { pause_id },
                    _ => unreachable!(),
                }
            }
            _ => return Err(invalid()),
        }
    };
    Ok(command)
}

enum Source {
    Pollable(AsyncFd<Descriptor>),
    Eof,
}

impl Source {
    async fn read(&self, buffer: &mut [u8]) -> io::Result<usize> {
        let Self::Pollable(fd) = self else {
            return Ok(0);
        };
        loop {
            let mut ready = fd.readable().await?;
            match ready
                .try_io(|fd| nix::unistd::read(&fd.get_ref().file, buffer).map_err(io::Error::from))
            {
                Ok(Err(error)) if error.kind() == io::ErrorKind::Interrupted => continue,
                Ok(result) => return result,
                Err(_) => continue,
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
        // dup shares the open-file description, including status flags.
        let _ = fcntl(&self.file, FcntlArg::F_SETFL(self.flags));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_json_commands_require_exact_fields() {
        for (line, expected) in [
            (
                r#"{"command":"step","pause_id":7}"#,
                DebugCommand::Step { pause_id: 7 },
            ),
            (
                r#"{"command":"continue","pause_id":7}"#,
                DebugCommand::Continue { pause_id: 7 },
            ),
            (
                r#"{"command":"inspect","pause_id":7}"#,
                DebugCommand::Inspect { pause_id: 7 },
            ),
            (r#"{"command":"pause"}"#, DebugCommand::Pause {}),
            (r#"{"command":"cancel"}"#, DebugCommand::Cancel {}),
        ] {
            assert_eq!(parse_debug(line).unwrap(), expected);
        }
        // In particular, Serde's internally tagged unit variants used to
        // ignore unknown fields even with enum-level deny_unknown_fields.
        for line in [
            r#"{"command":"step","pause_id":7,"extra":true}"#,
            r#"{"command":"continue","pause_id":7,"extra":true}"#,
            r#"{"command":"inspect","pause_id":7,"extra":true}"#,
            r#"{"command":"pause","extra":true}"#,
            r#"{"command":"cancel","extra":true}"#,
        ] {
            assert_eq!(
                parse_debug(line).unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "{line}"
            );
        }
    }

    #[tokio::test]
    async fn buffered_shortcuts_keep_the_pause_at_input_read_time() {
        let mut input = DebugInput {
            source: Source::Eof,
            buffer: [0; 1024],
            consumed: 0,
            filled: 4,
            line: Vec::new(),
            buffer_pause: 7,
            line_pause: None,
        };
        input.buffer[..4].copy_from_slice(b"n\nn\n");
        let current = AtomicU64::new(7);
        assert!(matches!(
            input.next_command(Some(&current)).await.unwrap(),
            Some(ControlCommand::Debug(DebugCommand::Step { pause_id: 7 }))
        ));
        current.store(8, Ordering::Release);
        assert!(matches!(
            input.next_command(Some(&current)).await.unwrap(),
            Some(ControlCommand::Debug(DebugCommand::Step { pause_id: 7 }))
        ));
    }
}
