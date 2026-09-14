# Turnforge 系统架构

本页描述当前 M0 的模块组合与关键决策；每个模块的详细架构和设计从[文档总索引](README.md)进入。
当前只有一个 Cargo crate，同时提供 library 和 CLI；逻辑模块划分不要求拆成多个 crates。

## 系统边界

Turnforge 是宿主与模型之间的 headless 调度内核：宿主提供输入、工具授权、取消信号和事件接收端，
Agent 管会话和模型/工具循环。模型响应和工具输出是数据，不是修改宿主权限的指令通道。

```text
CLI 宿主（配置、prompt / 调试命令输入、授权、Ctrl-C）
  ├─ Lab（可选）：本机模型预检、临时场景、结束后独立验收
  ├─ Learning（可选）：静态路线 / 快照解释 → 宿主文本输出
  ├─ DebugController → DebugSession（可选控制；安全边界等待）
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

`Agent::run_debug` 复用同一主链路，在首次模型前、完整 Assistant 提交后和每个 Tool 结果提交后提供安全边界。
只有 Agent 决定什么时候已安全暂停；宿主通过带 pause ID 的命令释放一步或连续运行，不改写历史。
命令输入、事件输出和 signal 属于 CLI；详细顺序与取消合同见[调试设计](modules/debugger/design.md)。

`turnforge lab` 组合相同 run_debug，通过固定 Ollama 基线和独占临时场景降低试用成本；
自动模式只在真实暂停事件后发 Step，结束后根据已提交消息与磁盘做独立验收，不把 Completed 当任务通过。
`turnforge learn` 无模型地显示课程；`lab --lesson` 只增加解释，不改变运行、权限或验收。
边界分别由 [Lab](modules/harness-lab/architecture.md) 和[学习模块](modules/learning/architecture.md)维护。

正常协作退出时，所有已提交调用都要有结果。工具实际清理保证、意外 drop/强杀、
外部消费者是否收到事件的边界分别见 [Agent](modules/agent/design.md)、[Shell](modules/shell-tool/design.md)
及[事件输出](modules/event-output/design.md)，不能用一句“支持取消”替代这些合同。

## 关键所有权

| 状态 / 资源 | owner | 临时消费者 | 结束责任 |
|---|---|---|---|
| transcript、运行阶段 | `Agent<M>` | Model 只读借用，宿主只读观察 | run 设置终态；Agent drop 释放内存 |
| 调试命令与暂停状态 | 宿主持有 Controller；run 消费 Session；Agent 写 RunState | 观察端接收 owned 快照 | Controller drop 取消；宿主仍 await run 清理 |
| HTTP 请求与 SSE assembly | Model / `OpenAiModel::request` | callback 获得增量值 | provider 完成或取消丢弃请求 |
| 工具定义、权限、实现 | `ToolRegistry` | Model 获得可见定义快照 | registry/Agent 生命周期 |
| 文件 worker、临时文件 | `FileTool::execute` / `atomic_write` | Agent 等待结果 | await worker，成功 persist 或 drop 临时文件 |
| shell、进程组、双管道 | `run_unix` | Agent 等待 ToolOutput | kill/wait/capture，具体失败边界见模块设计 |
| 事件 sender/receiver、stdout fd | CLI run 与 writer | stdout 下游 | sender 关闭、join writer、恢复 fd flags |
| Lab 临时工作区、合成预期事实 | CLI 持有 PreparedLab | 工具借路径、oracle 读消息/磁盘 | run/验收/输出完成后 TempDir drop |
| 课程和现场解释 | learning 的静态内容/纯函数 | CLI 借 snapshot 得到 owned String | 无资源或后台任务生命周期 |

## 关键架构选择

- **headless core**：宿主可替换；TUI/服务/IDE 不应成为 Agent 修改会话的特殊入口。
- **单 writer 会话**：`&mut Agent` 和私有历史约束修改者；不以全局大锁共享可变上下文。
- **模型/工具两个扩展边界**：Model 可在取消时 drop；Tool 必须管理副作用并完成清理后返回。
- **先串行工具**：调用顺序、文件副作用和历史顺序一致；并行化需要先设计资源冲突规则。
- **暂态与提交分离**：delta 用于观察；Assistant 校验成功后才进入历史，MessageCommitted 只是内存提交。
- **安全限制放在实际 owner**：registry 检查 capability，文件工具检查路径，shell 明示宿主权限；prompt 不替代隔离。
- **输出有独立生命周期**：不能让同步 stdout 阻塞整个调度器；背压与 deadline 的精确范围由输出设计定义。
- **调试复用执行 owner**：暂停在 async 安全边界，不阻塞 emit，不另建 runtime 或共享可变会话；工具 future 仍被 await。

## 能力边界与演进

当前支持 Chat Completions 的文本/function tools 子集、基础文件/shell 工具、单进程内存会话和原生语义调试。
本地 Lab 提供有限合成场景验收及六课学习提示，不是完整 benchmark、学习认证或新 UI runtime。
没有持久化恢复、上下文压缩、交互审批、skills/AGENTS 自动加载、Anthropic、Responses API、TUI 或 OS sandbox。
`--allow-shell` 不是工作区沙箱，也不受 `--allow-write` 限制；静态路径检查不是抗恶意并发变更的安全隔离。
调试快照含完整逻辑上下文但不是 provider 原始请求或恢复 checkpoint；没有 LLM Space 接入、历史修改或副作用撤销。

跨模块扩展先读对应设计，再更新 owner 和直接消费者的文档；不要把上游 roadmap 当成本项目已实现能力。
后续路线见[迁移对照](helixent-migration.md)，日常维护按[文档规范](documentation-guide.md)，
当前阶段的实际测试与限制见[M0 验证记录](verification.md)。
