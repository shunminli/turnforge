//! Synthetic, pinned-local-model scenarios for the CLI harness workbench.
//! The host owns this fixture through execution, output draining and verification.

use std::{
    collections::BTreeMap,
    error::Error,
    fs, io,
    net::{IpAddr, SocketAddr},
    path::{Component, Path},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use clap::{Args, ValueEnum};
use reqwest::{Client, Url};
use serde::{Deserialize, de::DeserializeOwned};
use tempfile::TempDir;
use turnforge::message::{Message, ToolCall, ToolOutput};

const RESPONSE_LIMIT: usize = 64 * 1024;
const WRITE_CONTENT: &str = "turnforge-lab-write-ok";
const SETUP_HINT: &str = "请先运行 bash scripts/serve-local-llm.sh；安装和基线修复见 docs/local-llm.md。不会自动安装或下载模型";

#[derive(Args)]
pub struct LabArgs {
    /// Synthetic scenario; write explicitly grants file writes inside its fixture.
    #[arg(long, value_enum, default_value = "read")]
    pub case: LabCase,
    /// Run without interactive pauses and verify the scenario automatically.
    #[arg(long)]
    pub auto: bool,
    /// Show an optional learning focus at each semantic pause.
    #[arg(long, value_enum)]
    pub lesson: Option<crate::learning::Lesson>,
    /// Ollama HTTP origin using a literal loopback IP, not a remote service.
    #[arg(long, default_value = "http://127.0.0.1:11434")]
    pub ollama_url: String,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum LabCase {
    Chat,
    Read,
    Write,
}

#[derive(Deserialize)]
struct Baseline {
    ollama_version: String,
    test_model: ModelBaseline,
}

#[derive(Deserialize)]
struct ModelBaseline {
    name: String,
    digest: String,
}

#[derive(Deserialize)]
struct Version {
    version: String,
}

#[derive(Deserialize)]
struct Tags {
    models: Vec<ModelBaseline>,
}

enum Expected {
    Chat,
    Read { marker: String, original: String },
    Write,
}

pub struct PreparedLab {
    pub workspace: TempDir,
    pub prompt: String,
    pub allow_write: bool,
    pub model: String,
    pub base_url: String,
    expected: Expected,
}

fn preflight_error(message: &str) -> io::Error {
    io::Error::other(format!("Ollama 预检失败：{message}。{SETUP_HINT}"))
}

fn local_origin(raw: &str) -> io::Result<Url> {
    let invalid = || {
        io::Error::other(
            "--ollama-url 必须是 loopback 字面 IP 的 HTTP origin，例如 http://127.0.0.1:11434；不能含凭据、路径、query 或 fragment",
        )
    };
    let url = Url::parse(raw).map_err(|_| invalid())?;
    if url.scheme() != "http"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || raw.trim() != raw
    {
        return Err(invalid());
    }
    // Parse the original authority, not URL's normalized host: names, integer
    // IPv4 aliases and DNS resolution must not broaden the local-only contract.
    let remainder = raw.strip_prefix("http://").ok_or_else(invalid)?;
    let (authority, path) = remainder.split_once('/').unwrap_or((remainder, ""));
    if !path.is_empty() {
        return Err(invalid());
    }
    let address = authority
        .parse::<SocketAddr>()
        .map(|address| address.ip())
        .or_else(|_| authority.parse::<IpAddr>())
        .or_else(|_| {
            authority
                .strip_prefix('[')
                .and_then(|address| address.strip_suffix(']'))
                .unwrap_or("")
                .parse::<IpAddr>()
        })
        .map_err(|_| invalid())?;
    if !address.is_loopback() {
        return Err(invalid());
    }
    Ok(url)
}

async fn local_json<T: DeserializeOwned>(client: &Client, url: Url) -> io::Result<T> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|_| preflight_error("无法连接本地服务，或请求超过 5 秒"))?;
    if !response.status().is_success() {
        return Err(preflight_error(&format!(
            "服务返回 HTTP {}（不跟随重定向）",
            response.status().as_u16()
        )));
    }
    if response
        .content_length()
        .is_some_and(|length| length > RESPONSE_LIMIT as u64)
    {
        return Err(preflight_error("响应超过 64 KiB 上限"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| preflight_error("读取响应失败，或请求超过 5 秒"))?
    {
        if chunk.len() > RESPONSE_LIMIT.saturating_sub(bytes.len()) {
            return Err(preflight_error("响应超过 64 KiB 上限"));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| preflight_error("响应不是有效的预期 JSON"))
}

pub async fn prepare(args: &LabArgs) -> Result<PreparedLab, Box<dyn Error>> {
    let origin = local_origin(&args.ollama_url)?;
    let baseline: Baseline = serde_json::from_str(include_str!("../dev/local-llm.lock.json"))?;
    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()?;
    let version: Version = local_json(&client, origin.join("api/version")?).await?;
    if version.version != baseline.ollama_version {
        return Err(preflight_error(&format!(
            "Ollama 版本与锁定基线 {} 不符",
            baseline.ollama_version
        ))
        .into());
    }
    let tags: Tags = local_json(&client, origin.join("api/tags")?).await?;
    let installed = tags
        .models
        .iter()
        .find(|model| model.name == baseline.test_model.name)
        .ok_or_else(|| preflight_error("未安装锁定的本地测试模型"))?;
    if installed.digest != baseline.test_model.digest {
        return Err(preflight_error("测试模型 digest 与锁定基线不符").into());
    }

    let workspace = tempfile::Builder::new()
        .prefix("turnforge-lab-")
        .tempdir()?;
    let (prompt, expected) = match args.case {
        LabCase::Chat => (
            "Reply with one short greeting sentence. Do not use any tools.".to_owned(),
            Expected::Chat,
        ),
        LabCase::Read => {
            // Neither prompt nor the workspace name reveals this oracle value.
            // A readable synthetic passphrase keeps this a tool/context demo,
            // rather than a long hexadecimal-copying benchmark. It is not a
            // security token; exact matching and the independent oracle remain.
            let words = [
                "amber", "birch", "cedar", "delta", "elm", "fern", "globe", "harbor", "iris",
                "jade", "kite", "lemon", "maple", "north", "ocean", "pearl",
            ];
            let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
            let marker = (0..6)
                .map(|index| words[((stamp >> (index * 4)) & 15) as usize])
                .collect::<Vec<_>>()
                .join("-");
            let original = format!("code = {marker}\n");
            fs::write(workspace.path().join("marker.txt"), &original)?;
            (
                "Use read_file to read marker.txt in the workspace. In your final answer, report the exact value assigned to code in that file. Read the actual file; do not guess. Do not modify any files.".to_owned(),
                Expected::Read { marker, original },
            )
        }
        LabCase::Write => (
            format!(
                "Use write_file to create result.txt in the workspace with exactly this content: {WRITE_CONTENT} (no final newline). After the tool succeeds, give a brief confirmation. Do not create any other files."
            ),
            Expected::Write,
        ),
    };
    Ok(PreparedLab {
        workspace,
        prompt,
        allow_write: matches!(args.case, LabCase::Write),
        model: baseline.test_model.name,
        base_url: origin.join("v1")?.to_string(),
        expected,
    })
}

impl PreparedLab {
    pub fn description(&self) -> String {
        let case = match self.expected {
            Expected::Chat => "chat：纯文本问候，不执行工具",
            Expected::Read { .. } => "read：读取未知 marker 并验证答案（只读）",
            Expected::Write => "write：授权在临时目录写入 result.txt 并验证精确内容",
        };
        format!(
            "Turnforge Harness Lab\n场景：{case}\n模型：{}\n接口：{}\n临时工作区：{}\n仅使用合成文件，不授权 shell；退出并完成清理后自动删除临时工作区。\n",
            self.model,
            self.base_url,
            self.workspace.path().display()
        )
    }

    /// Called after Completed, before the host queues its summary and drains
    /// output. Completion is not the oracle: inspect results and disk state.
    pub fn verify(&self, messages: &[Message]) -> Result<String, Box<dyn Error>> {
        let fail = |message: &str| io::Error::other(format!("Lab 验证失败：{message}"));
        let Some(Message::Assistant { message: last }) = messages.last() else {
            return Err(fail("没有最终助手消息").into());
        };
        if last.text.trim().is_empty() || !last.tool_calls.is_empty() {
            return Err(fail("最终助手消息为空或仍请求工具").into());
        }
        let mut pending: BTreeMap<&str, &ToolCall> = BTreeMap::new();
        let mut completed = Vec::new();
        for message in messages {
            match message {
                Message::Assistant { message } => {
                    if !pending.is_empty() {
                        return Err(fail("下一助手消息前仍有未闭合工具调用").into());
                    }
                    for call in &message.tool_calls {
                        if pending.insert(&call.id, call).is_some() {
                            return Err(fail("同批工具调用 ID 重复").into());
                        }
                    }
                }
                Message::Tool { call_id, output } => {
                    let call = pending
                        .remove(call_id.as_str())
                        .ok_or_else(|| fail("工具结果没有配对的调用"))?;
                    completed.push((call, output));
                }
                Message::User { .. } if !pending.is_empty() => {
                    return Err(fail("用户消息前仍有未闭合工具调用").into());
                }
                Message::User { .. } => {}
            }
        }
        if !pending.is_empty() {
            return Err(fail("运行结束时仍有未闭合工具调用").into());
        }
        match &self.expected {
            Expected::Chat => {
                if !completed.is_empty() {
                    return Err(fail("chat 场景不应调用工具").into());
                }
                Ok("模型返回非空最终回答，且没有调用工具。".to_owned())
            }
            Expected::Read { marker, original } => {
                let read = completed.iter().any(|(call, output)| {
                    call.name == "read_file"
                        && matches_path(&call.arguments["path"], "marker.txt")
                        && matches!(output, ToolOutput::Ok { data }
                            if data["text"].as_str().is_some_and(|text| text.contains(marker)))
                });
                if !read {
                    return Err(fail("缺少成功读取 marker.txt 的工具调用及结果").into());
                }
                if !last.text.contains(marker) {
                    return Err(fail(
                        "工具读取成功，但模型最终答案未逐字包含文件标记；模型输出未通过校验",
                    )
                    .into());
                }
                if fs::read_to_string(self.workspace.path().join("marker.txt"))? != *original {
                    return Err(fail("只读场景的原始文件被修改").into());
                }
                Ok("read_file 返回未知 marker，最终答案使用该值，原文件保持不变。".to_owned())
            }
            Expected::Write => {
                let written = completed.iter().any(|(call, output)| {
                    call.name == "write_file"
                        && matches_path(&call.arguments["path"], "result.txt")
                        && call.arguments["content"] == WRITE_CONTENT
                        && matches!(output, ToolOutput::Ok { data }
                            if matches_path(&data["path"], "result.txt")
                                && data["bytes"].as_u64() == Some(WRITE_CONTENT.len() as u64))
                });
                if !written {
                    return Err(fail("缺少 result.txt 的成功精确写入调用和结果").into());
                }
                if fs::read_to_string(self.workspace.path().join("result.txt"))? != WRITE_CONTENT {
                    return Err(fail("磁盘 result.txt 内容与预期不一致").into());
                }
                Ok("write_file 成功，独立磁盘检查确认 result.txt 内容完全一致。".to_owned())
            }
        }
    }
}

fn matches_path(value: &serde_json::Value, expected: &str) -> bool {
    value.as_str().is_some_and(|path| {
        Path::new(path)
            .components()
            .filter(|component| *component != Component::CurDir)
            .eq(Path::new(expected).components())
    })
}

#[cfg(test)]
mod tests {
    use super::local_origin;

    #[test]
    fn lab_origin_requires_literal_loopback_and_no_extra_url_components() {
        for valid in [
            "http://127.0.0.1:11434",
            "http://127.0.0.2/",
            "http://[::1]:11434/",
            "http://[::1]",
        ] {
            assert!(local_origin(valid).is_ok(), "{valid}");
        }
        for invalid in [
            "http://localhost:11434",
            "https://127.0.0.1",
            "http://192.168.1.1",
            "http://[::ffff:127.0.0.1]",
            "http://2130706433",
            "http://127.1",
            "http://127.0.0.1/v1",
            "http://127.0.0.1/foo/..",
            "http://127.0.0.1/%2e",
            "http://user:secret@127.0.0.1",
            "http://127.0.0.1?key=secret",
            "http://127.0.0.1/#fragment",
            " http://127.0.0.1",
        ] {
            assert!(local_origin(invalid).is_err(), "{invalid}");
        }
    }
}
