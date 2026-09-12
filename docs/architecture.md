# Turnforge 系统架构

本页描述当前 M0 的模块组合与关键决策；每个模块的详细架构和设计从[文档总索引](README.md)进入。
当前只有一个 Cargo crate，同时提供 library 和 CLI；逻辑模块划分不要求拆成多个 crates。

## 系统边界

Turnforge 是宿主与模型之间的 headless 调度内核：宿主提供输入、工具授权、取消信号和事件接收端，
Agent 管会话和模型/工具循环。模型响应和工具输出是数据，不是修改宿主权限的指令通道。

```text
CLI 宿主（配置、输入、授权、Ctrl-C）
  ├─ Agent（唯一会话 writer）
  │    ├─ Model 接口 → OpenAI adapter → 配置的 HTTP 模型端点
  │    └─ ToolRegistry（可见性 + 权限）
  │         ├─ FileTool / Workspace → 本地文件系统
  │         └─ ShellTool → Unix 子进程与管道
  └─ 事件输出（有界队列、writer）→ stdout / 下游消费者

Message / Event 为这些边界提供共同数据类型；不是独立执行服务。
```

这是运行关系，不是严格的 Rust import 图。公共导出入口是 [lib.rs](../src/lib.rs)，
宿主组合入口是 [main.rs](../src/main.rs)；库不加载终端、全局环境配置或 signal handler。

## 端到端主链路

1. 宿主解析配置，创建 workspace、provider、registry 和 Agent，建立取消信号及输出通道。
2. `Agent::run` 提交 User，借用历史及工具定义请求 Model；流式 delta 作为暂态 Event 传给宿主。
3. Model 返回完整 Assistant 后，Agent 校验并提交；没有 tool call 则自然结束。
4. 有 tool call 时按模型顺序经过 registry 的查找/权限检查，逐个执行或生成失败结果，再提交 Tool 消息。
5. 下一步 Model 消费完整结果；自然结束、取消、步数上限或模型失败分别有不同 RunOutcome。
6. CLI 并发等待 run 和输出 writer；取消会请求运行清理，但输出故障也可能让最终 stdout 不完整。

正常协作退出时，所有已提交调用都要有结果。工具实际清理保证、意外 drop/强杀、
外部消费者是否收到事件的边界分别见 [Agent](modules/agent/design.md)、[Shell](modules/shell-tool/design.md)
及[事件输出](modules/event-output/design.md)，不能用一句“支持取消”替代这些合同。

## 关键所有权

| 状态 / 资源 | owner | 临时消费者 | 结束责任 |
|---|---|---|---|
| transcript、运行阶段 | `Agent<M>` | Model 只读借用，宿主只读观察 | run 设置终态；Agent drop 释放内存 |
| HTTP 请求与 SSE assembly | Model / `OpenAiModel::request` | callback 获得增量值 | provider 完成或取消丢弃请求 |
| 工具定义、权限、实现 | `ToolRegistry` | Model 获得可见定义快照 | registry/Agent 生命周期 |
| 文件 worker、临时文件 | `FileTool::execute` / `atomic_write` | Agent 等待结果 | await worker，成功 persist 或 drop 临时文件 |
| shell、进程组、双管道 | `run_unix` | Agent 等待 ToolOutput | kill/wait/capture，具体失败边界见模块设计 |
| 事件 sender/receiver、stdout fd | CLI run 与 writer | stdout 下游 | sender 关闭、join writer、恢复 fd flags |

## 关键架构选择

- **headless core**：宿主可替换；TUI/服务/IDE 不应成为 Agent 修改会话的特殊入口。
- **单 writer 会话**：`&mut Agent` 和私有历史约束修改者；不以全局大锁共享可变上下文。
- **模型/工具两个扩展边界**：Model 可在取消时 drop；Tool 必须管理副作用并完成清理后返回。
- **先串行工具**：调用顺序、文件副作用和历史顺序一致；并行化需要先设计资源冲突规则。
- **暂态与提交分离**：delta 用于观察；Assistant 校验成功后才进入历史，MessageCommitted 只是内存提交。
- **安全限制放在实际 owner**：registry 检查 capability，文件工具检查路径，shell 明示宿主权限；prompt 不替代隔离。
- **输出有独立生命周期**：不能让同步 stdout 阻塞整个调度器；背压与 deadline 的精确范围由输出设计定义。

## 能力边界与演进

当前支持 Chat Completions 的文本/function tools 子集、基础文件/shell 工具和单进程内存会话。
没有持久化恢复、上下文压缩、交互审批、skills/AGENTS 自动加载、Anthropic、Responses API、TUI 或 OS sandbox。
`--allow-shell` 不是工作区沙箱，也不受 `--allow-write` 限制；静态路径检查不是抗恶意并发变更的安全隔离。

跨模块扩展先读对应设计，再更新 owner 和直接消费者的文档；不要把上游 roadmap 当成本项目已实现能力。
后续路线见[迁移对照](helixent-migration.md)，日常维护按[文档规范](documentation-guide.md)，
当前阶段的实际测试与限制见[M0 验证记录](verification.md)。
