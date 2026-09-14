# 模型边界：设计

状态：对应当前 M0 的 [model.rs](../../../src/model.rs) 和 [openai.rs](../../../src/openai.rs)。
责任与生命周期见[架构文档](architecture.md)；返回[文档入口](../../README.md)。

## 1. 公共接口合同

```rust
pub trait Model: Send + Sync {
    async fn complete(
        &self, request: ModelRequest<'_>, cancel: &CancellationToken,
        emit: &mut (dyn FnMut(ModelDelta) + Send),
    ) -> Result<AssistantMessage, ModelError>;
}
```

实际 trait 由 `async_trait` 展开。`ModelRequest` 借用 `system: &str`、
`messages: &[Message]`、`tools: &[ToolDefinition]`；没有可改写的会话或工具执行入口。
调用方允许取消时 drop 模型 future；实现必须取消安全，不得遗留自建 detached task 或执行工具。
`emit` 同步调用且不能阻塞/panic；`Text { text }` 和 `ToolArguments { index, fragment }`
均为 owned 暂态片段，以 `kind` 和 snake_case 序列化，不含完整工具 ID/name 或终态标记。
最终 `AssistantMessage` 才是可交给 Agent 校验/提交的事实，参见[协议设计](../protocol/design.md)。

## 2. 构造、配置与安全

`OpenAiModel::new(base_url, api_key, model, timeout)` 返回 `Result<Self, ModelError>`。

- URL 接受 HTTPS，或主机为 `localhost` / loopback IP 的 HTTP；拒绝 URL 用户名、密码、query、fragment。
- `base_url` 应含 API 前缀，如 `/v1`；去掉 path 尾部 `/` 后追加 `/chat/completions`。
- model 经 trim 判空，但存储原字符串；timeout 必须非零。Client 连接超时固定 10 秒，整体请求超时取传入值并覆盖流读取。
- `None` 或空 API key 不发送授权头；否则使用 `Bearer`，非法 header 值报配置错误，header 标为 sensitive。
- 禁用 HTTP 重定向；provider 不实现 `Debug`。非 2xx 响应只暴露状态码，不输出 body；reqwest 错误移除 URL。
- 环境变量读取归 CLI：优先非空 `TURNFORGE_API_KEY`，再取非空 `OPENAI_API_KEY`；库不自行读取环境。

本地专用 `OpenAiModel::new_local(base_url, model, timeout)` 同样返回 `Result<Self, ModelError>`：

- 无 api_key 参数，构造不设置授权头；不读取云凭据。
- 原始 URL authority 必须能解析为 loopback `IpAddr/SocketAddr`；拒绝 DNS 名（含 localhost）、
  简写/整数 IPv4 和 IPv4-mapped IPv6。可使用 HTTP 或 HTTPS 的字面 loopback，与 Lab 只接受 HTTP origin 的上层限制不同。
- 显式 `ClientBuilder::no_proxy()`；不依赖宿主临时更改环境，也不会跟随重定向。
- 其余 URL/模型/超时、请求序列化、SSE、错误及取消路径与 `new` 共用；没有另一套 provider 实现。

以上是传输防护，不是网络沙箱或秘密存储方案。选择 endpoint 仍意味着向该服务发送 prompt/工具数据；
通用 `new` 保留 reqwest 环境代理行为，只有 `new_local` 显式禁用。没有密钥内存清零；CLI 的 NDJSON 可能包含会话内容。

## 3. 请求序列化

每次发送一次 POST JSON；始终设置 `model`、`stream: true`、`stream_options.include_usage: true`。

| 内部输入 | Chat Completions 请求内容 |
|---|---|
| `system` | 首条 `role: system`，即使字符串为空也发送 |
| `Message::User` | `role: user` 和文本 `content` |
| `Message::Assistant` | `role: assistant`、文本；非空调用列表转换为 `type: function`，arguments 是 JSON 字符串 |
| `Message::Tool` | `role: tool`、`tool_call_id`；整个 `ToolOutput` 序列化为 content 字符串 |
| 非空 tools 切片 | function name/description/parameters；capability 不发送给模型 |

空 tools 时省略 `tools`；历史 usage 不进入请求。权限过滤在 Registry，provider 不代替权限判断。
没有设置 temperature、max tokens、tool_choice、parallel_tool_calls 或任意 provider 参数透传。
当前也没有请求体大小限制、上下文裁剪或历史合法性预检；通用会话结构由 Agent 管理。

## 4. SSE 与 Assembly

只接受成功 HTTP 且 Content-Type 的 media type 为 `text/event-stream`，大小写不敏感、允许参数。
`eventsource-stream` 处理 framing/UTF-8 分片；累计读取的响应 body chunk 超过 2 MiB 即失败。
限制发生在 SSE 解析前，包含 framing 字节；不包含 HTTP headers，也不是 token 上限。

`Assembly` 独占 text、`BTreeMap<u32, PendingCall>`、可选 usage 和 finish_reason。
收到数据后按以下规则处理：

1. `data.trim() == "[DONE]"` 立即调用 `finish()`；不等待 EOF，也不检查标记后的数据。
2. 其它 `event: error` 直接失败；JSON 不可反序列化或 chunk 内出现非 null 的 error 字段也失败。
3. usage 映射为 input/output/total tokens，采用最后一次值，不求和、不检查三者算术一致性。
4. choices 只允许 index 0；已有 finish_reason 后再收到任何 choice 都拒绝。无 choices 的 usage chunk 仍允许。
5. 非空 refusal 拒绝；content 追加到 text 并立即 emit `Text`。
6. tool index 必须小于 32；type 可省略，但若出现必须为 `function`。索引允许稀疏，最终按升序输出。
7. id/name 作为完整值记录；已有非空值后，后续只能重复相同值，不能把它们当字符串片段拼接。
8. arguments 是字符串片段，按同 index 到达顺序拼接并 emit `ToolArguments`，此时尚不解析 JSON。

每个普通 JSON event 后让出一次调度；未知 JSON 字段默认被 serde 忽略，不代表相应扩展已受支持。
到达 `[DONE]` 时，只接受 `stop + 无调用` 或 `tool_calls + 至少一个调用`；其它 finish reason、
缺失 finish reason、`length` 截断以及直接 EOF 全部失败。usage 不是完成标记，允许为空。
最终调用必须有非空 id/name，arguments 必须是完整 JSON object，且同条 Assistant 内 ID 唯一。
provider 不检查工具是否注册、权限是否允许或输入是否符合工具 schema；这些由工具边界完成。
[Agent 的二次校验](../agent/design.md)保护其它 provider，同样拒绝非法调用结构，最多每步 32 个调用。

## 5. 错误与取消映射

| `ModelError` | 当前来源及边界 |
|---|---|
| `Configuration(String)` | URL、空 model/零 timeout、授权 header 校验失败；错误不回显原配置值 |
| `Transport(String)` | Client 构造或 `.send()` 失败；请求 URL 已移除，包括该阶段的 timeout |
| `HttpStatus(u16)` | 非 2xx；不读入诊断正文，无本模块自动重试 |
| `Protocol(String)` | Content-Type、SSE/JSON/组装校验失败；正文读取中断、读取超时和 2 MiB 限制也在 SSE 层统一映射为此类 |
| `Cancelled` | `complete` 的优先取消分支；不是远端服务确认取消 |

需注意：body stream 的底层错误进入 `eventsource` 后会统一成为
`SSE transport interrupted or response limit exceeded`，调用方不能凭此区分网络中断、timeout 和限长。
Agent 把 `Cancelled` 转为取消 outcome，其余错误转为失败；模型阶段失败不提交半条 Assistant。
已 emit 的 delta 仍可能被宿主显示，消费者必须等完整消息提交，不能根据片段执行副作用。

## 6. 测试映射与证据边界

以下函数位于 [cli_http.rs](../../../tests/cli_http.rs)，使用真实本地 TCP 和 CLI，不需要模型密钥。

| 用例 | 保护的合同 |
|---|---|
| `fragmented_tool_arguments_round_trip_to_real_file_and_model` | 分段 SSE/UTF-8、参数组装、usage、真实写文件及下一请求的工具回填 |
| `malformed_or_incomplete_streams_never_execute_tools` | 非 JSON、无 DONE、length、坏参数、无 finish、error event 不触发工具 |
| `http_error_body_is_not_leaked_or_retried` | HTTP 429 正文不泄露，本次失败无第二次请求 |
| `model_endpoint_configuration_rejects_plaintext_remote_and_url_secrets` | 远程明文/URL 秘密/非法 header 拒绝，loopback HTTP 可用 |
| `blocked_stdout_does_not_block_ctrl_c_or_process_exit` | 流式输出背压时整条宿主链路仍可取消，不证明远端停止 |

[agent.rs 测试](../../../tests/agent.rs)中的 `invalid_calls_never_commit_or_execute`、
`provider_error_leaves_no_partial_assistant` 和 `host_cancellation_stops_a_pending_model`
分别补充通用调用校验、失败提交边界和不配合取消的 provider future 被丢弃的合同。

[lab_cli.rs](../../../tests/lab_cli.rs) 的 `lab_auto_chat_ignores_stdin_and_inherited_cloud_configuration`
通过真实 CLI/local provider 检查固定本地配置和无 Authorization 的模型请求；Lab 上层 origin 边界另由
`lab.rs::tests::lab_origin_requires_literal_loopback_and_no_extra_url_components` 检查。
目前没有直接覆盖 `OpenAiModel::new_local` 全部非法输入的独立矩阵；Lab 入口拒绝不等于证明所有库调用者。
以上测试也不证明任意代理环境或所有网络故障组合。

## 7. 已知缺口与维护清单

当前未接入 Responses/Anthropic、多模态、reasoning、重试/退避、缓存、token 预算或 provider 流式恢复。
本地协议用例不等于真实服务兼容认证；Content-Type、2 MiB、choice/refusal、finish 后数据、
id/name 改变、usage 异常和实际 HTTP 读取取消等分支尚无逐项专门回归用例。

- 改 `Model` 或消息结构时，同时检查 Agent 消费方式、暂态事件与最终提交是否仍然分离。
- 改 wire 转换时，检查历史 Assistant 的 arguments 字符串和 ToolOutput 字符串能否正确往返。
- 改 SSE 或限长时，补本地 HTTP 边界用例；不能只测试私有 `Assembly` 就宣称链路正确。
- 加新 provider 时记录信息损失和支持子集；加重试时先定义 delta 去重、取消与费用可观测性。
- 改错误/配置时重新检查诊断不泄露 URL/key/body，保留明确的失败类型和取消安全合同。
