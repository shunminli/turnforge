# 工具运行时：架构

本文说明现有 `ToolRegistry` 的边界，不代表已有插件系统、交互审批或操作系统沙箱。
接口细节见[设计文档](design.md)，全局入口见[文档索引](../../README.md)。

## 职责与非职责

工具运行时把宿主注册的可信 Rust 实现映射为模型可见的工具定义，并在执行时检查能力授权。
它统一返回 `ToolOutput`，让未知工具、拒绝授权和实际执行失败都能进入会话历史。

| 本模块负责 | 其他 owner 负责 |
|---|---|
| 名称唯一性、定义快照、能力过滤和执行分派 | CLI 选择工作区、授权和具体工具 |
| 执行前检查取消 | 工具观察运行中的取消并清理副作用 |
| 保存工具实现的所有权 | Agent 校验模型调用、决定顺序、提交结果 |
| 暴露 `Tool` 扩展接口 | 工具解析输入、限制资源；执行后端提供真正隔离 |

Registry 不验证 JSON Schema，不维护调用队列，不发事件，也没有通用超时、重试或回滚。
`Read`、`Write`、`Shell` 是宿主授予的类别，不是通过程序分析得到的行为证明。

## 上下游与主要路径

```text
CLI registry(HostArgs)
  → ToolRegistry::new(Permissions)
  → register(FileTool / ShellTool)
  → move 到 Agent<M>
      ├─ definitions() → ModelRequest.tools → 模型
      └─ execute(ToolCall, token) → Tool::execute → ToolOutput
                                                   ↓
                                           Agent 提交 Message::Tool
```

[CLI](../cli/architecture.md)会注册全部六个工具；未授权工具仍保存在 Registry 中，只是不展示。
默认模型只看到 `list_files`、`read_file`。即使模型自行构造隐藏工具调用，执行检查仍会拒绝。
`definitions()` 按 `BTreeMap` 的名称顺序输出；它不是注册顺序。
[Agent](../agent/architecture.md)在一次 run 的循环开始时获取定义，并按模型调用顺序串行执行。

## 所有权与生命周期

`ToolRegistry` 独占 `BTreeMap<String, Entry>` 和构造时的 `Permissions`。
每个私有 `Entry` 独占注册时取得的 `ToolDefinition` 和 `Box<dyn Tool>`。
`register` 接收 owned `impl Tool + 'static`；成功后调用方不再持有该实例。
重复名称返回 `RegistryError::Duplicate`，不会替换原有条目。

`definition()` 只在注册时读取一次；定义和能力标签此后使用快照。
`definitions()` 返回独立克隆，不给模型修改 Registry 内部元数据的能力。
`execute()` 只借用 Registry、`ToolCall` 和取消 token，把参数 JSON 克隆交给具体工具。
Registry 不保存调用中的 token，不拥有运行中的任务句柄，也不替工具做异步清理。

`Tool: Send + Sync` 支持宿主跨线程持有或调用工具，不等于工具本身无副作用。
当前 Agent 的串行执行是调度层选择；单独使用 Registry 的宿主仍能并发调用 `execute`。
若扩展工具使用内部可变状态，必须自行明确同步、重入与资源释放合同。

## 关键取舍

保留 `Tool` trait 是因为内核确实消费不同的文件和 shell 实现，且宿主需要扩展工具。
没有再包装插件管理器或共享可变服务容器；工具不反向持有 Agent，也不能直接修改 transcript。
错误采用数据结果而非 Registry 的执行异常，让模型可看到失败并选择下一步。
工具 panic 不会由 Registry 统一捕获；可信扩展仍需遵守正常返回与清理约定。

权限同时影响“模型知道什么”和“宿主允许执行什么”，两处使用同一 `Permissions::allows`。
这避免把模型不生成隐藏名称当成安全前提，但不能阻止恶意 Rust 工具谎报能力。
特别是 shell 获得宿主进程权限后也能写文件；`allow_shell=true` 并不要求 `allow_write=true`。

## 维护影响

新增工具通常只需实现 `Tool` 并在宿主注册；无需修改 Agent 的调度逻辑。
新增能力类别则需要同时审查宿主参数、定义过滤、执行检查与帮助/安全说明。
若引入每次调用审批，审批等待者的取消与终结责任必须另行定义，不能只让 `allows` 暂停。
若引入并行执行，资源冲突与结果提交顺序属于 Agent/调度设计，不能暗中藏进 Registry。

## 源码与验证入口

- [tools/mod.rs](../../../src/tools/mod.rs)：接口、注册、授权与分派。
- [agent.rs](../../../src/agent.rs)：调用校验、顺序、事件与结果提交。
- [文件工具架构](../filesystem-tools/architecture.md)、[Shell 架构](../shell-tool/architecture.md)：副作用 owner。
- [tests/tools.rs](../../../tests/tools.rs)：重名、隐藏定义、拒绝执行、未知工具。
- [tests/agent.rs](../../../tests/agent.rs)：顺序、取消后关闭待执行调用。
- [tests/cli_http.rs](../../../tests/cli_http.rs)：真实 CLI 的能力过滤和拒绝结果回传。
