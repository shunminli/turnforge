//! Opt-in, real-model smoke tests. They never download models or start a server.
//! Run after local setup: cargo test --locked --test local_llm -- --ignored --test-threads=1
//! The model's words are not golden outputs; files and committed events are the oracle.

use std::{
    collections::BTreeMap,
    io,
    path::Path,
    process::Stdio,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::Value;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    time::timeout,
};

const BASE_URL: &str = "http://127.0.0.1:11434/v1";
const MODEL: &str = "turnforge-test:qwen3-4b-v1";
const RUN_TIMEOUT: Duration = Duration::from_secs(300);
const CAPTURE_LIMIT: usize = 4 * 1024 * 1024;

struct CompletedCall {
    call: Value,
    output: Value,
}

struct ObservedRun {
    final_text: String,
    calls: Vec<CompletedCall>,
    diagnostic: String,
}

async fn check_local_baseline() {
    let lock: Value = serde_json::from_str(include_str!("../dev/local-llm.lock.json"))
        .expect("local LLM baseline lock must contain valid JSON");
    assert_eq!(lock["test_model"]["name"], MODEL);
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let version: Value = client
        .get("http://127.0.0.1:11434/api/version")
        .send()
        .await
        .expect("local Ollama is required; complete local setup before running ignored tests")
        .error_for_status()
        .expect("local Ollama version endpoint failed")
        .json()
        .await
        .expect("local Ollama version response is not JSON");
    assert_eq!(
        version["version"], lock["ollama_version"],
        "Ollama runtime drift: review/update the local baseline deliberately"
    );
    let tags: Value = client
        .get("http://127.0.0.1:11434/api/tags")
        .send()
        .await
        .expect("cannot query local Ollama models")
        .error_for_status()
        .expect("local Ollama tags endpoint failed")
        .json()
        .await
        .expect("local Ollama tags response is not JSON");
    let installed = tags["models"]
        .as_array()
        .expect("local Ollama tags must contain a models array")
        .iter()
        .find(|model| model["name"] == MODEL)
        .expect("turnforge-test:qwen3-4b-v1 is missing; complete local model setup first");
    assert_eq!(
        installed["digest"], lock["test_model"]["digest"],
        "Local model digest drift: do not silently change the regression baseline"
    );
    eprintln!(
        "local baseline: ollama={}, model={MODEL}, digest={}",
        version["version"].as_str().unwrap(),
        installed["digest"].as_str().unwrap()
    );
}

// Each pipe is borrowed only by its drain future. Keep memory bounded while still
// draining to EOF, so a large output cannot deadlock the child on a full pipe.
async fn drain(reader: &mut (impl AsyncRead + Unpin), bytes: &mut Vec<u8>) -> io::Result<bool> {
    let mut truncated = false;
    let mut buffer = [0; 8192];
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            return Ok(truncated);
        }
        let keep = count.min(CAPTURE_LIMIT.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&buffer[..keep]);
        truncated |= keep != count;
    }
}

fn diagnostic(stdout: &[u8], stderr: &[u8]) -> String {
    // Only synthetic prompts/files are used. Cap failure output independently of
    // capture memory, retaining the end of the trace where terminal errors appear.
    let tail = |bytes: &[u8]| {
        String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(32 * 1024)..]).into_owned()
    };
    format!(
        "model={MODEL}, endpoint={BASE_URL}\nstdout (tail):\n{}\nstderr (tail):\n{}",
        tail(stdout),
        tail(stderr)
    )
}

async fn run(root: &Path, prompt: &str, allow_write: bool) -> ObservedRun {
    let started_at = Instant::now();
    check_local_baseline().await;
    let mut command = Command::new(env!("CARGO_BIN_EXE_turnforge"));
    command
        .args(["run", prompt, "--model", MODEL, "--base-url", BASE_URL])
        .arg("--workspace")
        .arg(root)
        .args(["--json", "--max-steps", "4", "--request-timeout", "90"])
        .current_dir(root)
        // Do not inherit cloud keys, model overrides or uppercase/lowercase proxy
        // variables. This changes only the child, not the caller's environment.
        .env_clear()
        .env("NO_PROXY", "*")
        .env("no_proxy", "*")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if allow_write {
        command.arg("--allow-write");
    }
    // No --allow-shell: all model-requested file effects stay in this temporary
    // workspace under the existing trusted-local-workspace contract.
    let mut child = command
        .spawn()
        .expect("cannot start the real Turnforge CLI");
    let mut stdout_pipe = child.stdout.take().unwrap();
    let mut stderr_pipe = child.stderr.take().unwrap();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    // The test owns the child and both pipes. These are concurrently polled
    // futures, not spawned tasks; every normal path waits for exit and both EOFs.
    let result = timeout(RUN_TIMEOUT, async {
        tokio::join!(
            child.wait(),
            drain(&mut stdout_pipe, &mut stdout),
            drain(&mut stderr_pipe, &mut stderr)
        )
    })
    .await;
    let (status, stdout_result, stderr_result) = match result {
        Ok(result) => result,
        Err(_) => {
            // Dropping the join future cancels pipe reads, not the process. Kill
            // and explicitly reap before panic; kill_on_drop is only a fallback.
            let kill = child.start_kill();
            let cleanup = timeout(Duration::from_secs(5), child.wait()).await;
            panic!(
                "CLI exceeded {RUN_TIMEOUT:?}; kill={kill:?}, reap={cleanup:?}\n{}",
                diagnostic(&stdout, &stderr)
            );
        }
    };
    let diagnostic = diagnostic(&stdout, &stderr);
    assert!(
        matches!(stdout_result, Ok(false)) && matches!(stderr_result, Ok(false)),
        "capture failed or exceeded {CAPTURE_LIMIT} bytes: stdout={stdout_result:?}, stderr={stderr_result:?}\n{diagnostic}"
    );
    assert!(
        matches!(status, Ok(status) if status.success()),
        "CLI did not exit successfully: {status:?}\n{diagnostic}"
    );
    let stdout = String::from_utf8(stdout)
        .unwrap_or_else(|error| panic!("stdout is not UTF-8: {error}\n{diagnostic}"));
    let events: Vec<Value> = stdout
        .lines()
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("stdout is not NDJSON: {error}\n{diagnostic}"))
        })
        .collect();
    assert_eq!(
        events.iter().filter(|e| e["type"] == "run_started").count(),
        1,
        "{diagnostic}"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e["type"] == "run_finished")
            .count(),
        1,
        "{diagnostic}"
    );
    assert!(
        events.first().is_some_and(|e| e["type"] == "run_started")
            && events
                .last()
                .is_some_and(|e| { e["type"] == "run_finished" && e["outcome"] == "completed" }),
        "event stream did not close normally\n{diagnostic}"
    );
    assert!(
        events.iter().any(|e| {
            e["type"] == "model_delta"
                && e["delta"]["kind"] == "text"
                && e["delta"]["text"].as_str().is_some_and(|s| !s.is_empty())
        }),
        "missing streamed model text\n{diagnostic}"
    );

    // IDs are only unique within one assistant message, not across the run.
    // Empty this map before accepting the next assistant, independently of call
    // order or generated ID format, and verify a real start plus terminal result.
    let mut pending: BTreeMap<&str, (&Value, bool)> = BTreeMap::new();
    let mut calls = Vec::new();
    let mut last_message = None;
    for event in &events {
        if event["type"] == "tool_started" {
            let call = &event["call"];
            let id = call["id"].as_str().expect("tool start needs an ID");
            let (committed, started) = pending
                .get_mut(id)
                .unwrap_or_else(|| panic!("tool started without a committed call\n{diagnostic}"));
            assert_eq!(*committed, call, "{diagnostic}");
            assert!(!*started, "tool started twice\n{diagnostic}");
            *started = true;
        }
        if event["type"] != "message_committed" {
            continue;
        }
        let message = &event["message"];
        last_message = Some(message);
        match message["role"].as_str() {
            Some("user") => assert!(pending.is_empty(), "{diagnostic}"),
            Some("assistant") => {
                assert!(
                    pending.is_empty(),
                    "previous tool batch is open\n{diagnostic}"
                );
                for call in message["message"]["tool_calls"].as_array().unwrap() {
                    let id = call["id"].as_str().unwrap();
                    assert!(
                        pending.insert(id, (call, false)).is_none(),
                        "duplicate tool ID in the same assistant message\n{diagnostic}"
                    );
                }
            }
            Some("tool") => {
                let id = message["call_id"].as_str().unwrap();
                let (call, started) = pending
                    .remove(id)
                    .unwrap_or_else(|| panic!("tool result without a pending call\n{diagnostic}"));
                assert!(started, "tool result without execution start\n{diagnostic}");
                calls.push(CompletedCall {
                    call: call.clone(),
                    output: message["output"].clone(),
                });
            }
            _ => panic!("unexpected committed role\n{diagnostic}"),
        }
    }
    assert!(
        pending.is_empty(),
        "run ended with open calls\n{diagnostic}"
    );
    let last = last_message.unwrap_or_else(|| panic!("no committed messages\n{diagnostic}"));
    assert_eq!(last["role"], "assistant", "{diagnostic}");
    assert!(
        last["message"]["tool_calls"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "final assistant still requests tools\n{diagnostic}"
    );
    let final_text = last["message"]["text"].as_str().unwrap().to_owned();
    assert!(
        !final_text.trim().is_empty(),
        "empty final answer\n{diagnostic}"
    );
    let mut tool_counts = BTreeMap::new();
    for completed in &calls {
        *tool_counts
            .entry(completed.call["name"].as_str().unwrap())
            .or_insert(0_usize) += 1;
    }
    eprintln!(
        "local CLI evidence: elapsed={:.2}s, outcome=completed, committed_tool_counts={tool_counts:?}",
        started_at.elapsed().as_secs_f64()
    );
    ObservedRun {
        final_text,
        calls,
        diagnostic,
    }
}

#[tokio::test]
#[ignore = "requires the pinned local Ollama runtime and turnforge-test:qwen3-4b-v1 model"]
async fn local_llm_plain_text_stream_completes() {
    let workspace = tempfile::tempdir().unwrap();
    let result = run(
        workspace.path(),
        "Reply with one short greeting sentence. Do not use any tools.",
        false,
    )
    .await;
    assert!(result.calls.is_empty(), "{}", result.diagnostic);
}

#[tokio::test]
#[ignore = "requires the pinned local Ollama runtime and turnforge-test:qwen3-4b-v1 model"]
async fn local_llm_reads_unknown_file_marker() {
    let workspace = tempfile::tempdir().unwrap();
    let marker = format!(
        "turnforge-marker-{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    std::fs::write(
        workspace.path().join("marker.txt"),
        format!("code = {marker}\n"),
    )
    .unwrap();
    // The unknown value is absent from both the prompt and the workspace name.
    let result = run(
        workspace.path(),
        "Use read_file to read marker.txt in the workspace. In your final answer, report the exact value assigned to code in that file. Read the actual file; do not guess. Do not modify any files.",
        false,
    )
    .await;
    assert!(
        result.calls.iter().any(|completed| {
            completed.call["name"] == "read_file"
                && completed.output["status"] == "ok"
                && completed.output["data"]["text"]
                    .as_str()
                    .is_some_and(|text| text.contains(&marker))
        }),
        "no successful real read returned the unknown marker\n{}",
        result.diagnostic
    );
    assert!(
        result.final_text.contains(&marker),
        "final model answer did not use the file's unknown value\n{}",
        result.diagnostic
    );
}

#[tokio::test]
#[ignore = "requires the pinned local Ollama runtime and turnforge-test:qwen3-4b-v1 model"]
async fn local_llm_writes_file_and_finishes() {
    let workspace = tempfile::tempdir().unwrap();
    let expected = "turnforge-local-write-ok";
    let result = run(
        workspace.path(),
        "Use write_file to create result.txt in the workspace with exactly this content: turnforge-local-write-ok (no final newline). After the tool succeeds, give a brief confirmation. Do not create any other files.",
        true,
    )
    .await;
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("result.txt")).unwrap_or_else(
            |error| panic!("cannot read generated file: {error}\n{}", result.diagnostic)
        ),
        expected,
        "generated file content differs\n{}",
        result.diagnostic
    );
    assert!(
        result.calls.iter().any(|completed| {
            completed.call["name"] == "write_file"
                && completed.output["status"] == "ok"
                && completed.output["data"]["path"]
                    .as_str()
                    .is_some_and(|path| Path::new(path) == Path::new("result.txt"))
                && completed.output["data"]["bytes"] == expected.len()
        }),
        "missing successful write result for the independently checked file\n{}",
        result.diagnostic
    );
}
