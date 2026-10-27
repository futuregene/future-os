//! The workbench's data model.
//!
//! This is the contract's §5 `WorkbenchData`, kept dumb on purpose: plain
//! structs with no I/O. Whatever fills them — the demo builder today, the agent
//! adapter next — produces this one shape, and the screen paints from it. That
//! is what lets the whole surface be rendered and tested without an agent, and
//! what keeps the drawing code from growing a second source of truth for "how
//! many sessions are there".

use crate::workbench::files::FileTree;
use crate::workbench::i18n::{t, Key, Language};
use crate::workbench::image_view::ImagePixels;

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

/// One conversation in the sidebar's session panel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionRow {
    pub id: String,
    pub title: String,
    /// The workspace it is filed under, by name (`future-os`) — the rail and
    /// the session picker are too narrow for a path, and the path is the
    /// group's identity rather than the row's label.
    pub workspace: String,
    pub parent_id: Option<String>,
    /// How long ago it was last active (`5m`, `3h`).
    pub age: String,
    /// The session the workbench is attached to.
    pub current: bool,
}

/// One workspace in the 工作区 tab: a directory several conversations are filed
/// under, as the desktop's rail groups them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceGroup {
    /// The heading's text (the directory's own name).
    pub name: String,
    /// The directory, which is the group's identity — two checkouts can share a
    /// name.
    pub path: String,
    /// Above the unpinned workspaces, in the 置顶 section.
    pub pinned: bool,
    /// How long ago anything in it was refreshed (`5m`, `3h`).
    pub age: String,
    /// The conversations filed under it, most recently refreshed first.
    pub session_ids: Vec<String>,
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

/// One skill the `/` palette offers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillRow {
    pub name: String,
    pub description: String,
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

/// The sidebar's three panels, in tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelId {
    /// Every conversation by when it was refreshed — the default landing panel.
    #[default]
    Recent,
    /// The same conversations filed the way the desktop's rail files them.
    Workspaces,
    /// The workspace as a tree: directories open and close, files preview.
    Files,
}

impl PanelId {
    pub const ALL: [PanelId; 3] = [PanelId::Recent, PanelId::Workspaces, PanelId::Files];

    /// The tab label.
    pub fn label(self, lang: Language) -> &'static str {
        t(
            lang,
            match self {
                Self::Recent => Key::TabRecent,
                Self::Workspaces => Key::TabWorkspaces,
                Self::Files => Key::TabFiles,
            },
        )
    }

    /// The stable id used in hit regions and `/panel` arguments.
    pub fn id(self) -> &'static str {
        match self {
            Self::Recent => "recent",
            Self::Workspaces => "workspaces",
            Self::Files => "files",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|panel| panel.id().eq_ignore_ascii_case(id))
            // The old panel names keep working where they still mean something:
            // `sessions` is the list 最近 shows, and the rest of the five-panel
            // vocabulary is a way of asking for the file.
            .or(match id.to_ascii_lowercase().as_str() {
                "sessions" | "session" => Some(Self::Recent),
                "workspace" | "todo" | "jobs" | "usage" | "skills" => Some(Self::Workspaces),
                "file" | "tree" | "explorer" => Some(Self::Files),
                _ => None,
            })
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
    /// The approval mode the session runs under, as the agent spells it
    /// (`manual` | `sandbox` | `auto` | `off`) — the desktop composer's own four
    /// chips, not a fifth TUI-only vocabulary. There is no separate "permission
    /// level" here: the desktop exposes none either, and a control nobody else
    /// has is an invented option.
    pub mode: String,
    pub usage: UsageData,
    /// The conversations, most recently refreshed first. Both panels read this
    /// one list; 工作区 only adds the filing.
    pub sessions: Vec<SessionRow>,
    /// The workspace groups 工作区 files them under, pinned first.
    pub workspaces: Vec<WorkspaceGroup>,
    /// Conversations that belong to no workspace — the desktop's plain chats.
    /// Session ids, in the same order as `sessions`.
    pub chats: Vec<String>,
    /// Every model the agent offers, in the agent's order. The `/model` picker
    /// groups them by provider; an empty list is a picker that says so.
    pub models: Vec<ModelChoice>,
    /// The installed skills, for the `/` palette's skill half.
    pub skills: Vec<SkillRow>,
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
    /// The 文件 panel's tree of the session workspace.
    pub tree: FileTree,
    /// What the preview box is showing, if it is open — written by the adapter
    /// when the reader asks for a file, read by the surface to paint it.
    pub preview: Option<PreviewView>,
}

/// What the preview box holds: a picture's pixels, a document's text, or the
/// reason there is nothing to show.
///
/// The pixels travel with the frame rather than being re-read per paint: a
/// zoom is a redraw of the same bytes, and decoding on every keypress would
/// make `+` feel like a disk seek.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewView {
    Image {
        path: String,
        pixels: ImagePixels,
    },
    /// Markdown is kept as its source and rendered by the box, so the heading
    /// levels follow the theme in force like the transcript does.
    Markdown {
        path: String,
        text: String,
    },
    Text {
        path: String,
        lines: Vec<String>,
        /// The file was longer than the preview cap.
        truncated: bool,
    },
    Unavailable {
        path: String,
        reason: String,
    },
}

impl PreviewView {
    /// The path this preview is about — what the box's title names.
    pub fn path(&self) -> &str {
        match self {
            Self::Image { path, .. }
            | Self::Markdown { path, .. }
            | Self::Text { path, .. }
            | Self::Unavailable { path, .. } => path,
        }
    }

    /// The last path segment, for a title that fits the box.
    pub fn name(&self) -> &str {
        let path = self.path();
        let trimmed = path.trim_end_matches(['/', '\\']);
        trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed)
    }
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
        assert_eq!(PanelId::ALL.len(), 3);
        assert_eq!(PanelId::Recent.next(), PanelId::Workspaces);
        assert_eq!(PanelId::Workspaces.next(), PanelId::Files);
        assert_eq!(PanelId::Files.next(), PanelId::Recent);
        assert_eq!(PanelId::Recent.previous(), PanelId::Files);
        for panel in PanelId::ALL {
            assert_eq!(PanelId::parse(panel.id()), Some(panel));
            assert_eq!(PanelId::parse(&panel.id().to_uppercase()), Some(panel));
        }
        // The panel names the surface used to have still resolve: a `/panel
        // sessions` from habit lands somewhere real rather than nowhere.
        assert_eq!(PanelId::parse("sessions"), Some(PanelId::Recent));
        assert_eq!(PanelId::parse("workspace"), Some(PanelId::Workspaces));
        assert_eq!(PanelId::parse("files"), Some(PanelId::Files));
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
