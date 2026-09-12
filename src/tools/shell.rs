use std::time::Duration;

use async_trait::async_trait;
use schemars::{JsonSchema, schema_for};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Capability, Tool, ToolDefinition, Workspace};
use crate::{CancellationToken, message::ToolOutput};

pub struct ShellTool {
    workspace: Workspace,
    timeout: Duration,
}

impl ShellTool {
    pub fn new(workspace: Workspace, timeout: Duration) -> Self {
        Self { workspace, timeout }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ShellInput {
    /// Explain the intended action for the host's event log.
    description: String,
    command: String,
}

#[async_trait]
impl Tool for ShellTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name:"bash".into(),
            description:"Run bash -c in the workspace with host permissions (NOT sandboxed). No interactive stdin. Timeout and output limits apply; do not start background services.".into(),
            parameters:serde_json::to_value(schema_for!(ShellInput)).expect("schema is JSON"),
            capability:Capability::Shell,
        }
    }

    async fn execute(&self, arguments: Value, cancel: &CancellationToken) -> ToolOutput {
        let input: ShellInput = match serde_json::from_value(arguments) {
            Ok(input) => input,
            Err(error) => return ToolOutput::error("invalid_arguments", error.to_string()),
        };
        if input.command.trim().is_empty()
            || input.description.trim().is_empty()
            || self.timeout.is_zero()
        {
            return ToolOutput::error(
                "invalid_arguments",
                "Nonempty command/description and nonzero timeout required",
            );
        }
        if cancel.is_cancelled() {
            return ToolOutput::error("cancelled", "Not executed: run cancelled");
        }
        #[cfg(unix)]
        {
            run_unix(&self.workspace, &input.command, self.timeout, cancel).await
        }
        #[cfg(not(unix))]
        {
            ToolOutput::error(
                "unsupported_platform",
                "Shell is currently supported on Unix only",
            )
        }
    }
}

#[cfg(unix)]
async fn run_unix(
    workspace: &Workspace,
    command: &str,
    timeout: Duration,
    cancel: &CancellationToken,
) -> ToolOutput {
    use std::process::Stdio;
    use tokio::process::Command;

    let mut command_builder = Command::new("/bin/bash");
    command_builder
        .args(["--noprofile", "--norc", "-c", command])
        .current_dir(workspace.root())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("TURNFORGE_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .process_group(0)
        .kill_on_drop(true);
    let mut child = match command_builder.spawn() {
        Ok(child) => child,
        Err(error) => return ToolOutput::error("spawn_failed", error.to_string()),
    };
    let group = ProcessGroup(child.id().expect("newly spawned process has a PID") as i32);
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    // Poll both pipes and wait concurrently; reading stdout first can deadlock
    // when stderr fills its OS pipe buffer. Keep draining after the output cap.
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(("cancelled", "Shell cancelled; process group killed")),
        _ = tokio::time::sleep(timeout) => Err(("timeout", "Shell timed out; process group killed")),
        result = async { tokio::join!(child.wait(), capture(stdout), capture(stderr)) } => Ok(result),
    };
    let kill_result = group.kill();
    // Also clean up descendants on success; background processes are not owned
    // sessions. A process that deliberately escapes the group requires a sandbox.
    if let Err(error) = kill_result {
        return ToolOutput::error("cleanup_failed", error.to_string());
    }
    match result {
        Err((code, message)) => match child.wait().await {
            Ok(_) => ToolOutput::error(code, message),
            Err(error) => ToolOutput::error("cleanup_failed", error.to_string()),
        },
        Ok((Ok(status), Ok((stdout, stdout_truncated)), Ok((stderr, stderr_truncated)))) => {
            let data = json!({"exit_code":status.code(), "stdout":stdout,"stderr":stderr,"stdout_truncated":stdout_truncated,"stderr_truncated":stderr_truncated});
            if status.success() {
                ToolOutput::ok(data)
            } else {
                ToolOutput::error("command_failed", data.to_string())
            }
        }
        Ok(_) => ToolOutput::error("io_error", "Unable to wait for shell or capture output"),
    }
}

#[cfg(unix)]
struct ProcessGroup(i32);
#[cfg(unix)]
impl ProcessGroup {
    fn kill(&self) -> Result<(), nix::errno::Errno> {
        match nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(self.0),
            nix::sys::signal::Signal::SIGKILL,
        ) {
            Err(nix::errno::Errno::ESRCH) => Ok(()),
            result => result,
        }
    }
}
#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        let _ = self.kill();
    }
}

#[cfg(unix)]
async fn capture(mut pipe: impl tokio::io::AsyncRead + Unpin) -> std::io::Result<(String, bool)> {
    use tokio::io::AsyncReadExt;
    const LIMIT: usize = 32 * 1024;
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    let mut truncated = false;
    loop {
        let count = pipe.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let keep = (LIMIT - bytes.len()).min(count);
        bytes.extend_from_slice(&buffer[..keep]);
        truncated |= keep < count;
    }
    Ok((String::from_utf8_lossy(&bytes).into_owned(), truncated))
}
