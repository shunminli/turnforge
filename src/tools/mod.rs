use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::{
    CancellationToken,
    message::{ToolCall, ToolOutput},
};

mod files;
mod shell;
pub use files::{FileOperation, FileTool, Workspace};
pub use shell::ShellTool;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Read,
    Write,
    Shell,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Permissions {
    pub allow_write: bool,
    pub allow_shell: bool,
}

impl Permissions {
    fn allows(self, capability: Capability) -> bool {
        match capability {
            Capability::Read => true,
            Capability::Write => self.allow_write,
            Capability::Shell => self.allow_shell,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub capability: Capability,
}

/// A trusted extension. Implementations own side effects and must await cleanup
/// before returning on cancellation. A capability label is not a sandbox.
#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    async fn execute(&self, arguments: Value, cancel: &CancellationToken) -> ToolOutput;
}

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("duplicate tool name: {0}")]
    Duplicate(String),
}

struct Entry {
    definition: ToolDefinition,
    tool: Box<dyn Tool>,
}

pub struct ToolRegistry {
    entries: BTreeMap<String, Entry>,
    permissions: Permissions,
}

impl ToolRegistry {
    pub fn new(permissions: Permissions) -> Self {
        Self {
            entries: BTreeMap::new(),
            permissions,
        }
    }

    pub fn register(&mut self, tool: impl Tool + 'static) -> Result<(), RegistryError> {
        let definition = tool.definition();
        if self.entries.contains_key(&definition.name) {
            return Err(RegistryError::Duplicate(definition.name));
        }
        self.entries.insert(
            definition.name.clone(),
            Entry {
                definition,
                tool: Box::new(tool),
            },
        );
        Ok(())
    }

    /// Hide unavailable tools from the model, but also enforce the same policy
    /// at execution time: the model can still invent a hidden tool call.
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.entries
            .values()
            .filter(|entry| self.permissions.allows(entry.definition.capability))
            .map(|entry| entry.definition.clone())
            .collect()
    }

    pub async fn execute(&self, call: &ToolCall, cancel: &CancellationToken) -> ToolOutput {
        if cancel.is_cancelled() {
            return ToolOutput::error("cancelled", "Not executed: run cancelled");
        }
        let Some(entry) = self.entries.get(&call.name) else {
            return ToolOutput::error("unknown_tool", "Tool is not registered");
        };
        if !self.permissions.allows(entry.definition.capability) {
            return ToolOutput::error(
                "permission_denied",
                "Host has not granted this tool capability",
            );
        }
        entry.tool.execute(call.arguments.clone(), cancel).await
    }
}
