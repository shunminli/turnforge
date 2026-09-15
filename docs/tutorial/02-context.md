# 02 · 上下文：模型到底看到了什么

[学习总入口](../harness-learning.md) · 上一章：[01 调度循环](01-loop.md) · 下一章：[03 工具与权限](03-tools.md)

本章目标：从暂停现场追踪消息，理解借用、owned 快照、流式增量与提交边界为什么要分开。
前置：完成 [00 环境准备](00-setup.md)和第 01 章，能根据 `next.kind` 单步；仍在仓库根目录。
预计 35–50 分钟。本章只观察合成上下文，不修改提示词、源码或历史。

## 1. 新开一次有上下文提示的实验

```bash
cargo run --quiet --locked -- learn context
bash scripts/harness-lab.sh --case read --lesson context
```

在首次暂停输入 `i`。如果没有进入暂停，按第 00 章排查，不把预检错误当作模型执行结果。
这次关注四个字段：`system`、`messages`、`tools`、`pending_calls`。
先用自己的话各写一句含义，再继续执行。

`system` 是 Agent 配置；`messages` 是已提交的会话历史；`tools` 是本次授权后可见的定义；
`pending_calls` 是已提交但尚未得到结果的调用。它们不是四份相互独立、可任意编辑的会话。
此时 `messages` 只有 User，`pending_calls` 为空；模型尚未取得文件中的 code。
检查 User 文本：它告诉模型读 `marker.txt`，但不包含实际 code。

## 2. 在工具前找出“已提交但未配对”的调用

输入 `n`，等待模型完成；在 `AfterModel` 输入 `i`。
若 `next.kind = tool`，找最后一条 `role = assistant`，记录 `message.tool_calls` 中每项的 `id/name/arguments`。
再比较 `pending_calls`：刚提交后，该批调用还都没有 Tool 结果。

先不要执行工具，回答：“如果此刻把 `messages` 原封不动发给下一次模型，工具结果在哪里？”
答案是还没有。暂停快照允许展示执行中的中间态，它不等于可以立即重发的完整新请求。
如果模型没有调用工具，`pending_calls` 应为空；这也是实际结果，但无法完成本章的配对观察，记录后按 Finish 结束。

不要把 ID 猜成 `call-1`：ID 来自本次完整模型调用，必须读取实际值。
同一批里 ID 应唯一；目前的通用校验不是全会话全局 ID 去重或跨运行幂等保证。

## 3. 工具执行后追踪同一个 ID

在工具预览处输入一次 `n`，等 `AfterTool` 后输入 `i`。
找到新增的 `role = tool` 消息，它的 `call_id` 应等于刚才执行的 `ToolCall.id`。
检查 `output.status`，成功读文件时再读取 `output.data.text`；错误结果同样会进入上下文。

用实际 ID 在笔记写一对：

```text
Assistant.tool_calls 中的 id  →  Tool.call_id
模型提议的参数                 →  工具实际返回的 output
```

再看 `pending_calls`：只去掉本次已返回的调用，未执行项仍留着。
如果该批还有调用，按 `next` 一项项释放；直到 `next.kind = model` 时，当前批次应全部有结果。
`point` 此时仍是 `AfterTool`，不是一个新的 `BeforeModel`；`step` 仍指上一模型轮，`next.step` 才是即将发起的轮次。

常见单工具路径里消息数为 `1 → 2 → 3 → 4`，但判断配对要看 ID 与角色，不能只数条目。
多工具 Assistant 本身仍是 1 条消息，却需要多条 Tool 结果；错误和取消也要用结果闭合调用。

## 4. 预测下一次模型能知道什么，再放行

在 `next.kind = model` 的暂停处先不按键，用笔记写下它将新增知道的事实：
例如 `read_file` 成功返回的完整 code，或者工具返回的错误 code。
现在输入 `n`；最终 Assistant 应基于这些已提交事实作答，而不是靠 User prompt 猜文件内容。
继续按实际 `next` 到 Finish，释放后观察 Lab 的 PASS/FAIL 和 shell 退出码。

```bash
echo $?
```

模型也可能看到了正确结果却复制错误；这种情况下应保留工具文本和最终文本的差异。
不要把“上下文正确传入”直接等同于“模型答案必然正确”，也不要反过来仅凭错字判断上下文没传。
第 06 章会进一步拆开 Harness 合同与模型行为的验收。

## 5. 读源码：为什么请求借用、快照却复制

按顺序打开以下源码，分别圈出借用和 owned 的位置：

1. [src/message.rs](../../src/message.rs)：`Message` 的三个变体保存 `String/Vec/Value`，它们拥有数据，不指向某次 HTTP buffer。
2. [src/model.rs](../../src/model.rs)：`ModelRequest<'a>` 的 `&'a str`、`&'a [Message]`、`&'a [ToolDefinition]` 是只读借用。
3. [src/agent.rs](../../src/agent.rs) 的 `run_loop`：临时把自己的配置、历史和定义借给 `complete`；模型没有 `&mut Vec<Message>`。
4. 同文件的 `checkpoint`：构造 `DebugSnapshot` 时克隆 system/messages/tools/pending_calls，让事件拥有独立副本。
5. 同文件的 `commit`：只有这里把消息追加进 Agent 的历史，再通知观察者。

`'a` 表达借用可用的生命周期关系，不是“自动保存上下文”的机制。
模型调用结束，借用结束，Agent 才继续提交新历史；不需要给整个会话套 `Arc<Mutex<_>>` 才能实现这一点。
快照则要跨过事件边界供宿主消费，因此复制为 owned 数据；观察者清空自己收到的副本，也不能删 Agent 的历史。
代价是复制会占内存；owned 不意味着零成本、固定总预算或能跨进程恢复。

补充阅读：[协议架构](../modules/protocol/architecture.md)与[设计](../modules/protocol/design.md)。

## 6. 再往下读：屏幕上的字不一定已经提交

打开 [src/openai.rs](../../src/openai.rs)，按 `OpenAiModel::request` → `Assembly::push` → `Assembly::finish` 阅读。
`request` 先把逻辑结构翻译成 HTTP JSON：额外加入 system 消息，把内部 `Tool.call_id` 翻译为 `tool_call_id`，
把工具 arguments 和整个 ToolOutput 按外部协议编码成字符串。因此 `i` 显示的 JSON **不是原始 HTTP payload**。

流式返回时，`Assembly::push` 把文本/参数碎片积累到局部状态，同时发出 `ModelDelta` 供显示。
参数片段本身甚至可能不是完整 JSON；Lab 的简洁显示也不会把全部参数 delta 当作日志展示。
到 `[DONE]` 才调用 `finish`：核对结束原因、解析完整参数、构造 owned Assistant。
Agent 随后再执行 `validate_assistant`，通过才提交并考虑工具执行。

所以不应边看屏幕输出边执行半成品工具参数；流末尾失败时，已显示文字不能被撤回，却不会成为已提交 Assistant。
`MessageCommitted` 只意味着进入本进程内存历史，不意味着落盘。
快照没有原始 headers、完整 endpoint 配置或恢复导入接口；保存一份 JSON 也不是持久化恢复方案。
具体转换与失败规则见[模型设计](../modules/model/design.md)。

## 7. 跑两条确定性证据，核对自己的解释

先验证真实 HTTP 转换和工具回填（测试自己启动本地 TCP fixture，不使用 Ollama）：

```bash
cargo test --locked --test cli_http fragmented_tool_arguments_round_trip_to_real_file_and_model -- --exact --nocapture
```

应为 `running 1 test`、`1 passed`。打开 [tests/cli_http.rs](../../tests/cli_http.rs) 同名函数：
它把 `write_file` 参数拆成两个 SSE 片段，独立读回 `hello.txt`，再检查第二次请求中 Assistant 的 ID 与 Tool 的 `tool_call_id` 都为 fixture 的 `call-1`。
还检查 `ToolOutput` 字符串解码后 `status = ok`。这证明的是真实适配链路，不只是两个 Rust struct 长得一样。

再验证模型错误后的提交边界：

```bash
cargo test --locked --test agent provider_error_leaves_no_partial_assistant -- --exact --nocapture
```

同样应精确运行 1 条。打开 [tests/agent.rs](../../tests/agent.rs)：fixture 返回 HTTP 429 错误，断言只保留 1 条 User 和 1 个 terminal event。
这条测试证明 Agent 的错误路径，不是“所有 SSE 分片故障已覆盖”；SSE 非法流另有专门测试。
任一命令显示 `0 tests` 都不算验证；检查测试二进制名和完整函数名后再判断。

## 自检与本章产出

合上源码回答：ModelRequest 为什么用借用？快照能否改历史？AfterModel 的历史为什么可能暂时不能直接重发？delta 失败后怎么办？

<details>
<summary>展开参考答案</summary>

Model 只需临时读取请求，不应拥有或修改会话；生命周期约束保留 Agent 的单 writer。
快照是独立克隆，不能反向修改历史，也没有恢复权。
AfterModel 可能已有工具调用而尚无对应结果，要由 Agent 执行/取消并闭合调用。
delta 是暂态显示，协议失败不会提交 Assistant，也不允许执行其中碎片；已经显示的文字不自动消失。

</details>

完成标准：画出自己这次运行的一组调用/结果 ID 配对，写出“逻辑快照 → provider JSON”的三个差异，并标明提交 owner。
下一章：[03 工具、权限与真实副作用](03-tools.md)。
