# 文件系统工具：设计

职责与生命周期见[架构](architecture.md)，导航见[文档索引](../../README.md)。
以下字段来自 [files.rs](../../../src/tools/files.rs)，外层结果见[协议设计](../protocol/design.md)。

## API 与输入输出

公开入口是 `Workspace::new(path) -> io::Result<Workspace>`、`Workspace::root() -> &Path`、
`FileTool::new(Workspace, FileOperation)` 及 `Tool::execute(Value, &CancellationToken)`。
`Workspace::resolve`、`execute_sync`、输入结构和读写 helper 均为内部实现。
所有输入结构都有 `#[serde(deny_unknown_fields)]`，缺少必填字段、类型错误和未知字段都拒绝。

| 工具 | 参数 | `ToolOutput::Ok.data` |
|---|---|---|
| `read_file` | `path`；`start_line=1`；`max_bytes=32768` | `text`、`truncated`、`start_line`、`total_lines` |
| `list_files` | `path`，根目录用 `"."` | `entries: [{name, kind}]`、`truncated` |
| `write_file` | `path`、`content` | `path`、`bytes`（UTF-8 字节数） |
| `str_replace` | `path`、`old`、`new` | `path`、`replacements: 1`、`changed` |
| `mkdir` | `path` | `path` |

目录项 `kind` 为 `symlink`、`directory`、`file` 或 `special`；名称使用有损 UTF-8 转换。
`read_file` 的 start_line 从 1 计数；空文件仅允许从 1 开始，返回空文本和 `total_lines=0`。

## 路径与操作顺序

`resolve` 从规范根目录出发，遍历 `Path::components`：忽略 `CurDir`，接受 Normal，拒绝其余类型。
每加入一个 Normal 分量便检查 `symlink_metadata`；现存 symlink 拒绝，不存在的分量允许继续。
空字符串拒绝，绝对路径和 `..` 拒绝。检查并不保证后续打开时路径未被其他进程替换。

Read：检查起始行和输出上限 → resolve → 只读取普通文件且最多 4 MiB + 1 字节 → 验证 UTF-8
→ 计算 `lines().count()` → 跳过前面的行 → 按字节预算截断，向前退到字符边界。
它读取完整受限文件后选择输出，不是按字节或行 seek；`max_bytes` 仅限制返回文本。

List：resolve → `read_dir` → 每项检查取消 → 收集最多 1000 项 → 对收集结果按名称排序。
超过上限时只对已收集子集排序；文件系统遍历顺序未固定，截断子集不保证跨运行一致。
列出 symlink 自身是允许的，不会递归进入；目录项名称长度未另设字节上限。

Write / Replace 共用 `atomic_write`：检查内容 <= 4 MiB → 读取已有普通文件权限
→ 在父目录创建临时文件 → 设置已有权限 → write_all → sync_all → 检查取消 → persist 到目标。
父目录必须已存在；不是自动 mkdir。新文件权限依赖临时文件创建行为，不额外模拟原地创建规则。
Replace 先要求 `old` 非空、在原文本中恰好出现一次，再用 `replacen(..., 1)` 生成完整新内容。
Mkdir 在 resolve 后再次检查取消，再执行 `create_dir_all`；不提供多目录回滚。

## 限额、取消与错误矩阵

| 条件/位置 | 结果或保证 |
|---|---|
| 任一操作 worker 入口已取消 | `cancelled`，不开始文件操作 |
| List 遍历中取消 | 在下一次检查时返回 `cancelled`，不返回部分列表 |
| 临时文件写完、persist 前取消 | `cancelled`，尚未替换目标；临时文件随返回清理 |
| persist 后或 mkdir 执行中取消 | 不虚构回滚；返回操作实际完成结果 |
| 正在同步读取/写入/文件系统调用中 | 不可通过 token 抢占；调用方继续等待 worker |
| 路径为空/绝对/父级/symlink，读取或覆盖非普通文件 | `invalid_path` |
| 参数解析失败、读范围非法、`old` 为空 | `invalid_arguments` |
| `old` 出现零次或多次 | `ambiguous_match`；未写目标 |
| 读取内容或待写入内容超过 4 MiB | `file_too_large` |
| 文件不是有效 UTF-8 | `invalid_encoding` |
| 文件系统错误 | `io_error`，携带底层错误文本 |
| blocking worker 异常结束 | `internal_error`，固定消息 `File worker failed` |

`max_bytes` 必须为 1..=32768，start_line 不能为 0 或超过 `total_lines.max(1)`。
没有文件操作通用 deadline；CLI 的 tool timeout 仅作用于 ShellTool。
Read 权限始终允许；Write/Replace/Mkdir 由 Registry 的 Write 能力授权，直接调用工具不做授权。

## 关键不变量与明确限制

- 未成功解析输入、检查路径或确认唯一匹配之前，不修改目标文件。
- 每次 execute 等待自己的 blocking worker；取消是协作检查，不是事务撤销。
- 返回的读取文本保持 UTF-8 边界；读取上限不表示支持二进制。
- 文件写入以单次 persist 提交；没有多文件事务、并发修改冲突检测或断电目录持久性保证。
- symlink 检查不是 race-free capability；硬链接、权限变化、外部编辑仍在信任边界之外。

## 现有测试与覆盖缺口

[tests/tools.rs](../../../tests/tools.rs) 中：

- `write_read_replace_and_mkdir_operate_in_workspace`：真实目录/写读/替换/列表。
- `ambiguous_replacement_and_invalid_arguments_do_not_mutate`：多匹配、未知参数、无效行号。
- `path_escape_absolute_paths_and_symlinks_are_rejected`：父级、绝对路径、Unix symlink。
- `read_limits_preserve_utf8_and_reject_binary_or_large_files`：字符边界、二进制、大文件。
- `cancelled_write_has_no_side_effects`：入口即取消，不证明写入中间的所有时序。

[tests/cli_http.rs](../../../tests/cli_http.rs) 的 `fragmented_tool_arguments_round_trip_to_real_file_and_model`
覆盖流式参数拼装 → 真实文件写入 → 下一次模型请求中的工具结果。
未覆盖的主要边界：persist 前后取消竞争、IO 故障注入、权限/元数据保留、目录截断、空文件、
硬链接/TOCTOU、并发编辑丢失更新，以及恶意或挂起的网络文件系统。

## 变更检查清单

- [ ] 输入 schema、serde 字段、运行时范围校验与文档是否一致？
- [ ] 新操作是否复用 resolve，并明确 symlink/非普通文件行为？
- [ ] 取消发生在提交前后时，结果是否描述真实副作用？
- [ ] 有无新增 detached worker、隐含回滚或未限定的读取/输出？
- [ ] 写入策略改变是否影响权限、原子性、并发编辑或临时文件清理？
