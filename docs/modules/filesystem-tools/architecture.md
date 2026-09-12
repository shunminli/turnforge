# 文件系统工具：架构

本模块提供工作区内五种文件操作；详见[设计文档](design.md)与[文档索引](../../README.md)。
实现位置：[src/tools/files.rs](../../../src/tools/files.rs)。

## 职责与边界

`Workspace` 保存规范化根目录并检查相对路径，`FileTool` 把五种操作适配为统一 `Tool`。
本模块负责输入解析、读取/输出限额、常见路径越界拒绝，以及单文件替换的提交过程。
不负责模型调用、会话历史、授权决策、文件搜索、补丁格式、版本管理或操作系统隔离。

| `FileOperation` | 工具名 | 能力 | 主要副作用 |
|---|---|---|---|
| `Read` | `read_file` | Read | 读取一个 UTF-8 普通文件 |
| `List` | `list_files` | Read | 读取一层目录项 |
| `Write` | `write_file` | Write | 创建或覆盖一个文件 |
| `Replace` | `str_replace` | Write | 唯一文本匹配后的整文件替换 |
| `Mkdir` | `mkdir` | Write | 递归创建目录 |

能力由 [Registry](../tool-runtime/architecture.md)检查。直接调用 `FileTool::execute` 不经过授权层，
因此嵌入式宿主不能把 FileTool 本身当成审批接口。

## 上下游与主要路径

```text
CLI → Workspace::new(root) → FileTool::new(workspace.clone(), operation)
                                  ↓ Registry 拥有 FileTool
Agent → Registry → FileTool::execute(arguments, token)
                    → spawn_blocking → execute_sync
                        → parse → resolve → 文件操作
                    ← await worker ← Value / ToolOutput
Agent ← ToolOutput → 提交 Message::Tool
```

输入结构通过 serde 严格解析，JSON Schema 由相同结构通过 schemars 生成。
schema 之外的范围约束仍在运行时检查；模型知道 schema 不代表输入可信。
[协议模块](../protocol/architecture.md)拥有 `ToolOutput` 的外层结构，本模块拥有 `data` 的操作特定字段。

## 所有权与资源生命周期

`Workspace` 拥有一个 `PathBuf`，不是目录文件描述符，也不锁定整棵目录树。
`Workspace::new` 对根目录 canonicalize 并确认其为目录；克隆只复制路径值。
`FileTool` 独占一个 Workspace 值和固定的 `FileOperation`；实例之间不共享可变缓存。

`execute` 克隆 FileTool 和 CancellationToken，将二者与 owned JSON 参数移动进 blocking worker。
worker 的所有者是这次 execute future；正常调用必须 await `spawn_blocking` 的 JoinHandle。
取消 token 不会强制停止阻塞文件系统调用，也不会让调用方提前声称写入已回滚。
若宿主丢弃外层 future，阻塞任务可能继续运行；因此遵守 Agent 的“取消后继续等待”合同很重要。

读文件时普通 `File` 由 `read_text` 局部拥有并在返回时关闭。
写文件时 `atomic_write` 拥有同目录 `NamedTempFile`：写入、sync、检查取消后 persist 到目标路径。
persist 前失败由临时文件的生命周期清理；persist 成功即代表目标已替换，不再虚构取消回滚。

## 为什么这样组织

五种工具共享路径解析与结果结构，通过固定枚举区分操作，避免每个工具重复一套工作区规则。
同步文件操作放到 blocking pool，避免直接占住 async 执行线程，但不是可抢占 IO。
读写数据都设上限；目录只做一层，限制单次结果规模，不限制整个会话的内存或模型上下文总量。
替换要求恰好一次匹配，拒绝“猜一个位置”修改，但没有文件版本或 compare-and-swap 检查。

写入使用目标同目录临时文件，避免原地截断后才发现写入失败。
这里只提供单文件替换原子性；不提供多文件事务，也没有父目录 fsync 的断电持久性承诺。
保留已有文件的 `permissions()`，不承诺 owner、ACL、扩展属性或硬链接关系保持不变。

## 安全与维护影响

`resolve` 拒绝绝对路径、`..` 和现存路径分量中的 symlink，允许 `.`。
根目录自身经 canonicalize 解析；这与禁止根目录下路径的 symlink 穿越不是同一规则。
路径检查与实际打开不是原子操作，存在 TOCTOU 边界；硬链接也没有专门拒绝策略。
因此只适合可信本地工作区，不是针对恶意并发修改目录的隔离机制。
shell 使用同一个根目录作为初始 cwd，但不经过 `resolve`，不受这些路径规则约束。

新增文件操作应继续共用路径检查，明确读取大小、提交点、取消检查和资源清理。
若需要并发编辑安全，需要版本/冲突合同；当前 Agent 串行工具执行不能阻止外部进程修改文件。
若需要真正隔离，应引入基于执行后端/目录句柄的设计，而非在字符串路径检查上宣称 sandbox。

## 验证入口

- [tests/tools.rs](../../../tests/tools.rs)：读写替换、路径拒绝、UTF-8/大小限额和预取消写入。
- [tests/cli_http.rs](../../../tests/cli_http.rs)：独立 TCP fixture 驱动真实 CLI 写入、回传模型结果。
- [tests/agent.rs](../../../tests/agent.rs)：顺序和取消后的结果闭合，不直接覆盖文件系统竞争。
- [Shell 架构](../shell-tool/architecture.md)：同工作区路径与宿主执行权限的区别。
