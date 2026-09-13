# 本地 LLM：安装、运行与冒烟测试

本指南用独立的 Ollama 服务，为 Turnforge 提供本机 Chat Completions SSE API。
它不改变 Harness 的模型接口，不引入模型权重到 Git，也不安装开机启动服务。
返回[文档索引](README.md)；协议边界见[模型设计](modules/model/design.md)。

## 1. 两层测试，各自负责什么

| 层次 | 入口 | 能证明什么 | 不能证明什么 |
|---|---|---|---|
| 确定性回归，默认 CI | `cargo test --locked --all-targets`；[现有测试](../tests/cli_http.rs)等 | 人工构造的合法/异常协议、权限、取消、真实 CLI 与文件/进程合同 | 模型是否会选择合适工具、某个真实服务的兼容性 |
| 真实本地模型冒烟，显式启用 | [local_llm.rs](../tests/local_llm.rs) | 本机服务 → SSE → CLI → 工具 → 模型继续回答的真实闭环 | 全量协议覆盖、复杂 coding 能力、所有机器上稳定通过 |

真实模型测试默认 `#[ignore]`，不会自动安装、下载或启动服务；普通 CI 不依赖模型。
保留确定性回归作为门禁，不能用小模型冒烟替代它。`temperature=0` 和固定 seed
用于减少采样变化，不保证跨硬件、runtime 版本或输入变化时逐 token 一致。
失败不能靠反复重跑到绿来消除，应区分 Harness 故障、服务/基线漂移与模型行为变化。

## 2. 固定的测试基线

- Runtime：**Ollama 0.33.3**；基线信息见 [local-llm.lock.json](../dev/local-llm.lock.json)。
- 源模型：**`qwen3:4b-instruct-2507-q4_K_M`**，约 4.02B 参数，Q4_K_M，下载约 **2.5 GB**。
  这是权重包大小，不是推理内存上限；KV cache、上下文与推理缓冲还会占用内存。
  [官方 Ollama tag](https://ollama.com/library/qwen3:4b-instruct-2507-q4_K_M)
- 测试别名：**`turnforge-test:qwen3-4b-v1`**，由仓库 [Modelfile.local](../dev/Modelfile.local) 创建：
  context 8192、最多生成 1024 tokens、temperature 0、seed 42。
- API：**`http://127.0.0.1:11434/v1`**，无 key；不使用云端 fallback。

选择这个精确变体，是因为 Qwen 官方明确它只有非思考模式，支持工具调用，
不需要客户端额外发送关闭思考的参数。泛化的 `qwen3:4b` 不是这个精确变体，不能替换；
family 页面上的 thinking 标签也不能代替精确模型卡。
[Qwen 官方模型卡](https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507)

Turnforge 复用现有 OpenAI-compatible provider；Ollama 支持该接口的 streaming、tools
和 `stream_options.include_usage`。不需要 Ollama 专用 Rust SDK。
当前 Harness 不支持 reasoning 块，选非思考模型不意味着已经实现 reasoning 支持。
[Ollama 兼容接口](https://docs.ollama.com/api/openai-compatibility)

源 tag 和测试别名都可能被重新指向；**名称不是不可变版本**。lock 保存完整 digest，
真实模型测试会比较正在运行的服务版本及已安装测试别名的 digest；不匹配就失败。
source digest 用于基线溯源，不是测试每次重新下载源模型的指令。
不要把测试 tag 命名为 `local` 或 `cloud`：Ollama 0.33.3 会把 `:local` / `:cloud`
解释为来源选择后缀，而非普通 tag。[固定版本的来源解析实现](https://github.com/ollama/ollama/blob/v0.33.3/internal/modelref/modelref.go#L99)

## 3. macOS 隔离安装

以下使用官方 macOS standalone bundle，在用户数据目录安装固定版本。
Linux 需使用对应平台的官方 release artifact 并独立校验 SHA；不要运行 macOS 包。
Rust 工具链安装沿用[项目快速开始](../README.md#快速开始)。

从仓库根目录执行。命令拒绝覆盖已有版本目录，先校验 SHA-256 再解压；不使用 `curl | sh`：

```sh
(
  set -eu
  turnforge_data_dir="${XDG_DATA_HOME:-$HOME/.local/share}/turnforge"
  turnforge_install_dir="$turnforge_data_dir/ollama/0.33.3"
  if [ -e "$turnforge_install_dir" ]; then
    printf '%s\n' '目标版本目录已存在，请检查并复用，不覆盖。' >&2
    exit 1
  fi
  turnforge_download_dir=$(mktemp -d)
  turnforge_archive="$turnforge_download_dir/ollama-darwin.tgz"
  curl --fail --location --proto '=https' --tlsv1.2 \
    'https://github.com/ollama/ollama/releases/download/v0.33.3/ollama-darwin.tgz' \
    --output "$turnforge_archive"
  printf '%s  %s\n' \
    '342db03df80bb9db84ff64246031bd5f70c09b59ff52fa5cc9aaae3476cc4a9d' \
    "$turnforge_archive" | shasum -a 256 -c -
  mkdir -p "$turnforge_install_dir"
  tar -xzf "$turnforge_archive" -C "$turnforge_install_dir"
  printf '安装位置：%s\n下载缓存：%s\n' "$turnforge_install_dir" "$turnforge_download_dir"
)
```

下载与校验依据：[官方 v0.33.3 release](https://github.com/ollama/ollama/releases/tag/v0.33.3)。
解压目录直接包含 `ollama` 及其配套程序/动态库，**保留整包结构，不要只复制单个二进制**。
下载缓存留在上面打印的临时目录，确认后可自行清理该具体目录。
此流程不修改 shell profile，不注册后台守护程序，也不要求把 Ollama 加入全局 PATH。

## 4. 服务启动、下载与创建测试模型

在终端 A 的仓库根目录启动前台服务：

```sh
bash scripts/serve-local-llm.sh
```

[启动脚本](../scripts/serve-local-llm.sh) 的 owner 是当前终端，最终 `exec ollama serve`：

- 只监听 `127.0.0.1:11434`，设置 `OLLAMA_NO_CLOUD=1`。
- 模型数据放在 `${XDG_DATA_HOME:-$HOME/.local/share}/turnforge/models`，不在仓库。
- 单并发、最多驻留一个模型、默认 8192 context、空闲驻留 5 分钟。
- 清除常见云模型 key 和 HTTP/HTTPS/ALL proxy 环境变量，设置 `NO_PROXY=*`。
- 默认使用隔离安装路径；已有合适二进制时可用 `TURNFORGE_OLLAMA_BIN` 指定。
  这只替换程序路径，不跳过真实测试的 runtime 版本检查。

如果端口被占用，先确认已有服务归属；不要直接终止未知进程，也不要改成 `0.0.0.0`。
loopback 无鉴权服务不是多租户安全边界，本机其它进程仍可访问。

在终端 B 的仓库根目录设置客户端，然后下载和创建模型：

```sh
turnforge_data_dir="${XDG_DATA_HOME:-$HOME/.local/share}/turnforge"
turnforge_ollama_bin="${TURNFORGE_OLLAMA_BIN:-$turnforge_data_dir/ollama/0.33.3/ollama}"
export OLLAMA_HOST='127.0.0.1:11434'
export NO_PROXY='*' no_proxy='*'

"$turnforge_ollama_bin" pull qwen3:4b-instruct-2507-q4_K_M
"$turnforge_ollama_bin" create turnforge-test:qwen3-4b-v1 -f dev/Modelfile.local
"$turnforge_ollama_bin" show turnforge-test:qwen3-4b-v1 --modelfile
curl --fail --silent --show-error --noproxy '*' http://127.0.0.1:11434/api/version
curl --fail --silent --show-error --noproxy '*' http://127.0.0.1:11434/api/tags
```

`pull` 需要联网下载；服务禁用云端推理不等于禁止下载权重。下载需要可直连官方模型仓库，
脚本不会继承代理或代替用户修改网络配置。模型下载后，以上本地推理不需要云端 key。
`create` 创建带固定参数的本地模型，不是训练或微调；若这个别名已存在，先确认可以替换它。
逐项核对 `/api/version`、`/api/tags` 中的完整 digest 与 lock；不要因不匹配就自动更新 lock。
Ollama 兼容 API 不直接接收 `num_ctx`，因此参数由 Modelfile 负责。
[Modelfile 说明](https://docs.ollama.com/modelfile)

## 5. 运行测试和手工体验

先跑默认回归，再显式跑真实模型用例，禁止并发争抢单模型服务：

```sh
cargo test --locked --all-targets
cargo test --locked --test local_llm -- --ignored --test-threads=1 --nocapture
cargo test --locked --release --test local_llm -- --ignored --test-threads=1 --nocapture
```

这里的 `--ignored` 显式选择真实模型用例，`--test-threads=1` 让同一测试进程串行执行；
它不防止两个终端同时发起测试，因此 debug/release 两条命令也要依次等待完成。
查看用例而不调用模型可运行 `cargo test --locked --test local_llm -- --list`。

### 用例矩阵

实现与断言以 [local_llm.rs](../tests/local_llm.rs) 为准，以下 ID 用于报告关联，不是额外测试：

| ID / 测试符号 | 输入与权限 | 独立验收依据 |
|---|---|---|
| LLM-001 / `local_llm_plain_text_stream_completes` | 临时空目录；一句问候；只读 | 存在非空文本增量和最终回答；没有工具调用；正常结束 |
| LLM-002 / `local_llm_reads_unknown_file_marker` | 测试预写 `marker.txt`；只读；prompt 不包含临时生成的 marker | 成功的 `read_file` 结果和最终回答都包含预写 marker；不是模型猜中固定答案 |
| LLM-003 / `local_llm_writes_file_and_finishes` | 临时空目录；仅显式开启 `--allow-write` | 独立读取磁盘 `result.txt`，精确等于 `turnforge-local-write-ok`，无末尾换行；成功的 `write_file` 结果路径和字节数匹配；随后模型正常结束 |

共同断言包括：

- 推理前检查 Ollama 版本和测试模型完整 digest；不可达、缺模型或漂移都失败，不自动跳过。
- CLI 成功退出；stdout 是 NDJSON；恰有一对 `run_started` / `run_finished`，首尾有序且 outcome 为 `completed`。
- 每个工具开始事件对应已提交的调用，每个结果关闭已开始的调用；新 Assistant 之前上一批调用已闭合。
  调用 ID 只要求同一 Assistant 消息内唯一，不固定模型生成的 ID 或工具顺序。
- 最后提交的是没有待执行工具、文本非空的 Assistant，不把仅有流式增量当作完整回答。

测试启动真实 Turnforge binary，固定 endpoint/model，清空子进程继承环境并绕过代理，
只使用合成数据与临时工作区，永不传 `--allow-shell`。每次 CLI 最多 4 个模型步骤、
每次请求 90 秒，CLI 整体等待上限 300 秒；超时后尝试 kill 并限时等待回收。
测试还检查事件闭合、工具调用与结果配对、最后一条完整 Assistant 和 CLI 退出状态。
这些断言不是逐字比较问候或解释文本；真实模型仍可能不遵循提示或因资源争抢超时。

手工体验可在临时空工作区运行。显式指定 endpoint/model，移除云端凭据和代理，
避免环境里的 `OPENAI_API_KEY` 回退到本地请求；命令不会修改调用者的这些环境变量：

```sh
turnforge_smoke_dir=$(mktemp -d)
env -u TURNFORGE_API_KEY -u OPENAI_API_KEY -u ANTHROPIC_API_KEY -u OLLAMA_API_KEY \
  -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY -u http_proxy -u https_proxy -u all_proxy \
  NO_PROXY='*' no_proxy='*' \
  cargo run --locked -- run '用一句中文问候，不调用工具。' \
    --model turnforge-test:qwen3-4b-v1 --base-url http://127.0.0.1:11434/v1 \
    --workspace "$turnforge_smoke_dir" --max-steps 4 --request-timeout 90 --json
```

手工命令只有逐请求超时，没有测试 helper 的 300 秒整体 deadline。
`--json` 输出包含 prompt、模型文本、工具参数和结果；真实工作区内容仍需按敏感信息处理。
测试失败的诊断输出只基于合成内容；不要把生产会话改成固定 fixture 提交到 Git。

## 6. 停止、复用与升级

- **停止服务**：回到终端 A 按 Ctrl-C；不需要 `brew services`、`launchctl` 或开机启动配置。
- **仅释放模型内存**：终端 B 执行 `"$turnforge_ollama_bin" stop turnforge-test:qwen3-4b-v1`。
  这不是停止 API 服务，也不删除磁盘模型；可用 `"$turnforge_ollama_bin" ps` 查看驻留状态。
  [Ollama 资源生命周期 FAQ](https://docs.ollama.com/faq)
- **再次运行**：重新启动前台脚本即可；已经匹配 lock 的模型不需要每次 `pull/create`。
- **升级基线**：显式选择新 runtime/tag/量化或参数；重新核验官方安装包 SHA，审查
  Modelfile/template 与完整 digest，更新 lock 和依赖它的脚本/测试/文档，然后运行两层测试。
  不自动追踪 `latest`，不跳过 drift 检查，不以模型名字相同作为“未升级”的证据。

lock 记录的是模型清单的完整 digest，不是下载页面显示的短 ID，也不是唯一权重 blob 的 hash。
它不是包管理器，不会自动把漂移的上游 tag 恢复到旧 digest；复现依赖可用的匹配模型缓存或
经核验的匹配来源。模型权重、个人路径、API key 和运行日志不入库。
当前指南说明操作和测试合同；某次实测通过不等于后续所有设备、升级版本或任务都已获得兼容认证。

## 7. 失败分类与维护

| 观察到的失败 | 首先检查 | 不应采用的处理 |
|---|---|---|
| 连接失败、缺模型 | 前台服务是否运行、端口归属、缓存中是否存在锁定的别名 | 静默跳过、切到云端模型、终止未知监听进程 |
| runtime / digest drift | `/api/version`、`/api/tags` 与 lock 的差异，是否发生了人为升级 | 仅为了通过测试而改 lock 或放宽断言 |
| CLI 非零退出、协议或事件不闭合 | 用例的合成 NDJSON/stderr 诊断；对照 CLI/HTTP 确定性回归和模型设计 | 吞掉错误、仅凭部分文本判断成功 |
| 文件内容或工具选择不符合预期 | 实际磁盘/工具结果、模型行为、prompt 与固定基线 | 更新 expected 为本次模型输出、重复运行直到出现一次成功 |
| 超时或输出捕获失败 | 本机资源争抢、服务日志、子进程回收结果 | 无边界增加超时、并发多次重试 |

测试同时排空 stdout/stderr，每路最多保留 4 MiB；超过上限会失败。失败信息最多展示每路
已捕获内容的末尾 32 KiB，只来自合成用例；完整生产会话和个人日志不应提交到仓库。
修复后要区分定向复验与新一轮完整回归，保留首次失败原因；新增用例需说明独立 oracle 和权限边界。

已完成运行的环境、输入指纹和结果见[本地 LLM 回归记录（2026-09-12）](verification/local-llm-2026-09-12.md)。
该记录是历史证据，不代表此后每个提交都运行过真实模型，也不把默认 CI 的 ignored 计作通过。
