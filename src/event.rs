use serde::{Deserialize, Serialize};

use crate::debug::{DebugCommand, DebugPoint, DebugSnapshot};
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
    Running {
        step: u32,
        phase: Phase,
    },
    Paused {
        step: u32,
        pause_id: u64,
        point: DebugPoint,
    },
    Finished {
        outcome: RunOutcome,
    },
}

/// Ordered, ephemeral notifications. Not a durable replay or recovery log.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    RunStarted,
    StepStarted {
        step: u32,
    },
    ModelDelta {
        delta: ModelDelta,
    },
    MessageCommitted {
        message: Message,
    },
    ToolStarted {
        call: ToolCall,
    },
    DebugPaused {
        snapshot: Box<DebugSnapshot>,
    },
    DebugSnapshot {
        snapshot: Box<DebugSnapshot>,
    },
    DebugResumed {
        pause_id: u64,
    },
    DebugCommandRejected {
        command: DebugCommand,
        reason: String,
    },
    /// Host admission receipt; the core never emits this event itself.
    SteerQueued {
        input_id: u64,
    },
    /// The Agent committed the input as User, not necessarily sent HTTP.
    SteerApplied {
        input_id: u64,
    },
    SteerRejected {
        input_id: Option<u64>,
        reason: String,
    },
    RunFinished {
        outcome: RunOutcome,
    },
}
