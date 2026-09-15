# 01 · 跟着一次运行读懂 Agent 循环

[学习总入口](../harness-learning.md) · 上一章：[00 环境准备](00-setup.md) · 下一章：[02 上下文](02-context.md)

本章目标：不背函数名，而是亲手预测并验证“下一步由谁执行、会提交什么消息、何时结束”。
前置：完成第 00 章；终端位于仓库根目录，`cargo` 可用，本地 Ollama 已按基线启动。
预计 30–45 分钟。准备一份自己的笔记；教程和 CLI 不会自动保存学习进度。

## 1. 先打开路线，再启动实验

在终端执行：

```bash
cargo run --quiet --locked -- learn loop
bash scripts/harness-lab.sh --case read --lesson loop
```

第一条只显示课程，不调用模型。第二条预检本地服务后建立独立临时工作区，停在第一次模型请求之前。
看到 `[lab paused #1]`、`BeforeModel`、下一步“请求模型（第 1 轮）”即可继续。
若预检失败，回到第 00 章排查服务/模型；不要换云端或修改锁文件来绕过基线。
若立即退出，确认是在交互终端运行，没有给 stdin 接文件或关闭输入。

此时不要连续粘贴多行 `n`。每次只输入一条命令并回车，等新的暂停提示后再操作。
预送的命令不会合法地放行未来暂停点；“多按几次”不能代替看现场。

## 2. 在第一次暂停证明“还没问模型”

输入 `i` 并回车，查看 `[lab snapshot #1]` 后的完整 JSON；输入 `l` 可再看课程说明。
两者都是观察，不应新增模型请求、消息或暂停号。
在笔记写下 `point.kind`、`next.kind`、`step`、`messages` 条数、`pending_calls` 条数。

对于这个全新 Lab Agent，预期是：

| 字段 | 此刻的事实 |
|---|---|
| `point.kind` | `before_model` |
| `next` | `{"kind":"model","step":1}` |
| `messages` | 1 条 `user`，是 Lab 构造的任务 |
| `pending_calls` | 空；还没有模型提议的工具调用 |

注意：`step = 1` 表示即将执行的模型轮次，不是模型已经请求过一次。
`system` 单独存放，不算进 `messages` 的条数。只看计数而不看角色，很容易把这两点读错。

## 3. 只放行一次模型请求

先预测：“下一条完整消息应该是 Assistant，文件工具还不会执行。”然后输入 `n` 并回车。
等待 `[model step 1]` 和下一次暂停，输入 `i`。

正常的读文件路径会停在 `AfterModel`，`next.kind = tool`，参数中可看到工具名与路径。
这里表示完整 Assistant 已校验并提交，**不是工具已经执行**。
`messages` 通常变为 2 条；若 Assistant 提议多个工具，仍只新增 1 条 Assistant，调用都在其中。

模型可能先请求 `list_files`，也可能直接 `read_file`，不能把某个顺序当作调度器承诺。
如果 `next.kind = finish`，说明本次模型没有请求工具；继续到结束后 read 验收应失败，记录这个失败，不要记作读文件成功。
若出现模型/协议错误，记录错误并停止本轮；没有完整 Assistant 时，不应该出现可执行的工具预览。

## 4. 每次只执行一个工具

当 `next.kind = tool` 时，把工具名和 `arguments` 抄到笔记，再输入一次 `n`。
新的输出应包含 `[tool]`、`[result]`，然后停在 `AfterTool`；输入 `i` 检查结果。
`Message::Tool` 的结果可能是 `ok` 或 `error`。`AfterTool` 只证明工具已返回、结果已提交，不等于成功。

按实际 `next` 继续：

- `tool`：同一批还有调用，下一次 `n` 只执行下一个工具。
- `model`：本批结果已闭合，下一次 `n` 才把新历史交给模型。
- `finish`：到达结束动作；可能是模型步数上限，不再请求总结。

观察到 `read_file` 成功时，从 `output.data.text` 找到文件中的 code，把它留在笔记里。
不要把工具预览参数当作读到的内容；这个场景的 code 事先只在真实 `marker.txt` 中。
如果没有读成功、结果报错或模型反复调用工具，就记录实际分支，而不是补出教程里的理想答案。

## 5. 分清“最终回答”和“已经结束”

工具批次结束后，按 `next` 放行模型。在 `AfterModel` 且 `next.kind = finish` 时停下来：
检查最后一条 Assistant 的文本是否包含刚才实际读到的 code，且 `tool_calls` 为空。
此时最终回答已经提交，但 run 还没结束；输入 `n` 释放 Finish。

成功路径应看到 `[run] Completed` 和 `[PASS]`。回到 shell 后马上执行：

```bash
echo $?
```

应为 `0`。`Completed` 只说模型不再请求工具；Lab 的 PASS 还要求真实读取、答案和磁盘事实通过独立校验。
若 `[FAIL]`，保留最早的错误、工具结果和最终文本；不要仅因模型说“完成”就判定通过。

常见“一个模型调用一个工具，再总结”的新会话有 `1 → 2 → 3 → 4` 条消息：User、Assistant、Tool、Assistant。
这不是固定剧本：多工具、多模型轮次、错误和取消都会改变条数与暂停次数。
始终用 `point / next / messages` 判断当前位置，不能靠“应该已经按了三次 n”。

## 6. 回到源码，解释刚才看到的因果

按这个顺序打开 [src/agent.rs](../../src/agent.rs)，每一处都对照一条自己的现场记录：

1. `Agent::run_debug` → `run_with_control`：调试入口仍调用共同循环。接受输入后先提交 User；调试器不是另一套 Agent。
2. `run_loop` 的 `ModelRequest` 和 `self.model.complete(...)`：一次循环迭代发起一次模型请求，所以模型 step 与按键次数不同。
3. `validate_assistant` → `calls = assistant.tool_calls.clone()` → `commit`：先校验完整结果，再持有独立调用列表，然后提交 Assistant。
4. `for (index, call) in calls.iter().enumerate()`：工具串行执行；每个结果经 `commit` 回填；下一动作由剩余调用、步数上限决定。
5. `calls.is_empty()` 与末尾 `RunOutcome`：没有工具时走 Completed；还需越过调试 checkpoint 才真正返回。

最后打开 [src/event.rs](../../src/event.rs)，区分 `RunState`、`Phase`、`RunOutcome`。
enum 让“运行中/暂停/结束”能被分别表达，但合法转换仍依靠循环代码，并非定义 enum 就自动正确。

这里最重要的 Rust 所有权不是记住语法，而是找到**谁有写入权**：
`Agent` 拥有 `messages: Vec<Message>`，`run`/`run_debug` 借用 `&mut self`，同一 Agent 的运行不能被安全 Rust 调用者并发修改。
`calls.clone()` 让工具批次不再借用历史里的 Assistant，因此循环可以继续向 `messages` 追加 Tool 结果。
`commit` 把消息克隆写进历史，再把 owned 事件交给观察者；终端输出不是会话的第二个写入者。
详细合同见 [Agent 架构](../modules/agent/architecture.md)与[设计](../modules/agent/design.md)。

## 7. 用不依赖 LLM 的测试固定因果

在另一个仓库根目录终端运行：

```bash
cargo test --locked --test agent model_tools_model_and_second_user_turn_have_ordered_history -- --exact --nocapture
```

应显示 `running 1 test` 和 `1 passed`，不是 `0 tests`；若为 0，先核对完整函数名与 `--test agent`。
打开 [tests/agent.rs](../../tests/agent.rs) 中同名函数：它安排两个 `count` 工具和一个总结，而非等待真实模型自由发挥。
断言首次运行有 5 条消息、下一次请求中的两个 Tool 结果按 `a/b` 排列，计数分别为 `0/1`。
第二个 user turn 后总数为 7，第三次模型调用收到 6 条历史：新问题复用了之前历史，但还没有自己的新回复。
测试用可控输入隔离了调度器因果；本章 Lab 则确认了真实模型路径，两者不能互相替代。

## 自检与本章产出

不用看源码先回答：为什么执行一个工具不会让模型 step 加一？为什么最后还要释放 Finish？谁提交 Tool 结果？

<details>
<summary>展开参考答案</summary>

模型 step 只数 `complete` 所在循环轮次，调试 Step 可以释放模型、单个工具或结束动作。
最终 Assistant 后的 checkpoint 保留了一个可检查现场；越过它才返回 Completed。
工具只返回 `ToolOutput`，Agent 关联原调用 ID 并提交 `Message::Tool`；模型和输出层不能改写历史。

</details>

完成标准：留下一份至少覆盖初始、工具前、工具后、最终回答的观察记录，并能逐项指到上述源码。
如果没有拿到成功读取路径，把失败作为待查证记录，先用确定性测试理解循环，不伪造现场。

下一章：[02 上下文：模型到底看到了什么](02-context.md)。
