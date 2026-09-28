# CLI 使用参考

本页负责 `run`、`tools` 和 `learn` 的日常使用，以及 `run` / `debug` 共用的配置方式。
第一次启动看 [101](101-user-guide.md)；固定合成场景看 [Lab 参考](harness-lab.md)；
带暂停 ID 的控制和自动化看[调试参考](debugging.md)。这里不重复课程或调试协议。

以下命令在仓库根目录执行。`cargo run --locked --` 构建并运行当前 Turnforge，后面才是它的参数。
参数默认值、环境优先级和校验规则的完整合同见 [CLI 设计](modules/cli/design.md#公共入口)。

## 查看命令与工具

```sh
cargo run --locked -- --help
cargo run --locked -- run --help
cargo run --locked -- tools
```

`tools` 不调用模型，输出当前授权下可见的工具定义 JSON 数组。
默认只有读能力；它不是一次 Agent 运行，也不证明模型能正确调用这些工具。

## 配置模型服务

`run` / `debug` 使用支持 Chat Completions function tools 的模型服务。
替换下面两个占位值；URL 填 API 根路径，不要再加 `/chat/completions`：

```sh
export TURNFORGE_MODEL='your-model-id'
export TURNFORGE_BASE_URL='https://your-provider.example/v1'
```

在自己的终端安全设置 `TURNFORGE_API_KEY`，不要把 key 写进仓库、示例或命令参数。
未设置非空 `TURNFORGE_API_KEY` 时会回退到非空 `OPENAI_API_KEY`。
程序不会读取 Codex、Claude Code 凭据，也没有自动登录。

没有显式 URL 时使用 `https://api.openai.com/v1`，但**不会自动选择模型**。
命令行的 `--model` 和 `--base-url` 可以覆盖对应环境配置；没有 API key flag。
参数绑定和合法 URL 边界以 [CLI 设计](modules/cli/design.md#公共入口)为准。

本地无鉴权服务使用 loopback HTTP，并移除继承的云端 key：

```sh
unset TURNFORGE_API_KEY OPENAI_API_KEY
```

Ollama 的固定模型和 URL、安装及服务生命周期见[本地 LLM 指南](local-llm.md)。
把其中基线的模型名和 API URL 用作上面的环境值即可运行自己的 prompt。
`lab` 固定使用本地场景配置，`learn` 不调用模型；它们不继承这组 `run` 配置。

## 工作区与授权

确认选定工作区没有不应发给模型的内容。默认没有秘密文件过滤器；
模型读到的文件、prompt 和工具结果会发送给配置的服务。

默认只读地运行自己的任务：

```sh
cargo run --locked -- run '读取 README.md，概括项目结构' --workspace .
```

一个 `run` 只接受一个用户 turn，但可能发起多次模型请求和工具调用；不是持久化交互会话。
模型自然结束不等于任务已完成，仍需检查答案和实际文件。

需要修改文件时才添加 `--allow-write`，并先确认修改目标和已有内容：

```sh
cargo run --locked -- run '在 README.md 中补充测试说明' --workspace . --allow-write
```

文件写授权只作用于文件工具。工具只接受工作区内相对路径，并进行路径检查；
它不是抵御恶意并发目录替换的 OS sandbox。详细路径与副作用合同见[文件工具设计](modules/filesystem-tools/design.md)。

`--allow-shell` 是另一个独立授权：命令拥有宿主机权限，可读写工作区外文件、访问网络，
**即使没有 `--allow-write` 也不是只读 shell**。只有接受这一边界时才执行类似命令：

```sh
cargo run --locked -- run '运行测试并解释失败，不修改文件' --workspace . --allow-shell
```

prompt 中的“不修改”不是权限约束。进程清理和隔离限制见 [Shell 设计](modules/shell-tool/design.md)。
任何模式下取消都不撤销已经发生的写入或命令副作用。

## stdin、输出与运行限制

`run -` 从 stdin 读取 prompt，可以与 NDJSON 输出组合：

```sh
printf '%s' '读取 Cargo.toml 并解释依赖' | cargo run --locked -- run - --json
```

`debug` 的 stdin 专门用于控制命令，因此不能用 `debug -` 读取 prompt。
读取上限和输入失败行为见 [CLI 设计](modules/cli/design.md#建立运行的顺序)。

按任务配置模型请求次数和超时：

```sh
cargo run --locked -- run '分析项目' --max-steps 10 --request-timeout 120 --tool-timeout 30
```

`--max-steps` 计模型请求，不是工具调用数；`--request-timeout` 覆盖每次请求及流读取，
`--tool-timeout` 只配置 shell。完整范围和默认值仍由 [CLI 配置合同](modules/cli/design.md#公共入口)维护。

`--json` 的 stdout 每行一个事件，可能包含敏感的 prompt、模型文本、工具参数与结果。
普通文本流也可能只是暂态结果；自动化应结合退出码与完整终态判断。
输出阻塞或断开可能让终态缺失，不能只等待一个必然到达的 `RunFinished`。
事件与失败优先级见 [CLI 输出合同](modules/cli/design.md#事件与输出合同)，
退出码见 [CLI 结束合同](modules/cli/design.md#joinsignal-与错误优先级)。

## 查看离线课程

```sh
cargo run --locked -- learn
cargo run --locked -- learn loop
```

`learn` 输出路线或单课卡片，不创建模型、工具注册表或 Agent。
它不是完整教材；课程顺序和学习方法见[学习路线](harness-learning.md)，逐步实验只在 `tutorial/` 中维护。
