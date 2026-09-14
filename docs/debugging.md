# Turnforge 分步骤调试

状态：已实现；面向当前 macOS/Linux CLI。
原生调试运行的是 **Turnforge 自己的 Rust 内核**，不是模拟一套模型—工具循环。
它让你停下来检查上下文、工具参数和结果，然后推进一个语义动作。
架构原理见 [debugger 架构](modules/debugger/architecture.md)，精确合同见[设计](modules/debugger/design.md)。

## 1. 启动本地调试

先按[本地 LLM 指南](local-llm.md)安装固定模型，并在一个终端以前台方式启动服务：

```sh
./scripts/serve-local-llm.sh
```

另开终端进入仓库。清除云模型 key，以临时目录做练习，不授权 shell：

```sh
source "$HOME/.cargo/env"
turnforge_debug_workspace="$(mktemp -d "${TMPDIR:-/tmp}/turnforge-debug.XXXXXX")"
env -u TURNFORGE_API_KEY -u OPENAI_API_KEY \
  NO_PROXY='*' no_proxy='*' \
  cargo run --locked -- debug 'Write result.txt containing exactly hello, then report completion.' \
  --workspace "$turnforge_debug_workspace" --allow-write \
  --base-url http://127.0.0.1:11434/v1 --model turnforge-test:qwen3-4b-v1
```

没有 `--allow-write` 时，写工具不会展示且运行时仍会拒绝；调试本身不赋予工具权限。
CLI 接收的 prompt 必须在参数里，`debug -` 不读取 prompt，因为 stdin 专门用于控制命令。
不要让另一个程序同时读取这个 stdin。普通 `run -` 行为不变。

## 2. 跟着暂停点操作

首次模型请求前即暂停。看当前输出的 pause ID，用它输入控制命令，不要预先猜测后续 ID：

| 输入 | 作用 |
|---|---|
| `inspect 1` | 查看 pause 1 的只读完整快照，不推进 |
| `step 1` | 从 pause 1 执行一个模型请求、一个工具调用或最终结束动作 |
| `continue 1` | 从 pause 1 连续运行 |
| `pause` | 连续运行中请求在下一个安全边界停下 |
| `cancel` | 请求取消并等待清理 |

数字 1 只是初始示例；每次 `step` 后都使用新输出的 ID。延迟到达的旧 ID 会被拒绝，不能让下一步意外运行。
执行模型动作会看到完整响应；若模型提出多个工具调用，每个工具仍单独一步。
在工具执行前检查 `next` 和 `pending_calls`；在工具结果后检查 `messages` 末尾及磁盘实际内容。
模型不再调用工具时，还会在最终回答之后暂停一次；下一次 Step/Continue 才结束运行。

`Pause` 不会冻结正在运行的 shell 或 HTTP 流；它等下一安全边界。
`Cancel`/Ctrl-C 走协作取消，已完成的文件写入不回滚。关闭控制输入（EOF）也取消；
自动化驱动应保持 stdin 打开到运行结束，不能简单把一条 `step` 管道输入后马上关闭。
输入只支持终端、pipe/socket，`/dev/null` 立即取消；普通文件重定向会拒绝。
每行最多 4 KiB，不含 LF；空行、非法 UTF-8/命令或超长行都会取消并作为宿主错误退出。

## 3. NDJSON 与自动化驱动

加 `--json` 后 stdout 每行一个 Event。控制 stdin 可用文本命令或等价 JSON，一行一条：

```json
{"command":"step","pause_id":1}
```

```json
{"command":"continue","pause_id":2}
```

JSON 命令拒绝未知字段。自动化应做到：

1. 启动 `turnforge debug ... --json`，持续读取 stdout/stderr，同时保留 stdin 的写端。
2. 遇到 `debug_paused`，检查 `snapshot.version`、`point`、`next`、`messages` 和 `pending_calls`。
3. 校验当前现场，再发送带该 `pause_id` 的控制命令。
4. Inspect 的应答是 `debug_snapshot`；它不产生新的 pause ID。处理 `debug_command_rejected`，不要当成成功。
5. 收到 `run_finished` 后仍等待进程退出，检查退出码和真实文件结果；stdout 失败可能没有完整终态。

原有 `step_started.step` 是模型请求次数，不是用户点击“下一步”的次数。
`debug_paused` 表示 Agent 确实处于安全等待点；`model_delta` 只是暂态输出，不能作为已执行工具的证据。
快照不是 provider 原始请求：它不包含凭据和完整模型配置，AfterModel 时还可能含待执行的工具调用。

## 4. 测试分层

默认回归不依赖真实模型，适合维护状态机和 CLI 协议；真实本地模型用于验证实际 provider/工具闭环。

```sh
cargo test --locked --all-targets
cargo test --locked --release --all-targets
```

仅运行确定性调试专项（不需要启动 Ollama）：

```sh
cargo test --locked --test debugging
cargo test --locked --test cli_http debug_cli_
```

服务及固定模型准备好后，显式运行真实模型调试用例。与其它真实模型测试串行执行，不争抢单模型服务：

```sh
cargo test --locked --test local_llm local_llm_debug_steps_through_read_and_final_answer -- --ignored --test-threads=1 --nocapture
```

该用例只读临时目录中的随机 marker，自动消费真实 CLI 的暂停事件并发送 Step，检查工具预览、
读取结果快照、最终答案和最终结束确认。它不需要 `--allow-write` 或 `--allow-shell`。
用例矩阵见[本地 LLM 指南](local-llm.md#用例矩阵)；库/CLI 的精确测试映射见[debugger 设计](modules/debugger/design.md#测试与变更检查)。
确定性测试还独立计数模型/工具调用并检查实际磁盘，不能只把 stdout 中的“暂停”当作副作用隔离证明。
实际测试记录属于某个版本与环境的证据，不代表所有模型都稳定选择同一工具顺序。

## 5. 安全、故障与范围

- 快照包括完整 system、用户输入、模型文本、文件内容、工具参数和结果；可能敏感，不自动脱敏或上传。
- 调试 channel 和 stdout 队列都有限额，但没有历史/快照总内存预算；长会话反复 Inspect 可能明显耗内存。
- 无效 JSON、非法命令或输入读取故障会取消并以宿主错误退出 1；有效但 ID 过期的命令只产生拒绝事件，运行继续等待。
- stdout 不消费可能触发输出超时并取消；自动化必须持续排空，不等模型结束后才读取。
- 暂停现场只存在于当前 run future；进程退出、future 被丢弃或强杀后不能恢复。
- 暂停不是工具审批；读/写/shell 权限仍使用原有开关。没有条件断点、源码逐行调试或 rewind。

LLM Space 目前只能作为另一个实验台；本项目**尚未实现与其的调试适配**。
未来可映射这些观察事件和控制命令，但不能让界面复制并接管 Rust transcript 的写入权。
