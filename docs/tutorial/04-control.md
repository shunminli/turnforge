# 第 4 课：控制执行——暂停、单步、继续和取消

[学习路线](../harness-learning.md) · 上一课：[工具与权限](03-tools.md) · 下一课：[输入输出与生命周期](05-io.md)

## 本课要解决的问题

你已经能区分“模型提出工具调用”和“工具真的执行”。本课继续追问：用户按下一个键，
究竟是谁允许下一动作发生？为什么旧命令不能推进新暂停？取消是否会撤销刚写入的文件？

完成后，你应该能沿着 `stdin → DebugController → DebugSession → Agent` 解释一次控制，
并用确定性测试证明时序规则，而不是依靠手速。建议预留 45–60 分钟。
前置：完成[环境准备](00-setup.md)及第 3 课；以下 shell 命令均在仓库根目录执行。
交互实验需要已启动且匹配基线的 Ollama；源码阅读和确定性测试不需要模型。

## Step 1：先验证“看现场”不是“推进现场”

先查看课程卡片，然后启动只读实验：

```sh
cargo run --quiet --locked -- learn control
bash scripts/harness-lab.sh --case read --lesson control
```

等待实际的 `[lab paused #...]`，记下 `pause_id`、`point`、`next` 和已提交消息数。
接下来输入的是 **Lab 控制命令，不是 shell 命令**；每次输入一行并等待输出：

1. 输入 `i`，阅读完整快照。
2. 再输入 `i`，比较第二份快照。
3. 输入 `l`，阅读本课提示，再输入一次 `i`。

判定：这几次观察的暂停 ID、消息、待执行调用及 `next` 都应相同；不应因此出现新的模型请求或工具执行。
快照是已提交事实的副本，`l` 是宿主输出的课件，二者都不是 Agent 的输入消息。
如果界面还未显示暂停，先等它出现；不要把“尚未暂停”的快捷键提示当作模型故障。

现在输入 `p`。因为已经暂停，应看到 `already paused` 对应的拒绝提示，现场不前进。
这说明 Pause 是“请求运行中的 Agent 在下一个安全边界停下”，不是创建第二层暂停。

## Step 2：用一次单步和一次继续区分两种模式

在同一个实验中，先写下预测：当前 `next` 是模型请求、工具调用，还是结束动作？
输入 `n`，等下一份暂停提示，再输入 `i`。

核对：只发生了上一份快照承诺的一个语义动作；模型请求结束后可以有工具预览，
但预览本身不是工具已执行。不要按教程猜暂停总数：模型可能先列目录，再读文件。
`n` 不是 Rust 源码逐行执行，也不代表 `max_steps` 消耗一次；只有模型请求消耗模型步骤。

输入 `c`，让这个实验继续到结束。预期成功场景最终有 `[run] Completed` 和 `[PASS]`，
返回 shell 后立即检查退出码：

```sh
echo $?
```

应为 `0`。若出现 `[FAIL]`，保留首次错误，按[第 6 课](06-regression.md)分类，不重跑到成功为止。
Continue 只改变后续边界是否等待用户，不换 Agent、不删消息，也不自动扩大工具权限。

可选观察：运行中的 `c` 之后可以另行输入 `p`，请求下一安全边界暂停。
但本地小模型可能在你按下 `p` 前已经结束；“没赶上”不证明 Pause 失效，也不作为本课通过条件。
可靠的时序证明放在下一步。

## Step 3：用可控模型证明 Pause 不截断当前动作

运行已有的单个测试，不访问 Ollama：

```sh
cargo test --locked --test debugging pause_during_a_model_waits_for_its_complete_response_boundary -- --exact --nocapture
```

应看到 `running 1 test` 及该测试 `ok`；若为 `0 tests`，这是过滤器没有命中，不能记作验证通过。
在 [tests/debugging.rs](../../tests/debugging.rs) 按顺序阅读 `GatedModel` 和该测试：

1. `started: Arc<Notify>` 告诉测试“模型动作已经开始”。
2. `release: Arc<Notify>` 让模型明确等待，避免靠 sleep 或人的输入速度制造窗口。
3. 测试在动作中发送 Pause，然后放行完整响应。
4. 下一份快照包含完整的 Assistant，随后才接受继续。

`Arc` 在这里共享通知器，不共享可变 Agent；测试同时轮询运行与控制，依然只有 run 持有 `&mut Agent`。
再读 [agent.rs](../../src/agent.rs) 的 `run_loop`：模型完成、校验、commit 后才调用 checkpoint。
Pause 因而不能把半条模型响应提交成消息。工具动作也只在返回并完成自身清理后到达暂停边界；
这不代表任意工具都能被立即抢占，具体工具的取消合同仍归工具 owner。

## Step 4：追踪 pause ID 为什么还需要入队 epoch

按顺序打开 [debug_input.rs](../../src/debug_input.rs) 的 `next_command`、
[debug.rs](../../src/debug.rs) 的 `DebugController::try_send`、`QueuedCommand`、
`DebugSession::checkpoint`、`PauseEpoch::drop`。

先自己写下“只检查命令里的 ID 是否等于当前暂停 ID”可能漏掉什么，再对照实现：

1. Lab 输入把快捷键绑定到读到该行首字节时观察的暂停；缓冲里较早输入的下一行不能换绑未来现场。
2. Controller 入队时，再把 Session 发布的活动 epoch 放进私有 `QueuedCommand`；运行中该值为 `0`。
3. Session 发布新暂停后，只有**自报 ID 和入队 epoch 同时等于当前 ID**的 Step/Continue/Inspect 才有效。
4. `PauseEpoch` 是 RAII guard：正常恢复或其它离开暂停路径释放 guard 时，将 epoch 清零。

所以“提前猜中下一个 ID”也没有授权效力。`Arc<AtomicU64>` 只是这个相关性窗口的共享元数据：
Session 写，Controller 读；它不是第二份 Agent 状态，也不是可编辑会话。guard 的 Drop 关闭窗口，
不等于恢复被丢弃的 run future。

分别运行两个证据，每条都应只命中一个测试：

```sh
cargo test --locked --test debugging an_early_command_for_a_future_pause_cannot_release_that_pause -- --exact --nocapture
cargo test --locked --test debugging replenished_early_commands_cannot_cross_into_a_new_pause -- --exact --nocapture
```

阅读断言：前者在未来暂停出现前预送命令；后者在拒绝回调中持续补入旧命令。
二者都用工具调用计数检查是否越界，不只是检查屏幕上出现“拒绝”二字。
第二个测试尤其说明：排空有限命令只是调度公平性，不能替代入队 epoch 的安全检查。

## Step 5：取消结束运行，但不回滚世界

重新启动一个实验，等第一次暂停后输入 `q`：

```sh
bash scripts/harness-lab.sh --case read --lesson control
```

返回 shell 后立即执行 `echo $?`。在预检成功且输入输出健康时，应看到 `[CANCELLED]`、退出 `130`，
本次没有模型动作；这里仍有模型服务的预检 HTTP 请求，不能说“完全没有网络访问”。
若预检失败或输出损坏，先处理对应错误，不能仅凭输入过 `q` 就断言退出码必须为 `130`。

阅读 `DebugController::cancel`：它直接设置 CancellationToken，不需要等待命令队列腾位。
再沿 `Agent::run_loop` 查取消分支：未执行的已提交调用补 cancelled 结果，正在运行的工具仍被 await，
已经提交的结果和已经发生的文件副作用保留。Lab 最后的 TempDir 清理是实验目录生命周期，**不是取消回滚**。

用现有测试验证工具清理不能被“立即退出”取代：

```sh
cargo test --locked --test debugging cancellation_bypasses_a_full_queue_and_awaits_active_tool_cleanup -- --exact --nocapture
```

应命中一个测试。阅读其 cleanup 通知和完成断言，确认测试等待真实的工具收尾，不是丢弃 future 后假称取消成功。

## 自检与学习记录

先不看答案，解释：宿主为什么无需取得 `&mut Agent` 来 Inspect？为什么合法 ID 仍可能被拒绝？
为什么取消接口返回和整个运行已清理完成不是同一件事？

<details>
<summary>参考答案</summary>

Inspect 由正在持有 Agent 的 run 在安全点生成 owned 快照，宿主拿的是副本。
ID 必须属于当前已发布暂停，且命令入队 epoch 也匹配；数字相同不足以证明命令在正确时点提交。
cancel 只是发送协作取消信号；宿主必须继续 await run，工具才有机会清理并闭合调用结果。
调用已产生的副作用不能靠取消 token 自动撤销。

</details>

在自己的学习笔记记下：两次 Inspect 的 ID/消息数、一次 `n` 的前后动作、Pause 测试的门控点、
epoch 的写入与清零 owner、一次取消的退出码及解释。不要把完整业务快照或凭据提交到仓库。
能不看源码画出这条控制链后，进入[第 5 课：输入输出与生命周期](05-io.md)。
