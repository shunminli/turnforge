# 边调试边学习 Harness

学习入口有两种；两者共享同一套课程内容：

```sh
target/debug/turnforge learn
target/debug/turnforge learn context
bash scripts/harness-lab.sh --case read --lesson loop
```

`learn` 只显示总路线或单课，不需要 Ollama、不读取工作区、不请求模型。
`lab --lesson` 在每个暂停点给出简短解释、源码符号和观察问题；按 `l` 查看完整课件、`i` 看逻辑快照。
它不让 LLM 生成教程，不修改 prompt、工具权限、调度路径或 PASS 条件，不自动记录学习进度。

## 六阶段路线

| 阶段 | 先掌握什么 | 调试练习 | 人工验收：能解释或指出 |
|---|---|---|---|
| `loop` | 一次 User turn 可含多次模型/工具动作 | read 逐次 n，先预测 next 再执行 | 模型 step ≠ 调试动作；最终 Finish 为什么单独确认 |
| `context` | delta 暂态、完整消息提交 | 工具前后 i，比较 messages 与 pending_calls | call ID 配对；逻辑快照不是原始 HTTP 请求或恢复点 |
| `tools` | 模型提议，宿主授权和执行 | write 在工具前后观察；另一次 q 取消 | 可见工具与执行权限；预览不等于副作用，取消不是回滚 |
| `control` | 安全边界及协作取消 | i/n/c/p/q，比较不推进、推进与退出 | pause ID 和 epoch 防止旧/预送命令放行未来暂停 |
| `io` | headless core 与终端宿主分工 | 交互 read，再 `--case read --auto </dev/null` | run/control/writer 的结束责任；发 Event 不等于已交付 |
| `regression` | 模型自然结束与用户任务成败不同 | chat/read/write --auto | PASS 的独立消息/磁盘证据；真实小模型不替代确定性故障回归 |

每一课的完整前置、命令、源码入口和问题以 `turnforge learn LESSON` 为准。
建议一次只改一个观察点：先预测下一动作，再单步，最后用现场事实解释预测是否正确。
短模型运行可能在输入 `p` 前完成；这不证明暂停失效，Pause 只能作用于尚未跨过的安全边界。

## 从现象定位实现

- 调度：`src/agent.rs::Agent::run_loop`，单 writer 创建请求、提交 Assistant 和 Tool。
- 上下文：`src/model.rs::ModelRequest` 是借用；`Agent::checkpoint` 输出 owned 快照。
- 执行权：`src/tools/mod.rs::ToolRegistry::execute` 再查权限；文件副作用由 FileTool 完成。
- 控制：`src/debug.rs::DebugSession::checkpoint` 等待安全命令，不把工具 future 丢弃。
- 宿主：`src/main.rs` 组合控制/事件/信号，`src/output.rs::forward` 管交付和写入限制。
- 验收：`src/lab.rs` 对最终提交与实际文件做独立检查，不依靠模型自评。

源码入口使用仓库相对路径与符号，不固定易漂移的行号；入口可能是 private helper，也可直接定位阅读。
暂停提示只使用 DebugSnapshot 的逻辑事实，不知道网络原始 payload、模型内部思考或尚未执行动作的结果。

## 能力边界

课程是维护者编写的观察计划，不是“完成六课即掌握”的自动认证。没有测验判分、持久化进度、用户画像、
按源码行断点、历史编辑或 LLM Space 接入。学习不影响实验原有的预检、权限、取消和独立 oracle。
长历史快照仍有复制和序列化成本；课件不提供秘密脱敏或资源预算保证。

入门实验步骤见[Harness Lab 指南](harness-lab.md)，深层合同从[模块索引](README.md)进入。
课程自身的维护边界见[学习架构](modules/learning/architecture.md)与[学习设计](modules/learning/design.md)。
本轮实际执行过的学习实验与验证边界见[验收记录](verification/harness-lab-2026-09-14.md)。
