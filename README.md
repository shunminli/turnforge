# Turnforge

A lightweight agent harness in Rust, built for explicit state, reliable execution, and extensibility.

以 Helixent 为参考、用 Rust 重新实现的 **headless Agent Harness**。先建立可读、可测试的
模型—工具循环，再逐步迁移能力；不是 Helixent 的完整替代品，也不是逐文件语法翻译。

当前里程碑：**M0 / experimental**。一个 crate，同时提供 Rust library 和 CLI。
库不依赖 TUI；CLI 是第一个宿主，以后可接终端界面、服务或 IDE。

## 已实现

- 类型化会话、模型—工具循环、流式回复和 NDJSON 事件接口。
- OpenAI-compatible Chat Completions；文件读写、目录操作与 shell 工具。
- 默认只读，文件写入和 shell 分别授权；超时、协作取消、输出限额和背压处理。
- 原生语义单步调试、固定本地模型的 Harness Lab，以及联系源码的六阶段学习课程。
- 不依赖真实模型的确定性测试，以及显式启用的本地 Ollama 回归。

## 快速开始

开发和 CLI 目前以 **macOS / Linux** 为目标。按你的任务选择入口：

- **第一次使用**：[Turnforge 101 用户指南](docs/101-user-guide.md)，完成首次运行和一次单步体验。
- **从零学习 Harness 内核**：[学习路线](docs/harness-learning.md)，按顺序进入 `tutorial/` 的实验与综合练习。
- **运行自己的任务或接入模型**：[CLI 使用参考](docs/cli.md)。
- **查找某项操作或实现合同**：[文档总索引](docs/README.md)。

101 负责“先用起来”，教程负责“理解并能修改”；两者不是两套完整课程。

## 安全与范围

- 文件工具的路径限制不是 OS sandbox，只适用于可信本地工作区。
- `--allow-shell` 是独立的宿主机 shell 授权，能访问工作区外文件和网络；不受文件工具路径限制。
- 文件读取会把内容发给所配置的模型服务；默认没有秘密文件过滤器。事件与调试快照也可能含敏感数据。
- 取消不是回滚，已经发生的文件写入或命令副作用不会撤销。

具体授权方式见 [CLI 使用参考](docs/cli.md#工作区与授权)；路径、进程和输出限制分别由
[文件工具设计](docs/modules/filesystem-tools/design.md)、[Shell 设计](docs/modules/shell-tool/design.md)和
[输出设计](docs/modules/event-output/design.md)维护。

当前没有持久化/恢复、自动重试、上下文压缩、交互审批、skills/AGENTS 自动加载、MCP、
Anthropic、OpenAI Responses API、图片/推理块、TUI、subagent 或 OS sandbox。

## 开发与维护

[测试执行参考](docs/testing.md)是确定性回归、真实模型测试和失败定位的统一入口。
默认测试不调用真实模型；本地模型通过不能替代 CI 门禁。`Cargo.lock` 纳入版本管理。

先读[系统架构](docs/architecture.md)，再从[核心模块地图](docs/README.md#核心模块地图)进入
对应模块独立的架构与设计文档。修改时遵循[文档维护规范](docs/documentation-guide.md)。
[Helixent 迁移路线](docs/helixent-migration.md)区分现有与规划能力，历史证据由[索引](docs/README.md#路线与历史证据)单独归档。

## License / 来源

Turnforge 使用 Apache-2.0，见 [LICENSE](LICENSE)。参考项目、基线和代码来源限制见
[provenance](docs/provenance.md)。依赖保留各自许可证。
