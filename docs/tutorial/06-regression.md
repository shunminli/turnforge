# 第 6 课：回归验证——模型说完成了，Harness 如何判断真的完成

[学习路线](../harness-learning.md) · 上一课：[输入输出与生命周期](05-io.md) · 下一课：[综合练习](07-capstone.md)

## 本课要解决的问题

前五课学会了观察和控制一次执行，现在需要回答：修改 Harness 后，用什么证据判断没有破坏它？
本课把模型自然结束、工具成功、任务验收和宿主成功退出分开，并实际走一遍两层测试。
完成后，你应该能解释每项 PASS 的独立依据，也能保留和定位一次失败，而不是只统计绿色测试数。

建议预留 45–60 分钟及模型运行时间。前置：[环境准备](00-setup.md)和第 5 课。
以下命令从仓库根目录执行；真实模型阶段需本机锁定基线，确定性测试不需要 Ollama。
不要同时启动多个 Lab/真实模型测试，本机服务按单并发配置。

## Step 1：先写验收条件，再看运行结果

```sh
cargo run --quiet --locked -- learn regression
```

在 [lab.rs](../../src/lab.rs) 找到 `Expected`、`prepare` 和 `PreparedLab::verify`。
先不看模型输出，在学习笔记写下以下三个场景的验收依据：

| 场景 | 运行前已确定的事实 | PASS 还需要什么 |
|---|---|---|
| chat | 不需要工具 | 最终 Assistant 非空，整个已闭合历史中没有工具调用 |
| read | `marker.txt` 中的动态 marker 与原始内容 | 成功 read_file 结果及最终回答都包含原 marker，磁盘原文件不变 |
| write | `result.txt` 精确内容 `turnforge-lab-write-ok`，没有末尾换行 | 调用参数正确、成功结果路径/字节数正确、独立磁盘读回完全一致 |

共同前提是完整最终 Assistant 和合法配对的调用/结果。chat 不判断问候措辞的语义质量；
write 也没有扫描整个目录来证明 prompt 里的“没有其它文件”，不要把未检查的要求写成已有保证。

读 `prepare` 时确认：read 的期望 marker 在请求前产生，只在文件与私有 Expected 中保存，
不直接塞进 prompt 或工作区名字。它是时间值生成的可读演示词组，不是安全随机数或模型能力基准。
Expected 与 TempDir 由同一个 PreparedLab 持有；`verify(&self, messages: &[Message])` 只借用它们，
不把模型回答改成 expected，也不反向改写 Agent 历史。这就是 oracle 的独立性。

## Step 2：串行运行三种真实场景

每条命令完成后先记结果，再执行下一条；`--auto` 自动响应实际暂停，不跳过同一 Harness 调度路径。

```sh
bash scripts/harness-lab.sh --case chat --auto --lesson regression
echo $?
```

观察非空最终回答、没有 `[tool]`、最终 `[PASS]` 及退出 `0`。这证明有限的纯文本链路，不证明回答质量。

```sh
bash scripts/harness-lab.sh --case read --auto --lesson regression
echo $?
```

观察实际 `read_file` 及 `[result]`，比较文件返回的 marker 和最终回答；还需 `[PASS]`、退出 `0`。
不要要求固定工具调用 ID、精确措辞或暂停总数。模型可能多做一次合法读取；验收检查的是声明的事实。

```sh
bash scripts/harness-lab.sh --case write --auto --lesson regression
echo $?
```

观察 `write_file` 的 path/content 和工具结果。成功时 `[PASS]` 表示内部还独立读回了磁盘，
不是因为模型说“已写入”就通过。写权限只用于这个临时实验，始终不授权 shell。
正常退出后临时目录已清理，因此不要把“退出后找不到 result.txt”误认为写入失败。

任何一项失败，先停在该项记录 stderr、退出码、工具结果与最后回答；进入 Step 6 分类。
不要自动重试到绿，也不要用最后一次成功覆盖首次失败。修复后再清楚标注“定向复验”与“完整重跑”。

## Step 3：让一个假成功稳定地失败

真实模型不一定每次产生你想研究的错误。要验证拒绝“口头成功”，使用可控 HTTP 夹具：

```sh
cargo test --locked --test lab_cli lab_read_completed_without_a_tool_is_not_a_pass -- --exact --nocapture
```

应看到 `running 1 test` 和 `ok`。这不矛盾：**被测 CLI 应失败，检查它失败的测试应成功**。
在 [tests/lab_cli.rs](../../tests/lab_cli.rs) 打开该函数：服务端只返回“I have read the file successfully.”，
不提出工具调用；真实 binary 自然结束，但独立 oracle 拒绝它，测试检查退出 `1` 和 `[FAIL]`。
如果结果是 `0 tests`，说明没有命中，不是这个行为已经通过。

再验证写错内容不会假通过：

```sh
cargo test --locked --test lab_cli lab_write_pass_requires_the_real_file_to_match_the_independent_oracle -- --exact --nocapture
```

同样只命中一个测试，它内部覆盖正确和错误两种内容。夹具在模型后续请求到来时独立读取真实文件，
确认工具确实写了它请求的内容；错误分支即使写工具成功、模型自然结束，Lab 仍应 FAIL。
测试替换的是远端响应，不是 Rust 工具实现：仍走真实 HTTP、真实 CLI、真实文件副作用。

## Step 4：沿源码把“结束”拆成四个层次

按顺序读 [agent.rs](../../src/agent.rs) 的 `run_loop`，再读 [main.rs](../../src/main.rs) 的
`run` future、`scenario.verify(agent.messages())` 分支和最后的错误处理：

1. **Agent Completed**：一条有效完整 Assistant 没有请求更多工具，且没有取消；调度可以结束。
2. **工具结果成功**：具体工具返回事实，可能只是成功执行了一个并不符合场景目标的动作。
3. **Lab PASS**：Completed 后，oracle 检查已提交历史与独立文件事实；错误会把宿主运行结果改为 Err。
4. **进程退出 0**：oracle 成功还不够，输入/输出与收尾也必须成功，writer 必须交付并退出。

因此 Completed 不等于 PASS，PASS 文本也不是脱离进程退出码的成功凭证。
`Cancelled`、`StepLimit`、运行错误不会进入成功 oracle 分支；学习提示不参与这四层判断。
编译器限制可变借用，保证宿主不能在 run 持有 `&mut Agent` 时并发改历史；业务是否正确仍靠运行时检查和测试，
不能把“Rust 编译通过”当成任务完成证明。

## Step 5：执行两层回归，而不是只有真实模型冒烟

先跑默认确定性回归：

```sh
cargo test --locked --all-targets
```

查看失败/忽略数量；默认 `ignored` 的真实模型用例没有执行，不能计为通过。
默认回归用可控响应覆盖协议、权限、取消、文件和进程等边界；不是所有测试都只是纯内存 mock。

列出真实模型用例，这条命令只列清单，不调用模型：

```sh
cargo test --locked --test local_llm -- --list
```

确认 Ollama 基线正常，并在前一命令结束后，依次显式执行 debug 和 release 模型回归：

```sh
cargo test --locked --test local_llm -- --ignored --test-threads=1 --nocapture
```

记录结果；全部结束且没有需要先定位的失败，再运行：

```sh
cargo test --locked --release --test local_llm -- --ignored --test-threads=1 --nocapture
```

`--ignored` 选择真实模型用例；`--test-threads=1` 只约束当前测试进程，不阻止另一个终端争抢模型。
完整运行时变更门禁还包括格式、Clippy、release 确定性测试与构建，按 [CI](../../.github/workflows/ci.yml) 执行；
本课不把两条模型命令冒充完整上线检查，也不在教程中固定将来的测试总数。

阅读 [tests/local_llm.rs](../../tests/local_llm.rs) 的 `local_llm_reads_unknown_file_marker`：
测试先生成 marker、写文件，再把不含 marker 的 prompt 交给 `run`，最后用原变量对比工具结果和答案。
这里的 expected 来自请求前的测试输入，不来自本次输出。它与 Lab 的可读词组 fixture 不是同一个 marker 格式。
该文件的 Lab 冒烟另外从文本结果检查前后传递，但 Lab 内部的 PASS 仍由私有 Expected 独立验收，不能混为一谈。

## Step 6：给失败分类，决定下一份证据

| 现场 | 下一步检查 | 不要这样处理 |
|---|---|---|
| 无法连接、版本/digest 不符 | 服务归属、锁文件与实际基线；回到[环境准备](00-setup.md) | 改成云端、跳过预检、为了过关改 lock |
| 协议错误、消息不闭合、工具越界 | 对应确定性测试及模块设计，找到首次错误边界 | 吞掉错误或只看最后一段文本 |
| 工具结果正确，最终 marker 错 | 保留原 expected、实际文件/结果和模型答案，区分模型行为与传递故障 | 把 expected 改成本次输出 |
| 写入内容不对 | 调用参数、成功结果、独立磁盘读回三个事实 | 相信模型自评或只看 ToolStarted |
| 超时、输出捕获失败 | 资源争抢、服务日志、宿主收尾与输出测试 | 无边界加超时、并发反复重试 |

模型固定 seed/temperature 能减少变化，不保证跨设备或版本逐 token 一致；有限 PASS 不证明通用 coding 能力。
细节与诊断边界见[本地 LLM 指南](../local-llm.md)和[Lab 设计](../modules/harness-lab/design.md)。

## 自检与学习记录

先回答：write_file 成功为何仍可 FAIL？假成功用例的 Cargo 测试为何应为绿色？哪一层保存独立 expected？

<details>
<summary>参考答案</summary>

工具只证明它完成了指定动作；指定内容本身可以错，oracle 还检查任务目标和真实磁盘。
假成功测试验证 CLI 能拒绝不合格结果，所以子进程退出 1 正是它的成功条件。
Lab 的 PreparedLab/Expected 保存请求前的事实；普通本地读文件测试由测试函数保存先生成的 marker。
模型回答不能成为这次运行自己的 expected。

</details>

在笔记记录三个场景各自的证据、一次确定性负例的“子进程失败/测试成功”、实际回归命令及结果。
没有跑过的模型用例写“未执行”，首次失败单独保留；不要把合成日志与业务日志混在一起。
最后进入[综合练习](07-capstone.md)，把源码、现场和测试串成你自己的完整解释。
