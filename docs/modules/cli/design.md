# CLI 宿主设计

状态：已实现（M0）；对应 [src/main.rs](../../../src/main.rs)。
组合和 owner 见 [架构文档](architecture.md)，返回 [文档导航](../../README.md)。

## 公共入口

`turnforge run <PROMPT> [OPTIONS]` 运行一个 turn，可能包含多次模型请求和工具调用。
`turnforge tools [OPTIONS]` 输出当前授权的工具定义 JSON 数组，不调用模型。
`run -` 从 stdin 读取 UTF-8 prompt；其它字符串按字面传入，不读取文件或 stdin。

| 配置 | 优先级 / 缺省 | 校验与作用域 |
|---|---|---|
| model | `--model` > `TURNFORGE_MODEL`；无默认 | run 必需；provider 拒绝空白 ID |
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

stdin 使用同步 `Read::read_to_end`，发生在协作取消链路之前；当前没有 stdin deadline。
不应把后面的 Ctrl-C 清理保证外推到等待 stdin EOF、配置前置阶段或 `tools` 输出。

## 事件与输出合同

回调把 owned `Event` 交给 `sender.try_send`，失败时合并为 `output queue full or closed`。
第一次失败保存 `output_error` 并设置 token；之后所有事件不再尝试入队，包括 RunFinished。
Agent 仍被 await，以完成其内存状态和工具清理；run 返回后 drop Sender，让 writer 收到 EOF。
writer 对任何输出错误调用 token.cancel；receiver 随退出释放，使后续发送可能发现 closed。

`--json` 把 Event 序列化成一行一个对象；文本模式只写 Text delta，并在 RunFinished 写换行。
所有 Text delta 都会显示，包含中间步骤的文本，不是只显示最后一条已提交 Assistant。
stderr 的业务错误形如 `turnforge: {error}`；没有额外 JSON 错误对象或工具进度日志。
正常事件顺序属于 [协议设计](../protocol/design.md)，交付失败语义属于 [输出设计](../event-output/design.md)。

## join、signal 与错误优先级

`execution = join!(run, writer)` 必须等两者结束；writer 不会因 run 结束而被直接丢弃。
Ctrl-C 分支先取消，再等待 execution，最后检查 signal 自身结果；没有第二阶段强制终止 timer。
外层 select 无偏好分支，完成与信号同时就绪时不能假定取消胜出。

完成后的实际判断顺序如下，前一项错误会遮蔽后一项的返回值：

1. 若进入 signal 分支且 `ctrl_c()` 返回错误，清理结束后 `signal?` 返回该错误。
2. `output_result?`：writer 初始化、序列化或写入错误。
3. 保存的 queue error：满或关闭导致的交付故障。
4. `result?`：AgentError；否则将 RunOutcome 映射退出码。

| 路径 | 退出码 | 注意 |
|---|---|---|
| Completed / tools / help / version | 0 | Completed 只表示模型自然结束，不验证用户任务成败 |
| 运行、配置、输出、AgentError / Failed | 1 | stdout 的 RunFinished 可能缺失或与宿主失败不同 |
| StepLimit | 2 | 最后一步工具仍执行并提交结果 |
| Clap 参数错误 | 2 | 同码但不产生 Agent run；用 stderr/事件区分 |
| Cancelled | 130 | 输出故障先返回 1；不能仅凭 SIGINT 推导 130 |

stdout 故障时终态可能截断；下游必须同时检查进程退出码与完整记录，不能只等 RunFinished。
取消不是回滚，正常结束或错误返回都不撤销已经发生的文件/命令副作用。

## 验证与缺口

主要证据：[tests/cli_http.rs](../../../tests/cli_http.rs) 的真实 TCP → CLI → 文件/进程链路。
`fragmented_tool_arguments_round_trip_to_real_file_and_model` 覆盖模型参数、JSON 事件和工具结果回传。
`denied_tool_is_not_advertised_or_executed_but_returns_result` 覆盖默认只读和双重权限检查。
`cli_step_limit_has_distinct_exit_and_a_closed_tool_result` 覆盖退出 2 与工具结果闭合。
`actual_ctrl_c_cancels_shell_and_preserves_event_closure` 覆盖真实 SIGINT、退出 130 与 shell 后代清理。
`tools_and_help_work_without_a_model_or_key` 覆盖无需模型的入口；其它用例覆盖失败与输出堵塞。

当前没有专门的 flag/env 优先级矩阵、stdin 超限/编码、signal 注册失败或错误同时发生测试。
配置前同步 stdin 无超时；慢本地/网络文件系统可能拖延清理；这些不是已经验证的取消上界。

## 变更检查清单

- 新参数是否明确默认值、环境优先级、校验责任、秘密数据暴露风险？
- 新退出路径是否仍等待 run/writer，并保留正确的输出与运行错误优先级？
- 新宿主输出是否与 `tools` 同步输出区分，且不把日志混进 NDJSON？
- 并发或审批改动是否有 owner、取消、关闭与等待位置，并有真实子进程测试？
