# Turnforge 101 用户指南：从零跑通并理解 Harness

这是一份可以边操作、边读源码的入门指南。路线是：**跑通 → 单步观察 → 阅读源码 → 运行测试 → 自己写测试**。不要求先通读仓库，也不要求先学完 Rust。

本文对应 Turnforge 提交 [`fe5abd6`](https://github.com/shunminli/turnforge/tree/fe5abd6d7d252b5b8ca8707950ec023178b8d847)，命令面向 macOS/Linux。文内源码和教程使用仓库相对链接，随当前 checkout 阅读；章节详解见[完整学习教程](harness-learning.md)。如果你的 checkout 已升级，先对照最新文档确认接口差异，不要为照着教程操作而丢弃本地改动。

本文说明操作方法与预期结果，不是一份本轮实际回归报告。只有你实际执行并记录结果的步骤，才能算作自己的验证证据。

## 学习路线

```text
用户任务 → 模型提出行动 → Harness 检查权限并执行工具
                      → 结果进入历史 → 再请求模型 → 结束
```

按 0–8 步顺序操作：环境 → chat → read/loop → context → write/tools → control → IO → regression → 自写权限测试。
建议第一次完成 0–4，再完成 5–8。每次暂停记录下一动作、历史变化、真实文件变化和对应源码。CLI 不自动保存学习进度。
模型负责提出行动；Harness 负责状态、执行、权限、取消与生命周期。Cargo 是 Rust 构建/测试工具，Turnforge 才是项目 CLI。

## 0. 准备环境：先分清目录和终端

### 0.1 获取或进入仓库

已有仓库时，替换下面的占位路径并进入；不要覆盖本地改动：

```bash
cd "/你的实际路径/turnforge"
pwd
ls Cargo.toml rust-toolchain.toml scripts/harness-lab.sh
git status --short
git rev-parse --short HEAD
```

尚未克隆时，在自己选择且不存在同名 `turnforge` 目录的位置运行 `git clone https://github.com/shunminli/turnforge.git`，再 `cd turnforge`。
找不到上述三个文件，先修正目录。本文后续 shell 命令除特别注明外，都在仓库根目录运行。

### 0.2 加载 Rust，打开无需模型的课程

已通过 rustup 安装 Rust 的终端执行：

```bash
source "$HOME/.cargo/env"
cargo --version
cargo run --quiet --locked -- learn
cargo run --quiet --locked -- learn loop
```

预期看到学习路线以及 loop 课程卡片。若 `~/.cargo/env` 不存在，先按 [项目快速开始](../README.md#快速开始) 准备 Rust，不要将安装问题当作 Harness 故障。

`cargo run` 会按需编译；第一次编译可能需要联网获取工具链和依赖。`learn` 本身不调用模型。`--locked` 沿用仓库锁定依赖，不悄悄更新 Cargo.lock。

找不到 `./target/debug/turnforge` 并不奇怪：`target/` 是本机编译产物，不在 Git 中。使用本文的 `cargo run` 和脚本即可，不需要预先寻找二进制。

### 0.3 准备三个终端

| 终端 | 用途 | 操作边界 |
|---|---|---|
| A | 本地 Ollama API 服务 | 仅服务尚未启动时使用；前台运行，Ctrl-C 停止自己启动的服务 |
| B | Harness 实验 | 暂停时输入 `i/n/l/c/q`，不是输入 shell 命令 |
| C | 观察真实磁盘、运行独立测试 | 不要手动替模型创建实验目标文件 |

每个新终端先进入仓库；需要 Cargo 的终端再执行 `source "$HOME/.cargo/env"`。`echo $?` 必须等 Lab 返回 shell 后立即执行，不能在 Lab 的调试输入框里输入。

### 0.4 先检查服务，再决定是否启动

在 B 做只读检查：

```bash
curl --fail --silent --show-error --max-time 5 --noproxy '*' \
  http://127.0.0.1:11434/api/version
```

接口成功时，不要再启动第二个服务。连接失败、且本地模型已按指南安装时，才在 A 运行：

```bash
bash scripts/serve-local-llm.sh
```

保持 A 打开，回到 B 重做 curl 检查。端口被占用时先确认服务归属，不杀未知进程，也不要把监听地址改成 `0.0.0.0`。

若尚未安装 Ollama 或缺模型，先按 [本地 LLM 安装与固定基线指南](local-llm.md) 准备。该版本使用 Ollama 0.33.3 与测试别名 `turnforge-test:qwen3-4b-v1`，完整 digest 以 [锁文件](../dev/local-llm.lock.json) 为准。API 可达不等于模型/版本匹配；后续 Lab 会预检。本文不自动安装、下载或配置开机启动。

源码入口：[服务脚本](../scripts/serve-local-llm.sh)、[学习课件](../src/learning.rs)。

完成标准：能打开 `learn`，能指出仓库根目录，并知道 Ollama 与 Harness 是两个生命周期独立的进程。

## 1. 只聊天：确认整条链路跑通

终端 B：

```bash
bash scripts/harness-lab.sh --case chat --auto
echo $?
```

成功路径应看到模型回答、`[run] Completed`、`[PASS]`，最后退出码 `0`。这里的 PASS 检查正常结束、最终回答非空、没有工具调用，不评价回答的语义质量。

如果预检报版本、digest 或模型缺失，回到第 0 步核对；不要为通过而修改锁文件或切换云端模型。模型结果失败则保留首次错误，按第 7 步分类。

源码入口：[启动脚本](../scripts/harness-lab.sh)、[Lab 准备与验收](../src/lab.rs)。

完成标准：独立跑出一个 chat 的 PASS 与退出 `0`，同时能说出它没有验证工具或回答质量。

## 2. 单步读文件：理解 Agent 循环

```bash
bash scripts/harness-lab.sh --case read --lesson loop
```

Lab 创建临时 `marker.txt`，其中保存本次生成的 marker，随后在首次模型请求前暂停。

### 调试快捷键

每次输入一条并按回车，等待新现场；不要预先粘贴多行 `n`。

| 输入 | 含义 |
|---|---|
| `i` | 查看完整快照，不推进 |
| `n` 或直接回车 | 放行下一语义动作 |
| `l` | 查看当前课程提示 |
| `c` | 继续运行 |
| `p` | 请求运行中的 Agent 在下一个安全边界暂停 |
| `q` | 取消 |

### 按现场操作，不按固定次数操作

1. 首次暂停输入 `i`：应看到 `point.kind = before_model`、一条 User 历史、`next.kind = model`、空 `pending_calls`。system 单独存放，不算进 messages 条数。
2. 输入 `n` 请求一次模型，再 `i`：如果 `next.kind = tool`，Assistant 已提出工具调用，但工具尚未执行。模型可能先列目录，也可能直接读文件。
3. 当 `next.kind = tool`，记录工具名、参数与调用 ID，再按一次 `n`：等到 `AfterTool`，检查新增的 Tool 消息。AfterTool 只表示已返回并提交结果，不保证 `output.status` 是成功。
4. 继续依据 `next`：`tool` 表示还有工具；`model` 才表示下一次模型请求；`finish` 表示结束动作。
5. 成功读取后，记录工具返回的实际 marker。最终 Assistant 应包含该 marker，且不再请求工具。最后还要按一次 `n` 释放 Finish，才真正结束。

返回 shell 后立即检查：

```bash
echo $?
```

成功路径要求 PASS 与 `0`。若模型没有调用读工具，仍放行 Finish，让 Lab 报出实际 FAIL；不能把模型“说读过了”记作成功。暂停数、调用 ID、工具顺序都不是固定剧本。

打开 [Agent 实现](../src/agent.rs)，找：

`run_debug → run_loop → model.complete → validate_assistant → commit Assistant → tools.execute → commit Tool → 下一轮`。

只有 Agent 拥有并追加会话历史；工具返回结果，由 Agent 关联调用 ID 后提交。模型轮次只统计模型请求，`n` 还可以放行单个工具或 Finish。

用不依赖 Ollama 的确定性测试固定理解：

```bash
cargo test --locked --test agent model_tools_model_and_second_user_turn_have_ordered_history -- --exact --nocapture
```

预期 `running 1 test`、`1 passed`。`0 tests` 是过滤器没有命中，不算通过。

完成标准：能解释“按一次 n”为什么不等于“模型运行一轮”，并记录初始、工具前、工具后、最终回答四个现场。详见 [循环教程](tutorial/01-loop.md)。

## 3. 追踪上下文：模型究竟看到了什么

```bash
bash scripts/harness-lab.sh --case read --lesson context
```

仍用 `i`、`n`，本次聚焦四个字段：

- `system`：系统提示。
- `messages`：已提交历史。
- `tools`：经过本次宿主授权、对模型可见的工具定义。
- `pending_calls`：已经提出、尚未得到结果的调用。

在工具前记录一次实际调用 ID；工具后检查新增 Tool 消息：

`Assistant.tool_calls[].id → Tool.call_id`：两者应是本次同一个调用 ID。

工具执行前没有读取结果；执行后结果进入历史，当前调用从 pending_calls 移除。该批工具全部闭合后，下一次模型请求才能利用这些结果。不要用猜测的 `call-1` 或单纯消息计数代替实际配对。

源码按顺序读：[消息结构](../src/message.rs) → [ModelRequest](../src/model.rs) → [OpenAI 兼容适配](../src/openai.rs)。

重点区分：ModelRequest 临时只读借用历史；DebugSnapshot 是独立副本，不能改写 Agent；屏幕流式增量可能尚未成为完整消息。快照 JSON 也不是原始 HTTP payload：适配器还会翻译 system、调用 ID、工具参数与结果。

验证流式参数拼接、真实写入、工具结果回传模型的测试：

```bash
cargo test --locked --test cli_http fragmented_tool_arguments_round_trip_to_real_file_and_model -- --exact --nocapture
```

应精确运行 1 个测试。它使用可控本地 HTTP 服务，不调用 Ollama。

完成标准：留下真实 ID 配对，能区分“屏幕文本”“已提交消息”“模型 HTTP 请求”。详见 [上下文教程](tutorial/02-context.md)。

## 4. 写文件：亲眼验证工具副作用

终端 B：

```bash
bash scripts/harness-lab.sh --case write --lesson tools
```

复制启动 banner 中“临时工作区”后的完整路径，只复制路径。每次运行不同，不能复用旧路径。

终端 C 单独执行下面这一行，等待输入后，粘贴路径并按回车：

```bash
read -r turnforge_lab_dir
```

再确认目标尚不存在：

```bash
test -d "$turnforge_lab_dir" \
  && test ! -e "$turnforge_lab_dir/result.txt" \
  && printf '%s\n' '文件尚未创建'
```

回到 B，按实际 `next` 单步到准备执行 `write_file`，用 `i` 核对预期参数：

```json
{"path":"result.txt","content":"turnforge-lab-write-ok"}
```

`./result.txt` 也可作为等价工作区路径理解；内容必须精确且无末尾换行。若参数错误，保留现场并 `q` 取消，不要手工修文件以制造 PASS。

在工具预览处先不执行。回到 C 再运行一次“不存在”检查，证明模型提议尚未造成写入。然后在 B 只按一次 `n`，等待 write 返回并停在 `AfterTool`。确认 Tool 结果 `status = ok`、字节数 `22`，保持暂停，不要继续到结束。

此时在 C 读取真实文件：

```bash
wc -c < "$turnforge_lab_dir/result.txt"
od -An -tx1 "$turnforge_lab_dir/result.txt"
printf '%s' 'turnforge-lab-write-ok' \
  | cmp - "$turnforge_lab_dir/result.txt" \
  && printf '%s\n' '磁盘内容完全匹配，且无末尾换行'
```

预期 22 字节，末字节为 `6b`（k），不是换行 `0a`，并显示精确匹配提示。比较命令不修改目标文件。

再回到 B，按实际 next 推进并释放 Finish，检查 PASS 和 shell 退出 `0`。正常退出后临时目录会被清理，因此必须在暂停时检查磁盘；目录清理不是写入被事务回滚。

源码入口：[注册与权限检查](../src/tools/mod.rs)、[文件操作与提交点](../src/tools/files.rs)。注册工具不等于授权，隐藏工具也不能省去执行端权限检查。

```bash
cargo test --locked --test tools registry_enforces_capabilities_and_rejects_duplicates -- --exact --nocapture
```

应精确运行 1 个测试。`--case write` 选择本次实验的写权限；`--lesson tools` 只添加教学提示，不赋予权限。Lab 不授权 shell。工作区路径检查也不是 OS 级 sandbox。

完成标准：用同一次运行的证据区分“模型说要写”“宿主允许写”“工具成功”“磁盘正确”。详见 [工具教程](tutorial/03-tools.md)。

## 5. 暂停与取消：理解调试控制

```bash
bash scripts/harness-lab.sh --case read --lesson control
```

第一组实验：首次暂停连续两次输入 `i`，确认暂停 ID、消息数、下一动作相同；输入 `n` 观察一个语义动作，再输入 `c` 继续到结束。成功路径检查 PASS 和退出 `0`。

第二组实验：重新运行上面的命令，等首次暂停输入 `q`，返回 shell 后立即：

```bash
echo $?
```

在预检成功、输入输出正常时应为 `130`，表示取消，不是 PASS。此时没有推进模型动作，但之前的服务预检仍有本地 HTTP 请求，不能说完全没有网络访问。

Pause 在安全边界停下，不是 Rust 行级断点，也不强制截断正在执行的动作。`c` 后再手动 `p` 可能赶不上小模型，不能靠手速判断正确性；运行可控时序测试：

```bash
cargo test --locked --test debugging pause_during_a_model_waits_for_its_complete_response_boundary -- --exact --nocapture
```

应精确运行 1 个测试。源码入口：[DebugController / DebugSession](../src/debug.rs)、[输入命令绑定](../src/debug_input.rs)。

完成标准：能解释 Inspect 不推进、Step 放行一个动作、Continue 不改变权限、取消需等待收尾。取消不自动撤销已经完成的文件写入；预送命令也不能提前放行未来暂停。详见 [控制教程](tutorial/04-control.md)。

## 6. 输入关闭后：程序如何结束

先运行自动模式：

```bash
bash scripts/harness-lab.sh --case chat --auto --lesson io </dev/null
echo $?
```

成功时 PASS、退出 `0`。auto 不创建交互输入读取器，不依赖 stdin；这是自动响应暂停事件，不是“把 EOF 当继续”。

再运行交互模式：

```bash
bash scripts/harness-lab.sh --case chat --lesson io </dev/null
echo $?
```

在预检成功、输出正常时，应取消并退出 `130`：交互模式把 EOF 转为取消。若预检、输入或输出出错，可能是 `1`，要按真实错误排查。

打开 [CLI 宿主](../src/main.rs) 和 [输出实现](../src/output.rs)，寻找：

`run` 推进 Agent，结束时通知 `run_done`；`control` 停止读命令并释放自己的 Sender；`writer` 在所有 Sender 释放后排空输出。`tokio::join!` 等待三者都结束。

`async` 不自动新建线程，Agent 完成也不等于宿主已交付完输出。用保持 stdin 开放的真实进程测试验证正常退出：

```bash
cargo test --locked --test lab_cli lab_default_read_shortcuts_pause_inspect_and_finish_with_stdin_open -- --exact --nocapture
```

应精确运行 1 个测试。完成标准：记录两种 stdin 模式的退出码，能解释为什么 run 结束后还必须关闭控制与输出链。详见 [生命周期教程](tutorial/05-io.md)。

## 7. 回归：模型说完成，不等于任务成功

先用可控模型制造“口头成功”：

```bash
cargo test --locked --test lab_cli lab_read_completed_without_a_tool_is_not_a_pass -- --exact --nocapture
```

预期运行 1 个测试并通过。原因是被测 CLI 应失败：模型声称读过文件，却没有调用工具；Lab 正确拒绝它，检查拒绝行为的测试因此为绿色。

阅读 [独立验收逻辑](../src/lab.rs) 与 [测试夹具](../tests/lab_cli.rs)，区分四层事实：

1. Completed：模型不再请求工具，Agent 自然完成。
2. ToolOutput 成功：工具完成指定操作，但操作本身仍可能不符合任务目标。
3. Lab PASS：独立验收历史和磁盘事实；预期在请求前就已定义，不来自模型自评。
4. 退出 `0`：输入、输出、收尾也都成功。屏幕上出现 PASS 不能代替检查退出码。

### 7.1 默认确定性回归

```bash
cargo test --locked --all-targets
```

它不要求启动 Ollama。报告为 ignored 的真实模型用例没有执行，不能计作通过。

### 7.2 显式真实模型回归

确认固定本地基线正常后执行：

```bash
cargo test --locked --test local_llm \
  -- --ignored --test-threads=1 --nocapture
```

记录结果；需要 release 对照时，按[分章回归教程](tutorial/06-regression.md#step-5执行两层回归而不是只有真实模型冒烟)继续，先结束当前测试并处理失败。

两层测试不能互相替代。真实模型验证服务、协议、工具与模型的真实闭环，但不穷尽异常时序。`--test-threads=1` 只限制当前测试进程，请勿在另一个终端同时运行 Lab 或模型测试。

源码入口：[本地模型测试](../tests/local_llm.rs)。完整运行时变更门禁还需按 [CI 配置](../.github/workflows/ci.yml) 执行格式、Clippy、debug/release 测试和构建；这里不把模型冒烟当作完整上线检查。

### 7.3 失败先保留，再分类

| 现象 | 首先检查 | 不应采取的处理 |
|---|---|---|
| 不可达、缺模型、基线不符 | 服务归属、版本、完整 digest | 跳过预检、改 lock、偷偷换云端 |
| 协议错误、消息不闭合 | 首次错误、调用/结果配对、确定性测试 | 只看最后一段回答 |
| 文件或 marker 错误 | 参数、工具结果、独立磁盘事实、最终回答 | 把 expected 改成本次输出 |
| 超时、输出失败 | 资源争抢、服务日志、宿主收尾 | 并发重试、无限增加超时 |

保留首次失败的命令、退出码、合成输入与错误。修复后区分定向复验和完整重跑，不反复重跑到一次绿色就抹去失败。不要上传凭据、业务文件或真实会话日志。

完成标准：能解释“被测 CLI 失败、测试反而成功”，并分别记录两层回归；未运行的明确写“未执行”。详见 [回归教程](tutorial/06-regression.md)。

## 8. 综合练习：自己写一个权限测试

本练习不改生产功能、不调用 Ollama、不授予 shell。先预测：写权限关闭时，已注册的写工具不可见且执行返回 `permission_denied`，磁盘不产生文件；权限开启时，应写入预先指定的精确内容。

先检查工作区，确认未提交内容归属和已有分支，再新建自己的学习分支：

```bash
git status --short
git switch -c codex/study-permission-contract
```

分支已存在时先检查，再决定复用，不强制切换或清理。
打开[综合练习第 2 步](tutorial/07-capstone.md#step-2在自己的学习分支准备测试文件)，用编辑器新建 `tests/learning_permissions.rs`，复制该节完整 Rust 示例并保存。已有同名文件时先阅读，不直接覆盖。
**这是需要你创建的练习文件，不是仓库预置的 test target。先保存文件，再执行：**

```bash
cargo test --locked --test learning_permissions learning_write_permission_controls_visibility_and_effects -- --exact
```

必须看到 1 个测试通过。缺 test target 时检查文件路径；`0 tests` 时检查函数名与过滤器。
沿 [ToolRegistry](../src/tools/mod.rs) 的 `register → definitions → execute` 和 [FileTool](../src/tools/files.rs) 阅读：注册不等于授权；隐藏工具不是唯一防线；磁盘由测试独立检查。这个小测试不包含模型请求、Agent 调度或宿主生命周期。

按综合练习第 5 步，仅把自己练习文件中的预期错误码临时改为 `unknown_tool`，确认同一测试失败，再用编辑器恢复 `permission_denied`，确认通过。不要改生产代码迎合错误断言。

```bash
cargo fmt --all -- --check
cargo test --locked --test learning_permissions
git status --short
git diff -- tests/learning_permissions.rs
```

未跟踪的新文件可能不出现在 `git diff`，须结合 `git status` 和编辑器查看。是否提交练习由你决定；不要提交 target、临时目录或真实会话日志。
完成标准：留下自己的测试与解释，说明权限如何影响可见性、执行和磁盘事实，以及本测试没有覆盖哪些层。

## 101 完成清单

- [ ] 能从仓库根目录打开 `learn`，不依赖手找 target 二进制。
- [ ] chat 有实际 PASS 与退出 `0` 记录。
- [ ] read 能按 `point / next` 单步，解释谁提交 User、Assistant、Tool。
- [ ] 留下一组真实工具调用 ID 与结果 call_id 的配对。
- [ ] 在同一次 write 运行中观察到“工具前不存在、工具后 22 字节精确匹配”。
- [ ] 能区分 Inspect、Step、Continue、Pause、Cancel，知道取消不回滚已完成副作用。
- [ ] 比较过 auto 与交互 EOF，能解释 run/control/writer 的结束关系。
- [ ] 能区分 Completed、工具成功、PASS、进程退出 `0`。
- [ ] 确定性与真实模型回归分别记录，未执行项不记为通过，首次失败不被覆盖。
- [ ] 亲手创建权限测试，观察正确预期通过、教学错误预期失败、恢复后通过。

每次实验在自己的笔记中记录：checkout commit、运行命令、暂停 point/next、消息和 call ID、磁盘事实、PASS/FAIL 与退出码、对应源码、已证明与未覆盖的内容。首次失败与后续复验分开保存，不把业务日志或凭据提交到仓库。

完成 101 后，再进入 [系统架构](architecture.md) 和 [11 个模块的架构/设计索引](README.md)。101 覆盖当前已实现的核心执行链；它不是所有未来 Harness 能力的完成声明，也不能替代某次修改的风险评估与验证。
