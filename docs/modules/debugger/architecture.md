# 原生语义调试：架构

状态：已实现。实现入口：[debug.rs](../../../src/debug.rs)、
[agent.rs](../../../src/agent.rs)。配套：[设计](design.md) · [使用指南](../../debugging.md) · [总索引](../../README.md)。

## 目标与非目标

让宿主观察并控制**同一个正在运行的 Agent**：在模型请求前、完整助手消息提交后、每条工具结果提交后
查看上下文，选择单步或连续执行。这里的“步”是模型/工具的语义动作，不是 Rust 源码行，也不是逐 token 暂停。

不实现任意编辑历史、撤销文件副作用、时光倒流、会话导入、断线恢复、持久化日志或交互式工具审批。
不接管 provider 协议或工具生命周期；不包含 LLM Space 前端适配。调试开启不改变静态工具权限。

## 真实消费者与模块边界

```text
CLI stdin 控制输入 → DebugController → 有界命令 channel → DebugSession
                                            cancel ───→ CancellationToken
                                                           ↓
                   Agent::run_debug（同一 run loop、transcript owner）
                       ├─ 安全边界：等待控制、复制 snapshot
                       ├─ Model::complete
                       └─ ToolRegistry::execute（等待真实清理）
                                     ↓
                      Event → CLI 原有输出桥 → stdout
```

库不读取 stdin，也不阻塞事件 callback 来等用户。暂停等待属于 Agent 的 async checkpoint；
同步 `emit` 仍只做短通知。普通 `Agent::run` 经过同一执行循环但不安装调试控制，不发调试事件。
CLI 是当前实际控制端；未来 GUI/服务可消费相同逻辑合同，不得另建可变 transcript 副本作为第二个 owner。

## 所有权与生命周期

| 状态 / 资源 | owner | 借用、转移与结束 |
|---|---|---|
| `messages`、`RunState`、模型和工具 | `Agent<M>` | `run_debug(&mut self, ...)` 独占借用；宿主不能并发改写 |
| 命令 sender、取消 token | `DebugController` | 宿主持有；不实现 Clone；drop 请求取消，不代替 await run |
| receiver、步进模式、pause 计数 | `DebugSession` | 每个 run 消费一次；不得复用于下一次 run |
| 当前暂停 epoch | Session 发布，Controller 只读共享的 `Arc<AtomicU64>` | 私有命令相关性元数据；入队时复制到 `QueuedCommand`，离开暂停由 `PauseEpoch` 清零 |
| 当前暂停快照 | Agent checkpoint / DebugSession 的运行期数据 | owned 值经 Event 交给消费者；不向外泄漏会话可变引用 |
| 模型请求 | 现有 Model future | 取消可 drop，完整响应才提交 |
| 文件 worker / shell | 现有 Tool future | 不因 Pause 或 Cancel 丢弃；先等待工具返回或清理，再到边界 |
| 控制 stdin、输出和 run futures | CLI | 一起受宿主生命周期控制；输入结束取消，run 结束停止输入并等待输出 |

没有 `Arc<Mutex<Agent>>`、调试后台 worker 或子对象反向持有 Agent。
快照复制是观察边界的值转移，不共享可变历史。控制端释放导致协作取消；直接丢弃 run future 仍是不安全恢复点。
共享原子 epoch 只有 Session 写入，Controller 无法设置它或修改 Agent 状态；这个窄小共享边界用于关联
命令提交时的暂停现场，不是另一个 transcript/RunState owner，也不作为公共状态查询接口。

## 为什么只在安全边界暂停

模型返回完整内容后，Agent 已验证并提交助手消息，但工具尚未执行，适合查看具体调用及参数。
工具返回后，Agent 已提交真实结果并完成该工具自身的清理，适合查看副作用和剩余调用。
暂停不是让任意 future 停在中间，也不能借等待用户之名消耗一个尚未开始的 HTTP/shell 超时。

`Step` 执行一个模型请求、一个工具调用或最终结束动作；`max_steps` 仍只计模型请求次数。
单独保留最终结束动作，使无工具回答和最后一批工具结果也能被观察，之后才发 `RunFinished`。
这比仅用 `--max-steps 1` 更精确：后者是本轮运行上限，不是保留现场的暂停接口。

## 控制面与观察面分离

控制命令不修改消息，只改变下一次安全边界是否停下。带 `pause_id` 的命令必须匹配当前暂停；
命令入队时还绑定 Session 当时发布的暂停 epoch；消费时同时检查 epoch 与命令 ID，
已执行、运行中预送或延迟到达的旧命令不能释放新的暂停点。
有限次排空命令只是公平调度，不是安全边界：发送者可能在排空期间补入更多命令。
Cancel 使用与 run 共享的 token，独立于命令队列，不等待队列腾位。

观察事件属于原有 best-effort Event 流，不是调试协议的可靠存储。快照包含 system、完整已提交历史、
可见工具和待执行调用，但没有模型凭据、provider 原始 HTTP 请求或持久化恢复信息。
源码和用户手动放入 prompt 的秘密仍会原样进入快照；“不采集凭据”不等于“自动脱敏”。

快照复制与序列化耗时、内存随历史增长；命令队列限项数，原有事件队列也只限项数。
本版本没有全局 transcript/快照字节预算，不适合把长历史无限反复 Inspect 后保存在外部内存中。

## LLM Space 与未来适配（未实现）

未来适配可以把 `debug_paused` 显示成工作台中的请求、工具调用或结果检查面板，
把按钮转换成带当前 pause ID 的 Step/Continue/Inspect。Rust 仍是唯一调度和状态 owner。
外部界面若编辑消息，应视为新的分叉运行需求，不得假装修改当前快照能反向改变 Agent。

远程控制还需单独设计鉴权、session/run ID、事件序号、重连与输入限额；当前进程内 channel 不提供这些保证。
MCP 包装一次 Turnforge 外层调用也不等于获得内部断点。LLM Space 适配尚未实现。

## 维护影响

新增暂停点必须明确下一动作、已提交内容、待配对调用和取消路径；不能在工具清理之前发“已暂停”。
改字段需同时检查 [协议](../protocol/design.md)、[Agent](../agent/design.md)、
[CLI](../cli/design.md)、[事件输出](../event-output/design.md) 及自动化驱动。
详细序列化、错误和证据入口见[设计](design.md)。
