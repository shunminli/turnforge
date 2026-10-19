//! Real HTTP framing -> production provider -> Agent/file boundaries, without an LLM.
use std::{future::pending, time::Duration};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
    task::JoinHandle,
};
use turnforge::{
    Agent, AgentConfig, AgentError, CancellationToken, Event, Model, RunOutcome,
    message::Message,
    model::{ModelError, ModelRequest},
    openai::OpenAiModel,
    tools::{FileOperation, FileTool, Permissions, ToolRegistry, Workspace},
};

struct Server {
    url: String,
    stop: oneshot::Sender<()>,
    task: JoinHandle<usize>,
}

impl Server {
    async fn start(header_delay: Duration, frames: Vec<(Duration, String)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            socket.set_nodelay(true).unwrap();
            let mut request = Vec::new();
            let header_end = loop {
                let mut buffer = [0; 4096];
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
                if let Some(index) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    break index + 4;
                }
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            assert!(headers.starts_with("POST /v1/chat/completions HTTP/1.1"));
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            while request.len() < header_end + length {
                let mut buffer = [0; 4096];
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
            }
            let body: Value = serde_json::from_slice(&request[header_end..]).unwrap();
            assert_eq!(body["stream"], true);
            let script = async {
                tokio::time::sleep(header_delay).await;
                if socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n")
                    .await
                    .is_ok()
                {
                    for (delay, frame) in frames {
                        tokio::time::sleep(delay).await;
                        if socket.write_all(frame.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                }
                // Keep the response open until its client has returned. EOF must
                // not be the reason a stalled request finishes.
                pending::<()>().await;
            };
            tokio::select! {
                _ = script => unreachable!(),
                _ = stopped => {},
            }
            drop(socket);
            let mut requests = 1;
            while let Ok(Ok((socket, _))) =
                tokio::time::timeout(Duration::from_millis(500), listener.accept()).await
            {
                requests += 1;
                drop(socket);
            }
            requests
        });
        Self { url, stop, task }
    }

    async fn finish(self) -> usize {
        let _ = self.stop.send(());
        self.task.await.unwrap()
    }

    fn model(&self, total: Duration) -> OpenAiModel {
        OpenAiModel::new_local(&self.url, "fixture-model", total).unwrap()
    }
}

fn event(delta: Value, finish: Option<&str>) -> String {
    format!(
        "data: {}\n\n",
        json!({"choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
    )
}

fn request() -> ModelRequest<'static> {
    ModelRequest {
        system: "test instruction",
        messages: &[],
        tools: &[],
    }
}

#[tokio::test]
async fn stalled_streams_never_commit_or_execute_provisional_tools_or_retry() {
    let tool = event(
        json!({"tool_calls":[{"index":0,"id":"call-one","type":"function",
            "function":{"name":"write_file","arguments":"{\"path\":\"effect.txt\",\"content\":\"must not be written\"}"}}]}),
        Some("tool_calls"),
    );
    for first in [None, Some(tool)] {
        let has_delta = first.is_some();
        let server = Server::start(
            Duration::ZERO,
            first
                .into_iter()
                .map(|body| (Duration::ZERO, body))
                .collect(),
        )
        .await;
        let model = server
            .model(Duration::from_secs(4))
            .with_http_retries(3)
            .unwrap()
            .with_stream_idle_timeout(Duration::from_millis(400))
            .unwrap();
        let root = tempfile::tempdir().unwrap();
        let mut tools = ToolRegistry::new(Permissions {
            allow_write: true,
            ..Permissions::default()
        });
        tools
            .register(FileTool::new(
                Workspace::new(root.path()).unwrap(),
                FileOperation::Write,
            ))
            .unwrap();
        let mut agent = Agent::new(model, tools, AgentConfig::default());
        let mut events = Vec::new();
        let result = tokio::time::timeout(
            Duration::from_secs(6),
            agent.run("fixture task", &CancellationToken::new(), &mut |e| {
                events.push(e)
            }),
        )
        .await
        .unwrap();
        assert!(matches!(
            result,
            Err(AgentError::Model(ModelError::StreamIdleTimeout))
        ));
        assert_eq!(agent.messages().len(), 1);
        assert!(matches!(agent.messages()[0], Message::User { .. }));
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Event::ToolStarted { .. }))
        );
        assert_eq!(
            events.iter().any(|e| matches!(e, Event::ModelDelta { .. })),
            has_delta
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(
                    e,
                    Event::RunFinished {
                        outcome: RunOutcome::Failed
                    }
                ))
                .count(),
            1
        );
        assert!(!root.path().join("effect.txt").exists());
        assert_eq!(server.finish().await, 1);
    }
}

#[tokio::test]
async fn complete_events_reset_idle_and_the_default_has_no_idle_limit() {
    for idle in [Some(Duration::from_millis(450)), None] {
        let delay = if idle.is_some() { 150 } else { 600 };
        let mut frames = vec![(
            Duration::from_millis(delay),
            event(json!({"content":"one"}), None),
        )];
        if idle.is_some() {
            frames.extend((0..3).map(|_| {
                (
                    Duration::from_millis(150),
                    event(json!({"content":"two"}), None),
                )
            }));
        }
        frames.push((Duration::from_millis(150), event(json!({}), Some("stop"))));
        frames.push((Duration::from_millis(150), "data: [DONE]\n\n".into()));
        let server = Server::start(Duration::ZERO, frames).await;
        let mut model = server.model(Duration::from_secs(4));
        if let Some(idle) = idle {
            model = model.with_stream_idle_timeout(idle).unwrap();
        }
        let message = model
            .complete(request(), &CancellationToken::new(), &mut |_| {})
            .await
            .unwrap();
        assert_eq!(
            message.text,
            if idle.is_some() {
                "onetwotwotwo"
            } else {
                "one"
            }
        );
        assert_eq!(server.finish().await, 1);
    }
}

#[tokio::test]
async fn comments_and_partial_frames_cannot_keep_a_stalled_stream_alive() {
    for partial in [false, true] {
        let mut frames = if partial {
            vec![(Duration::ZERO, "data: {\"choices\":[".into())]
        } else {
            Vec::new()
        };
        frames.extend((0..15).map(|_| {
            (
                Duration::from_millis(80),
                if partial { " " } else { ": heartbeat\n\n" }.into(),
            )
        }));
        frames.push((Duration::ZERO, if partial {
            "{\"index\":0,\"delta\":{\"content\":\"too late\"},\"finish_reason\":\"stop\"}]}\n\n".into()
        } else {
            event(json!({"content":"too late"}), Some("stop"))
        }));
        frames.push((Duration::ZERO, "data: [DONE]\n\n".into()));
        let server = Server::start(Duration::ZERO, frames).await;
        let result = server
            .model(Duration::from_secs(4))
            .with_stream_idle_timeout(Duration::from_millis(400))
            .unwrap()
            .complete(request(), &CancellationToken::new(), &mut |_| {})
            .await;
        assert!(matches!(result, Err(ModelError::StreamIdleTimeout)));
        assert_eq!(server.finish().await, 1);
    }
}

#[tokio::test]
async fn request_timeout_still_bounds_headers_and_an_active_stream() {
    for waiting_headers in [true, false] {
        let server = Server::start(
            if waiting_headers {
                Duration::from_millis(900)
            } else {
                Duration::ZERO
            },
            (0..20)
                .map(|_| {
                    (
                        Duration::from_millis(80),
                        event(json!({"content":"active"}), None),
                    )
                })
                .collect(),
        )
        .await;
        let model = server
            .model(Duration::from_millis(400))
            .with_stream_idle_timeout(if waiting_headers {
                Duration::from_millis(150)
            } else {
                Duration::from_millis(800)
            })
            .unwrap();
        let result = model
            .complete(request(), &CancellationToken::new(), &mut |_| {})
            .await;
        if waiting_headers {
            assert!(matches!(result, Err(ModelError::Transport(_))));
        } else {
            assert!(matches!(result, Err(ModelError::Protocol(_))));
        }
        assert_eq!(server.finish().await, 1);
    }
}

#[tokio::test]
async fn invalid_idle_limits_and_cancellation_preserve_existing_boundaries() {
    for idle in [Duration::ZERO, Duration::MAX] {
        assert!(matches!(
            OpenAiModel::new_local("http://127.0.0.1:1/v1", "fixture", Duration::from_secs(3))
                .unwrap()
                .with_stream_idle_timeout(idle),
            Err(ModelError::Configuration(_))
        ));
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let model = OpenAiModel::new_local(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "fixture",
        Duration::from_secs(3),
    )
    .unwrap()
    .with_stream_idle_timeout(Duration::from_millis(400))
    .unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        model.complete(request(), &cancel, &mut |_| {}).await,
        Err(ModelError::Cancelled)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );

    let server = Server::start(
        Duration::ZERO,
        vec![(
            Duration::ZERO,
            event(json!({"content":"provisional"}), None),
        )],
    )
    .await;
    let model = server
        .model(Duration::from_secs(3))
        .with_stream_idle_timeout(Duration::from_millis(400))
        .unwrap();
    let cancel = CancellationToken::new();
    let mut deltas = 0;
    let result = model
        .complete(request(), &cancel, &mut |_| {
            deltas += 1;
            cancel.cancel();
        })
        .await;
    assert!(matches!(result, Err(ModelError::Cancelled)));
    assert_eq!(deltas, 1);
    assert_eq!(server.finish().await, 1);
}
