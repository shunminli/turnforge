//! Text/function subset of the Chat Completions SSE protocol, not Responses API.
use std::{
    collections::BTreeMap,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use reqwest::{
    Client, ClientBuilder, Url,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    CancellationToken,
    message::{AssistantMessage, Message, ToolCall, Usage},
    model::{Model, ModelDelta, ModelError, ModelRequest},
};

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

/// No Debug implementation: authorization material must not enter diagnostics.
pub struct OpenAiModel {
    client: Client,
    endpoint: Url,
    model: String,
}

impl OpenAiModel {
    /// `base_url` includes the API prefix (usually `/v1`). Only HTTPS or
    /// loopback HTTP is accepted. Redirects are disabled; secrets are not logged.
    pub fn new(
        base_url: &str,
        api_key: Option<&str>,
        model: &str,
        timeout: Duration,
    ) -> Result<Self, ModelError> {
        Self::build(base_url, api_key, model, timeout, Client::builder())
    }

    /// Local-only transport: literal loopback IP, no credentials or proxy use.
    /// The API prefix and SSE contract are the same as `new`.
    pub fn new_local(base_url: &str, model: &str, timeout: Duration) -> Result<Self, ModelError> {
        // Check the original authority so URL normalization cannot turn a DNS
        // name, integer IPv4 address or shorthand into an accepted literal IP.
        let authority = base_url
            .split_once("://")
            .and_then(|(_, remainder)| remainder.split(['/', '?', '#']).next())
            .unwrap_or("");
        let local = authority
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
            .is_ok_and(|address| address.is_loopback());
        if !local {
            return Err(config("local models require a literal loopback IP URL"));
        }
        Self::build(base_url, None, model, timeout, Client::builder().no_proxy())
    }

    fn build(
        base_url: &str,
        api_key: Option<&str>,
        model: &str,
        timeout: Duration,
        builder: ClientBuilder,
    ) -> Result<Self, ModelError> {
        let mut endpoint = Url::parse(base_url).map_err(|_| config("invalid base URL"))?;
        let host = endpoint.host_str().unwrap_or("").trim_matches(['[', ']']);
        let local = host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback());
        if endpoint.scheme() != "https" && !(endpoint.scheme() == "http" && local) {
            return Err(config("use HTTPS, or HTTP on loopback for local models"));
        }
        if !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(config(
                "base URL cannot contain credentials, query or fragment",
            ));
        }
        if model.trim().is_empty() || timeout.is_zero() {
            return Err(config("model and nonzero timeout are required"));
        }
        let path = format!("{}/chat/completions", endpoint.path().trim_end_matches('/'));
        endpoint.set_path(&path);
        let mut headers = HeaderMap::new();
        if let Some(key) = api_key.filter(|key| !key.is_empty()) {
            let mut auth = HeaderValue::from_str(&format!("Bearer {key}"))
                .map_err(|_| config("invalid API key header"))?;
            auth.set_sensitive(true);
            headers.insert(AUTHORIZATION, auth);
        }
        let client = builder
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(timeout)
            .build()
            .map_err(transport)?;
        Ok(Self {
            client,
            endpoint,
            model: model.into(),
        })
    }

    async fn request(
        &self,
        request: ModelRequest<'_>,
        emit: &mut (dyn FnMut(ModelDelta) + Send),
    ) -> Result<AssistantMessage, ModelError> {
        let mut messages = vec![json!({"role":"system", "content":request.system})];
        for message in request.messages {
            messages.push(match message {
                Message::User { text } => json!({"role":"user", "content":text}),
                Message::Assistant { message } => {
                    let mut value = json!({"role":"assistant", "content": message.text});
                    if !message.tool_calls.is_empty() {
                        value["tool_calls"] = Value::Array(message.tool_calls.iter().map(|call| json!({
                            "id":call.id, "type":"function", "function":{"name":call.name,"arguments":call.arguments.to_string()}
                        })).collect());
                    }
                    value
                }
                Message::Tool { call_id, output } => json!({"role":"tool", "tool_call_id":call_id, "content":serde_json::to_string(output).expect("ToolOutput is JSON")}),
            });
        }
        let mut body = json!({"model":self.model, "messages":messages, "stream":true,"stream_options":{"include_usage":true}});
        if !request.tools.is_empty() {
            body["tools"] = Value::Array(request.tools.iter().map(|tool| json!({
                "type":"function", "function":{"name":tool.name,"description":tool.description,"parameters":tool.parameters}
            })).collect());
        }
        let response = self
            .client
            .post(self.endpoint.clone())
            .json(&body)
            .send()
            .await
            .map_err(transport)?;
        if !response.status().is_success() {
            // Do not echo response bodies: gateways can include prompts or keys.
            return Err(ModelError::HttpStatus(response.status().as_u16()));
        }
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !content_type
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case("text/event-stream")
        {
            return Err(protocol("expected text/event-stream"));
        }
        let mut bytes_seen = 0usize;
        let bytes = response.bytes_stream().map(move |chunk| {
            let chunk = chunk.map_err(transport)?;
            bytes_seen = bytes_seen.saturating_add(chunk.len());
            if bytes_seen > MAX_RESPONSE_BYTES {
                return Err(protocol("response exceeds 2 MiB"));
            }
            Ok(chunk)
        });
        let mut stream = bytes.eventsource();
        let mut assembly = Assembly::default();
        while let Some(event) = stream.next().await {
            let event = event
                .map_err(|_| protocol("SSE transport interrupted or response limit exceeded"))?;
            if event.data.trim() == "[DONE]" {
                return assembly.finish();
            }
            if event.event == "error" {
                return Err(protocol("provider sent an error event"));
            }
            let chunk: Chunk =
                serde_json::from_str(&event.data).map_err(|_| protocol("malformed JSON chunk"))?;
            assembly.push(chunk, emit)?;
            // A buffered HTTP chunk may contain many SSE events. Yield so the
            // host can drain output and observe cancellation between events.
            tokio::task::yield_now().await;
        }
        Err(protocol("stream ended without [DONE]"))
    }
}

#[async_trait]
impl Model for OpenAiModel {
    async fn complete(
        &self,
        request: ModelRequest<'_>,
        cancel: &CancellationToken,
        emit: &mut (dyn FnMut(ModelDelta) + Send),
    ) -> Result<AssistantMessage, ModelError> {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(ModelError::Cancelled),
            result = self.request(request, emit) => result,
        }
    }
}

fn config(message: &str) -> ModelError {
    ModelError::Configuration(message.into())
}
fn protocol(message: &str) -> ModelError {
    ModelError::Protocol(message.into())
}
fn transport(error: reqwest::Error) -> ModelError {
    ModelError::Transport(error.without_url().to_string())
}

#[derive(Deserialize)]
struct Chunk {
    #[serde(default)]
    choices: Vec<Choice>,
    usage: Option<WireUsage>,
    error: Option<Value>,
}
#[derive(Deserialize)]
struct Choice {
    index: u32,
    #[serde(default)]
    delta: Delta,
    finish_reason: Option<String>,
}
#[derive(Default, Deserialize)]
struct Delta {
    content: Option<String>,
    refusal: Option<String>,
    #[serde(default)]
    tool_calls: Vec<CallDelta>,
}
#[derive(Deserialize)]
struct CallDelta {
    index: u32,
    id: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    function: Option<FunctionDelta>,
}
#[derive(Deserialize)]
struct FunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}
#[derive(Deserialize)]
struct WireUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

#[derive(Default)]
struct PendingCall {
    id: String,
    name: String,
    arguments: String,
}
#[derive(Default)]
struct Assembly {
    text: String,
    calls: BTreeMap<u32, PendingCall>,
    usage: Option<Usage>,
    finish_reason: Option<String>,
}

impl Assembly {
    fn push(
        &mut self,
        chunk: Chunk,
        emit: &mut (dyn FnMut(ModelDelta) + Send),
    ) -> Result<(), ModelError> {
        if chunk.error.is_some() {
            return Err(protocol("provider reported an error"));
        }
        if let Some(usage) = chunk.usage {
            self.usage = Some(Usage {
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
                total_tokens: usage.total_tokens,
            });
        }
        for choice in chunk.choices {
            if choice.index != 0 || self.finish_reason.is_some() {
                return Err(protocol("unexpected choice or data after finish"));
            }
            if choice.delta.refusal.as_ref().is_some_and(|s| !s.is_empty()) {
                return Err(protocol("model refused the request"));
            }
            if let Some(text) = choice.delta.content {
                self.text.push_str(&text);
                emit(ModelDelta::Text { text });
            }
            for delta in choice.delta.tool_calls {
                if delta.index >= 32 || delta.kind.as_ref().is_some_and(|kind| kind != "function") {
                    return Err(protocol("unsupported tool type or excessive tool index"));
                }
                let pending = self.calls.entry(delta.index).or_default();
                if let Some(id) = delta.id {
                    if !pending.id.is_empty() && pending.id != id {
                        return Err(protocol("tool id changed mid-stream"));
                    }
                    pending.id = id;
                }
                if let Some(function) = delta.function {
                    if let Some(name) = function.name {
                        if !pending.name.is_empty() && pending.name != name {
                            return Err(protocol("tool name changed mid-stream"));
                        }
                        pending.name = name;
                    }
                    if let Some(fragment) = function.arguments {
                        pending.arguments.push_str(&fragment);
                        emit(ModelDelta::ToolArguments {
                            index: delta.index,
                            fragment,
                        });
                    }
                }
            }
            self.finish_reason = choice.finish_reason;
        }
        Ok(())
    }

    fn finish(self) -> Result<AssistantMessage, ModelError> {
        match self.finish_reason.as_deref() {
            Some("stop") if self.calls.is_empty() => {}
            Some("tool_calls") if !self.calls.is_empty() => {}
            _ => return Err(protocol("missing, truncated or inconsistent finish reason")),
        }
        let mut tool_calls = Vec::new();
        let mut ids = std::collections::HashSet::new();
        for call in self.calls.into_values() {
            let arguments: Value = serde_json::from_str(&call.arguments)
                .map_err(|_| protocol("incomplete tool arguments"))?;
            if call.id.is_empty()
                || call.name.is_empty()
                || !arguments.is_object()
                || !ids.insert(call.id.clone())
            {
                return Err(protocol("invalid or duplicate tool call"));
            }
            tool_calls.push(ToolCall {
                id: call.id,
                name: call.name,
                arguments,
            });
        }
        Ok(AssistantMessage {
            text: self.text,
            tool_calls,
            usage: self.usage,
        })
    }
}
