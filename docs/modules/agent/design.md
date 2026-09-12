# Agent 调度与会话：设计

证据：[agent.rs](../../../src/agent.rs)、[Agent 测试](../../../tests/agent.rs)。
配套：[架构](architecture.md) · [总索引](../../README.md)。

## 公共接口

| 接口 | 合同 |
|---|---|
| `Agent::new(model, tools, config)` | 转移三个值的所有权，建立空历史与 Ready 状态 |
| `run(&mut self, input, &CancellationToken, &mut FnMut(Event))` | 异步推进一个 user turn，返回 `Result<RunOutcome, AgentError>` |
| `messages(&self) -> &[Message]` | 只读历史，不含 system 配置 |
| `state(&self) -> &RunState` | 只读运行状态，不能外部改写 |

callback 实际类型为 `&mut (dyn FnMut(Event) + Send)`；同步调用，不允许阻塞或 panic。
扩展宿主需在自己的边界提供适当背压，不能在 callback 内做阻塞网络或 stdout 写入。
`AgentConfig` 只有 `system: String` 与 `max_steps: NonZeroU32`，默认上限 20；没有统一总时限或 token 预算。

## 接受 run 与状态推进

1. 当前为 Running 时，返回 `AgentError::Interrupted`，不产生新事件。
2. 将 input 转 String，空白输入返回 `EmptyInput`；原字符串非空时完整保留，不自动 trim 存储。
3. 设置 `Running { step: 0, phase: Model }`，发 RunStarted，再提交 User。
4. 快照当前可见工具定义；step 从 1 到 max_steps，每轮开始检查取消。
5. 设置 Model 阶段，发 StepStarted；将只读历史和定义借给 `Model::complete`。
6. 成功时校验完整 Assistant，提交后如果没有调用则 Completed（若此时已取消则 Cancelled）。
7. 有调用时进入 Tools 阶段，逐条执行/跳过并提交结果；检查取消后决定是否进入下一轮。
8. run_loop 返回后设置 Finished，并发一次 RunFinished；发生模型错误时 state/event 为 Failed，但函数返回 Err。

默认接受任何 Finished 后的下一次 run；同一个 Agent 保留上一次用户输入及已提交历史。
模型错误不会撤销已有 User；没有“失败 run 自动回滚到之前”的逻辑。

## 模型响应与工具批次

`validate_assistant` 拒绝超过 32 个调用、空 id/name、非 object arguments、同批重复 ID。
这是 provider 之后的独立边界检查，不校验工具是否已注册、参数是否符合工具 schema。
非法 Assistant 不提交且不执行工具；抛 `ModelError::Protocol` 转入 AgentError。

提交 Assistant 前复制调用列表；因此执行批次不借用正在追加的 messages。
每个调用遵循：

- 已取消：构造 `ToolOutput::Error { code: "cancelled", ... }`，不发 ToolStarted、不调用 registry。
- 未取消：发 ToolStarted，再调用 registry；registry 会再次检查 token、工具存在性和权限。
- 得到结果后统一提交 `Message::Tool { call_id, output }`，然后 `yield_now().await`。

`ToolStarted` 表示调度器开始处理该调用，不代表已经授权或产生了副作用。
串行执行使 tool result 顺序等于调用顺序。最后一轮仍执行并记录工具结果，再返回 StepLimit。
不要为节约一条结果在达到上限时留下悬空 tool call。

## 结束与错误矩阵

| 条件 | run 返回 | 历史与事件 |
|---|---|---|
| 未接受：Running / 空输入 | Err(Interrupted / EmptyInput) | 不产生新 RunStarted |
| token 在 step 前取消 | Ok(Cancelled) | 已接受的 User 保留；不再请求模型 |
| 模型等待期间取消 / 模型返回 Cancelled | Ok(Cancelled) | 丢弃未完成的模型响应，不提交半条 Assistant |
| 模型 HTTP/协议/transport 错误或非法 Assistant | Err(Model(...)) | 设置 Failed，已有消息保留，发 RunFinished(Failed) |
| 工具 unknown/denied/参数/IO/timeout 错误 | 当前 step 继续 | 作为 ToolOutput 提交，模型可基于结果继续 |
| 工具运行期间 token 取消 | Ok(Cancelled)（清理和配对后） | 当前工具返回实际结果，剩余调用补 cancelled |
| 助手不再调用工具 | Ok(Completed)，或取消已观察时 Cancelled | 完整 Assistant 已提交 |
| 最后一步仍产生工具调用且未取消 | Ok(StepLimit) | 所有调用都有结果，无额外模型总结 |

工具返回一个 code 为 `cancelled` 的输出，并不会单独让 Agent 结束；是否结束仍由 token 决定。
同理，工具 timeout 不是 Agent 的总超时。`RunOutcome::Failed` 作为状态/事件使用，当前错误路径返回 Err，
不是 `Ok(Failed)`。宿主可以有更高优先级的输出错误，见 [CLI 设计](../cli/design.md)。

## 取消与中断边界

Model 调用置于 biased select，取消优先；其实现必须允许 future 被丢弃且无孤立副作用任务。
Tool 调用不放在外层 cancel-select 中，避免丢弃执行 future 造成子进程/文件 worker 失去清理 owner。
工具内 token check 和清理实现的边界见[文件设计](../filesystem-tools/design.md)与[Shell 设计](../shell-tool/design.md)。

宿主正确用法是发 token 后继续 await run。若直接 timeout/drop/task.abort，Running 状态不会自动改为 Finished，
后续调用被 Interrupted 拒绝；没有重新绑定中断现场的恢复 API。
正常完整等待、无 panic 的路径才保证每个已接受 run 发一个 terminal event。消费者能否收到另属输出合同。

## 不变量与执行者

- 单 writer：`run(&mut self)`、私有 messages 和只读 getter，由 Rust 借用及可见性保证。
- 先校验后提交：`validate_assistant` 在 Assistant commit 之前，由代码顺序和回归保护。
- 完整调用配对：串行工具循环与取消补结果负责，不是 serde 类型本身保证。
- 状态互斥：RunState enum 表达；合法转换仍由 `run` / `run_loop` 的实现维护。
- 公平调度：每个 Tool result 之后 yield，避免立即完成批次在一次 poll 中挤满宿主队列。

## 测试与维护清单

| 合同 | 现有测试 |
|---|---|
| 多工具顺序、跨 user turn 历史 | `model_tools_model_and_second_user_turn_have_ordered_history` |
| 取消配对、不执行剩余调用 | `cancellation_closes_pending_calls_without_running_them` |
| 上限仍有 Tool 结果 | `step_limit_still_closes_tool_calls` |
| 非法调用不提交/执行 | `invalid_calls_never_commit_or_execute` |
| 模型失败无半条助手消息 | `provider_error_leaves_no_partial_assistant` |
| 启动前和等待中取消 | `pre_cancelled_run_does_not_call_model`、`host_cancellation_stops_a_pending_model` |
| 丢弃后不可续跑 | `dropping_a_run_prevents_ambiguous_resume` |
| 立即完成的 32 个工具结果与宿主公平性 | [CLI 测试](../../../tests/cli_http.rs) `a_full_batch_of_immediate_tool_errors_does_not_overflow_output` |

尚无空输入、超过 32 个调用、所有 panic/drop 时点、超长会话的穷尽测试；也无持久化恢复或并行工具合同。
修改时检查：step 计数定义、terminal 位置、所有调用配对、模型/工具不同的取消策略、
event sink 是否还非阻塞、同步完成路径是否仍让出调度。新并发或恢复能力先补跨模块设计与真实失败用例。
