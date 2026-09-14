# CLI 宿主设计

状态：已实现（M0 + 原生调试/Lab/学习宿主）；对应 [src/main.rs](../../../src/main.rs)、[debug_input.rs](../../../src/debug_input.rs)。
组合和 owner 见 [架构文档](architecture.md)，返回 [文档导航](../../README.md)。

## 公共入口

`turnforge run <PROMPT> [OPTIONS]` 运行一个 turn，可能包含多次模型请求和工具调用。
`turnforge tools [OPTIONS]` 输出当前授权的工具定义 JSON 数组，不调用模型。
`run -` 从 stdin 读取 UTF-8 prompt；其它字符串按字面传入，不读取文件或 stdin。
`turnforge debug <PROMPT> [OPTIONS]` 复用 run 配置，首次模型请求前暂停，stdin 专用于控制。
`debug -` 是配置错误（退出 1），不尝试读取 prompt；普通 run 的 stdin 行为不变。
`turnforge lab [--case chat|read|write] [--auto] [--ollama-url ORIGIN] [--lesson LESSON]` 为固定合成实验。
`turnforge learn [LESSON]` 无模型地输出六阶段路线/单课。两者不使用下表的环境配置；Lab 参数详见[Lab 设计](../harness-lab/design.md)。

| 配置 | 优先级 / 缺省 | 校验与作用域 |
|---|---|---|
| model | `--model` > `TURNFORGE_MODEL`；无默认 | run/debug 必需；provider 拒绝空白 ID |
| base URL | `--base-url` > `TURNFORGE_BASE_URL` > `https://api.openai.com/v1` | provider 附加 `/chat/completions` |
| API key | 非空 `TURNFORGE_API_KEY` > 非空 `OPENAI_API_KEY` > 无鉴权 | 无 CLI flag；不是自动凭据发现 |
| workspace | `--workspace` > `.` | `Workspace::new` 规范化并验证目录 |
| 写授权 | `--allow-write`；缺省 false | 仅文件写能力 |
| shell 授权 | `--allow-shell`；缺省 false | 宿主权限的非沙箱 shell |
| max steps | `--max-steps` > 20 | `NonZeroU32`；模型请求次数上限 |
| 请求超时 | `--request-timeout` > 120 秒 | 整数 1–3600；每次 HTTP 请求含流读取 |
| 工具超时 | `--tool-timeout` > 30 秒 | 整数 1–3600；只配置 shell，含排空 |
| 输出模式 | `--json`；缺省 false | NDJSON 或文本增量 |

除表中 model、base URL、key 的环境配置外，没有自定义配置文件或其它参数环境绑定。
“非空 key”只检查空字符串，不 trim 空白；格式是否可作为 Header 由 provider 校验。
HTTPS 或 loopback HTTP、拒绝 URL 凭据/query/fragment 等规则见 [模型设计](../model/design.md)。

## 建立运行的顺序

1. Clap 解析子命令、参数、环境；参数错误可能直接退出，不经过 `execute`。
2. 建立工作区和完整工具 Registry；`tools` 此时同步打印并返回。
3. run 读取 prompt；stdin 最多读取 1 MiB + 1 字节，超限拒绝，非 UTF-8 拒绝。
4. 读取 key，构造 OpenAiModel 和 AgentConfig，在 system 末尾附加工作区与权限说明。
5. 构造 Agent、token、64 项 channel；并发轮询 run/writer，监听 Ctrl-C。

debug 在建立 registry 前拒绝 prompt `-`；配置 provider/Agent 后建立 Controller/Session 和 DebugInput，
再并发 run/writer/control。input 构造或 fd 类型校验失败时还没有开始 Agent run。

Lab 先 `lab::prepare` 完成本机预检和场景，再构造固定 RunArgs：max_steps 4、每模型请求 90 秒、
临时 workspace、仅 write 场景文件写授权、始终无 shell、无 key，provider 使用 `OpenAiModel::new_local`。
HostMode::Lab 持有 PreparedLab、automatic 和 lesson；自动模式不创建 DebugInput。
`learn` 不创建 registry、provider 或 Agent，只把 catalog 放入容量 1 的 OutputItem channel，经 forward 写出并返回。
模型预检与 learn 输出位于 run/signal 组合之前；不要将运行期 Ctrl-C 清理保证外推到这些前置路径。

stdin 使用同步 `Read::read_to_end`，发生在协作取消链路之前；当前没有 stdin deadline。
不应把后面的 Ctrl-C 清理保证外推到等待 stdin EOF、配置前置阶段或 `tools` 输出。

## 调试控制输入

`DebugInput::stdin()` 获得 owned dup File；TTY、FIFO 和 Unix socket 走 AsyncFd/O_NONBLOCK；
`/dev/null` 走立即 EOF。普通文件及其它不支持的设备报错，提示使用 pipe，不退回同步阻塞读取。
Descriptor Drop 尝试恢复原 flags；因 stdin/stdout 可能共享 open-file description，input 创建先于 writer、
释放晚于 writer。控制读取不 spawn worker，无需靠进程退出回收后台读任务。

输入用固定 1024 字节读取缓冲，每行最多 4096 字节（不含 LF，包含末尾 CR），有效 UTF-8。
解析时 trim 首尾空白，支持 CRLF；原始 debug 空行属于非法命令，Lab 空行代表下一步。
EOF 前无 LF 的残行先尝试解析，之后读取 EOF 取消。
支持 `step ID`、`continue ID`、`inspect ID`、`pause`、`cancel`；ID 为十进制 u64，缺失、符号或多余词拒绝。
JSON 一行一个 `DebugCommand`，结构校验归[debugger 协议](../debugger/design.md)，未知字段拒绝。
诊断使用固定简短文本，不把无效输入原文回显到 stderr。

control 用 biased select，优先观察 `run_done`，然后 cancel，再读下一命令。
Step/Continue/Inspect/Pause 通过 `try_send`；Cancel 和 EOF 直接取消 token。
语法/UTF-8/超长行/IO 错误或控制队列 full/closed 先取消，再等待 run/writer 清理，最终作为 input error 返回。
已入队但 pause ID 失效是库级命令拒绝事件，不是输入语法失败；不会退出或自动释放另一暂停点。
run 完成时 `run_done.cancel()` 停止还开放的 stdin，避免等待用户再按回车才能退出。
EOF 与完成竞速时不改写已经返回的 outcome；不能把“关闭 stdin”绝对等同于退出 130。

### Lab 快捷控制和课程

`next_command(lab_pause: Option<&AtomicU64>)` 返回 `ControlCommand::{Debug,Help,Learn,NotPaused}`。
None 保留原始严格 debug 解析；Lab 另外支持 Enter/n/next/step、c/continue、i/inspect、p、q、h/help/?、l/learn。
完整 `step ID`/`continue ID`/`inspect ID`/`pause`/`cancel` 和 JSON 仍可走严格调试解析。
h/l 仅排队 Notice，不发 DebugCommand；未选课程的 l 返回总路线，已选返回该课。

`observed_pause` 由事件 sink 从 DebugPaused 设置 ID、DebugResumed/RunFinished 清零。
读取一批字节时保存 `buffer_pause`；每一行的首字节确定 `line_pause`，跨 read 时保留，直到解析完成。
因此缓冲里的多个 n 不会因较晚解析而被重新绑定后来的暂停；输入时 ID 为 0 则返回 NotPaused 提示，不推进。
这只是快捷键相关性，最终命令仍经过库的 pause ID/epoch 校验。操作不会修改 transcript 或提升权限。

自动 Lab 在 sink 收到 DebugPaused 后直接提交带该 ID 的 Step，包括最后 Finish；没有 stdin 控制 future。
自动发送错误取消并作为运行错误返回；不提前生成下一 ID，不用 Continue 冒充逐动作调试。

## 事件与输出合同

回调把 owned `Event` 交给 `sender.try_send`，失败时合并为 `output queue full or closed`。
第一次失败保存 `output_error` 并设置 token；之后所有事件不再尝试入队，包括 RunFinished。
Agent 仍被 await，以完成其内存状态和工具清理；run 完成设置 run_done。
Lab 此时对 Completed 运行执行 PreparedLab::verify，随后排队 PASS/FAIL/CANCELLED Notice，再 drop 自己的 Sender。
control 还持有一份用于 h/l 的 Sender；它被 run_done 停止并释放后，writer 才能收到 EOF 并排空。
writer 对任何输出错误调用 token.cancel；receiver 随退出释放，使后续发送可能发现 closed。

`--json` 把 Event 序列化成一行一个对象；文本模式写 Text delta，并在 RunFinished 写换行。
调试文本模式另外显示暂停/Inspect 的完整 pretty JSON 快照和命令提示，显示恢复及命令拒绝通知；
普通 run 不发这些事件，因此文本行为保持不变。
所有 Text delta 都会显示，包含中间步骤的文本，不是只显示最后一条已提交 Assistant。
stderr 的业务错误形如 `turnforge: {error}`；没有额外 JSON 错误对象或独立工具进度日志。
Lab 使用独立文本投影：模型/工具/结果/暂停摘要，i 显示完整快照；课程、帮助和最终验收报告走相同有界 writer。
这些 Notice 属于 binary 的 OutputItem，不新增库 Event，也不混入普通 run/debug 的 NDJSON。
正常事件顺序属于 [协议设计](../protocol/design.md)，交付失败语义属于 [输出设计](../event-output/design.md)。

## join、signal 与错误优先级

`execution = join!(run, writer, control)` 必须等待三者结束；普通 run 的 control 立即成功返回。
writer 不会因 run 结束而被直接丢弃，control 则由 run_done 明确停止读取。
Ctrl-C 分支先取消，再等待 execution，最后检查 signal 自身结果；没有第二阶段强制终止 timer。
外层 select 无偏好分支，完成与信号同时就绪时不能假定取消胜出。

完成后的实际判断顺序如下，前一项错误会遮蔽后一项的返回值：

1. 若进入 signal 分支且 `ctrl_c()` 返回错误，清理结束后 `signal?` 返回该错误。
2. `output_result?`：writer 初始化、序列化或写入错误。
3. 保存的 queue error：满或关闭导致的交付故障。
4. `input_result?`：调试输入或控制 channel 错误。
5. `result?`：AgentError；否则将 RunOutcome 映射退出码。

| 路径 | 退出码 | 注意 |
|---|---|---|
| Completed / tools / help / version | 0 | Completed 只表示模型自然结束，不验证用户任务成败 |
| 运行、配置、输出、调试输入、AgentError / Failed | 1 | stdout 的 RunFinished 可能缺失或与宿主失败不同 |
| StepLimit | 2 | 最后一步工具仍执行并提交结果 |
| Clap 参数错误 | 2 | 同码但不产生 Agent run；用 stderr/事件区分 |
| Cancelled | 130 | 输出/输入故障先返回 1；不能仅凭 SIGINT 或输入 EOF 推导 130 |

Lab 的 Completed 还需 oracle 成功才返回 0；oracle 失败置运行结果为 Err，退出 1。
PASS 先被排队，宿主仍需等待 writer；看到 PASS 但后续输出/输入错误时仍可能非零退出。
Help/Learn/NotPaused 输出排队失败作为 control error 取消，仍服从上述优先级。

stdout 故障时终态可能截断；下游必须同时检查进程退出码与完整记录，不能只等 RunFinished。
取消不是回滚，正常结束或错误返回都不撤销已经发生的文件/命令副作用。

## 验证与缺口

主要证据：[tests/cli_http.rs](../../../tests/cli_http.rs) 的真实 TCP → CLI → 文件/进程链路。
`fragmented_tool_arguments_round_trip_to_real_file_and_model` 覆盖模型参数、JSON 事件和工具结果回传。
`denied_tool_is_not_advertised_or_executed_but_returns_result` 覆盖默认只读和双重权限检查。
`cli_step_limit_has_distinct_exit_and_a_closed_tool_result` 覆盖退出 2 与工具结果闭合。
`actual_ctrl_c_cancels_shell_and_preserves_event_closure` 覆盖真实 SIGINT、退出 130 与 shell 后代清理。
`tools_and_help_work_without_a_model_or_key` 覆盖无需模型的入口；其它用例覆盖失败与输出堵塞。

同文件的 `debug_cli_steps_real_tool_effects_and_finishes_with_stdin_open` 覆盖真实文件副作用时点、
Inspect、旧 ID 拒绝、最终确认及开放 stdin 的正常退出；`debug_cli_eof_and_invalid_input_cancel_pending_tools`
覆盖 EOF/非法输入取消和工具配对；`debug_cli_sigint_during_shell_awaits_cleanup_and_closes_calls`
覆盖调试下真实进程清理；`debug_cli_rejects_stdin_prompt_and_regular_file_control_input` 覆盖输入通道前置拒绝。
`debug_input.rs` 的 `debug_json_commands_require_exact_fields` 覆盖 CLI 到命令类型的严格 JSON 解析，
尤其 Pause/Cancel 多余字段不能被静默接受；调度协议验收另见[debugger 设计](../debugger/design.md#测试与变更检查)。

[tests/lab_cli.rs](../../../tests/lab_cli.rs) 的
`lab_default_read_shortcuts_pause_inspect_and_finish_with_stdin_open` 覆盖 Lab 真实文本交互、课程显示不推进及开放 stdin 结束，
`lab_auto_chat_ignores_stdin_and_inherited_cloud_configuration` 覆盖自动模式不依赖 stdin 和云配置，
`lab_eof_and_quit_cancel_without_running_a_model` 覆盖初始安全点的 EOF/q；
`learn_catalog_and_lessons_need_no_llm` 覆盖六课程的无模型入口。
`debug_input.rs::tests::buffered_shortcuts_keep_the_pause_at_input_read_time` 补充同 buffer 多个 n 不重绑定新 ID 的状态边界。
预检、oracle 的证据另见[Lab 设计](../harness-lab/design.md#验证与缺口)。

当前没有专门的 flag/env 优先级矩阵、stdin 超限/编码、signal 注册失败或错误同时发生测试。
这里的 stdin 超限/编码缺口包括普通 prompt 与调试行输入；已有少量手工检查不代替自动化边界矩阵。
配置前同步 stdin 无超时；慢本地/网络文件系统可能拖延清理；这些不是已经验证的取消上界。

## 变更检查清单

- 新参数是否明确默认值、环境优先级、校验责任、秘密数据暴露风险？
- 新退出路径是否仍等待 run/writer，并保留正确的输出与运行错误优先级？
- 新宿主输出是否与 `tools` 同步输出区分，且不把日志混进 NDJSON？
- 并发或审批改动是否有 owner、取消、关闭与等待位置，并有真实子进程测试？
- 调试输入是否严格限长、遇错清理、run 结束可停止读取，并正确恢复共享 fd flags？
