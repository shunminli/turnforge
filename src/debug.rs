//! Semantic execution control. The Agent owns facts; this module owns only
//! one run's command receiver, stepping mode and pause identifiers.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::{
    CancellationToken, Event, RunOutcome, RunState,
    message::{Message, ToolCall},
    tools::ToolDefinition,
};

const COMMAND_CAPACITY: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum DebugCommand {
    Step { pause_id: u64 },
    Continue { pause_id: u64 },
    Inspect { pause_id: u64 },
    Pause {},
    Cancel {},
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DebugPoint {
    BeforeModel,
    AfterModel,
    AfterTool { index: u32, call_id: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DebugAction {
    Model { step: u32 },
    Tool { index: u32, call: ToolCall },
    Finish { outcome: RunOutcome },
}

/// A read-only value copy, not a resume checkpoint or provider HTTP request.
/// Contains sensitive context, but no provider credentials or transport headers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DebugSnapshot {
    pub version: u32,
    pub pause_id: u64,
    pub step: u32,
    pub point: DebugPoint,
    pub next: DebugAction,
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub pending_calls: Vec<ToolCall>,
    pub max_steps: u32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DebugControlError {
    #[error("debug command queue is full")]
    Full,
    #[error("debug session has ended")]
    Closed,
}

/// The host owns this handle for the entire run. Dropping it cancels the run;
/// keep it alive through output draining. Cancellation never waits for queue space.
pub struct DebugController {
    commands: mpsc::Sender<QueuedCommand>,
    cancel: CancellationToken,
    paused_epoch: Arc<AtomicU64>,
}

impl DebugController {
    pub fn try_send(&self, command: DebugCommand) -> Result<(), DebugControlError> {
        if matches!(command, DebugCommand::Cancel {}) {
            self.cancel();
            return Ok(());
        }
        self.commands
            .try_send(QueuedCommand {
                command,
                paused_epoch: self.paused_epoch.load(Ordering::Acquire),
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => DebugControlError::Full,
                mpsc::error::TrySendError::Closed(_) => DebugControlError::Closed,
            })
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

impl Drop for DebugController {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[derive(PartialEq, Eq)]
enum Mode {
    Step,
    Continuous,
}

/// Consumed by one run, never shared or reused as a persistent session.
pub struct DebugSession {
    commands: mpsc::Receiver<QueuedCommand>,
    cancel: CancellationToken,
    mode: Mode,
    pause_id: u64,
    paused_epoch: Arc<AtomicU64>,
}

// Only the session publishes this epoch; it is command-correlation metadata,
// not another owner of Agent state. Zero means there is no active pause.
struct QueuedCommand {
    command: DebugCommand,
    paused_epoch: u64,
}

struct PauseEpoch<'a>(&'a AtomicU64);

impl Drop for PauseEpoch<'_> {
    fn drop(&mut self) {
        self.0.store(0, Ordering::Release);
    }
}

/// Use a fresh cancellation token per run. Clones of this same token share the
/// normal host cancellation path; this function does not create a second scope.
pub fn debug_channel(cancel: &CancellationToken) -> (DebugController, DebugSession) {
    let (sender, receiver) = mpsc::channel(COMMAND_CAPACITY);
    let paused_epoch = Arc::new(AtomicU64::new(0));
    (
        DebugController {
            commands: sender,
            cancel: cancel.clone(),
            paused_epoch: paused_epoch.clone(),
        },
        DebugSession {
            commands: receiver,
            cancel: cancel.clone(),
            mode: Mode::Step,
            pause_id: 0,
            paused_epoch,
        },
    )
}

impl DebugSession {
    pub(crate) fn cancellation(&self) -> &CancellationToken {
        &self.cancel
    }

    // Borrow context only while stopped at a safe boundary. Never store Agent
    // references in the session, controller, events or asynchronous host tasks.
    pub(crate) async fn checkpoint(
        &mut self,
        state: &mut RunState,
        snapshot: impl FnOnce(u64) -> DebugSnapshot,
        emit: &mut (dyn FnMut(Event) + Send),
    ) {
        // Drain a bounded batch so a noisy producer cannot starve cancellation.
        // This is not the safety boundary: a callback can refill the queue.
        // Enqueue-time epochs reject leftovers from outside the current pause.
        for _ in 0..COMMAND_CAPACITY {
            if self.cancel.is_cancelled() {
                return;
            }
            match self.commands.try_recv() {
                Ok(queued) => match queued.command {
                    DebugCommand::Pause {} => self.mode = Mode::Step,
                    DebugCommand::Cancel {} => self.cancel.cancel(),
                    command => reject(command, "not paused; wait for debug_paused", emit),
                },
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    self.cancel.cancel();
                    return;
                }
            }
        }
        if self.cancel.is_cancelled() || self.mode == Mode::Continuous {
            return;
        }
        // At most 1 + u32::MAX * 33 pauses in one run, so u64 cannot overflow.
        self.pause_id += 1;
        let snapshot = snapshot(self.pause_id);
        *state = RunState::Paused {
            step: snapshot.step,
            pause_id: snapshot.pause_id,
            point: snapshot.point.clone(),
        };
        // Enable commands before notifying the host, including synchronous
        // callbacks. Clear on resume, cancellation, panic or dropped run future.
        self.paused_epoch.store(self.pause_id, Ordering::Release);
        let epoch = PauseEpoch(&self.paused_epoch);
        emit(Event::DebugPaused {
            snapshot: Box::new(snapshot.clone()),
        });
        loop {
            let command = tokio::select! {
                biased;
                _ = self.cancel.cancelled() => return,
                command = self.commands.recv() => command,
            };
            let Some(queued) = command else {
                self.cancel.cancel();
                return;
            };
            let command = queued.command;
            match command {
                DebugCommand::Step { pause_id } | DebugCommand::Continue { pause_id }
                    if pause_id == self.pause_id && queued.paused_epoch == self.pause_id =>
                {
                    self.mode = if matches!(command, DebugCommand::Step { .. }) {
                        Mode::Step
                    } else {
                        Mode::Continuous
                    };
                    drop(epoch);
                    emit(Event::DebugResumed { pause_id });
                    return;
                }
                DebugCommand::Inspect { pause_id }
                    if pause_id == self.pause_id && queued.paused_epoch == self.pause_id =>
                {
                    emit(Event::DebugSnapshot {
                        snapshot: Box::new(snapshot.clone()),
                    });
                }
                DebugCommand::Cancel {} => self.cancel.cancel(),
                DebugCommand::Pause {} => reject(command, "already paused", emit),
                _ => reject(
                    command,
                    "stale pause_id or command submitted outside this pause",
                    emit,
                ),
            }
            // Even immediately ready commands must not monopolize the run task.
            tokio::task::yield_now().await;
        }
    }
}

fn reject(command: DebugCommand, reason: &str, emit: &mut (dyn FnMut(Event) + Send)) {
    emit(Event::DebugCommandRejected {
        command,
        reason: reason.into(),
    });
}
