# 工具运行时：设计

范围：`src/tools/mod.rs` 的现有契约。职责关系见[架构](architecture.md)，导航见[文档索引](../../README.md)。

## 类型与 API

| 符号 | 实际契约 |
|---|---|
| `Capability::{Read, Write, Shell}` | 序列化为 `read`、`write`、`shell` |
| `Permissions { allow_write, allow_shell }` | 默认均为 false；Read 始终允许 |
| `ToolDefinition` | `name`、`description`、`parameters: Value`、`capability` |
| `Tool: Send + Sync` | `definition()` 返回定义；异步 `execute(Value, &CancellationToken)` 返回 `ToolOutput` |
| `ToolRegistry::new(Permissions)` | 构造空 Registry，授权保存在私有字段 |
| `register(impl Tool + 'static)` | 接管实现；返回 `Result<(), RegistryError>` |
| `definitions()` | 返回授权工具的定义克隆，按名称排序 |
| `execute(&ToolCall, &CancellationToken)` | 返回一个 `ToolOutput`，不提交历史、不发事件 |

当前没有 unregister、覆盖注册、运行时更新权限或访问内部工具的方法。
`ToolCall` 与 `ToolOutput` 的字段及序列化由[消息协议](../protocol/design.md)维护。
Registry 不使用 `ToolCall.id` 做去重；Agent 在提交 Assistant 前完成本步 ID/参数基本校验。

## 注册与执行顺序

注册顺序：读取 `tool.definition()` → 按 `definition.name` 查重 → 插入定义快照和 boxed 实现。
不会检查名称是否为空、schema 是否有效，或实现是否与声明的 capability 一致。
这些是可信扩展的责任；不要把“成功注册”理解为已完成安全审计。

每次执行严格遵循：

1. token 已取消：直接返回 `cancelled`。
2. 查找 `call.name`：不存在则返回 `unknown_tool`。
3. 检查已缓存定义的 capability：未授权则返回 `permission_denied`。
4. 克隆 `call.arguments`，调用并等待 `entry.tool.execute`。
5. 原样返回工具结果；具体输入 schema 校验由工具执行。

因此“取消 + 未知名称”优先报告取消，“拒绝 + 无效参数”优先报告拒绝。
调用工具后没有 Registry 层的第二次取消判断：成功副作用不能被简单改写成未执行。

## 取消与错误矩阵

| 条件 | 结果 | 会不会调用实现 |
|---|---|---|
| 注册名称已存在 | `RegistryError::Duplicate(name)` | 不执行工具，不覆盖原条目 |
| 执行入口 token 已取消 | `ToolOutput::Error { code: "cancelled", .. }` | 否 |
| 未注册名称 | `unknown_tool` | 否 |
| Write/Shell 未获授权 | `permission_denied` | 否 |
| 工具输入/IO/运行错误 | 工具定义的 `ToolOutput::Error` | 是 |
| 执行中 token 取消 | 由具体工具返回实际结果并完成其清理 | 是 |
| 扩展 panic 或永不返回 | 无通用捕获或超时保障 | 是 |

Agent 对已提交的调用逐个提交终结结果，包括未知、拒绝和取消。
若在 Agent 检查 token 后、Registry 检查前发生取消，可能已经发出 `ToolStarted`，但工具未执行。
这不是实际副作用已经发生的证据；事件语义见[Agent 设计](../agent/design.md)。

## 权限与资源界限

Registry 没有总运行时间、参数字节数或输出字节数限制；限制分布在模型、Agent 和具体工具中。
Agent 每步最多接受 32 个调用，不是 Registry 独立调用的限额。
CLI 的 `--tool-timeout` 配置的是 ShellTool，不会自动限制文件或自定义工具。
默认 Read 允许访问文件工具工作区内的数据；它不是“无敏感信息”保证。
`allow_shell` 可以产生任意宿主允许的文件、网络等副作用，能力类别之间不是分级沙箱。

## 关键不变量

- 一个 Registry 内每个名称只有一个定义快照和实现，不静默覆盖。
- 展示和执行使用同一授权判断；隐藏工具仍必须在执行端拒绝。
- `Tool` 不拥有或修改 Agent 的历史；唯一结果提交者是 Agent。
- 调用方取消后应继续 await，给具体工具清理与返回事实的机会。
- Schema 是给模型的描述，具体实现必须自行严格验证，不能信任模型输入。

## 现有验证与未覆盖边界

[tests/tools.rs](../../../tests/tools.rs) 中 `registry_enforces_capabilities_and_rejects_duplicates`
覆盖重名、默认隐藏 Write、执行拒绝和未知名称。
[tests/cli_http.rs](../../../tests/cli_http.rs) 中 `denied_tool_is_not_advertised_or_executed_but_returns_result`
覆盖模型伪造隐藏调用后仍收到拒绝结果；`a_full_batch_of_immediate_tool_errors_does_not_overflow_output`
覆盖一整批立即拒绝与输出链路的交互。
[tests/agent.rs](../../../tests/agent.rs) 中 `cancellation_closes_pending_calls_without_running_them`
验证取消后剩余调用的结果闭合，而非自定义工具一定具有正确清理实现。

当前没有针对自定义工具谎报 capability、panic、卡死、动态 definition 或并发重入的防护测试。
也没有通用 schema 编译/校验测试；这些能力尚未由 Registry 提供。

## 变更检查清单

- [ ] 修改 capability 时同时核对 `Permissions::allows`、CLI 参数及默认可见工具。
- [ ] 新工具的 schema、实际解析和工具结果能否一致表达错误？
- [ ] 新扩展的副作用 owner、取消检查点与等待清理位置是否明确？
- [ ] 若改变执行顺序/错误码，是否同步 Agent 历史与 CLI 协议测试？
- [ ] 是否在 Registry 引入了本应属于工具、审批宿主或调度层的状态？

源码：[tools/mod.rs](../../../src/tools/mod.rs)、[message.rs](../../../src/message.rs)、[main.rs](../../../src/main.rs)。
