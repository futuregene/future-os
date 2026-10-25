//! The workbench's data model.
//!
//! This is the contract's §5 `WorkbenchData`, kept dumb on purpose: plain
//! structs with no I/O. Whatever fills them — the demo builder today, the agent
//! adapter next — produces this one shape, and the screen paints from it. That
//! is what lets the whole surface be rendered and tested without an agent, and
//! what keeps the drawing code from growing a second source of truth for "how
//! many sessions are there".

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

/// One goal todo, as the sidebar shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TodoItem {
    pub text: String,
    pub done: bool,
    /// The item a goal is currently working on.
    pub active: bool,
}

/// One conversation in the sidebar's session panel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionRow {
    pub id: String,
    pub title: String,
    /// The workspace it is filed under (`~/future-os`).
    pub workspace: String,
    /// How long ago it was last active (`5m`, `3h`).
    pub age: String,
    /// The session the workbench is attached to.
    pub current: bool,
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

/// What the input box holds. The cursor is a byte offset into `text`, the same
/// unit the agent's request takes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputView {
    pub text: String,
    pub cursor: usize,
    /// Shown while `text` is empty.
    pub placeholder: String,
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

    /// The tab label (the contract's `待办 任务 会话 用量 技能`).
    pub fn label(self) -> &'static str {
        match self {
            Self::Todo => "待办",
            Self::Jobs => "任务",
            Self::Sessions => "会话",
            Self::Usage => "用量",
            Self::Skills => "技能",
        }
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
    pub skills: Vec<SkillRow>,
    pub jobs: Vec<JobRow>,
    /// Already-rendered ANSI rows of the conversation (see `ChatArea`). The
    /// workbench never parses markdown itself: the same renderer the inline TUI
    /// uses produces these, so a reply looks the same in both.
    pub messages: Vec<String>,
    pub input: InputView,
    pub activity: Activity,
    pub palette: &'static str,
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
