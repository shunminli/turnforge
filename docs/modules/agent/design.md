# Agent 调度与会话：设计

证据：[agent.rs](../../../src/agent.rs)、[Agent 测试](../../../tests/agent.rs)。
配套：[架构](architecture.md) · [总索引](../../README.md)。

## 公共接口

| 接口 | 合同 |
|---|---|
| `Agent::new(model, tools, config)` | 转移三个值的所有权，建立空历史与 Ready 状态 |
| `run(&mut self, input, &CancellationToken, &mut FnMut(Event))` | 异步推进一个 user turn，返回 `Result<RunOutcome, AgentError>` |
| `run_debug(&mut self, input, DebugSession, &mut FnMut(Event))` | 消费一次调试会话，复用同一循环和返回类型；在安全边界等待控制 |
| `messages(&self) -> &[Message]` | 只读历史，不含 system 配置 |
| `state(&self) -> &RunState` | 只读运行状态，不能外部改写 |

callback 实际类型为 `&mut (dyn FnMut(Event) + Send)`；同步调用，不允许阻塞或 panic。
扩展宿主需在自己的边界提供适当背压，不能在 callback 内做阻塞网络或 stdout 写入。
`AgentConfig` 有 `system: String`、`max_steps: NonZeroU32` 与 `tool_repeat_limit: Option<NonZeroU32>`。
max_steps 默认 20，tool_repeat_limit 默认 None（关闭）；没有统一总时限或 token 预算。
核心接受任何非零重复上限，1 表示首个非空工具批次即被阻止；CLI 参数限定至少 2，详见 [CLI 设计](../cli/design.md)。
新增公开配置字段需要完整 struct literal 的库调用方补字段或使用 `..AgentConfig::default()`；
`AgentError` 新增 `ToolLoop`，穷尽匹配旧变体的库调用方也需要更新。
默认行为、现有 Message/Event 形状和 RunOutcome 变体保持不变。

## 接受 run 与状态推进

1. 当前为 Running 或 Paused 时，返回 `AgentError::Interrupted`，不产生新事件。
2. 将 input 转 String，空白输入返回 `EmptyInput`；原字符串非空时完整保留，不自动 trim 存储。
3. 设置 `Running { step: 0, phase: Model }`，发 RunStarted，再提交 User。
4. 快照当前可见工具定义；step 从 1 到 max_steps，每轮开始检查取消。
5. 设置 Model 阶段，发 StepStarted；将只读历史和定义借给 `Model::complete`。
6. 成功时校验并提交完整 Assistant；启用保护时更新当前 run 的工具批次重复计数。
   没有调用则 Completed（若此时已取消则 Cancelled），不受重复上限影响。
7. 有调用时进入 Tools 阶段，逐条执行/跳过并提交结果；检查取消后决定是否进入下一轮。
8. run_loop 返回后设置 Finished，并发一次 RunFinished；模型错误或重复保护触发时 state/event 为 Failed，但函数返回 Err。

默认接受任何 Finished 后的下一次 run；同一个 Agent 保留上一次用户输入及已提交历史。
模型错误不会撤销已有 User；没有“失败 run 自动回滚到之前”的逻辑。

上面描述普通 `run` 的自动运行路径。调试路径在初始模型前、完整 Assistant 提交后、
正常批次的每条 Tool 结果提交后调用安全 checkpoint；Step/Continue 决定继续方式，Cancel 仍沿本循环结束。
最后无工具回复和达到上限的最后工具结果均可在终态前检查；调试用户还需释放 `Finish` 动作。
快照、Pause/Inspect、ID 校验和消息中间态见[调试设计](../debugger/design.md)，不另建一份 Agent 执行实现。

## 模型响应与工具批次

`validate_assistant` 拒绝超过 32 个调用、空 id/name、非 object arguments、同批重复 ID。
这是 provider 之后的独立边界检查，不校验工具是否已注册、参数是否符合工具 schema。
非法 Assistant 不提交且不执行工具；抛 `ModelError::Protocol` 转入 AgentError。

提交 Assistant 前复制调用列表；因此执行批次不借用正在追加的 messages。
每个调用遵循：

- 已取消：构造 `ToolOutput::Error { code: "cancelled", ... }`，不发 ToolStarted、不调用 registry。
- 未取消且批次被重复保护阻止：构造 code 为 tool_loop 的错误，不发 ToolStarted、不调用 registry。
- 未取消且未被保护阻止：发 ToolStarted，再调用 registry；registry 会再次检查 token、工具存在性和权限。
- 得到结果后统一提交 `Message::Tool { call_id, output }`，然后 `yield_now().await`。

`ToolStarted` 表示调度器开始处理该调用，不代表已经授权或产生了副作用。
串行执行使 tool result 顺序等于调用顺序。未触发重复保护的最后一轮仍执行并记录工具结果，再返回 StepLimit。
不要为节约一条结果在达到上限时留下悬空 tool call。

### 可选工具批次重复保护

每个非空 Assistant 工具批次计一次：名称、参数值和顺序均与上一批相同时增加连续计数，
不同批次重新从 1 计数。调用 ID、Assistant 文本和 usage 不参与比较。
参数按 `serde_json::Value` 判等，不按 JSON 字符串判等；同一批内多个相同调用不会各自增加计数。
检测状态为 `run_loop` 独占的上一批值副本及计数，失败、取消或自然结束后下一 user turn 都从零开始。

达到 `tool_repeat_limit` 的当前 Assistant 已通过校验并提交，但该批没有任何工具获得执行机会。
沿原有逐调用循环提交 `ToolOutput::Error { code: "tool_loop", ... }`，不发 ToolStarted，每条结果仍 yield。
调试 AfterModel 保留完整 pending_calls，next 为 `Finish { outcome: Failed }`；
Step/Continue 释放这一 Finish 后，整批补齐结果再终止，不产生 AfterTool 暂停或额外模型请求。
闭合整批后返回 `AgentError::ToolLoop { limit: NonZeroU32 }`，state 和终态事件为 Failed。
重复上限与模型步数上限同轮达到时，以 ToolLoop 失败结束。

取消检查先于每条保护结果：已写入的 tool_loop 结果保留，尚未写入的调用补 cancelled；
在 Finish(Failed) 暂停处、结果闭合中或最后结果提交后观察到取消，均返回 Cancelled，
不再等待人工释放或返回 ToolLoop。已有模型、真实工具的取消和清理路径不变。
该保护不检测交替出现的不同批次，不识别工具输出中的实际进展，也不撤销先前批次的副作用。

## 结束与错误矩阵

| 条件 | run 返回 | 历史与事件 |
|---|---|---|
| 未接受：Running 或 Paused / 空输入 | Err(Interrupted / EmptyInput) | 不产生新 RunStarted |
| token 在 step 前取消 | Ok(Cancelled) | 已接受的 User 保留；不再请求模型 |
| 模型等待期间取消 / 模型返回 Cancelled | Ok(Cancelled) | 丢弃未完成的模型响应，不提交半条 Assistant |
| 模型 HTTP/协议/transport 错误或非法 Assistant | Err(Model(...)) | 设置 Failed，已有消息保留，发 RunFinished(Failed) |
| 工具 unknown/denied/参数/IO/timeout 错误 | 当前 step 继续 | 作为 ToolOutput 提交，模型可基于结果继续 |
| 相同有序工具批次达到配置上限且未取消 | Err(ToolLoop { limit }) | Assistant 保留；该批全部补 tool_loop，不执行；设置 Failed 并发 RunFinished |
| 工具运行期间 token 取消 | Ok(Cancelled)（清理和配对后） | 当前工具返回实际结果，剩余调用补 cancelled |
| 助手不再调用工具 | Ok(Completed)，或取消已观察时 Cancelled | 完整 Assistant 已提交 |
| 最后一步仍产生工具调用且未取消 | Ok(StepLimit) | 所有调用都有结果，无额外模型总结 |
| 调试暂停处取消 | Ok(Cancelled) | 已提交结果保留，待执行调用补 cancelled；不再等待人工释放 |

工具返回一个 code 为 `cancelled` 的输出，并不会单独让 Agent 结束；是否结束仍由 token 决定。
同理，工具 timeout 不是 Agent 的总超时。`RunOutcome::Failed` 作为状态/事件使用，当前错误路径返回 Err，
不是 `Ok(Failed)`。宿主可以有更高优先级的输出错误，见 [CLI 设计](../cli/design.md)。

## 取消与中断边界

Model 调用置于 biased select，取消优先；其实现必须允许 future 被丢弃且无孤立副作用任务。
Tool 调用不放在外层 cancel-select 中，避免丢弃执行 future 造成子进程/文件 worker 失去清理 owner。
工具内 token check 和清理实现的边界见[文件设计](../filesystem-tools/design.md)与[Shell 设计](../shell-tool/design.md)。

宿主正确用法是发 token 后继续 await run。若直接 timeout/drop/task.abort，Running/Paused 状态不会自动改为 Finished，
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
| 相同批次忽略 ID/对象键序；整批不执行、失败优先于步数上限、下次 run 计数重置 | `repeated_tool_batch_closes_every_call_without_executing_the_batch` |
| 名称、参数、数量、顺序变化打断重复 | `tool_batch_changes_break_the_repeat_streak` |
| 默认关闭；同批 32 个相同调用只计一次 | `tool_repeat_guard_is_disabled_by_default_and_counts_batches` |
| 核心上限 1 阻止首批；调试 Finish 闭合整批、无工具动作；无工具回答不受保护影响 | `tool_repeat_guard_debug_finish_closes_the_batch_without_tool_actions` |
| Finish 暂停、部分/全部保护结果提交后取消优先 | `cancellation_overrides_a_tripped_tool_repeat_guard_before_and_during_closure` |
| 立即完成的 32 个工具结果与宿主公平性 | [CLI 测试](../../../tests/cli_http.rs) `a_full_batch_of_immediate_tool_errors_does_not_overflow_output` |

尚无空输入、超过 32 个调用、所有 panic/drop 时点、超长会话的穷尽测试；重复保护不证明任务停滞或成功。
也无持久化恢复或并行工具合同。
修改时检查：step 计数定义、terminal 位置、所有调用配对、模型/工具不同的取消策略、
event sink 是否还非阻塞、同步完成路径是否仍让出调度。新并发或恢复能力先补跨模块设计与真实失败用例。
调试专属命令与安全边界验收集中在[debugger 设计](../debugger/design.md#测试与变更检查)，避免重复维护两套合同。
