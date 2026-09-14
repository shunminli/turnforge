# 事件输出设计

状态：已实现（M0）；实现：[src/output.rs](../../../src/output.rs)。
跨模块关系见 [架构文档](architecture.md)，返回 [文档导航](../../README.md)。

## 接口与字节投影

`forward(events: Receiver<OutputItem>, mode: DisplayMode, cancel: &CancellationToken) -> io::Result<()>`。
函数获得 receiver 所有权，借用取消 token；自身不修改 token，出错后的取消由 CLI writer 包装层负责。
它首先创建 Output，再依次 `recv`、编码并完整写入当前事件，然后读取下一事件。
`OutputItem::{Event(Event), Notice(String)}` 是 binary 内桥接，`DisplayMode::{Json,Text,Lab { lesson, automatic }}` 控制投影。
Notice 直接写传入文本，当前只有 learn/Lab 文本路径生产它；普通 run/debug 的 Json 路径不排队 Notice。
因此原有 NDJSON 形状不变，不能给 Json 路径随意新增 Notice 后仍宣称纯 Event stream。

| 模式 / 事件 | 写入内容 |
|---|---|
| JSON / 任意 Event | `serde_json::to_vec(event)` 后追加一个 LF |
| 文本 / ModelDelta::Text | 原始 UTF-8 text 字节，不自动插入间隔 |
| 文本 / RunFinished | 一个 LF，不显示 outcome |
| 文本 / DebugPaused | 暂停 ID、pretty JSON snapshot、该 ID 的命令提示 |
| 文本 / DebugSnapshot | Inspect 快照 ID、pretty JSON snapshot、命令提示 |
| 文本 / DebugResumed | `[debug resumed #ID]` 通知 |
| 文本 / DebugCommandRejected | 被拒绝命令的 JSON 和 reason，不输出用户无效输入原文 |
| 文本 / 其它事件 | 忽略，不调用 write |
| Lab / StepStarted、ToolStarted、Tool committed | 模型进度、工具及参数预览、工具结果预览 |
| Lab / DebugPaused | ID、point、next、消息/待调用计数；可选 learning::hint；快捷键或 auto 提示 |
| Lab / DebugSnapshot | i 请求的完整 pretty JSON 快照 |
| Lab / DebugResumed、Rejected、RunFinished | 恢复、拒绝原因、运行 outcome；运行 outcome 不是 oracle 结果 |
| Notice | 原样 UTF-8 文本，承载 Lab 说明/帮助/课程/PASS/FAIL 或离线 learn |

普通 run/debug 不追加额外成功信息或 stderr 进度；Lab 的额外验收 Notice 由宿主生成，不由 writer 自行判断。
详细 Event 字段见 [协议设计](../protocol/design.md)。
调试提示与快照属于 stdout 文本投影；`--json` 不混入这些提示，而是直接序列化对应 Event。
文本 delta 与 MessageCommitted 不重复显示，但每个模型步骤的 Text delta 都可能显示。
JSON 不保证全行原子写入，尤其事件大于 pipe 缓冲时；断开/超时可能留下半行。
Lab 的参数/工具结果 preview 先序列化完整 JSON，再取前 1200 个 Unicode char，超过时追加截断提示。
此限额只控制显示篇幅，不限制序列化分配/原始快照大小；i 可查看完整快照，不进行脱敏。
自动模式也显示暂停摘要，但自动 Step 已由 sink 驱动；不能将 writer 显示时间当作 Agent 当时仍等待用户。

## 队列合同（CLI 对接）

容量在 [main.rs](../../../src/main.rs) 固定为 64 个 OutputItem，通过 `try_send` 入队；离线 learn 是 1 项。
full 与 closed 合并为同一诊断；首次失败后 sink 不再投递，并请求取消。
队列不是字节限额，没有 terminal 专用槽位，也不会重试失败事件。
run 完成后排队 Lab 验收报告并 drop 自己的 Sender；run_done 停止 control 后关闭其 Sender。
健康 writer 等全部发送者关闭，排空已有项后收到 None，返回 Ok。
取消不会关闭 receiver，也不会跳过排空；输出故障才会使 forward 提前返回。

## stdout fd 初始化与释放

1. `nix::unistd::dup(io::stdout())` 获得 owned fd，转成 File。
2. 检查 metadata：普通文件或 rdev 与 `/dev/null` 相同的字符设备走 `Output::File`。
3. 其它类型读取 `F_GETFL`，保存 OFlag；用 `F_SETFL(flags | O_NONBLOCK)` 设置非阻塞。
4. 把 `Descriptor { file, flags }` 注册进 Tokio `AsyncFd`，得到 Pollable。

pipe、TTY 等使用 Pollable；非普通文件并不必然受支持，AsyncFd 注册仍可能失败。
`Descriptor::drop` 尝试恢复旧 flags，因为 dup 与原 stdout 共享 open-file description。
恢复错误被忽略，强制进程终止也无法保证 Drop 运行；这不是故障恢复事务。
File 分支不设置 flags，也无恢复动作；Output 的所有 fd 均按 owned File 释放。

## 部分写入与 readiness

Pollable 的 `write_all` 持有尚未写完的 slice，等待 `fd.writable()`，再用 guard.try_io 写入。
写出 N 字节后推进 slice；WouldBlock 清理 readiness 后重试；其它 I/O 错误立即返回。
写出 0 字节视为 WriteZero；只有 slice 排空才返回成功。
这允许等待期间调度 run/signal；不会启动阻塞输出线程或丢失其 JoinHandle。
File 分支直接调用同步 `std::io::Write::write_all`，没有异步抢占点。

## 精确写入 deadline 语义

forward 为**每个要输出的 Event 或 Notice**创建并 pin 一个 write future，随后执行 biased select：

```text
cancel.cancelled() 就绪 → timeout(100ms, 同一个 write future)
否则与 timeout(2s, 同一个 write future) 竞速
```

正常情况下，当前事件写入受 2 秒 timeout 约束；不是整个队列或整个运行的 2 秒上限。
取消分支优先检查；若取消在写入期间发生，原 2 秒分支被丢弃，改为给同一未完成写入最多 100ms。
已写部分不重发；新预算从取消分支开始计时，不是从该事件最初写入时刻计时。
取消与 2 秒 timeout 同时就绪时，biased select 可以优先选择取消并给出新预算。
因此不能宣称“任何单事件总耗时严格不超过 2 秒”，也不是取两个绝对 deadline 的较小值。

token 已取消时，后续每个待写事件都会各自获得 100ms 尝试；不是全局 100ms 排空期限。
任一事件超时立即返回 TimedOut：`stdout stalled; output may be incomplete`，不会继续写后续事件。
等待 `events.recv()` 本身没有取消 select；即使 token 已取消，仍要等 run 关闭 Sender 或产生事件。
事件序列化发生在 timeout 之前，也不计入该写入预算。
调试 pretty JSON 同样在写入预算前构造，历史很长时的复制/序列化成本不受 2 秒写入 timer 限制。

对 File 分支，同步 write 一旦阻塞，Tokio timeout 不能中断系统调用。
普通本地文件和 /dev/null 的成功路径受支持；网络/特殊文件系统挂起不在取消时延保证范围内。
这些条件共同说明：本模块没有可用于推导整体退出时延的全局 deadline。

## 错误传播

创建 fd、metadata/fcntl/注册、JSON 序列化、实际 write、超时都可以返回 io::Error。
CLI 在 writer 返回错误时取消 run，等待 join，并让输出错误优先于 queue/Agent 错误。
详见 [CLI 设计](../cli/design.md)的错误优先级；此时通常退出 1，而非保证 130。
事件流是尽力交付的有序通知：下游要处理非零退出、EOF、半行和缺失 RunFinished。

## 验证与缺口

证据：[tests/cli_http.rs](../../../tests/cli_http.rs)，通过真实 CLI/stdout fd 而非模拟 writer。
`blocked_stdout_does_not_block_ctrl_c_or_process_exit` 保持管道不读，发 SIGINT 后验证非零退出与 stalled 诊断。
`stdout_can_be_redirected_to_a_regular_file_or_dev_null` 覆盖本地普通文件和 /dev/null 成功路径。
`a_full_batch_of_immediate_tool_errors_does_not_overflow_output` 覆盖 32 个立即失败工具的事件排空。
`actual_ctrl_c_cancels_shell_and_preserves_event_closure` 验证健康消费者下取消事件闭合，不代表故障端必达。
`debug_cli_steps_real_tool_effects_and_finishes_with_stdin_open` 覆盖调试 NDJSON 事件与输入命令的交互，
`debug_cli_sigint_during_shell_awaits_cleanup_and_closes_calls` 覆盖调试取消的健康输出端闭合。

[lab_cli.rs](../../../tests/lab_cli.rs) 的 `lab_default_read_shortcuts_pause_inspect_and_finish_with_stdin_open`
通过真实文本 stdout 检查暂停/Inspect/课程与最终报告；`learn_catalog_and_lessons_need_no_llm`
覆盖离线 Notice 交付。它们不证明所有模式的 stdout 故障分支或自动化屏幕体验。

尚无专门测试强制 queue full、broken pipe、TTY flag 恢复、两种 timer 同时就绪或普通 run 文本模式。
现有测试不是全局延迟证明，也没有覆盖网络文件系统挂起或恢复 flags 失败。

## 变更检查清单

- 输出路径是否仍区分可 poll fd 与同步文件，且共享 flags 变更有恢复 owner？
- 新限额按事件数、字节数还是时间计算，是否计入编码与全部排空？
- 超时/取消是否继续同一写入、会否重发部分字节，是否错误承诺终态必达？
- writer 结束是否能取消 run；run 结束是否关闭 Sender；是否仍等待双方清理？
- 测试是否真实保持消费者不读，避免靠测试主动排空隐藏阻塞问题？
