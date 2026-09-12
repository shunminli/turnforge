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
    Agent, AgentConfig, CancellationToken, Event, Model, RunOutcome, RunState,
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
