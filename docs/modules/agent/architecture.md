# Agent 调度与会话：架构

源码：[agent.rs](../../../src/agent.rs)。配套：[设计](design.md) · [总索引](../../README.md)。

## 职责

`Agent<M>` 把一个用户输入推进为多步模型/工具交互，同时独占当前会话历史。
它决定何时请求模型、何时提交完整消息、按什么顺序调用工具，以及何时结束 run。

不负责 HTTP 协议、具体文件/进程操作、UI、配置加载、signal handler、持久化或任务成功评估。
“模型不再调用工具”是自然结束条件，不等同于用户需求已通过验收。

## 组合与依赖

| 组成 | 持有关系 | 职责 |
|---|---|---|
| `model: M` | Agent 持有实现 `Model` 的具体类型 | 推理与暂态流式输出 |
| `tools: ToolRegistry` | Agent 持有已配置的 registry | 查找/授权并调用可信工具 |
| `config: AgentConfig` | Agent 持有 | system prompt 与非零步数上限 |
| `messages: Vec<Message>` | Agent 独占可变 | 跨 user turn 的内存历史 |
| `state: RunState` | Agent 独占可变 | 可读取的阶段与结束结果 |
| token / event callback | run 调用期间借用 | 宿主请求取消、消费通知 |
| 可选 `DebugSession` | `run_debug` 消费，run 期间持有 | 有界控制接收端、单步模式与暂停序号；不持有 Agent |

[CLI](../cli/architecture.md) 是实际宿主；未来其它宿主也应调用相同库 API。
`ModelRequest` 临时借用 system、messages 和可见工具定义，provider 不能获得会话的可变引用。
Tool 只接收自己的 JSON 参数和取消信号，不回指 Agent，也不直接提交消息。

## 生命周期与主链路

```text
宿主创建 Agent（Ready）
  → run(&mut self)：User 提交
    → Model phase：delta 暂态通知 → 完整消息校验 → Assistant 提交
    → Tools phase：逐个调用或补 cancelled → Tool 结果提交
    → 再次 Model / Completed / Cancelled / StepLimit / Failed
  → Finished，可接受下一个 user turn
```

一个 run 包含多个 step，一个 step 是一次模型请求及它返回的工具批次。
调试的 Step 命令则只释放一次模型/一次工具/最终结束动作，不能与这里的模型 step 计数混用。
`&mut self` 在调用期间独占 Agent，防止正常 Rust 调用者并发启动两个 run。
`messages()` / `state()` 只提供只读引用；没有清空、任意追加、外部 setter 或恢复导入入口。

Agent 不 spawn 子任务。模型 future 可以取消丢弃；工具 future 要由执行实现完成清理后再返回。
CLI 发 token 后继续等待 run；Agent 不以一个统一超时直接中断任意副作用工具。
每个工具结果提交后让出调度，即使未知/拒绝工具立即完成，也给宿主机会处理输出与取消。

## 调试运行

`run_debug` 与 `run` 共享执行循环，前者在安全边界协作等待控制，后者保持自动运行。
Agent 设置 `Paused` 并通过事件发送 owned 快照；仍持有同一 `&mut self`，不会把历史修改权交给 Controller。
首次模型请求前、完整 Assistant 提交后、每条 Tool 结果提交后均可暂停；模型/工具执行中不强行暂停 future。
工具结果后的边界意味着工具已返回且其清理 owner 完成正常清理路径，不是仅仅收到一个 stdout 标记。

控制 channel、pause ID、防旧命令重放和快照字段属于[调试模块](../debugger/design.md)。
Agent 仍负责所有调用结果配对；暂停处取消同样沿原工具补齐逻辑结束，不等待额外 Finish 命令。

## 关键取舍

- **单一状态 owner**：不用全局上下文或包住整个 Agent 的大锁；并发复杂度留在真实资源 owner。
- **串行工具**：先保持文件副作用和 transcript 顺序一致。未来并行化需要资源冲突及提交顺序设计。
- **完整消息再提交**：模型流式失败不会在 transcript 中留下可执行的半条助手消息。
- **错误工具结果可继续推理**：工具失败反馈给模型；模型/协议失败则终止本次 run。
- **取消不回滚**：历史记录已发生的事实，对未执行调用补结果，不假装修改消失。

## 故障责任与限制

校验、状态推进、提交顺序和 step limit 属于 Agent；权限属于[工具运行时](../tool-runtime/design.md)，
子进程清理属于[Shell](../shell-tool/design.md)，流完整性属于[模型适配器](../model/design.md)。
最终事件发出与 stdout 交付不同，后者由[事件输出](../event-output/design.md)负责。

意外 drop 或 callback panic 可使 run 停在 Running 或 Paused；后续调用明确拒绝恢复不确定状态。
这不是完整的 crash recovery：已发生的外部副作用仍存在，历史也没有持久化。
会话长期增长没有 token/内存总预算；max_steps 只限制本次 run 的模型请求次数。
调试快照额外复制历史；这是观察成本，不是持久化或固定内存预算。

## 维护影响

改调度顺序要同时检查消息配对、工具副作用、取消交付、Model 借用范围和宿主输出公平性。
改会话恢复/持久化要先定义提交成功与落盘的关系，不能在 `commit` 后加 best-effort 写盘就宣称可恢复。
具体接口、结束矩阵和回归入口在[Agent 设计](design.md)。
