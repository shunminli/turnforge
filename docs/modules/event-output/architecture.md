# 事件输出架构

状态：已实现（M0）；代码：[src/output.rs](../../../src/output.rs)、[src/main.rs](../../../src/main.rs)。
本模块仅属于 CLI binary，不是 library 的 Event 定义层。
具体 fd 和 deadline 规则见 [设计文档](design.md)，返回 [文档导航](../../README.md)。

## 职责边界

把 Agent 产生的有序暂态事件，投影成 CLI stdout 的文本或 NDJSON 字节。
在 pipe/TTY 不消费时保留可取消性，并让输出失败反馈到宿主取消路径。
不生成会话事实、不修改 Agent、不重试模型，也不负责工具进程清理。
不提供持久化、至少一次/恰好一次交付、断线续传、事件版本协商或日志脱敏。

Event 的类型与逻辑顺序由 [协议模块](../protocol/architecture.md) 定义。
本模块只能保证成功传输部分的顺序，不能保证完整流总能抵达外部消费者。
RunFinished 在库中被发出，不代表它已成功排队、写完、被读取或持久化。

## 数据通路

```text
Agent::run 同步 emit(Event)
  → CLI sink.try_send(Event) → 64 项 mpsc → forward
      │                                  ├─ NDJSON: serialize + '\n'
      │                                  └─ 文本: Text delta / 结束换行
      └─ 满/关闭 → token.cancel()             ↓
                                        Output::write_all
                                          ├─ Pollable: AsyncFd + O_NONBLOCK
                                          └─ File: 普通文件 / /dev/null
```

sink 与 `forward` 分别由 run/writer future 持有；它们通过 `join!` 并发轮询，无独立后台任务。
注册、关闭、取消的宿主级顺序见 [CLI 架构](../cli/architecture.md)。

## owner 与资源生命周期

| 对象 | 创建者 / owner | 结束方式 |
|---|---|---|
| `Receiver<Event>` | execute 创建，移入 forward | sender 全部关闭并排空，或 forward 出错后 drop |
| 当前 event / bytes | forward 的当前循环迭代 | 完成本事件写入，或遇错丢弃剩余字节 |
| stdout 的 dup File | `Output::stdout` | Output drop 关闭复制的 fd，不关闭原 stdout fd |
| 原始 fd flags | `Descriptor.flags` 保存 | Descriptor::drop 尝试恢复 `F_SETFL` |
| `AsyncFd<Descriptor>` | `Output::Pollable` | 注册随 drop 释放，Descriptor 执行恢复 |
| 当前 write future | forward pin 后交给 select / timeout | 成功、错误或超时返回后释放 |

`dup` 并不复制独立的 open-file description；设置非阻塞 flags 会影响共享该描述的 fd。
因此 Pollable 在 Drop 中恢复原始 flags，不假设只改了自己的整数 fd。
这隐含 CLI 正常运行期间没有其它 stdout writer 并发改 flags；恢复失败目前不对外报告。

## 隔离策略与设计取舍

同步 emit 不能 await 消费者，且不得阻塞模型/工具取消；因此回调只做有界 `try_send`。
队列满/关闭时采取“记录首次交付错误并取消 run”，不重试、不丢旧保新、不无限缓存。
容量只限制事件数量；一个事件可以携带较大文本、参数或完整消息，没有字节级队列预算。
序列化又会分配当前事件的字节缓冲，故 64 项不能解释成固定内存占用。

管道与终端使用 readiness 驱动的非阻塞写，不以 `spawn_blocking(stdout.write_all)` 逃避取消。
普通文件不便可移植地注册 epoll/kqueue，走同步文件写；/dev/null 也作为同步快速路径。
这一折中支持常见本地重定向，但不承诺网络文件系统、异常设备阻塞时能及时响应取消。

## 故障传播

`forward` 初始化、序列化或写入出错后返回；writer 包装层设置取消 token 并结束。
run 会继续观察 token、完成工具清理，随后宿主 join 返回并优先报告 writer 错误。
消费者已断开/停止消费时，尝试交付终态也可能失败；已输出的 NDJSON 尾行可能只有前半段。
文本模式的增量同样可能来自最终失败的模型响应，不能当作已提交且成功的答案。

不能仅靠输出事件判断宿主成功：即使 RunFinished 是 Completed，之后输出故障仍可使进程非零退出。
反过来，宿主内存中的工具消息闭合也不能修补外部已经丢失的事件流。
写入等待是每事件局部预算，不是全局 run/drain deadline，精确竞速见 [设计文档](design.md)。

## 安全与维护边界

NDJSON 包含用户输入、模型内容、工具参数与结果，可能含源码和秘密；本模块不作过滤。
改变事件字段应与协议模块协商兼容性；改变文本投影不应悄悄改写 NDJSON 合同。
增加多个输出消费者时须重新设计慢端隔离、关闭责任和失败策略，不能直接复制当前 Sender。
可靠审计日志需要独立持久化提交合同，不能将 stdout 重定向等同于 durable store。
现有集成覆盖和新增行为的检查项见 [设计文档](design.md#验证与缺口)。
