//! Turnforge's headless harness. Read `agent::Agent::run` for the orchestration loop.
//!
//! The host owns input, output and cancellation; the agent exclusively owns its
//! transcript. No terminal, signal handler or global configuration lives here.

pub mod agent;
pub mod event;
pub mod message;
pub mod model;
pub mod openai;
pub mod tools;

pub use agent::{Agent, AgentConfig, AgentError};
pub use event::{Event, Phase, RunOutcome, RunState};
pub use model::Model;
pub use tokio_util::sync::CancellationToken;
