# 原生调试本地验收：2026-09-14

日期按 America/Los_Angeles。使用说明见[调试指南](../debugging.md)，
长期合同见[debugger 设计](../modules/debugger/design.md)。本记录不替代这些合同。

最终结论：修复版 `1096d8…858ec` 独立非模型验收 verified；本机真实模型两种 profile 均 4/4 通过。
下文保留首版失败与修复过程，最终交付对应“修复版与重新验收”，而非首版指纹。

## 对象与环境

- 基线：`31ba63de6d36982ec849d5b3bcef4edb5a84e8b4`。
- 验证对象：`codex/semantic-debugger` 上未提交的原生调试实现与测试增量；未提交、推送或触发远端 CI。
- macOS 15.7.3（24G419），Apple Silicon；rustc 1.98.1。
- Ollama 0.33.3，`http://127.0.0.1:11434/v1`；仅 loopback，沿用前台启动脚本，无开机启动。
- 固定模型 `turnforge-test:qwen3-4b-v1`；完整 digest
  `1cfb1620611f6fdcdbc90e3c32f0b0123b7ac36502c790eb90897941703ec363`。
  参数及来源沿用 [lock](../../dev/local-llm.lock.json) 和 [Modelfile](../../dev/Modelfile.local)，未更改。

首版源码、测试、Cargo 输入、工具链及 CI 定义冻结指纹（独立验收发现问题，已被下文修复版替代）：

```text
b58dfcf25a2bafc97f19eb73e161dc677b15a8d141ccf4864ef231d437d219a6
```

在本版本仓库根目录计算（同时纳入未跟踪的新增源码/测试；文档不在此指纹内）：

```sh
git ls-files --cached --others --exclude-standard -z -- \
  src tests Cargo.toml Cargo.lock rust-toolchain.toml .github/workflows/ci.yml \
  | xargs -0 shasum -a 256 | shasum -a 256
```

## 首版实现自检与全量门禁

| 检查 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --locked --all-targets -- -D warnings` | 通过 |
| `cargo test --locked --all-targets` | 43 通过；4 个真实模型用例按设计 ignored |
| `cargo test --locked --release --all-targets` | 43 通过；4 个真实模型用例按设计 ignored |
| `cargo build --locked --release` | 通过 |
| `git diff --check` | 通过 |

43 项由 CLI 解析单测 1、Agent 8、真实 CLI/HTTP 15、debugger 10、工具 9 构成。
本轮新增 15 项确定性用例及 1 项真实模型用例。测试映射见
[debugger 设计](../modules/debugger/design.md#测试与变更检查)。

真实 CLI 回归使用临时目录和本机 HTTP fixture，检查工具执行前磁盘无副作用、单步后出现真实文件、
旧 ID 拒绝、Inspect 不推进、stdin 保持打开仍结束、EOF/输入错误配对取消、SIGINT 等待 shell 清理。
手工 PTY 验证也到达初始暂停、返回 Inspect 快照，Ctrl-C 后发出 cancelled 终态且退出 130，没有请求模型。

## 首版固定本地模型验收

依次执行，未同时争抢模型服务；这两条显式测试均首轮通过，没有以重跑改变结果：

```sh
cargo test --locked --test local_llm -- --ignored --test-threads=1 --nocapture
cargo test --locked --release --test local_llm -- --ignored --test-threads=1 --nocapture
```

| 用例 | Debug | Release | 独立可观察结果 |
|---|---|---|---|
| LLM-001 文本流 | 0.49 秒，通过 | 0.46 秒，通过 | 无工具，非空增量、完整回答、正常终态 |
| LLM-002 读取未知 marker | 1.77 秒，通过 | 1.60 秒，通过 | 实际读取结果和最终回答包含测试生成的 marker |
| LLM-003 写入精确内容 | 1.70 秒，通过 | 1.53 秒，通过 | 独立读取磁盘，内容、字节数与成功结果匹配 |
| LLM-004 逐步调试读取 | 4.22 秒，通过 | 1.86 秒，通过 | 每次收到暂停后才发送当前 ID 的 Step；读取结果快照与最终答案匹配 |
| 合计 | 4/4，8.20 秒 | 4/4，5.46 秒 | 默认回归的 ignored 未计入上述通过数 |

本次 LLM-004 在两种 profile 下都观察到 6 个语义暂停点：首次请求前、工具预览/结果和最终回答。
模型选择了 `list_files` 后 `read_file`；这是本轮观测，不是固定工具序列或固定暂停数的测试要求。
测试只使用合成数据、临时工作区和本地模型，无 shell 授权或云端模型 fallback。
耗时包含基线检查和 CLI 运行，Debug 的首个用例还包含冷模型加载；不是推理性能基准。

## 发现、修复与边界

实现自检发现 `Pause` / `Cancel` 原先使用 serde unit variant 时会接受未知 JSON 字段，
例如 `{"command":"pause","extra":true}`。修复归属核心命令类型：改为零字段 struct variant，
保持正常 wire JSON 不变，并增加核心及 CLI 严格字段回归。真实 CLI 反例复验退出 1，
run 以 cancelled 闭合；没有在 CLI 添加第二套字段 schema 来掩盖核心解析问题。

运行前本地服务未启动，使用已有脚本恢复；未重新下载模型或改 lock。
CLI 手工自检曾有一个诊断文字 oracle 漏掉 `HTTP` 单词而报断言失败，核对实际进程后确认
它已经正确在 stdin 开放时以模型错误退出；这是自检脚本断言问题，不是 runtime 故障。

上述首版冻结 artifact 的独立验收判为 **failed**：虽然全部既有门禁通过，审查发现命令队列
只 drain 8 次，拒绝事件回调可同步补入未来 `Step 2`。第 8 次拒绝后留下的预发命令会在暂停 #2
发布后被误接受，违反“提前发送未来 ID 不能放行”的合同。修复必须回到核心入队相关性，
不能只多 drain 一次；本记录保留首版失败，后续新指纹和复验结果另行记录。

## 修复版与重新验收

新增 `replenished_early_commands_cannot_cross_into_a_new_pause` 首先在原版单独运行并失败：
补位 8 次，工具实际执行 1 次（预期 0），Inspect 未收到响应。测试先取消并 await 清理，再判断结果。
修复后同一反例通过，未修改预期或放宽断言。

核心采用私有 `QueuedCommand` 捕获入队当时的暂停 epoch；只有 Session 发布 epoch，0 表示无活动暂停。
Step/Continue/Inspect 同时校验命令 ID 与入队 epoch；RAII 在恢复前、取消或暂停 future 丢弃时清零。
有界 drain 仅负责调度公平性，不承担防提前命令的安全保证；公共 API 与 JSON 形状不变。

修复版采用与前文相同的计算方法，冻结指纹为：

```text
1096d8fd72f9bf0b0201a733674cf95b3141508860bfe3755beb5d1e766858ec
```

主代理定向复验：11 项核心调试测试、fmt、全目标 Clippy 通过。之后重新执行两条完整真实模型命令，
没有并发推理；版本/digest 仍逐 case 校验，未更改模型基线：

| 用例 | 修复版 Debug | 修复版 Release |
|---|---|---|
| LLM-001 文本流 | 通过，0.46 秒 | 通过，0.45 秒 |
| LLM-002 读取未知 marker | 通过，1.58 秒 | 通过，1.57 秒 |
| LLM-003 写入精确内容 | 通过，1.51 秒 | 通过，1.51 秒 |
| LLM-004 逐步调试读取 | 通过，2.87 秒 | 通过，1.79 秒 |
| 合计 | 4/4，6.43 秒 | 4/4，5.33 秒 |

两种 profile 的 LLM-004 仍各经过 6 个实际暂停点，验证真实文件结果快照和最终回答后才释放最终结束。
这些是代码修复后的新一轮验证，不是对旧版失败的无改动重跑。

独立验证者未参与实现、不修改仓库、不调用 LLM；开始与结束指纹均与修复版一致，结论 **verified**，
原 P2 关闭，未发现新的阻断问题：

| 独立复验 | 结果 |
|---|---|
| fmt、全目标 Clippy、release build | 通过 |
| `cargo test --locked --all-targets` | 44/44；4 个真实模型用例按设计 ignored |
| `cargo test --locked --release --all-targets` | 44/44；4 个真实模型用例按设计 ignored |
| stdout 不消费且 stdin 保持打开 | 2.049 秒退出 1，报告 stalled，未开始模型请求 |
| 共享 PTY，Inspect 后 Cancel | 快照返回、退出 130、可修改 fd flags 恢复、stderr 为空 |
| 非 UTF-8、超过 4 KiB、空命令、JSON 未知字段 | 均退出 1，发 cancelled 终态，未开始模型请求 |
| 真实文件副作用、EOF、SIGINT、stdin 打开时正常结束 | CLI 集成测试通过 |

最终 44 项确定性测试包含 debugger 11、CLI/HTTP 15、Agent 8、工具 9、CLI 解析 1。
独立 PTY 初次探针曾要求全部 GETFL bits 相同；macOS 写入会改变内核维护的标记，因此以独立写入对照
排除探针错误后，仅验证程序可修改的 flags 恢复，不将内核标记变化算作产品故障。
文档按实际实现校准，新增模块双文档、指南、测试矩阵和直接消费者合同；相对路径/锚点与 diff 检查通过。

本轮不声称：LLM Space 已接入、源码行级断点、历史编辑/回滚、持久化恢复、生产就绪或 Linux 实测。
快照可能包含敏感上下文，且没有全局历史/快照字节预算；暂停也不会回滚已有文件副作用。
远端 CI 与真实模型跨设备稳定性不在本次本地验收证据内。
