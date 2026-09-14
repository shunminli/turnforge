# Harness Lab 与学习入口本地验收：2026-09-14

日期按 America/Los_Angeles。使用见 [Lab 指南](../harness-lab.md)与[学习指南](../harness-learning.md)，
长期合同见 [Lab 设计](../modules/harness-lab/design.md)与[学习设计](../modules/learning/design.md)。

结论：最终冻结实现通过独立本地验收；debug/release 各 54 项确定性测试及各 5 项真实模型回归通过。
真实 launcher 的交互 read、自动 write/chat 也通过。未提交、推送或触发远端 CI，不等同于跨平台或生产就绪。

## 验证对象

- Git 基线：`31ba63de6d36982ec849d5b3bcef4edb5a84e8b4`，分支 `codex/semantic-debugger`。
- 沿用此前未提交的原生调试增量；本轮新增 Lab/学习模块、CLI 入口、启动脚本及相关测试/文档。
  此前的[调试验收记录](debugging-2026-09-14.md)保留为历史证据，不把整个工作区差异都归为本轮新增。
- macOS 15.7.3（24G419），Apple M4 Pro arm64，48 GiB RAM；rustc 1.98.1。
- Ollama 0.33.3，loopback `127.0.0.1:11434`；固定模型 `turnforge-test:qwen3-4b-v1`。
- 模型 digest：`1cfb1620611f6fdcdbc90e3c32f0b0123b7ac36502c790eb90897941703ec363`。
  [模型 lock](../../dev/local-llm.lock.json)及[参数](../../dev/Modelfile.local)不变；无重新安装、下载或开机启动。

最终源码、测试、构建输入、启动脚本与 dev 配置指纹：

```text
6b1a322859b1b8973e51cded4e676ba7d72f4943767d9ebb8f3885fa57581c68
```

在仓库根目录计算；包括未跟踪的新增代码，不包括文档。脚本可执行权限另外检查。
此范围比此前调试记录多含 scripts/dev，不可直接把两个摘要的差异解释为代码差异：

```sh
git ls-files --cached --others --exclude-standard -z -- \
  src tests Cargo.toml Cargo.lock rust-toolchain.toml .github/workflows/ci.yml scripts dev \
  | xargs -0 shasum -a 256 | shasum -a 256
```

## 首次真实试用失败与调整

冻结前第一次运行 `bash scripts/harness-lab.sh --auto --lesson regression`，实际读取到
`turnforge-lab-18d5540751d0cd58`，模型最终输出却为 `turnforge-lab-18d5540651d0cd58`。
Agent 正常结束，但独立校验发现一个字符错误，程序正确 FAIL 并退出 1；没有把 Completed 当作 PASS。

Lab 的用途是观察调度、工具与上下文链路，随后将合成演示数据从十六进制串改成六个短英文词的动态 marker，
同时区分“未实际读取”和“读到了但最终回答不符”的错误信息。预期仍在请求前产生、只写入文件，
最终回答仍须包含精确 marker，且仍验证成功工具结果与磁盘原内容。没有从模型输出反推预期或放宽判定。
这不修复模型的抄写能力，也不保证未来每次通过；六词值由时间生成，不是保密随机数或唯一标识。

未修改模型基线、参数、既有四项真实回归或 provider 推理语义；没有对原失败 artifact 无改动重试到绿。
下述验证对应调整后重新冻结的实现。一次格式自检差异也在冻结前由 rustfmt 修正。

## 冻结后独立验收

独立验证者未参与实现、不改文件、不调用模型。开始与结束指纹均匹配上文，结论 **verified**：

| 检查 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --locked --all-targets -- -D warnings` | 通过 |
| `cargo test --locked --all-targets` | 54 通过，5 个真实模型用例按设计 ignored |
| `cargo test --locked --release --all-targets` | 54 通过，5 个真实模型用例按设计 ignored |
| `cargo build --locked --release` | 通过 |
| `git diff --check`、launcher 语法及可执行权限 | 通过 |

54 项由 binary 单测 3、Agent 8、CLI/HTTP 15、debugger 11、Lab CLI 8、工具 9 构成。
ignored 不计入通过数。测试符号映射见 [Lab 设计](../modules/harness-lab/design.md#验证与缺口)。

额外真实进程探针确认：

- 一次写入两条 `n` 不能越过未来暂停；`i/l` 不推进模型或工具。
- stdin 保持打开时 `q` 退出 130，临时目录清理；初始 EOF 不调用模型。
- 非 UTF-8 输入退出 1 并取消运行；stdout 关闭、stdin 仍打开时退出 1，不因两个输出 Sender 悬挂。
- 从 `/tmp` 使用脚本绝对路径 `--help` 可启动；不要求调用者当前目录就是仓库。

本轮本地 HTTP 测试也覆盖版本/digest 漂移、无模型、拒绝重定向、云配置/代理隔离、独立磁盘验收及无服务学习入口。
这些有限测试不等于 `OpenAiModel::new_local` 的所有构造参数组合已建立直接单测矩阵。

## 固定本地模型回归

两种 profile 依次执行，不并发争抢模型；每个用例仍检查固定版本与 digest：

```sh
cargo test --locked --test local_llm -- --ignored --test-threads=1 --nocapture
cargo test --locked --release --test local_llm -- --ignored --test-threads=1 --nocapture
```

| 用例 | Debug | Release |
|---|---|---|
| LLM-001 文本流 | 通过，0.45 秒 | 通过，0.45 秒 |
| LLM-002 读取未知 marker | 通过，1.91 秒 | 通过，1.63 秒 |
| LLM-003 精确写入并读回 | 通过，1.94 秒 | 通过，1.55 秒 |
| LLM-004 原生逐步调试读取 | 通过，1.76 秒 | 通过，1.83 秒 |
| LLM-005 Lab 自动读取、无 stdin | 通过，1.20 秒 | 通过，1.21 秒 |
| 合计 | 5/5，7.43 秒 | 5/5，6.69 秒 |

LLM-004 均观察到 6 个暂停、`list_files` 后 `read_file`；具体工具序列不是硬编码断言。
新增 LLM-005 检查实际文件工具结果、最终回答、自动步进和退出后的临时目录清理。
耗时包含预检与 CLI 运行，不是模型性能 benchmark；真实小模型不替代确定性故障回归。

## 实际用户入口与学习流程

除测试二进制外，实际执行以下 launcher：

| 命令 | 现场与结果 |
|---|---|
| `bash scripts/harness-lab.sh --lesson loop` | PTY 中 i/l 查看现场与课程，Enter/n 逐步读取、模型回答及 Finish；4 个暂停，PASS，退出 0 |
| `bash scripts/harness-lab.sh --case write --auto --lesson tools` | 4 个暂停，真实写入 22 字节，独立内容读回匹配，PASS，退出 0 |
| `bash scripts/harness-lab.sh --case chat --auto` | 无工具、非空最终回答，2 个暂停，PASS，退出 0 |

逐项检查上述临时目录在进程结束后均不存在。read 的最终文本精确包含实际读取的
`iris-birch-pearl-maple-lemon-harbor`；write 内容为 `turnforge-lab-write-ok`，无末尾换行。
这只是本次现场观测，不是后续运行的固定期望。

六课入口均覆盖静态学习目标、前置、源码符号、调试练习和人工验收问题。
`learn` 不需要模型服务，`lab --lesson` 仅增加真实暂停点说明；不更改 prompt、权限、调度或 oracle。
学习提示不冒充原始 HTTP 请求、模型内部思考或工具成功证明，也不自动认证用户已掌握知识。

## 文档、维护与边界

新增 Lab 与学习模块各自架构/设计双文档，两份使用指南和本轮计划/证据；总地图现有 11 组模块文档。
同步直接受影响的 CLI、输出、模型合同和本地回归矩阵。源码符号及 Markdown 相对路径/锚点检查通过。
长期知识放回职责 owner；未修改 AGENTS、另设记忆目录或改写历史验收记录。

本轮未验证 Linux、远端 CI、跨设备模型稳定性；未实现网页、LLM Space 适配、源码行断点、学习进度存储或自动评分。
课程是源码观察计划，不保证教学效果。临时目录清理不覆盖 SIGKILL/系统崩溃；取消不回滚已完成副作用。
只读快照仍可能含敏感数据，队列限项不代表全局字节预算，文件工具不是 OS sandbox。
所有结果只覆盖本机合成场景；无云 fallback、shell 授权或 Git 发布操作。
