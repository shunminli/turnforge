# OpenCoder 可靠性能力吸收验收（2026-10-08）

这是本轮输入、实现和验证记录，不是实时状态或新的操作手册。
现行合同由 [Agent](../modules/agent/design.md)、[Model](../modules/model/design.md) 和 [CLI](../modules/cli/design.md) 维护。

## 冻结输入与范围

- Turnforge 基线：`d070aade16dbaa128beae4725930199216d8d4fd`，开始时工作树干净。
- OpenCoder upstream：`8bf74a10109dc16c0d087df23e1ea829ed1dd259`。
- Fork merge：`1b3b04d85b7f5d6864c79ead84fcad7cb3c738db`，tree 与 upstream 一致。
- 实施分支：`codex/opencoder-runtime-adoption`；本轮未提交、未推送 Turnforge。
- 本机：macOS / aarch64，仓库固定 Rust 1.98.1；Cargo 使用 `/Users/bytedance/.cargo/bin/cargo`。

新增的是显式可选的连续有序工具批次防护，以及指定 HTTP 状态的流前有限重试。
不引入 Responses、流中重试、技能/AGENTS 加载、持久化恢复、并行工具、自动记忆或上游依赖。
实现及测试独立编写，来源与差异见 [provenance](../provenance.md)。

## Ownership / 兼容与删除检查

Agent 的 `run_loop` 独占上一批调用副本与重复计数；run 返回或 drop 后释放，不跨用户 turn。
OpenAiModel 只持有不可变重试上限，实际计数、Response、等待由单个 `request` future 持有。
取消 select 覆盖 HTTP 与等待；无新 task、channel、共享锁或长期 token。
现有 `&mut Agent` 单 writer、串行工具、Tool cleanup 等待及消息闭合保持不变。

CLI 是两个 API 的真实消费者；run/debug opt-in，Lab 固定关闭。
公开配置新增 `tool_repeat_limit`，完整 AgentConfig struct literal 需要补字段或使用 default update；
AgentError 新增 ToolLoop。Message/Event/DebugAction 序列化形状和 RunOutcome 变体没有改变。

删除检查已去除两个重复 setup 的 guard 测试，将跨 run reset 与核心 limit=1/纯文本路径并入行为用例。
没有测试专用生产 hook、兼容 wrapper、额外后台清理路径或未消费的公共接口。
主要生产净增约 110 行（Agent、模型及 CLI 配置；不含 CLI 参数测试），其余成本主要是行为测试及 owner 文档。

## 已执行门禁

在仓库根执行，均 exit 0；没有在线模型请求或测试预期修改：

```sh
/Users/bytedance/.cargo/bin/cargo fmt --all -- --check
/Users/bytedance/.cargo/bin/cargo clippy --locked --all-targets -- -D warnings
/Users/bytedance/.cargo/bin/cargo test --locked --all-targets
/Users/bytedance/.cargo/bin/cargo test --locked --release --all-targets
/Users/bytedance/.cargo/bin/cargo build --locked --release
git diff --check
```

改动前：54 passed / 0 failed / 5 ignored。
改动后 debug 与 release 各：66 passed / 0 failed / 5 ignored。
分组：binary 5、Agent 13、CLI/HTTP 20、debugging 11、Lab CLI 8、tools 9；library 0。
5 项 ignored 是仓库显式要求固定 Ollama runtime/model 的真实本地模型用例，本轮未执行，不算通过。

另执行 release binary `run --help`，确认两项参数真实暴露；库级/API及参数配置均有消费者。

## 证据证明什么

| 风险 / 合同 | 最高证明力证据 |
|---|---|
| 防护阻止第二次相同写入，并闭合调用、Failed/退出 1 | `tests/cli_http.rs::repeated_tool_batch_guard_closes_calls_before_a_second_real_write`，真实 TCP → binary → 文件 → NDJSON；只有第一次 ToolStarted、两个模型步骤，第二调用为 tool_loop |
| ID 不参与比较、参数/名称/顺序/数量变化重置，下一用户 turn 重置 | `tests/agent.rs` 的重复批次与变化测试；counting tool 确认实际执行次数 |
| 同批 32 个相同调用不误判；默认关闭保留行为 | Agent 批次测试及原 CLI 32 个即时拒绝结果的输出公平性回归 |
| AfterModel 精确预览 Finish(Failed)，一次释放闭合整批，无假工具动作 | Agent 通过生产 debug_channel/run_debug 的行为测试 |
| 暂停、结果闭合中、最后结果提交后的取消优先 | Agent 定向测试，保留既有结果、剩余 cancelled，唯一 Cancelled 终态 |
| 指定状态恢复且请求体一致，HTTP 尝试不增加 Agent step | 真实 HTTP/CLI 状态矩阵；429/502/503/504 恢复，其它状态 fail-fast |
| 三次重试耗尽、错误正文不泄露、无半条 Assistant | 真实 HTTP/CLI 四次 POST 及终态检查；原默认不重试测试保留 |
| 收到拒绝响应附近取消后没有后续请求；未知远端处理结果不重放 | 真实 TCP 与生产 OpenAiModel，观察窗口确认一条 POST，无 delta |
| 成功流格式失败不重试、不执行完整但未合法完成的工具 | 原 malformed/incomplete SSE 回归启用 http_retries=3，仍只一条 POST |
| 既有取消/背压/进程清理与 Lab 独立 oracle 无回归 | 完整 debug/release 测试，包括 SIGINT、shell 后代、开放 stdin、blocked stdout、真实 Lab 文件验收 |

两名独立审查者只读复核 guard 与 retry，未发现阻断问题。取消测试在服务端发 503 后等待 25ms，
可确证取消后无第二请求，但高负载下不能严格证明取消瞬间已处于 backoff 而非 HTTP 等待；
退避归同一个可取消 future 的结论另有直接源码证据，不把时间假设当作精确阶段 oracle。

## 剩余边界

状态重试不保证提供方未处理或未收费；只汇总最终成功响应 usage，不做跨尝试计费统计。
没有 Retry-After、jitter、跨尝试总 deadline 或 SSE 重放。
重复防护是原样重复的启发式限制，不检测变参/交替批次、任务停滞或业务成功；取消也不回滚先前副作用。
真实在线服务兼容性、故障穷尽和固定本地模型本轮未验收。工作树修改需要用户另行决定提交/发布。
