# OpenCoder 流空闲与补充输入吸收记录（2026-10-19）

本篇记录本轮输入、选择和验证证据，不代替当前使用参考或模块合同。
合同 owner：[Model](../modules/model/design.md)、[Agent](../modules/agent/design.md)、
[Debugger](../modules/debugger/design.md)、[CLI](../modules/cli/design.md)、[协议](../modules/protocol/design.md)。

## 冻结输入

- Turnforge 起点：`997b0a2d314caea65d02e96ef1bc79bf042b4036`，保留已有完整请求 body 字节准入。
- 实施分支：`codex/opencoder-steering-20261019`。
- OpenCoder upstream：`ed6214dc192b5abaad8ef154847b92ed4de032ae`。
- Fork merge：`adb6daffd95d81d60f8f3e73497a0f5dd33cc287`；正常合并、无内容冲突，保留双方历史。
- Fork 与 upstream tree：`66b30a2b962c2e4fc50b6b92bf4d0c8c191c730d`，内容完全一致。
- 外部输入能力来源：`9a8213e3dba80353fefcf7d20aa57bf7461f27c8`。

起点 fork 的 221 条独有提交属于已保留历史：`34dc1ad2` 与其第二父提交、共同基线 `3c44c266`
内容 tree 完全相同，因此不存在需要覆盖上游才能保留的 fork 独有代码。本次上游正常前进 13 个提交。

## 能力选择与边界

| 上游能力 / 机制 | Turnforge 的处理 |
|---|---|
| 观察器持续运行时接收额外输入 | 吸收责任分离：Controller 有界接纳，Agent 仍唯一写历史；等待下一模型边界，不中断当前模型和工具 |
| 外部 JSONL 观察器空闲检测 | 映射到现有 OpenAI SSE consumer；成功 headers 后等待完整 data event，独立于请求总超时 |
| 外部脚本控制协议、持久 request/receipt journal、身份 reconciliation | 不直接移植：Turnforge 没有外部执行宿主或持久恢复；仅提供当前 run 内 queued/applied/rejected |
| TODO 文件、评论、独立存储及 NFS 导出 | 不直接移植：属于项目工作台与分布式资源服务，当前 headless 模型—工具循环没有对应 consumer |
| Brain 分层调度、派发与节点索引 ACK | 不直接移植：依赖上游控制面、Worker 和持久调度；当前没有分布式任务编排 owner |
| Web/TUI 导航、项目视图与发布工具 | 不直接移植：不是当前 Rust 内核及 CLI 的使用边界；不为移植而引入界面或发布平台 |

两项适配独立实现，不复制上游代码/测试/资源，不引入 OpenCoder 依赖。来源与许可证检查边界见
[provenance](../provenance.md#opencoder-observer-preserving-input-and-stream-liveness-ideas-2026-10-19)。

## Ownership 与兼容

Model request future 持有 SSE reader 和当前 idle 等待；OpenAiModel 只保存不可变配置。
成功 headers 以前仍由请求总超时约束；注释或未闭合字节不能维持一条没有完整事件的流。
空闲/流错误不重试，不把暂态工具参数提交为 Assistant，也不执行它们。

Controller 持有独立输入 sender，DebugSession 持有 receiver；Agent 在下一模型请求前、完整工具结果闭合后
提交额外 User，再发 Applied。不会转移 Agent 可变引用、替换 emit、另起后台任务或解除暂停。
步数预算不增加；应用新输入后只重置重复工具批次连续计数。
终态先关闭接纳，再等待 receiver 排空到 None 并拒绝剩余输入；最终回答不因为有排队输入而自动启动新 turn。

`SteerQueued` 是 CLI 接纳事件，库侧 `try_steer` 成功只返回 ID；Applied 不等于 HTTP 已发出。
已有 `--max-request-bytes` 在额外 User 提交后的下一请求上继续生效，可能在网络前拒绝。
Message 角色、既有 DebugCommand 和 RunOutcome 不变；Event 新增变体，穷尽匹配消费者须处理。
run/debug 新增可选流空闲参数；Lab 保持固定任务且不开放两项能力。

## 本地验证冻结结果

在仓库根依次执行下列命令，全部 exit 0。本次涉及 Agent、Model、Event 与 CLI 的直接消费者，
因此按 Turnforge CI 合同完成 debug/release 门禁；不将此范围外推为上游 OpenCoder 的全量验证。

```sh
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo test --locked --release --all-targets
cargo build --locked --release
git diff --check
```

`cargo fmt --all` 只做机械格式化，随后格式检查通过。Debug 与 release 各 86 passed / 0 failed / 5 ignored；
分组为 binary 6、Agent 13、CLI/HTTP 26、debugging 11、Lab 8、steering 8、stream_idle 5、tools 9、library 0。
五项 ignored 是需要固定本地 Ollama 的真实模型用例，本轮未运行，不计为通过。
正式门禁首次全部通过，没有失败重跑、降低断言或隐藏失败。

门禁前定向验证了 `steering` 8 项、`stream_idle` 5 项和 `debug_steering_*` 2 项。
实现自审发现 Tokio `try_send` 可能已取得 channel 容量而尚未完成发送的窗口：
终态处理改为 close 后 `recv().await` 到 None，保留所有成功接纳输入的回执；
`concurrent_admission_and_finish_preserve_every_successful_input_receipt` 比较成功接纳与终态回执的多重集合。
这是正式门禁前的自审修复，不是首次门禁失败。

| 入口 | 已验证的风险 |
|---|---|
| [stream_idle.rs](../../tests/stream_idle.rs) | 真实 HTTP 的静默、注释、分帧、完整事件续期、请求总超时及不重试 |
| [steering.rs](../../tests/steering.rs) | 接纳边界、暂停不推进、工具批次闭合、预算不重置、结束拒绝、callback 补队列与并发接纳收尾 |
| [cli_http.rs](../../tests/cli_http.rs) | binary/stdin/NDJSON 的接纳与应用、真实文件时点和 Applied 后仍在 HTTP 前执行的请求 body 准入 |
| 一次性 Node HTTP fixture → 实际 CLI 冒烟 | 完整暂态 write_file 参数也不能绕过闲置失败执行文件写入；不是 cli_http.rs 内的自动化用例 |
| 本地执行 [CI](../../.github/workflows/ci.yml) 的门禁命令 | 格式、Clippy、debug/release 测试与 release 构建；不等于远端 Actions 已执行 |

上述额外冒烟启动真实 HTTP fixture，并执行 `target/debug/turnforge run --json --allow-write`
及 `--stream-idle-timeout 1 --request-timeout 4 --http-retries 3`。
服务发送完整 `write_file` 参数和 finish reason，但不发送 `[DONE]` 且保持连接；
实际 CLI 退出 1、终态 Failed，服务仅收到一条 HTTP 请求，没有已提交 Assistant、没有 ToolStarted，临时目录为空。
该链路验证实际 CLI flag → provider → Agent → 文件/NDJSON 消费者，没有调用真实模型。

release binary 的 `run --help` 已核对新增参数。全仓静态检查覆盖 54 个 Markdown、557 个相对链接、
40 个锚点、72 个 shell block 的 `bash -n` 及 11 对模块 architecture/design，均通过。

OpenCoder fork 的同步验证已实际运行：
`cargo test --locked -p opencoder-session --test harness_control --test harness_codex --test harness_startup --test harness_stack`，
16 passed / 0 failed；分别为 6、6、3、1。fetch 后 fork 本地/远端 main 相同，ahead/behind 为 0/0。
这只证明上游 fork 的定向 Harness 链路；Turnforge 适配的证据由上述独立门禁提供。

## 冻结边界

本记录冻结于本地验证完成、Turnforge 提交和发布之前；此时没有本轮远端合并或 Actions 通过声明。
发布结果以对应 PR 与远端 CI 为准，不把本地检查改称远端验证。
OpenCoder 只执行了上述 16 项定向测试；两项目本轮均未验证真实模型、Linux runc、部署或外部 Codex 控制协议。
额外输入没有持久化、幂等重放、撤销或跨进程恢复承诺；Applied 也不保证后续 HTTP 发送或模型采用。
