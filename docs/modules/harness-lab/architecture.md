# 本地 Harness Lab：架构

实现入口：[src/lab.rs](../../../src/lab.rs)、[scripts/harness-lab.sh](../../../scripts/harness-lab.sh)。
配套：[设计](design.md) · [使用指南](../../harness-lab.md) · [文档导航](../../README.md)。

## 职责与非职责

Lab 是 binary 内的生产消费者：为维护者提供固定小模型上的可重复合成实验。
它拥有本机 Ollama 基线预检、临时场景准备、已完成运行的独立结果验收；不重新实现模型—工具循环。
`main.rs` 仍拥有 Agent、输入/输出/取消生命周期，`debug.rs` 仍拥有暂停协议。
学习提示由独立[学习模块](../learning/architecture.md)生成，不参与场景验收。

不提供通用 benchmark、云模型选择、任意 workspace、任意 prompt、shell、自动安装、自动起服务、模型下载或网页。
不得为了本地例题通过而改变 provider、工具或调度语义；失败应保留为事实而不是自动重试。

## 组合关系

```text
harness-lab.sh → turnforge lab
                  ├─ Lab：参数/本机预检 → 临时 PreparedLab
                  ├─ CLI：原 Agent + run_debug + 控制/输出/信号
                  │         └─ Ollama Chat Completions + 真实文件工具
                  └─ Lab：Completed + 已提交 messages + 磁盘 → PASS / FAIL
```

固定基线使模型/版本变化显式化；临时合成工作区降低试用误触用户数据的风险；独立 oracle 避免把模型自评当作正确性证明。
Lab 与 `tests/local_llm.rs` 共享验证目的，但不是用启动 Cargo 测试代替产品入口。确定性故障测试继续由测试夹具负责。

## 所有权与生命周期

| 资源 / 状态 | owner | 结束责任 |
|---|---|---|
| 固定模型/版本/digest、端点校验 | Lab | 只读配置，无全局可变状态 |
| 预检 HTTP client / response | 预检调用 | await 完成或超时；不启动常驻 worker |
| 动态 marker、prompt、目标路径与预期内容 | PreparedLab / 私有 Expected 枚举 | 借给 CLI 创建 Agent 输入；验收不从模型响应倒推预期 |
| 临时目录 | PreparedLab 的 TempDir | 宿主运行、输出、oracle 结束后 Drop 清理 |
| Agent/Controller/输入/输出 futures | CLI | cancel/join/关闭职责见 CLI 文档 |
| 最终 PASS/FAIL | CLI 根据 PreparedLab 验收投影 | 不能早于运行完成；输出失败不视为成功交付 |

临时工作区必须活过整个 run 和验收；工具持有路径并不代替目录 owner。
Oracle 借用最终 transcript 并读取实际文件，不修改 Agent 历史，不影响执行期间状态。

## 安全与取舍

只允许明确的 loopback IP HTTP origin，预检与 provider 都禁代理/重定向，不向本地模型发送云凭据。
`write` 的显式场景选择授予临时工作区文件写权限；所有场景无 shell，因此不会继承 shell 的完整宿主能力。
这仍不是不可信模型的 OS sandbox：文件工具的并发路径攻击限制与进程强杀清理限制没有被 Lab 修复。

固定参数牺牲任意配置，换取新手无需理解 provider flags 即可复现一条真实链路。
真实小模型仍可能失败；PASS 只证明这次场景满足有限 oracle，不证明所有 Harness 行为或模型质量。

## 维护影响

新增场景需在 Lab owner 声明初始文件、权限、prompt、独立验收事实和取消后的目录生命周期；不要把私有测试分支塞入核心。
基线升级要同步固定模型文档、预检、真实回归，并记录实际验证，不能仅改名称绕过 digest。
修改控制/输出生命周期应改 CLI owner；修改课件应改学习 owner。跨模块证据见[设计](design.md)。
