# 第 7 章：综合练习——自己写一个权限测试

[课程目录](../harness-learning.md) · 上一章：[回归与验收](06-regression.md)

前置：第 1–3、6 章。你已经会观察，现在把“模型只能提议，宿主才有执行权”写成一个可执行断言。
本章不增加生产功能、不需要 Ollama；只在临时目录验证文件写权限，不授予 shell。

## Step 1：先写预期，再碰实现

在自己的笔记里回答这两种输入应该发生什么：

| 宿主配置 | 工具是否已注册 | 对模型是否可见 | 直接提交 write_file 后 | 磁盘 |
|---|---|---|---|---|
| `allow_write=false` | 是 | 不可见 | `permission_denied` | 文件不存在 |
| `allow_write=true` | 是 | 可见 | `ToolOutput::Ok` | 精确包含预期内容 |

“不向模型展示工具”不是唯一防线：本练习直接构造一个 ToolCall，模拟模型仍提出隐藏工具的情况。
预期文本由测试提前定义，不从 ToolOutput 或磁盘现有内容生成。

## Step 2：在自己的学习分支准备测试文件

先检查工作区，不要覆盖自己或别人的未提交改动：

```sh
git status --short
git switch -c codex/study-permission-contract
```

若分支已存在或目录不干净，先确认已有内容再决定复用，不使用强制切换、reset 或清理命令。
用编辑器新建 `tests/learning_permissions.rs`；如果该文件已存在，先阅读，不要直接覆盖。
将以下完整代码放入文件（这是让你亲手创建的练习文件，不是仓库已经内置的新测试）：

```rust
use serde_json::json;
use turnforge::{
    CancellationToken,
    message::{ToolCall, ToolOutput},
    tools::{FileOperation, FileTool, Permissions, ToolRegistry, Workspace},
};

#[tokio::test]
async fn learning_write_permission_controls_visibility_and_effects() {
    for allow_write in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut registry = ToolRegistry::new(Permissions {
            allow_write,
            allow_shell: false,
        });
        registry
            .register(FileTool::new(workspace, FileOperation::Write))
            .unwrap();

        assert_eq!(registry.definitions().len(), usize::from(allow_write));
        let expected = "learning-owned-by-host";
        let call = ToolCall {
            id: "learning-call".into(),
            name: "write_file".into(),
            arguments: json!({"path": "result.txt", "content": expected}),
        };
        let cancel = CancellationToken::new();
        let result = registry.execute(&call, &cancel).await;
        let target = dir.path().join("result.txt");

        if allow_write {
            assert!(matches!(result, ToolOutput::Ok { .. }));
            assert_eq!(std::fs::read_to_string(&target).unwrap(), expected);
        } else {
            assert!(matches!(
                result,
                ToolOutput::Error { ref code, .. } if code == "permission_denied"
            ));
            assert!(!target.exists());
        }
    }
}
```

两个分支各有独立 TempDir，避免先写成功的残留文件污染拒绝分支；`dir` 一直存活到文件断言结束。
对模型可见性断言只适用于本例仅注册一个写工具的设置，不是说 Turnforge 默认只有一个工具。

## Step 3：运行自己的测试

在仓库根目录运行：

```sh
cargo test --locked --test learning_permissions learning_write_permission_controls_visibility_and_effects -- --exact
```

应运行 **1 个测试并通过**。如果是 `0 tests`，检查函数名和 `--exact`；如果提示没有这个 test target，
检查文件确实保存为 `tests/learning_permissions.rs`。不要把“编译成功但没执行测试”当作完成。

然后运行已有的同合同测试，比较两者覆盖范围：

```sh
cargo test --locked --test tools registry_enforces_capabilities_and_rejects_duplicates -- --exact
```

也应执行 1 个测试。原用例还覆盖重复注册、未知工具等路径；本练习聚焦权限关闭/开启和真实磁盘结果，
不代表替换原测试，也不是全量 Harness 端到端验证。

## Step 4：逐层解释这段代码为什么成立

按这个顺序读 [tools/mod.rs](../../src/tools/mod.rs)：

1. `register`：Registry 持有工具和定义，注册不等于授权。
2. `definitions`：返回按宿主权限过滤的可见定义；这是给模型的能力声明。
3. `execute`：先检查取消，再找工具，再检查实际能力权限，最后才调用工具；不能仅依赖模型自觉。
4. `FileTool::execute` 与 [files.rs](../../src/tools/files.rs) 中的写入路径：工具 owner 完成实际文件操作后才返回结果。

再解释所有权：`workspace` 被 move 进 FileTool，FileTool 被 move 进 Registry；调用时 `&call`、`&cancel`
只是临时借用。`TempDir` 则由测试持有，直到两个结果分支的磁盘断言结束才 Drop。
因此 Registry 拥有“如何调用工具”，却不拥有测试目录的存活期；两种责任不应该混成同一个对象。

## Step 5：检验你是否真的知道断言在保护什么

先预测，再仅在练习文件内临时把拒绝分支的预期错误码改成 `unknown_tool`，运行 Step 3 的第一条命令。
它应失败：工具已注册，错误来自授权，而不是查找。保留失败原因，然后用编辑器或撤销恢复原来的
`permission_denied`，再验证恢复后的测试通过。**不要修改生产代码让这个错误预期成立。**

这不是“测试失败就改 expected”：错误预期是预先声明的教学负例，用来确认测试确实执行到了相应断言。
真实回归失败必须先解释实际行为与合同的差异，不能用当前输出来重写正确答案。

## Step 6：回到一次真正的 Harness 运行

如果本地模型已准备好，再运行：

```sh
bash scripts/harness-lab.sh --case write --lesson tools
```

在 `next.kind=tool` 时按 `i`，用一句话解释：谁生成这个 call，谁持有权限，谁还没执行写入？
在工具结果后的暂停检查实际文件；最后释放 Finish，让 Lab 独立验收。
小测试只证明 Registry/FileTool 这段边界；真正的模型请求、Agent 消息闭合、暂停和宿主输出要靠本次实验及其它回归共同验证。

## Step 7：提交前自查，但不要自动发布练习

```sh
cargo fmt --all -- --check
cargo test --locked --test learning_permissions
git diff -- tests/learning_permissions.rs
git status --short
```

新文件未跟踪时 `git diff` 可能没有内容，须结合 `git status` 和编辑器查看文件。
本教程不要求 commit/push/合并；是否保留或提交练习，由你决定。不把 target、临时目录或会话日志加入 Git。

## 最终自检与交付物

请留下自己的测试和一页解释：

- 一次 read/write 如何从 User 走到 Assistant、Tool、下一轮 Model、Finish？
- 为什么 `ModelRequest` 的借用、DebugSnapshot 的复制和 Agent 的可变所有权不是同一种权力？
- 为什么 ToolOutput 成功仍需要 Lab 检查磁盘，Completed 也不等于 PASS？
- 取消发生在工具前、工具中、工具后，各有什么不同？哪些已完成副作用不会回滚？
- 某个测试通过时，你能明确说出它**没有**覆盖的层吗？

<details>
<summary>参考判定标准</summary>

能沿源码指出提交与执行位置，而不是只复述命令；能用消息配对和磁盘事实解释结果；
能区分模型输出问题、协议/调度问题和环境配置问题；新增断言有事先定义的合同，不靠模型自评。
还不能做到时，回到相应章节重做观察，而不是以六课都运行过作为“已经掌握”的证明。

</details>

完成后，下一步可以选择深入 [模型适配](../modules/model/design.md)、[工具扩展](../modules/tool-runtime/design.md)
或[调试状态机](../modules/debugger/design.md)。这些是后续主题，不是本教程已经实现的新功能。
