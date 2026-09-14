# Harness 学习内容：设计

实现：[src/learning.rs](../../../src/learning.rs)。配套：[架构](architecture.md) · [指南](../../harness-learning.md)。

## 接口

| binary 内接口 | 合同 |
|---|---|
| `Lesson` | `Loop / Context / Tools / Control / Io / Regression`，Clap ValueEnum，CLI 小写取值 |
| `catalog(Option<Lesson>) -> String` | None 返回六阶段路线；Some 返回该课目标、前置、源码、命令、观察练习和人工验收 rubric |
| `hint(Lesson, &DebugSnapshot) -> String` | 借用当前快照生成中文原理说明、源码和下一观察问题；不保存借用 |

`turnforge learn [LESSON]` 不依赖模型、工作区或配置；Lab 的 `--lesson LESSON` 只影响文本观察投影。
交互 `l/learn` 返回当前所选课的 catalog，未选课返回总路线；不发 Step、Inspect 或其它 DebugCommand。
输出通过原有 CLI 受限 writer，不在学习函数内部 `print` 或 `eprintln`。

## 内容与事实约束

总路线标记为 `Harness 学习路线 / learn`；单课为 `[learn ID]`；现场提示为 `[learn ID @pause N]`。
这些是人类可读文本，不是新增稳定机器协议；机器调试仍使用原始 debug 的 Event/NDJSON。

`hint` 从 DebugPoint 解释“首次请求未开始 / Assistant 已提交 / 工具结果已提交”，从 DebugAction 解释
下一模型、下一工具或结束动作；上下文数量仅计算 snapshot 的消息、可见工具和 pending calls。
对 Final snapshot 不说“已通过”：结束动作尚未释放，且最终 Lab oracle 还没有运行。
工具提交不保证工具成功；对应课程要求查看 ToolOutput 与真实文件，不把 AfterTool 自动描述为成功。
课程使用相对源码路径和符号；不说快照包含原始 HTTP payload、凭据或可确定性重放状态。

## 无状态与错误

有效 Lesson 由 Clap 枚举解析；非法课程名是参数错误，不调用 Ollama。
两个函数不返回业务 Result、不做 IO；输出失败、输入 EOF、取消和运行结束责任均留给 CLI。
没有课程进度文件、隐藏环境配置、事件订阅线程、模型调用或测试专用执行分支。

## 验证与维护

真实 CLI 检查位于 [tests/lab_cli.rs](../../../tests/lab_cli.rs)：
`learn_catalog_and_lessons_need_no_llm` 在无本机服务、无模型配置时检查总路线和六课入口；
`lab_default_read_shortcuts_pause_inspect_and_finish_with_stdin_open` 检查 Lab 的课程显示与 `l` 不推进模型或工具。
这些是内容/执行隔离证据，不是用户掌握程度的测量。
完整课程事实还需人工将当前源码符号、暂停含义、实验命令与 rubric 逐项对照。
未来内容修改不应同时改变 Lab prompt 或 oracle 来迎合示例；执行语义变化先修改真正 owner。
