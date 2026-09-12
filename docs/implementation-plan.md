# M0 — 可运行的 headless Rust 内核

Baseline: turnforge `cac4fb5`；参考 Helixent 1.3.1，`5cc1fb3faf29b8db8dce614e925c89030ae2558b`。
目标：先迁移可解释、可测试的核心行为，不承诺完整产品等价。

## 变更合同与执行记录

| ID | 范围 / owner | 验收 | 状态 |
|---|---|---|---|
| T1 | crate、CLI、工具链、CI | fmt / clippy / test / build 可执行 | done |
| T2 | Agent 独占 transcript，Model 借用快照 | model → tools → model；取消/上限后无悬空 tool call | done |
| T3 | OpenAI-compatible provider 独占流解析状态 | 真实本地 HTTP SSE、碎片参数、异常终止、usage 可缺省 | done |
| T4 | ToolRegistry / workspace / shell 管理副作用 | 权限默认拒写拒 shell、目录限制、取消回收、超时 | done |
| T5 | 独立 CLI 集成测试与学习文档 | 不依赖 API key 的完整链路；明确未验证事项 | done |

## 所有权、生命周期与约束

- `Agent::run(&mut self, ...)` 是唯一 transcript 写入者；provider/tools 不获得可变会话。
- `Model` 管 HTTP 与 SSE；成功返回前校验完整消息，不能把碎片参数当空对象执行。
- `Tool` 管自身副作用与取消清理；首版串行执行，无 detached tool task。
- CLI 管配置、stdout、Ctrl-C；库不持有全局配置、终端或 signal handler。
- `CancellationToken` 是协作取消请求，不是回滚；写操作完成后保留事实结果。
- 所有已提交 tool call 都有结果，包括取消后尚未执行的调用。
- 先用一个 crate 的模块边界；有真实复用/发布需求再拆 workspace crates。

## 非目标与安全边界

本次不实现 TUI、Anthropic、Responses API、MCP、skills、middleware 插件、持久化、
上下文压缩、subagent、OS sandbox、Windows shell。后续见迁移路线。
文件路径检查是误操作防护，不是抗恶意并发文件系统变更的 sandbox。
shell 显式授权后拥有宿主机权限；不自动读取或使用本机其它应用凭据。
只改本地 turnforge，不修改参考仓库、不 commit/push、不发布。

## 验证策略

E0: 格式、clippy warnings-as-errors、build；E1: 状态/权限/解析定向回归；
E2: 独立本地 HTTP fixture → 真实 binary → 真实临时目录/子进程；
E3: 真实模型调用需要用户配置 endpoint/model/key，本次不将 E2 冒充 E3。

自检和独立复验结果见 `verification.md`。审查新增 F1（stdout 背压阻止取消）已通过
CLI 专属 nonblocking 输出修复，测试在完全不读取 stdout 的条件下验证进程退出。
F2（立即完成的工具批次造成队列自溢出）通过工具结果提交后的调度让出点修复，
32 个被拒绝调用可完整回传并继续模型请求。最终 28 tests 在 debug/release 均通过。
