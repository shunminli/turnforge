# M0 验证记录

## 范围和 artifact

- 本次目标：可运行的 Rust headless 核心，不是完整 Helixent 功能等价或生产发布。
- Baseline: `cac4fb5`；本地分支 `codex/initialize-rust-harness`；未 commit / push。
- 环境：本机 macOS aarch64，Rust / Cargo 1.98.1。
- 执行状态：T1–T5 implementation complete。
- 验证范围：本地协议及执行环境；真实付费模型和 Linux 尚未验收。
- Verification state: verified for M0 local scope；不等同于生产 release-ready。
- 源码/依赖冻结摘要：`7fd9bb1f7a40fc436f5c87fc69140c37b9f35c944b411f6dbccac50e1fee6bff`。

摘要生成方式（从仓库根目录执行，顺序也是合同的一部分）：

```sh
shasum -a 256 Cargo.toml Cargo.lock rust-toolchain.toml src/*.rs src/tools/*.rs tests/*.rs | shasum -a 256
```

## 已执行的开发门禁

| 检查 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --locked --all-targets -- -D warnings` | 通过，无 warning |
| `cargo test --locked --all-targets` | 28 tests 通过：Agent 8、CLI/HTTP 11、tools 9 |
| `cargo test --locked --release --all-targets` | 同样 28 tests 通过，包含真实 release CLI |
| `cargo build --locked --release` | 通过，生成 `target/release/turnforge` |
| release binary `--version` / `tools --help` | 通过，版本 0.1.0 |
| `git diff --check` | 通过 |

## 证据层次

- E0：上述格式、静态检查和构建。
- E1：Agent 的顺序、跨 user turn 历史、错误、重复调用拒绝、步数上限、取消和非法恢复；
  工具的权限/输入验证、路径/静态 symlink 越界、UTF-8/大小限制、原子替换、双管道排空、退出码。
- E2：测试独立编写 HTTP/SSE 响应，不调用待测 provider 的序列化来生成期望值。
  真实 CLI 完成碎片参数 → 写临时文件 → tool result 回传 → 最终模型回复。
  还验证拒绝权限、HTTP 429/错误正文不泄漏、流中断、非法 JSON、length 结束、真实 SIGINT，
  以及 stdout 不被消费时仍能退出；测试不访问实际模型账户。

文件/子进程 oracle：直接检查临时目录实际内容，取消后等待潜在后代写入时点，确认没有
迟到副作用。HTTP oracle：直接检查下一次请求中的 tool_call_id 和结果字段。

## 独立审查发现与闭环

F1 / P2：原 CLI 在事件 callback 同步写 stdout，下游停读后 pipe 填满，Ctrl-C 也无法执行。
独立审查以实际 binary + 本地 SSE 复现；不能通过发信号后立刻开始读取 stdout 来掩盖。

修复：CLI 独立输出模块，64 项有界事件队列，Unix nonblocking fd / AsyncFd；与 run 一起 join，
有写入 deadline 和取消后有限 drain，没有不可取消的 stdout worker。
回归：`blocked_stdout_does_not_block_ctrl_c_or_process_exit` 在进程退出前完全不读取 stdout。
stdout 故障下输出可以不完整，进程以失败结束，不宣称 terminal event 已成功交付。

F2 / P2：一批 32 个立即返回的拒绝/未知工具调用会连续发事件，正常消费者也可能被
单次 poll 中的 burst 填满队列。修复在每条 Tool result 提交后主动让出调度；没有扩大队列。
`a_full_batch_of_immediate_tool_errors_does_not_overflow_output` 验证 32 个拒绝结果完整回传，
随后继续模型请求并自然结束。

验证期间还修复了 fixture 本身的 macOS 竞态：nonblocking listener 的 accepted socket
可能继承 O_NONBLOCK；blocking 请求读取器现在显式重置该标志，再应用读取超时。
没有跳过失败用例或放宽 oracle，debug 和 release 全部重跑通过。

最终独立复验：F1 在 stdout 始终未读的条件下，SIGINT 后约 0.15 秒退出；F2 通过真实 HTTP
返回 32 个未知调用，得到完整 32 个结果和唯一 `run_finished: completed`，退出 0。
审查者复跑 11 个 CLI 集成测试全部通过，F1/F2 无未解决项，未修改被验证源码。

## 未覆盖，不应推导为 release-ready

- E3 未运行：没有配置/调用真实 endpoint、model、key；第三方兼容性与任务成功率未验证。
- GitHub Actions 仅已配置；未推送，因此不能称远端 CI 通过。Linux 待该环境实际运行。
- 没有安全隔离验收：文件路径检查不是 OS sandbox，shell 可逃离工作目录和访问网络。
- 不覆盖恶意文件系统并发替换、网络文件系统卡死、进程主动逃离进程组、强杀宿主后的恢复。
- 不覆盖长会话内存预算/上下文溢出、持久化、交互审批或多 provider 等未实现能力。

外部验收的下一步由项目使用者配置一个明确的模型服务，然后在临时仓库运行读文件、修改、
测试失败修复和取消四类任务。需要新测试或修复时更新源码摘要并重新验证，不能沿用旧结论。
