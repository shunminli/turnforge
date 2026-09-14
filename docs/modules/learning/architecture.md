# Harness 学习内容：架构

实现：[src/learning.rs](../../../src/learning.rs)。配套：[设计](design.md) · [学习指南](../../harness-learning.md)。

## 独立职责

这是 CLI-only 的静态课程和现场解释模块。它将固定学习目标与当前 DebugSnapshot 的事实投影成文本，
被 `turnforge learn` 和 `lab --lesson` 两个真实入口消费；不是调度器、模型 prompt 或另一个 Agent。
课程与场景验收分离：教程说明人应该观察什么，Lab oracle 判定本次合成任务是否满足预期。

不请求 LLM、读取源码/文件、写进度、输出 stdout、启动任务或持有 Agent。课程不修改执行、权限或验收。
不声称理解用户已学会什么，也不将点击下一步当作完成学习的证明。

## 所有权与调用方向

```text
learn 的 lesson / LabArgs.lesson
        ├─ catalog → owned String → CLI 有界输出
        └─ DebugSnapshot 只读借用 → hint → owned String → Lab 文本投影
```

`Lesson` 是 Copy 的六值选择，不是可变学习状态；`catalog` 读取静态课件，`hint` 只在调用期间借用快照。
输出 String 的生命周期转移给调用宿主；没有资源 close、drop、cancel 或 join 责任，也不需要共享锁。
快照由 Agent/Debugger 创建，hint 不保存它，不产生第二份可变历史或模型请求。

## 为什么保持静态、只读

目标是学习真实 Harness 的边界；动态模型讲解会增加模型依赖、不可验证事实和额外调度，削弱观察可重复性。
固定课程附真实源码符号，现场提示仅解释 point/next/messages/tools/pending_calls 的逻辑含义。
课件不以硬编码行号锚定代码，也不推断未发生动作的成败。

## 维护影响与限制

调度、协议、工具、CLI/output 或 Lab oracle 改变时，相关课程源码入口和 rubric 需要同步人工检查。
课程没有自动跟踪源码变化的机制；测试只能覆盖内容入口与无执行副作用，不能证明教学效果或所有解释永久正确。
新增学习进度、评分或远程 UI 将形成新状态/隐私/持久化职责，不能暗中加到这两个纯文本函数中。
