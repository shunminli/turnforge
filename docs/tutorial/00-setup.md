# 第 0 章：课程准备与阅读地图

[课程目录](../harness-learning.md) · 下一章：[一次 Agent 循环](01-loop.md)

本章把“已经会运行”转成“准备观察实现”：核对学习前置，分配观察终端，并建立最小源码地图。
本教程以仓库当前的 Rust CLI 为对象，面向 macOS/Linux；课程只操作合成实验，不使用真实业务数据。

## Step 1：核对学习前置

先确认自己已经完成 [101 用户指南](../101-user-guide.md) 的首次使用闭环：

- 知道本机 checkout 的仓库根目录，并能在需要的终端使用 Cargo。
- 至少有一次 chat 的实际 PASS 和退出 `0` 记录，而不是只看到了模型回答。
- 能打开 Lab、等到实际暂停后操作，再回到 shell 查看退出码。

这些尚未完成时，回到 101 操作；本章不维护第二套克隆、构建、服务启动或 chat 自检步骤。
模型服务安装、版本或 digest 不匹配由[本地 LLM 环境指南](../local-llm.md)处理，
不要将环境失败当作内核实验结果，也不要为了继续上课而修改锁文件。

本章以后的所有 shell 命令，除特别注明外，都在仓库根目录运行。
真实 Lab 实验需要匹配基线的本地服务仍然可用；完成过 101 不代表模型服务永远运行。
没有服务时，可以先读源码和运行各章的确定性测试，但真实模型观察须记为“未执行”，不能补写预期现场。

## Step 2：安排三个终端的学习分工

| 终端 | 本课中的角色 | 观察与生命周期边界 |
|---|---|---|
| A | 已准备好的 Ollama 本地 API 服务 | 负责模型服务，不负责 Agent 历史；不要为上课重复启动服务 |
| B | Harness 实验与学习操作 | 每次暂停预测下一动作，再输入 n/i/l/q；实验结束后返回 shell |
| C | 第 3 章观察临时文件、运行独立测试 | 只读核验磁盘事实，不替模型创建目标文件；不要同时争抢模型 |

需要服务、Cargo 或新终端的配置时，按 [101](../101-user-guide.md) 完成对应准备后再回来。
`lab` 提示下输入的是调试命令，不是 shell；`echo $?`、`ls` 等要等实验退出或在终端 C 执行。
课程使用的确切按键见 [Lab 控制参考](../harness-lab.md#交互控制)，每次只提交一条命令并等待新现场。

后续第 3 章要求 B 停在 `AfterTool`，同时由 C 检查文件。这种安排是为了得到独立事实，
不是为同一个实验创建第二个 Agent。正常退出后临时目录会被清理，不能到实验结束后再补做磁盘观察。
取消或关闭 B 不等于停止 A，也不意味着已发生的副作用被回滚。

## Step 3：带着最小地图进入源码

```text
main.rs：装配 Host、Model、ToolRegistry、取消和输入输出
   └─ Agent::run_debug → Agent::run_loop：唯一提交 messages 的循环
        ├─ Model::complete → OpenAiModel：一次模型请求与 SSE 组装
        ├─ ToolRegistry::execute → FileTool：权限检查与真实副作用
        └─ DebugSession::checkpoint：安全边界上的暂停控制
main.rs / output.rs：现场展示；Lab：独立验收；learning.rs：只读课件
```

先读[系统架构](../architecture.md)的组合关系，不必一次读完所有模块。
第 1 章从 [agent.rs](../../src/agent.rs) 的 `run_debug → run_with_control → run_loop → commit` 开始。
`main.rs` 是宿主而不是 Agent 内核；不要先被全部 CLI 参数和终端文件描述符细节淹没。

### 阅读所需的最小 Rust 词表

| 写法 / 概念 | 在本项目中怎么理解 | 后续章节 |
|---|---|---|
| crate / Cargo | crate 是 Rust 编译单元；Cargo 管构建与测试，Turnforge 才是我们的 CLI | 本章 |
| `&T` / `&mut T` | 只读借用 / 独占可变借用；`&mut Agent` 限制同一会话被并发修改 | 1 |
| owned / move / clone | 自己持有值、转移所有权、复制值；快照复制不等于复制运行控制权 | 2 |
| `trait Model` / `trait Tool` | 接口约定；实际实现仍需遵守取消、结果和副作用合同 | 2–3 |
| `enum` / `Result` | 明确有限状态或成功/错误分支，不能把错误结果当作成功值 | 3–4 |
| `async` / `.await` | future 在被轮询时推进；await 等待不等于新建后台线程 | 4–5 |
| channel / `Drop` | 值的传递通道 / owner 离开作用域时释放资源；释放不等于事务回滚 | 5 |

遇到术语时先用“谁拥有、谁借用、何时结束”解释，再看 Rust 语法；不要求先完成一门 Rust 语言课程。
使用[课程目录中的记录模板](../harness-learning.md#学习记录模板)记下自己的预测与实际证据，课程不会自动保存进度。

## 本章自检

为什么要区分 A/B/C？终端 C 看到文件不存在，能否不问观察时点就断言工具未执行？
在源码地图中，哪一个 owner 能追加会话历史？

<details>
<summary>参考答案</summary>

A 提供模型响应，B 拥有本次 Harness 运行，C 独立读取磁盘事实；关闭一个不等于关闭另外两个。
必须在实验仍存活的明确暂停点观察文件；退出后的 TempDir 清理不能证明从未执行写入。
Agent 拥有并提交 messages；模型、课件和终端观察者都不是第二个会话 writer。

</details>

准备好后进入 [第 1 章：亲手走完一次 Agent 循环](01-loop.md)。
