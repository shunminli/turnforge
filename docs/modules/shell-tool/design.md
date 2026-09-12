# Shell 工具：设计

职责与资源 owner 见[架构](architecture.md)，导航见[文档索引](../../README.md)。
实现：[src/tools/shell.rs](../../../src/tools/shell.rs)；公共工具契约见[运行时设计](../tool-runtime/design.md)。

## API、输入与结果

`ShellTool::new(workspace: Workspace, timeout: Duration)` 保存固定工作区和每次执行的时限。
`Tool::definition()` 返回名称 `bash`、能力 `Shell`，schema 来自私有 `ShellInput`。
输入只接受必填字符串 `description`、`command`，未知字段拒绝；没有 cwd/env/stdin 参数。
`execute` 拒绝 trim 后为空的 command/description，以及构造时为零的 timeout。
description 不参与 shell 脚本，也不是安全决策；完整 ToolCall 由 Agent 的事件路径暴露给宿主。

成功返回 `ToolOutput::Ok { data }`：

```json
{"exit_code":0,"stdout":"...","stderr":"...","stdout_truncated":false,"stderr_truncated":false}
```

非零退出返回 `ToolOutput::Error { code: "command_failed", message }`，message 是上述结构的 JSON 字符串。
信号终止时 `status.code()` 可为 null；不能假设每次失败都有数字 exit code。
其他错误只返回错误码与说明，不含已捕获的部分输出。

## 启动和执行顺序

1. serde 解析参数，检查非空和时限，再检查预取消。
2. Unix 使用 `/bin/bash --noprofile --norc -c <command>`；cwd 为 `Workspace::root()`。
3. stdin null；stdout/stderr piped；移除 `TURNFORGE_API_KEY`、`OPENAI_API_KEY`、`ANTHROPIC_API_KEY`、`BASH_ENV`、`ENV`。
4. 设置新进程组和 `kill_on_drop(true)` 后 spawn，取得 Child、ProcessGroup guard 和两条管道。
5. `select! { biased; ... }` 按取消、超时、`join!(child.wait(), capture(stdout), capture(stderr))` 顺序竞争。
6. 任一分支结束后显式 `group.kill()`；若失败，立即返回 `cleanup_failed`。
7. 取消/超时且 kill 成功：继续 `child.wait().await` 后返回原始取消/超时错误。
8. 正常 join 且 kill 成功：检查 wait/capture 结果及 exit status，构造成功或 command_failed。

取消和超时同时 ready 时取消优先；若进程完成但取消已 ready，也可能报告取消。
正常分支要求进程 wait 和两路 EOF 都结束，timeout 因而覆盖启动后的执行与管道排空。
spawn 自身在进入 `select!` 之前，不能把 timeout 解释为可抢占的进程创建时限。

## 取消与错误矩阵

| 场景 | 错误码/结果 | 清理行为 |
|---|---|---|
| 缺字段、未知字段、空参数、零时限 | `invalid_arguments` | 未启动进程 |
| 参数有效但 token 已取消 | `cancelled` | 未启动进程 |
| 非 Unix 执行 | `unsupported_platform` | 未启动进程 |
| spawn 失败 | `spawn_failed` | 无已返回的 Child |
| 运行中取消 | `cancelled` | killpg 成功后等待直接子进程 |
| 超时，包括未排空管道 | `timeout` | killpg 成功后等待直接子进程 |
| killpg 或取消/超时后的 wait 失败 | `cleanup_failed` | guard/kill_on_drop 仅兜底，不能声称完全清理 |
| 正常等待或 capture 返回 IO 错误 | `io_error` | 正常 join 分支完成后已尝试 killpg |
| 正常退出码为 0 | Ok(data) | 仍尝试 killpg 清理残留组成员 |
| 非零退出或其他非成功退出状态 | `command_failed` | 同正常清理；输出编码进 message |

`ProcessGroup::kill` 发送 SIGKILL；ESRCH（组已不存在）视为成功，其他错误向上返回。
Drop guard 忽略 kill 错误，不能做异步 wait；宿主应取消 token 并继续 await，而非 abort 工具 future。
所有清理保证均限于未逃离进程组的子进程，且依赖 OS 调用成功。

## 限额与安全边界

`capture` 每次读取最多 8192 字节，stdout/stderr 分别保留 32768 原始字节；达到上限后仍排空至 EOF。
`*_truncated` 表示丢弃过超出上限的字节，而不是 UTF-8 解码失败。
有损 UTF-8 的替换字符可能让最终字符串字节数超过原始 32768 字节；不要把它当最终 JSON 大小上限。
没有 shell CPU、内存、磁盘、网络或总输出生成量配额，也没有命令长度的模块内上限。
CLI `--tool-timeout` 默认 30 秒、范围 1..=3600；库构造只在 execute 时拒绝零时限。
Registry 的 Shell 授权是唯一内置能力门；工作区只影响 cwd，Write 授权不会限制 shell 修改文件。
未清空全部环境变量，移除已知 key 不构成机密隔离；NDJSON 也可能含命令与输出中的敏感信息。

## 关键不变量

- stdout 与 stderr 必须同时排空，不能先读完其中一条再读取另一条。
- 达到保留上限后继续消费，不能让正常输出上限制造管道死锁。
- 取消路径在可清理情况下等待直接子进程后再返回；失败时明确报告 cleanup_failed。
- shell 执行不脱离调用生命周期成为后台服务；不承诺阻止主动逃离组的进程。
- stdout/stderr 是数据；stderr 有内容并不等于命令失败。

## 现有测试与缺口

[tests/tools.rs](../../../tests/tools.rs)：`shell_drains_stderr_and_preserves_nonzero_exit` 使用超过 pipe 常见容量的 stderr，
检查排空、截断和 exit 7；`shell_timeout_kills_descendants_before_they_write` 检查超时后后代不再写文件；
`shell_cancellation_waits_for_cleanup` 检查运行中 token 取消的同类副作用边界。
[tests/cli_http.rs](../../../tests/cli_http.rs)：`actual_ctrl_c_cancels_shell_and_preserves_event_closure`
通过真实 SIGINT 检查 CLI 退出 130、取消工具结果和后代写入未发生。

尚未针对 spawn/killpg/wait 故障、非 Unix 返回、环境变量过滤、无效 UTF-8、成功分支残留后台进程、
进程主动逃离组、取消与完成竞争做专门验证。现有取消测试不证明任意恶意进程树都能被回收。

## 变更检查清单

- [ ] 启动参数、环境继承与能力声明是否改变安全边界？
- [ ] 修改读管道方案后是否仍并发排空，并保留明确上限？
- [ ] 新资源由谁关闭、取消、等待，是否有 detached task 或后台进程？
- [ ] 错误结果能否区分 command_failed、timeout、cancelled 和 cleanup_failed？
- [ ] 若添加 stdin/PTY/会话模式，是否先定义独立生命周期而非复用一次性执行假设？
