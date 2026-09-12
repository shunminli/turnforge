use std::{
    io::{self, Read},
    num::NonZeroU32,
    path::PathBuf,
    process::ExitCode,
    time::Duration,
};

use clap::{Args, Parser, Subcommand};
use turnforge::{
    Agent, AgentConfig, CancellationToken, Event, RunOutcome,
    openai::OpenAiModel,
    tools::{FileOperation, FileTool, Permissions, ShellTool, ToolRegistry, Workspace},
};

#[cfg(not(unix))]
compile_error!("The Turnforge CLI currently supports macOS and Linux only");
mod output;

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
    /// User prompt. Use '-' to read from stdin (up to 1 MiB).
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
    let args = match cli.command {
        Command::Tools(args) => {
            let (_, tools) = registry(&args)?;
            serde_json::to_writer_pretty(io::stdout(), &tools.definitions())?;
            println!();
            return Ok(0);
        }
        Command::Run(args) => args,
    };
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
    let key = std::env::var("TURNFORGE_API_KEY")
        .ok()
        .filter(|key| !key.is_empty())
        .or_else(|| {
            std::env::var("OPENAI_API_KEY")
                .ok()
                .filter(|key| !key.is_empty())
        });
    let model = OpenAiModel::new(
        &args.base_url,
        key.as_deref(),
        &args.model,
        Duration::from_secs(args.request_timeout),
    )?;
    let mut config = AgentConfig {
        max_steps: args.max_steps,
        ..AgentConfig::default()
    };
    config.system.push_str(&format!("\nWorkspace root: {}. File tool paths must be workspace-relative. Only tools granted by the host are available.", workspace.root().display()));
    let mut agent = Agent::new(model, tools, config);
    let cancel = CancellationToken::new();
    let (sender, receiver) = tokio::sync::mpsc::channel(64);
    let run = async {
        let mut output_error = None;
        let mut sink = |event: Event| {
            if output_error.is_none() && sender.try_send(event).is_err() {
                output_error = Some(io::Error::other(
                    "output queue full or closed; run cancelled",
                ));
                cancel.cancel();
            }
        };
        let result = agent.run(prompt, &cancel, &mut sink).await;
        drop(sender);
        (result, output_error)
    };
    let writer = async {
        let result = output::forward(receiver, args.json, &cancel).await;
        if result.is_err() {
            cancel.cancel();
        }
        result
    };
    // No detached signal task. On Ctrl-C, keep polling the run until tools
    // have cleaned up and all committed calls have terminal results.
    let ((result, queue_error), output_result) = {
        let execution = async { tokio::join!(run, writer) };
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
    Ok(match result? {
        RunOutcome::Completed => 0,
        RunOutcome::StepLimit => 2,
        RunOutcome::Cancelled => 130,
        RunOutcome::Failed => 1,
    })
}
