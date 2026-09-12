use std::{fs, time::Duration};

use serde_json::{Value, json};
use turnforge::{
    CancellationToken,
    message::{ToolCall, ToolOutput},
    tools::{FileOperation, FileTool, Permissions, ShellTool, Tool, ToolRegistry, Workspace},
};

fn data(output: ToolOutput) -> Value {
    match output {
        ToolOutput::Ok { data } => data,
        other => panic!("expected success, got {other:?}"),
    }
}
fn code(output: ToolOutput) -> String {
    match output {
        ToolOutput::Error { code, .. } => code,
        other => panic!("expected error, got {other:?}"),
    }
}
async fn file(workspace: &Workspace, operation: FileOperation, args: Value) -> ToolOutput {
    FileTool::new(workspace.clone(), operation)
        .execute(args, &CancellationToken::new())
        .await
}

#[tokio::test]
async fn write_read_replace_and_mkdir_operate_in_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = Workspace::new(dir.path()).unwrap();
    data(file(&workspace, FileOperation::Mkdir, json!({"path":"src"})).await);
    data(
        file(
            &workspace,
            FileOperation::Write,
            json!({"path":"src/a.rs", "content":"one\ntwo\n"}),
        )
        .await,
    );
    assert_eq!(
        data(
            file(
                &workspace,
                FileOperation::Read,
                json!({"path":"src/a.rs","start_line":2})
            )
            .await
        )["text"],
        "two\n"
    );
    data(
        file(
            &workspace,
            FileOperation::Replace,
            json!({"path":"src/a.rs","old":"two","new":"three"}),
        )
        .await,
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("src/a.rs")).unwrap(),
        "one\nthree\n"
    );
    let listing = data(file(&workspace, FileOperation::List, json!({"path":"src"})).await);
    assert_eq!(listing["entries"][0]["name"], "a.rs");
}

#[tokio::test]
async fn ambiguous_replacement_and_invalid_arguments_do_not_mutate() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = Workspace::new(dir.path()).unwrap();
    fs::write(dir.path().join("a"), "duplicate duplicate").unwrap();
    assert_eq!(
        code(
            file(
                &workspace,
                FileOperation::Replace,
                json!({"path":"a","old":"duplicate","new":"changed"})
            )
            .await
        ),
        "ambiguous_match"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("a")).unwrap(),
        "duplicate duplicate"
    );
    assert_eq!(
        code(
            file(
                &workspace,
                FileOperation::Write,
                json!({"path":"a","content":"new","extra":true})
            )
            .await
        ),
        "invalid_arguments"
    );
    assert_eq!(
        code(
            file(
                &workspace,
                FileOperation::Read,
                json!({"path":"a","start_line":0})
            )
            .await
        ),
        "invalid_arguments"
    );
}

#[tokio::test]
async fn path_escape_absolute_paths_and_symlinks_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let workspace = Workspace::new(dir.path()).unwrap();
    for path in [
        "../escape".to_string(),
        outside.path().join("escape").display().to_string(),
    ] {
        assert_eq!(
            code(
                file(
                    &workspace,
                    FileOperation::Write,
                    json!({"path":path,"content":"bad"})
                )
                .await
            ),
            "invalid_path"
        );
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
        assert_eq!(
            code(
                file(
                    &workspace,
                    FileOperation::Write,
                    json!({"path":"link/escape","content":"bad"})
                )
                .await
            ),
            "invalid_path"
        );
        assert!(!outside.path().join("escape").exists());
    }
}

#[tokio::test]
async fn read_limits_preserve_utf8_and_reject_binary_or_large_files() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = Workspace::new(dir.path()).unwrap();
    fs::write(dir.path().join("a"), "中文hello").unwrap();
    let result = data(
        file(
            &workspace,
            FileOperation::Read,
            json!({"path":"a","max_bytes":4}),
        )
        .await,
    );
    assert_eq!(result["text"], "中");
    assert_eq!(result["truncated"], true);
    fs::write(dir.path().join("bin"), [255, 254]).unwrap();
    assert_eq!(
        code(file(&workspace, FileOperation::Read, json!({"path":"bin"})).await),
        "invalid_encoding"
    );
    fs::File::create(dir.path().join("large"))
        .unwrap()
        .set_len(4 * 1024 * 1024 + 1)
        .unwrap();
    assert_eq!(
        code(file(&workspace, FileOperation::Read, json!({"path":"large"})).await),
        "file_too_large"
    );
}

#[tokio::test]
async fn cancelled_write_has_no_side_effects() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = Workspace::new(dir.path()).unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    let result = FileTool::new(workspace, FileOperation::Write)
        .execute(json!({"path":"a","content":"no"}), &cancel)
        .await;
    assert_eq!(code(result), "cancelled");
    assert!(!dir.path().join("a").exists());
}

#[tokio::test]
async fn registry_enforces_capabilities_and_rejects_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = Workspace::new(dir.path()).unwrap();
    let mut registry = ToolRegistry::new(Permissions::default());
    registry
        .register(FileTool::new(workspace.clone(), FileOperation::Write))
        .unwrap();
    assert!(
        registry
            .register(FileTool::new(workspace, FileOperation::Write))
            .is_err()
    );
    assert!(registry.definitions().is_empty());
    let call = ToolCall {
        id: "1".into(),
        name: "write_file".into(),
        arguments: json!({"path":"a","content":"bad"}),
    };
    assert_eq!(
        code(registry.execute(&call, &CancellationToken::new()).await),
        "permission_denied"
    );
    assert!(!dir.path().join("a").exists());
    assert_eq!(
        code(
            registry
                .execute(
                    &ToolCall {
                        name: "missing".into(),
                        ..call
                    },
                    &CancellationToken::new()
                )
                .await
        ),
        "unknown_tool"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn shell_drains_stderr_and_preserves_nonzero_exit() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = Workspace::new(dir.path()).unwrap();
    let shell = ShellTool::new(workspace, Duration::from_secs(3));
    let result = data(shell.execute(json!({"description":"pipe test","command":"head -c 100000 /dev/zero >&2; printf ok"}), &CancellationToken::new()).await);
    assert_eq!(result["stdout"], "ok");
    assert_eq!(result["stderr_truncated"], true);
    let result = shell
        .execute(
            json!({"description":"exit test","command":"printf out; printf err >&2; exit 7"}),
            &CancellationToken::new(),
        )
        .await;
    match result {
        ToolOutput::Error { code, message } => {
            assert_eq!(code, "command_failed");
            let payload: Value = serde_json::from_str(&message).unwrap();
            assert_eq!(payload["exit_code"], 7);
            assert_eq!(payload["stdout"], "out");
            assert_eq!(payload["stderr"], "err");
        }
        _ => panic!("expected nonzero exit"),
    }
}

#[cfg(unix)]
#[tokio::test]
async fn shell_timeout_kills_descendants_before_they_write() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = Workspace::new(dir.path()).unwrap();
    let shell = ShellTool::new(workspace, Duration::from_millis(100));
    let result = shell.execute(json!({"description":"timeout test","command":"(sleep 0.5; printf escaped > escaped) & wait"}), &CancellationToken::new()).await;
    assert_eq!(code(result), "timeout");
    tokio::time::sleep(Duration::from_millis(650)).await;
    assert!(!dir.path().join("escaped").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn shell_cancellation_waits_for_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = Workspace::new(dir.path()).unwrap();
    let shell = ShellTool::new(workspace, Duration::from_secs(3));
    let cancel = CancellationToken::new();
    let (result, _) = tokio::join!(shell.execute(json!({"description":"cancel test","command":"printf ready > ready; (sleep 0.5; printf escaped > escaped) & wait"}), &cancel), async {
        tokio::time::timeout(Duration::from_secs(2), async { while !dir.path().join("ready").exists() { tokio::time::sleep(Duration::from_millis(10)).await; } }).await.unwrap();
        cancel.cancel();
    });
    assert_eq!(code(result), "cancelled");
    tokio::time::sleep(Duration::from_millis(650)).await;
    assert!(!dir.path().join("escaped").exists());
}
