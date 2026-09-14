# 原生语义调试：实现计划

基线：`31ba63de6d36982ec849d5b3bcef4edb5a84e8b4`。范围是当前进程内的 Rust Agent 调试，
不是 LLM Space runtime 替换、源码行级 debugger 或可恢复持久化。用户授权本地实现与测试；不提交/推送。

## 执行合同

- 同一 Agent、同一 run future、同一工具执行路径，默认 `run` 保持原行为。
- 调试首次请求前暂停；之后完整模型响应提交、每个工具结果提交后有安全暂停点。
- Step 前进到下一个语义边界（一次模型/一次工具，或最终结束）；Continue 连续执行；
  Pause 在下一个安全边界停下；Inspect 只返回当前暂停快照；Cancel 沿原清理路径结束。
- Step/Continue/Inspect 必须带当前 pause ID；旧命令不能意外释放未来的暂停点。
- 暂停不自动授权工具、不回滚副作用，不丢弃副作用 future。模型或工具超时只在实际操作时生效。
- 快照是 owned 只读逻辑上下文（system/messages/可见工具/待执行调用/下一动作），
  不捕获凭据，不声称等同于 provider 原始 HTTP payload 或可导入的恢复 checkpoint。
- CLI `debug` 用独立 stdin 命令流、stdout 文本/NDJSON，prompt 不再从 stdin 读取；
  EOF/输入错误/Ctrl-C/输出故障都取消并等待清理，运行完成时输入读取必须可停止且无 detached task。

## Ownership 与模块边界

Agent 独占 transcript 和 RunState，只有 Agent 能决定何时到达安全点。
每个 run 消费一个 DebugSession，后者持有有界命令 receiver、步进模式和 pause 计数；
宿主持有 DebugController（sender 与取消 token），控制端释放意味着取消。
emit 仍是同步短回调，绝不在回调中等待用户；等待发生在 Agent 的 async checkpoint。
CLI 拥有输入 fd、输出 fd 和运行的生命周期；库不依赖终端。状态观察使用事件的值副本。

## 依赖有序工作项

| ID | 状态 | 动作与完成定义 | 验收 / 依赖 |
|---|---|---|---|
| T1 | done | 固定控制协议、所有权、暂停点、错误和兼容合同 | 原生 debugger 架构/设计双文档；无外部决策 blocker |
| T2 | done | library DebugSession/Controller、快照、Agent 安全点 | 已用入队暂停 epoch 修复独立验收发现的提前命令补位边界；依赖 T1 |
| T3 | done | CLI debug 宿主和可取消命令输入 | 真实进程交互、EOF/SIGINT/输出故障能退出；依赖 T1/T2 API |
| T4 | done | 确定性与本地模型集成测试 | 已补未来 ID 持续补位反例，先红后绿；依赖 T2/T3 |
| T5 | done | 更新直接受影响的模块文档、使用指南及索引 | 已校准入队暂停关联合同；依赖 T2/T3/T4 |
| T6 | done | 冻结实现，独立验证与本地 LLM 复验 | 修复版独立 verified，44 debug/44 release，真实模型各4项通过；依赖 T2–T5 |

T2/T3/T4 可以在共享协议固定后并行；T5 同步整理，最终以实际代码与测试为准。
T6 前冻结源码，不在独立验证中修改待验目标。失败返回对应实现项，保留首次失败证据。
已完成结果与两版指纹见[本轮验证记录](verification/debugging-2026-09-14.md)。首版独立审查发现
提前命令补位问题后返回 T2/T4，修复并重新冻结复验；没有忽略首次失败或将旧通过记录当作新版本证据。

## 仓库知识变更范围

新增 debugger 架构/设计承载状态、控制协议和生命周期 Specification；调试指南承载用户可见能力。
Agent、protocol、CLI、event-output 的直接合同因新增暂停状态/事件和输入宿主而更新；README、
文档索引与总架构补充模块导航。本地 LLM 指南补调试 case 和执行方法。
现有 AGENTS 治理仍准确，不修改；不新建平行 memory 目录或仅记录任务经过的 changelog。
历史验证记录保持原样，本轮版本证据使用独立的新验证记录。

## 后续阶段（本轮不实现）

- LLM Space / GUI 薄适配：转换观察事件和控制命令，不搬移 Rust 状态 owner。
- Provider 原始请求的安全导出、统一 run/sequence ID、版本化远程协议与鉴权。
- 历史编辑、分叉、持久化恢复：需额外定义副作用重放/幂等、日志提交和迁移合同。
- 条件断点、Rust 源码行级调试、并行工具调度。
