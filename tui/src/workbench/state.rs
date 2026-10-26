//! The workbench's data model.
//!
//! This is the contract's §5 `WorkbenchData`, kept dumb on purpose: plain
//! structs with no I/O. Whatever fills them — the demo builder today, the agent
//! adapter next — produces this one shape, and the screen paints from it. That
//! is what lets the whole surface be rendered and tested without an agent, and
//! what keeps the drawing code from growing a second source of truth for "how
//! many sessions are there".

use crate::workbench::i18n::{t, Key, Language};

/// Who the session is: the four facts the header and the launchpad print.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionMeta {
    /// Empty means "no session yet" — which is exactly the launchpad state.
    pub id: String,
    pub title: String,
    pub cwd: String,
    /// Short git branch name, or empty outside a repository.
    pub branch: String,
}

/// Usage figures for the status bar and the usage panel.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct UsageData {
    pub ctx_used: u64,
    pub ctx_total: u64,
    pub cost: f64,
    /// Cache hit rate, 0..=100.
    pub cache_pct: u8,
    /// Output tokens per second of the run in flight (0 when idle).
    pub tps: f64,
}

impl UsageData {
    /// Context pressure as a fraction, clamped to 0..=1 (a model that reports a
    /// window of 0 must not divide by zero).
    pub fn context_fraction(&self) -> f64 {
        if self.ctx_total == 0 {
            0.0
        } else {
            (self.ctx_used as f64 / self.ctx_total as f64).clamp(0.0, 1.0)
        }
    }
}

/// How far a goal todo has got, as the panel's one-cell marker says it.
///
/// The four states are the goal control plane's own vocabulary folded down to
/// what fits beside a line of text: `✔` finished, `◉` claimed and running, `○`
/// waiting, `✖` blocked or superseded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TodoState {
    #[default]
    Open,
    /// Claimed by an agent (a live lease) right now.
    Active,
    Done,
    /// Blocked by a gate or a failed dependency.
    Blocked,
    /// Superseded or cancelled: it will not happen.
    Cancelled,
}

impl TodoState {
    /// The one-cell marker the panel prints before the text.
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Open => "○",
            Self::Active => "◉",
            Self::Done => "✔",
            Self::Blocked | Self::Cancelled => "✖",
        }
    }

    pub fn is_done(self) -> bool {
        self == Self::Done
    }

    /// Still worth counting as outstanding (`○` and `◉`).
    pub fn is_open(self) -> bool {
        matches!(self, Self::Open | Self::Active)
    }
}

/// One goal todo, as the sidebar shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TodoItem {
    /// The control plane's todo id (empty for a fixture that names none).
    pub id: String,
    pub text: String,
    /// `P0`…`P3`, or empty when the store recorded none.
    pub priority: String,
    pub state: TodoState,
}

/// One conversation in the sidebar's session panel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionRow {
    pub id: String,
    pub title: String,
    /// The workspace it is filed under (`~/future-os`).
    pub workspace: String,
    pub parent_id: Option<String>,
    /// How long ago it was last active (`5m`, `3h`).
    pub age: String,
    /// The session the workbench is attached to.
    pub current: bool,
}

/// One model the `/model` picker offers.
///
/// The agent's `list_models` answers with a flat list of these (see
/// `rpc::types::ModelInfo`); the picker groups them by `provider`, so the
/// provider has to travel with the row rather than be re-derived from the model
/// id (a provider is not always its id's prefix).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelChoice {
    pub id: String,
    /// Display name; falls back to the id when the agent names none.
    pub label: String,
    pub provider: String,
    /// Context window in tokens (0 when the agent reported none).
    pub context_window: u64,
    /// The level the agent would run this model at by default.
    pub thinking_level: String,
}

/// The reasoning levels a model can run at, weakest first.
///
/// One list for the whole crate: `index.rs` parses `--model sonnet:high`
/// against the same six names, and the `/model` picker's slider walks them.
pub const THINKING_LEVELS: [&str; 6] = ["off", "minimal", "low", "medium", "high", "xhigh"];

/// The position of `level` in [`THINKING_LEVELS`]; an unknown level is `off`, so
/// a slider always has a defined place to stand.
pub fn thinking_index(level: &str) -> usize {
    THINKING_LEVELS
        .iter()
        .position(|known| known.eq_ignore_ascii_case(level))
        .unwrap_or(0)
}

/// One skill in the sidebar's skills panel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillRow {
    pub name: String,
    pub description: String,
}

/// One scheduled job in the sidebar's jobs panel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobRow {
    pub name: String,
    pub state: String,
}

/// What the input box holds. The editor itself lives in `Live`; this snapshot
/// keeps all renderers free of editing and I/O.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputView {
    pub text: String,
    pub cursor: usize,
    /// Shown while `text` is empty.
    pub placeholder: String,
    #[doc(hidden)]
    pub editor: super::editor::EditorState,
}

impl InputView {
    pub fn from_editor(editor: super::editor::EditorState, placeholder: String) -> Self {
        Self {
            text: editor.text().to_string(),
            cursor: editor.cursor(),
            placeholder,
            editor,
        }
    }
    /// Keyboard navigation uses the last rendered width; this fallback is only
    /// for input arriving before the first frame.
    pub fn editor_width(&self, frame_width: u16) -> usize {
        (frame_width as usize).saturating_sub(44).clamp(8, 76)
    }
}

impl InputView {
    /// Insert `insert` at the cursor and leave the cursor after it.
    pub fn insert(&mut self, insert: &str) {
        let at = self.cursor.min(self.text.len());
        // Never split a multi-byte character: a cursor can only be reported in
        // the middle of one by a stale client, and slicing there would panic.
        let at = floor_char_boundary(&self.text, at);
        self.text.insert_str(at, insert);
        self.cursor = at + insert.len();
    }

    /// Delete the character before the cursor.
    pub fn backspace(&mut self) {
        let at = floor_char_boundary(&self.text, self.cursor.min(self.text.len()));
        if at == 0 {
            return;
        }
        let start = self.text[..at]
            .char_indices()
            .next_back()
            .map(|(index, _)| index)
            .unwrap_or(0);
        self.text.replace_range(start..at, "");
        self.cursor = start;
    }

    pub fn move_left(&mut self) {
        let at = floor_char_boundary(&self.text, self.cursor.min(self.text.len()));
        self.cursor = self.text[..at]
            .char_indices()
            .next_back()
            .map(|(index, _)| index)
            .unwrap_or(0);
    }

    pub fn move_right(&mut self) {
        let at = floor_char_boundary(&self.text, self.cursor.min(self.text.len()));
        self.cursor = self.text[at..]
            .char_indices()
            .nth(1)
            .map(|(index, _)| at + index)
            .unwrap_or(self.text.len());
    }
}

fn floor_char_boundary(text: &str, mut at: usize) -> usize {
    at = at.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// What the agent is doing right now — the activity row's source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActivityKind {
    #[default]
    Idle,
    Thinking,
    Tool,
    Streaming,
}

impl ActivityKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Thinking => "thinking",
            Self::Tool => "tool",
            Self::Streaming => "streaming",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Activity {
    pub kind: ActivityKind,
    /// Free-form detail (`read src/main.rs`).
    pub detail: String,
    pub elapsed_secs: u64,
    pub tools: usize,
}

impl Activity {
    /// Is the spinner supposed to spin? A still screen must not burn frames.
    pub fn is_busy(&self) -> bool {
        self.kind != ActivityKind::Idle
    }
}

/// The sidebar's five panels, in tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelId {
    #[default]
    Todo,
    Jobs,
    Sessions,
    Usage,
    Skills,
}

impl PanelId {
    pub const ALL: [PanelId; 5] = [
        PanelId::Todo,
        PanelId::Jobs,
        PanelId::Sessions,
        PanelId::Usage,
        PanelId::Skills,
    ];

    /// The tab label (the contract's `待办 任务 会话 用量 技能`, and its English
    /// reading).
    pub fn label(self, lang: Language) -> &'static str {
        t(
            lang,
            match self {
                Self::Todo => Key::TabTodo,
                Self::Jobs => Key::TabJobs,
                Self::Sessions => Key::TabSessions,
                Self::Usage => Key::TabUsage,
                Self::Skills => Key::TabSkills,
            },
        )
    }

    /// The stable id used in key bindings, hit regions and `/panel` arguments.
    pub fn id(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::Jobs => "jobs",
            Self::Sessions => "sessions",
            Self::Usage => "usage",
            Self::Skills => "skills",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|panel| panel.id().eq_ignore_ascii_case(id))
    }

    pub fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|panel| *panel == self)
            .unwrap_or(0)
    }

    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    pub fn previous(self) -> Self {
        let count = Self::ALL.len();
        Self::ALL[(self.index() + count - 1) % count]
    }
}

/// Everything the workbench paints.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorkbenchData {
    pub app_name: String,
    pub version: String,
    pub connected: bool,
    pub session: SessionMeta,
    pub model: String,
    pub effort: String,
    pub mode: String,
    pub permission: String,
    pub usage: UsageData,
    pub todos: Vec<TodoItem>,
    pub sessions: Vec<SessionRow>,
    /// Every model the agent offers, in the agent's order. The `/model` picker
    /// groups them by provider; an empty list is a picker that says so.
    pub models: Vec<ModelChoice>,
    pub skills: Vec<SkillRow>,
    pub jobs: Vec<JobRow>,
    /// Already-rendered ANSI rows of the conversation (see `ChatArea`). The
    /// workbench never parses markdown itself: the same renderer the inline TUI
    /// uses produces these, so a reply looks the same in both.
    pub messages: Vec<String>,
    pub input: InputView,
    pub activity: Activity,
    /// A failed request or command, shown in place of the hint row until
    /// something succeeds. Distinct from the transcript: it is about the
    /// surface's own last action, not part of the conversation.
    pub notice: Option<String>,
    /// The agent's authoritative queued runs, in its queue order.
    pub queue: Vec<QueuedPrompt>,
    /// Increases when transcript content changes; lets the view preserve a
    /// reader's position without guessing from activity labels.
    pub revision: u64,
    /// Mouse capture can be released for native selection/copy.
    pub mouse_capture: bool,
    /// The pending approval, including the agent's curated risk/action data.
    pub approval: Option<ApprovalView>,
    /// Read-only git working-tree review.
    pub diff: Option<DiffView>,
    /// Real user-message fork points from `get_fork_messages`.
    pub fork_points: Vec<ForkPoint>,
    /// Read-only git worktree list.
    pub worktrees: Vec<WorktreeView>,
    /// Files a `@` mention can name, relative to the session's working
    /// directory. Filled by the adapter once per directory; empty disables the
    /// menu's file half rather than suggesting nothing.
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ApprovalView {
    pub request_id: String,
    pub tool: String,
    pub kind: String,
    pub risk: String,
    pub title: String,
    pub summary: String,
    pub action: String,
    pub requested_action: String,
    pub sandbox_boundary: String,
    /// Agent-suggested path/access rules for this run's workspace.
    pub rules: Vec<AllowRule>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AllowRule {
    pub path: String,
    pub access: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffView {
    pub files: Vec<super::diff_review::DiffFile>,
    pub selected: usize,
    pub patch: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForkPoint {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorktreeView {
    pub path: String,
    pub label: String,
    pub description: String,
    pub current: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueuedPrompt {
    pub run_id: String,
    pub text: String,
}

impl WorkbenchData {
    /// Is this the launchpad (no session attached yet)?
    pub fn on_launchpad(&self) -> bool {
        self.session.id.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(text: &str, cursor: usize) -> InputView {
        InputView {
            text: text.into(),
            cursor,
            placeholder: String::new(),
            editor: Default::default(),
        }
    }

    #[test]
    fn editing_respects_multibyte_boundaries() {
        let mut view = input("中文", 1); // a byte inside the first character
        view.insert("x");
        assert_eq!(view.text, "x中文");
        assert_eq!(view.cursor, 1);

        // A cursor on a character boundary deletes the character before it…
        let mut view = input("中文", 3);
        view.backspace();
        assert_eq!(view.text, "文");
        assert_eq!(view.cursor, 0);
        // …and one reported inside a character floors to the boundary, so it
        // deletes nothing rather than half a glyph.
        let mut view = input("中文", 1);
        view.backspace();
        assert_eq!(view.text, "中文");
    }

    #[test]
    fn the_cursor_walks_characters_not_bytes() {
        let mut view = input("a中b", 0);
        view.move_right();
        assert_eq!(view.cursor, 1);
        view.move_right();
        assert_eq!(view.cursor, 4);
        view.move_right();
        assert_eq!(view.cursor, 5);
        view.move_right();
        assert_eq!(view.cursor, 5, "right stops at the end");
        view.move_left();
        assert_eq!(view.cursor, 4);
    }

    #[test]
    fn inserting_at_the_end_of_a_long_draft_keeps_the_cursor_after_it() {
        let mut view = input("", 0);
        for ch in "hello".chars() {
            view.insert(&ch.to_string());
        }
        assert_eq!(view.text, "hello");
        assert_eq!(view.cursor, 5);
    }

    #[test]
    fn panels_cycle_in_the_contracts_order() {
        assert_eq!(PanelId::ALL.len(), 5);
        assert_eq!(PanelId::Todo.next(), PanelId::Jobs);
        assert_eq!(PanelId::Skills.next(), PanelId::Todo);
        assert_eq!(PanelId::Todo.previous(), PanelId::Skills);
        for panel in PanelId::ALL {
            assert_eq!(PanelId::parse(panel.id()), Some(panel));
            assert_eq!(PanelId::parse(&panel.id().to_uppercase()), Some(panel));
        }
        assert_eq!(PanelId::parse("nope"), None);
    }

    #[test]
    fn an_empty_session_is_the_launchpad() {
        let mut data = WorkbenchData::default();
        assert!(data.on_launchpad());
        data.session.id = "s1".into();
        assert!(!data.on_launchpad());
    }

    #[test]
    fn the_reasoning_slider_walks_the_crates_own_levels() {
        assert_eq!(THINKING_LEVELS.len(), 6);
        assert_eq!(THINKING_LEVELS[0], "off");
        assert_eq!(THINKING_LEVELS[5], "xhigh");
        for (index, level) in THINKING_LEVELS.iter().enumerate() {
            assert_eq!(thinking_index(level), index);
            assert_eq!(thinking_index(&level.to_uppercase()), index);
        }
        // A level an older agent invented has a defined place to stand.
        assert_eq!(thinking_index(""), 0);
        assert_eq!(thinking_index("galaxy-brain"), 0);
    }

    #[test]
    fn a_zero_context_window_does_not_divide_by_zero() {
        assert_eq!(UsageData::default().context_fraction(), 0.0);
        let usage = UsageData {
            ctx_used: 6_900,
            ctx_total: 1_000_000,
            ..Default::default()
        };
        assert!((usage.context_fraction() - 0.0069).abs() < 1e-9);
        // A model that reports more used than total still draws a full bar.
        let over = UsageData {
            ctx_used: 20,
            ctx_total: 10,
            ..Default::default()
        };
        assert_eq!(over.context_fraction(), 1.0);
    }

    #[test]
    fn only_a_busy_activity_spins() {
        assert!(!Activity::default().is_busy());
        let thinking = Activity {
            kind: ActivityKind::Thinking,
            ..Default::default()
        };
        assert!(thinking.is_busy());
        assert_eq!(thinking.kind.label(), "thinking");
    }
}
