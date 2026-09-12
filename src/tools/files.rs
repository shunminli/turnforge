use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

use async_trait::async_trait;
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};

use super::{Capability, Tool, ToolDefinition};
use crate::{CancellationToken, message::ToolOutput};

const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
const MAX_READ_BYTES: usize = 32 * 1024;
const MAX_ENTRIES: usize = 1000;

/// A canonical root and conservative path checks, NOT an OS sandbox. Use only
/// trusted local workspaces: path checks and open are not race-free capabilities.
#[derive(Clone, Debug)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    pub fn new(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let root = path.as_ref().canonicalize()?;
        if !root.is_dir() {
            return Err(std::io::Error::other("workspace must be a directory"));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn resolve(&self, relative: &str) -> Result<PathBuf, ToolOutput> {
        if relative.is_empty() {
            return Err(ToolOutput::error("invalid_path", "Path cannot be empty"));
        }
        let mut result = self.root.clone();
        for component in Path::new(relative).components() {
            match component {
                Component::Normal(part) => {
                    result.push(part);
                    match fs::symlink_metadata(&result) {
                        Ok(metadata) if metadata.file_type().is_symlink() => {
                            return Err(ToolOutput::error(
                                "invalid_path",
                                "Symlink traversal is not allowed",
                            ));
                        }
                        Ok(_) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => return Err(io_error(error)),
                    }
                }
                Component::CurDir => {}
                _ => {
                    return Err(ToolOutput::error(
                        "invalid_path",
                        "Use a workspace-relative path without '..'",
                    ));
                }
            }
        }
        Ok(result)
    }
}

#[derive(Clone, Copy)]
pub enum FileOperation {
    Read,
    List,
    Write,
    Replace,
    Mkdir,
}

#[derive(Clone)]
pub struct FileTool {
    workspace: Workspace,
    operation: FileOperation,
}

impl FileTool {
    pub fn new(workspace: Workspace, operation: FileOperation) -> Self {
        Self {
            workspace,
            operation,
        }
    }

    fn execute_sync(
        &self,
        arguments: Value,
        cancel: &CancellationToken,
    ) -> Result<Value, ToolOutput> {
        check_cancel(cancel)?;
        match self.operation {
            FileOperation::Read => {
                let input: ReadInput = parse(arguments)?;
                if input.start_line == 0 || input.max_bytes == 0 || input.max_bytes > MAX_READ_BYTES
                {
                    return Err(ToolOutput::error(
                        "invalid_arguments",
                        "start_line >= 1; max_bytes must be 1..32768",
                    ));
                }
                let text = read_text(&self.workspace.resolve(&input.path)?)?;
                let total_lines = text.lines().count();
                if input.start_line > total_lines.max(1) {
                    return Err(ToolOutput::error(
                        "invalid_arguments",
                        "start_line exceeds file length",
                    ));
                }
                let selected = text
                    .split_inclusive('\n')
                    .skip(input.start_line - 1)
                    .collect::<String>();
                let mut end = selected.len().min(input.max_bytes);
                while !selected.is_char_boundary(end) {
                    end -= 1;
                }
                Ok(
                    json!({"text":&selected[..end], "truncated":end < selected.len(), "start_line":input.start_line, "total_lines":total_lines}),
                )
            }
            FileOperation::List => {
                let input: PathInput = parse(arguments)?;
                let mut entries = Vec::new();
                let mut truncated = false;
                for entry in fs::read_dir(self.workspace.resolve(&input.path)?).map_err(io_error)? {
                    check_cancel(cancel)?;
                    if entries.len() == MAX_ENTRIES {
                        truncated = true;
                        break;
                    }
                    let entry = entry.map_err(io_error)?;
                    let kind = entry.file_type().map_err(io_error)?;
                    entries.push(json!({"name":entry.file_name().to_string_lossy(), "kind":if kind.is_symlink() {"symlink"} else if kind.is_dir() {"directory"} else if kind.is_file() {"file"} else {"special"}}));
                }
                entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
                Ok(json!({"entries":entries, "truncated":truncated}))
            }
            FileOperation::Write => {
                let input: WriteInput = parse(arguments)?;
                let path = self.workspace.resolve(&input.path)?;
                atomic_write(&path, &input.content, cancel)?;
                Ok(json!({"path":input.path, "bytes":input.content.len()}))
            }
            FileOperation::Replace => {
                let input: ReplaceInput = parse(arguments)?;
                if input.old.is_empty() {
                    return Err(ToolOutput::error(
                        "invalid_arguments",
                        "old must be nonempty",
                    ));
                }
                let path = self.workspace.resolve(&input.path)?;
                let original = read_text(&path)?;
                if original.matches(&input.old).count() != 1 {
                    return Err(ToolOutput::error(
                        "ambiguous_match",
                        "old must occur exactly once; no changes made",
                    ));
                }
                let updated = original.replacen(&input.old, &input.new, 1);
                atomic_write(&path, &updated, cancel)?;
                Ok(json!({"path":input.path, "replacements":1, "changed":original != updated}))
            }
            FileOperation::Mkdir => {
                let input: PathInput = parse(arguments)?;
                let path = self.workspace.resolve(&input.path)?;
                check_cancel(cancel)?;
                fs::create_dir_all(&path).map_err(io_error)?;
                Ok(json!({"path":input.path}))
            }
        }
    }
}

#[async_trait]
impl Tool for FileTool {
    fn definition(&self) -> ToolDefinition {
        let (name, description, parameters, capability) = match self.operation {
            FileOperation::Read => (
                "read_file",
                "Read UTF-8 text. Paths are relative to workspace; no '..' or symlinks. max_bytes <= 32768, start_line is 1-based. Files <= 4 MiB.",
                schema_for!(ReadInput),
                Capability::Read,
            ),
            FileOperation::List => (
                "list_files",
                "List one directory (use '.' for root), up to 1000 entries; no recursive traversal.",
                schema_for!(PathInput),
                Capability::Read,
            ),
            FileOperation::Write => (
                "write_file",
                "Atomically create or overwrite a UTF-8 file <= 4 MiB inside workspace. Parent must exist; use mkdir first. Read existing files before overwriting.",
                schema_for!(WriteInput),
                Capability::Write,
            ),
            FileOperation::Replace => (
                "str_replace",
                "Replace exactly one occurrence of old with new in a workspace file. Fails without mutation on zero or multiple matches.",
                schema_for!(ReplaceInput),
                Capability::Write,
            ),
            FileOperation::Mkdir => (
                "mkdir",
                "Create a directory and its parents inside workspace.",
                schema_for!(PathInput),
                Capability::Write,
            ),
        };
        ToolDefinition {
            name: name.into(),
            description: description.into(),
            parameters: serde_json::to_value(parameters).expect("schema is JSON"),
            capability,
        }
    }

    async fn execute(&self, arguments: Value, cancel: &CancellationToken) -> ToolOutput {
        let tool = self.clone();
        let token = cancel.clone();
        // Join even on cancellation: a file operation may have committed. The
        // blocking owner returns its actual result, not a fictional rollback.
        match tokio::task::spawn_blocking(move || tool.execute_sync(arguments, &token)).await {
            Ok(Ok(data)) => ToolOutput::ok(data),
            Ok(Err(output)) => output,
            Err(_) => ToolOutput::error("internal_error", "File worker failed"),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PathInput {
    path: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReadInput {
    path: String,
    #[serde(default = "first_line")]
    start_line: usize,
    #[serde(default = "read_limit")]
    max_bytes: usize,
}
fn first_line() -> usize {
    1
}
fn read_limit() -> usize {
    MAX_READ_BYTES
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WriteInput {
    path: String,
    content: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReplaceInput {
    path: String,
    old: String,
    new: String,
}

fn parse<T: DeserializeOwned>(value: Value) -> Result<T, ToolOutput> {
    serde_json::from_value(value)
        .map_err(|error| ToolOutput::error("invalid_arguments", error.to_string()))
}
fn io_error(error: std::io::Error) -> ToolOutput {
    ToolOutput::error("io_error", error.to_string())
}
fn check_cancel(cancel: &CancellationToken) -> Result<(), ToolOutput> {
    if cancel.is_cancelled() {
        Err(ToolOutput::error(
            "cancelled",
            "Not executed: run cancelled",
        ))
    } else {
        Ok(())
    }
}

fn read_text(path: &Path) -> Result<String, ToolOutput> {
    let metadata = fs::metadata(path).map_err(io_error)?;
    if !metadata.is_file() {
        return Err(ToolOutput::error(
            "invalid_path",
            "Only regular files can be read",
        ));
    }
    let file = File::open(path).map_err(io_error)?;
    let mut bytes = Vec::new();
    file.take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(ToolOutput::error("file_too_large", "File exceeds 4 MiB"));
    }
    String::from_utf8(bytes).map_err(|_| ToolOutput::error("invalid_encoding", "File is not UTF-8"))
}

fn atomic_write(path: &Path, text: &str, cancel: &CancellationToken) -> Result<(), ToolOutput> {
    if text.len() > MAX_FILE_BYTES {
        return Err(ToolOutput::error("file_too_large", "Content exceeds 4 MiB"));
    }
    let permissions = match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Some(metadata.permissions()),
        Ok(_) => {
            return Err(ToolOutput::error(
                "invalid_path",
                "Only regular files can be overwritten",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(io_error(error)),
    };
    let parent = path
        .parent()
        .ok_or_else(|| ToolOutput::error("invalid_path", "File has no parent"))?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(io_error)?;
    if let Some(permissions) = permissions {
        file.as_file()
            .set_permissions(permissions)
            .map_err(io_error)?;
    }
    file.write_all(text.as_bytes()).map_err(io_error)?;
    file.as_file().sync_all().map_err(io_error)?;
    check_cancel(cancel)?;
    file.persist(path).map_err(|error| io_error(error.error))?;
    Ok(())
}
