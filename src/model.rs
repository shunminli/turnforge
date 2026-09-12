use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::{
    message::{AssistantMessage, Message},
    tools::ToolDefinition,
};

pub struct ModelRequest<'a> {
    pub system: &'a str,
    pub messages: &'a [Message],
    pub tools: &'a [ToolDefinition],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelDelta {
    Text { text: String },
    ToolArguments { index: u32, fragment: String },
}

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("model request cancelled")]
    Cancelled,
    #[error("model HTTP request failed: {0}")]
    Transport(String),
    #[error("model endpoint returned HTTP {0}")]
    HttpStatus(u16),
    #[error("invalid model stream: {0}")]
    Protocol(String),
    #[error("invalid provider configuration: {0}")]
    Configuration(String),
}

/// Implementations must be cancellation-safe when their future is dropped:
/// no detached tasks or side-effecting tools. Deltas are provisional until
/// a validated AssistantMessage is returned. Callbacks must not block or panic.
#[async_trait]
pub trait Model: Send + Sync {
    async fn complete(
        &self,
        request: ModelRequest<'_>,
        cancel: &CancellationToken,
        emit: &mut (dyn FnMut(ModelDelta) + Send),
    ) -> Result<AssistantMessage, ModelError>;
}
