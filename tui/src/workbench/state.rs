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
use crate::workbench::syntax;

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
    /// The agent reports a run in flight in this conversation — the row draws
    /// the spinner, and `最近` files it under 活跃.
    ///
    /// The desktop's rail shows the same fact from its own run ledger; the TUI
    /// has only what `list_sessions` reports, which is a live per-session flag
    /// on the agent (`is_streaming`), so a run another client started shows up
    /// here too — as long as the list is re-read while it runs (see
    /// `run::SESSIONS_POLL`).
    pub streaming: bool,
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
    /// The level the agent starts a conversation with on this model
    /// (`thinkingLevel`; `"default"` on a current agent, the legacy
    /// `"high"`/`"off"` on an older one).
    pub thinking_level: String,
    /// The levels this model accepts, exactly as the agent declared them: the
    /// implicit [`DEFAULT_THINKING_LEVEL_NAME`] first, then wire values sent
    /// verbatim. `None` is an agent that predates the field entirely (an older
    /// agent, whose models fall back to the legacy six levels); `Some([])` never
    /// happens on a current agent, which always sends at least `["default"]`.
    pub thinking_levels: Option<Vec<String>>,
    /// The level the agent starts a conversation at on this model, already
    /// clamped to [`Self::thinking_levels`]. `None` on an older agent.
    pub default_thinking_level: Option<String>,
}

impl ModelChoice {
    /// The levels the picker offers for this model, in display order.
    ///
    /// A current agent's declaration is taken as-is (it is already
    /// default-first). An older agent never sent one, so the legacy six levels
    /// are offered with the implicit `default` in front — the shape every
    /// current model has. The empty list a current agent cannot send is
    /// normalized to the implicit `default` alone.
    pub fn effective_thinking_levels(&self) -> Vec<String> {
        effective_thinking_levels(self.thinking_levels.as_deref())
    }

    /// True for a model listed by an agent that predates `thinkingLevels`: the
    /// level vocabulary is the legacy six, and `default` must not travel on the
    /// wire (see [`legacy_wire_thinking_level`]).
    pub fn is_legacy_thinking(&self) -> bool {
        self.thinking_levels.is_none()
    }

    /// The level to actually send for this model. An older agent does not know
    /// the reserved `default`, so it is mapped to the legacy fallback
    /// (`medium`); a current agent receives the level verbatim.
    pub fn wire_thinking_level(&self, level: &str) -> String {
        if self.is_legacy_thinking() {
            legacy_wire_thinking_level(level).to_string()
        } else {
            level.to_string()
        }
    }
}

/// The stable reference for a model: `provider/id` where there is a provider.
///
/// A catalog id is provider-local even when it happens to contain a slash, so
/// the provider is prefixed rather than inferred from the id. This is the
/// spelling the desktop and phone clients send, and the one the picker applies:
/// a bare id is ambiguous the moment two providers offer it — the agent
/// resolves it against its own order (user-defined `models.json` entries before
/// the built-in catalogue), so `deepseek-flash` picked from `future` silently
/// became `westlakehpc/deepseek-flash`.
pub fn model_reference(provider: &str, id: &str) -> String {
    if provider.is_empty() {
        id.to_string()
    } else {
        format!("{provider}/{id}")
    }
}

/// The reserved level meaning "send no thinking/effort parameter at all".
///
/// It is implicit on a current agent: every model's declared list starts with
/// it, it is never stored in `models.json`, and it is the fallback for a level
/// the model does not offer.
pub const DEFAULT_THINKING_LEVEL_NAME: &str = "default";

/// Level names the product has translations for. Not a wire vocabulary: a model
/// may declare any other string (`max` included) and it is sent verbatim. These
/// six are the legacy fallback an older agent's models run on.
pub const KNOWN_THINKING_LEVELS: [&str; 6] = ["off", "minimal", "low", "medium", "high", "xhigh"];

/// Level names the CLI accepts besides the legacy six: the reserved `default`
/// (`--thinking default` asks for the provider default) and the wire value
/// `max` (a model may declare it; the agent clamps anything it does not offer).
pub const EXTRA_THINKING_LEVELS: [&str; 2] = [DEFAULT_THINKING_LEVEL_NAME, "max"];

/// Whether `level` is a spelling `--thinking` / `--model id:level` accepts: the
/// legacy six, the reserved `default`, or `max`. A model may declare any other
/// string and the TUI sends it verbatim from a picker — this only governs what
/// a user can type on the command line.
pub fn is_valid_thinking_level(level: &str) -> bool {
    EXTRA_THINKING_LEVELS.contains(&level) || KNOWN_THINKING_LEVELS.contains(&level)
}

/// The level vocabulary for a model that declared none (an agent that predates
/// `thinkingLevels`): the implicit `default` followed by the legacy six.
pub fn fallback_thinking_levels() -> Vec<String> {
    std::iter::once(DEFAULT_THINKING_LEVEL_NAME.to_string())
        .chain(KNOWN_THINKING_LEVELS.iter().map(|level| level.to_string()))
        .collect()
}

/// The levels a model offers, from what the agent declared.
///
/// `None` — the field is absent, i.e. an older agent — falls back to
/// [`fallback_thinking_levels`]. `Some([])` is normalized to the implicit
/// `default` alone: a current agent always declares at least that, and a
/// picker with an empty ring would have nowhere to stand.
pub fn effective_thinking_levels(declared: Option<&[String]>) -> Vec<String> {
    match declared {
        None => fallback_thinking_levels(),
        Some([]) => vec![DEFAULT_THINKING_LEVEL_NAME.to_string()],
        Some(levels) => levels.to_vec(),
    }
}

/// The position of `level` in `levels`, matched case-insensitively; an unknown
/// level starts at the top of the ring (`default`), so a slider always has a
/// defined place to stand.
pub fn thinking_index_in(levels: &[String], level: &str) -> usize {
    levels
        .iter()
        .position(|known| known.eq_ignore_ascii_case(level.trim()))
        .unwrap_or(0)
}

/// The next level in a model's ring.
///
/// A level inside the ring advances to the one after it, wrapping; an unknown
/// current level (nothing staged yet) starts the ring at its first entry, which
/// is always `default`. `None` when the ring has no entries.
pub fn next_thinking_level(levels: &[String], current: &str) -> Option<String> {
    let current = current.trim();
    if current.is_empty() {
        return levels.first().cloned();
    }
    match levels
        .iter()
        .position(|level| level.eq_ignore_ascii_case(current))
    {
        Some(index) => Some(levels[(index + 1) % levels.len()].clone()),
        None => levels.first().cloned(),
    }
}

/// The wire value for `level` on a model from an older agent.
///
/// An older agent does not offer the reserved `default` and rejects (or
/// misreads) it, so the contract's legacy fallback is sent instead: `medium`.
pub fn legacy_wire_thinking_level(level: &str) -> &str {
    if level
        .trim()
        .eq_ignore_ascii_case(DEFAULT_THINKING_LEVEL_NAME)
    {
        // `medium` is the legacy six's fallback (the contract's "default medium").
        "medium"
    } else {
        level
    }
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
        /// The file's lines, already split and — for a language this build
        /// knows — classified into coloured runs (`syntax::highlight`), so the
        /// box paints a line without scanning it again on every frame.
        ///
        /// Shared rather than owned: the adapter rebuilds this data once per
        /// event and once per frame, and a 500 KB preview is not something to
        /// copy that often.
        lines: std::sync::Arc<[syntax::Line]>,
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
    fn the_thinking_levels_fallback_is_default_plus_the_legacy_six() {
        assert_eq!(KNOWN_THINKING_LEVELS.len(), 6);
        assert_eq!(KNOWN_THINKING_LEVELS[0], "off");
        assert_eq!(KNOWN_THINKING_LEVELS[5], "xhigh");
        let fallback = fallback_thinking_levels();
        assert_eq!(fallback.len(), 7);
        assert_eq!(fallback[0], DEFAULT_THINKING_LEVEL_NAME);
        assert_eq!(
            fallback[1..].iter().map(String::as_str).collect::<Vec<_>>(),
            KNOWN_THINKING_LEVELS.to_vec()
        );
        // An absent declaration (an older agent) is the fallback…
        assert_eq!(effective_thinking_levels(None), fallback);
        // …a declared list is taken as-is…
        let declared = vec![
            DEFAULT_THINKING_LEVEL_NAME.to_string(),
            "off".to_string(),
            "max".to_string(),
        ];
        assert_eq!(effective_thinking_levels(Some(&declared)), declared);
        // …and the empty list only a buggy agent could send becomes the
        // implicit default alone, never an empty ring.
        assert_eq!(
            effective_thinking_levels(Some(&[])),
            vec![DEFAULT_THINKING_LEVEL_NAME.to_string()]
        );
        for (index, level) in fallback.iter().enumerate() {
            assert_eq!(thinking_index_in(&fallback, level), index);
            assert_eq!(thinking_index_in(&fallback, &level.to_uppercase()), index);
        }
        // A level an older agent invented starts at the top of the ring.
        assert_eq!(thinking_index_in(&fallback, ""), 0);
        assert_eq!(thinking_index_in(&fallback, "galaxy-brain"), 0);
    }

    #[test]
    fn the_cli_accepts_default_and_max_on_top_of_the_legacy_six() {
        for level in KNOWN_THINKING_LEVELS
            .iter()
            .copied()
            .chain(["default", "max"])
        {
            assert!(is_valid_thinking_level(level), "{level}");
        }
        // Not a level name, so a model id containing a colon stays whole.
        assert!(!is_valid_thinking_level("beta"));
        assert!(!is_valid_thinking_level(""));
        assert!(!is_valid_thinking_level("Default"), "the wire is lowercase");
    }

    #[test]
    fn the_ring_walks_forward_and_wraps() {
        let levels = fallback_thinking_levels();
        assert_eq!(
            next_thinking_level(&levels, DEFAULT_THINKING_LEVEL_NAME).as_deref(),
            Some("off")
        );
        assert_eq!(
            next_thinking_level(&levels, "medium").as_deref(),
            Some("high")
        );
        assert_eq!(
            next_thinking_level(&levels, "xhigh").as_deref(),
            Some(DEFAULT_THINKING_LEVEL_NAME),
            "the ring wraps"
        );
        // An unset or unknown level starts the ring at its first entry, which is
        // `default`: that is the level a launchpad begins at.
        assert_eq!(
            next_thinking_level(&levels, "").as_deref(),
            Some(DEFAULT_THINKING_LEVEL_NAME)
        );
        assert_eq!(
            next_thinking_level(&levels, "galaxy-brain").as_deref(),
            Some(DEFAULT_THINKING_LEVEL_NAME)
        );
        assert_eq!(next_thinking_level(&[], "medium"), None);
    }

    #[test]
    fn the_reserved_default_never_travels_to_an_older_agent() {
        let current = ModelChoice {
            thinking_levels: Some(vec![
                DEFAULT_THINKING_LEVEL_NAME.to_string(),
                "high".to_string(),
            ]),
            ..ModelChoice::default()
        };
        assert_eq!(current.wire_thinking_level("default"), "default");
        assert_eq!(current.wire_thinking_level("high"), "high");
        assert!(!current.is_legacy_thinking());

        let legacy = ModelChoice::default();
        assert!(legacy.is_legacy_thinking());
        assert_eq!(legacy.wire_thinking_level("default"), "medium");
        assert_eq!(legacy.wire_thinking_level("high"), "high");
        assert_eq!(legacy.wire_thinking_level("max"), "max");
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
