# 03 · 工具、权限与真实副作用

[学习总入口](../harness-learning.md) · 上一章：[02 上下文](02-context.md) · 下一章：[04 调试控制](04-control.md)

本章目标：用第二个终端验证文件什么时候真的出现，并区分模型提议、授权、执行、结果提交与取消。
前置：完成 [00 环境准备](00-setup.md)至第 02 章；主终端位于仓库根目录，本地模型可用。
预计 40–55 分钟。实验仅操作 Lab 的合成临时目录，不使用真实业务文件，不授权 shell 工具。

## 1. 启动写文件场景，但先不执行

主终端执行：

```bash
cargo run --quiet --locked -- learn tools
bash scripts/harness-lab.sh --case write --lesson tools
```

在首次暂停输入 `i`，查看 `tools`：write 场景可见文件读写工具，不包括 shell。
这不是课程赋予的能力；是 `--case write` 选择了固定的写权限，`--lesson tools` 只加解释。
在启动 banner 找到“临时工作区：”后的完整路径，**只复制路径**。
每次运行路径不同，不要使用别人的截图、上一轮路径或自己猜一个 `/tmp/turnforge-lab-*`。

## 2. 第二个终端确认“目标尚不存在”

保留主终端暂停。在第二个终端执行下面两行；运行第一行后粘贴刚复制的完整路径并回车：

```bash
read -r turnforge_lab_dir
test -d "$turnforge_lab_dir" && test ! -e "$turnforge_lab_dir/result.txt" && printf '%s\n' '目标尚不存在'
```

应显示“目标尚不存在”。没有输出则先检查路径复制、目录是否仍存在，以及主终端是否已经结束。
这两条命令只检查现场，不创建文件。后面的磁盘检查继续使用同一第二终端和变量。
不要通过手工写 `result.txt` 帮模型通过验收，否则改变了本课要观察的因果。

## 3. 在工具预览停住，核对模型提议

回到主终端，输入一次 `n` 请求模型；等下一次暂停后输入 `i`。
沿实际 `next` 单步，直到看到准备执行 `write_file`，且参数是：

```json
{"path":"result.txt","content":"turnforge-lab-write-ok"}
```

路径也可能写成 `./result.txt`，按等价工作区路径理解；内容必须没有最后的换行。
若模型先调用读/列目录，按实际 `next` 观察，不固定假设“第 2 次暂停就是 write”。
若参数错误、提出其他写操作，先保留快照并输入 `q` 结束；不要为了走完教程去手工修目标文件。
若模型直接 Finish 没有写入，放行后应由 Lab 判 FAIL，不算本章写成功。

此时仍不要释放 write。第二终端重新执行：

```bash
test -d "$turnforge_lab_dir" && test ! -e "$turnforge_lab_dir/result.txt" && printf '%s\n' '工具提议尚未造成写入'
```

应显示“工具提议尚未造成写入”。这次观察把 Assistant 中的调用与磁盘事实明确分开了。

## 4. 只执行 write 一步，马上读真实磁盘

主终端输入一次 `n`，等 `write_file` 的 `[result]` 和 `AfterTool` 暂停，输入 `i`。
确认新增 Tool 的 `call_id` 与该 write 调用相同，`output.status = ok`，`data.bytes = 22`。
**保持这个暂停，不要再按 n/c。** 下一步可能会请求模型并最终结束，结束后临时目录会被清理。

现在第二终端执行：

```bash
wc -c < "$turnforge_lab_dir/result.txt"
od -An -tx1 "$turnforge_lab_dir/result.txt"
```

第一条应为 `22`。第二条列出 UTF-8 字节，末字节应为 `6b`（`k`），而不是换行 `0a`。
用下面的独立比较再确认内容完全一致：

```bash
printf '%s' 'turnforge-lab-write-ok' | cmp - "$turnforge_lab_dir/result.txt" && printf '%s\n' '磁盘内容精确匹配，且无末尾换行'
```

这里只向比较程序的 stdin 输出预期值，不改磁盘。应显示精确匹配提示。
若 Tool 返回 error 或磁盘不一致，记录结果/路径/字节，不把 `AfterTool` 解释成成功，也不要继续改变现场。
检查完回到主终端，按实际 `next` 到最终 Finish 并释放，观察 PASS 和 `echo $?` 是否为 0。
正常退出后目录消失是 Lab 的资源清理，**不是工具写入被事务回滚**。

## 5. 另开一轮，在写入前取消

用新的运行验证不同的决策；不要复用刚才已经写成功的现场：

```bash
bash scripts/harness-lab.sh --case write --lesson tools
```

按实际 `next` 走到 `write_file` 预览，但不放行 write，输入 `q` 并回车。
应取消并回到 shell，立即执行 `echo $?`，正常取消码为 `130`，不是 PASS。
若模型没有产生工具调用，记为“未获得取消预览现场”，不要声称本次证明了工具前取消。

本轮结束时 TempDir 也会清理，所以“找不到目录”不能证明工具从没执行。
可靠证据要来自调度计数与结果配对测试；下一步用可控模型直接观察这些事实。
同理，在工具已成功后取消，不承诺撤销磁盘修改；取消要求停止后续动作并闭合结果。

## 6. 跑现有测试，拆开两层保证

先验证调试预览取消时工具实现根本没被调用：

```bash
cargo test --locked --test debugging cancelling_at_the_tool_preview_closes_calls_without_executing_them -- --exact --nocapture
```

应精确运行并通过 1 条测试。打开 [tests/debugging.rs](../../tests/debugging.rs) 同名函数：
fixture 提议两个工具；控制端在模型后取消。断言实际调用计数为 0、两条 Tool 结果都是 `cancelled`、没有 ToolStarted、仅一个 RunFinished。
计数器不依赖 Lab 临时目录删除，因此能直接证明“未执行”而不是“执行后看不见了”。

再验证具体文件工具入口已取消时不写文件：

```bash
cargo test --locked --test tools cancelled_write_has_no_side_effects -- --exact --nocapture
```

也应是 `running 1 test`、`1 passed`。打开 [tests/tools.rs](../../tests/tools.rs)：它在调用前取消 token，
断言结果 code 为 `cancelled`，并在 TempDir 仍存活时检查目标文件不存在。
它不证明文件写入中所有竞争时序，也不是 persist 后自动回滚。任何 `0 tests` 都应先纠正命令再判断。

## 7. 沿源码把“提议”接到文件提交点

依次阅读，不跳过每一层为什么存在：

1. [src/tools/mod.rs](../../src/tools/mod.rs) 的 `register`：Registry 接管工具实例，缓存定义；重复名称拒绝，不静默覆盖。
2. `definitions` 与 `Permissions::allows`：模型只能看到授权定义；隐藏不等于执行端可以省去权限检查。
3. `execute`：依次检查取消、工具存在、能力授权，才把 owned 参数交给具体实现。它不替工具验证 JSON Schema，也不提交历史。
4. [src/tools/files.rs](../../src/tools/files.rs) 的 `FileTool::execute` → `execute_sync`：严格解析参数、检查路径，再执行具体操作。
5. `atomic_write`：同目录临时文件写入并 sync，检查取消后 `persist` 到目标；目标替换是重要提交点。
6. [src/agent.rs](../../src/agent.rs) 的工具循环：等待实际结果，再关联 ID 提交 Tool；不能拿 ToolStarted 当授权或磁盘成功证据。

Rust 里的 `Box<dyn Tool>` 让 Registry 拥有不同具体类型的工具；`Send + Sync` 是跨线程使用要求，不是安全审计或无副作用声明。
文件操作把 cloned FileTool/token 与 owned 参数 `move` 进 `spawn_blocking`，并 await JoinHandle；这样同步 IO 不直接占据 async 线程。
取消 token 不会强制终止阻塞文件系统调用，所以 owner 必须等 worker 返回事实与清理，不能丢 future 后声称“取消成功且没写”。
`NamedTempFile` 在 persist 前失败会随 Drop 清理；persist 成功后已替换目标，RAII 也不会神奇地撤销它。

`Workspace::resolve` 拒绝绝对路径、`..`、现存 symlink 分量，但检查与实际打开并非原子操作。
这叫保守的工作区路径检查，不是 OS sandbox；恶意并发目录替换、硬链接等仍有边界。
本课不打开 shell 授权；shell 的能力不能被文件路径规则约束。详见[工具运行时设计](../modules/tool-runtime/design.md)和[文件设计](../modules/filesystem-tools/design.md)。

## 自检与本章产出

回答：为什么隐藏工具还要执行端授权？AfterTool 是否必然成功？退出后目录不见证明了什么？文件提交后取消会怎样？

<details>
<summary>展开参考答案</summary>

模型可以自己构造隐藏名称，所以 Registry 仍要拒绝未授权执行。
AfterTool 只说明实现返回、结果已提交，需检查 ToolOutput 和磁盘。
Lab 退出清理只证明其临时目录生命周期结束，不能倒推出从未写入。
提交后的副作用是真实事实，取消不回滚；Agent 仍需结束活动操作、给未执行调用补取消结果。

</details>

完成标准：留下同一次运行的工具前“文件不存在”、工具后“22 字节精确匹配”记录，以及两条取消测试各自证明/不证明的内容。
下一章：[04 调试控制与协作取消](04-control.md)。
