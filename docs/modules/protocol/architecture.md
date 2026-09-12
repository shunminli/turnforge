# 消息与事件协议：架构

源码：[message.rs](../../../src/message.rs)、[event.rs](../../../src/event.rs)；
公共导出：[lib.rs](../../../src/lib.rs)。配套：[设计](design.md) · [总索引](../../README.md)。

## 职责与非职责

模块给模型、调度器、工具和宿主提供共同的数据语言：

- `Message` 表达用户、完整助手回复和工具结果。
- `ToolCall` / `ToolOutput` 关联请求与执行事实，`Usage` 保存可选的提供方计数。
- `Event` 表达运行中可观察的通知，`RunState` / `Phase` / `RunOutcome` 区分阶段与结束原因。

这里不运行模型或工具、不修改会话、不负责输出队列、持久化、时钟或 ID 生成。
纯数据结构没有后台任务、文件句柄或主动取消行为。类型能区分变体，不能单独证明消息序列合法。

## 两种数据，不是两份事实来源

| 数据 | 产生 / 所有者 | 消费者 | 语义 |
|---|---|---|---|
| `Vec<Message>` | `Agent` 独占并提交 | `ModelRequest` 只读借用 | 当前进程内模型上下文 |
| `ModelDelta` | Model 实现 | Agent 包装为 Event，宿主显示 | 暂态增量，可能最终失败 |
| `MessageCommitted` | Agent 先写历史，再发送克隆值 | 宿主观察或输出 | 已进入内存 transcript，不表示落盘 |
| `RunFinished` | Agent 完成协作运行后产生 | 宿主决定显示/退出 | 运行结束通知，不是任务正确性的证明 |

`Event` 包装的 `ModelDelta` 定义位于 [model.rs](../../../src/model.rs)，由[模型模块](../model/architecture.md)
负责其生成语义。逻辑上都属于跨边界数据，但不为文档分类移动 Rust 类型。

## 依赖与数据流

```text
用户输入 → Agent → Message::User
模型 → delta → Event::ModelDelta → 宿主（可以先显示）
模型 → 完整 AssistantMessage → Agent 校验/提交 → MessageCommitted
工具 → ToolOutput → Agent 关联 call_id/提交 → MessageCommitted
```

[Agent](../agent/architecture.md) 是会话唯一写入者；消费者取得的 Event 值是克隆或独立数据。
修改消费者自己的对象不会修改 Agent 的历史。消息数据使用 owned String/Vec/Value，不借用 HTTP buffer。
运行期间 `ModelRequest` 才创建短生命周期只读借用，不能把它当成全局共享会话。

[模型适配器](../model/design.md) 把内部 Message 翻译为外部协议；内部 serde JSON 不是 OpenAI wire schema。
[工具运行时](../tool-runtime/design.md) 返回输出，不持有 Message 或 transcript 的可变引用。
[事件输出](../event-output/architecture.md) 负责传输，不是事实提交 owner。

## 关键取舍

- enum 表达互斥角色/状态，避免用多个布尔值和可空字段拼装“当前是否执行中”。
- 消息与事件分开：显示增量可以失败，完整消息只能在校验后提交。
- `ToolOutput::Error` 保留机器可区分的 code；工具失败不是必须终止整个 Agent 的异常。
- system prompt 保存在 `AgentConfig`，不放入 `Message`；不要假设导出历史已包括完整运行配置。
- 数据可 serde 序列化，但没有协议版本、事件 sequence、run ID 或 durable commit 保证。
  可序列化不等于可直接用作稳定存储格式。

## 维护边界

消息字段变更同时影响 provider 请求翻译、工具结果、事件输出和潜在宿主消费者。
事件变体变更影响所有匹配 Event 的代码；仅修改 JSON 标签也可能破坏外部消费者。
新增图片/推理块、持久化或 replay 前应先明确数据归属和兼容策略，不能把占位字段当已支持能力。

协议本身没有敏感信息过滤，事件可带完整用户输入、文件内容和命令参数；日志上传必须由宿主做决定。
取消、配对校验、最终事件何时发出等现行规则详见[协议设计](design.md)和[Agent 设计](../agent/design.md)。
