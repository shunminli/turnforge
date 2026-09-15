# 一键测试 Harness：本地实验 CLI

`turnforge lab` 把固定 Ollama 配置、合成测试场景、调试控制和结果验收封装到一个入口。
它运行真正的 Rust `Agent::run_debug`，不是另一个 Agent loop，也不是 LLM Space 或网页界面。

## 最快开始

在仓库根目录执行：

```sh
bash scripts/harness-lab.sh
```

默认是 **read 交互场景**：程序创建临时工作区和含动态未知 marker 的文件，首次模型调用前暂停。
按 Enter 执行一个动作，逐步看到模型提议工具、读取结果和最终回答；不需要记暂停 ID 或填写模型参数。

脚本不安装软件、下载模型或自动启动服务。若 Ollama 未运行，在另一终端保持前台服务：

```sh
bash scripts/serve-local-llm.sh
```

环境首次准备与固定模型来源见[本地 LLM 指南](local-llm.md)。不要因预检失败跳过模型指纹检查。
已构建 binary 时可直接使用 `target/debug/turnforge lab`；脚本负责定位仓库、构建并启动 CLI。

## 场景与自动验收

```sh
bash scripts/harness-lab.sh --case chat --auto
bash scripts/harness-lab.sh --case read --auto
bash scripts/harness-lab.sh --case write --auto
```

| 场景 | 执行内容 | PASS 依赖的独立事实 |
|---|---|---|
| `chat` | 请求合成文本回答 | 正常结束，最终已提交回答非空且全程没有工具调用（不判断问候语义） |
| `read`（默认） | 读取临时文件中的未知 marker | 实际成功的读取调用/工具结果，以及最终回答包含文件 marker |
| `write` | 按指定内容创建临时文件 | 成功的写入结果、磁盘读回内容符合预期及正常结束 |

`--auto` 不读取 stdin；它在每个真实 `debug_paused` 后提交一次 Step，包括最终 Finish。
这不是 Continue，也不会在尚未出现的暂停点预送命令。没有人工确认也会逐边界执行，并完成相同场景验收。
自动化应同时检查进程退出码和最终 PASS/FAIL；模型的 Completed 或“已完成”文本本身不足以验收。
取消、模型错误、步数耗尽或 oracle 不满足不能产生 PASS。真实小模型输出并非确定性证明；不自动重试到通过。

每次只运行一个场景，使用新的临时工作区；结束后清理。默认/read/chat 不授予写权限，只有 `write` 授予文件写能力。
所有场景均不授权 shell，不接受自定义 workspace 或任意 prompt，不会让模型读取仓库或用户文件来完成例题。
静态文件路径检查仍不是 OS sandbox；进程强杀时不保证临时目录清理。

## 交互控制

每条输入后按回车：

| 输入 | 含义 |
|---|---|
| Enter / `n` | 用当前观察到的暂停 ID 单步 |
| `c` | 从当前暂停连续运行 |
| `i` | 获取当前完整只读快照，不推进 |
| `p` | 在下一个安全边界暂停；不能冻结执行中的网络请求或文件工具 |
| `q` / Ctrl-C | 协作取消并等待清理，不回滚已完成副作用 |
| `h` / `help` | 显示控制帮助 |
| `l` / `learn` | 显示所选课程；未选课时显示总学习路线 |

快捷键由宿主关联到已观察的 pause ID，仍经过原有 Controller/epoch 校验，不绕过调试内核。
不要预先粘贴多组步进命令；一次操作后观察下一暂停。没有活动暂停时不能用 n/c/i 释放未来暂停。
EOF 触发取消；输入与运行结束竞速时不会改写已经完成的结果。常规 `turnforge debug` 的严格 ID/JSON 输入保持不变。

## 边调试边学

```sh
cargo run --quiet --locked -- learn
bash scripts/harness-lab.sh --case read --lesson loop
bash scripts/harness-lab.sh --case write --lesson tools
```

`--lesson` 在暂停点加上当前位置原理、源码符号和观察问题，不修改场景 prompt、权限、调度或验收条件。
逐步操作、预期现场、源码解读和自检答案见[Harness 完整学习教程](harness-learning.md)。
首次使用先完成[环境准备](tutorial/00-setup.md)；上面的 Cargo 命令会按需构建，不依赖已存在的 target/debug 二进制。
课程不自动判断是否学会，不存储学习进度。

## 固定配置与诊断

默认 Ollama origin 是 `http://127.0.0.1:11434`，测试模型固定为 `turnforge-test:qwen3-4b-v1`。
启动前核对 Ollama 版本、模型名称及 digest；完整预检和 oracle 合同见[Lab 设计](modules/harness-lab/design.md)。
如需其它本地端口，可显式传 `--ollama-url http://127.0.0.1:PORT`；仅接受无凭据、query、fragment 和路径的
loopback 字面 IP HTTP origin。不能用域名、云端 URL 或 `/v1` 路径绕过本机限制。

Lab 不读取 `TURNFORGE_MODEL`、`TURNFORGE_BASE_URL` 或云 API key 来选择模型，不跟随代理或 HTTP 重定向。
服务缺失/版本或模型不匹配会清楚失败，不自动下载、降级或更换模型。具体排障入口：

- 服务连接失败：先前台启动 `serve-local-llm.sh`，确认端口和 `--ollama-url` 一致。
- 基线不匹配：按[固定模型基线](local-llm.md)核对版本和模型；不要仅换一个名称相同的权重。
- 模型自然结束但 FAIL：查看真实工具结果与场景错误；自然结束不证明调用了要求的工具。
- 需要自定义 prompt、workspace 或机器 NDJSON：改用[原生 `debug`](debugging.md)，它不提供 lab 场景验收。

提示与快照可能包含临时文件内容和模型回复；lab 使用合成数据，但通用 debug 没有自动脱敏。

架构 owner 见[Lab 架构](modules/harness-lab/architecture.md)；宿主输入/退出和输出限制分别见
[CLI 设计](modules/cli/design.md)、[事件输出设计](modules/event-output/design.md)。
