use std::{
    collections::VecDeque,
    num::NonZeroU32,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use serde_json::{Value, json};
use turnforge::{
    Agent, AgentConfig, AgentError, CancellationToken, DebugAction, DebugCommand, DebugPoint,
    Event, Model, RunOutcome, RunState, debug_channel,
    message::{AssistantMessage, Message, ToolCall, ToolOutput},
    model::{ModelDelta, ModelError, ModelRequest},
    tools::{Capability, Permissions, Tool, ToolDefinition, ToolRegistry},
};

struct Script {
    replies: Mutex<VecDeque<Result<AssistantMessage, ModelError>>>,
    histories: Arc<Mutex<Vec<Vec<Message>>>>,
}
#[async_trait]
impl Model for Script {
    async fn complete(
        &self,
        request: ModelRequest<'_>,
        _: &CancellationToken,
        _: &mut (dyn FnMut(ModelDelta) + Send),
    ) -> Result<AssistantMessage, ModelError> {
        self.histories
            .lock()
            .unwrap()
            .push(request.messages.to_vec());
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected model request")
    }
}

struct Counter {
    calls: Arc<AtomicUsize>,
    cancel_on_call: bool,
}
#[async_trait]
impl Tool for Counter {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "count".into(),
            description: "test counter".into(),
            parameters: json!({"type":"object"}),
            capability: Capability::Read,
        }
    }
    async fn execute(&self, _: Value, cancel: &CancellationToken) -> ToolOutput {
        let count = self.calls.fetch_add(1, Ordering::SeqCst);
        if self.cancel_on_call {
            cancel.cancel();
        }
        ToolOutput::ok(json!({"count":count}))
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
        text: "response".into(),
        tool_calls: calls,
        usage: None,
    }
}
fn script(replies: Vec<Result<AssistantMessage, ModelError>>) -> Script {
    Script {
        replies: Mutex::new(replies.into()),
        histories: Arc::default(),
    }
}
fn registry(counter: Arc<AtomicUsize>, cancel_on_call: bool) -> ToolRegistry {
    let mut tools = ToolRegistry::new(Permissions::default());
    tools
        .register(Counter {
            calls: counter,
            cancel_on_call,
        })
        .unwrap();
    tools
}
fn terminal_events(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, Event::RunFinished { .. }))
        .count()
}

#[tokio::test]
async fn model_tools_model_and_second_user_turn_have_ordered_history() {
    let model = script(vec![
        Ok(reply(vec![call("a"), call("b")])),
        Ok(reply(vec![])),
        Ok(reply(vec![])),
    ]);
    let histories = model.histories.clone();
    let count = Arc::default();
    let mut agent = Agent::new(model, registry(count, false), AgentConfig::default());
    let mut events = Vec::new();
    let outcome = agent
        .run("first", &CancellationToken::new(), &mut |event| {
            events.push(event)
        })
        .await
        .unwrap();
    assert_eq!(outcome, RunOutcome::Completed);
    assert_eq!(terminal_events(&events), 1);
    assert_eq!(agent.messages().len(), 5);
    let transcript = histories.lock().unwrap()[1].clone();
    assert!(
        matches!(&transcript[2], Message::Tool { call_id, output:ToolOutput::Ok { data } } if call_id=="a" && data["count"]==0)
    );
    assert!(
        matches!(&transcript[3], Message::Tool { call_id, output:ToolOutput::Ok { data } } if call_id=="b" && data["count"]==1)
    );
    agent
        .run("second", &CancellationToken::new(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(agent.messages().len(), 7);
    assert_eq!(histories.lock().unwrap()[2].len(), 6);
}

#[tokio::test]
async fn cancellation_closes_pending_calls_without_running_them() {
    let count = Arc::new(AtomicUsize::new(0));
    let mut agent = Agent::new(
        script(vec![Ok(reply(vec![call("a"), call("b")]))]),
        registry(count.clone(), true),
        AgentConfig::default(),
    );
    let mut events = Vec::new();
    let outcome = agent
        .run("cancel", &CancellationToken::new(), &mut |e| events.push(e))
        .await
        .unwrap();
    assert_eq!(outcome, RunOutcome::Cancelled);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(terminal_events(&events), 1);
    assert!(
        matches!(&agent.messages()[3], Message::Tool { call_id, output:ToolOutput::Error { code, .. } } if call_id=="b" && code=="cancelled")
    );
    assert_eq!(
        agent.state(),
        &RunState::Finished {
            outcome: RunOutcome::Cancelled
        }
    );
}

#[tokio::test]
async fn step_limit_still_closes_tool_calls() {
    let mut agent = Agent::new(
        script(vec![Ok(reply(vec![call("a")]))]),
        registry(Arc::default(), false),
        AgentConfig {
            max_steps: NonZeroU32::new(1).unwrap(),
            ..AgentConfig::default()
        },
    );
    assert_eq!(
        agent
            .run("go", &CancellationToken::new(), &mut |_| {})
            .await
            .unwrap(),
        RunOutcome::StepLimit
    );
    assert!(matches!(
        agent.messages().last(),
        Some(Message::Tool { .. })
    ));
}

#[tokio::test]
async fn invalid_calls_never_commit_or_execute() {
    for calls in [
        vec![call("a"), call("a")],
        vec![call("")],
        vec![ToolCall {
            arguments: json!([]),
            ..call("a")
        }],
    ] {
        let count = Arc::new(AtomicUsize::new(0));
        let mut agent = Agent::new(
            script(vec![Ok(reply(calls))]),
            registry(count.clone(), false),
            AgentConfig::default(),
        );
        let mut events = Vec::new();
        assert!(
            agent
                .run("go", &CancellationToken::new(), &mut |e| events.push(e))
                .await
                .is_err()
        );
        assert_eq!(count.load(Ordering::SeqCst), 0);
        assert_eq!(agent.messages().len(), 1);
        assert_eq!(terminal_events(&events), 1);
        assert_eq!(
            agent.state(),
            &RunState::Finished {
                outcome: RunOutcome::Failed
            }
        );
    }
}

#[tokio::test]
async fn provider_error_leaves_no_partial_assistant() {
    let mut agent = Agent::new(
        script(vec![Err(ModelError::HttpStatus(429))]),
        registry(Arc::default(), false),
        AgentConfig::default(),
    );
    let mut events = Vec::new();
    assert!(
        agent
            .run("go", &CancellationToken::new(), &mut |e| events.push(e))
            .await
            .is_err()
    );
    assert_eq!(agent.messages().len(), 1);
    assert_eq!(terminal_events(&events), 1);
}

#[tokio::test]
async fn pre_cancelled_run_does_not_call_model() {
    let mut agent = Agent::new(
        script(vec![]),
        registry(Arc::default(), false),
        AgentConfig::default(),
    );
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        agent.run("go", &cancel, &mut |_| {}).await.unwrap(),
        RunOutcome::Cancelled
    );
}

struct Pending;
#[async_trait]
impl Model for Pending {
    async fn complete(
        &self,
        _: ModelRequest<'_>,
        _: &CancellationToken,
        _: &mut (dyn FnMut(ModelDelta) + Send),
    ) -> Result<AssistantMessage, ModelError> {
        std::future::pending().await
    }
}

#[tokio::test]
async fn host_cancellation_stops_a_pending_model() {
    let mut agent = Agent::new(
        Pending,
        registry(Arc::default(), false),
        AgentConfig::default(),
    );
    let cancel = CancellationToken::new();
    let mut sink = |_| {};
    let (result, _) = tokio::join!(agent.run("go", &cancel, &mut sink), async {
        tokio::task::yield_now().await;
        cancel.cancel();
    });
    assert_eq!(result.unwrap(), RunOutcome::Cancelled);
}

#[tokio::test]
async fn dropping_a_run_prevents_ambiguous_resume() {
    let mut agent = Agent::new(
        Pending,
        registry(Arc::default(), false),
        AgentConfig::default(),
    );
    let token = CancellationToken::new();
    let mut sink = |_| {};
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(10),
            agent.run("go", &token, &mut sink)
        )
        .await
        .is_err()
    );
    assert!(matches!(
        agent.run("again", &token, &mut sink).await,
        Err(turnforge::AgentError::Interrupted)
    ));
}

#[tokio::test]
async fn repeated_tool_batch_closes_every_call_without_executing_the_batch() {
    let first = vec![
        ToolCall {
            arguments: json!({"path":"a", "line":1}),
            ..call("a")
        },
        ToolCall {
            arguments: json!({"path":"b"}),
            ..call("b")
        },
    ];
    let repeated = vec![
        ToolCall {
            arguments: json!({"line":1, "path":"a"}),
            ..call("fresh-a")
        },
        ToolCall {
            arguments: json!({"path":"b"}),
            ..call("fresh-b")
        },
    ];
    let model = script(vec![
        Ok(reply(first.clone())),
        Ok(reply(repeated)),
        Ok(reply(first)),
        Ok(reply(vec![])),
    ]);
    let histories = model.histories.clone();
    let count = Arc::new(AtomicUsize::new(0));
    let limit = NonZeroU32::new(2).unwrap();
    let mut agent = Agent::new(
        model,
        registry(count.clone(), false),
        AgentConfig {
            max_steps: limit,
            tool_repeat_limit: Some(limit),
            ..AgentConfig::default()
        },
    );
    let mut events = Vec::new();
    assert!(matches!(
        agent
            .run("stop the loop", &CancellationToken::new(), &mut |event| {
                events.push(event)
            })
            .await,
        Err(AgentError::ToolLoop { limit: actual }) if actual == limit
    ));
    assert_eq!(histories.lock().unwrap().len(), 2);
    assert_eq!(count.load(Ordering::SeqCst), 2);
    assert_eq!(agent.messages().len(), 7);
    for (message, expected_id) in agent.messages()[5..].iter().zip(["fresh-a", "fresh-b"]) {
        assert!(
            matches!(message, Message::Tool { call_id, output: ToolOutput::Error { code, .. } }
                if call_id == expected_id && code == "tool_loop")
        );
    }
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::ToolStarted { .. }))
            .count(),
        2
    );
    assert_eq!(terminal_events(&events), 1);
    assert_eq!(
        agent.state(),
        &RunState::Finished {
            outcome: RunOutcome::Failed
        }
    );
    // A failed run leaves closed history, but no detector state carries into
    // the next user turn, even when it requests the exact same operations.
    assert_eq!(
        agent
            .run("try again", &CancellationToken::new(), &mut |_| {})
            .await
            .unwrap(),
        RunOutcome::Completed
    );
    assert_eq!(count.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn tool_batch_changes_break_the_repeat_streak() {
    let a = ToolCall {
        arguments: json!({"path":"a"}),
        ..call("a")
    };
    let b = ToolCall {
        arguments: json!({"path":"b"}),
        ..call("b")
    };
    // Every dimension changes the requested operations: order, length,
    // arguments or name. None should be treated as the previous batch.
    for changed in [
        vec![b.clone(), a.clone()],
        vec![a.clone()],
        vec![
            a.clone(),
            ToolCall {
                arguments: json!({"path":"c"}),
                ..b.clone()
            },
        ],
        vec![
            a.clone(),
            ToolCall {
                name: "unregistered".into(),
                ..b.clone()
            },
        ],
    ] {
        let mut agent = Agent::new(
            script(vec![
                Ok(reply(vec![a.clone(), b.clone()])),
                Ok(reply(changed)),
                Ok(reply(vec![])),
            ]),
            registry(Arc::default(), false),
            AgentConfig {
                tool_repeat_limit: NonZeroU32::new(2),
                ..AgentConfig::default()
            },
        );
        assert_eq!(
            agent
                .run("make progress", &CancellationToken::new(), &mut |_| {})
                .await
                .unwrap(),
            RunOutcome::Completed
        );
    }
}

#[tokio::test]
async fn tool_repeat_guard_is_disabled_by_default_and_counts_batches() {
    assert!(AgentConfig::default().tool_repeat_limit.is_none());
    for limit in [None, NonZeroU32::new(2)] {
        let count = Arc::new(AtomicUsize::new(0));
        let mut replies = vec![Ok(reply((0..32).map(|id| call(&id.to_string())).collect()))];
        if limit.is_none() {
            replies.push(Ok(reply(vec![call("again")])));
            replies.push(Ok(reply(vec![call("again-again")])));
        }
        replies.push(Ok(reply(vec![])));
        let mut agent = Agent::new(
            script(replies),
            registry(count.clone(), false),
            AgentConfig {
                tool_repeat_limit: limit,
                ..AgentConfig::default()
            },
        );
        assert_eq!(
            agent
                .run(
                    "independent operations",
                    &CancellationToken::new(),
                    &mut |_| {}
                )
                .await
                .unwrap(),
            RunOutcome::Completed
        );
        assert_eq!(
            count.load(Ordering::SeqCst),
            if limit.is_none() { 34 } else { 32 }
        );
    }
}

#[tokio::test]
async fn tool_repeat_guard_debug_finish_closes_the_batch_without_tool_actions() {
    let count = Arc::new(AtomicUsize::new(0));
    let mut agent = Agent::new(
        script(vec![
            Ok(reply(vec![call("a"), call("b")])),
            Ok(reply(vec![])),
        ]),
        registry(count.clone(), false),
        AgentConfig {
            tool_repeat_limit: NonZeroU32::new(1),
            ..AgentConfig::default()
        },
    );
    let cancel = CancellationToken::new();
    let (controller, session) = debug_channel(&cancel);
    let mut snapshots = Vec::new();
    let mut events = Vec::new();
    let result = agent
        .run_debug("inspect blocked calls", session, &mut |event| {
            if let Event::DebugPaused { snapshot } = &event {
                snapshots.push((**snapshot).clone());
                controller
                    .try_send(DebugCommand::Step {
                        pause_id: snapshot.pause_id,
                    })
                    .unwrap();
            }
            events.push(event);
        })
        .await;
    assert!(matches!(result, Err(AgentError::ToolLoop { .. })));
    assert_eq!(count.load(Ordering::SeqCst), 0);
    assert_eq!(snapshots.len(), 2);
    assert!(matches!(snapshots[1].point, DebugPoint::AfterModel));
    assert_eq!(snapshots[1].pending_calls, vec![call("a"), call("b")]);
    assert_eq!(snapshots[1].messages.len(), 2);
    assert!(matches!(
        snapshots[1].next,
        DebugAction::Finish {
            outcome: RunOutcome::Failed
        }
    ));
    assert_eq!(agent.messages().len(), 4);
    assert!(agent.messages()[2..].iter().all(|message| matches!(
        message,
        Message::Tool { output: ToolOutput::Error { code, .. }, .. } if code == "tool_loop"
    )));
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, Event::ToolStarted { .. }))
    );
    assert_eq!(terminal_events(&events), 1);
    assert_eq!(
        agent
            .run("text only", &CancellationToken::new(), &mut |_| {})
            .await
            .unwrap(),
        RunOutcome::Completed
    );
}

#[tokio::test]
async fn cancellation_overrides_a_tripped_tool_repeat_guard_before_and_during_closure() {
    for cancel_after_results in 0..=2 {
        let count = Arc::new(AtomicUsize::new(0));
        let mut agent = Agent::new(
            script(vec![Ok(reply(vec![call("a"), call("b")]))]),
            registry(count.clone(), false),
            AgentConfig {
                tool_repeat_limit: NonZeroU32::new(1),
                ..AgentConfig::default()
            },
        );
        let cancel = CancellationToken::new();
        let (controller, session) = debug_channel(&cancel);
        let mut events = Vec::new();
        let mut committed_results = 0;
        let result = agent
            .run_debug("cancel blocked calls", session, &mut |event| {
                if let Event::DebugPaused { snapshot } = &event {
                    let should_cancel = cancel_after_results == 0
                        && matches!(snapshot.point, DebugPoint::AfterModel);
                    if should_cancel {
                        controller.cancel();
                    } else {
                        controller
                            .try_send(DebugCommand::Step {
                                pause_id: snapshot.pause_id,
                            })
                            .unwrap();
                    }
                }
                if matches!(
                    event,
                    Event::MessageCommitted {
                        message: Message::Tool { .. }
                    }
                ) {
                    committed_results += 1;
                    if committed_results == cancel_after_results {
                        controller.cancel();
                    }
                }
                events.push(event);
            })
            .await;
        assert_eq!(result.unwrap(), RunOutcome::Cancelled);
        assert_eq!(count.load(Ordering::SeqCst), 0);
        let results: Vec<_> = agent
            .messages()
            .iter()
            .filter_map(|message| match message {
                Message::Tool {
                    output: ToolOutput::Error { code, .. },
                    ..
                } => Some(code.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            results,
            match cancel_after_results {
                0 => vec!["cancelled", "cancelled"],
                1 => vec!["tool_loop", "cancelled"],
                _ => vec!["tool_loop", "tool_loop"],
            }
        );
        assert_eq!(terminal_events(&events), 1);
        assert!(
            events
                .iter()
                .all(|event| !matches!(event, Event::ToolStarted { .. }))
        );
    }
}
