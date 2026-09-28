# Harness Lab 使用参考

本页用于查参数、场景与控制命令，不是另一份入门或学习教程。
首次成功运行见 [101 用户指南](101-user-guide.md)，原理实验见[学习目录](harness-learning.md)，
安装和服务操作见[本地 LLM 指南](local-llm.md)，完整回归运行清单见[测试参考](testing.md)。

`turnforge lab` 把固定 Ollama 配置、合成场景、调试控制和结果验收封装到一个入口，
运行真正的 Rust `Agent::run_debug`，不是另一套 Agent loop、LLM Space 或网页界面。

## 调用形式与参数

在仓库根目录使用启动脚本；以下是参数语法，不是可以逐字复制的命令：

```text
bash scripts/harness-lab.sh [--case chat|read|write] [--auto]
                          [--ollama-url ORIGIN] [--lesson LESSON]
```

脚本定位仓库、按需构建，再调用 `turnforge lab`；不安装软件、下载模型或启动服务。
已经构建 binary 的宿主可直接传同样参数。常规 `run/debug` 的配置另见 [CLI 使用参考](cli.md)。

| 参数 | 缺省 | 用途 |
|---|---|---|
| `--case chat\|read\|write` | `read` | 选择合成场景；只有 write 授权临时工作区文件写入 |
| `--auto` | 关闭 | 无 stdin 控制，由宿主响应每个真实暂停并 Step，包括 Finish |
| `--ollama-url ORIGIN` | `http://127.0.0.1:11434` | 本机 Ollama origin，不是 `/v1` API 路径 |
| `--lesson LESSON` | 无 | 选择 `loop/context/tools/control/io/regression` 的现场学习提示 |

不接受自定义 prompt、workspace、model、API key、shell 权限或 NDJSON 输出参数。
课程只增加观察解释，不改变 prompt、权限、调度或 PASS 条件，也不保存学习进度。

## 场景与结果

| 场景 | 执行内容 | 验收概要 |
|---|---|---|
| `chat` | 请求合成文本回答 | 无工具的非空最终回答，不评价问候语义 |
| `read` | 读取临时文件中的未知 marker | 真实读取结果与最终回答使用该 marker |
| `write` | 按指定内容创建临时文件 | 工具结果与真实磁盘内容匹配预期 |

精确路径、内容、调用关联和断言由 [Lab 设计](modules/harness-lab/design.md#场景事实与独立验收)唯一维护。
自动模式仍经过真实语义边界，不是 Continue，也不提前发送未来暂停的命令。
检查最终 PASS/FAIL **及进程退出码**；模型的 Completed 或“已完成”文本本身不足以验收。
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

快捷键由宿主关联到已观察的 pause ID，不绕过调试内核；协议细节见[debugger 设计](modules/debugger/design.md)。
不要预先粘贴多组步进命令；一次操作后观察下一暂停。没有活动暂停时不能用 n/c/i 释放未来暂停。
EOF 触发取消；输入与运行结束竞速时不会改写已经完成的结果。常规 `turnforge debug` 的严格 ID/JSON 输入保持不变。

## 固定配置与诊断

运行环境由仓库 [lock](../dev/local-llm.lock.json) 固定；启动前核对 Ollama 版本、模型名称及 digest。
完整预检合同见[Lab 设计](modules/harness-lab/design.md)，安装和基线恢复见[本地 LLM 指南](local-llm.md)。
如需其它本地端口，可显式传 `--ollama-url http://127.0.0.1:PORT`；仅接受无凭据、query、fragment 和路径的
loopback 字面 IP HTTP origin。不能用域名、云端 URL 或 `/v1` 路径绕过本机限制。

Lab 不读取 `TURNFORGE_MODEL`、`TURNFORGE_BASE_URL` 或云 API key 来选择模型，不跟随代理或 HTTP 重定向。
服务缺失/版本或模型不匹配会清楚失败，不自动下载、降级或更换模型。具体排障入口：

- 服务连接失败：按[服务生命周期](local-llm.md#4-服务启动下载与创建测试模型)检查服务归属及端口是否与 `--ollama-url` 一致。
- 基线不匹配：按[固定模型基线](local-llm.md)核对版本和模型；不要仅换一个名称相同的权重。
- 模型自然结束但 FAIL：查看真实工具结果与场景错误；自然结束不证明调用了要求的工具。
- 需要自定义 prompt、workspace 或机器 NDJSON：改用[原生 `debug`](debugging.md)，它不提供 lab 场景验收。

提示与快照可能包含临时文件内容和模型回复；lab 使用合成数据，但通用 debug 没有自动脱敏。

架构 owner 见[Lab 架构](modules/harness-lab/architecture.md)；宿主输入/退出和输出限制分别见
[CLI 设计](modules/cli/design.md)、[事件输出设计](modules/event-output/design.md)。
