# CLI 宿主架构

状态：已实现（M0 + 原生调试/Lab/学习宿主）；实现入口：[src/main.rs](../../../src/main.rs)、[debug_input.rs](../../../src/debug_input.rs)。
本篇说明组合与资源责任；参数和退出行为见 [设计文档](design.md)。
返回 [文档导航](../../README.md)。

## 职责与非职责

CLI 是 headless 库的第一个真实消费者，每个 `run`、`debug` 或 `lab` 进程创建一个 Agent、运行一个用户 turn。
它负责参数解析、权限配置、模型和工具组装、事件交付、Ctrl-C 与退出码。
它不拥有 Agent 的会话状态机，不解析 SSE，不实现工具业务，也不直接改写 transcript。
`debug` 支持交互控制一个 turn，但没有交互多轮会话、TUI、配置文件、自动登录、持久化恢复或动态 provider 选择。
编译入口明确限定 Unix，支持目标是 macOS/Linux；不是跨平台终端抽象。
Lab 的基线/临时场景/oracle 属于[Lab 模块](../harness-lab/architecture.md)，课程属于[学习模块](../learning/architecture.md)。
CLI 只把它们组合到原有调度和输出生命周期；`learn` 不创建模型或 Agent。

## 组件关系

```text
Cli / RunArgs / HostArgs / HostMode
  └─ execute
      ├─ registry → Workspace + ToolRegistry
      ├─ OpenAiModel + AgentConfig → Agent<OpenAiModel>
      ├─ run future → Agent::run / run_debug → Event/Notice sink → mpsc(64)
      ├─ writer future ← output::forward ← Receiver<OutputItem>
      ├─ control future ← DebugInput → DebugController（debug / 交互 lab）
      └─ select(execution = join!(run, writer, control), ctrl_c)
```

`tools` 子命令在 `registry` 后直接输出可用工具定义，不创建模型或 Agent。
`run` 的 registry、prompt 输入和模型配置校验先完成，随后才建立取消与输出链路。
`lab` 先预检并创建 PreparedLab，再构造固定 RunArgs；HostMode 持有临时场景直到整个 run/验收/输出结束。
`learn` 将一份静态课程 Notice 放入容量 1 的输出 channel 后关闭 sender，通过同一个 writer 交付。

| 资源 | owner / transfer | 结束责任 |
|---|---|---|
| 工作区根与能力授权 | `registry` 构造；工具持有各自 Workspace 克隆 | 工具随 Registry/Agent 释放 |
| 模型、工具集合、历史 | 移入 `Agent`；宿主持有 Agent 并给 run 可变借用 | `execute` 等待 run 结束后释放 |
| 取消 token | `execute` 创建；run、writer、signal 路径共享借用 | 请求取消不等于提前 drop run |
| OutputItem Sender | run 回调/最终验收通知与 control 的帮助/课程通知各持一份 | run 结束关闭自己的 Sender；control 被 run_done 停止并关闭最后一份 |
| OutputItem Receiver / stdout | Receiver 移入 writer；stdout 由 `output::forward` 创建 | 全部发送者关闭后排空；writer 返回时释放 |
| 调试 controller / session | execute 持有 controller；session 移入 run_debug | controller 生命周期覆盖 run；正常结束后释放 |
| 调试 stdin fd / read buffer | execute 持有 `DebugInput`，control 借用 | control 可停止；writer 结束后 input drop 恢复 flags |
| `run_done` token | execute 创建；run 返回时设置 | 停止仍保持开放的调试 stdin，不取消已完成结果 |
| execution future | signal select 持有、pin | 正常或取消都等待 run、writer、control |
| observed_pause | execute 的 AtomicU64；sink 从 DebugPaused/Resumed/Finished 投影 | input 只读用于快捷键，不是库的暂停授权 owner |

run、writer 与 control 是 `tokio::join!` 并发轮询的 future，不是各自 `spawn` 的后台任务。
宿主没有 detached signal task，也没有阻塞 stdout worker；工具内部 worker 的清理由工具负责。

## 调试输入 owner

`DebugInput` 复制 stdin fd，只接受可 poll 的终端、FIFO、socket；`/dev/null` 表示立即 EOF。
普通文件重定向明确拒绝，而不是启动不可取消的阻塞读取线程。输入使用 AsyncFd + O_NONBLOCK；
固定读取 buffer 和单行限额归这个宿主输入模块，不泄漏到 headless library。

input 在 writer 初始化 stdout 前创建，并活到 writer 结束后：stdin/stdout 可能共享同一个终端
open-file description，恢复非阻塞 flags 的顺序必须与设置相反。与输出一样，强杀和恢复失败不是事务保证。
run 结束设置 `run_done`，control 停止等待下一行；EOF、语法/读取错误或队列错误取消 run 并等待清理。
调试 Pause 是库安全点的操作，不通过让控制输入线程持有 Agent 锁来冻结运行。

Lab 的快捷键由同一个 DebugInput 解析；一个 buffer 内的行绑定 read 时观察的 pause ID，跨 read 半行绑定首字节的 ID。
不能等解析下一行时才换成新 pause ID。库仍验证命令 ID 与入队 epoch，宿主 observed_pause 只是 UI 观察投影。
自动 Lab 不创建/读取 stdin；sink 收到真实 DebugPaused 时提交一次 Step，不通过快捷键输入预送未来命令。

## 取消链路与完成边界

Ctrl-C 分支先调用 `cancel.cancel()`，再 `execution.await`，保留工具清理和消息闭合机会。
输出队列故障由事件 sink 发出取消；writer 自身失败也发出取消。
Agent 负责处理已提交 tool call 的终态；CLI 不能以 `abort` 代替工具清理。
这只约束正常协作取消，不保证宿主被 SIGKILL、panic 或运行 future 被外部丢弃时恢复。

`execution` 与 `ctrl_c()` 的外层 select 未设 `biased`；两者同时就绪不承诺信号优先。
信号到达时 Agent 可能已完成而 writer 仍在排空，取消 token 不会改写已返回的 RunOutcome。
因此“收到 Ctrl-C”不能单独推导退出码；实际结果还受输出失败优先级影响。

## 为什么宿主采用有界事件桥接

Agent 的事件回调是同步 `FnMut`，不能在回调中 await 慢消费者。
宿主用 `try_send` 保持回调短小；队列容量是 64 个 OutputItem（Event/Notice），不是 64 KiB 或总内存限额。
首次发送失败即记住错误并取消；之后不再尝试发送任何事件，允许 run 完成内存内清理。
这是一种失败并取消策略，不是可靠事件总线、丢弃旧事件或无限等待的背压协议。

队列隔离生产与输出，但无法保证终态抵达已堵塞/断开的消费者。
输出形式、Unix fd 生命周期和每事件写入超时详见 [事件输出架构](../event-output/architecture.md)。

## 权限组合边界

`registry` 始终注册 5 个文件操作和 `bash`，由 Registry 在展示与执行两处过滤授权。
默认模型可见的只有 `list_files`、`read_file`；不是只把读工具对象放进 Registry。
`allow_write` 与 `allow_shell` 独立；shell 能以宿主权限读写工作区外资源。
工作区路径限制属于文件工具，system prompt 中的工作区说明不能替代权限校验。
相关约束见 [工具运行时架构](../tool-runtime/architecture.md)。

## 后续扩展约束

- 新宿主应复用 Agent/Model/Tool，不把 CLI 参数类型引入 library。
- 多轮会话应明确保留 Agent 的 owner，而不是每轮重建后假称拥有上下文。
- 新增 provider 应在组合层选择适配器；协议差异留在 [模型模块](../model/architecture.md)。
- 交互审批需要独立的请求/应答生命周期；当前静态能力开关不能冒充审批队列。
- 可持续输出或恢复必须先设计交付合同；当前 [协议](../protocol/architecture.md) 只是暂态通知。

## 维护检查

修改宿主生命周期时，同时检查 `execute` 中的 Sender 关闭、writer 返回、取消传播和 join。
调试路径还要检查 `run_done`、输入取消、Controller Drop 及共享 fd flags 的恢复顺序。
修改配置时同步 `--help`、README 和本模块设计表，不增加隐式凭据来源。
行为证据与目前缺口见 [设计文档的验证部分](design.md#验证与缺口)。
