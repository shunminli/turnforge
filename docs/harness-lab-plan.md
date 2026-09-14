# Harness Lab 与学习入口：执行计划

## 合同与边界

在已完成的原生调试增量上增加真实用户入口，不重建 Agent 循环、不修改 Agent/Debugger 协议。
provider 增加 `OpenAiModel::new_local`，让本机调用显式禁代理且不接收云凭据；原构造入口行为不变。
用户授权本地实现与受控测试；沿用当前未提交改动，不提交、推送、安装或下载模型。

- `turnforge lab` / `scripts/harness-lab.sh`：默认本地 read 场景，检查固定 Ollama/模型基线，创建临时数据。
- `--case chat|read|write`：明确场景；只有 write 开启文件写权限，全部禁止 shell。
- 默认交互简化命令；`--auto` 不读 stdin，逐暂停点推进并验证实际结果，Completed 不自动等于 PASS。
- `turnforge learn [LESSON]` 输出分阶段学习规划；`lab --lesson ...` 在真实暂停点结合源码解释、提出观察问题。
  学习投影不改变模型请求、权限、工具执行或测试 oracle，也不自动声称用户已学会。
- 仅支持显式 loopback IP 的 HTTP Ollama origin；忽略云模型/key 环境配置，无云端 fallback。
- 不做 Web UI、LLM Space 适配、自定义场景持久化、学习进度存储或自动启动服务。

## Ownership 与验证

`PreparedLab` 唯一持有临时目录和独立预期，直到 run、结果验证及输出排空后释放；不保存 Agent 引用。
Agent 仍独占 transcript。CLI 从事件维护只读 pause ID 投影；快捷键关联输入读取时的暂停点，
不从未来现场补填旧输入；真实接收时仍由 DebugController 的入队 epoch 校验。
输入 buffer、read future、output fd 沿用可取消宿主；run/control 各持有自己的 Sender，run_done 停止输入，
join 等待全部结束。无 detached task。学习模块只生成文字，不修改运行或访问网络。

| ID | 状态 | 动作与完成标准 | 依赖 / 证据 |
|---|---|---|---|
| L1 | done | 固定入口、资源、权限、学习与 oracle 合同 | 本计划及 Lab/学习模块双文档 |
| L2 | done | 本地预检、临时场景、独立验证、一键脚本 | 真实 HTTP/磁盘断言及三个实际 launcher 场景 |
| L3 | done | CLI 简洁输出、快捷键与自动模式 | 原 run/debug 回归，旧输入关联、EOF/q、stdout 故障与正常结束 |
| L4 | done | 学习目录和实时暂停点解说 | 六课源码入口；无模型 learn、i/l 不推进；真实 read/write 课程 |
| L5 | done | 集成测试、直接模块文档及导航 | 8 项 Lab CLI 测试、新增真实模型用例、11 组模块双文档 |
| L6 | done | 冻结后独立验证 | 独立 verified；debug/release 各 54 项；真实模型各 5 项通过，文档链接检查通过 |

主入口、场景、课程与测试可在合同固定后并行；真模型测试串行。失败回到真实 owner 修复后重新冻结。
保留原调试验证记录；本轮证据另记，不把历史通过数当作新版本结果。

## 完成证据与维护落点

本轮冻结指纹、完整命令、首个真实模型失败及设计调整见
[Lab 与学习入口验收记录](verification/harness-lab-2026-09-14.md)。未提交或推送，未触发远端 CI。

长期知识归入 [Lab](modules/harness-lab/design.md)、[学习](modules/learning/design.md)及直接受影响的
CLI、输出、模型模块；更新总索引和两份使用指南。不修改 AGENTS 治理规则，也不建立重复的仓库记忆目录。
