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
| 原生语义调试 | [debug.rs](../src/debug.rs)、Agent 安全点 | [架构](modules/debugger/architecture.md) | [设计](modules/debugger/design.md) |
| 模型接口与 OpenAI 适配 | [model.rs](../src/model.rs)、[openai.rs](../src/openai.rs) | [架构](modules/model/architecture.md) | [设计](modules/model/design.md) |
| 工具注册与权限运行时 | [tools/mod.rs](../src/tools/mod.rs) | [架构](modules/tool-runtime/architecture.md) | [设计](modules/tool-runtime/design.md) |
| 工作区与文件工具 | [tools/files.rs](../src/tools/files.rs) | [架构](modules/filesystem-tools/architecture.md) | [设计](modules/filesystem-tools/design.md) |
| Shell 执行与进程清理 | [tools/shell.rs](../src/tools/shell.rs) | [架构](modules/shell-tool/architecture.md) | [设计](modules/shell-tool/design.md) |
| CLI 宿主与配置 | [main.rs](../src/main.rs)、[debug_input.rs](../src/debug_input.rs) | [架构](modules/cli/architecture.md) | [设计](modules/cli/design.md) |
| 事件输出与背压 | [output.rs](../src/output.rs) | [架构](modules/event-output/architecture.md) | [设计](modules/event-output/design.md) |
| 本地 Harness Lab | [lab.rs](../src/lab.rs)、[launcher](../scripts/harness-lab.sh) | [架构](modules/harness-lab/architecture.md) | [设计](modules/harness-lab/design.md) |
| 学习路线与现场解释 | [learning.rs](../src/learning.rs) | [架构](modules/learning/architecture.md) | [设计](modules/learning/design.md) |

[lib.rs](../src/lib.rs) 是公共导出入口，其职责由系统架构与各模块 API 文档共同覆盖。
`ModelDelta` 的 Rust 定义在模型模块；它进入 Event 的封装与消息区别由协议文档解释。
未来新增 provider 如果形成独立的协议/生命周期边界，应拥有自己的文档对，而不是无限扩写 model 文档。

## 按维护任务导航

| 要做的变更 | 先读 | 同时检查的消费者 |
|---|---|---|
| 新增消息字段、事件类型、输出格式 | 协议设计 | 模型序列化、Agent 提交、事件输出和外部消费者 |
| 修改循环、取消、步数上限、并行工具 | Agent 设计 | Model drop 合同、Tool cleanup 合同、CLI 背压 |
| 调试命令、暂停点、快照或外部调试前端 | Debugger 设计 | Agent 单 writer、协议序列化、CLI 输入取消、事件输出 |
| 新接模型服务、流式协议、模型鉴权 | 模型设计 | 协议可表达性、CLI 配置、真实 HTTP 测试 |
| 新工具、权限或审批 | 工具运行时设计 | CLI 授权、工具输入验证、Agent 失败与取消 |
| 文件读写、路径、原子替换 | 文件工具设计 | 工具 schema、权限和实际文件副作用测试 |
| 子进程、超时、shell 环境 | Shell 设计 | 进程组退出、双管道、CLI SIGINT 测试 |
| 终端/服务/IDE 新宿主 | CLI 和事件输出架构 | Agent 无 UI 依赖、sink 非阻塞与结束交付 |
| 一键本地实验、基线、临时场景或 PASS 验收 | Harness Lab 设计 | CLI 控制/输出生命周期、模型本地构造、真实文件 oracle |
| 学习路线、源码导读和暂停解释 | 学习设计 | 相关源码 owner、Lab 任务与验收不受课程影响 |

## 全局文档

- [系统架构](architecture.md)：模块关系与端到端控制流。
- [文档维护规范](documentation-guide.md)：如何更新、检查和扩展文档体系。
- [本地 LLM 指南](local-llm.md)：固定 Ollama/小模型基线、前台服务生命周期与显式启用的真实模型冒烟。
- [原生调试指南](debugging.md)：语义单步、只读快照、控制命令、CLI 自动化和能力边界。
- [Harness Lab 指南](harness-lab.md)：一键启动合成实验、快捷控制和固定本地模型验收。
- [Harness 学习指南](harness-learning.md)：六阶段路线、源码入口和边调试边学的人工 rubric。
- [Lab 与学习实现计划](harness-lab-plan.md)：入口、权限、生命周期及本轮完成证据。
- [Lab 与学习验收记录（2026-09-14）](verification/harness-lab-2026-09-14.md)：冻结实现、首次失败、独立门禁和真实用户入口验证。
- [调试实现计划](debugging-plan.md)：本轮实现范围与验收台账，不代替模块当前设计。
- [原生调试验收记录（2026-09-14）](verification/debugging-2026-09-14.md)：冻结输入、确定性与本地模型测试、修复和验收边界。
- [本地 LLM 回归记录（2026-09-12）](verification/local-llm-2026-09-12.md)：已执行用例、输入指纹、环境与结果边界；属于历史证据。
- [Helixent 迁移路线](helixent-migration.md)：参考基线、差异与未实现能力。
- [代码来源说明](provenance.md)：来源和许可证检查边界。
- [M0 实现计划](implementation-plan.md)、[M0 验证记录](verification.md)：阶段性历史证据，不作为永久基线或实时状态。

## 证据入口

源码是现状的直接证据。测试提供有限的行为证明，未覆盖项在模块设计文档显式列出。

- [Agent 测试](../tests/agent.rs)：会话顺序、取消、失败、步数与中断恢复限制。
- [调试状态机测试](../tests/debugging.rs)：逐模型/工具动作、只读快照、pause ID、取消清理和普通 run 兼容。
- [工具测试](../tests/tools.rs)：输入/权限/路径、文件副作用、进程与管道。
- [CLI/HTTP 测试](../tests/cli_http.rs)：真实 TCP、binary、NDJSON、信号与输出消费。
- [Harness Lab/学习入口测试](../tests/lab_cli.rs)：预检、环境隔离、真实文件 oracle、快捷控制和无模型课程。
- [本地 LLM 冒烟](../tests/local_llm.rs)：真实本机模型的文本、读文件、写文件和单步调试闭环；默认忽略，不替代确定性回归。
- [CI](../.github/workflows/ci.yml)：目前执行 Rust 格式、Clippy、debug/release 测试和构建；不自动证明文档与代码语义一致。
