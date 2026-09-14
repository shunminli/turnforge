//! Semantic debugger contract: the host advances the real Agent, not a second loop.
use std::{
    collections::VecDeque,
    num::NonZeroU32,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::{Notify, mpsc};
use turnforge::{
    Agent, AgentConfig, AgentError, CancellationToken, DebugAction, DebugCommand,
    DebugControlError, DebugPoint, DebugSnapshot, Event, Model, RunOutcome, RunState,
    debug_channel,
    message::{AssistantMessage, Message, ToolCall, ToolOutput},
    model::{ModelDelta, ModelError, ModelRequest},
    tools::{Capability, Permissions, Tool, ToolDefinition, ToolRegistry},
};

struct Script {
    replies: Mutex<VecDeque<AssistantMessage>>,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Model for Script {
    async fn complete(
        &self,
        _: ModelRequest<'_>,
        _: &CancellationToken,
        _: &mut (dyn FnMut(ModelDelta) + Send),
    ) -> Result<AssistantMessage, ModelError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected model request"))
    }
}

struct Counter {
    calls: Arc<AtomicUsize>,
    capability: Capability,
}

#[async_trait]
impl Tool for Counter {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "count".into(),
            description: "record one operation".into(),
            parameters: json!({"type":"object"}),
            capability: self.capability,
        }
    }
    async fn execute(&self, _: Value, _: &CancellationToken) -> ToolOutput {
        ToolOutput::ok(json!({"count":self.calls.fetch_add(1, Ordering::SeqCst)}))
    }
}

fn call(id: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "count".into(),
        arguments: json!({}),
    }
}

fn reply(calls: Vec<ToolCall>) -> AssistantMessage {
    AssistantMessage {
        text: "fixed reply".into(),
        tool_calls: calls,
        usage: None,
    }
}

fn agent(
    replies: Vec<AssistantMessage>,
    model_calls: Arc<AtomicUsize>,
    tool_calls: Arc<AtomicUsize>,
    capability: Capability,
    max_steps: u32,
) -> Agent<Script> {
    let mut tools = ToolRegistry::new(Permissions::default());
    tools
        .register(Counter {
            calls: tool_calls,
            capability,
        })
        .unwrap();
    Agent::new(
        Script {
            replies: Mutex::new(replies.into()),
            calls: model_calls,
        },
        tools,
        AgentConfig {
            system: "fixed system".into(),
            max_steps: NonZeroU32::new(max_steps).unwrap(),
        },
    )
}

async fn next_pause(events: &mut mpsc::UnboundedReceiver<Event>) -> DebugSnapshot {
    loop {
        match events
            .recv()
            .await
            .expect("run closed before expected pause")
        {
            Event::DebugPaused { snapshot } => return *snapshot,
            Event::RunFinished { outcome } => panic!("run finished early: {outcome:?}"),
            _ => {}
        }
    }
}

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("debug lifecycle did not complete in five seconds")
}

fn tool_results(messages: &[Message]) -> Vec<(&str, &ToolOutput)> {
    messages
        .iter()
        .filter_map(|m| match m {
            Message::Tool { call_id, output } => Some((call_id.as_str(), output)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn step_inspect_and_stale_ids_preserve_exact_operation_boundaries() {
    let model_calls = Arc::new(AtomicUsize::new(0));
    let tool_calls = Arc::new(AtomicUsize::new(0));
    let mut agent = agent(
        vec![reply(vec![call("a"), call("b")]), reply(vec![])],
        model_calls.clone(),
        tool_calls.clone(),
        Capability::Read,
        3,
    );
    let cancel = CancellationToken::new();
    let (controller, session) = debug_channel(&cancel);
    let (sender, mut events) = mpsc::unbounded_channel();
    let mut sink = |event| sender.send(event).unwrap();
    let (result, _) = bounded(async {
        tokio::join!(agent.run_debug("input", session, &mut sink), async {
            let mut snapshot = next_pause(&mut events).await;
            assert_eq!(snapshot.pause_id, 1);
            assert_eq!(snapshot.point, DebugPoint::BeforeModel);
            assert!(matches!(snapshot.next, DebugAction::Model { step: 1 }));
            assert_eq!(snapshot.version, 1);
            assert_eq!(snapshot.system, "fixed system");
            assert_eq!(snapshot.messages.len(), 1);
            assert_eq!(snapshot.tools[0].name, "count");
            assert!(snapshot.pending_calls.is_empty());
            assert_eq!(snapshot.max_steps, 3);
            assert_eq!(model_calls.load(Ordering::SeqCst), 0);
            assert_eq!(tool_calls.load(Ordering::SeqCst), 0);
            // The event is an owned copy. Mutating it must not change Agent state.
            snapshot.system.clear();
            snapshot.messages.clear();
            snapshot.tools.clear();
            controller
                .try_send(DebugCommand::Inspect { pause_id: 1 })
                .unwrap();
            match events.recv().await.unwrap() {
                Event::DebugSnapshot { snapshot } => {
                    assert_eq!(snapshot.system, "fixed system");
                    assert_eq!(snapshot.messages.len(), 1);
                    assert_eq!(snapshot.tools[0].name, "count");
                    assert_eq!(snapshot.pause_id, 1);
                }
                other => panic!("inspect did not return snapshot: {other:?}"),
            }
            assert_eq!(model_calls.load(Ordering::SeqCst), 0);
            controller
                .try_send(DebugCommand::Step { pause_id: 1 })
                .unwrap();
            let snapshot = next_pause(&mut events).await;
            assert_eq!(snapshot.pause_id, 2);
            assert_eq!(snapshot.point, DebugPoint::AfterModel);
            assert!(
                matches!(&snapshot.next, DebugAction::Tool { index: 0, call } if call.id == "a")
            );
            assert_eq!(snapshot.pending_calls.len(), 2);
            assert_eq!(snapshot.messages.len(), 2);
            assert_eq!(model_calls.load(Ordering::SeqCst), 1);
            assert_eq!(tool_calls.load(Ordering::SeqCst), 0);
            controller
                .try_send(DebugCommand::Step { pause_id: 1 })
                .unwrap();
            assert!(matches!(
                events.recv().await,
                Some(Event::DebugCommandRejected { .. })
            ));
            controller
                .try_send(DebugCommand::Inspect { pause_id: 2 })
                .unwrap();
            assert!(matches!(
                events.recv().await,
                Some(Event::DebugSnapshot { .. })
            ));
            assert_eq!(tool_calls.load(Ordering::SeqCst), 0);
            controller
                .try_send(DebugCommand::Step { pause_id: 2 })
                .unwrap();
            let snapshot = next_pause(&mut events).await;
            assert_eq!(snapshot.pause_id, 3);
            assert_eq!(
                snapshot.point,
                DebugPoint::AfterTool {
                    index: 0,
                    call_id: "a".into()
                }
            );
            assert!(
                matches!(&snapshot.next, DebugAction::Tool { index: 1, call } if call.id == "b")
            );
            assert_eq!(
                snapshot
                    .pending_calls
                    .iter()
                    .map(|c| c.id.as_str())
                    .collect::<Vec<_>>(),
                ["b"]
            );
            assert_eq!(tool_results(&snapshot.messages).len(), 1);
            assert_eq!(tool_calls.load(Ordering::SeqCst), 1);
            controller
                .try_send(DebugCommand::Step { pause_id: 3 })
                .unwrap();
            let snapshot = next_pause(&mut events).await;
            assert_eq!(snapshot.pause_id, 4);
            assert!(matches!(snapshot.next, DebugAction::Model { step: 2 }));
            assert!(snapshot.pending_calls.is_empty());
            assert_eq!(tool_results(&snapshot.messages).len(), 2);
            assert_eq!(tool_calls.load(Ordering::SeqCst), 2);
            assert_eq!(model_calls.load(Ordering::SeqCst), 1);
            controller
                .try_send(DebugCommand::Step { pause_id: 4 })
                .unwrap();
            let snapshot = next_pause(&mut events).await;
            assert_eq!(snapshot.pause_id, 5);
            assert!(matches!(
                snapshot.next,
                DebugAction::Finish {
                    outcome: RunOutcome::Completed
                }
            ));
            assert_eq!(snapshot.messages.len(), 5);
            assert_eq!(model_calls.load(Ordering::SeqCst), 2);
            controller
                .try_send(DebugCommand::Step { pause_id: 5 })
                .unwrap();
            loop {
                if let Event::RunFinished { outcome } = events.recv().await.unwrap() {
                    assert_eq!(outcome, RunOutcome::Completed);
                    break;
                }
            }
            // Keep the controller alive until the terminal event, as a real host does.
            drop(controller);
        })
    })
    .await;
    assert_eq!(result.unwrap(), RunOutcome::Completed);
    assert_eq!(agent.messages().len(), 5);
}

#[tokio::test]
async fn continue_has_the_same_transcript_and_non_debug_events_as_run() {
    let create = || {
        agent(
            vec![reply(vec![call("a")]), reply(vec![])],
            Arc::default(),
            Arc::default(),
            Capability::Read,
            3,
        )
    };
    let mut normal = create();
    let mut debug = create();
    let token = CancellationToken::new();
    let mut normal_events = Vec::new();
    normal
        .run("input", &token, &mut |e| normal_events.push(e))
        .await
        .unwrap();
    let (controller, session) = debug_channel(&token);
    let mut debug_events = Vec::new();
    assert_eq!(
        bounded(debug.run_debug("input", session, &mut |e| {
            if let Event::DebugPaused { snapshot } = &e {
                controller
                    .try_send(DebugCommand::Continue {
                        pause_id: snapshot.pause_id,
                    })
                    .unwrap();
            }
            debug_events.push(e);
        }))
        .await
        .unwrap(),
        RunOutcome::Completed
    );
    let debug_events: Vec<_> = debug_events
        .into_iter()
        .filter(|e| {
            !matches!(
                e,
                Event::DebugPaused { .. }
                    | Event::DebugSnapshot { .. }
                    | Event::DebugResumed { .. }
                    | Event::DebugCommandRejected { .. }
            )
        })
        .collect();
    assert_eq!(
        serde_json::to_value(normal.messages()).unwrap(),
        serde_json::to_value(debug.messages()).unwrap()
    );
    assert_eq!(
        serde_json::to_value(normal_events).unwrap(),
        serde_json::to_value(debug_events).unwrap()
    );
}

struct GatedModel {
    started: Arc<Notify>,
    release: Arc<Notify>,
}
#[async_trait]
impl Model for GatedModel {
    async fn complete(
        &self,
        _: ModelRequest<'_>,
        _: &CancellationToken,
        _: &mut (dyn FnMut(ModelDelta) + Send),
    ) -> Result<AssistantMessage, ModelError> {
        self.started.notify_one();
        self.release.notified().await;
        Ok(reply(vec![]))
    }
}

#[tokio::test]
async fn pause_during_a_model_waits_for_its_complete_response_boundary() {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let mut agent = Agent::new(
        GatedModel {
            started: started.clone(),
            release: release.clone(),
        },
        ToolRegistry::new(Permissions::default()),
        AgentConfig::default(),
    );
    let token = CancellationToken::new();
    let (controller, session) = debug_channel(&token);
    let (sender, mut events) = mpsc::unbounded_channel();
    let mut sink = |event| sender.send(event).unwrap();
    let (result, _) = bounded(async { tokio::join!(agent.run_debug("input", session, &mut sink), async {
        assert_eq!(next_pause(&mut events).await.pause_id, 1);
        controller.try_send(DebugCommand::Continue { pause_id: 1 }).unwrap();
        started.notified().await;
        controller.try_send(DebugCommand::Pause {}).unwrap();
        release.notify_one();
        let snapshot = next_pause(&mut events).await;
        assert_eq!(snapshot.pause_id, 2);
        assert!(matches!(snapshot.messages.last(), Some(Message::Assistant { message }) if message.text == "fixed reply"));
        controller.try_send(DebugCommand::Continue { pause_id: 2 }).unwrap();
        loop { if matches!(events.recv().await, Some(Event::RunFinished { .. })) { break; } }
    }) }).await;
    assert_eq!(result.unwrap(), RunOutcome::Completed);
}

#[tokio::test]
async fn cancelling_at_the_tool_preview_closes_calls_without_executing_them() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut agent = agent(
        vec![reply(vec![call("a"), call("b")])],
        Arc::default(),
        calls.clone(),
        Capability::Read,
        3,
    );
    let token = CancellationToken::new();
    let (controller, session) = debug_channel(&token);
    let mut events = Vec::new();
    assert_eq!(
        bounded(agent.run_debug("input", session, &mut |e| {
            if let Event::DebugPaused { snapshot } = &e {
                if snapshot.pause_id == 1 {
                    controller
                        .try_send(DebugCommand::Step { pause_id: 1 })
                        .unwrap();
                } else {
                    controller.cancel();
                }
            }
            events.push(e);
        }))
        .await
        .unwrap(),
        RunOutcome::Cancelled
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let results = tool_results(agent.messages());
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(
        |(_, result)| matches!(result, ToolOutput::Error { code, .. } if code == "cancelled")
    ));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::ToolStarted { .. }))
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::RunFinished { .. }))
            .count(),
        1
    );
}

struct CleanupTool {
    started: Arc<Notify>,
    cancelling: Arc<Notify>,
    release: Arc<Notify>,
    cleaned: Arc<AtomicBool>,
}
#[async_trait]
impl Tool for CleanupTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "count".into(),
            description: "owned cleanup".into(),
            parameters: json!({"type":"object"}),
            capability: Capability::Read,
        }
    }
    async fn execute(&self, _: Value, token: &CancellationToken) -> ToolOutput {
        self.started.notify_one();
        token.cancelled().await;
        self.cancelling.notify_one();
        self.release.notified().await;
        self.cleaned.store(true, Ordering::SeqCst);
        ToolOutput::error("cancelled", "cleanup completed")
    }
}

#[tokio::test]
async fn cancellation_bypasses_a_full_queue_and_awaits_active_tool_cleanup() {
    let started = Arc::new(Notify::new());
    let cancelling = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let cleaned = Arc::new(AtomicBool::new(false));
    let mut tools = ToolRegistry::new(Permissions::default());
    tools
        .register(CleanupTool {
            started: started.clone(),
            cancelling: cancelling.clone(),
            release: release.clone(),
            cleaned: cleaned.clone(),
        })
        .unwrap();
    let mut agent = Agent::new(
        Script {
            replies: Mutex::new(vec![reply(vec![call("a"), call("b")])].into()),
            calls: Arc::default(),
        },
        tools,
        AgentConfig::default(),
    );
    let token = CancellationToken::new();
    let (controller, session) = debug_channel(&token);
    let mut sink = |e| {
        if let Event::DebugPaused { snapshot } = e {
            controller
                .try_send(DebugCommand::Continue {
                    pause_id: snapshot.pause_id,
                })
                .unwrap();
        }
    };
    let (result, _) = bounded(async {
        tokio::join!(agent.run_debug("input", session, &mut sink), async {
            started.notified().await;
            let mut full = false;
            for _ in 0..1024 {
                if controller.try_send(DebugCommand::Pause {}) == Err(DebugControlError::Full) {
                    full = true;
                    break;
                }
            }
            assert!(full, "control channel must have a bounded capacity");
            controller.cancel();
            cancelling.notified().await;
            assert!(!cleaned.load(Ordering::SeqCst));
            release.notify_one();
        })
    })
    .await;
    assert_eq!(result.unwrap(), RunOutcome::Cancelled);
    assert!(
        cleaned.load(Ordering::SeqCst),
        "Agent dropped the side-effect future before cleanup"
    );
    let results = tool_results(agent.messages());
    assert_eq!(
        results.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert!(results.iter().all(
        |(_, result)| matches!(result, ToolOutput::Error { code, .. } if code == "cancelled")
    ));
}

#[tokio::test]
async fn dropping_the_controller_cancels_a_paused_run_without_a_model_request() {
    let model_calls = Arc::new(AtomicUsize::new(0));
    let mut agent = agent(
        vec![],
        model_calls.clone(),
        Arc::default(),
        Capability::Read,
        1,
    );
    let token = CancellationToken::new();
    let (controller, session) = debug_channel(&token);
    let mut controller = Some(controller);
    assert_eq!(
        bounded(agent.run_debug("input", session, &mut |e| {
            if matches!(e, Event::DebugPaused { .. }) {
                drop(controller.take());
            }
        }))
        .await
        .unwrap(),
        RunOutcome::Cancelled
    );
    assert_eq!(model_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn dropping_a_paused_future_does_not_allow_ambiguous_resume() {
    let mut agent = agent(vec![], Arc::default(), Arc::default(), Capability::Read, 1);
    let token = CancellationToken::new();
    let (_controller, session) = debug_channel(&token);
    let mut sink = |_| {};
    {
        let future = agent.run_debug("input", session, &mut sink);
        tokio::pin!(future);
        assert!(futures_util::poll!(&mut future).is_pending());
    }
    assert!(matches!(agent.state(), RunState::Paused { .. }));
    assert!(matches!(
        agent.run("again", &token, &mut sink).await,
        Err(AgentError::Interrupted)
    ));
}

#[tokio::test]
async fn stepping_never_grants_permission_or_bypasses_the_step_limit() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut agent = agent(
        vec![reply(vec![call("a")])],
        Arc::default(),
        calls.clone(),
        Capability::Write,
        1,
    );
    let token = CancellationToken::new();
    let (controller, session) = debug_channel(&token);
    let mut snapshots = Vec::new();
    assert_eq!(
        bounded(agent.run_debug("input", session, &mut |e| {
            if let Event::DebugPaused { snapshot } = e {
                assert!(snapshot.tools.is_empty());
                controller
                    .try_send(DebugCommand::Step {
                        pause_id: snapshot.pause_id,
                    })
                    .unwrap();
                snapshots.push(snapshot);
            }
        }))
        .await
        .unwrap(),
        RunOutcome::StepLimit
    );
    assert_eq!(snapshots.len(), 3);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(
        matches!(tool_results(agent.messages())[0].1, ToolOutput::Error { code, .. } if code == "permission_denied")
    );
}

#[tokio::test]
async fn an_early_command_for_a_future_pause_cannot_release_that_pause() {
    let tool_calls = Arc::new(AtomicUsize::new(0));
    let mut agent = agent(
        vec![reply(vec![call("a")])],
        Arc::default(),
        tool_calls.clone(),
        Capability::Read,
        1,
    );
    let token = CancellationToken::new();
    let (controller, session) = debug_channel(&token);
    let mut rejected = 0;
    let mut pauses = 0;
    let outcome = bounded(agent.run_debug("input", session, &mut |event| match event {
        Event::DebugPaused { snapshot } => {
            pauses += 1;
            if snapshot.pause_id == 2 {
                assert_eq!(rejected, 1, "early future-ID command was not rejected");
                assert_eq!(tool_calls.load(Ordering::SeqCst), 0);
            }
            controller
                .try_send(DebugCommand::Step {
                    pause_id: snapshot.pause_id,
                })
                .unwrap();
            if snapshot.pause_id == 1 {
                // This ID will exist later, but the corresponding pause has not
                // happened. Preloading it must not authorize that later tool.
                controller
                    .try_send(DebugCommand::Step { pause_id: 2 })
                    .unwrap();
            }
        }
        Event::DebugCommandRejected {
            command: DebugCommand::Step { pause_id: 2 },
            ..
        } => rejected += 1,
        _ => {}
    }))
    .await
    .unwrap();
    assert_eq!(outcome, RunOutcome::StepLimit);
    assert_eq!(pauses, 3);
    assert_eq!(tool_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn replenished_early_commands_cannot_cross_into_a_new_pause() {
    let tool_calls = Arc::new(AtomicUsize::new(0));
    let mut agent = agent(
        vec![reply(vec![call("a")])],
        Arc::default(),
        tool_calls.clone(),
        Capability::Read,
        1,
    );
    let token = CancellationToken::new();
    let (controller, session) = debug_channel(&token);
    let mut preview_published = false;
    let mut inspected_preview = false;
    let mut replenished = 0;
    let outcome = bounded(agent.run_debug("input", session, &mut |event| match event {
        Event::DebugPaused { snapshot } if snapshot.pause_id == 1 => {
            controller
                .try_send(DebugCommand::Step { pause_id: 1 })
                .unwrap();
            controller
                .try_send(DebugCommand::Step { pause_id: 2 })
                .unwrap();
        }
        Event::DebugCommandRejected {
            command: DebugCommand::Step { pause_id: 2 },
            ..
        } if !preview_published && replenished < 64 => {
            // The synchronous consumer can refill the bounded channel while the
            // Agent rejects commands. None was issued for a published pause 2.
            // Cap this adversarial input so a broken drain loop cannot hang CI.
            replenished += 1;
            controller
                .try_send(DebugCommand::Step { pause_id: 2 })
                .unwrap();
        }
        Event::DebugPaused { snapshot } if snapshot.pause_id == 2 => {
            preview_published = true;
            controller
                .try_send(DebugCommand::Inspect { pause_id: 2 })
                .unwrap();
        }
        Event::DebugSnapshot { snapshot } if snapshot.pause_id == 2 => {
            inspected_preview = true;
            controller.cancel();
        }
        // If the stale command advanced the tool, allow cleanup before failing
        // the oracle below rather than panicking inside the event callback.
        Event::DebugPaused { .. } => controller.cancel(),
        _ => {}
    }))
    .await
    .unwrap();
    assert_eq!(outcome, RunOutcome::Cancelled);
    assert!(preview_published);
    assert_eq!(
        tool_calls.load(Ordering::SeqCst),
        0,
        "an early command executed a tool without any Step/Continue sent after pause 2; inspected={inspected_preview}, replenished={replenished}"
    );
    assert!(
        inspected_preview,
        "Inspect must observe the still-paused tool preview"
    );
}

#[test]
fn debug_commands_have_strict_json_fields_and_required_pause_ids() {
    for valid in [
        json!({"command":"step","pause_id":1}),
        json!({"command":"continue","pause_id":1}),
        json!({"command":"inspect","pause_id":1}),
        json!({"command":"pause"}),
        json!({"command":"cancel"}),
    ] {
        let command: DebugCommand = serde_json::from_value(valid.clone()).unwrap();
        assert_eq!(serde_json::to_value(command).unwrap(), valid);
        let mut extra = valid;
        extra["unexpected"] = json!(true);
        assert!(serde_json::from_value::<DebugCommand>(extra).is_err());
    }
    for invalid in [
        json!({"command":"step"}),
        json!({"command":"continue","pause_id":-1}),
        json!({"command":"inspect","pause_id":"1"}),
        json!({"command":"pause","pause_id":1}),
        json!({"command":"cancel","pause_id":1}),
        json!({"command":"unknown"}),
    ] {
        assert!(serde_json::from_value::<DebugCommand>(invalid).is_err());
    }
}
