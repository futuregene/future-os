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
use crate::workbench::i18n::{t, Key};
use crate::workbench::state::{
    Activity, ActivityKind, GitKind, GitRow, GitSection, GitView, InputView, ModelChoice,
    SessionMeta, SessionRow, SkillRow, UsageData, WorkbenchData, WorkspaceGroup,
};
use crate::workbench::theme::FUTURE_DARK;

/// Which demo screen to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    /// No session yet: the launchpad.
    Launchpad,
    /// A conversation with the work between two answers folded away.
    Chat,
    /// Many workspaces and conversations, to see the rail scroll.
    Workspaces,
}

impl Scenario {
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "launchpad" | "home" => Some(Self::Launchpad),
            "chat" => Some(Self::Chat),
            "workspaces" | "workspace" | "todo" => Some(Self::Workspaces),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Launchpad => "launchpad",
            Self::Chat => "chat",
            Self::Workspaces => "workspaces",
        }
    }
}

/// The scenario's data at the default palette.
pub fn data(scenario: Scenario) -> WorkbenchData {
    match scenario {
        Scenario::Launchpad => launchpad(),
        Scenario::Chat => chat(),
        Scenario::Workspaces => workspaces(),
    }
}

fn base() -> WorkbenchData {
    WorkbenchData {
        fusion_enabled: Some(true),
        version: "0.11.0".into(),
        connected: true,
        session: SessionMeta::default(),
        model: "future/deepseek-flash".into(),
        effort: "medium".into(),
        mode: "manual".into(),
        usage: UsageData {
            ctx_used: 6_900,
            ctx_total: 1_000_000,
            cost: 0.0123,
            // 780.0k / 812.4k = 96%: the demo's three counts have to agree with
            // the hit rate beside them, or the row teaches the wrong arithmetic.
            cache_pct: 96,
            input_tokens: 812_400,
            output_tokens: 24_800,
            cache_read_tokens: 780_000,
            tps: 0.0,
        },
        sessions: vec![
            SessionRow {
                id: "s1".into(),
                title: "修复 parser 的 off-by-one".into(),
                workspace: "future-os".into(),
                age: "2m".into(),
                current: true,
                parent_id: None,
                streaming: false,
                created_at_ms: 0,
                pinned: false,
            },
            SessionRow {
                id: "s2".into(),
                title: "workbench 布局草图".into(),
                workspace: "future-os".into(),
                age: "1h".into(),
                current: false,
                parent_id: None,
                streaming: false,
                created_at_ms: 0,
                pinned: false,
            },
            SessionRow {
                id: "s3".into(),
                title: "mobile 恢复会话".into(),
                workspace: "mobile".into(),
                age: "3h".into(),
                current: false,
                parent_id: None,
                streaming: false,
                created_at_ms: 0,
                pinned: false,
            },
            SessionRow {
                id: "s4".into(),
                title: "blog: why one agent".into(),
                workspace: "futureos-wechat-articles".into(),
                age: "2d".into(),
                current: false,
                parent_id: None,
                streaming: false,
                created_at_ms: 0,
                pinned: false,
            },
            SessionRow {
                id: "s5".into(),
                title: "把 NATS 的 TLS 说清楚".into(),
                workspace: "临时对话".into(),
                age: "4d".into(),
                current: false,
                parent_id: None,
                streaming: false,
                created_at_ms: 0,
                pinned: false,
            },
        ],
        workspaces: vec![
            WorkspaceGroup {
                name: "future-os".into(),
                path: "/home/dev/future-os".into(),
                age: "2m".into(),
                session_ids: vec!["s1".into(), "s2".into()],
            },
            WorkspaceGroup {
                name: "mobile".into(),
                path: "/home/dev/future-os/.worktrees/mobile".into(),
                age: "3h".into(),
                session_ids: vec!["s3".into()],
            },
            WorkspaceGroup {
                name: "futureos-wechat-articles".into(),
                path: "/home/dev/futureos-wechat-articles".into(),
                age: "2d".into(),
                session_ids: vec!["s4".into()],
            },
        ],
        chats: vec!["s5".into()],
        models: demo_models(),
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
        // The `@` menu's candidates: a handful of real paths, so a demo frame
        // can show the menu with something in it.
        files: vec![
            "tui/src/workbench/mod.rs".into(),
            "tui/src/workbench/screen.rs".into(),
            "tui/src/workbench/live.rs".into(),
            "docs/tui/workbench-spec.md".into(),
            "AGENTS.md".into(),
        ],
        messages: Default::default(),
        row_stops: Vec::new(),
        input: InputView {
            placeholder: t(
                crate::workbench::i18n::Language::Zh,
                Key::InputHintLaunchpad,
            )
            .into(),
            ..InputView::default()
        },
        activity: Activity::default(),
        compacting: false,
        notice: None,
        queue: Vec::new(),
        revision: 0,
        mouse_capture: true,
        approval: None,
        diff: None,
        fork_points: Vec::new(),
        worktrees: Vec::new(),
        git: None,
        tree: demo_tree(),
        preview: None,
    }
}

/// A picture for the preview box, generated rather than read from disk: a
/// screenshot of the viewer must not depend on whose checkout is rendered.
///
/// It is a smooth two-axis gradient with a diagonal band, which is enough to
/// show that the half-block renderer keeps its aspect ratio, that the 6×6×6
/// fallback still reads as a picture, and that a zoom actually enlarges it.
/// The `/git` panel's content for a frame: a small log with a merge in it, the
/// refs that log sits on, one edited file and one stash, with the first merge
/// commit selected. Deterministic like every other demo value — a screenshot of
/// this panel must not depend on whose checkout is rendered.
pub fn git() -> GitView {
    let commits = vec![
        row(
            Some("9e4a47d2a"),
            "*   ",
            &[
                ("feat(tui): a git panel for the workbench", GitKind::Plain),
                (
                    "HEAD -> dev-huichen, origin/dev-huichen",
                    GitKind::Decoration,
                ),
                ("9e4a47d2a", GitKind::Sha),
                ("2026-10-27", GitKind::Meta),
                ("huichen", GitKind::Meta),
            ],
        ),
        row(None, "|\\", &[]),
        row(
            Some("4c1b8f0e3"),
            "| * ",
            &[
                ("fix(mobile): stop folding code blocks", GitKind::Plain),
                ("fix/mobile-code-block", GitKind::Decoration),
                ("4c1b8f0e3", GitKind::Sha),
                ("2026-10-27", GitKind::Meta),
                ("huichen", GitKind::Meta),
            ],
        ),
        row(
            Some("7cefc9cea"),
            "| * ",
            &[
                ("docs: goal control plane notes", GitKind::Plain),
                ("tag: v0.11.0", GitKind::Decoration),
                ("7cefc9cea", GitKind::Sha),
                ("2026-10-25", GitKind::Meta),
                ("huichen", GitKind::Meta),
            ],
        ),
        row(None, "|/", &[]),
        row(
            Some("ae142f75a"),
            "* ",
            &[
                ("fix(tui): one row per record", GitKind::Plain),
                ("ae142f75a", GitKind::Sha),
                ("2026-10-26", GitKind::Meta),
                ("huichen", GitKind::Meta),
            ],
        ),
        row(
            Some("56ec934f6"),
            "* ",
            &[
                ("chore(deps): bump the rust toolchain", GitKind::Plain),
                ("56ec934f6", GitKind::Sha),
                ("2026-10-24", GitKind::Meta),
                ("huichen", GitKind::Meta),
            ],
        ),
    ];
    let detail = "commit 9e4a47d2a\nAuthor: huichen <huichen@example.invalid>\nDate:   2026-10-27\n\n    fix(mobile): stop folding code blocks — a long line pans instead\n\n mobile/src/components/markdown.tsx | 24 ++++++++++++++----------\n 1 file changed, 14 insertions(+), 10 deletions(-)\n\ndiff --git a/mobile/src/components/markdown.tsx b/mobile/src/components/markdown.tsx\n@@ -118,9 +118,13 @@ export function CodeBlock({ text }: Props) {\n-  return <Text numberOfLines={0}>{text}</Text>;\n+  // A code block pans; it never folds.\n+  return (\n+    <ScrollView horizontal>\n+      <Text>{text}</Text>\n+    </ScrollView>\n+  );"
        .lines()
        .map(str::to_string)
        .collect();
    GitView {
        repo: "future-os".into(),
        section: GitSection::Commits,
        rows: commits,
        selected: 0,
        detail,
        filter: None,
        error: None,
    }
}

/// One painted `/git` row: the gutter, then the fields that fit, each with what
/// it is so the frame paints it the way the real read would.
fn row(key: Option<&str>, prefix: &str, pieces: &[(&str, GitKind)]) -> GitRow {
    let pieces: Vec<(&str, GitKind)> = pieces
        .iter()
        .copied()
        .filter(|(piece, _)| !piece.is_empty())
        .collect();
    GitRow {
        key: key.map(str::to_string),
        prefix: prefix.to_string(),
        pieces: pieces
            .iter()
            .map(|(piece, _)| (*piece).to_string())
            .collect(),
        kinds: pieces.iter().map(|(_, kind)| *kind).collect(),
    }
}

pub fn preview_image() -> crate::workbench::state::PreviewView {
    use crate::workbench::state::PreviewView;
    PreviewView::Image {
        path: "/home/dev/future-os/design.png".into(),
        pixels: picture_pixels(),
    }
}

/// The pixels those pictures are drawn from: a two-axis gradient with a
/// diagonal band, large enough that fitting it into cells has a visible aspect
/// ratio and a zoom has something to cut into.
pub fn picture_pixels() -> crate::workbench::image_view::ImagePixels {
    let (width, height) = (96usize, 64usize);
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let band = ((x + y) / 8) % 2 == 0;
            let r = (x * 255 / width.max(1)) as u8;
            let g = (y * 255 / height.max(1)) as u8;
            let b = if band { 200 } else { 60 };
            rgba.extend_from_slice(&[r, g, b, 255]);
        }
    }
    crate::workbench::image_view::ImagePixels {
        width,
        height,
        rgba: rgba.into(),
    }
}

/// A workspace tree for the 文件 panel: a real checkout's shape, with one
/// directory open, so the panel's indent and its markers can be reviewed.
fn demo_tree() -> crate::workbench::files::FileTree {
    use crate::workbench::files::{FileNode, FileTree, NodeKind};
    let node = |path: &str, name: &str, depth: usize, kind: NodeKind, expanded: bool, size: u64| {
        FileNode {
            path: std::path::PathBuf::from(path),
            name: name.to_string(),
            depth,
            kind,
            expanded,
            size,
            truncated: false,
        }
    };
    FileTree::from_rows(
        "/home/dev/future-os",
        vec![
            node(
                "/home/dev/future-os/docs",
                "docs",
                0,
                NodeKind::Dir,
                true,
                0,
            ),
            node(
                "/home/dev/future-os/docs/tui",
                "tui",
                1,
                NodeKind::Dir,
                false,
                0,
            ),
            node(
                "/home/dev/future-os/docs/README.md",
                "README.md",
                1,
                NodeKind::File,
                false,
                4_096,
            ),
            node("/home/dev/future-os/tui", "tui", 0, NodeKind::Dir, false, 0),
            node(
                "/home/dev/future-os/AGENTS.md",
                "AGENTS.md",
                0,
                NodeKind::File,
                false,
                18_900,
            ),
            node(
                "/home/dev/future-os/Cargo.toml",
                "Cargo.toml",
                0,
                NodeKind::File,
                false,
                2_100,
            ),
            node(
                "/home/dev/future-os/design.png",
                "design.png",
                0,
                NodeKind::File,
                false,
                182_400,
            ),
        ],
    )
}

/// The models a believable `/model` picker lists: three providers, one of them
/// with two models, a context window of 1M and a small one — enough shapes for
/// the grouping, the capability column and the `✓` to be reviewed on screen.
fn demo_models() -> Vec<ModelChoice> {
    fn declared(levels: &[&str]) -> Option<Vec<String>> {
        Some(levels.iter().map(|level| level.to_string()).collect())
    }
    vec![
        ModelChoice {
            id: "deepseek-flash".into(),
            label: "deepseek-flash".into(),
            provider: "westlakehpc".into(),
            context_window: 1_000_000,
            thinking_level: "default".into(),
            thinking_levels: declared(&["default", "off", "low", "medium", "high", "xhigh"]),
            default_thinking_level: Some("default".into()),
            is_default: true,
        },
        ModelChoice {
            id: "Qwen".into(),
            label: "Qwen 3.8 27B".into(),
            provider: "westlakehpc".into(),
            context_window: 262_144,
            thinking_level: "default".into(),
            thinking_levels: declared(&["default", "off", "low", "medium", "high"]),
            default_thinking_level: Some("default".into()),
            is_default: false,
        },
        ModelChoice {
            id: "gpt-6-astra".into(),
            label: "gpt-6-astra".into(),
            provider: "azure".into(),
            context_window: 1_000_000,
            thinking_level: "default".into(),
            thinking_levels: declared(&["default", "medium", "high", "max"]),
            default_thinking_level: Some("default".into()),
            is_default: false,
        },
        // Declares only the implicit default: the picker's slider has one stop.
        ModelChoice {
            id: "glm-flash".into(),
            label: "glm-flash".into(),
            provider: "westlakehpc".into(),
            context_window: 200_000,
            thinking_level: "default".into(),
            thinking_levels: declared(&["default"]),
            default_thinking_level: Some("default".into()),
            is_default: false,
        },
        // Same shapes as the real `/models` list, including a level the product
        // has no translation for (`max` is known, but it is still a wire value).
        ModelChoice {
            id: "omlx-local".into(),
            label: "omlx (local)".into(),
            provider: "omlx".into(),
            context_window: 131_072,
            thinking_level: "default".into(),
            thinking_levels: declared(&[
                "default", "off", "minimal", "low", "medium", "high", "xhigh", "max",
            ]),
            default_thinking_level: Some("default".into()),
            is_default: false,
        },
    ]
}

/// The launchpad: nothing attached yet, so the header has no session and the
/// body is the composer.
///
/// It carries the product defaults a fresh screen starts from, not the values of
/// the conversation `/new` left behind: the plain chat (no workspace), the
/// agent's default model at the level it starts at, and 完全访问.
pub fn launchpad() -> WorkbenchData {
    let mut data = base();
    data.input.text.clear();
    data.sessions[0].current = false;
    // The workspace chip names the directory the first message would run in.
    data.session.cwd = "~/.future/agent/workspace".into();
    data.session.chat = true;
    // The agent's default model at the level it starts at, and 完全访问 — the
    // other three product defaults a fresh screen opens on.
    data.model = "westlakehpc/deepseek-flash".into();
    data.effort = "default".into();
    data.mode = "off".into();
    data
}

/// A conversation with the work folded away, which is what the default reading
/// level produces: one summary row above the answer it produced.
pub fn chat() -> WorkbenchData {
    let mut data = base();
    data.session = SessionMeta {
        id: "s1".into(),
        title: "修复 parser 的 off-by-one".into(),
        cwd: "~/future-os".into(),
        chat: false,
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
        // The process row the conversation renderer draws at the default level,
        // worded in the interface language of this demo (Chinese).
        style(p.muted, " ▸ 工具调用 2 次 · 思考 1 次"),
        String::new(),
        style(p.fg, " 循环用的是 `<=`，最后一位会越界读一个字节。"),
        style(p.fg, "改成 `<` 之后 41 个用例都过了。"),
        String::new(),
        style(p.muted, " ▸ 补一个回归测试"),
        style(p.muted, " read src/parser.rs"),
        style(p.muted, " $ cargo test -p parser --off-by-one"),
        String::new(),
    ]
    .into();
    data
}

/// Twelve workspaces and their conversations, to exercise the rail's window and
/// its "more" row.
pub fn workspaces() -> WorkbenchData {
    let mut data = chat();
    data.session.title = "workbench 侧栏验收".into();
    let names = [
        "future-os",
        "mobile",
        "futureos-wechat-articles",
        "agent-harness",
        "loop",
        "docs-site",
        "slides",
        "desktop",
        "remote-peers",
        "channels",
        "goal-plane",
        "screenshots",
    ];
    data.sessions = Vec::new();
    data.workspaces = Vec::new();
    for (index, name) in names.iter().enumerate() {
        let id = format!("w{index}");
        data.sessions.push(SessionRow {
            id: id.clone(),
            title: format!("{} 的第一条会话", name),
            workspace: format!("~/{name}"),
            age: format!("{}h", index + 1),
            current: index == 0,
            parent_id: None,
            streaming: false,
            created_at_ms: 0,
            // Two pinned conversations, so the 置顶 block is on screen: the
            // filing still lists them (the rail lifts them, not the filing).
            pinned: index < 2,
        });
        data.workspaces.push(WorkspaceGroup {
            name: (*name).to_string(),
            path: format!("/home/dev/{name}"),
            age: format!("{}h", index + 1),
            session_ids: vec![id],
        });
    }
    data.chats = Vec::new();
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scenario_is_reachable_by_name() {
        for scenario in [Scenario::Launchpad, Scenario::Chat, Scenario::Workspaces] {
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
        assert!(text.contains("工具调用 2 次 · 思考 1 次"), "{text}");
        assert!(text.contains("41 个用例都过了"), "{text}");
    }

    #[test]
    fn the_workspace_scenario_is_long_enough_to_scroll() {
        let data = workspaces();
        assert_eq!(data.workspaces.len(), 12);
        assert!(data.sessions.iter().any(|row| row.pinned));
        assert!(data.sessions.iter().any(|row| !row.pinned));
        for group in &data.workspaces {
            for id in &group.session_ids {
                assert!(data.sessions.iter().any(|session| &session.id == id));
            }
        }
    }

    #[test]
    fn the_demo_is_deterministic() {
        // Screenshot comparisons and hit-region assertions depend on this: two
        // builds of the same scenario must produce identical bytes.
        assert_eq!(chat(), chat());
        assert_eq!(launchpad(), launchpad());
        assert_eq!(workspaces(), workspaces());
    }
}
