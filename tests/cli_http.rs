//! Independent protocol oracle: real TCP server -> real CLI -> real filesystem.
//! Fixture responses do not use turnforge's request/response serialization.
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

struct Response {
    status: u16,
    body: String,
}
struct Server {
    base_url: String,
    thread: thread::JoinHandle<Vec<Value>>,
}

impl Server {
    fn start(responses: Vec<Response>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let thread = thread::spawn(move || {
            let mut requests = Vec::new();
            for response in responses {
                let deadline = Instant::now() + Duration::from_secs(8);
                let mut socket = loop {
                    match listener.accept() {
                        Ok((socket, _)) => break socket,
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock
                                && Instant::now() < deadline =>
                        {
                            thread::sleep(Duration::from_millis(5))
                        }
                        _ => return requests,
                    }
                };
                // macOS accept can inherit O_NONBLOCK from the listener; the
                // request reader below intentionally uses blocking I/O + timeout.
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                requests.push(read_request(&mut socket));
                let header = format!(
                    "HTTP/1.1 {} Fixture\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response.status,
                    response.body.len()
                );
                if socket.write_all(header.as_bytes()).is_err() {
                    continue;
                }
                // Deliberately split JSON, UTF-8 and SSE delimiters across writes.
                for chunk in response.body.as_bytes().chunks(17) {
                    if socket.write_all(chunk).is_err() {
                        break;
                    }
                }
            }
            requests
        });
        Self { base_url, thread }
    }
    fn finish(self) -> Vec<Value> {
        self.thread.join().unwrap()
    }
}

fn read_request(socket: &mut TcpStream) -> Value {
    let mut bytes = Vec::new();
    let mut buf = [0; 4096];
    let header_end = loop {
        let count = socket.read(&mut buf).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buf[..count]);
        if let Some(pos) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break pos + 4;
        }
        assert!(bytes.len() < 64 * 1024);
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    assert!(headers.starts_with("POST /v1/chat/completions HTTP/1.1"));
    let length: usize = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().unwrap())
        })
        .unwrap();
    while bytes.len() < header_end + length {
        let count = socket.read(&mut buf).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buf[..count]);
    }
    serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
}

fn response(body: String) -> Response {
    Response { status: 200, body }
}
fn event(delta: Value, finish: Option<&str>) -> String {
    format!(
        "data: {}\r\n\r\n",
        json!({"choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
    )
}
fn final_text(text: &str) -> Response {
    response(format!(
        "{}{}data: [DONE]\r\n\r\n",
        event(json!({"content":text}), None),
        event(json!({}), Some("stop"))
    ))
}
fn tool(name: &str, args: Value) -> Response {
    response(format!(
        "{}{}data: [DONE]\n\n",
        event(
            json!({"tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":name,"arguments":args.to_string()}}]}),
            None
        ),
        event(json!({}), Some("tool_calls"))
    ))
}
fn command(root: &Path, url: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_turnforge"));
    command
        .args([
            "run",
            "fixture task",
            "--model",
            "fixture-model",
            "--base-url",
            url,
            "--workspace",
        ])
        .arg(root)
        .args(["--json", "--request-timeout", "3", "--tool-timeout", "1"])
        .env_remove("OPENAI_API_KEY")
        .env_remove("TURNFORGE_API_KEY")
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("ALL_PROXY")
        .env("NO_PROXY", "*");
    command
}
fn events(output: &Output) -> Vec<Value> {
    String::from_utf8(output.stdout.clone())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn assert_outcome(output: &Output, code: i32, outcome: &str) {
    assert_eq!(
        output.status.code(),
        Some(code),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let events = events(output);
    assert_eq!(
        events
            .iter()
            .filter(|e| e["type"] == "run_finished")
            .count(),
        1
    );
    assert_eq!(events.last().unwrap()["outcome"], outcome);
}

#[test]
fn fragmented_tool_arguments_round_trip_to_real_file_and_model() {
    let dir = tempfile::tempdir().unwrap();
    let first = format!(
        "{}{}{}data: {{\"choices\":[],\"usage\":{{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}}}\n\ndata: [DONE]\n\n",
        event(
            json!({"tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":"write_file","arguments":"{\"path\":\"hello.txt\","}}]}),
            None
        ),
        event(
            json!({"tool_calls":[{"index":0,"function":{"arguments":"\"content\":\"你好，Rust!\"}"}}]}),
            None
        ),
        event(json!({}), Some("tool_calls"))
    );
    let server = Server::start(vec![response(first), final_text("完成")]);
    let output = command(dir.path(), &server.base_url)
        .arg("--allow-write")
        .output()
        .unwrap();
    assert_outcome(&output, 0, "completed");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("hello.txt")).unwrap(),
        "你好，Rust!"
    );
    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["model"], "fixture-model");
    assert_eq!(requests[0]["stream_options"]["include_usage"], true);
    let second = &requests[1]["messages"];
    assert_eq!(second[2]["tool_calls"][0]["id"], "call-1");
    assert_eq!(second[3]["role"], "tool");
    assert_eq!(second[3]["tool_call_id"], "call-1");
    let result: Value = serde_json::from_str(second[3]["content"].as_str().unwrap()).unwrap();
    assert_eq!(result["status"], "ok");
    let events = events(&output);
    assert!(
        events
            .iter()
            .any(|e| e["message"]["message"]["usage"]["total_tokens"] == 15)
    );
    assert!(
        events
            .iter()
            .any(|e| e["message"]["message"]["text"] == "完成")
    );
}

#[test]
fn denied_tool_is_not_advertised_or_executed_but_returns_result() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(vec![
        tool("write_file", json!({"path":"blocked","content":"bad"})),
        final_text("Permission denied"),
    ]);
    let output = command(dir.path(), &server.base_url).output().unwrap();
    assert_outcome(&output, 0, "completed");
    assert!(!dir.path().join("blocked").exists());
    let requests = server.finish();
    let names: Vec<_> = requests[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["list_files", "read_file"]);
    let result: Value =
        serde_json::from_str(requests[1]["messages"][3]["content"].as_str().unwrap()).unwrap();
    assert_eq!(result["code"], "permission_denied");
}

#[test]
fn a_full_batch_of_immediate_tool_errors_does_not_overflow_output() {
    let dir = tempfile::tempdir().unwrap();
    let calls:Vec<_> = (0..32).map(|index| json!({"index":index,"id":format!("call-{index}"),"type":"function","function":{"name":"write_file","arguments":"{\"path\":\"blocked\",\"content\":\"no\"}"}})).collect();
    let first = format!(
        "{}{}data: [DONE]\n\n",
        event(json!({"tool_calls":calls}), None),
        event(json!({}), Some("tool_calls"))
    );
    let server = Server::start(vec![response(first), final_text("All denied")]);
    let output = command(dir.path(), &server.base_url).output().unwrap();
    assert_outcome(&output, 0, "completed");
    let events = events(&output);
    assert_eq!(
        events
            .iter()
            .filter(|event| event["message"]["output"]["code"] == "permission_denied")
            .count(),
        32
    );
    assert!(!dir.path().join("blocked").exists());
    let requests = server.finish();
    assert_eq!(requests[1]["messages"].as_array().unwrap().len(), 35);
}

#[test]
fn malformed_or_incomplete_streams_never_execute_tools() {
    let cases = [
        "data: not-json\n\ndata: [DONE]\n\n".into(),
        event(json!({"content":"partial"}), None),
        format!(
            "{}data: [DONE]\n\n",
            event(json!({"content":"partial"}), Some("length"))
        ),
        format!(
            "{}{}data: [DONE]\n\n",
            event(
                json!({"tool_calls":[{"index":0,"id":"call-1","function":{"name":"write_file","arguments":"{"}}]}),
                None
            ),
            event(json!({}), Some("tool_calls"))
        ),
        "data: [DONE]\n\n".into(),
        "event: error\ndata: {\"error\":\"test error\"}\n\n".into(),
    ];
    for body in cases {
        let dir = tempfile::tempdir().unwrap();
        let server = Server::start(vec![response(body)]);
        let output = command(dir.path(), &server.base_url)
            .arg("--allow-write")
            .output()
            .unwrap();
        assert_outcome(&output, 1, "failed");
        assert!(events(&output).iter().all(|e| e["type"] != "tool_started"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        assert_eq!(server.finish().len(), 1);
    }
}

#[test]
fn http_error_body_is_not_leaked_or_retried() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(vec![Response {
        status: 429,
        body: "SECRET-FROM-GATEWAY".into(),
    }]);
    let output = command(dir.path(), &server.base_url).output().unwrap();
    assert_outcome(&output, 1, "failed");
    assert!(!String::from_utf8_lossy(&output.stderr).contains("SECRET-FROM-GATEWAY"));
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn cli_step_limit_has_distinct_exit_and_a_closed_tool_result() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(vec![tool("list_files", json!({"path":"."}))]);
    let output = command(dir.path(), &server.base_url)
        .args(["--max-steps", "1"])
        .output()
        .unwrap();
    assert_outcome(&output, 2, "step_limit");
    assert!(
        events(&output)
            .iter()
            .any(|e| e["message"]["role"] == "tool")
    );
    server.finish();
}

#[test]
fn tools_and_help_work_without_a_model_or_key() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_turnforge"))
        .args(["tools", "--workspace"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let tools: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(tools.as_array().unwrap().len(), 2);
    assert!(
        Command::new(env!("CARGO_BIN_EXE_turnforge"))
            .arg("--help")
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[cfg(unix)]
#[test]
fn actual_ctrl_c_cancels_shell_and_preserves_event_closure() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(vec![tool(
        "bash",
        json!({"description":"signal test", "command":"printf ready > ready; (sleep 1; printf escaped > escaped) & wait"}),
    )]);
    let mut child = command(dir.path(), &server.base_url)
        .arg("--allow-shell")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !dir.path().join("ready").exists() {
        if Instant::now() > deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("shell never became ready");
        }
        thread::sleep(Duration::from_millis(10));
    }
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(child.id() as i32),
        nix::sys::signal::Signal::SIGINT,
    )
    .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_outcome(&output, 130, "cancelled");
    assert!(
        events(&output)
            .iter()
            .any(|e| e["message"]["output"]["code"] == "cancelled")
    );
    thread::sleep(Duration::from_millis(1200));
    assert!(!dir.path().join("escaped").exists());
    server.finish();
}

#[cfg(unix)]
#[test]
fn blocked_stdout_does_not_block_ctrl_c_or_process_exit() {
    let dir = tempfile::tempdir().unwrap();
    let mut body = String::new();
    for _ in 0..16 {
        body.push_str(&event(json!({"content":"x".repeat(16 * 1024)}), None));
    }
    body.push_str(&event(json!({}), Some("stop")));
    body.push_str("data: [DONE]\n\n");
    let server = Server::start(vec![response(body)]);
    let mut child = command(dir.path(), &server.base_url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Wait until the response has been supplied, then leave stdout completely
    // unread throughout cancellation; draining here would hide the regression.
    assert_eq!(server.finish().len(), 1);
    thread::sleep(Duration::from_millis(200));
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(child.id() as i32),
        nix::sys::signal::Signal::SIGINT,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(!status.success());
            break;
        }
        if Instant::now() > deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("blocked stdout prevented cancellation");
        }
        thread::sleep(Duration::from_millis(10));
    }
    // Only now consume output. NDJSON may end mid-line if its consumer stalls.
    let output = child.wait_with_output().unwrap();
    assert!(String::from_utf8_lossy(&output.stderr).contains("stdout stalled"));
}

#[test]
fn model_endpoint_configuration_rejects_plaintext_remote_and_url_secrets() {
    use turnforge::openai::OpenAiModel;
    for base in [
        "http://example.com/v1",
        "https://user:password@example.com/v1",
        "https://example.com/v1?key=secret",
        "file:///tmp/model",
    ] {
        assert!(OpenAiModel::new(base, None, "fixture", Duration::from_secs(1)).is_err());
    }
    assert!(
        OpenAiModel::new(
            "http://127.0.0.1:1234/v1",
            None,
            "fixture",
            Duration::from_secs(1)
        )
        .is_ok()
    );
    assert!(
        OpenAiModel::new(
            "https://example.com/v1",
            Some("bad\nheader"),
            "fixture",
            Duration::from_secs(1)
        )
        .is_err()
    );
}

#[test]
fn stdout_can_be_redirected_to_a_regular_file_or_dev_null() {
    let dir = tempfile::tempdir().unwrap();
    for null in [false, true] {
        let server = Server::start(vec![final_text("redirected")]);
        let stdout = if null {
            Stdio::null()
        } else {
            Stdio::from(std::fs::File::create(dir.path().join("events.jsonl")).unwrap())
        };
        let status = command(dir.path(), &server.base_url)
            .stdout(stdout)
            .stderr(Stdio::piped())
            .status()
            .unwrap();
        assert!(status.success());
        server.finish();
    }
    let text = std::fs::read_to_string(dir.path().join("events.jsonl")).unwrap();
    assert!(text.contains("run_finished"));
}
