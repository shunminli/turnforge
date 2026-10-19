//! Public control/Agent contracts; CLI/TCP delivery is covered separately.
use std::{
    collections::VecDeque,
    num::NonZeroU32,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use serde_json::json;
use turnforge::{
    Agent, AgentConfig, AgentError, CancellationToken, DebugCommand, DebugControlError, DebugPoint,
    Event, Model, RunOutcome, SteerControlError, debug_channel,
    message::{AssistantMessage, Message, ToolCall},
    model::{ModelDelta, ModelError, ModelRequest},
    tools::{Permissions, ToolRegistry},
};

type Histories = Arc<Mutex<Vec<Vec<Message>>>>;

struct Script {
    replies: Mutex<VecDeque<Result<AssistantMessage, ModelError>>>,
    histories: Histories,
}

#[async_trait]
impl Model for Script {
    async fn complete(
        &self,
        request: ModelRequest<'_>,
        _: &CancellationToken,
        emit: &mut (dyn FnMut(ModelDelta) + Send),
    ) -> Result<AssistantMessage, ModelError> {
        self.histories
            .lock()
            .unwrap()
            .push(request.messages.to_vec());
        emit(ModelDelta::Text {
            text: "observed".into(),
        });
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("extra model request")
    }
}

fn reply(ids: &[&str]) -> Result<AssistantMessage, ModelError> {
    Ok(AssistantMessage {
        text: "complete".into(),
        // Unknown tools still exercise real Registry execution and pairing.
        tool_calls: ids
            .iter()
            .map(|id| ToolCall {
                id: (*id).into(),
                name: "missing".into(),
                arguments: json!({}),
            })
            .collect(),
        usage: None,
    })
}

fn agent(
    replies: Vec<Result<AssistantMessage, ModelError>>,
    config: AgentConfig,
) -> (Agent<Script>, Histories) {
    let histories = Arc::default();
    (
        Agent::new(
            Script {
                replies: Mutex::new(replies.into()),
                histories: Arc::clone(&histories),
            },
            ToolRegistry::new(Permissions::default()),
            config,
        ),
        histories,
    )
}

fn receipts(events: &[Event]) -> Vec<(u64, bool)> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::SteerApplied { input_id } => Some((*input_id, true)),
            Event::SteerRejected {
                input_id: Some(input_id),
                ..
            } => Some((*input_id, false)),
            _ => None,
        })
        .collect()
}

#[test]
fn steer_admission_is_bounded_and_independent_of_debug_commands() {
    let cancel = CancellationToken::new();
    let (controller, session) = debug_channel(&cancel);
    assert_eq!(controller.try_steer(" \n"), Err(SteerControlError::Empty));
    assert_eq!(
        controller.try_steer("好".repeat(1366)),
        Err(SteerControlError::TooLarge)
    );
    for _ in 0..8 {
        controller.try_send(DebugCommand::Pause {}).unwrap();
    }
    assert_eq!(
        controller.try_send(DebugCommand::Pause {}),
        Err(DebugControlError::Full)
    );
    let mut ids = vec![
        controller
            .try_steer(format!("{}a", "好".repeat(1365)))
            .unwrap(),
    ];
    for _ in 1..8 {
        ids.push(controller.try_steer("queued").unwrap());
    }
    assert!(ids.windows(2).all(|pair| pair[0] != 0 && pair[0] < pair[1]));
    assert_eq!(controller.try_steer("full"), Err(SteerControlError::Full));
    assert!(!cancel.is_cancelled());
    drop(session);
    assert_eq!(
        controller.try_steer("closed"),
        Err(SteerControlError::Closed)
    );
    assert!(!cancel.is_cancelled());
    drop(controller);
    assert!(cancel.is_cancelled());
}

#[tokio::test]
async fn steer_waits_for_closed_tool_batch_and_preserves_observer_and_pauses() {
    let (mut agent, histories) =
        agent(vec![reply(&["a", "b"]), reply(&[])], AgentConfig::default());
    let cancel = CancellationToken::new();
    let (controller, session) = debug_channel(&cancel);
    let mut events = Vec::new();
    let mut id = None;
    let outcome = agent
        .run_debug("original", session, &mut |event| {
            if let Event::DebugPaused { snapshot } = &event {
                if snapshot.point == DebugPoint::AfterModel && snapshot.step == 1 {
                    id = Some(controller.try_steer("new directive").unwrap());
                    // Admission does not release this pause or alter its snapshot.
                    controller
                        .try_send(DebugCommand::Inspect {
                            pause_id: snapshot.pause_id,
                        })
                        .unwrap();
                }
                if snapshot.step == 1 {
                    assert_eq!(
                        snapshot
                            .messages
                            .iter()
                            .filter(|m| matches!(m, Message::User { .. }))
                            .count(),
                        1
                    );
                }
                controller
                    .try_send(DebugCommand::Step {
                        pause_id: snapshot.pause_id,
                    })
                    .unwrap();
            }
            events.push(event);
        })
        .await
        .unwrap();
    assert_eq!(outcome, RunOutcome::Completed);
    assert_eq!(receipts(&events), vec![(id.unwrap(), true)]);
    let histories = histories.lock().unwrap();
    assert_eq!(histories.len(), 2);
    assert!(matches!(histories[1].as_slice(), [
        Message::User { .. }, Message::Assistant { .. },
        Message::Tool { call_id: a, .. }, Message::Tool { call_id: b, .. }, Message::User { text },
    ] if a == "a" && b == "b" && text == "new directive"));
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::ModelDelta { .. }))
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::ToolStarted { .. }))
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::RunStarted))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::RunFinished { .. }))
            .count(),
        1
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::DebugSnapshot { .. }))
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::SteerQueued { .. }))
    );
    assert_eq!(controller.try_steer("late"), Err(SteerControlError::Closed));
}

#[tokio::test]
async fn applied_steer_resets_repeat_streak_without_resetting_model_steps() {
    let config = AgentConfig {
        max_steps: NonZeroU32::new(3).unwrap(),
        tool_repeat_limit: NonZeroU32::new(2),
        ..AgentConfig::default()
    };
    let (mut agent, histories) = agent(vec![reply(&["a"]), reply(&["b"]), reply(&[])], config);
    let (controller, session) = debug_channel(&CancellationToken::new());
    let mut events = Vec::new();
    assert_eq!(
        agent
            .run_debug("original", session, &mut |event| {
                if let Event::DebugPaused { snapshot } = &event {
                    if matches!(snapshot.point, DebugPoint::AfterTool { .. }) && snapshot.step == 1
                    {
                        controller
                            .try_steer("repeat the operation for the revised task")
                            .unwrap();
                    }
                    controller
                        .try_send(DebugCommand::Step {
                            pause_id: snapshot.pause_id,
                        })
                        .unwrap();
                }
                events.push(event);
            })
            .await
            .unwrap(),
        RunOutcome::Completed
    );
    assert_eq!(histories.lock().unwrap().len(), 3);
    assert_eq!(
        events
            .iter()
            .filter_map(|event| match event {
                Event::StepStarted { step } => Some(*step),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::ToolStarted { .. }))
            .count(),
        2
    );
    assert_eq!(receipts(&events), vec![(1, true)]);
}

#[tokio::test]
async fn terminal_paths_reject_pending_inputs_without_creating_followup_requests() {
    for case in [
        "completed",
        "step_limit",
        "model_error",
        "request_budget",
        "cancelled",
    ] {
        let response = match case {
            "step_limit" => reply(&["a", "b"]),
            "model_error" => Err(ModelError::Protocol("fixture failure".into())),
            "request_budget" => Err(ModelError::RequestTooLarge {
                limit: 1,
                actual: 2,
            }),
            _ => reply(&[]),
        };
        let config = AgentConfig {
            max_steps: NonZeroU32::new(1).unwrap(),
            ..AgentConfig::default()
        };
        let (mut agent, histories) = agent(vec![response], config);
        let cancel = CancellationToken::new();
        let (controller, session) = debug_channel(&cancel);
        let mut ids = Vec::new();
        let mut events = Vec::new();
        let outcome = agent
            .run_debug("original", session, &mut |event| {
                if let Event::DebugPaused { snapshot } = &event {
                    controller
                        .try_send(DebugCommand::Continue {
                            pause_id: snapshot.pause_id,
                        })
                        .unwrap();
                }
                if matches!(event, Event::StepStarted { .. }) {
                    ids.push(controller.try_steer("pending one").unwrap());
                    ids.push(controller.try_steer("pending two").unwrap());
                    if case == "cancelled" {
                        cancel.cancel();
                    }
                }
                if matches!(event, Event::SteerRejected { .. }) {
                    // Admission must already be closed before terminal callbacks.
                    assert_eq!(
                        controller.try_steer("too late"),
                        Err(SteerControlError::Closed)
                    );
                }
                events.push(event);
            })
            .await;
        match case {
            "completed" => assert_eq!(outcome.unwrap(), RunOutcome::Completed),
            "step_limit" => assert_eq!(outcome.unwrap(), RunOutcome::StepLimit),
            "cancelled" => assert_eq!(outcome.unwrap(), RunOutcome::Cancelled),
            _ => assert!(matches!(outcome, Err(AgentError::Model(_)))),
        }
        assert_eq!(
            receipts(&events),
            ids.into_iter().map(|id| (id, false)).collect::<Vec<_>>(),
            "{case}"
        );
        assert_eq!(
            histories.lock().unwrap().len(),
            usize::from(case != "cancelled"),
            "{case}"
        );
        assert_eq!(
            agent
                .messages()
                .iter()
                .filter(|m| matches!(m, Message::User { .. }))
                .count(),
            1,
            "{case}"
        );
        assert!(
            matches!(events.last(), Some(Event::RunFinished { .. })),
            "{case}"
        );
    }
}

#[tokio::test]
async fn cancellation_after_one_commit_preserves_its_receipt_and_rejects_the_rest() {
    let (mut agent, histories) = agent(vec![], AgentConfig::default());
    let cancel = CancellationToken::new();
    let (controller, session) = debug_channel(&cancel);
    let first = controller.try_steer("first").unwrap();
    let second = controller.try_steer("second").unwrap();
    let mut events = Vec::new();
    assert_eq!(agent.run_debug("original", session, &mut |event| {
        if let Event::DebugPaused { snapshot } = &event {
            controller.try_send(DebugCommand::Continue { pause_id: snapshot.pause_id }).unwrap();
        }
        if matches!(&event, Event::MessageCommitted { message: Message::User { text } } if text == "first") {
            cancel.cancel();
        }
        events.push(event);
    }).await.unwrap(), RunOutcome::Cancelled);
    assert_eq!(receipts(&events), vec![(first, true), (second, false)]);
    assert!(histories.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_rejected_run_closes_already_admitted_inputs_without_starting_a_run() {
    let (mut agent, histories) = agent(vec![], AgentConfig::default());
    let (controller, session) = debug_channel(&CancellationToken::new());
    let id = controller.try_steer("already queued").unwrap();
    let mut events = Vec::new();
    assert!(matches!(
        agent
            .run_debug(" ", session, &mut |event| events.push(event))
            .await,
        Err(AgentError::EmptyInput)
    ));
    assert_eq!(receipts(&events), vec![(id, false)]);
    assert_eq!(events.len(), 1);
    assert!(histories.lock().unwrap().is_empty());
    assert_eq!(controller.try_steer("late"), Err(SteerControlError::Closed));
}

#[tokio::test]
async fn event_callback_refills_cannot_starve_the_model_or_lose_terminal_receipts() {
    let config = AgentConfig {
        max_steps: NonZeroU32::new(1).unwrap(),
        ..AgentConfig::default()
    };
    let (mut agent, histories) = agent(vec![reply(&[])], config);
    let (controller, session) = debug_channel(&CancellationToken::new());
    let mut ids: Vec<_> = (0..8)
        .map(|_| controller.try_steer("initial").unwrap())
        .collect();
    let mut events = Vec::new();
    assert_eq!(
        agent
            .run_debug("original", session, &mut |event| {
                if let Event::DebugPaused { snapshot } = &event {
                    controller
                        .try_send(DebugCommand::Continue {
                            pause_id: snapshot.pause_id,
                        })
                        .unwrap();
                }
                if matches!(event, Event::SteerApplied { .. }) {
                    ids.push(controller.try_steer("refilled").unwrap());
                }
                events.push(event);
            })
            .await
            .unwrap(),
        RunOutcome::Completed
    );
    assert_eq!(histories.lock().unwrap().len(), 1);
    let expected: Vec<_> = ids
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, index < 8))
        .collect();
    assert_eq!(receipts(&events), expected);
    assert_eq!(ids.len(), 16);
}

#[tokio::test]
async fn concurrent_admission_and_finish_preserve_every_successful_input_receipt() {
    struct AdmissionRace {
        start: std::sync::mpsc::Sender<()>,
        admitted: Arc<tokio::sync::Notify>,
    }
    #[async_trait]
    impl Model for AdmissionRace {
        async fn complete(
            &self,
            _: ModelRequest<'_>,
            _: &CancellationToken,
            _: &mut (dyn FnMut(ModelDelta) + Send),
        ) -> Result<AssistantMessage, ModelError> {
            self.start.send(()).unwrap();
            self.admitted.notified().await;
            reply(&[])
        }
    }

    let (start, ready) = std::sync::mpsc::channel();
    let admitted = Arc::new(tokio::sync::Notify::new());
    let (controller, session) = debug_channel(&CancellationToken::new());
    let controller = Arc::new(controller);
    let initial_id = controller.try_steer("before model").unwrap();
    let producer = {
        let controller = Arc::clone(&controller);
        let admitted = Arc::clone(&admitted);
        std::thread::spawn(move || {
            ready.recv().unwrap();
            let mut successful = vec![controller.try_steer("model active").unwrap()];
            admitted.notify_one();
            for _ in 0..256 {
                match controller.try_steer("race with close") {
                    Ok(id) => successful.push(id),
                    Err(SteerControlError::Full) => {}
                    Err(SteerControlError::Closed) => break,
                    other => panic!("unexpected admission result: {other:?}"),
                }
            }
            successful
        })
    };
    let mut agent = Agent::new(
        AdmissionRace { start, admitted },
        ToolRegistry::new(Permissions::default()),
        AgentConfig::default(),
    );
    let mut events = Vec::new();
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        agent.run_debug("original", session, &mut |event| {
            if let Event::DebugPaused { snapshot } = &event {
                controller
                    .try_send(DebugCommand::Continue {
                        pause_id: snapshot.pause_id,
                    })
                    .unwrap();
            }
            events.push(event);
        }),
    )
    .await
    .unwrap()
    .unwrap();
    let mut accepted = vec![initial_id];
    accepted.extend(producer.join().unwrap());
    accepted.sort_unstable();
    let mut terminal_ids: Vec<_> = receipts(&events).into_iter().map(|(id, _)| id).collect();
    terminal_ids.sort_unstable();
    assert_eq!(outcome, RunOutcome::Completed);
    assert_eq!(terminal_ids, accepted);
    assert!(accepted.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(receipts(&events)[0], (initial_id, true));
    assert!(
        receipts(&events)
            .iter()
            .skip(1)
            .all(|(_, applied)| !applied)
    );
    assert_eq!(
        controller.try_steer("finished"),
        Err(SteerControlError::Closed)
    );
}
