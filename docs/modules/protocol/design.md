# 消息与事件协议：设计

证据：[message.rs](../../../src/message.rs)、[event.rs](../../../src/event.rs)、[model.rs](../../../src/model.rs)。
配套：[架构](architecture.md) · [总索引](../../README.md)。以下是当前序列化与调用合同，不是版本化 RPC 标准。

## 数据结构

| 类型 | 当前字段 / 变体 | 校验或限制归属 |
|---|---|---|
| `ToolCall` | `id: String`、`name: String`、`arguments: Value` | Agent 检查非空 ID/名称、object 参数、同消息内 ID 唯一 |
| `AssistantMessage` | `text: String`、`tool_calls: Vec<ToolCall>`、`usage: Option<Usage>` | provider 先验证完整性；Agent 再校验调用结构 |
| `Usage` | `input_tokens`、`output_tokens`、`total_tokens`，均 `u64` | provider 映射计数，不在此重算或计费 |
| `ToolOutput` | `Ok { data: Value }` / `Error { code: String, message: String }` | 具体工具/registry 定义 code 和 data |
| `Message` | `User { text }` / `Assistant { message }` / `Tool { call_id, output }` | Agent 提交和维护顺序 |

`ToolCall` 可被直接构造，因此“能够构造/反序列化”不是已经通过业务校验的证明。
每个 Assistant 最多 32 个工具调用；ID 唯一性只在该消息内部检查，不是全会话全局去重。
`AssistantMessage::default()` 的 Rust 默认值不等于 serde 允许省略所有字段。
这些公共数据结构未设置 `deny_unknown_fields`；内置工具输入的严格校验另见[工具设计](../tool-runtime/design.md)。

## 序列化形状

`Message` 使用 `role` 标签，`ToolOutput` 使用 `status` 标签；变体名为 snake_case。
以下示例是内部消息 JSON，不是 provider 请求：

```json
{"role":"assistant","message":{"text":"准备读取","tool_calls":[{"id":"c1","name":"read_file","arguments":{"path":"README.md"}}],"usage":null}}
```

```json
{"role":"tool","call_id":"c1","output":{"status":"ok","data":{"text":"example","truncated":false,"start_line":1,"total_lines":1}}}
```

失败结果的形状是 `{"status":"error","code":"permission_denied","message":"..."}`。
`data` 没有统一跨工具 schema；调用方不能把所有成功结果都当字符串，或把所有错误 message 都当 JSON。
OpenAI adapter 把 `arguments` 和整个 `ToolOutput` 分别编码成外部 API 要求的字符串，见[模型设计](../model/design.md)。

## 事件和状态

`Event` 用 `type` 标签，包含以下变体：

| JSON type | 载荷 | 不能据此推导 |
|---|---|---|
| `run_started` | 无 | 模型或工具已经执行 |
| `step_started` | `step`，从 1 开始 | 当前请求必然成功 |
| `model_delta` | `delta` | 文本/参数已经提交，或碎片可执行 |
| `message_committed` | `message` | 消息已持久化或 stdout 已交付 |
| `tool_started` | 完整 `call` | 已通过 registry 权限或真的产生副作用 |
| `run_finished` | `outcome` | 用户任务已经被正确完成 |
| `debug_paused` | `snapshot` | 已持久化、自动获得工具授权或可在另一进程恢复 |
| `debug_snapshot` | `snapshot` | 已执行下一动作或新增暂停点；它是 Inspect 的只读响应 |
| `debug_resumed` | `pause_id` | 下一模型/工具必然成功 |
| `debug_command_rejected` | `command`、`reason` | 命令已推进运行；该通知恰好表示拒绝 |

`ModelDelta` 的 `kind` 为 `text { text }` 或 `tool_arguments { index, fragment }`。
index 是本次 provider 消息内的索引，碎片可能不是合法 JSON，也没有独立的全局 call ID。
终态枚举序列化为 `completed`、`cancelled`、`step_limit`、`failed`。
`RunState` 用 `status` 标签；运行中附带 `step` 和 `phase`（`model` / `tools`）。
调试暂停为 `Paused { step, pause_id, point }`，序列化 status 为 `paused`。
普通 `run` 不发上述 `debug_*` 事件；既有消费者若穷尽匹配 Rust enum，需要显式处理新增变体。
快照 v1、DebugPoint/DebugAction 和严格命令序列化详见[调试设计](../debugger/design.md)。

## 配对、提交和取消不变量

1. Agent 先把 Message 写入 `messages`，再发包含克隆值的 `MessageCommitted`。
2. 只有完整且通过校验的 Assistant 进入历史，ModelDelta 不能直接作为调用执行。
3. 正常协作退出时，每个已提交工具调用都有对应 `Message::Tool`；未知、拒绝和取消也是结果。
4. `ToolStarted` 在 registry 执行检查之前发出。已取消而跳过的待执行调用只有结果，没有 ToolStarted。
5. run 的 future 被完整等待且 callback 不 panic 时，已接受的 run 发一个 RunFinished。
   丢弃 future、panic、输出故障或宿主强杀不受“消费者收到一次 terminal”的保证保护。

例如，启动前 token 已取消时，当前 Agent 仍提交用户输入，再结束为 Cancelled；不调用模型。
模型失败时已显示的 delta 不会被“回滚显示”，但不会进入完整 Assistant 历史。
有多个调用时取消，已完成的结果保留，尚未开始的调用写入 cancelled，不虚构回滚。

## 兼容性与限制

- 无 session/run/step ID 全局命名、事件序号、时间戳或 JSON 版本字段；宿主不得跨运行盲目合流后依赖顺序恢复。
- 调试 snapshot 有独立 version，pause ID 只在一次 DebugSession 内有意义；不是完整 Event 的全局版本或 ID。
- Message 不保存 system prompt、工具定义、权限、模型配置，因此仅它本身不足以精确重放运行。
- Usage 可缺省；缺省不等于零，不能把 token 字段当预算执行器。
- 工具错误 code 为 String，不是封闭 enum；消费者应保留未知 code 的处理路径。
- 没有图片、推理内容块、持久化恢复或跨版本迁移。

## 测试与维护检查

[Agent 测试](../../../tests/agent.rs) 的 `model_tools_model_and_second_user_turn_have_ordered_history`、
`invalid_calls_never_commit_or_execute`、`cancellation_closes_pending_calls_without_running_them`
覆盖顺序、校验和取消配对。`provider_error_leaves_no_partial_assistant` 覆盖失败时不提交半条消息。
[CLI 测试](../../../tests/cli_http.rs) 的 `fragmented_tool_arguments_round_trip_to_real_file_and_model`
覆盖实际 JSON 事件和 provider 之间的字段翻译；`actual_ctrl_c_cancels_shell_and_preserves_event_closure` 覆盖协作终态。

尚无所有公共类型的完整 serde golden/round-trip 或版本兼容测试，不能声称稳定 wire ABI。
修改前后检查：角色/标签是否变化；ToolOutput 是否仍可编码；各消费者 match 是否完整；
旧事件解析方是否兼容；配对/取消/usage 缺省用例是否仍成立。新增兼容承诺需配套测试和版本策略。
