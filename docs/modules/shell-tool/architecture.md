# Shell 工具：架构

本模块实现一个受超时和输出保留上限约束的 Unix shell 调用，不是终端服务或 sandbox。
接口与错误路径见[设计文档](design.md)，项目导航见[文档索引](../../README.md)。

## 职责与非职责

`ShellTool` 拥有工作区路径与每次执行时限，将一个 `bash` 工具调用转换为子进程和管道操作。
它负责启动 shell、同时读取 stdout/stderr、等待完成，并在取消/超时/正常完成后清理进程组。
不负责批准命令、分析命令安全性、提交会话历史、提供交互 stdin、PTY 或可恢复后台服务。
工具事件中的命令由 Agent 记录；ShellTool 的 `description` 用于输入说明，不直接产生额外事件。

## 上下游与主要路径

```text
CLI --allow-shell / --tool-timeout / --workspace
  → ToolRegistry（Shell 授权）
  → ShellTool::execute
      → 严格解析 ShellInput → run_unix
          → /bin/bash + 独立进程组 + 两条管道
          → 并发轮询 child.wait / capture(stdout) / capture(stderr)
          → 正常、取消或超时 → killpg → 必要时等待 child
      ← ToolOutput
  → Agent 提交 Message::Tool → 模型或事件输出消费者
```

授权由 [Registry](../tool-runtime/architecture.md)检查，副作用生命周期由 ShellTool 负责。
[Agent](../agent/architecture.md)不通过 `select!` 丢弃正在执行的工具，而是传递取消 token 后继续 await。
[CLI](../cli/architecture.md)收到 Ctrl-C 后同样保持运行 future 被轮询，让清理与结果闭合完成。

## 所有权与资源生命周期

`ShellTool` 保存 owned `Workspace` 和 `Duration`，不保留上一次子进程或会话状态。
每次 `run_unix` 独占 `tokio::process::Child`、取出的两条输出管道及 `ProcessGroup(pid)` guard。
`process_group(0)` 让新 shell 成为独立组的组长；取消时可同时终止未脱离该组的后代。
stdin 是 null，shell 不会从宿主的输入流读取交互内容。

`tokio::join!` 同时轮询等待进程和两条管道，外层 `select!` 同时等待取消和 deadline。
这里没有 spawn 出独立 async reader 任务，因此不需要额外任务句柄或 reader join 管理器。
管道达到输出保留上限后仍继续读取，避免把输出限长误变成进程背压死锁。

在正常路径，等待包含进程退出和两条管道 EOF；随后仍发送进程组 SIGKILL 清理剩余组成员。
在取消/超时路径，读管道 future 被丢弃，先杀进程组，再 await 直接子进程的退出。
`ProcessGroup::Drop` 尝试再次杀组，`kill_on_drop(true)` 是直接子进程的兜底；都不能替代完整 await。
显式 killpg 或 wait 失败会返回 `cleanup_failed`，不能承诺操作系统清理已成功。

## 关键取舍

采用 `/bin/bash --noprofile --norc -c`，避免用户 profile/rc 改变命令初始化行为。
移除 `BASH_ENV` 和 `ENV`，也移除三种已知模型 API key 环境变量；未构建完整环境白名单。
当前只支持 Unix 实现；库在非 Unix 返回 `unsupported_platform`，CLI 本身仅支持 macOS/Linux。

stdout/stderr 各保留前 32 KiB 原始字节，并继续排空；输出最终按有损 UTF-8 转换。
不把 stderr 非空当成执行失败；依据进程退出状态判断成功。
非零退出用 `command_failed`，把 exit code 和两路输出作为 JSON 文本放进错误 message。
取消/超时只返回终结错误，不回传当时已读到的部分输出。

## 安全与维护影响

工作区只设置初始 cwd；命令可以 `cd`、访问绝对路径、使用网络并以宿主权限执行其他程序。
`allow_shell` 不受 `allow_write` 的文件工具授权约束，不能把 shell 当成只读执行接口。
环境变量移除只减少特定意外泄露，不阻止读取文件凭据、访问其他环境变量或启动外部程序。
进程可以主动脱离进程组；严格后代隔离或资源配额需要独立 sandbox/容器执行后端。

后台后代若保留 stdout/stderr，EOF 未到就不会正常完成，最终可能命中 deadline。
因此“清理成功后台进程”不等于支持后台服务会话；工具说明明确要求不启动后台服务。
扩展 PTY、stdin 或持续会话需要新的资源 owner 与关闭协议，不能只新增一个执行参数。
改变取消、输出格式或清理顺序时，应同时检查 Agent 的结果闭合和 CLI 的信号/退出码行为。

## 源码与验证入口

- [src/tools/shell.rs](../../../src/tools/shell.rs)：execute、进程组 guard、并发管道读取。
- [tests/tools.rs](../../../tests/tools.rs)：大 stderr、非零退出、超时和 token 取消的真实进程检查。
- [tests/cli_http.rs](../../../tests/cli_http.rs)：真实 Ctrl-C 到 shell 清理与 NDJSON 终结结果。
- [文件工具架构](../filesystem-tools/architecture.md)：工作区路径检查与 shell cwd 的区别。
