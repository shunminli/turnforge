# 本地 Harness Lab：设计

实现：[src/lab.rs](../../../src/lab.rs)、[launcher](../../../scripts/harness-lab.sh)。
配套：[架构](architecture.md) · [使用](../../harness-lab.md) · [CLI 设计](../cli/design.md)。

## CLI 合同

`turnforge lab [--case chat|read|write] [--auto] [--ollama-url ORIGIN] [--lesson LESSON]`。
缺省 `read`、交互控制、origin `http://127.0.0.1:11434`。课程取值见[学习设计](../learning/design.md)。
不向用户暴露 RunArgs：无自定义 prompt、workspace、model、API key、shell、任意权限或 NDJSON flag；宿主内部构造固定 RunArgs 复用执行路径。
常规 run/debug 的环境优先级不应用于 Lab；课程仅控制文本解释。
`--auto` 是宿主在收到每个暂停后发送 Step，不改库的 DebugCommand/Agent API。

## 预检边界

Lab 固定使用 Ollama `0.33.3` 和模型 `turnforge-test:qwen3-4b-v1`，模型 digest 为
`1cfb1620611f6fdcdbc90e3c32f0b0123b7ac36502c790eb90897941703ec363`。
权重来源、参数和下载流程见[本地 LLM 指南](../../local-llm.md)，本入口不下载或修改它们。

origin 必须是 HTTP、loopback 字面 IP，不能含凭据、query、fragment、非根路径；域名不通过 DNS 解析放行。
预检访问同一 origin 的 `/api/version`、`/api/tags`，比较精确版本、名称和 digest，失败则不开始 Agent 请求。
预检每个请求最多 5 秒、响应最多 64 KiB（包括流式实际计数），错误诊断不回显 body。
请求禁代理和重定向；provider 使用 `OpenAiModel::new_local` 请求固定 origin 下 `/v1/chat/completions`，不设置 Authorization。
参数和凭据不回退到云环境，不因请求错误换端点、换模型或自动重试。

## 场景事实与独立验收

`prepare(&LabArgs)` 返回 `PreparedLab`：独占 TempDir、prompt、allow_write、model、base_url，
以及私有 `Expected` 枚举。`description()` 生成入口说明；`verify(&[Message])` 借用最终 transcript 验收。
基线从 [dev/local-llm.lock.json](../../../dev/local-llm.lock.json) 编译期读取，不另维护另一份运行时锁文件。
每个 PreparedLab 有独占临时目录、在请求前生成的预期事实、固定 prompt 和能力配置。
read 的 marker 存在真实文件，不能把目标 marker 同时塞进 prompt，让模型绕开读取也能回答。
write 的目标内容来自测试输入，必须通过真实文件读回验证，不能只相信 ToolStarted 或模型口头确认。

验收先要求运行自然结束；再根据场景检查已提交 Assistant/Tool 消息和真实文件事实。
工具调用必须与真实工具结果通过 call ID 关联，成功结果不能由文本“成功”代替；最终 Assistant 也必须是已提交消息。
chat 要求最终文本非空且全程没有工具调用，不验证是否符合自然语言“问候”的语义。
read 的 `marker.txt` 内容为 `code = <六个短英文词组成的动态 marker>\n`；由时间值生成以便观察，不是保密随机数。
read 要求匹配路径的成功 `read_file` 结果 `data.text` 包含 marker、最终文本包含 marker，以及磁盘原内容未变。
write 要求 `write_file` 对 `result.txt` 的参数 content 严格为 `turnforge-lab-write-ok`（无 LF），成功结果的
`data.path/bytes` 一致，且独立磁盘读回精确一致。最终 Assistant 非空且无待工具调用；不要求特定确认措辞。
路径匹配按 components 比较，允许 `./`，不允许借不同目标冒充指定文件。
当前 oracle 不枚举全部临时目录来证明“没有其它文件”，也不将 prompt 中每一句要求冒充已验证条件。

## 结束与失败

预检失败不发 RunStarted；输入/输出/信号生命周期仍由 CLI 执行。
Completed 仅是 oracle 的前提；oracle 失败退出非零，不输出 PASS。Cancelled/StepLimit/Failed 不伪装 PASS。
输出故障仍优先于业务成功；不承诺堵塞/断开的 stdout 收到完整报告。
oracle 在 Agent 返回 Completed 后、writer 完成排空前执行；结果通过相同有界队列交付，看到 PASS 也须检查最终退出码。
临时目录在正常返回或 Rust unwind 时 Drop；SIGKILL、系统崩溃等不保证清理。取消不回滚已经完成工具动作。

## 验证与缺口

确定性真实 binary/HTTP/文件场景由 [tests/lab_cli.rs](../../../tests/lab_cli.rs) 承载：

- `lab_auto_chat_ignores_stdin_and_inherited_cloud_configuration`：auto 不读 stdin、云配置不改变实验目标。
- `lab_default_read_shortcuts_pause_inspect_and_finish_with_stdin_open`：默认场景、快捷键、课程观察不推进、开放 stdin 退出。
- `lab_write_pass_requires_the_real_file_to_match_the_independent_oracle`：写入消息与真实磁盘验收。
- `lab_read_completed_without_a_tool_is_not_a_pass`：模型自然结束不能绕开读取事实。
- `lab_preflight_rejects_drift_and_redirects_before_model_calls`：版本/模型漂移和重定向不进入模型请求。
- `lab_rejects_remote_or_secret_origins_and_reports_unreachable_ollama`：端点边界和缺失服务诊断。
- `lab_eof_and_quit_cancel_without_running_a_model`：初始暂停 EOF/q 取消不调用模型。

`lab_origin_requires_literal_loopback_and_no_extra_url_components` 是同模块 URL 边界单测；
`local_llm_lab_auto_reads_fixture_without_stdin` 位于 [tests/local_llm.rs](../../../tests/local_llm.rs)，默认忽略，
需要显式启用真实 Ollama。测试符号与实际通过记录分开维护，后者不写入本模块永久合同。
有限场景不覆盖全部模型行为、恶意并发目录变更、所有 stdout 故障或不可中断存储阻塞。

## 变更检查

- 新参数是否扩大到任意端点、文件或 shell？扩展必须重新说明权限边界。
- oracle 是否独立于模型自评、仍检查真实提交与磁盘，而非从输出反推预期？
- 临时目录是否仍存活到 run/writer/oracle 结束，没有遗留后台任务？
- 自动控制是否仅响应已出现暂停，保留 Controller 的 pause ID/epoch 检查？
- 基线、课程和文本输出变化是否同步其直接消费者，而不改核心执行来满足示例？
