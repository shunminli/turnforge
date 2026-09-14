# 原生语义调试：设计

状态：已实现；测试覆盖和证据缺口见文末。
证据入口：[debug.rs](../../../src/debug.rs)、[agent.rs](../../../src/agent.rs)、
[event.rs](../../../src/event.rs)。配套：[架构](architecture.md) · [指南](../../debugging.md)。

## 库入口

| API | 合同 |
|---|---|
| `debug_channel(&CancellationToken)` | 建立一个控制会话，返回 `(DebugController, DebugSession)` |
| `DebugController::try_send(DebugCommand)` | 非阻塞提交命令；返回 `Result<(), DebugControlError>`；入队不等于 Agent 已接受 |
| `DebugController::cancel()` | 直接请求 token 取消，不依赖命令队列 |
| `Agent::run_debug(input, session, emit)` | 消费 session，独占借用 Agent，返回与普通 run 相同的结果类型 |

Controller 不可 Clone；Drop 请求取消。Session 只用于一次 run，没有跨 run 恢复或重绑定方法。
每个 run 使用新 CancellationToken；channel 克隆传入的同一 token，不建立子取消作用域。
因此 Controller Drop 也会取消该 token 的其它观察者；宿主应持有 controller 到 run/输出清理完成。
普通 `Agent::run(input, token, emit)` 的签名、非调试事件和自动运行行为保持不变。
两种入口使用同一个模型/工具循环，不能通过调试模式跳过 registry 权限或 Assistant 校验。

## 命令与拒绝

`DebugCommand` 的 serde 标签为 `command`，snake_case，并拒绝未知字段：

```json
{"command":"step","pause_id":1}
```

| 命令 | 合法时点与含义 |
|---|---|
| `step { pause_id }` | 当前 Paused 且 ID 相符；释放一个模型/工具/结束动作，到下一安全点再暂停 |
| `continue { pause_id }` | 当前 Paused 且 ID 相符；自动运行，直到 Pause、取消或终态 |
| `inspect { pause_id }` | 当前 Paused 且 ID 相符；发当前快照副本，不推进动作或产生新 pause ID |
| `pause` | 请求在下一个安全边界暂停；不能抢占正在执行的工具或模型；已经暂停时拒绝 |
| `cancel` | 立即设置 token；进入原有协作清理，不等待下一条命令 |

命令 channel 最多容纳 8 项。`try_send` 的队列满/已关闭是宿主即时错误；成功入队的命令仍会在
Agent 消费时校验运行状态、pause ID 和入队时的暂停 epoch。队列错误变体为 `DebugControlError::Full / Closed`；库不因 Full 自动取消，
CLI 则把它作为输入故障取消。`try_send(Cancel {})` 直接设置 token 并成功返回，即使队列满或 session 已结束。
拒绝通过 `debug_command_rejected` 事件反馈，不能理解成成功推进。
旧 ID 不因数字“看起来像下一步”被修正；消费者每次都应使用最近 `debug_paused` 提供的 ID。
输入字符串转 JSON/枚举由 CLI 或其它宿主负责；库只处理类型化命令。
到达边界时最多排空 8 条运行中积压的命令：Pause 设置单步模式，其它带 ID 的命令拒绝为
`not paused; wait for debug_paused`。不能在首次 `debug_paused` 前预送 `step 1` 来释放首个暂停点。
这个 bounded drain 只限制每次处理量；同步拒绝回调或并发发送者可能补位，不能靠“排空过”证明剩余命令安全。

真正的相关性约束由私有 `QueuedCommand { command, paused_epoch }` 承载：

1. `debug_channel` 创建值为 0 的 `Arc<AtomicU64>`；0 表示没有活动暂停。只有 Session/其 guard 写入，Controller 只读。
2. `try_send` 在入队前以 Acquire 读取当前 epoch 并保存到 envelope，不信任命令自报的 ID 足够证明提交时点。
3. Session 设置 Paused 后、发 `DebugPaused` 前以 Release 发布当前 pause ID，使同步事件消费者也能发送有效命令。
4. Step/Continue/Inspect 被消费时，命令 ID 和 envelope epoch 必须都等于当前 pause ID；预送或旧命令不能跨越新暂停。
5. `PauseEpoch` RAII guard 在恢复事件前显式 Drop；取消、正常返回、panic unwind 或丢弃等待 future 时也 Drop，清零 epoch。
   清零仅关闭命令相关性窗口，不恢复已丢弃 run 的 Agent 状态，也不承诺 abort/强杀后的恢复。

已经暂停但 ID/epoch 不匹配时拒绝为 `stale pause_id or command submitted outside this pause`，Pause 拒绝为 `already paused`。
epoch/envelope 不属于公共 API 或 JSON wire，没有增加宿主提供的新字段。
reason 是诊断 String，不是新的封闭错误 enum；拒绝后继续等待有效命令，处理后 yield 以保留取消/输出调度机会。

## 暂停点、下一动作与步骤

| `DebugPoint` | 当时已提交的事实 | 可能的 `DebugAction` |
|---|---|---|
| `BeforeModel` | 首次 User 已提交，尚无模型请求 | `Model { step: 1 }` |
| `AfterModel` | 完整且通过校验的 Assistant | 首个 `Tool { index, call }` 或 `Finish { outcome: Completed }` |
| `AfterTool { index, call_id }` | 指定工具的真实结果；index 为当前批次从 0 开始 | 下一 Tool、下一 Model 或 `Finish { outcome: StepLimit }` |

初始安全点在第一次模型请求之前；模型和工具动作之后到下一安全点。
`BeforeModel` 只在初始使用；后续模型请求前的暂停点是上一批最后工具的 `AfterTool`，不会再生成一个冗余暂停。
一轮 `step` 命令与 Agent 的 `step` 字段不是同一计量单位：Agent step 仍从 1 开始统计模型请求。
工具批次依次处理，不因为一次模型返回多个调用就把整批当作一次调试动作。
最后的无工具回答也先停在 AfterModel；还需 Step 或 Continue 才从可观察现场进入 Completed。
达到 max_steps 时仍闭合该批全部结果，然后暂停在 Finish(StepLimit) 之前，不额外请求总结。

## 快照 v1

`DebugSnapshot` 使用 owned 数据，不是 `ModelRequest<'_>` 的引用视图：
`DebugPoint` 和 `DebugAction` 都用 `kind` 标签、snake_case；`AfterTool.index` 是 u32，pause ID 是 u64。

| 字段 | 含义 |
|---|---|
| `version` | 当前为 1，仅标记快照形状；不是完整远程 RPC 版本协商 |
| `pause_id` | 本 DebugSession 的暂停序号；不能跨运行当全局 ID |
| `step`、`point`、`next` | 当前模型步骤、安全边界及释放后动作 |
| `system`、`messages` | Agent 配置的 system 和全部已提交内存历史 |
| `tools` | 当前权限下向模型展示的工具定义 |
| `pending_calls` | 已提交但尚未产生结果的剩余调用，维持模型给出的顺序 |
| `max_steps` | 本 run 的模型请求次数上限 |

AfterModel 的 messages 可含尚未配对的工具调用；这是正在运行的合法暂停现场，不能把快照直接作为
完整新请求送给 provider。pending_calls 是检查辅助数据，不是允许用户替换的执行队列。
Inspect 不改变 transcript、pending_calls、工具权限或 pause ID。
AfterTool 的 `snapshot.step` 仍是刚执行工具所属的模型步骤，`next.model.step` 可以是下一步编号。
快照不包含 API key、原始 HTTP headers、模型端点完整配置，也不宣称足够确定性重放。
system/messages/arguments/results 仍可能含敏感业务内容；序列化没有自动脱敏或总字节上限。

## 状态与事件序列

`RunState` 增加 `Paused { step, pause_id, point }`。状态由 Agent 写入；暂停期间 run future 保留
对 Agent 的独占借用，宿主不能用 `state()` 绕过借用规则观察，实时观察应消费事件。

```text
RunStarted → User committed
  → Paused / debug_paused
  → 接受 Step 或 Continue / debug_resumed
  → Running(Model) → StepStarted → delta → Assistant committed
  → Paused / debug_paused
  → ... Running(Tools) → ToolStarted → Tool committed → Paused ...
  → 释放 Finish → Finished / RunFinished
```

Continue 模式不会为每个边界发一组假暂停/恢复事件；边界只在实际停下时发 `debug_paused`。
Inspect 发 `debug_snapshot { snapshot }`；命令拒绝发 `debug_command_rejected { command, reason }`。
Rust Event 中两种 snapshot 载荷是 `Box<DebugSnapshot>`，JSON 仍是直接嵌套对象；Box 不提供共享写入或总内存限额。
`debug_resumed { pause_id }` 指某暂停已被有效 Step/Continue 释放，不承诺下一个动作一定成功。
事件随原有 Event 输出路径交付，可能因宿主背压或输出错误丢失；不得从“未收到”推导“未执行”。

## 取消、失败与中断

- 等待命令可取消；token 优先，控制端 Drop/Cancel 都能结束等待。
- 模型执行中 Pause 不打断流；Cancel 沿原模型 drop-safe 路径结束，不提交半条助手消息。
- 工具执行中 Pause 不丢弃 future；Cancel 仍等待工具实际完成/清理，剩余调用补 cancelled 结果。
- AfterModel 暂停时取消，所有未执行调用补结果，不发虚构的 ToolStarted 或产生文件副作用。
- AfterTool 暂停时取消，已完成结果和外部副作用保留，只补剩余调用。
- 取消不再等待人工确认 Finish；完整协作路径写 Finished(Cancelled)，发一个 RunFinished。
- 非法模型响应沿原 Failed 路径结束；不会为无效 Assistant 暴露可执行的 AfterModel 暂停。
- 丢弃 Running 或 Paused 的 run future，后续 run/run_debug 返回 Interrupted；无恢复接口。

HTTP/shell 超时适用于已开始的操作；人工暂停本身没有自动超时。
正常取消仍要求宿主 await run。callback panic、进程强杀和存储系统挂起不属于自动恢复保证。

## 测试与变更检查

确定性库证据：[tests/debugging.rs](../../../tests/debugging.rs)。

| 合同 / 独立观察 | 测试符号 |
|---|---|
| 操作计数不提前增长、逐个工具、修改事件副本不改会话、Inspect、旧 ID、最终确认 | `step_inspect_and_stale_ids_preserve_exact_operation_boundaries` |
| 去掉调试事件后与普通 run 事件/历史一致 | `continue_has_the_same_transcript_and_non_debug_events_as_run` |
| 预送未来 pause ID 不得释放尚未发生的暂停 | `an_early_command_for_a_future_pause_cannot_release_that_pause` |
| drain 期间反复补位的未来 ID 仍不执行工具，新暂停可 Inspect/Cancel | `replenished_early_commands_cannot_cross_into_a_new_pause` |
| 运行中 Pause 等到完整模型响应 | `pause_during_a_model_waits_for_its_complete_response_boundary` |
| 工具预览取消不执行且补齐结果 | `cancelling_at_the_tool_preview_closes_calls_without_executing_them` |
| 队列已满仍能取消、必须等待工具 cleanup | `cancellation_bypasses_a_full_queue_and_awaits_active_tool_cleanup` |
| 控制端 Drop 取消初始暂停，不请求模型 | `dropping_the_controller_cancels_a_paused_run_without_a_model_request` |
| 丢弃暂停 future 后不可恢复 | `dropping_a_paused_future_does_not_allow_ambiguous_resume` |
| Step 不放宽权限或步数上限 | `stepping_never_grants_permission_or_bypasses_the_step_limit` |
| 全部命令 JSON round-trip、未知字段/缺失或非法 ID 拒绝 | `debug_commands_have_strict_json_fields_and_required_pause_ids` |

真实 binary/HTTP/文件与信号证据在 [tests/cli_http.rs](../../../tests/cli_http.rs)：

- `debug_cli_steps_real_tool_effects_and_finishes_with_stdin_open`：工具预览和旧 ID 后磁盘仍未写；正确一步后才写；stdin 保持开放也能退出。
- `debug_cli_eof_and_invalid_input_cancel_pending_tools`：EOF/非法输入取消已提交调用，不产生预览工具副作用。
- `debug_cli_sigint_during_shell_awaits_cleanup_and_closes_calls`：真实 SIGINT 后等待 shell 清理，并检查后代未延迟写文件。
- `debug_cli_rejects_stdin_prompt_and_regular_file_control_input`：拒绝 stdin 同时承载 prompt 和控制流，普通文件输入在 run 开始前失败。

固定本地模型的显式用例 `local_llm_debug_steps_through_read_and_final_answer` 位于
[tests/local_llm.rs](../../../tests/local_llm.rs)：逐暂停点发 Step，读取 prompt 不含的动态 marker，
验证真实结果快照及最终回答，并释放最终 Finish。它默认忽略，不能把普通 CI 的 ignored 当作通过。

这些测试不覆盖所有命令时序交错、TTY flags 恢复失败、超长历史资源耗尽或所有 provider 故障点。
Pause 正在执行的工具、跨运行重复 ID 的外部路由和远程断线恢复不由以上有限证据自动证明；
远程路由/恢复本身尚未实现。

修改时必须检查暂停事件相对 commit 的位置、pending_calls 与消息事实是否一致、工具清理是否仍被 await、
控制 receiver 关闭是否会永久等待、epoch 是否在所有离开暂停路径清零、入队绑定是否可能被绕过、
output backpressure 是否可反向取消，以及快照拷贝的成本。
远程连接、编辑历史、恢复与幂等副作用需要新合同；不能仅给现有 struct 再加一个布尔开关。
