use serde::{Deserialize, Serialize};

use crate::message::{Message, ToolCall};
use crate::model::ModelDelta;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Model,
    Tools,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    Completed,
    Cancelled,
    StepLimit,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RunState {
    Ready,
    Running { step: u32, phase: Phase },
    Finished { outcome: RunOutcome },
}

/// Ordered, ephemeral notifications. Not a durable replay or recovery log.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    RunStarted,
    StepStarted { step: u32 },
    ModelDelta { delta: ModelDelta },
    MessageCommitted { message: Message },
    ToolStarted { call: ToolCall },
    RunFinished { outcome: RunOutcome },
}
