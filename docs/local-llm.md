# 本地 LLM：安装、基线与服务生命周期

本指南用独立的 Ollama 服务，为 Turnforge 提供本机 Chat Completions SSE API。
它不改变 Harness 的模型接口，不引入模型权重到 Git，也不安装开机启动服务。
本页只负责本地模型环境，不编排 Harness 学习或回归流程。
首次跑通看 [101 用户指南](101-user-guide.md)，测试命令与验收看[测试参考](testing.md)，
返回[文档索引](README.md)；协议边界见[模型设计](modules/model/design.md)。

## 1. 使用前提

安装、下载和创建别名是显式的环境操作，Lab 与回归测试不会自动执行它们。
已有匹配环境时直接复用，不需要每次重新安装或下载。首次仓库/Rust 准备见
[101 用户指南](101-user-guide.md)；两层测试的分工见[测试参考](testing.md)。

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
Rust 工具链与仓库准备见 [101 用户指南](101-user-guide.md)。

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

测试操作已统一到[真实模型回归](testing.md#真实模型回归)；首次体验见
[101 用户指南](101-user-guide.md)，自定义模型与 prompt 见 [CLI 使用参考](cli.md)。
保留本节标题以兼容已有链接，不再维护第二份测试或手工运行流程。

### 用例矩阵

完整矩阵与各入口的断言边界已移至[测试参考：用例矩阵](testing.md#用例矩阵)。

## 6. 停止、复用与升级

- **停止服务**：回到终端 A 按 Ctrl-C；不需要 `brew services`、`launchctl` 或开机启动配置。
- **仅释放模型内存**：终端 B 执行 `"$turnforge_ollama_bin" stop turnforge-test:qwen3-4b-v1`。
  这不是停止 API 服务，也不删除磁盘模型；可用 `"$turnforge_ollama_bin" ps` 查看驻留状态。
  [Ollama 资源生命周期 FAQ](https://docs.ollama.com/faq)
- **再次运行**：重新启动前台脚本即可；已经匹配 lock 的模型不需要每次 `pull/create`。
- **升级基线**：显式选择新 runtime/tag/量化或参数；重新核验官方安装包 SHA，审查
  Modelfile/template 与完整 digest，更新 lock 和依赖它的脚本/测试/文档，然后执行[两层回归](testing.md)。
  不自动追踪 `latest`，不跳过 drift 检查，不以模型名字相同作为“未升级”的证据。

lock 记录的是模型清单的完整 digest，不是下载页面显示的短 ID，也不是唯一权重 blob 的 hash。
它不是包管理器，不会自动把漂移的上游 tag 恢复到旧 digest；复现依赖可用的匹配模型缓存或
经核验的匹配来源。模型权重、个人路径、API key 和运行日志不入库。
当前指南说明环境操作；某次实测通过不等于后续所有设备、升级版本或任务都已获得兼容认证。

## 7. 环境故障与基线维护

| 观察到的失败 | 首先检查 | 不应采用的处理 |
|---|---|---|
| 连接失败、缺模型 | 前台服务是否运行、端口归属、缓存中是否存在锁定的别名 | 静默跳过、切到云端模型、终止未知监听进程 |
| runtime / digest drift | `/api/version`、`/api/tags` 与 lock 的差异，是否发生了人为升级 | 仅为了通过测试而改 lock 或放宽断言 |

环境已匹配但执行失败时，进入[测试失败定位](testing.md#失败定位)，不要把 Harness 或模型行为问题
当成重新安装理由。不得仅为一次失败修改 lock、替换 expected 或自动重试到绿。
