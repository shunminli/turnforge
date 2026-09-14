//! Real CLI and HTTP protocol tests for the isolated local Harness lab.
//! The fixture supplies model choices; only the CLI executes file tools.
use std::{
    path::PathBuf,
    process::{Output, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    process::{Child, ChildStdin, ChildStdout, Command},
    task::JoinHandle,
    time::timeout,
};

const LIMIT: Duration = Duration::from_secs(12);

#[derive(Debug)]
struct Request {
    path: String,
    headers: String,
    body: Value,
}

struct Reply {
    status: u16,
    headers: String,
    body: String,
}

impl Reply {
    fn json(body: Value) -> Self {
        Self {
            status: 200,
            headers: "Content-Type: application/json\r\n".into(),
            body: body.to_string(),
        }
    }

    fn sse(delta: Value, finish: &str) -> Self {
        Self {
            status: 200,
            headers: "Content-Type: text/event-stream\r\n".into(),
            body: format!(
                "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                json!({"choices":[{"index":0,"delta":delta,"finish_reason":null}]}),
                json!({"choices":[{"index":0,"delta":{},"finish_reason":finish}]})
            ),
        }
    }

    fn text(text: &str) -> Self {
        Self::sse(json!({"content":text}), "stop")
    }

    fn tool(name: &str, arguments: Value) -> Self {
        Self::sse(
            json!({"tool_calls":[{"index":0,"id":"lab-call","type":"function","function":{"name":name,"arguments":arguments.to_string()}}]}),
            "tool_calls",
        )
    }
}

fn lock() -> Value {
    serde_json::from_str(include_str!("../dev/local-llm.lock.json")).unwrap()
}

fn baseline(request: &Request) -> Option<Reply> {
    match request.path.as_str() {
        "/api/version" => Some(Reply::json(json!({"version":lock()["ollama_version"]}))),
        "/api/tags" => Some(Reply::json(json!({"models":[lock()["test_model"]]}))),
        "/v1/chat/completions" => None,
        path => panic!("unexpected endpoint: {path}"),
    }
}

// One owned task holds the listener and sequential fixture state. A normal
// finish joins it; Drop aborts it only when a test assertion already failed.
struct Server {
    url: String,
    model_calls: Arc<AtomicUsize>,
    task: Option<JoinHandle<Vec<Request>>>,
}

impl Server {
    async fn start(
        count: usize,
        mut respond: impl FnMut(&Request) -> Reply + Send + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let model_calls = Arc::new(AtomicUsize::new(0));
        let observed = model_calls.clone();
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for _ in 0..count {
                let (mut socket, _) = timeout(LIMIT, listener.accept()).await.unwrap().unwrap();
                let request = timeout(LIMIT, read_request(&mut socket)).await.unwrap();
                if request.path == "/v1/chat/completions" {
                    observed.fetch_add(1, Ordering::SeqCst);
                }
                let reply = respond(&request);
                requests.push(request);
                let head = format!(
                    "HTTP/1.1 {} Fixture\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    reply.status,
                    reply.headers,
                    reply.body.len()
                );
                socket.write_all(head.as_bytes()).await.unwrap();
                socket.write_all(reply.body.as_bytes()).await.unwrap();
            }
            // A rejected preflight must not follow a redirect or call the model.
            assert!(
                timeout(Duration::from_millis(100), listener.accept())
                    .await
                    .is_err(),
                "unexpected request after the expected protocol sequence"
            );
            requests
        });
        Self {
            url,
            model_calls,
            task: Some(task),
        }
    }

    async fn finish(mut self) -> Vec<Request> {
        timeout(LIMIT, self.task.as_mut().unwrap())
            .await
            .unwrap()
            .unwrap()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

async fn read_request(socket: &mut TcpStream) -> Request {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    let end = loop {
        let n = socket.read(&mut buffer).await.unwrap();
        assert_ne!(n, 0);
        bytes.extend_from_slice(&buffer[..n]);
        assert!(
            bytes.len() < 128 * 1024,
            "fixture request exceeded its bound"
        );
        if let Some(pos) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
    let first = headers.lines().next().unwrap();
    let path = first.split_whitespace().nth(1).unwrap().to_owned();
    let length = headers
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    assert!(length < 128 * 1024);
    while bytes.len() < end + length {
        let n = socket.read(&mut buffer).await.unwrap();
        assert_ne!(n, 0);
        bytes.extend_from_slice(&buffer[..n]);
    }
    let body = if length == 0 {
        Value::Null
    } else {
        serde_json::from_slice(&bytes[end..end + length]).unwrap()
    };
    Request {
        path,
        headers,
        body,
    }
}

fn command(url: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_turnforge"));
    command
        .args(["lab", "--ollama-url", url])
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

async fn output(mut command: Command) -> Output {
    timeout(LIMIT, command.output())
        .await
        .expect("lab CLI exceeded its test deadline")
        .unwrap()
}

fn assert_status(output: &Output, code: i32, label: &str) -> String {
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(code), "{stdout}\n{stderr}");
    assert!(
        stdout.contains(label),
        "missing {label}: {stdout}\n{stderr}"
    );
    if code != 0 {
        assert!(
            !stdout.contains("[PASS]"),
            "failure was advertised as passing"
        );
    }
    stdout
}

struct Interactive {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    text: String,
}

impl Interactive {
    fn start(url: &str, args: &[&str]) -> Self {
        let mut child = command(url)
            .args(args)
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        Self {
            input: child.stdin.take(),
            output: BufReader::new(child.stdout.take().unwrap()),
            child,
            text: String::new(),
        }
    }

    async fn until(&mut self, marker: &str) {
        timeout(LIMIT, async {
            loop {
                let mut line = String::new();
                assert_ne!(
                    self.output.read_line(&mut line).await.unwrap(),
                    0,
                    "EOF awaiting {marker}: {}",
                    self.text
                );
                self.text.push_str(&line);
                if line.contains(marker) {
                    return;
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timeout awaiting {marker}: {}", self.text));
    }

    async fn send(&mut self, line: &str) {
        self.input
            .as_mut()
            .unwrap()
            .write_all(line.as_bytes())
            .await
            .unwrap();
    }

    async fn snapshot(&mut self, id: u64) -> Value {
        self.send("i\n").await;
        self.until(&format!("[lab snapshot #{id}]")).await;
        let mut json = String::new();
        timeout(LIMIT, async {
            loop {
                let mut line = String::new();
                assert_ne!(self.output.read_line(&mut line).await.unwrap(), 0);
                self.text.push_str(&line);
                json.push_str(&line);
                if let Ok(value) = serde_json::from_str::<Value>(&json) {
                    return value;
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("incomplete snapshot JSON: {json}"))
    }

    async fn finish(mut self, code: i32, label: &str) -> String {
        let mut stderr = String::new();
        let mut stderr_pipe = self.child.stderr.take().unwrap();
        let (status, out, err) = timeout(LIMIT, async {
            tokio::join!(
                self.child.wait(),
                self.output.read_to_string(&mut self.text),
                stderr_pipe.read_to_string(&mut stderr)
            )
        })
        .await
        .unwrap_or_else(|_| panic!("lab did not finish with stdin open: {}", self.text));
        out.unwrap();
        err.unwrap();
        assert_eq!(
            status.unwrap().code(),
            Some(code),
            "{}\n{stderr}",
            self.text
        );
        assert!(self.text.contains(label), "{}\n{stderr}", self.text);
        if code != 0 {
            assert!(!self.text.contains("[PASS]"));
        }
        self.text
    }
}

fn workspace(system: &str) -> PathBuf {
    let root = system
        .split("Workspace root: ")
        .nth(1)
        .unwrap()
        .split(". File tool paths")
        .next()
        .unwrap();
    PathBuf::from(root)
}

fn read_reply(request: &Request) -> Reply {
    if let Some(reply) = baseline(request) {
        return reply;
    }
    let messages = request.body["messages"].as_array().unwrap();
    if let Some(tool) = messages
        .iter()
        .rev()
        .find(|message| message["role"] == "tool")
    {
        let result: Value = serde_json::from_str(tool["content"].as_str().unwrap()).unwrap();
        assert_eq!(result["status"], "ok");
        return Reply::text(result["data"]["text"].as_str().unwrap());
    }
    Reply::tool("read_file", json!({"path":"marker.txt"}))
}

#[tokio::test]
async fn lab_auto_chat_ignores_stdin_and_inherited_cloud_configuration() {
    let server = Server::start(3, |request| {
        baseline(request).unwrap_or_else(|| Reply::text("Hello from the fixture."))
    })
    .await;
    let mut command = command(&server.url);
    command
        .args(["--case", "chat", "--auto"])
        .env("TURNFORGE_API_KEY", "synthetic-turnforge-secret")
        .env("OPENAI_API_KEY", "synthetic-openai-secret")
        .env("TURNFORGE_MODEL", "incorrect-cloud-model")
        .env("TURNFORGE_BASE_URL", "https://example.invalid/v1")
        .env("HTTP_PROXY", "http://127.0.0.1:1")
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env("ALL_PROXY", "http://127.0.0.1:1")
        .env("http_proxy", "http://127.0.0.1:1")
        .env("https_proxy", "http://127.0.0.1:1")
        .env("all_proxy", "http://127.0.0.1:1");
    let stdout = assert_status(&output(command).await, 0, "[PASS]");
    assert!(stdout.contains("[lab paused #1]"));
    let requests = server.finish().await;
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[2].body["model"], lock()["test_model"]["name"]);
    for request in &requests {
        assert!(
            !request
                .headers
                .to_ascii_lowercase()
                .contains("authorization:")
        );
        assert!(!format!("{request:?}").contains("synthetic-"));
    }
    assert!(!workspace(requests[2].body["messages"][0]["content"].as_str().unwrap()).exists());
}

#[tokio::test]
async fn lab_default_read_shortcuts_pause_inspect_and_finish_with_stdin_open() {
    let server = Server::start(4, read_reply).await;
    let mut process = Interactive::start(&server.url, &["--lesson", "loop"]);
    process.until("[lab paused #1]").await;
    assert_eq!(server.model_calls.load(Ordering::SeqCst), 0);
    let initial = process.snapshot(1).await;
    assert_eq!(initial["point"]["kind"], "before_model");
    let root = workspace(initial["system"].as_str().unwrap());
    let marker_file = std::fs::read_to_string(root.join("marker.txt")).unwrap();
    assert!(!initial.to_string().contains(marker_file.trim()));
    process.send("l\n").await;
    process.until("[learn loop]").await;
    assert_eq!(server.model_calls.load(Ordering::SeqCst), 0);
    process.send("n\n").await;
    process.until("[lab paused #2]").await;
    assert_eq!(server.model_calls.load(Ordering::SeqCst), 1);
    let preview = process.snapshot(2).await;
    assert_eq!(preview["next"]["kind"], "tool");
    assert_eq!(preview["pending_calls"][0]["name"], "read_file");
    assert!(
        !preview["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["role"] == "tool")
    );
    process
        .send("{\"command\":\"step\",\"pause_id\":1}\n")
        .await;
    process.until("[lab rejected]").await;
    assert_eq!(server.model_calls.load(Ordering::SeqCst), 1);
    process.send("\n").await;
    process.until("[lab paused #3]").await;
    assert_eq!(server.model_calls.load(Ordering::SeqCst), 1);
    let after = process.snapshot(3).await;
    assert_eq!(after["point"]["kind"], "after_tool");
    assert!(after["messages"].to_string().contains(marker_file.trim()));
    process.send("c\n").await;
    let stdout = process.finish(0, "[PASS]").await;
    assert!(stdout.contains("read_file"));
    assert!(
        stdout.contains("src/agent.rs"),
        "learning hints absent: {stdout}"
    );
    assert!(!root.exists(), "temporary lab workspace survived CLI exit");
    let requests = server.finish().await;
    assert_eq!(requests.len(), 4);
    let names: Vec<_> = requests[2].body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["list_files", "read_file"]);
}

#[tokio::test]
async fn lab_write_pass_requires_the_real_file_to_match_the_independent_oracle() {
    for content in ["turnforge-lab-write-ok", "incorrect-content"] {
        let server = Server::start(4, move |request| {
            if let Some(reply) = baseline(request) {
                return reply;
            }
            if request.body["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["role"] == "tool")
            {
                let system = request.body["messages"][0]["content"].as_str().unwrap();
                assert_eq!(
                    std::fs::read_to_string(workspace(system).join("result.txt")).unwrap(),
                    content
                );
                Reply::text("The write is finished.")
            } else {
                Reply::tool("write_file", json!({"path":"result.txt","content":content}))
            }
        })
        .await;
        let mut command = command(&server.url);
        command.args(["--case", "write", "--auto"]);
        let correct = content == "turnforge-lab-write-ok";
        assert_status(
            &output(command).await,
            if correct { 0 } else { 1 },
            if correct { "[PASS]" } else { "[FAIL]" },
        );
        let requests = server.finish().await;
        let root = workspace(requests[2].body["messages"][0]["content"].as_str().unwrap());
        assert!(!root.exists());
        assert!(
            !requests[2].body["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["function"]["name"] == "bash")
        );
    }
}

#[tokio::test]
async fn lab_read_completed_without_a_tool_is_not_a_pass() {
    let server = Server::start(3, |request| {
        baseline(request).unwrap_or_else(|| Reply::text("I have read the file successfully."))
    })
    .await;
    let mut command = command(&server.url);
    command.arg("--auto");
    assert_status(&output(command).await, 1, "[FAIL]");
    let requests = server.finish().await;
    assert!(!workspace(requests[2].body["messages"][0]["content"].as_str().unwrap()).exists());
}

#[tokio::test]
async fn lab_preflight_rejects_drift_and_redirects_before_model_calls() {
    for failure in ["version", "missing", "digest", "redirect"] {
        let count = if matches!(failure, "version" | "redirect") {
            1
        } else {
            2
        };
        let server = Server::start(count, move |request| {
            if failure == "version" {
                assert_eq!(request.path, "/api/version");
                Reply::json(json!({"version":"0.0.0"}))
            } else if failure == "redirect" {
                Reply { status: 302, headers: "Location: /redirect-target\r\n".into(), body: String::new() }
            } else if request.path == "/api/tags" {
                if failure == "missing" {
                    Reply::json(json!({"models":[]}))
                } else {
                    Reply::json(json!({"models":[{"name":lock()["test_model"]["name"],"digest":"wrong-digest"}]}))
                }
            } else {
                baseline(request).expect("preflight must not call a model")
            }
        }).await;
        let mut command = command(&server.url);
        command.arg("--auto");
        let result = output(command).await;
        assert_eq!(result.status.code(), Some(1), "{failure}: {result:?}");
        assert!(!String::from_utf8_lossy(&result.stdout).contains("[PASS]"));
        assert!(!result.stderr.is_empty());
        assert_eq!(server.model_calls.load(Ordering::SeqCst), 0);
        assert_eq!(server.finish().await.len(), count);
    }
}

#[tokio::test]
async fn lab_rejects_remote_or_secret_origins_and_reports_unreachable_ollama() {
    for url in [
        "http://example.invalid:11434",
        "https://example.invalid",
        "http://user:synthetic-secret@127.0.0.1:11434",
        "http://127.0.0.1:11434/v1",
        "http://127.0.0.1:11434?key=synthetic-secret",
    ] {
        let mut command = command(url);
        command.arg("--auto");
        let result = output(command).await;
        assert_eq!(result.status.code(), Some(1), "{result:?}");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!text.contains("synthetic-secret"));
        assert!(!text.contains("[PASS]"));
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let mut command = command(&url);
    command.arg("--auto");
    let result = output(command).await;
    assert_eq!(result.status.code(), Some(1));
    assert!(!result.stderr.is_empty());
}

#[tokio::test]
async fn lab_eof_and_quit_cancel_without_running_a_model() {
    for eof in [true, false] {
        let server = Server::start(2, |request| baseline(request).unwrap()).await;
        let mut process = Interactive::start(&server.url, &[]);
        process.until("[lab paused #1]").await;
        let initial = process.snapshot(1).await;
        let root = workspace(initial["system"].as_str().unwrap());
        if eof {
            drop(process.input.take());
        } else {
            process.send("q\n").await;
        }
        process.finish(130, "[CANCELLED]").await;
        assert_eq!(server.model_calls.load(Ordering::SeqCst), 0);
        assert_eq!(server.finish().await.len(), 2);
        assert!(!root.exists());
    }
}

#[tokio::test]
async fn learn_catalog_and_lessons_need_no_llm() {
    for lesson in [
        None,
        Some("loop"),
        Some("context"),
        Some("tools"),
        Some("control"),
        Some("io"),
        Some("regression"),
    ] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_turnforge"));
        command
            .arg("learn")
            .env_clear()
            .env("TURNFORGE_BASE_URL", "http://127.0.0.1:1")
            .env("OPENAI_API_KEY", "synthetic-secret")
            .stdin(Stdio::null())
            .kill_on_drop(true);
        if let Some(lesson) = lesson {
            command.arg(lesson);
        }
        let result = output(command).await;
        assert!(result.status.success(), "{result:?}");
        let stdout = String::from_utf8(result.stdout).unwrap();
        if let Some(lesson) = lesson {
            assert!(stdout.contains(&format!("[learn {lesson}]")), "{stdout}");
            assert!(
                stdout.contains("src/") || stdout.contains("tests/"),
                "{stdout}"
            );
        } else {
            for name in ["loop", "context", "tools", "control", "io", "regression"] {
                assert!(stdout.contains(name), "catalog omitted {name}: {stdout}");
            }
        }
        assert!(!stdout.contains("[lab paused"));
    }
}
