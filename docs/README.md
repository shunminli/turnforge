# Turnforge 文档索引

这里是维护者入口，描述当前 M0 实现，不是所有规划功能已经落地的声明。
首次了解项目先读[系统架构](architecture.md)，修改代码前进入对应模块的架构与设计文档。

## 文档分工

- **架构文档 / architecture.md**：模块为何存在、负责与不负责什么、上下游关系、所有权与生命周期、取舍。
- **设计文档 / design.md**：当前接口、数据、控制流程、不变量、错误与取消语义、测试依据及变更检查。
- **路线与验证**：规划说明还未实现的能力；验证记录说明某个 artifact 实际得到什么证据。
  两者都不能代替模块当前设计。

## 核心模块地图

模块按可维护的职责划分，不等于 Cargo crate；当前仍为一个 crate。每个模块各有两份独立文档。

| 模块 | 实现 owner | 架构 | 设计 |
|---|---|---|---|
| 消息与事件协议 | [message.rs](../src/message.rs)、[event.rs](../src/event.rs) | [架构](modules/protocol/architecture.md) | [设计](modules/protocol/design.md) |
| Agent 调度与会话 | [agent.rs](../src/agent.rs) | [架构](modules/agent/architecture.md) | [设计](modules/agent/design.md) |
| 模型接口与 OpenAI 适配 | [model.rs](../src/model.rs)、[openai.rs](../src/openai.rs) | [架构](modules/model/architecture.md) | [设计](modules/model/design.md) |
| 工具注册与权限运行时 | [tools/mod.rs](../src/tools/mod.rs) | [架构](modules/tool-runtime/architecture.md) | [设计](modules/tool-runtime/design.md) |
| 工作区与文件工具 | [tools/files.rs](../src/tools/files.rs) | [架构](modules/filesystem-tools/architecture.md) | [设计](modules/filesystem-tools/design.md) |
| Shell 执行与进程清理 | [tools/shell.rs](../src/tools/shell.rs) | [架构](modules/shell-tool/architecture.md) | [设计](modules/shell-tool/design.md) |
| CLI 宿主与配置 | [main.rs](../src/main.rs) | [架构](modules/cli/architecture.md) | [设计](modules/cli/design.md) |
| 事件输出与背压 | [output.rs](../src/output.rs) | [架构](modules/event-output/architecture.md) | [设计](modules/event-output/design.md) |

[lib.rs](../src/lib.rs) 是公共导出入口，其职责由系统架构与各模块 API 文档共同覆盖。
`ModelDelta` 的 Rust 定义在模型模块；它进入 Event 的封装与消息区别由协议文档解释。
未来新增 provider 如果形成独立的协议/生命周期边界，应拥有自己的文档对，而不是无限扩写 model 文档。

## 按维护任务导航

| 要做的变更 | 先读 | 同时检查的消费者 |
|---|---|---|
| 新增消息字段、事件类型、输出格式 | 协议设计 | 模型序列化、Agent 提交、事件输出和外部消费者 |
| 修改循环、取消、步数上限、并行工具 | Agent 设计 | Model drop 合同、Tool cleanup 合同、CLI 背压 |
| 新接模型服务、流式协议、模型鉴权 | 模型设计 | 协议可表达性、CLI 配置、真实 HTTP 测试 |
| 新工具、权限或审批 | 工具运行时设计 | CLI 授权、工具输入验证、Agent 失败与取消 |
| 文件读写、路径、原子替换 | 文件工具设计 | 工具 schema、权限和实际文件副作用测试 |
| 子进程、超时、shell 环境 | Shell 设计 | 进程组退出、双管道、CLI SIGINT 测试 |
| 终端/服务/IDE 新宿主 | CLI 和事件输出架构 | Agent 无 UI 依赖、sink 非阻塞与结束交付 |

## 全局文档

- [系统架构](architecture.md)：模块关系与端到端控制流。
- [文档维护规范](documentation-guide.md)：如何更新、检查和扩展文档体系。
- [Helixent 迁移路线](helixent-migration.md)：参考基线、差异与未实现能力。
- [代码来源说明](provenance.md)：来源和许可证检查边界。
- [M0 实现计划](implementation-plan.md)、[M0 验证记录](verification.md)：阶段性历史证据，不作为永久基线或实时状态。

## 证据入口

源码是现状的直接证据。测试提供有限的行为证明，未覆盖项在模块设计文档显式列出。

- [Agent 测试](../tests/agent.rs)：会话顺序、取消、失败、步数与中断恢复限制。
- [工具测试](../tests/tools.rs)：输入/权限/路径、文件副作用、进程与管道。
- [CLI/HTTP 测试](../tests/cli_http.rs)：真实 TCP、binary、NDJSON、信号与输出消费。
- [CI](../.github/workflows/ci.yml)：目前执行 Rust 格式、Clippy、debug/release 测试和构建；不自动证明文档与代码语义一致。
