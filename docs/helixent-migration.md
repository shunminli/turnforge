# Helixent → Turnforge 迁移对照

参考基线：Helixent 1.3.1，commit `5cc1fb3faf29b8db8dce614e925c89030ae2558b`。
迁移单位是行为和边界，不是把 141 个 TS/TSX 文件机械换语言。

| Helixent | Turnforge M0 | 有意差异 / 后续 |
|---|---|---|
| foundation messages / model | `message.rs` / `model.rs` | 文本 + function tools；图片/thinking 尚未迁移 |
| `agent/agent.ts` | `agent.rs` | 单一可变 owner、显式状态、只读 transcript |
| agent event | `event.rs` | delta / commit / tool / terminal 分开，非持久化事件日志 |
| community/openai | `openai.rs` | 直接 HTTP/SSE；严格校验完成语义；不依赖 usage 判结束 |
| community/anthropic | 未迁移 | M1：Messages API、内容块、stop reason、usage |
| tool registry / zod | `tools/mod.rs` + serde/schemars | schema 和输入类型同源、权限双重校验 |
| read / list / write / str_replace / mkdir | `tools/files.rs` | 相对路径、限制 symlink；replace 要求唯一匹配；write 父目录需存在 |
| bash（实际启动 zsh） | `tools/shell.rs` | 明确 `/bin/bash`；排空双管道、超时、进程组清理 |
| 并行 `_act` | 串行调度 | 先保证副作用顺序，后续按资源冲突并行 |
| mutable middleware hooks | 未迁移 | 先明确审批/上下文处理的专用边界，避免重新引入共享可变 Context |
| 审批 resolver 队列 | 静态 capability grant | M1：取消感知的宿主审批接口，无 UI 依赖 |
| skills / AGENTS 加载 | 未迁移 | M2：来源优先级、路径作用域、提示注入、预算 |
| grep / glob / patch / file_info / move / ask_user / todo | 未迁移 | M2：逐项迁移，先冻结输入输出及失败合同 |
| Ink TUI | 纯 CLI / NDJSON | M3：UI 作为独立宿主 |
| session persistence / subagent roadmap | 未迁移 | 自主设计，不能声称已具备上游 roadmap 中的能力 |

## 建议里程碑

### M0：内核闭环（本次）

可配置一个 provider；模型 → 工具 → 模型可运行；取消/失败/步数终止有测试。
不依赖 live key 的 CLI 协议集成；库无终端依赖。尚未做真实账号/model 验收。

### M1：宿主控制与第二个 provider

增加 Anthropic；完善内容模型；宿主交互审批；给 provider 协议建立共享契约测试。
验收：拒绝/超时/取消审批均不执行工具；两个 provider 可完成同一个文件修改任务。

### M2：可靠会话与 coding 能力

设计持久化与恢复合同；预算/compaction；AGENTS/skills；补齐搜索和 patch 工具。
验收：进程重启后历史一致；文件修改中断不被误写为成功；上下文截断不破坏 tool-call 配对。

### M3：产品化入口与执行隔离

按真实需求选择 TUI、服务、IDE；接真实 sandbox 后端；再做并行工具或 subagent。
验收：所有宿主共用内核；权限与生命周期合同不依赖某个界面。

## 维护原则

每次迁移记录源 commit、行为差异、回归测试和未解决边界。不追求 fork 自动同步；
上游变更应先判断解决了什么问题，再决定是否吸收设计。Rust 只是实现语言，
真正的收益取决于边界清晰、失败语义可验证，以及是否控制住产品范围。
