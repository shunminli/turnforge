# Turnforge

A lightweight agent harness in Rust, built for explicit state, reliable execution, and extensibility.

以 Helixent 为参考、用 Rust 重新实现的 **headless Agent Harness**。先建立可读、可测试的
模型—工具循环，再逐步迁移能力；不是 Helixent 的完整替代品，也不是逐文件语法翻译。

当前里程碑：**M0 / experimental**。一个 crate，同时提供 Rust library 和 CLI。
库不依赖 TUI；CLI 是第一个宿主，以后可接终端界面、服务或 IDE。

## 已实现

- `Agent::run(&mut self, ...)` 单一会话所有者，类型化消息、阶段和结束原因。
- OpenAI-compatible Chat Completions SSE：文本增量、碎片工具参数、可选 usage。
- `read_file`、`list_files`、`write_file`、`str_replace`、`mkdir`、`bash`。
- 工具串行执行；错误成为工具结果；取消后为尚未执行的调用补齐结果。
- 默认只读；写文件和 shell 必须分别显式授权。
- Ctrl-C 协作取消、模型请求超时、shell 超时、Unix 进程组清理、输出限额。
- NDJSON 事件接口；本地 HTTP → 真实 CLI → 文件/子进程的集成测试。

## 快速开始

开发和 shell 工具目前以 **macOS / Linux** 为目标。需要 Rust/rustup 和 `/bin/bash`。
仓库通过 `rust-toolchain.toml` 固定已验证的 Rust 版本。

```sh
source "$HOME/.cargo/env"
cargo build --locked
cargo run --locked -- tools
```

使用你的模型服务（模型必须支持 Chat Completions 的 function tools）：

```sh
export TURNFORGE_MODEL='your-model-id'
export TURNFORGE_BASE_URL='https://your-provider.example/v1'
# 在自己的终端安全设置 TURNFORGE_API_KEY；不要把 key 写进仓库或命令参数。
# 未设置 TURNFORGE_API_KEY 时，CLI 会回退到 OPENAI_API_KEY。

cargo run --locked -- run '读取 README.md，概括项目结构' --workspace .
cargo run --locked -- run '在 README.md 中补充测试说明' --workspace . --allow-write
```

`TURNFORGE_BASE_URL` 缺省是 `https://api.openai.com/v1`；不会自动选择模型。
本地无鉴权模型可使用 `http://127.0.0.1:端口/v1`，并取消上述 key 环境变量。
程序不会读取 Codex、Claude Code 的凭据，也没有自动登录流程。

授权执行 shell（拥有宿主机权限，请先读下面的安全边界）：

```sh
cargo run --locked -- run '运行测试并解释失败，不修改文件' --workspace . --allow-shell
```

机器集成、stdin 输入与限制：

```sh
printf '%s' '读取 Cargo.toml 并解释依赖' | cargo run --locked -- run - --json
cargo run --locked -- run '分析项目' --max-steps 10 --request-timeout 120 --tool-timeout 30
cargo run --locked -- run --help
```

`--json` 的 stdout 每行一个事件。事件包含用户输入、模型文本、工具参数和结果，
可能包含敏感数据；不要随意上传。普通文本模式中的流式文本是暂态结果，是否成功以结束状态为准。
输出使用有界队列和非阻塞管道；下游不消费、队列溢出或写入持续阻塞会终止运行。
此时 NDJSON 可能不完整，不能要求一个已经堵塞/断开的输出端仍收到 terminal event。

退出码：`0` 模型自然结束，`1` 运行/协议错误，`2` 达到步数上限（也是 CLI 参数错误码），
`130` 取消。自然结束不等于用户任务一定完成；宿主仍需检查结果和文件。

## 安全与范围

- CLI 注册全部内置工具，默认只向模型展示读工具；即使模型臆造写操作，运行时也会再次拒绝。
- `--allow-write` 允许工作区内文件写入；`--allow-shell` 是**独立的、不受文件工具路径限制的完整 shell 授权**。
  仅开 shell 也能修改或读取工作区外的文件、访问网络，绝非“只读 shell”。
- 文件工具只接受相对路径，拒绝 `..`、绝对路径及静态 symlink 穿越。
  路径检查不抵御恶意并发替换目录；只用于可信本地工作区，不是 OS sandbox。
- 文件读取会把内容发给所配置的模型服务；默认没有秘密文件过滤器。
- shell 清理普通同进程组后代；主动 `setsid` 逃离的进程需要 OS sandbox 才能可靠约束。
  子进程移除了常见模型 key 环境变量，但仍继承其它环境及宿主文件访问权。
- 取消不是回滚；已经完成的文件写入或命令副作用保留。强制杀掉宿主不承诺恢复。
- 单次 SSE 响应最多 2 MiB；单文件最多 4 MiB；读取输出最多 32 KiB；shell 每路输出最多 32 KiB。
  达到 shell 输出限额后继续排空管道；文件写入要求父目录存在。

当前没有持久化/恢复、自动重试、上下文压缩、交互审批、skills/AGENTS 自动加载、MCP、
Anthropic、OpenAI Responses API、图片/推理块、TUI、subagent 或 OS sandbox。

## 开发与学习

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo test --locked --release --all-targets
cargo build --locked --release
```

测试无需 API key，也不调用付费模型。`Cargo.lock` 纳入版本管理。
CI 已配置 macOS/Linux 门禁；远端结果要在推送后确认。

维护入口是[文档总索引](docs/README.md)。8 个核心模块分别有独立的架构与设计文档，
涵盖职责、所有权、接口、状态、取消/错误、安全边界及对应测试。

建议依次阅读[系统架构](docs/architecture.md)、[协议设计](docs/modules/protocol/design.md)、
[Agent 设计](docs/modules/agent/design.md)，再按索引进入模型、工具、CLI 和输出模块。
[Helixent 迁移路线](docs/helixent-migration.md)区分现有与规划能力；[验证记录](docs/verification.md)保留 M0 的验证证据。
修改模块时遵循[文档维护规范](docs/documentation-guide.md)，同步更新实际变化的合同，不把未来方案写成已实现。

## License / 来源

Turnforge 使用 Apache-2.0，见 [LICENSE](LICENSE)。参考项目、基线和代码来源限制见
[provenance](docs/provenance.md)。依赖保留各自许可证。
