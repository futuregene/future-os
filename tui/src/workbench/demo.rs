//! Demo data — the workbench filled with a believable session, no agent.
//!
//! Two jobs. First, screenshots and design review: the surface has to be looked
//! at with realistic content (a long conversation, ten sessions, an occupied
//! context bar) before it is wired to anything, or every layout bug shows up
//! late with real data. Second, the frame tests: they assert width-totality and
//! hit regions at many sizes, which needs data that is *shaped* like the real
//! thing and completely deterministic.
//!
//! Nothing here talks to the agent, and nothing reads the clock.

use crate::workbench::ansi::style;
use crate::workbench::state::{
    Activity, ActivityKind, InputView, JobRow, SessionMeta, SessionRow, SkillRow, TodoItem,
    UsageData, WorkbenchData,
};
use crate::workbench::theme::{Palette, FUTURE_DARK};

/// Which demo screen to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    /// No session yet: the launchpad.
    Launchpad,
    /// A conversation with the work between two answers folded away.
    Chat,
    /// A long goal todo list, to see the sidebar scroll.
    Todo,
}

impl Scenario {
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "launchpad" | "home" => Some(Self::Launchpad),
            "chat" => Some(Self::Chat),
            "todo" | "todos" => Some(Self::Todo),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Launchpad => "launchpad",
            Self::Chat => "chat",
            Self::Todo => "todo",
        }
    }
}

/// The scenario's data at the default palette.
pub fn data(scenario: Scenario) -> WorkbenchData {
    match scenario {
        Scenario::Launchpad => launchpad(),
        Scenario::Chat => chat(),
        Scenario::Todo => todo(),
    }
}

fn base(palette: &Palette) -> WorkbenchData {
    WorkbenchData {
        app_name: "future-tui".into(),
        version: "0.11.0".into(),
        connected: true,
        session: SessionMeta::default(),
        model: "future/deepseek-flash".into(),
        effort: "medium".into(),
        mode: "manual".into(),
        permission: "workspace".into(),
        usage: UsageData {
            ctx_used: 6_900,
            ctx_total: 1_000_000,
            cost: 0.0123,
            cache_pct: 96,
            tps: 0.0,
        },
        todos: vec![
            TodoItem {
                text: "复现 parser 的 off-by-one".into(),
                done: true,
                active: false,
            },
            TodoItem {
                text: "补一个回归测试".into(),
                done: false,
                active: true,
            },
            TodoItem {
                text: "跑 tui 的完整测试套件".into(),
                done: false,
                active: false,
            },
        ],
        sessions: vec![
            SessionRow {
                id: "s1".into(),
                title: "修复 parser 的 off-by-one".into(),
                workspace: "~/future-os".into(),
                age: "2m".into(),
                current: true,
            },
            SessionRow {
                id: "s2".into(),
                title: "workbench 布局草图".into(),
                workspace: "~/future-os".into(),
                age: "1h".into(),
                current: false,
            },
            SessionRow {
                id: "s3".into(),
                title: "mobile 恢复会话".into(),
                workspace: "~/future-os/.worktrees/mobile".into(),
                age: "3h".into(),
                current: false,
            },
            SessionRow {
                id: "s4".into(),
                title: "blog: why one agent".into(),
                workspace: "~/futureos-wechat-articles".into(),
                age: "2d".into(),
                current: false,
            },
        ],
        skills: vec![
            SkillRow {
                name: "future-versioning".into(),
                description: "worktree、分支与提交".into(),
            },
            SkillRow {
                name: "future-peer-review".into(),
                description: "按证据评审稿件".into(),
            },
            SkillRow {
                name: "future-web".into(),
                description: "搜公开网页并核对事实".into(),
            },
        ],
        jobs: vec![
            JobRow {
                name: "dev 分支 PR 合并".into(),
                state: "每 10m".into(),
            },
            JobRow {
                name: "夜间覆盖率".into(),
                state: "每天 03:00".into(),
            },
        ],
        messages: Vec::new(),
        input: InputView {
            placeholder: "描述你要做的事 · @ 文件 · # 会话 · / 命令".into(),
            ..InputView::default()
        },
        activity: Activity::default(),
        palette: palette.id,
    }
}

/// The launchpad: nothing attached yet, so the header has no session and the
/// body is the composer.
pub fn launchpad() -> WorkbenchData {
    let mut data = base(&FUTURE_DARK);
    data.input.text.clear();
    data.sessions[0].current = false;
    data
}

/// A conversation with the work folded away, which is what the default reading
/// level produces: one summary row above the answer it produced.
pub fn chat() -> WorkbenchData {
    let mut data = base(&FUTURE_DARK);
    data.session = SessionMeta {
        id: "s1".into(),
        title: "修复 parser 的 off-by-one".into(),
        cwd: "~/future-os".into(),
        branch: "dev-huichen".into(),
    };
    data.activity = Activity {
        kind: ActivityKind::Thinking,
        detail: "补一个回归测试".into(),
        elapsed_secs: 3,
        tools: 2,
    };
    data.usage.tps = 42.0;
    let p = FUTURE_DARK;
    data.messages = vec![
        style(p.accent, " ❯ 看看 parser 的 off-by-one，顺手补个回归测试"),
        String::new(),
        style(p.muted, " ▸ 2 tool calls · 1 thought"),
        String::new(),
        style(p.fg, " 循环用的是 `<=`，最后一位会越界读一个字节。"),
        style(p.fg, "改成 `<` 之后 41 个用例都过了。"),
        String::new(),
        style(p.muted, " ▸ 补一个回归测试"),
        style(p.muted, " read src/parser.rs"),
        style(p.muted, " $ cargo test -p parser --off-by-one"),
        String::new(),
    ];
    data
}

/// A long todo list, to exercise the sidebar's window and its "more" row.
pub fn todo() -> WorkbenchData {
    let mut data = chat();
    data.todos = (0..24)
        .map(|index| TodoItem {
            text: format!(
                "第 {} 项：把 workbench 的 {} 接上真实数据",
                index + 1,
                panel_for(index)
            ),
            done: index % 4 == 0,
            active: index == 2,
        })
        .collect();
    data.session.title = "workbench 侧栏验收".into();
    data
}

fn panel_for(index: usize) -> &'static str {
    const NAMES: [&str; 5] = ["待办", "任务", "会话", "用量", "技能"];
    NAMES[index % NAMES.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scenario_is_reachable_by_name() {
        for scenario in [Scenario::Launchpad, Scenario::Chat, Scenario::Todo] {
            assert_eq!(Scenario::parse(scenario.name()), Some(scenario));
            assert_eq!(
                Scenario::parse(&scenario.name().to_uppercase()),
                Some(scenario)
            );
        }
        assert_eq!(Scenario::parse("home"), Some(Scenario::Launchpad));
        assert_eq!(Scenario::parse("nope"), None);
    }

    #[test]
    fn the_launchpad_has_no_session_and_the_chat_does() {
        assert!(launchpad().on_launchpad());
        let chat = chat();
        assert!(!chat.on_launchpad());
        assert_eq!(chat.session.branch, "dev-huichen");
        assert!(!chat.messages.is_empty());
    }

    #[test]
    fn the_chat_scenario_shows_the_reading_levels_shape() {
        // The default level's summary row, as the conversation renderer emits
        // it — the demo data mirrors the real one so screenshots are honest.
        let chat = chat();
        let text: String = chat
            .messages
            .iter()
            .map(|line| crate::utils::strip_ansi_codes(line))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("2 tool calls"), "{text}");
        assert!(text.contains("41 个用例都过了"), "{text}");
    }

    #[test]
    fn the_todo_scenario_is_long_enough_to_scroll() {
        let data = todo();
        assert_eq!(data.todos.len(), 24);
        assert!(data.todos.iter().any(|todo| todo.done));
        assert!(data.todos.iter().any(|todo| todo.active));
    }

    #[test]
    fn the_demo_is_deterministic() {
        // Screenshot comparisons and hit-region assertions depend on this: two
        // builds of the same scenario must produce identical bytes.
        assert_eq!(chat(), chat());
        assert_eq!(launchpad(), launchpad());
    }
}
