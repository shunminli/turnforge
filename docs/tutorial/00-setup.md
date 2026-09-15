# 第 0 章：准备环境与阅读地图

[课程目录](../harness-learning.md) · 下一章：[一次 Agent 循环](01-loop.md)

本章结束时，你应该能打开离线课程、找到正确仓库，并知道真实模型实验需要哪些进程。
本教程以仓库当前的 Rust CLI 为对象，面向 macOS/Linux；不安装新服务、不把模型权重放进 Git。

## Step 1：先确认你在哪个目录

在终端中进入你已经克隆的 **turnforge 仓库目录**。下面的路径是占位符，必须替换成自己的实际路径：

```sh
cd "/你的实际路径/turnforge"
pwd
ls Cargo.toml rust-toolchain.toml scripts/harness-lab.sh
```

最后一条应列出三个文件。如果找不到，先修正目录，不要继续执行后面的命令。
`coding agent` 这样的父目录不是仓库根目录；`./target/...` 中的 `.` 也不会自动帮你寻找子目录。
本章以后的所有命令，除特别注明外，都在这个仓库根目录运行。

## Step 2：用 Cargo 构建并打开课程

如果机器已经通过 rustup 安装 Rust，在当前终端加载 Cargo：

```sh
source "$HOME/.cargo/env"
cargo --version
cargo run --quiet --locked -- learn
```

你应该看到 `Harness 学习路线 / learn` 和六个课程名。这一步不调用 Ollama。
`cargo run` 会按需构建并启动程序；第一次可能需要下载 Rust 工具链或 Rust 依赖，离线机器需要已有缓存。
“课程不调用模型”不等于“首次编译无需网络”。`--locked` 要求沿用仓库的 Cargo.lock，而不是悄悄更新依赖。

如果 `~/.cargo/env` 不存在，说明这条 rustup 安装路径尚未准备好；按[项目快速开始](../../README.md#快速开始)
确认 Rust 安装，不要把后面的 `command not found` 当作 Harness 内核问题。
如果 Cargo 已安装但没在 PATH，也可以直接运行 `"$HOME/.cargo/bin/cargo" run --quiet --locked -- learn`。

再打开第一课卡片：

```sh
cargo run --quiet --locked -- learn loop
```

应看到目标、前置、源码、运行、观察练习与人工验收。卡片是速查摘要；完整教学内容在各章 Markdown 中。

### 为什么 Git 拉取后没有 target/debug/turnforge

Git 保存源码；`target/` 被 [.gitignore](../../.gitignore) 忽略，因为它是本机编译输出。
只有先构建、没有改 Cargo 输出目录时，才通常存在 `target/debug/turnforge`。
本教程使用 `cargo run` 和启动脚本，不要求你预先找到二进制，更不需要提交它到远端。

## Step 3：分清三个终端的职责

| 终端 | 用途 | 生命周期 |
|---|---|---|
| A | Ollama 本地 API 服务（只有服务尚未启动才需要） | 前台运行；在这个终端 Ctrl-C 停服务 |
| B | Harness 实验与学习操作 | 等待你输入 n/i/l/q；实验结束后返回 shell |
| C | 第 3 章观察临时文件、运行独立测试 | 只读观察文件；不要误在实验输入框里执行 shell 命令 |

每个新终端先做 Step 1；需要 Cargo 的终端再做 Step 2 的 `source`。
`lab` 提示下输入的是调试命令，不是 shell；`echo $?`、`ls` 等要等实验退出或在终端 C 执行。

## Step 4：检查本地模型服务

先做只读检查，不要重复启动服务：

```sh
curl --fail --silent --show-error --max-time 5 --noproxy '*' http://127.0.0.1:11434/api/version
```

若连接失败且本地模型已按[本地 LLM 指南](../local-llm.md)安装，在终端 A 启动：

```sh
bash scripts/serve-local-llm.sh
```

保持 A 打开，回到 B 重做连接检查。若端口被占用，先核对服务归属，不要杀未知进程或换成公开监听地址。
服务应使用仓库锁定的 Ollama 版本与测试模型；仅版本接口成功还不证明模型名称/digest 匹配。

首次安装、权重下载和模型别名创建按[本地模型准备](../local-llm.md#4-服务启动下载与创建测试模型)完成；
已匹配的环境不需要每次重新下载。教程和 Lab 均不会自动安装、启动服务或下载模型。

## Step 5：做一次最小环境自检

在 B 执行并等待结束：

```sh
bash scripts/harness-lab.sh --case chat --auto
echo $?
```

正常情况下，你会看到模型回复、`[run] Completed`、`[PASS]`，最后退出码 `0`。
Lab 会先核对完整服务/模型基线；`chat` 的 PASS 只检查正常结束、最终回答非空且无工具调用，不评价问候语义。

如果失败，先读 `[FAIL]` 或预检错误：

- 连接失败：回到 Step 4 检查服务和端口。
- 版本/digest 不符或缺模型：核对[固定基线](../local-llm.md#2-固定的测试基线)，不要为了通过修改 lock。
- 模型调用了工具或结果不符：保留本次错误，进入[第 6 章](06-regression.md)学习失败分类；不要重跑到一次成功就忽略失败。

## Step 6：带着最小地图进入源码

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

## 本章自检

能否不看旧聊天记录，独立启动 `learn`？能否指出“找不到文件”“服务不可达”“模型结果失败”分别发生在哪一层？

<details>
<summary>参考答案</summary>

找不到 `target/debug/turnforge` 首先是目录/编译产物问题，尚未进入 Agent；服务不可达发生在 Lab 预检或模型传输；
模型完成但 FAIL 则需要核对本次真实调用、消息与文件，不能把这三类问题混成“LLM 坏了”。
终端 A 拥有服务，B 拥有本次实验；关闭 B 不应被当成停止 Ollama 的操作。

</details>

准备好后进入 [第 1 章：亲手走完一次 Agent 循环](01-loop.md)。
