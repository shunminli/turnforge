//! CLI-only curriculum and read-only explanations of committed debug facts.
use turnforge::{DebugAction, DebugPoint, DebugSnapshot};

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum Lesson {
    Loop,
    Context,
    Tools,
    Control,
    Io,
    Regression,
}

/// Static study material: no provider, filesystem, progress store or execution.
pub fn catalog(lesson: Option<Lesson>) -> String {
    let Some(lesson) = lesson else {
        return "\
Harness 学习路线 / learn
按顺序学习六课；课程只是观察提示，不会替你确认掌握程度。
1. loop       调度：User → Model → Tool → Model → Finish
2. context    上下文：暂态 delta 与已提交消息，逻辑快照与 HTTP payload
3. tools      工具：模型提议、宿主授权、真实结果与副作用
4. control    控制：pause ID、单步/继续、安全边界与协作取消
5. io         宿主：控制输入、事件输出、背压与 run 生命周期
6. regression 回归：模型说完成不等于任务通过，用独立事实验收
起点：src/agent.rs::Agent::run_loop；文档：docs/harness-learning.md
看一课：turnforge learn loop
边跑边学：turnforge lab --case read --lesson loop
无交互回归：turnforge lab --case read --auto --lesson regression
命令均在仓库构建出的 turnforge 上运行；也可使用 bash scripts/harness-lab.sh。
"
        .into();
    };
    let (id, title, prerequisite, source, command, exercise, rubric) = match lesson {
        Lesson::Loop => (
            "loop",
            "一条真实 Agent 调度循环",
            "能运行 lab；不要求先理解 Rust async。",
            "src/agent.rs::Agent::{run_debug,run_loop,commit}；src/event.rs::{RunState,Event}",
            "turnforge lab --case read --lesson loop",
            "逐次按 Enter：先看 next，再预测是模型请求、工具执行还是结束；记录每次 messages 的变化。",
            "能解释模型 step 与调试动作不是同一计数；指出最终 Assistant 提交后为什么还需释放 Finish。",
        ),
        Lesson::Context => (
            "context",
            "上下文与提交边界",
            "先完成 loop，知道 Assistant 与 Tool 各由谁提交。",
            "src/model.rs::ModelRequest；src/agent.rs::Agent::{run_loop,checkpoint}；src/openai.rs::OpenAiModel::request",
            "turnforge lab --case read --lesson context",
            "在模型前、工具前、工具后按 i；比较 system、messages、tools、pending_calls，找到 call_id 配对。",
            "能区分暂态 delta 与完整 Assistant；知道工具未配对的快照不是可直接重发的完整请求，也不是 HTTP 原文。",
        ),
        Lesson::Tools => (
            "tools",
            "模型提议与工具执行权",
            "先完成 context；理解 read 场景默认无写权限。",
            "src/tools/mod.rs::ToolRegistry::{definitions,execute}；src/tools/files.rs::FileTool::execute；src/agent.rs::Agent::run_loop",
            "turnforge lab --case write --lesson tools",
            "在 next=tool 暂停查看参数；执行一步后比较 ToolOutput 与临时文件，再用独立一次运行在工具前 q 取消。",
            "能指出注册/可见/授权/执行四者区别；明白取消不撤销已完成写入，write 场景授权不包含 shell。",
        ),
        Lesson::Control => (
            "control",
            "安全暂停与协作取消",
            "先完成 loop；练习时使用合成临时场景。",
            "src/debug.rs::DebugController::try_send；src/debug.rs::DebugSession::checkpoint；src/agent.rs::Agent::run_loop",
            "turnforge lab --case read --lesson control",
            "按 i 验证不推进，再按 n；另一次运行用 c 后 p，观察暂停只能出现在安全边界；最后用 q 取消。",
            "能解释 lab 快捷键如何关联观察到的 pause ID；原始 debug 的旧/预送命令为什么不能放行后来的暂停。短运行可能在 p 到达前已结束。",
        ),
        Lesson::Io => (
            "io",
            "宿主输入、输出与生命周期",
            "先完成 control；了解 stdout/pipe 的基本用途。",
            "src/main.rs::execute；src/debug_input.rs::DebugInput；src/output.rs::forward；tests/cli_http.rs",
            "turnforge lab --case read --lesson io",
            "交互观察暂停摘要和 i 的完整快照；再运行 lab --case read --auto </dev/null，比较无 stdin 控制也能完成的路径。",
            "能指出 Agent 不读终端；run 完成停止输入、输出失败取消 run、工具清理仍需 await；事件发出不等于消费者已收到。",
        ),
        Lesson::Regression => (
            "regression",
            "用外部事实验收 Harness",
            "先完成 loop、context 和 tools。",
            "src/lab.rs::PreparedLab::verify；tests/lab_cli.rs；tests/local_llm.rs",
            "turnforge lab --case read --auto --lesson regression",
            "分别运行 chat/read/write；找出每个 PASS 所依赖的提交消息与真实文件事实，比较模型 Completed 与 lab 验收结果。",
            "能说明未知读取 marker 为什么不能只放在 prompt，为什么写入必须读回；真实小模型测试不能替代可控故障的确定性回归。",
        ),
    };
    format!(
        "[learn {id}] {title}\n目标：{title}，并能用现场证据解释。\n前置：{prerequisite}\n源码：{source}\n运行：{command}\n观察练习：{exercise}\n人工验收：{rubric}\n说明：课程不更改 prompt、权限、执行或验收条件；没有自动学习进度。\n"
    )
}

/// Explain only facts available at this semantic boundary; never mutate them.
pub fn hint(lesson: Lesson, snapshot: &DebugSnapshot) -> String {
    let point = match &snapshot.point {
        DebugPoint::BeforeModel => "首次模型请求尚未开始，User 已提交",
        DebugPoint::AfterModel => "完整 Assistant 已校验并提交",
        DebugPoint::AfterTool { .. } => "一条工具结果已提交，该工具已返回",
    };
    let next = match &snapshot.next {
        DebugAction::Model { .. } => "一次模型请求",
        DebugAction::Tool { .. } => "一个工具调用",
        DebugAction::Finish { .. } => "结束动作（不是新模型请求）",
    };
    let (id, explanation, source, question) = match lesson {
        Lesson::Loop => (
            "loop",
            format!(
                "{point}；下一动作是{next}。模型 step={}，并非已单步动作数。",
                snapshot.step
            ),
            "src/agent.rs::Agent::run_loop",
            "下一次按 n 后，应该新增哪一种消息，还是只结束运行？",
        ),
        Lesson::Context => (
            "context",
            format!(
                "当前逻辑快照含 {} 条已提交消息、{} 个可见工具、{} 个待结果调用。它不是 provider HTTP 原文。",
                snapshot.messages.len(),
                snapshot.tools.len(),
                snapshot.pending_calls.len()
            ),
            "src/agent.rs::Agent::checkpoint；src/model.rs::ModelRequest",
            "按 i 查 call_id：哪些调用已配对？下一次模型能看到什么新结果？",
        ),
        Lesson::Tools => (
            "tools",
            format!(
                "{point}；尚有 {} 个调用等待结果。调用来自模型提议，实际执行仍由 Registry 校验权限。",
                snapshot.pending_calls.len()
            ),
            "src/tools/mod.rs::ToolRegistry::execute；src/agent.rs::Agent::run_loop",
            "next 是工具时，预览参数与执行后的 ToolOutput、文件事实是否一致？",
        ),
        Lesson::Control => (
            "control",
            format!(
                "当前是安全暂停 #{}。n/c/i 关联观察到的暂停 ID；库还会检查命令提交时的 epoch。",
                snapshot.pause_id
            ),
            "src/debug.rs::DebugController::try_send；src/debug.rs::DebugSession::checkpoint",
            "i 为什么不推进？q 取消后哪些未执行调用仍需要补终态结果？",
        ),
        Lesson::Io => (
            "io",
            format!(
                "暂停 #{} 的通知由宿主输出；Agent 不读取终端。事件队列只限项数，并非可靠日志或固定字节预算。",
                snapshot.pause_id
            ),
            "src/main.rs::execute；src/output.rs::forward",
            "若 stdout 不再消费，谁请求取消、谁等待工具清理、谁停止 stdin？",
        ),
        Lesson::Regression => (
            "regression",
            format!(
                "{point}；当前暂停本身不是 PASS。场景在运行结束后独立检查已提交结果与真实文件。"
            ),
            "src/lab.rs::PreparedLab::verify；tests/lab_cli.rs",
            "即使模型声称成功或返回 Completed，什么外部事实仍可能让验收失败？",
        ),
    };
    format!(
        "[learn {id} @pause {}] {explanation}\n源码：{source}\n观察：{question}\n",
        snapshot.pause_id
    )
}
