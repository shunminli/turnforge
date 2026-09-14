# 模型边界：架构

状态：对应当前 M0 实现。本文描述责任和依赖；字段、协议和错误细节见[设计文档](design.md)。
返回[文档入口](../../README.md)。

## 1. 模块定位

本模块把“借用一份会话，得到一条完整 Assistant 消息”与具体模型服务协议隔离。
公共边界位于 [model.rs](../../../src/model.rs)，唯一生产适配器位于 [openai.rs](../../../src/openai.rs)。
当前适配的是 Chat Completions SSE 的纯文本和 function tools 子集，不是所有 OpenAI API。

职责包括：

- 定义 `ModelRequest`、`ModelDelta`、`ModelError` 和 `Model::complete`。
- 把内部消息和工具定义转换为 HTTP 请求，处理模型认证和请求超时。
- 解析流式响应、发出暂态增量、组装并校验完整 `AssistantMessage`。
- 在请求结束、错误或取消时释放当前请求资源，不启动工具。

明确不负责：会话提交、工具权限和执行、运行步数、持久化、UI、审批以及重试策略。
这些边界分别属于 [Agent](../agent/architecture.md)、工具层和宿主，而不是 provider。

## 2. 依赖与调用链

```text
CLI 宿主构造 OpenAiModel（endpoint / key / model / timeout）
  → Agent<M> 持有 M
      → complete(ModelRequest 借用, cancel 借用, delta callback 借用)
          → request: 内部消息 → Chat Completions JSON → reqwest
          → eventsource-stream → Assembly.push → ModelDelta → 宿主
          → [DONE] → Assembly.finish → AssistantMessage
      → Agent 再校验 → 提交消息 → 决定是否执行工具
```

`model.rs` 只依赖内部消息、工具定义和取消信号，不依赖具体 HTTP SDK。
`openai.rs` 依赖 `reqwest` 的客户端/响应流、`eventsource-stream` 的 SSE framing、
`serde_json` 的协议转换及 Tokio 的取消竞争和调度让出。
模型适配器不访问工作区、`ToolRegistry::execute` 或会话的可变引用。
事件包装、消息提交和工具结果回填由 [agent.rs](../../../src/agent.rs) 完成。

通用 run/debug 使用 `OpenAiModel::new`；本地 [Lab](../harness-lab/architecture.md) 使用
`OpenAiModel::new_local`，构造时限定字面 loopback 地址、没有 key 参数并显式禁用环境代理。
两者复用私有构造和同一请求/SSE 路径，不通过修改进程全局环境临时切换网络策略。
Ollama 版本/digest 检查不属于通用 provider，仍由 Lab 预检负责。

## 3. Ownership 与生命周期

| 对象 | owner / 借用关系 | 生命周期与可变状态 |
|---|---|---|
| `OpenAiModel` | CLI 构造后 move 给 `Agent<M>` | 与 Agent 同寿命；持有 HTTP Client、endpoint 和 model ID |
| `ModelRequest<'a>` | 按值传入，字段只读借用 system/messages/tools | 只在一次调用期间使用，不保存会话引用 |
| `CancellationToken` | 宿主持有，provider 借用 | 通知当前请求取消，不存进长期 provider |
| `emit` | 调用方提供，provider 临时独占可变借用 | 同步回调，调用方必须保证不阻塞、不 panic |
| 请求 JSON / Response / SSE stream | `request` future | 仅当前请求持有；future 完成或 drop 时释放 |
| `Assembly` | `request` future 独占 | 文本、工具片段、usage 和 finish reason 的唯一写入者 |
| `AssistantMessage` | `Assembly::finish` 返回 owned value | 交给 Agent；provider 不再持有或改写它 |

`complete` 使用 `&self`，`Model: Send + Sync` 允许实现被并发借用；每个 HTTP 调用的
组装状态仍是局部变量。当前 `Agent::run(&mut self)` 对同一会话串行调用模型。
模型边界不要求 provider 保存会话，也没有为会话引入 `Arc<Mutex<_>>`。
认证头由 Client 的默认 headers 持有；没有实现 provider 的 `Debug`，避免误输出授权材料。

## 4. 两阶段结果：暂态展示与已验证事实

`ModelDelta` 在 SSE 到达时就可以发出，但不代表服务端正常完成，也不授权执行工具。
只有 `[DONE]` 触发 `Assembly::finish` 且校验通过，才返回一条完整消息。
如果最后发现截断、非法参数或重复 ID，之前已经展示的 delta 不能被撤回，
但 Agent 不会提交半条 Assistant，也不会执行这些暂态工具片段。

这种分离保留响应速度，同时把副作用的入口放在完整消息校验之后。
具体事件语义与序列化属于[消息与事件协议](../protocol/design.md)。
适配器校验 wire 协议，Agent 还会独立校验通用工具调用结构；二者责任不同，
后者保护将来不经过 OpenAI SSE 的其它 `Model` 实现。

## 5. 取消和资源闭合

`OpenAiModel::complete` 通过优先检查取消的 `tokio::select!` 竞争当前请求。
取消获胜时返回 `ModelError::Cancelled`，丢弃本地 HTTP future；本模块没有自建后台任务。
每处理一个普通 JSON SSE event 会 `yield_now()`，让宿主有机会排空输出和发出取消。
这不保证同步 callback 的取消时延，因此 callback 不得执行阻塞 I/O。

Agent 也在模型阶段竞争取消，故所有 `Model` 实现都必须允许 future 被安全丢弃。
这个合同不能套用于有文件/进程副作用的工具；工具取消仍需等待清理，见 Agent 文档。
丢弃 HTTP future 只终止本地等待，不承诺远端服务已经停止生成或不再计费。

## 6. 设计取舍与扩展边界

- 直接维护小型 HTTP/SSE 适配器：支持自选兼容 endpoint，也必须自行维护协议校验。
- 通用消息保持纯文本/function tools：减少 M0 面积，但不能无损承载多模态、reasoning 或签名块。
- 工具按 SSE index 排序组装：恢复稳定的调用顺序，不把网络分片顺序变成执行顺序。
- 不自动重试：错误返回真实失败，避免隐式重复请求、成本和暂态事件混淆。
- 不暴露任意 JSON 参数透传：新配置应先说明真实消费者和兼容性，再增加显式字段。

增加 Anthropic 或 Responses 适配器时，先确认现有 `AssistantMessage` 是否能表达其信息，
不能把不可表示的信息悄悄丢掉后宣称兼容。模型名称并不自动选择协议。
增加重试、usage 汇总或上下文裁剪时，应先确定 owner 和跨尝试的事件/费用合同。
provider 的请求转换可以变化，但不能越过 Agent 的消息提交边界或自行执行工具。

## 7. 维护入口

先读 [Model contract](../../../src/model.rs)，再读 `OpenAiModel::new/new_local/request` 和 `Assembly`。
主要跨边界证据是 [cli_http.rs](../../../tests/cli_http.rs)：真实 TCP → CLI → 文件 → 下一次请求。
Agent 对模型失败和取消的处理由 [agent.rs 测试](../../../tests/agent.rs)补充。
具体用例映射、尚未覆盖的协议分支与修改清单见[设计文档](design.md)。
