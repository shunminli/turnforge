# 测试与回归参考

本页是完整回归运行清单的唯一维护入口：说明运行什么、需要哪些前置、用什么事实验收。
它不是一次已执行测试的报告，也不替代[分章教程](harness-learning.md)中的学习实验。
所有命令在仓库根目录执行；仓库和 Rust 准备见 [101 用户指南](101-user-guide.md)，
模型安装、服务启停和锁定环境见[本地 LLM 指南](local-llm.md)。

| 层次 | 能证明什么 | 不能证明什么 |
|---|---|---|
| 确定性回归，默认 CI | 可控合法/异常协议、权限、取消、真实 CLI 与文件/进程合同 | 真实模型是否选择合适工具、某服务的兼容性 |
| 真实本地模型回归，显式启用 | 本机服务 → SSE → CLI → 工具 → 模型继续回答的闭环 | 全量协议覆盖、复杂 coding 能力、所有机器上稳定通过 |

真实模型测试不能替代确定性门禁。固定 seed 和 temperature 用于减少采样变化，
不保证跨硬件、runtime 版本或输入变化时逐 token 一致。失败不能靠反复重跑到绿来消除。

## 确定性回归

以下与 [macOS/Linux CI](../.github/workflows/ci.yml) 的门禁顺序一致；逐条执行，失败先诊断：

```sh
rustup show active-toolchain
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo test --locked --release --all-targets
cargo build --locked --release
```

默认测试无需 API key，也不调用真实模型。真实模型用例标有 `#[ignore]`，
输出中的 `ignored` 表示未执行，不计作通过。`--locked` 沿用仓库 `Cargo.lock`。
本地门禁通过不等于远端 CI 已通过；推送后仍要检查该提交对应的远端结果。

仅改文档时按[文档维护规范](documentation-guide.md)核对链接、模块覆盖和 claim 准确性；
不要从格式检查推导出真实模型兼容性，也不把历史验证结果写成本轮实测。

## 真实模型回归

前置是[锁定的 Ollama 与模型](local-llm.md)已经安装，服务正在本机前台运行。
测试不自动安装、下载、启动服务，也不会因不可达、缺模型或基线漂移而静默跳过。

先完成确定性回归，再依次执行；上一条结束且失败已处理后才开始下一条：

```sh
cargo test --locked --test local_llm -- --ignored --test-threads=1 --nocapture
cargo test --locked --release --test local_llm -- --ignored --test-threads=1 --nocapture
```

`--ignored` 显式选择真实模型用例；`--test-threads=1` 仅串行化本测试进程，
不阻止另一个终端并发调用同一服务。运行期间不要同时启动 Lab 或另一轮模型测试。
只列出用例、不调用模型：

```sh
cargo test --locked --test local_llm -- --list
```

使用真实 Turnforge binary、合成输入与临时工作区，清空子进程继承环境，永不授权 shell。
测试固定本机 endpoint/model；Lab 路径还走产品自己的本机预检与禁代理/重定向 provider。
每个 CLI 最多 4 次模型请求，每次请求 90 秒。测试对 CLI 退出和管道排空设置 300 秒等待上限；
超时后尝试 kill，并最多等待 5 秒回收。这不是整个 Cargo 构建或手工 CLI 的全局 deadline。

stdout/stderr 并行排空，每路最多保留 4 MiB；捕获超限或读取失败会使测试失败。
失败诊断最多显示每路已捕获内容的末尾 32 KiB，均来自合成用例。实现与限额见
[tests/local_llm.rs](../tests/local_llm.rs) 的 `run`、`drain`、`diagnostic` 与 Lab 用例。
这些有界测试捕获不代表普通运行已具备会话总内存限制。

## 调试专项

只运行确定性调试状态机与 CLI 检查，不需要 Ollama：

```sh
cargo test --locked --test debugging
cargo test --locked --test cli_http debug_cli_
cargo test --locked --test lab_cli
```

前两条分别检查库的语义调试和原生 `debug` 宿主，第三条检查 Lab/学习入口与合成 HTTP 场景。
精确的合同—测试映射由 [debugger 设计](modules/debugger/design.md#测试与变更检查)、
[Lab 设计](modules/harness-lab/design.md#验证与缺口)维护，本页不重复整个符号表。
定向过滤命令应实际命中测试；`0 tests` 不是验证成功。

已经完成模型准备时，可单独验证真实模型调试闭环；这只是定向复验，不算完整回归：

```sh
cargo test --locked --test local_llm local_llm_debug_steps_through_read_and_final_answer -- --exact --ignored --test-threads=1 --nocapture
```

该用例只读临时 marker 文件，不授权写入或 shell，与其它真实模型任务串行执行。
控制命令和自动化输入使用方法见[原生调试参考](debugging.md)。

## 用例矩阵

实现与断言以 [tests/local_llm.rs](../tests/local_llm.rs) 为准。ID 用于报告关联，不是额外测试：

| ID / 测试符号 | 入口、输入与权限 | 独立验收依据 |
|---|---|---|
| LLM-001 / `local_llm_plain_text_stream_completes` | `run --json`；临时空目录；一句问候；只读 | 非空文本增量、非空最终已提交回答、没有工具调用、正常结束；不逐字比较问候 |
| LLM-002 / `local_llm_reads_unknown_file_marker` | `run --json`；测试预写 `marker.txt`；prompt 不含动态 marker；只读 | 成功 read_file 结果与最终回答都包含预写 marker |
| LLM-003 / `local_llm_writes_file_and_finishes` | `run --json`；临时空目录；显式 `--allow-write` | 独立读取 `result.txt`，精确为 `turnforge-local-write-ok`、无 LF；成功 write_file 结果路径/字节数匹配，随后自然结束 |
| LLM-004 / `local_llm_debug_steps_through_read_and_final_answer` | `debug --json`；只读临时 marker 文件；驱动保持 stdin 开放 | 真正暂停后才发当前 ID 的 Step；工具前预览、read_file 结果、AfterTool 快照与最终答案含 marker；释放 Finish 后成功退出 |
| LLM-005 / `local_llm_lab_auto_reads_fixture_without_stdin` | `lab --case read --auto`；stdin 为 `/dev/null`；产品创建只读合成场景 | Lab 文本含 PASS、不含 FAIL、至少 4 次语义暂停及读工具结果；最终模型输出含读取 marker；成功退出且 banner 指向的临时目录已清理 |

五个用例都先检查运行中的 Ollama 版本与测试别名完整 digest。只有 **LLM-001～004**
通过 `run` helper 检查 NDJSON；LLM-005 使用 **Lab 文本投影**，不能套用 NDJSON 断言。

### 普通 run / debug 的事件断言

LLM-001～004 共同检查：

- 成功退出，stdout 每行是事件；恰有一对 `run_started` / `run_finished`，首尾有序且 outcome 为 `completed`。
- 有非空流式文本；最后提交的是文本非空、没有待执行 tool call 的 Assistant，而非仅暂态增量。
- 每个 ToolStarted 对应已提交调用，每个 Tool 结果关闭已开始调用；新的 Assistant 前上一批调用已闭合。
  ID 只要求在同一 Assistant 内唯一，不固定生成格式或工具顺序。

LLM-004 还在每次 `debug_paused` 后提交当前 ID 的 Step；首个快照只含 User，pause ID 随实际暂停递增，
snapshot version 为 1。最终快照的下一动作为 Finish(Completed)，驱动释放它后等待 CLI 和输出结束。
不要求固定暂停总数，也不从 stdout 的“暂停”文字单独推导磁盘没有副作用；相关隔离由确定性测试补充。

### Lab 文本入口的断言边界

LLM-005 解析 `[result]` 中成功读取的 marker，并检查最终模型步骤的输出使用该值；
真实文件预期与 transcript 的独立核验由产品 `PreparedLab::verify` 执行，详见
[Lab 场景事实与独立验收](modules/harness-lab/design.md#场景事实与独立验收)。
该测试另外观察退出码、文本结果和退出后的目录清理，不是把产品输出当成完整事件协议。
它不覆盖所有 Lab 场景；chat/write 的确定性路径由 `tests/lab_cli.rs` 承载。

## 失败定位

| 观察到的失败 | 首先检查 | 不应采用的处理 |
|---|---|---|
| 连接失败、缺模型、runtime / digest drift | [环境与基线排查](local-llm.md#7-环境故障与基线维护) | 静默跳过、换云端、杀未知服务、为了通过修改 lock |
| CLI 非零退出、协议或事件不闭合 | 合成 NDJSON/stderr，CLI/HTTP 确定性回归及对应模块设计 | 吞错误、仅凭模型部分文本判断成功 |
| 文件内容或工具选择不符 | 实际磁盘/工具结果、模型行为、prompt 与固定基线 | 用实际输出替换 expected、反复重跑直到偶然成功 |
| 超时或输出捕获失败 | 本机资源争抢、服务日志、子进程回收结果 | 无边界放大超时、并发多次重试 |

保留首次失败的命令、提交、基线、退出码与合成诊断，先区分环境问题、Harness 故障与模型行为变化。
修复后明确记录“定向复验”或“新一轮完整回归”；未运行的写“未执行”，不能把 ignored 计入通过。
新增用例需说明独立 oracle 与权限边界，不能依赖模型自评或用生产会话充当固定 fixture。
完整生产会话、个人路径、凭据和模型权重不得作为测试日志入库。

已执行的历史证据分别见[本地模型记录](verification/local-llm-2026-09-12.md)、
[原生调试记录](verification/debugging-2026-09-14.md)、[Lab 记录](verification/harness-lab-2026-09-14.md)。
这些只证明所记录 artifact 与环境，不代表此后每个提交都已运行真实模型，也不代替本页的当前操作参考。
