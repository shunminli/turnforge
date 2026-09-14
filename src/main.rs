use std::{
    io::{self, Read},
    num::NonZeroU32,
    path::PathBuf,
    process::ExitCode,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use clap::{Args, Parser, Subcommand};
use turnforge::{
    Agent, AgentConfig, CancellationToken, DebugCommand, Event, RunOutcome, debug_channel,
    openai::OpenAiModel,
    tools::{FileOperation, FileTool, Permissions, ShellTool, ToolRegistry, Workspace},
};

#[cfg(not(unix))]
compile_error!("The Turnforge CLI currently supports macOS and Linux only");
mod debug_input;
mod lab;
mod learning;
mod output;

enum HostMode {
    Run,
    Debug,
    Lab {
        scenario: lab::PreparedLab,
        automatic: bool,
        lesson: Option<learning::Lesson>,
    },
}

#[derive(Parser)]
#[command(version, about = "A headless Rust coding-agent harness (experimental)")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run one user turn, including any model/tool iterations.
    Run(RunArgs),
    /// Debug one turn at model/tool boundaries; stdin accepts control commands.
    Debug(RunArgs),
    /// Test the Harness with a pinned local Ollama model and synthetic tasks.
    Lab(lab::LabArgs),
    /// Explore the Harness learning roadmap without starting a model.
    Learn {
        #[arg(value_enum)]
        lesson: Option<learning::Lesson>,
    },
    /// Inspect enabled tool definitions without calling a model.
    Tools(HostArgs),
}

#[derive(Args)]
struct HostArgs {
    /// Workspace root for file tools and initial shell cwd.
    #[arg(long, default_value = ".")]
    workspace: PathBuf,
    /// Grant file creation/replacement inside workspace.
    #[arg(long)]
    allow_write: bool,
    /// Grant UNSANDBOXED shell execution with host permissions.
    #[arg(long)]
    allow_shell: bool,
    /// Per-shell deadline (seconds), includes pipe draining.
    #[arg(long, default_value = "30", value_parser = clap::value_parser!(u64).range(1..=3600))]
    tool_timeout: u64,
}

#[derive(Args)]
struct RunArgs {
    #[command(flatten)]
    host: HostArgs,
    /// User prompt. Only 'run' accepts '-' to read stdin (up to 1 MiB).
    prompt: String,
    /// A model ID supported by your chosen Chat Completions endpoint.
    #[arg(long, env = "TURNFORGE_MODEL")]
    model: String,
    /// API prefix; '/chat/completions' is appended. HTTPS or loopback HTTP.
    #[arg(
        long,
        env = "TURNFORGE_BASE_URL",
        default_value = "https://api.openai.com/v1"
    )]
    base_url: String,
    #[arg(long, default_value = "20")]
    max_steps: NonZeroU32,
    /// Total timeout for each HTTP model request, including streaming (seconds).
    #[arg(long, default_value = "120", value_parser = clap::value_parser!(u64).range(1..=3600))]
    request_timeout: u64,
    /// Emit ordered NDJSON events to stdout (contains prompts/tool data).
    #[arg(long)]
    json: bool,
}

fn registry(args: &HostArgs) -> Result<(Workspace, ToolRegistry), Box<dyn std::error::Error>> {
    let workspace = Workspace::new(&args.workspace)?;
    let mut tools = ToolRegistry::new(Permissions {
        allow_write: args.allow_write,
        allow_shell: args.allow_shell,
    });
    for operation in [
        FileOperation::Read,
        FileOperation::List,
        FileOperation::Write,
        FileOperation::Replace,
        FileOperation::Mkdir,
    ] {
        tools.register(FileTool::new(workspace.clone(), operation))?;
    }
    tools.register(ShellTool::new(
        workspace.clone(),
        Duration::from_secs(args.tool_timeout),
    ))?;
    Ok((workspace, tools))
}

#[tokio::main]
async fn main() -> ExitCode {
    match execute(Cli::parse()).await {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("turnforge: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn execute(cli: Cli) -> Result<u8, Box<dyn std::error::Error>> {
    let (args, mode) = match cli.command {
        Command::Learn { lesson } => {
            let (sender, receiver) = tokio::sync::mpsc::channel(1);
            sender.try_send(output::OutputItem::Notice(learning::catalog(lesson)))?;
            drop(sender);
            output::forward(
                receiver,
                output::DisplayMode::Text,
                &CancellationToken::new(),
            )
            .await?;
            return Ok(0);
        }
        Command::Tools(args) => {
            let (_, tools) = registry(&args)?;
            serde_json::to_writer_pretty(io::stdout(), &tools.definitions())?;
            println!();
            return Ok(0);
        }
        Command::Run(args) => (args, HostMode::Run),
        Command::Debug(args) => (args, HostMode::Debug),
        Command::Lab(options) => {
            let scenario = lab::prepare(&options).await?;
            let args = RunArgs {
                host: HostArgs {
                    workspace: scenario.workspace.path().to_path_buf(),
                    allow_write: scenario.allow_write,
                    allow_shell: false,
                    tool_timeout: 30,
                },
                prompt: scenario.prompt.clone(),
                model: scenario.model.clone(),
                base_url: scenario.base_url.clone(),
                max_steps: NonZeroU32::new(4).unwrap(),
                request_timeout: 90,
                json: false,
            };
            (
                args,
                HostMode::Lab {
                    scenario,
                    automatic: options.auto,
                    lesson: options.lesson,
                },
            )
        }
    };
    let debug = !matches!(mode, HostMode::Run);
    let (scenario, automatic, lesson) = match &mode {
        HostMode::Lab {
            scenario,
            automatic,
            lesson,
        } => (Some(scenario), *automatic, *lesson),
        _ => (None, false, None),
    };
    if debug && args.prompt == "-" {
        return Err(
            "debug does not accept prompt '-'; stdin is reserved for control commands".into(),
        );
    }
    let (workspace, tools) = registry(&args.host)?;
    let prompt = if args.prompt == "-" {
        let mut bytes = Vec::new();
        io::stdin().take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 1024 * 1024 {
            return Err("stdin prompt exceeds 1 MiB".into());
        }
        String::from_utf8(bytes)?
    } else {
        args.prompt
    };
    let key = if scenario.is_some() {
        None
    } else {
        std::env::var("TURNFORGE_API_KEY")
            .ok()
            .filter(|key| !key.is_empty())
            .or_else(|| {
                std::env::var("OPENAI_API_KEY")
                    .ok()
                    .filter(|key| !key.is_empty())
            })
    };
    let model = if scenario.is_some() {
        OpenAiModel::new_local(
            &args.base_url,
            &args.model,
            Duration::from_secs(args.request_timeout),
        )?
    } else {
        OpenAiModel::new(
            &args.base_url,
            key.as_deref(),
            &args.model,
            Duration::from_secs(args.request_timeout),
        )?
    };
    let mut config = AgentConfig {
        max_steps: args.max_steps,
        ..AgentConfig::default()
    };
    config.system.push_str(&format!("\nWorkspace root: {}. File tool paths must be workspace-relative. Only tools granted by the host are available.", workspace.root().display()));
    let mut agent = Agent::new(model, tools, config);
    let cancel = CancellationToken::new();
    let (controller, session) = if debug {
        let (controller, session) = debug_channel(&cancel);
        (Some(controller), Some(session))
    } else {
        (None, None)
    };
    // Created before stdout changes its flags; dropped after the writer exits.
    // stdin and stdout can share one terminal open-file description.
    let mut input = if debug && !automatic {
        Some(debug_input::DebugInput::stdin()?)
    } else {
        None
    };
    let run_done = CancellationToken::new();
    // Observer projection only; the library remains the authority for accepting
    // commands. Input stamps buffered shortcuts before a later pause can arrive.
    let observed_pause = AtomicU64::new(0);
    let (sender, receiver) = tokio::sync::mpsc::channel(64);
    let control_sender = sender.clone();
    let run = async {
        let mut output_error = None;
        let mut enqueue = |item| {
            if output_error.is_none() && sender.try_send(item).is_err() {
                output_error = Some(io::Error::other(
                    "output queue full or closed; run cancelled",
                ));
                cancel.cancel();
            }
        };
        if let Some(scenario) = scenario {
            enqueue(output::OutputItem::Notice(format!(
                "{}\n{}",
                scenario.description(),
                if automatic {
                    "自动模式：逐暂停点推进，完成后独立校验结果。\n"
                } else {
                    output::LAB_HELP
                }
            )));
        }
        let mut auto_error = None;
        let mut sink = |event: Event| {
            let pause = match &event {
                Event::DebugPaused { snapshot } => {
                    observed_pause.store(snapshot.pause_id, Ordering::Release);
                    Some(snapshot.pause_id)
                }
                Event::DebugResumed { .. } | Event::RunFinished { .. } => {
                    observed_pause.store(0, Ordering::Release);
                    None
                }
                _ => None,
            };
            enqueue(output::OutputItem::Event(event));
            if automatic
                && let Some(pause_id) = pause
                && let Some(controller) = &controller
                && let Err(error) = controller.try_send(DebugCommand::Step { pause_id })
            {
                auto_error = Some(error);
                controller.cancel();
            }
        };
        let mut result = match session {
            Some(session) => agent.run_debug(prompt, session, &mut sink).await,
            None => agent.run(prompt, &cancel, &mut sink).await,
        }
        .map_err(|error| Box::new(error) as Box<dyn std::error::Error>);
        run_done.cancel();
        if let Some(error) = auto_error {
            result = Err(Box::new(error));
        }
        if let Some(scenario) = scenario {
            let notice = match &result {
                Ok(RunOutcome::Completed) => match scenario.verify(agent.messages()) {
                    Ok(summary) => format!("[PASS] {summary}\n"),
                    Err(error) => {
                        let notice = format!("[FAIL] 场景结果校验失败：{error}\n");
                        result = Err(error);
                        notice
                    }
                },
                Ok(RunOutcome::Cancelled) => "[CANCELLED] 已取消，未完成场景验证。\n".into(),
                Ok(RunOutcome::StepLimit) => "[FAIL] 达到模型步骤上限，未完成场景验证。\n".into(),
                _ => "[FAIL] 运行失败，未完成场景验证。\n".into(),
            };
            enqueue(output::OutputItem::Notice(notice));
        }
        drop(sender);
        (result, output_error)
    };
    let display = if scenario.is_some() {
        output::DisplayMode::Lab { lesson, automatic }
    } else if args.json {
        output::DisplayMode::Json
    } else {
        output::DisplayMode::Text
    };
    let writer = async {
        let result = output::forward(receiver, display, &cancel).await;
        if result.is_err() {
            cancel.cancel();
        }
        result
    };
    let control_handles = controller.as_ref().zip(input.as_mut());
    let control_done = &run_done;
    let control_cancel = &cancel;
    let lab_pause = scenario.map(|_| &observed_pause);
    let control = async move {
        let Some((controller, input)) = control_handles else {
            return Ok::<(), io::Error>(());
        };
        loop {
            let command = tokio::select! {
                biased;
                _ = control_done.cancelled() => return Ok(()),
                _ = control_cancel.cancelled() => return Ok(()),
                command = input.next_command(lab_pause) => command,
            };
            match command {
                Ok(Some(debug_input::ControlCommand::Debug(DebugCommand::Cancel {})))
                | Ok(None) => {
                    controller.cancel();
                    return Ok(());
                }
                Ok(Some(debug_input::ControlCommand::Debug(command))) => {
                    if let Err(error) = controller.try_send(command) {
                        controller.cancel();
                        return Err(io::Error::other(error));
                    }
                }
                Ok(Some(command)) => {
                    let text = match command {
                        debug_input::ControlCommand::Help => output::LAB_HELP.into(),
                        debug_input::ControlCommand::Learn => learning::catalog(lesson),
                        debug_input::ControlCommand::NotPaused => {
                            "[lab] 输入时尚未暂停，请等暂停提示后再操作；p 暂停，q 取消。\n".into()
                        }
                        debug_input::ControlCommand::Debug(_) => unreachable!(),
                    };
                    if control_sender
                        .try_send(output::OutputItem::Notice(text))
                        .is_err()
                    {
                        controller.cancel();
                        return Err(io::Error::other(
                            "output queue full or closed; run cancelled",
                        ));
                    }
                }
                Err(error) => {
                    controller.cancel();
                    return Err(error);
                }
            }
        }
    };
    // No detached signal task. On Ctrl-C, keep polling the run until tools
    // have cleaned up and all committed calls have terminal results.
    let ((result, queue_error), output_result, input_result) = {
        let execution = async { tokio::join!(run, writer, control) };
        tokio::pin!(execution);
        tokio::select! {
            result = &mut execution => result,
            signal = tokio::signal::ctrl_c() => {
                cancel.cancel();
                let result = execution.await;
                signal?;
                result
            }
        }
    };
    output_result?;
    if let Some(error) = queue_error {
        return Err(error.into());
    }
    input_result?;
    Ok(match result? {
        RunOutcome::Completed => 0,
        RunOutcome::StepLimit => 2,
        RunOutcome::Cancelled => 130,
        RunOutcome::Failed => 1,
    })
}
