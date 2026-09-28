# Turnforge 101 用户指南：第一次成功运行

这篇只解决一个问题：**第一次把 Turnforge 用起来**。
完成后，你能启动本地实验、看到一次自动验收通过，并手动推进一次运行。
源码讲解、六阶段实验和编程练习只在[学习教程](harness-learning.md)维护；本页不是它的浓缩副本。

## 1. 准备仓库与 Rust

目前使用 macOS 或 Linux。尚未安装 Rust 时，先按 [Rust 官方安装说明](https://rust-lang.org/tools/install/)
安装 rustup；仓库通过 [rust-toolchain.toml](../rust-toolchain.toml)选择工具链。

还没有仓库才执行：

```sh
git clone https://github.com/shunminli/turnforge.git
cd turnforge
```

已有仓库则进入原目录，不重复 clone，也不要为跟随教程覆盖自己的修改。
把下面路径替换为你的真实仓库路径：

```sh
cd "/你的实际路径/turnforge"
```

在仓库根目录确认环境：

```sh
pwd
ls Cargo.toml rust-toolchain.toml scripts/harness-lab.sh
source "$HOME/.cargo/env"
cargo --version
```

**Cargo 是 Rust 的构建工具，Turnforge 才是本项目 CLI。**
后面的 Lab 脚本会构建当前源码再启动程序，不需要手动寻找 `target/debug/turnforge`，
也不会误用忘记重新构建的旧 binary。首次构建需要下载依赖。

## 2. 准备本地模型服务

完成[本地 LLM 指南](local-llm.md)中的安装以及
[服务启动、下载与创建测试模型](local-llm.md#4-服务启动下载与创建测试模型)。
版本、模型名称和基线只在那里维护，不在本页再抄一份。

保留运行 Ollama 的终端，另开一个终端回到仓库根目录继续下面步骤。
已有服务时先按本地 LLM 指南检查是否符合基线，不要重复占用端口。
Lab 启动时会预检服务和模型；不会自动安装、下载或启动服务。

## 3. 完成第一次自动运行

在仓库根目录执行：

```sh
bash scripts/harness-lab.sh --case chat --auto
echo $?
```

预期：程序显示场景结果 `PASS`，紧接着的退出码为 `0`。
这证明这次合成场景通过了验收，不代表所有 Harness 功能都已经验证。
如果失败，保留第一次输出，先按下文定位，不要反复重跑直到出现 PASS。

## 4. 体验一次手动单步

```sh
bash scripts/harness-lab.sh --case read
```

这个实验使用临时合成文件，不会拿你的项目文件作为模型输入。
看到暂停提示后，按以下顺序操作，每次都等新的输出再输入：

1. 输入 `i` 并回车，查看当前现场；它只读，不会推进执行。
2. 输入 `n` 并回车，放行下一个语义动作；这不是 Rust 源码逐行断点。
3. 再次暂停后，输入 `c` 并回车，继续到结束。
4. 确认出现 `PASS`，程序返回终端后执行 `echo $?`，预期为 `0`。

需要中止时输入 `q`；在没有其它输入/输出错误的取消路径上，预期退出码为 `130`。
取消不是回滚。完整快捷键、场景参数与结果含义只在 [Lab 使用参考](harness-lab.md)维护。

## 5. 首次使用完成清单

- 能在仓库根目录启动 Lab，不依赖手动维护的旧 binary。
- chat 自动场景和 read 手动场景均得到 `PASS`，且各自进程退出 `0`。
- 知道查看现场、推进、继续和取消的入口。

这就是 101 的终点，不要求你此时理解内部状态机、会话提交或工具权限实现。

## 遇到问题去哪里

| 现象 | 下一步 |
|---|---|
| 找不到 Cargo、脚本或 Cargo.toml | 回到本页第 1 节确认安装和当前目录 |
| 服务连接失败、版本或模型基线不匹配 | [本地模型环境与维护](local-llm.md) |
| 不知道 Lab 参数、暂停或验收输出的含义 | [Lab 使用参考](harness-lab.md) |
| 想复现失败并执行完整回归 | [测试执行参考](testing.md) |

## 下一步只选一个目标

- **系统学习底层实现**：进入[学习路线](harness-learning.md)，从课程准备开始逐章实操。
- **使用自己的任务、工作区或模型**：看 [CLI 使用参考](cli.md)。
- **调试自己的任务或接外部调试前端**：看[调试操作参考](debugging.md)。
- **维护某个模块**：从[文档总索引](README.md)定位它的架构和设计合同。
