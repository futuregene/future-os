//! ChatArea — scrollable chat view matching the TS style. 1:1 port of
//! `tui/src/components/chat-area.ts`.
//!
//! Renders messages with proper markdown, tool output, and streaming,
//! including the deferred re-render queue and the streaming prefix cache
//! (both TS streaming optimizations).

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

use regex::Regex;

use crate::components::diff::{self, DiffTheme};
use crate::components::markdown::{MarkdownRenderer, MarkdownTheme, MarkdownThemePartial};
use crate::theme::{bold, dim, fg, italic, Theme, DARK_THEME};
use crate::tui::{Component, RESET};
use crate::utils::{
    apply_background_to_line, strip_ansi_codes, truncate_to_width, wrap_text_with_ansi,
    TruncateOptions,
};

/// Body rows a *failed* call shows while collapsed — the only body a collapsed
/// entry renders at all; `ctrl+g` expands every body to the hard cap below.
const COLLAPSED_TOOL_OUTPUT_ROWS: usize = 4;
/// Hard cap for an expanded tool-output body — a `cat` of a huge file must not
/// make the diff renderer walk megabytes on every frame.
const EXPANDED_TOOL_OUTPUT_ROWS: usize = 200;

// ─── Types ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
    System,
    Tool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Running,
    Complete,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Queued,
    Running,
    Terminal,
    Failed,
    Cancelled,
    Superseded,
    LostOnAgentRestart,
}

impl RunState {
    /// The string rendered by `dim(msg.runState)` (the TS stores the label
    /// verbatim on the message).
    pub fn label(&self) -> &'static str {
        match self {
            RunState::Queued => "queued",
            RunState::Running => "running",
            RunState::Terminal => "terminal",
            RunState::Failed => "failed",
            RunState::Cancelled => "cancelled",
            RunState::Superseded => "superseded",
            RunState::LostOnAgentRestart => "lost_on_agent_restart",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub id: String,
    pub role: ChatRole,
    pub content: String,
    pub name: Option<String>,      // tool name
    pub tool: Option<String>,      // tool call id
    pub tool_args: Option<String>, // tool arguments (JSON, for display)
    pub tool_status: Option<ToolStatus>,
    pub exit_code: Option<i32>,
    pub timestamp: Option<u64>,
    pub thinking: Option<String>,
    pub pending: bool, // streaming in progress
    pub stopped: bool, // generation was interrupted
    pub welcome: bool, // skip prefix/icon for welcome messages
    pub run_id: Option<String>,
    pub run_state: Option<RunState>,
    pub queue_position: Option<u32>,
}

impl ChatMessage {
    pub fn new(id: String, role: ChatRole, content: &str) -> Self {
        ChatMessage {
            id,
            role,
            content: content.to_string(),
            name: None,
            tool: None,
            tool_args: None,
            tool_status: None,
            exit_code: None,
            timestamp: None,
            thinking: None,
            pending: false,
            stopped: false,
            welcome: false,
            run_id: None,
            run_state: None,
            queue_position: None,
        }
    }
}

#[derive(Debug, Clone)]
struct RenderedLine {
    text: String,
    // Written but never read by ChatArea itself; the TS stores it for the
    // app-layer overlay renderer (P3). Kept for structural parity.
    #[allow(dead_code)]
    dim: bool,
}

#[derive(Debug, Clone)]
struct StreamRenderCache {
    cut: usize,   // char offset up to which `lines` was rendered
    text: String, // the rendered prefix, for startsWith validation
    lines: Vec<String>,
}

// ─── Compact activity (ctrl+d) ────────────────────────────────────────────

/// Marks a folded row — a run of calls or thinking standing in for many rows.
const FOLD_MARKER: &str = "▸";

/// What kind of thing a message folds into. Folding is per kind: two reads fold
/// together, a read and a shell call do not (the same rule the desktop's
/// collapsed bursts use, so a fold always means one homogeneous run).
#[derive(Debug, Clone, PartialEq, Eq)]
enum FoldKind {
    /// A completed call to the named tool.
    Tool(String),
    /// A thinking block with no visible answer text of its own.
    Thinking,
}

/// Does this message fold into the run above it?
///
/// Only *completed* calls and *thinking-only* messages fold. A running or failed
/// call passes through, which is what keeps a fold from hiding work in progress
/// or a failure reason, and it also breaks the run around it.
fn fold_kind(msg: &ChatMessage, fold_tools: bool) -> Option<FoldKind> {
    match msg.role {
        ChatRole::Tool if fold_tools && msg.tool_status == Some(ToolStatus::Complete) => {
            Some(FoldKind::Tool(tool_name_of(msg).to_string()))
        }
        ChatRole::Assistant
            if msg.content.trim().is_empty()
                && msg
                    .thinking
                    .as_deref()
                    .is_some_and(|t| !t.trim().is_empty()) =>
        {
            Some(FoldKind::Thinking)
        }
        _ => None,
    }
}

/// The tool a call row belongs to: its name, or the call id when the agent sent
/// none (a replay of an old journal).
fn tool_name_of(msg: &ChatMessage) -> &str {
    msg.name
        .as_deref()
        .or(msg.tool.as_deref())
        .unwrap_or("tool")
}

/// The distinct targets (file paths) of a folded run of file-tool calls, plus
/// the number of calls whose arguments carry no readable path.
///
/// Counting distinct files is what keeps the folded row's noun honest: a burst
/// that read the same file four times says `read 1 file`, because that is what
/// it did — the expanded rows are the same file four times over. The unreadable
/// calls are added back on top, so a call the fold cannot describe is still
/// counted.
fn tool_targets(run: &[ChatMessage], tool_name: &str) -> (Vec<String>, usize) {
    let is_file_tool = matches!(tool_name, "read" | "write" | "edit");
    let mut seen = std::collections::HashSet::new();
    let mut targets = Vec::new();
    let mut unreadable = 0usize;
    for msg in run {
        let target = if is_file_tool {
            msg.tool_args
                .as_deref()
                .and_then(|args| serde_json::from_str::<serde_json::Value>(args).ok())
                .and_then(|args| {
                    ["path", "file_path", "filePath"]
                        .into_iter()
                        .find_map(|key| args.get(key).and_then(serde_json::Value::as_str))
                        .filter(|path| !path.is_empty())
                        .map(str::to_owned)
                })
        } else {
            None
        };
        match target {
            Some(target) if seen.insert(target.clone()) => targets.push(target),
            Some(_) => {}
            None => unreadable += 1,
        }
    }
    (targets, unreadable)
}

/// Is `fresh` the layout `old` describes, plus standalone messages at the end?
///
/// Appending a message that renders its own rows leaves every message already
/// on screen exactly as it was, so the incremental paths can append it. A fold —
/// or a fold dissolving — rewrites an *earlier* message's rows, which the
/// incremental paths cannot express.
fn compact_layout_extends(old: &[usize], fresh: &[usize]) -> bool {
    old.len() <= fresh.len()
        && old.iter().zip(fresh).all(|(was, now)| was == now)
        && fresh[old.len()..]
            .iter()
            .enumerate()
            .all(|(offset, head)| *head == old.len() + offset)
}

// ─── Streaming helpers (ported regexes + findStreamCut) ───────────────────

/// `^ {0,3}(`{3,}|~{3,})` — fence opener inside stream cut scanning.
static STREAM_FENCE_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
/// `^ {0,3}\[[^\]\n]+\]:\s*\S` (m flag) — link reference definitions.
static STREAM_LINK_DEF_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();

fn stream_fence_re() -> &'static Regex {
    STREAM_FENCE_RE.get_or_init(|| Regex::new(r"^ *(`{3,}|~{3,})").unwrap())
}

fn stream_link_def_re() -> &'static Regex {
    STREAM_LINK_DEF_RE.get_or_init(|| Regex::new(r"(?m)^ {0,3}\[[^\]\n]+\]:\s*\S").unwrap())
}

/// Port of `findStreamCut` — largest safe cut point for incremental markdown
/// rendering: offset just past the last blank line outside any code fence.
fn find_stream_cut(text: &str) -> usize {
    let mut cut = 0;
    let mut in_fence = false;
    let mut fence_char = '\0';
    let mut line_start = 0;
    while line_start < text.len() {
        // Only newline-terminated lines are considered: the unterminated
        // tail is still growing (a blank-looking tail could become a fence
        // or content) — and cutting at len+1 would slice out of bounds.
        let Some(nl) = text[line_start..].find('\n').map(|i| line_start + i) else {
            break;
        };
        let line = &text[line_start..nl];
        if let Some(cap) = stream_fence_re().captures(line) {
            let ch = cap.get(1).unwrap().as_str().chars().next().unwrap();
            if !in_fence {
                in_fence = true;
                fence_char = ch;
            } else if ch == fence_char {
                in_fence = false;
            }
        } else if !in_fence && line.trim().is_empty() {
            // A blank final line without a trailing newline yields nl ==
            // text.len(); cut must stay within bounds (panic on slice).
            cut = (nl + 1).min(text.len());
        }
        line_start = nl + 1;
    }
    cut
}

/// Math.random()-based id generator (`Math.random().toString(36).slice(2, 10)`
/// and `crypto.randomUUID()`).
fn next_random_f64() -> f64 {
    // xorshift64* → [0, 1)
    static STATE: AtomicU64 = AtomicU64::new(0);
    let mut state = STATE.load(Ordering::Relaxed);
    if state == 0 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9e37_79b9_7f4a_7c15);
        state = seed_from_entropy(nanos, &STATE as *const _ as u64);
    }
    state ^= state >> 12;
    state ^= state << 25;
    state ^= state >> 27;
    STATE.store(state, Ordering::Relaxed);
    let x = state.wrapping_mul(0x2545_f491_4f6c_dd1d);
    // 53-bit mantissa → [0, 1)
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// Initial xorshift state from time + address entropy, with a fixed nonzero
/// fallback (xorshift degenerates at state 0).
fn seed_from_entropy(nanos: u64, addr: u64) -> u64 {
    let state = nanos ^ addr.rotate_left(17) ^ 0x9e37_79b9_7f4a_7c15;
    if state == 0 {
        return 0x2545_f491_4f6c_dd1d;
    }
    state
}

fn new_id() -> String {
    // Math.random().toString(36) → "0.xxxxxxxx" → slice(2, 10) = 8 base36 chars
    let r = next_random_f64();
    let mut n = r * 36.0f64.powi(9);
    let mut s = String::with_capacity(8);
    for _ in 0..9 {
        let digit = (n % 36.0) as u32;
        n = (n / 36.0).floor();
        s.push(char::from_digit(digit, 36).unwrap_or('0'));
    }
    // slice(2,10) of "0.<digits>" — drop the leading "0." and keep 8 chars
    s.chars().take(8).collect()
}

fn random_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

// ─── Tool output body ──────────────────────────────────────────────────────

/// Two columns of left indent for a tool-output body row.
const TOOL_OUTPUT_INDENT: usize = 2;

/// The body of a tool result: a parsed unified diff when there is one,
/// otherwise the remaining plain text.
///
/// The body is parsed once per render and both the `+N -M` badge and the rows
/// come from the same parse, so they can never disagree. ANSI is stripped
/// first: raw escape sequences from a command (cursor moves, OSC titles) would
/// otherwise corrupt the diff-based renderer. The diff renderer re-adds the
/// only colour the body needs.
#[derive(Debug, Default, PartialEq, Eq)]
struct ToolOutput {
    diff: Vec<diff::DiffLine>,
    text_lines: Vec<String>,
}

impl ToolOutput {
    fn parse(content: &str) -> Self {
        if content.trim().is_empty() {
            return Self::default();
        }
        let cleaned = strip_ansi_codes(content);
        let trimmed = cleaned.trim_end();
        if looks_like_diff(trimmed) {
            return Self {
                diff: diff::parse_any_diff(trimmed),
                text_lines: Vec::new(),
            };
        }
        Self {
            diff: Vec::new(),
            text_lines: cleaned
                .lines()
                .map(|line| line.trim_end().to_string())
                .collect(),
        }
    }

    fn is_empty(&self) -> bool {
        self.diff.is_empty() && self.text_lines.iter().all(|line| line.is_empty())
    }

    /// `+N -M` badge for the tool row (nothing when the body is not a diff).
    fn summary_ansi(&self, theme: &Theme) -> Option<String> {
        if self.diff.is_empty() {
            return None;
        }
        let stats = diff::diff_stats(&self.diff);
        if stats.added == 0 && stats.removed == 0 {
            return None;
        }
        Some(format!(
            "{} {}",
            fg(theme.success as u8, &format!("+{}", stats.added)),
            fg(theme.error as u8, &format!("-{}", stats.removed))
        ))
    }

    /// Body rows plus the number of rows left out.
    fn rows(
        &self,
        width: usize,
        expanded: bool,
        error: bool,
        theme: &Theme,
    ) -> (Vec<String>, usize) {
        if self.is_empty() || width <= TOOL_OUTPUT_INDENT {
            return (Vec::new(), 0);
        }
        let inner = width - TOOL_OUTPUT_INDENT;
        let limit = if expanded {
            EXPANDED_TOOL_OUTPUT_ROWS
        } else {
            COLLAPSED_TOOL_OUTPUT_ROWS
        };

        if !self.diff.is_empty() {
            // `render_diff` reserves a row for its own `… N more lines` marker
            // whenever it truncates, so ask for one more row than the budget
            // and drop the marker (the caller owns the marker text).
            let mut rows = diff::render_diff(
                &self.diff,
                inner,
                &diff_theme(theme),
                limit.saturating_add(1),
            );
            let hidden = self.diff.len().saturating_sub(limit);
            if hidden > 0 {
                rows.pop();
            }
            let rows = rows
                .into_iter()
                .map(|row| {
                    format!(
                        "  {}",
                        truncate_to_width(&row, inner, &TruncateOptions::default())
                    )
                })
                .collect();
            return (rows, hidden);
        }

        let total = self.text_lines.len();
        let colour = if error {
            theme.error as u8
        } else {
            theme.tool_output as u8
        };
        let rows = self
            .text_lines
            .iter()
            .take(limit)
            .map(|line| {
                format!(
                    "  {}",
                    fg(
                        colour,
                        &truncate_to_width(line, inner, &TruncateOptions::default())
                    )
                )
            })
            .collect();
        (rows, total.saturating_sub(limit))
    }
}

/// True when a tool body really is a diff, rather than plain text that
/// [`diff::parse_unified_diff`] would happily classify as context/meta lines
/// (it does not reject arbitrary text). An `apply_patch` envelope, a hunk
/// header or a `---`/`+++` header pair is required.
fn looks_like_diff(text: &str) -> bool {
    if diff::is_apply_patch(text) {
        return true;
    }
    let mut minus_header = false;
    let mut plus_header = false;
    for line in text.lines() {
        if line.starts_with("@@") || line.starts_with("@@ ") {
            return true;
        }
        if line.starts_with("--- ") {
            minus_header = true;
        } else if line.starts_with("+++ ") {
            plus_header = true;
        }
    }
    minus_header && plus_header
}

/// The diff palette derived from the active [`Theme`].
fn diff_theme(theme: &Theme) -> DiffTheme {
    DiffTheme {
        add_fg: theme.success as u8,
        remove_fg: theme.error as u8,
        hunk_fg: theme.accent as u8,
        meta_fg: theme.dim as u8,
        ..DiffTheme::default()
    }
}

// ─── ChatArea ─────────────────────────────────────────────────────────────

pub struct ChatArea {
    messages: Vec<ChatMessage>,
    viewport_top: usize,
    viewport_height: usize,
    rendered_lines: Vec<RenderedLine>,
    auto_scroll: bool,
    width: usize,
    thinking_hidden: bool,
    last_render_width: i64,
    dirty: bool,
    pending_rerender: BTreeSet<usize>,
    flushing: bool,
    stream_caches: std::collections::HashMap<String, StreamRenderCache>,

    md: MarkdownRenderer,
    md_thinking: MarkdownRenderer,
    theme: Theme,
    /// `ctrl+g`: render each tool call's body beneath its row. Collapsed, a call
    /// is one row (a failure keeps its body — see `render_tool_message`).
    tool_output_expanded: bool,
    /// `ctrl+d`: fold runs of consecutive tool calls and thinking blocks into
    /// compact summary rows (see [`Self::compact_heads`]). Off by default: the
    /// transcript renders exactly as it does without it.
    compact_activity: bool,
    /// The compact layout the rendered lines were built with — one entry per
    /// message, the index of the message that renders it (itself when it
    /// renders its own rows). Compared against a freshly computed layout to
    /// notice when a fold *appeared* (a tool completing into the run above it,
    /// which changes an earlier message's rows) and the transcript has to be
    /// rebuilt. Only meaningful while `compact_activity` is set.
    compact_layout: Vec<usize>,
    on_change: Option<Box<dyn FnMut()>>,
    message_line_ranges: Vec<(usize, i64)>,
}

impl ChatArea {
    pub fn new(max_width: usize, theme: Option<Theme>) -> Self {
        let theme = theme.unwrap_or(DARK_THEME);
        let (md, md_thinking) = build_markdown(theme);

        ChatArea {
            messages: Vec::new(),
            viewport_top: 0,
            viewport_height: 20,
            rendered_lines: Vec::new(),
            auto_scroll: true,
            width: max_width,
            thinking_hidden: false,
            last_render_width: -1,
            dirty: false,
            pending_rerender: BTreeSet::new(),
            flushing: false,
            stream_caches: std::collections::HashMap::new(),
            md,
            md_thinking,
            theme,
            tool_output_expanded: false,
            compact_activity: false,
            compact_layout: Vec::new(),
            on_change: None,
            message_line_ranges: Vec::new(),
        }
    }

    /// Swap the palette (the `/theme` command) and rebuild the markdown
    /// renderers, which bake the colours in at construction time.
    pub fn set_theme(&mut self, theme: Theme) {
        if self.theme == theme {
            return;
        }
        let (md, md_thinking) = build_markdown(theme);
        self.md = md;
        self.md_thinking = md_thinking;
        self.theme = theme;
        self.rerender();
    }

    pub fn theme(&self) -> Theme {
        self.theme
    }

    pub fn tool_output_expanded(&self) -> bool {
        self.tool_output_expanded
    }

    pub fn set_tool_output_expanded(&mut self, expanded: bool) {
        if self.tool_output_expanded != expanded {
            self.tool_output_expanded = expanded;
            self.rerender();
        }
    }

    /// Flip tool-output expansion (`ctrl+g`); returns the new state.
    pub fn toggle_tool_output_expanded(&mut self) -> bool {
        self.set_tool_output_expanded(!self.tool_output_expanded);
        self.tool_output_expanded
    }

    /// Is the compact-activity view on (`ctrl+d`)?
    pub fn compact_activity(&self) -> bool {
        self.compact_activity
    }

    /// Turn the compact view on/off (`ctrl+d`).
    ///
    /// On, an uninterrupted run of completed calls to the same tool folds into
    /// one summary row (`read 3 files`), and a thinking block folds into a
    /// one-line marker — the vertical-space equivalent of the desktop's
    /// collapsed activity bursts. Off (the default) is the transcript exactly
    /// as it always renders.
    ///
    /// Expanding tool output (`ctrl+g`) wins over folding: asking for every
    /// tool body and hiding the calls are contradictory, so with bodies
    /// expanded the calls stay individual rows.
    pub fn set_compact_activity(&mut self, on: bool) {
        if self.compact_activity == on {
            return;
        }
        self.compact_activity = on;
        self.rerender();
    }

    /// Flip the compact view (`ctrl+d`); returns the new state. Like the other
    /// view toggles it jumps to the bottom: the transcript's height changes by
    /// however much was folded, which invalidates any scroll offset the reader
    /// was holding.
    pub fn toggle_compact_activity(&mut self) -> bool {
        self.set_compact_activity(!self.compact_activity);
        self.set_auto_scroll(true);
        self.compact_activity
    }

    /// Fold an uninterrupted run of *completed* calls to the same tool?
    fn folds_tool_runs(&self) -> bool {
        self.compact_activity && !self.tool_output_expanded
    }

    pub fn last_message(&self) -> Option<&ChatMessage> {
        self.messages.last()
    }

    /// The content of the most recent assistant message (thinking excluded) —
    /// what `/copy` and `ctrl+x` put on the clipboard. `None` when the
    /// transcript has no assistant text yet.
    pub fn last_assistant_text(&self) -> Option<String> {
        self.messages
            .iter()
            .rev()
            .find(|msg| msg.role == ChatRole::Assistant && !msg.content.trim().is_empty())
            .map(|msg| msg.content.trim_end().to_string())
    }

    // ─── Public API ─────────────────────────────────────────────────────

    pub fn set_width(&mut self, w: usize) {
        if w != self.width {
            self.width = w;
            self.rerender();
        }
    }

    pub fn set_viewport_height(&mut self, h: usize) {
        self.viewport_height = h;
        if self.viewport_top + self.viewport_height > self.rendered_lines.len() {
            self.viewport_top = self
                .rendered_lines
                .len()
                .saturating_sub(self.viewport_height);
        }
    }

    pub fn set_auto_scroll(&mut self, v: bool) {
        self.auto_scroll = v;
        if v {
            self.scroll_to_bottom();
        }
    }

    /// Is the view following the transcript's tail? False once the reader has
    /// scrolled up (or back into history), so appended rows land below the
    /// viewport instead of dragging it down.
    pub fn auto_scroll(&self) -> bool {
        self.auto_scroll
    }

    pub fn add_message(&mut self, msg: ChatMessage) {
        self.messages.push(msg);
        if self.last_render_width == -1 {
            self.rerender();
        } else {
            self.append_last_message();
        }
        if self.auto_scroll {
            self.scroll_to_bottom();
        }
        if let Some(cb) = &mut self.on_change {
            cb();
        }
    }

    pub fn bind_user_run(
        &mut self,
        message_id: &str,
        run_id: &str,
        run_state: RunState,
        queue_position: Option<u32>,
    ) {
        let Some(index) = self.messages.iter().position(|m| m.id == message_id) else {
            return;
        };
        if self.messages[index].role != ChatRole::User {
            return;
        }
        self.messages[index].id = run_id.to_string();
        self.messages[index].run_id = Some(run_id.to_string());
        self.messages[index].run_state = Some(run_state);
        self.messages[index].queue_position = queue_position;
        self.rerender_message(index);
    }

    pub fn update_run_state(&mut self, run_id: &str, run_state: RunState) {
        let Some(index) = self
            .messages
            .iter()
            .position(|m| m.run_id.as_deref() == Some(run_id))
        else {
            return;
        };
        self.messages[index].run_state = Some(run_state);
        self.rerender_message(index);
    }

    /// Current lifecycle state tracked for `run_id` (None when the run is
    /// unknown to this chat).
    pub fn run_state(&self, run_id: &str) -> Option<RunState> {
        self.messages
            .iter()
            .find(|m| m.run_id.as_deref() == Some(run_id))
            .and_then(|m| m.run_state)
    }

    /// True when this chat owns a message bound to `run_id` — i.e. the run
    /// was submitted by this client (foreign runs from other clients on the
    /// same session never get a `bind_user_run`).
    pub fn has_run(&self, run_id: &str) -> bool {
        self.messages
            .iter()
            .any(|m| m.run_id.as_deref() == Some(run_id))
    }

    pub fn update_queue_position(&mut self, run_id: &str, queue_position: u32) {
        let Some(index) = self
            .messages
            .iter()
            .position(|m| m.run_id.as_deref() == Some(run_id))
        else {
            return;
        };
        self.messages[index].queue_position = Some(queue_position);
        self.rerender_message(index);
    }

    pub fn upsert_queued_run(&mut self, run_id: &str, display_text: &str, queue_position: u32) {
        if let Some(index) = self
            .messages
            .iter()
            .position(|m| m.run_id.as_deref() == Some(run_id))
        {
            self.messages[index].run_state = Some(RunState::Queued);
            self.messages[index].queue_position = Some(queue_position);
            self.rerender_message(index);
            return;
        }
        self.add_message(ChatMessage {
            id: run_id.to_string(),
            role: ChatRole::User,
            content: display_text.to_string(),
            run_id: Some(run_id.to_string()),
            run_state: Some(RunState::Queued),
            queue_position: Some(queue_position),
            ..ChatMessage::new(String::new(), ChatRole::User, "")
        });
    }

    pub fn set_message_run_state(&mut self, message_id: &str, run_state: RunState) {
        let Some(index) = self.messages.iter().position(|m| m.id == message_id) else {
            return;
        };
        self.messages[index].run_state = Some(run_state);
        self.rerender_message(index);
    }

    pub fn set_on_change(&mut self, cb: impl FnMut() + 'static) {
        self.on_change = Some(Box::new(cb));
    }

    pub fn update_last_message(&mut self, content: &str) {
        if let Some(idx) = self.find_assistant_index() {
            self.messages[idx].content = content.to_string();
            self.messages[idx].pending = true;
            self.rerender_message(idx);
            if self.auto_scroll {
                self.scroll_to_bottom();
            }
        }
    }

    pub fn append_to_last_message(&mut self, delta: &str) {
        // When the last message is a tool result from a previous turn, a new
        // assistant response is starting — push a fresh message.
        if self
            .messages
            .last()
            .is_some_and(|m| m.role == ChatRole::Tool)
        {
            self.add_message(ChatMessage::new(new_id(), ChatRole::Assistant, ""));
        }
        if let Some(idx) = self.find_assistant_index() {
            self.messages[idx].content.push_str(delta);
            self.messages[idx].pending = true;
            self.mark_message_dirty(idx);
        }
    }

    pub fn mark_last_assistant_stopped(&mut self) {
        if let Some(idx) = self.find_assistant_index() {
            let msg = &mut self.messages[idx];
            msg.pending = false;
            msg.stopped = true;
            self.rerender_message(idx);
        }
    }

    pub fn mark_last_message_complete(&mut self) {
        if let Some(idx) = self.find_assistant_index() {
            self.messages[idx].pending = false;
            self.rerender_message(idx);
        }
    }

    // ─── Tool call management ───────────────────────────────────────────

    pub fn add_tool_start(&mut self, tool_id: &str, tool_name: &str, tool_args: Option<String>) {
        // The agent emits tool_start twice for the same call; update the
        // existing bubble instead of appending a second one.
        let existing_idx = if tool_id.is_empty() {
            None
        } else {
            self.find_tool_index(tool_id)
        };
        if let Some(existing_idx) = existing_idx {
            let existing = &mut self.messages[existing_idx];
            if !tool_name.is_empty() {
                existing.name = Some(tool_name.to_string());
            }
            if let Some(args) = &tool_args {
                existing.tool_args = Some(args.clone());
            }
            self.rerender_message(existing_idx);
            if self.auto_scroll {
                self.scroll_to_bottom();
            }
            return;
        }
        let mut msg = ChatMessage::new(random_uuid(), ChatRole::Tool, "");
        msg.name = Some(tool_name.to_string());
        msg.tool = Some(tool_id.to_string());
        msg.tool_status = Some(ToolStatus::Running);
        if tool_args.is_some() {
            msg.tool_args = tool_args;
        }
        self.messages.push(msg);
        if self.last_render_width == -1 {
            self.rerender();
        } else {
            self.append_last_message();
        }
        if self.auto_scroll {
            self.scroll_to_bottom();
        }
    }

    pub fn append_tool_delta(&mut self, tool_id: &str, text: &str) {
        if let Some(idx) = self.find_tool_index(tool_id) {
            self.messages[idx].content.push_str(text);
            self.mark_message_dirty(idx);
        }
    }

    /// Finish a running call: `output` is the `tool_end` text (kept when the
    /// body never streamed), and `is_error` marks a failed call.
    ///
    /// The flag comes from the agent's own structured result (`error`, or a
    /// non-zero `exit_code` that is not a soft fail), so a failure is styled —
    /// and keeps its body — the same way a replayed one is.
    pub fn finish_tool(&mut self, tool_id: &str, output: Option<&str>, is_error: bool) {
        if let Some(idx) = self.find_tool_index(tool_id) {
            // `tool_delta` already streamed the body in the common case; a
            // `tool_end` payload that carries text and an empty body means the
            // agent never streamed it, so keep it (history replay does the
            // same via `apply_messages`).
            if let Some(text) = output {
                if self.messages[idx].content.trim().is_empty() && !text.trim().is_empty() {
                    self.messages[idx].content = text.to_string();
                }
            }
            self.messages[idx].tool_status = Some(if is_error {
                ToolStatus::Error
            } else {
                ToolStatus::Complete
            });
            self.rerender_message(idx);
        }
    }

    // ─── Thinking management ────────────────────────────────────────────

    pub fn start_thinking(&mut self) {
        if self.messages.is_empty() {
            self.messages.push(ChatMessage {
                id: new_id(),
                role: ChatRole::Assistant,
                content: String::new(),
                thinking: Some(String::new()),
                pending: true, // thinking streaming IS streaming in progress
                ..ChatMessage::new(String::new(), ChatRole::Assistant, "")
            });
            if self.last_render_width == -1 {
                self.rerender();
            } else {
                self.append_last_message();
            }
            return;
        }
        let last_idx = self.messages.len() - 1;
        let last = &mut self.messages[last_idx];
        if last.role == ChatRole::Assistant {
            // Subsequent thinking blocks in the same turn are concatenated
            // directly into one thinking section.
            if last.thinking.is_none() {
                last.thinking = Some(String::new());
            }
            last.pending = true;
            self.rerender_message(last_idx);
        } else {
            self.messages.push(ChatMessage {
                id: new_id(),
                role: ChatRole::Assistant,
                content: String::new(),
                thinking: Some(String::new()),
                pending: true,
                ..ChatMessage::new(String::new(), ChatRole::Assistant, "")
            });
            if self.last_render_width == -1 {
                self.rerender();
            } else {
                self.append_last_message();
            }
        }
    }

    pub fn append_thinking_delta(&mut self, text: &str) {
        // Target the last assistant message, not the literal last message: a
        // user message queued mid-stream (enqueue_if_busy) is pushed after the
        // streaming assistant, and the thinking deltas still belong to that
        // assistant turn. Using `find_assistant_index` mirrors
        // `append_to_last_message`, which already survives a trailing queued
        // user message.
        let Some(idx) = self.find_assistant_index() else {
            return;
        };
        let msg = &mut self.messages[idx];
        if let Some(thinking) = msg.thinking.as_mut() {
            thinking.push_str(text);
            self.mark_message_dirty(idx);
        }
    }

    pub fn end_thinking(&mut self) {
        let Some(idx) = self.find_assistant_index() else {
            return;
        };
        if self.messages[idx].thinking.is_some() {
            self.rerender_message(idx);
        }
    }

    pub fn set_thinking_hidden(&mut self, hidden: bool) {
        if self.thinking_hidden != hidden {
            self.thinking_hidden = hidden;
            self.rerender();
        }
    }

    /// Flip thinking visibility for all messages (ctrl+o); returns the new
    /// state (`true` = hidden). Re-renders on change and jumps to the
    /// bottom — after expanding, the fresh content would otherwise grow
    /// off-screen; after collapsing, the answer lands at the bottom anyway.
    /// (Also keeps `viewport_top` valid: collapsing shrinks the line count.)
    pub fn toggle_thinking_hidden(&mut self) -> bool {
        self.set_thinking_hidden(!self.thinking_hidden);
        self.set_auto_scroll(true);
        self.thinking_hidden
    }

    pub fn clear_messages(&mut self) {
        self.messages = Vec::new();
        self.rerender();
        // The old transcript is gone, so its scroll position is meaningless:
        // a cleared chat follows the tail again. Without this a session switch
        // opens the new conversation at the previous one's offset (and keeps
        // `auto_scroll` off, so the new session never follows its own output).
        self.set_auto_scroll(true);
    }

    /// Insert `messages` above the transcript, keeping the view anchored on the
    /// line it was showing. Returns the number of rendered lines the insert
    /// added above the previous content (0 when there was nothing to add or no
    /// render to measure).
    ///
    /// This is how older history arrives while the user reads it (the journal
    /// is paged backwards): `viewport_top` is a line index into
    /// `rendered_lines`, so inserting above it without shifting would silently
    /// scroll the transcript by the height of everything inserted. Rendering is
    /// per message, so the pre-existing lines are untouched — the shift is
    /// exactly the line count the prepended block occupies, read back from the
    /// fresh layout rather than recomputed from markdown metrics.
    ///
    /// `auto_scroll` is left alone: at the tail it still tracks the tail (the
    /// shift equals the new max offset), and scrolled up it stays anchored.
    pub fn prepend_messages(&mut self, messages: Vec<ChatMessage>) -> usize {
        if messages.is_empty() {
            return 0;
        }
        let prepended = messages.len();
        let mut all = messages;
        all.extend(std::mem::take(&mut self.messages));
        self.messages = all;
        self.rerender();
        // In the old layout the first pre-existing message started at line 0,
        // and now it starts at `start` — so `start` is the insertion height.
        let Some((start, _)) = self.message_line_ranges.get(prepended).copied() else {
            // No render yet (`rerender` deferred until the first render()
            // learns the width): the whole transcript is laid out from scratch
            // anyway, and the viewport is still at its top.
            return 0;
        };
        self.viewport_top = self.viewport_top.saturating_add(start);
        if let Some(cb) = &mut self.on_change {
            cb();
        }
        start
    }

    pub fn scroll_up(&mut self, lines: usize) -> bool {
        if self.viewport_top == 0 {
            return false;
        }
        self.viewport_top = self.viewport_top.saturating_sub(lines);
        self.auto_scroll = false;
        true
    }

    pub fn scroll_down(&mut self, lines: usize) -> bool {
        let max_top = self
            .rendered_lines
            .len()
            .saturating_sub(self.viewport_height);
        if self.viewport_top >= max_top {
            return false;
        }
        self.viewport_top = max_top.min(self.viewport_top + lines);
        if self.viewport_top >= max_top {
            self.auto_scroll = true;
        }
        true
    }

    pub fn is_at_top(&self) -> bool {
        self.viewport_top == 0
    }

    /// Rows the viewport shows (`get_height` is the *rendered* height, which is
    /// smaller while the transcript is still shorter than the terminal).
    pub fn viewport_height(&self) -> usize {
        self.viewport_height
    }

    pub fn is_at_bottom(&self) -> bool {
        let max_top = self
            .rendered_lines
            .len()
            .saturating_sub(self.viewport_height);
        self.viewport_top >= max_top
    }

    pub fn scroll_to_bottom(&mut self) {
        self.viewport_top = self
            .rendered_lines
            .len()
            .saturating_sub(self.viewport_height);
    }

    // ─── Rendering ──────────────────────────────────────────────────────

    pub fn get_height(&self) -> usize {
        self.rendered_lines.len().min(self.viewport_height)
    }

    pub fn invalidate(&mut self) {
        self.last_render_width = -1;
    }

    /// Render ALL lines (bypass viewport) — seeds terminal scrollback.
    pub fn render_all(&mut self, width: usize) -> Vec<String> {
        if width as i64 != self.last_render_width || self.dirty {
            self.last_render_width = width as i64;
            self.width = width;
            self.dirty = false;
            self.rerender();
        }
        self.flush_pending_rerenders();
        self.rendered_lines
            .iter()
            .map(|rl| rl.text.clone())
            .collect()
    }

    pub fn render(&mut self, width: usize) -> Vec<String> {
        if width as i64 != self.last_render_width || self.dirty {
            self.last_render_width = width as i64;
            self.width = width;
            self.dirty = false;
            self.rerender();
        }
        self.flush_pending_rerenders();
        // Clamp after content shrink (collapsed thinking, /clear) — a stale
        // viewport_top past the new end would slice out of range.
        let max_top = self
            .rendered_lines
            .len()
            .saturating_sub(self.viewport_height);
        if self.viewport_top > max_top {
            self.viewport_top = max_top;
        }
        let end = (self.viewport_top + self.viewport_height).min(self.rendered_lines.len());
        self.rendered_lines[self.viewport_top..end]
            .iter()
            .map(|rl| rl.text.clone())
            .collect()
    }

    /// Mark a message for deferred re-render (batched at next render()).
    fn mark_message_dirty(&mut self, msg_idx: usize) {
        if self.last_render_width == -1 {
            self.dirty = true;
        } else {
            self.pending_rerender.insert(msg_idx);
        }
        if self.auto_scroll {
            self.scroll_to_bottom();
        }
        if let Some(cb) = &mut self.on_change {
            cb();
        }
    }

    /// Apply deferred message re-renders — at most once per rendered frame.
    fn flush_pending_rerenders(&mut self) {
        if self.pending_rerender.is_empty() {
            return;
        }
        let idxs: Vec<usize> = self.pending_rerender.iter().copied().collect();
        self.pending_rerender.clear();
        self.flushing = true;
        for idx in idxs {
            self.rerender_message(idx);
        }
        self.flushing = false;
    }

    /// The compact layout: for each message, the index of the message that
    /// renders it — itself, except for a message folded into the run above it.
    ///
    /// A run is an uninterrupted stretch of messages that fold together: calls
    /// to the same tool that all completed, or one-line thinking blocks. Only
    /// the run's first message renders (the summary row); the rest render
    /// nothing, which is what makes a long burst of calls one row instead of
    /// twenty. A text or user message — or a failed or still-running tool —
    /// breaks the run, so a fold never hides content the user needs to see.
    ///
    /// Pure and cheap (no rendering): called to notice when a fold appeared
    /// without the rendered lines being rebuilt.
    fn compact_heads(messages: &[ChatMessage], fold_tools: bool) -> Vec<usize> {
        let mut heads: Vec<usize> = (0..messages.len()).collect();
        let mut run: Option<(FoldKind, usize)> = None;
        for (i, msg) in messages.iter().enumerate() {
            let Some(kind) = fold_kind(msg, fold_tools) else {
                run = None;
                continue;
            };
            match run {
                Some((ref run_kind, head)) if *run_kind == kind => heads[i] = head,
                _ => run = Some((kind, i)),
            }
        }
        heads
    }
    /// The layout in effect for the rendered lines.
    fn layout(&self) -> Vec<usize> {
        if self.compact_activity {
            Self::compact_heads(&self.messages, self.folds_tool_runs())
        } else {
            (0..self.messages.len()).collect()
        }
    }

    /// Is `msg_idx` folded into an earlier message (and therefore rendering no
    /// rows of its own)?
    fn is_folded(&self, msg_idx: usize) -> bool {
        self.compact_layout
            .get(msg_idx)
            .is_some_and(|head| *head != msg_idx)
    }

    /// Recompute the compact layout and, when the rendered lines no longer
    /// match it, rebuild the transcript. Returns `true` when the caller must
    /// not assume `msg_idx` rendered on its own.
    ///
    /// Growth that adds *standalone* messages (the common case: a user turn, an
    /// assistant reply, the first call of a run) leaves every existing message's
    /// rows untouched, so the incremental paths stay in charge. A fold appearing
    /// changes an earlier message's row (a run's count) — and, when it dissolves
    /// again, brings rows back — so the transcript is rebuilt: correctness over
    /// a splice that would have to reason about both sides of the fold.
    fn sync_compact_layout(&mut self) -> bool {
        if !self.compact_activity {
            return false;
        }
        let fresh = Self::compact_heads(&self.messages, self.folds_tool_runs());
        if compact_layout_extends(&self.compact_layout, &fresh) {
            self.compact_layout = fresh;
            return false;
        }
        self.rerender();
        true
    }

    fn rerender(&mut self) {
        self.pending_rerender.clear();
        self.stream_caches.clear();
        // Defer until first render() has set the correct terminal width.
        if self.last_render_width == -1 {
            self.dirty = true;
            return;
        }
        self.dirty = false;

        self.compact_layout = self.layout();
        self.rendered_lines = Vec::new();
        self.message_line_ranges = Vec::new();
        // A folded message renders nothing and takes no separator with it, so
        // the transcript stays as tight as the summary row implies.
        let mut rendered_any = false;
        for i in 0..self.messages.len() {
            if self.is_folded(i) {
                let at = self.rendered_lines.len();
                self.message_line_ranges.push((at, at as i64 - 1));
                continue;
            }
            if rendered_any {
                self.rendered_lines.push(RenderedLine {
                    text: String::new(),
                    dim: true,
                });
            }
            let start = self.rendered_lines.len();
            self.render_message(i);
            self.message_line_ranges
                .push((start, self.rendered_lines.len() as i64 - 1));
            rendered_any = true;
        }
        self.rendered_lines.push(RenderedLine {
            text: String::new(),
            dim: true,
        });
    }

    /// Re-render only the message at msg_idx, splicing its lines in-place.
    fn rerender_message(&mut self, msg_idx: usize) {
        // A fold may have appeared (a tool completed into the run above it):
        // that changes an earlier message's rows, so the transcript is rebuilt
        // instead of spliced.
        if self.sync_compact_layout() {
            return;
        }
        // The message's rows live in the run's summary row, and the summary
        // depends on which messages are folded — not on this one's content.
        if self.is_folded(msg_idx) {
            return;
        }
        if self.last_render_width == -1 || msg_idx >= self.message_line_ranges.len() {
            self.rerender();
            return;
        }
        let range = self.message_line_ranges[msg_idx];
        // end can be start - 1 for a zero-line message (TS number semantics);
        // oldLen = end - start + 1 → 0 in that case.
        let old_len = (range.1 - range.0 as i64 + 1).max(0) as usize;

        // Render into a temp array via swap (avoids threading out params).
        let saved = std::mem::take(&mut self.rendered_lines);
        self.render_message(msg_idx);
        let new_lines = std::mem::take(&mut self.rendered_lines);
        self.rendered_lines = saved;

        let new_len = new_lines.len();
        self.rendered_lines
            .splice(range.0..range.0 + old_len, new_lines);
        let delta = new_len as i64 - old_len as i64;
        self.message_line_ranges[msg_idx] = (range.0, range.0 as i64 + new_len as i64 - 1);
        for i in msg_idx + 1..self.message_line_ranges.len() {
            self.message_line_ranges[i].0 = (self.message_line_ranges[i].0 as i64 + delta) as usize;
            self.message_line_ranges[i].1 += delta;
        }
        if self.auto_scroll {
            self.scroll_to_bottom();
        }
        // During a flush the render is already in flight — re-firing
        // onChange would just schedule a redundant extra frame.
        if !self.flushing {
            if let Some(cb) = &mut self.on_change {
                cb();
            }
        }
    }

    /// Append the last message in `messages` to renderedLines (assumes the
    /// message was already pushed).
    fn append_last_message(&mut self) {
        // A message that folds into the run above it changes that run's summary
        // row, which sits somewhere in the middle of the transcript.
        if self.sync_compact_layout() {
            return;
        }
        self.rendered_lines.pop();
        if self.messages.len() > 1 {
            self.rendered_lines.push(RenderedLine {
                text: String::new(),
                dim: true,
            });
        }
        let start = self.rendered_lines.len();
        self.render_message(self.messages.len() - 1);
        self.message_line_ranges
            .push((start, self.rendered_lines.len() as i64 - 1));
        self.rendered_lines.push(RenderedLine {
            text: String::new(),
            dim: true,
        });
        if self.auto_scroll {
            self.scroll_to_bottom();
        }
        if let Some(cb) = &mut self.on_change {
            cb();
        }
    }

    fn find_assistant_index(&self) -> Option<usize> {
        self.messages
            .iter()
            .rposition(|m| m.role == ChatRole::Assistant)
    }

    /// Test-only view: (role, plain-text content) per message.
    #[cfg(test)]
    pub(crate) fn plain_messages(&self) -> Vec<(ChatRole, String)> {
        self.messages
            .iter()
            .map(|m| (m.role, crate::utils::strip_ansi_codes(&m.content)))
            .collect()
    }

    /// Test-only view: the last assistant message's thinking text.
    #[cfg(test)]
    pub(crate) fn last_assistant_thinking(&self) -> Option<&str> {
        self.messages
            .iter()
            .rev()
            .find(|m| m.role == ChatRole::Assistant)
            .and_then(|m| m.thinking.as_deref())
    }

    fn find_tool_index(&self, tool_id: &str) -> Option<usize> {
        self.messages
            .iter()
            .rposition(|m| m.role == ChatRole::Tool && m.tool.as_deref() == Some(tool_id))
    }

    fn render_message(&mut self, msg_idx: usize) {
        // A folded run renders as one summary row, written by its first message
        // (the rest render nothing at all — see `compact_heads`).
        let run = self.compact_run(msg_idx);
        let msg = self.messages[msg_idx].clone();
        match msg.role {
            ChatRole::User => self.render_user_message(&msg),
            ChatRole::Assistant => self.render_assistant_message(&msg, run),
            ChatRole::Tool if run > 1 => self.render_tool_run(msg_idx, run),
            ChatRole::Tool => self.render_tool_message(&msg),
            ChatRole::System => self.render_system_message(&msg),
        }
    }

    /// How many messages render together as the row at `msg_idx`: its own fold
    /// run, or 1 when nothing is folded into it.
    fn compact_run(&self, msg_idx: usize) -> usize {
        let mut end = msg_idx + 1;
        while end < self.compact_layout.len() && self.compact_layout[end] == msg_idx {
            end += 1;
        }
        end - msg_idx
    }

    // ─── User message (markdown + full-width background Box) ────────────

    fn render_user_message(&mut self, msg: &ChatMessage) {
        let rendered = self
            .md
            .render_text(&msg.content, self.width.saturating_sub(2).max(1));
        for line in rendered {
            let text = format!(" {line}");
            let bg_line = apply_background_to_line(&text, self.width, self.theme.user_bg);
            self.rendered_lines.push(RenderedLine {
                text: bg_line,
                dim: false,
            });
        }
        if msg.run_state == Some(RunState::Queued) {
            let suffix = match msg.queue_position {
                Some(q) => format!(" (#{q})"),
                None => String::new(),
            };
            // TS: dim(`queued${suffix}`) — suffix inside the dim.
            let t = format!(" {}", dim(&format!("queued{suffix}")));
            self.rendered_lines.push(RenderedLine {
                text: apply_background_to_line(&t, self.width, self.theme.user_bg),
                dim: true,
            });
        }
        if matches!(
            msg.run_state,
            Some(RunState::Cancelled)
                | Some(RunState::Superseded)
                | Some(RunState::LostOnAgentRestart)
        ) {
            let label = msg.run_state.unwrap().label();
            let t = format!(" {}", dim(label));
            self.rendered_lines.push(RenderedLine {
                text: apply_background_to_line(&t, self.width, self.theme.user_bg),
                dim: true,
            });
        }
    }

    // ─── Assistant message (markdown, thinking first) ───────────────────

    fn render_assistant_message(&mut self, msg: &ChatMessage, run: usize) {
        let has_thinking = msg
            .thinking
            .as_deref()
            .is_some_and(|t| !t.trim().is_empty());

        // Render thinking block FIRST (before content). Collapsed thinking
        // (ctrl+o) renders nothing — except a one-line hint while thinking is
        // actively streaming (pending, no content yet) so a live run doesn't
        // look frozen; historical thinking stays fully hidden.
        if has_thinking && !self.thinking_hidden {
            if self.compact_activity {
                // Compact view: the reasoning is summarized on one row (and, for
                // an uninterrupted run of thinking blocks, on one row for the
                // whole run — the followers render nothing at all).
                self.push_thinking_marker(msg, run);
            } else {
                let thinking = msg.thinking.as_deref().unwrap_or("");
                let thinking_lines = if msg.pending {
                    Self::render_streaming_markdown(
                        &mut self.stream_caches,
                        &format!("{}:t", msg.id),
                        thinking,
                        self.width.saturating_sub(2).max(1),
                        &mut self.md_thinking,
                    )
                } else {
                    self.md_thinking
                        .render_text(thinking, self.width.saturating_sub(2).max(1))
                };
                let think_prefix = format!("\x1b[3m\x1b[38;5;{}m", self.theme.thinking_text);
                for line in thinking_lines {
                    if line.is_empty() {
                        self.rendered_lines.push(RenderedLine {
                            text: String::new(),
                            dim: true,
                        });
                    } else {
                        // Re-apply thinking style after EVERY ANSI reset.
                        let styled = reapply_style(&format!(" {line}"), &think_prefix);
                        self.rendered_lines.push(RenderedLine {
                            text: format!("{think_prefix}{styled}{RESET}"),
                            dim: true,
                        });
                    }
                }
            }
        } else if has_thinking && msg.pending && msg.content.trim().is_empty() {
            // The thinking is hidden (ctrl+o) but still streaming: a live run
            // must not look frozen while nothing else is on screen.
            self.rendered_lines.push(RenderedLine {
                text: fg(
                    self.theme.thinking_text as u8,
                    &italic(" Thinking... (ctrl+o to expand)"),
                ),
                dim: true,
            });
        }

        // Spacer between thinking and content (only when thinking is shown —
        // the streaming placeholder never coexists with content).
        if has_thinking && !self.thinking_hidden && !msg.content.trim().is_empty() {
            self.rendered_lines.push(RenderedLine {
                text: String::new(),
                dim: true,
            });
        }

        // Render markdown content.
        let content_width = self.width.saturating_sub(2).max(1);
        let rendered = if msg.pending {
            Self::render_streaming_markdown(
                &mut self.stream_caches,
                &format!("{}:c", msg.id),
                &msg.content,
                content_width,
                &mut self.md,
            )
        } else {
            self.md.render_text(&msg.content, content_width)
        };
        if !msg.pending {
            // Final full render — drop the streaming caches for this message.
            self.stream_caches.remove(&format!("{}:t", msg.id));
            self.stream_caches.remove(&format!("{}:c", msg.id));
        }
        for line in rendered {
            if line.is_empty() {
                self.rendered_lines.push(RenderedLine {
                    text: String::new(),
                    dim: true,
                });
            } else {
                self.rendered_lines.push(RenderedLine {
                    text: format!(" {line}"),
                    dim: false,
                });
            }
        }

        // Interrupted generation marker.
        if msg.stopped {
            self.rendered_lines.push(RenderedLine {
                text: fg(self.theme.thinking_text as u8, &italic(" ■ interrupted")),
                dim: true,
            });
        }
    }

    /// Incremental markdown render for a streaming (pending) message.
    fn render_streaming_markdown(
        stream_caches: &mut std::collections::HashMap<String, StreamRenderCache>,
        key: &str,
        text: &str,
        width: usize,
        renderer: &mut MarkdownRenderer,
    ) -> Vec<String> {
        // Reference-style link definitions retroactively change earlier
        // rendering, which breaks prefix caching — render in full.
        if stream_link_def_re().is_match(text) {
            return renderer.render_text(text, width);
        }
        let cut = find_stream_cut(text);
        let mut prefix_lines: Option<Vec<String>> = None;
        let mut tail_start = 0;
        // Cache is only valid while the previously rendered prefix is unchanged.
        if let Some(cache) = stream_caches.get(key) {
            if !text.starts_with(&cache.text) {
                stream_caches.remove(key);
            } else if cache.cut <= cut {
                prefix_lines = Some(cache.lines.clone());
                tail_start = cache.cut;
            }
        }
        if cut > tail_start {
            // Extend the cache: render the newly stabilized segment on its own.
            let seg_lines = renderer.render_text(&text[tail_start..cut], width);
            prefix_lines = Some(match prefix_lines {
                Some(mut p) => {
                    p.extend(seg_lines);
                    p
                }
                None => seg_lines,
            });
            let prefix = text[..cut].to_string();
            let lines = prefix_lines.clone().unwrap();
            stream_caches.insert(
                key.to_string(),
                StreamRenderCache {
                    cut,
                    text: prefix,
                    lines,
                },
            );
            tail_start = cut;
        }
        let tail_lines = renderer.render_text(&text[tail_start..], width);
        match prefix_lines {
            Some(mut p) => {
                p.extend(tail_lines);
                p
            }
            None => tail_lines,
        }
    }

    // ─── Tool message (single-line header only) ─────────────────────────

    /// One row for a folded run of calls to the same tool (`▸ read 3 files`).
    ///
    /// The row carries what the individual rows would have said at a glance —
    /// which tool, and how much of it — and nothing else: no bodies (that is
    /// `ctrl+g`), no targets (twenty paths is what the fold exists to avoid).
    /// The `▸` marks it as a fold rather than a call.
    fn render_tool_run(&mut self, msg_idx: usize, run: usize) {
        let tool_name = tool_name_of(&self.messages[msg_idx]);
        let count = if tool_name == "shell" {
            // Every command stands on its own, so the run's size is its count.
            run
        } else {
            // File tools name the unit they worked on (the same wording — and
            // the same distinct-file counting — the desktop's collapsed bursts
            // use); an unknown tool keeps the plain call count, which is the
            // only thing that is true of it.
            let (targets, unreadable) =
                tool_targets(&self.messages[msg_idx..msg_idx + run], tool_name);
            (targets.len() + unreadable).min(run).max(1)
        };
        let line = format!(
            " {} {}",
            dim(FOLD_MARKER),
            self.format_tool_run_label(tool_name, count)
        );
        self.rendered_lines.push(RenderedLine {
            text: apply_background_to_line(&line, self.width, self.theme.tool_success_bg),
            dim: true,
        });
    }

    /// `read 3 files` / `$ 4 commands` / `mcp_tool ×3` — the folded row's label,
    /// in the same visual language as a single call's row.
    fn format_tool_run_label(&self, tool_name: &str, count: usize) -> String {
        let plural = |one: &'static str, many: &'static str| if count == 1 { one } else { many };
        let title = |text: &str| fg(self.theme.tool_title as u8, &bold(text));
        let tally = |text: String| fg(self.theme.tool_output as u8, &text);
        match tool_name {
            "shell" => format!(
                "{} {}",
                title("$"),
                tally(format!("{count} {}", plural("command", "commands")))
            ),
            "read" | "write" | "edit" => format!(
                "{} {}",
                title(tool_name),
                tally(format!("{count} {}", plural("file", "files")))
            ),
            other => format!("{} {}", title(other), tally(format!("×{count}"))),
        }
    }

    /// One row standing in for a thinking block (or an uninterrupted run of
    /// them) in the compact view: that the model reasoned, and how many blocks —
    /// not the text.
    fn push_thinking_marker(&mut self, msg: &ChatMessage, run: usize) {
        let mut label = "thinking".to_string();
        if run > 1 {
            label.push_str(&format!(" ×{run}"));
        }
        if msg.pending {
            label.push('…');
        }
        let text = format!(" {} {}", dim(FOLD_MARKER), italic(&label));
        self.rendered_lines.push(RenderedLine {
            text: fg(self.theme.thinking_text as u8, &text),
            dim: true,
        });
    }

    fn render_tool_message(&mut self, msg: &ChatMessage) {
        let tool_name = msg
            .name
            .as_deref()
            .unwrap_or(msg.tool.as_deref().unwrap_or("tool"));
        let status = msg.tool_status.unwrap_or(ToolStatus::Running);

        let bg_color = match status {
            ToolStatus::Error => self.theme.tool_error_bg,
            ToolStatus::Complete => self.theme.tool_success_bg,
            ToolStatus::Running => self.theme.tool_pending_bg,
        };

        let tool_args = msg.tool_args.as_deref();
        let body = ToolOutput::parse(&msg.content);
        let mut line = format!(" {}", self.format_tool_call(tool_name, tool_args));
        if let Some(summary) = body.summary_ansi(&self.theme) {
            line.push(' ');
            line.push_str(&summary);
        }

        self.rendered_lines.push(RenderedLine {
            text: apply_background_to_line(&line, self.width, bg_color),
            dim: status == ToolStatus::Complete,
        });

        // A call is one row — the call itself. The body is what `ctrl+g` (and
        // `/tool-output`) is for, so a transcript of twenty calls is twenty
        // rows rather than twenty previews. A *failure* is the exception: its
        // reason is not something the user should have to know a key to read.
        if !self.tool_output_expanded && status != ToolStatus::Error {
            return;
        }

        let (rows, hidden) = body.rows(
            self.width,
            self.tool_output_expanded,
            status == ToolStatus::Error,
            &self.theme,
        );
        self.rendered_lines.extend(
            rows.into_iter()
                .map(|text| RenderedLine { text, dim: false }),
        );
        if hidden > 0 {
            self.rendered_lines.push(RenderedLine {
                text: fg(
                    self.theme.dim as u8,
                    &truncate_to_width(
                        &format!(
                            "  … {hidden} more line{plural} · {hint}",
                            plural = if hidden == 1 { "" } else { "s" },
                            hint = if self.tool_output_expanded {
                                "truncated"
                            } else {
                                "ctrl+g to expand"
                            }
                        ),
                        self.width,
                        &TruncateOptions::default(),
                    ),
                ),
                dim: true,
            });
        }
    }

    /// Format tool call display per tool type.
    fn format_tool_call(&mut self, tool_name: &str, tool_args: Option<&str>) -> String {
        let max_for = |prefix_len: usize| -> usize {
            10usize.max(self.width.saturating_sub(2).saturating_sub(prefix_len))
        };
        let Some(tool_args) = tool_args else {
            return fg(
                self.theme.tool_title as u8,
                &bold(&truncate_to_width(
                    tool_name,
                    max_for(tool_name.len()),
                    &TruncateOptions::default(),
                )),
            );
        };

        match serde_json::from_str::<serde_json::Value>(tool_args) {
            Ok(args) => match tool_name {
                "shell" => {
                    let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
                    let first_line = cmd.split('\n').next().unwrap_or("");
                    let cmd_text = if !first_line.is_empty() {
                        first_line.to_string()
                    } else {
                        "...".to_string()
                    };
                    format!(
                        "{} {}",
                        fg(self.theme.tool_title as u8, &bold("$")),
                        truncate_to_width(&cmd_text, max_for(1), &TruncateOptions::default())
                    )
                }
                "read" => {
                    let file_path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                    let mut range_info = String::new();
                    if let Some(offset) = args.get("offset").and_then(|v| v.as_f64()) {
                        let start = offset;
                        let end: String = match args.get("limit").and_then(|v| v.as_f64()) {
                            Some(limit) => {
                                let e = start + limit - 1.0;
                                js_num(e)
                            }
                            None => String::new(),
                        };
                        let end_suffix = if end.is_empty() {
                            String::new()
                        } else {
                            format!("-{end}")
                        };
                        range_info = format!(":{}{}", js_num(start), end_suffix);
                    }
                    let max_path =
                        5usize.max(max_for(4).saturating_sub(visible_width_of(&range_info)));
                    let path_display = if !file_path.is_empty() {
                        fg(
                            self.theme.accent as u8,
                            &truncate_to_width(file_path, max_path, &TruncateOptions::default()),
                        )
                    } else {
                        fg(self.theme.tool_output as u8, "...")
                    };
                    format!(
                        "{} {}{}",
                        fg(self.theme.tool_title as u8, &bold("read")),
                        path_display,
                        fg(self.theme.error as u8, &range_info)
                    )
                }
                "write" => {
                    let file_path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                    let path_display = if !file_path.is_empty() {
                        fg(
                            self.theme.accent as u8,
                            &truncate_to_width(file_path, max_for(5), &TruncateOptions::default()),
                        )
                    } else {
                        fg(self.theme.tool_output as u8, "...")
                    };
                    format!(
                        "{} {}",
                        fg(self.theme.tool_title as u8, &bold("write")),
                        path_display
                    )
                }
                "edit" => {
                    let file_path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                    let path_display = if !file_path.is_empty() {
                        fg(
                            self.theme.accent as u8,
                            &truncate_to_width(file_path, max_for(4), &TruncateOptions::default()),
                        )
                    } else {
                        fg(self.theme.tool_output as u8, "...")
                    };
                    format!(
                        "{} {}",
                        fg(self.theme.tool_title as u8, &bold("edit")),
                        path_display
                    )
                }
                _ => {
                    let arg_summary = args.to_string();
                    let truncated = truncate_to_width(
                        &arg_summary,
                        max_for(tool_name.len()),
                        &TruncateOptions::default(),
                    );
                    format!(
                        "{} {}",
                        fg(self.theme.tool_title as u8, &bold(tool_name)),
                        fg(self.theme.tool_output as u8, &truncated)
                    )
                }
            },
            Err(_) => {
                let display_args = truncate_to_width(
                    tool_args,
                    max_for(tool_name.len()),
                    &TruncateOptions::default(),
                );
                format!(
                    "{} {}",
                    fg(self.theme.tool_title as u8, &bold(tool_name)),
                    fg(self.theme.tool_output as u8, &display_args)
                )
            }
        }
    }

    // ─── System message ────────────────────────────────────────────────

    fn render_system_message(&mut self, msg: &ChatMessage) {
        let wrap_width = self.width.saturating_sub(2).max(1);
        let lines: Vec<&str> = msg.content.split('\n').collect();
        if msg.welcome {
            for line in lines {
                if line.trim().is_empty() {
                    self.rendered_lines.push(RenderedLine {
                        text: String::new(),
                        dim: true,
                    });
                } else {
                    let wrapped = wrap_text_with_ansi(line, wrap_width);
                    for wl in wrapped {
                        self.rendered_lines.push(RenderedLine {
                            text: wl,
                            dim: true,
                        });
                    }
                }
            }
            return;
        }
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            let lower = line.to_lowercase();
            let is_error = lower.contains("error") || lower.contains("failed");
            let color = if is_error {
                fg(self.theme.error as u8, line)
            } else {
                fg(self.theme.dim as u8, line)
            };
            let wrapped = wrap_text_with_ansi(&color, wrap_width);
            for wl in wrapped {
                self.rendered_lines.push(RenderedLine {
                    text: format!(" {wl}"),
                    dim: true,
                });
            }
        }
    }
}

impl Component for ChatArea {
    fn render(&mut self, width: usize) -> Vec<String> {
        ChatArea::render(self, width)
    }

    fn handle_input(&mut self, _data: &str) {}

    fn invalidate(&mut self) {
        self.last_render_width = -1;
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────

/// Build the assistant and thinking markdown renderers for `theme`.
///
/// The assistant renderer takes the palette's markdown roles
/// ([`MarkdownTheme::from_theme`]), so `/theme` repaints the transcript instead
/// of leaving it on the dark constants baked into `MarkdownTheme::default`.
///
/// Thinking renders entirely in the thinking gray: every markdown element that
/// would normally get an accent color is mapped to thinkingText (bold/italic/
/// underline stay attribute-only; the reset-reapply pass in
/// renderAssistantMessage restores the gray).
fn build_markdown(theme: Theme) -> (MarkdownRenderer, MarkdownRenderer) {
    let md_theme = MarkdownTheme::from_theme(&theme);
    let tc = theme.thinking_text as u8;
    let think_fg = move |s: &str| fg(tc, s);
    let md_thinking = MarkdownRenderer::with_markdown_theme(md_theme.clone().with_partial(
        MarkdownThemePartial {
            heading: Some(std::rc::Rc::new(think_fg)),
            link: Some(std::rc::Rc::new(think_fg)),
            link_url: Some(std::rc::Rc::new(think_fg)),
            code: Some(std::rc::Rc::new(think_fg)),
            code_block: Some(std::rc::Rc::new(move |s: &str| fg(tc, &dim(s)))),
            code_block_border: Some(std::rc::Rc::new(move |s: &str| fg(tc, &dim(s)))),
            // `quote` is the field the blockquote arm actually reads (raw
            // fg+italic for both the text and its `│ ` border); the palette's
            // `md_quote` is overridden so a quote inside thinking keeps the
            // thinking gray, exactly as when the arm hardcoded 244.
            quote: Some(std::rc::Rc::new(think_fg)),
            hr: Some(std::rc::Rc::new(think_fg)),
            list_bullet: Some(std::rc::Rc::new(think_fg)),
            strikethrough: Some(std::rc::Rc::new(think_fg)),
            ..Default::default()
        },
    ));
    (MarkdownRenderer::with_markdown_theme(md_theme), md_thinking)
}

/// `/\x1b\[0?m/g` replace with `\x1b[0m{prefix}` (reapply style after resets).
fn reapply_style(line: &str, prefix: &str) -> String {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\x1b\[0?m").unwrap());
    let replacement = format!("\x1b[0m{prefix}");
    re.replace_all(line, replacement).into_owned()
}

fn visible_width_of(s: &str) -> usize {
    crate::utils::visible_width(s)
}

/// JS number formatting: `5.0` → "5", `5.5` → "5.5".
fn js_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e21 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 120;

    fn new_chat() -> ChatArea {
        ChatArea::new(W, None)
    }

    #[test]
    fn toggle_thinking_hidden_collapses_and_expands() {
        let mut chat = new_chat();
        chat.render(W);
        set_messages(
            &mut chat,
            vec![ChatMessage {
                id: "m".into(),
                role: ChatRole::Assistant,
                thinking: Some("step by step reasoning".into()),
                content: "final answer".into(),
                ..ChatMessage::new(String::new(), ChatRole::Assistant, "")
            }],
        );

        // Expanded by default: full thinking text is rendered.
        let expanded = chat.render_all(W);
        assert!(expanded
            .iter()
            .any(|l| l.contains("step by step reasoning")));

        // Collapse: thinking block disappears entirely (no placeholder);
        // the answer content stays visible and we stick to the bottom.
        assert!(chat.toggle_thinking_hidden());
        let collapsed = chat.render_all(W);
        assert!(!collapsed
            .iter()
            .any(|l| l.contains("step by step reasoning")));
        assert!(!collapsed.iter().any(|l| l.contains("Thinking...")));
        assert!(collapsed.iter().any(|l| l.contains("final answer")));
        assert!(chat.auto_scroll);

        // Expand again, still pinned to the bottom.
        assert!(!chat.toggle_thinking_hidden());
        let reexpanded = chat.render_all(W);
        assert!(reexpanded
            .iter()
            .any(|l| l.contains("step by step reasoning")));
        assert!(chat.auto_scroll);
    }

    #[test]
    fn collapsed_thinking_placeholder_only_while_streaming() {
        let mut chat = new_chat();
        chat.render(W);
        chat.set_thinking_hidden(true);

        // Thinking actively streaming (start_thinking marks the message
        // pending): one-line placeholder, never the reasoning text.
        chat.start_thinking();
        chat.append_thinking_delta("secret reasoning");
        let lines = chat.render_all(W);
        assert!(lines.iter().any(|l| l.contains("Thinking...")));
        assert!(!lines.iter().any(|l| l.contains("secret reasoning")));

        // Content starts (thinking done): placeholder gone, answer visible,
        // reasoning still hidden.
        chat.append_to_last_message("answer");
        let lines = chat.render_all(W);
        assert!(!lines.iter().any(|l| l.contains("Thinking...")));
        assert!(!lines.iter().any(|l| l.contains("secret reasoning")));
        assert!(lines.iter().any(|l| l.contains("answer")));

        // Run complete: still nothing but the answer.
        chat.mark_last_message_complete();
        let lines = chat.render_all(W);
        assert!(!lines.iter().any(|l| l.contains("Thinking...")));
        assert!(!lines.iter().any(|l| l.contains("secret reasoning")));
        assert!(lines.iter().any(|l| l.contains("answer")));
    }

    #[test]
    fn render_clamps_stale_viewport_after_content_shrink() {
        let mut chat = new_chat();
        chat.set_viewport_height(5);
        set_messages(
            &mut chat,
            (0..50)
                .map(|i| ChatMessage::new(format!("m{i}"), ChatRole::User, &format!("prompt {i}")))
                .collect(),
        );
        chat.render(W);
        // Scrolled up (auto_scroll off), then the content shrinks to nearly
        // nothing — render() must clamp the stale viewport instead of
        // slicing out of range.
        chat.set_auto_scroll(true); // jump to the bottom first
        chat.scroll_up(3);
        assert!(!chat.auto_scroll);
        assert!(chat.viewport_top > 0);
        chat.clear_messages();
        let lines = chat.render(W); // must not panic
        assert!(chat.viewport_top <= chat.rendered_lines.len());
        assert_eq!(lines.len(), chat.rendered_lines.len().min(5));
    }

    fn set_messages(chat: &mut ChatArea, messages: Vec<ChatMessage>) {
        chat.messages = messages;
        chat.rerender();
    }

    // ─── Compact activity (ctrl+d) ─────────────────────────────────────

    /// A completed tool call, the way a live run builds one (start, then end).
    fn completed_tool(name: &str, args: &str, body: &str) -> ChatMessage {
        let mut msg = ChatMessage::new(format!("t-{name}-{args}-{body}"), ChatRole::Tool, body);
        msg.name = Some(name.into());
        msg.tool = Some(format!("call-{name}-{args}"));
        msg.tool_args = Some(args.into());
        msg.tool_status = Some(ToolStatus::Complete);
        msg
    }

    fn read_of(path: &str) -> ChatMessage {
        completed_tool("read", &format!(r#"{{"path":"{path}"}}"#), "body\n")
    }

    /// The invariant the compact folds must not break: whatever the incremental
    /// paths did (appends, splices, folds), the transcript is exactly what a
    /// from-scratch layout of the same messages would produce.
    fn assert_matches_fresh_render(chat: &mut ChatArea) {
        let incremental = chat.render_all(W);
        // The ground truth is a full re-layout of the same messages, not
        // another walk through the incremental paths.
        let mut fresh = ChatArea::new(W, None);
        fresh.set_tool_output_expanded(chat.tool_output_expanded());
        fresh.set_thinking_hidden(chat.thinking_hidden);
        fresh.compact_activity = chat.compact_activity;
        set_messages(&mut fresh, chat.messages.clone());
        let expected = fresh.render_all(W);
        assert_eq!(
            incremental, expected,
            "incremental rendering drifted from a fresh layout"
        );
        // …and the line bookkeeping agrees with the lines it claims to describe.
        assert_eq!(
            chat.message_line_ranges.len(),
            chat.messages.len(),
            "every message has a range"
        );
        for (start, end) in &chat.message_line_ranges {
            assert!(
                *end < chat.rendered_lines.len() as i64,
                "range ({start}, {end}) is outside {} lines",
                chat.rendered_lines.len()
            );
            assert!(
                *end >= *start as i64 - 1,
                "range ({start}, {end}) is inverted"
            );
        }
        // Ranges never overlap: a fold's members are zero-width and sit between
        // the rows of the message before them and the one after.
        let mut previous_end: i64 = -1;
        for (start, end) in &chat.message_line_ranges {
            assert!(
                *start as i64 > previous_end,
                "range ({start}, {end}) overlaps the previous one"
            );
            previous_end = *end;
        }
    }

    /// The transcript's visible rows as plain text: ANSI stripped, the UI's
    /// leading indent and background padding trimmed, blank separator rows
    /// dropped.
    fn compact_lines(chat: &mut ChatArea) -> Vec<String> {
        render_trimmed(chat)
            .iter()
            .map(|line| strip(line).trim().to_string())
            .filter(|line| !line.is_empty())
            .collect()
    }

    #[test]
    fn compact_view_folds_a_run_of_calls_to_one_row() {
        let mut chat = new_chat();
        chat.render(W);
        for path in ["/a.rs", "/b.rs", "/c.rs"] {
            chat.add_message(read_of(path));
        }
        // Off by default: three calls, three rows.
        assert!(!chat.compact_activity());
        assert_eq!(
            compact_lines(&mut chat)
                .iter()
                .filter(|line| line.contains("read"))
                .count(),
            3
        );

        chat.set_compact_activity(true);
        let lines = compact_lines(&mut chat);
        assert_eq!(lines, vec!["▸ read 3 files"], "{lines:?}");

        // Toggling back restores every call: nothing was destroyed.
        chat.set_compact_activity(false);
        let lines = compact_lines(&mut chat);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines[0].contains("read /a.rs"), "{lines:?}");
    }

    #[test]
    fn compact_view_labels_each_tool_kind() {
        for (tool, args, expected) in [
            (
                "shell",
                [r#"{"command":"ls"}"#, r#"{"command":"pwd"}"#],
                "▸ $ 2 commands",
            ),
            (
                "read",
                [r#"{"path":"/a"}"#, r#"{"path":"/b"}"#],
                "▸ read 2 files",
            ),
            (
                "write",
                [r#"{"path":"/a"}"#, r#"{"path":"/b"}"#],
                "▸ write 2 files",
            ),
            (
                "edit",
                [r#"{"path":"/a"}"#, r#"{"path":"/b"}"#],
                "▸ edit 2 files",
            ),
            ("mcp_thing", [r#"{"x":1}"#, r#"{"x":2}"#], "▸ mcp_thing ×2"),
        ] {
            let mut chat = new_chat();
            chat.render(W);
            chat.set_compact_activity(true);
            for args in args {
                chat.add_message(completed_tool(tool, args, "out\n"));
            }
            let lines = compact_lines(&mut chat);
            assert_eq!(lines, vec![expected], "{tool}: {lines:?}");
            assert_matches_fresh_render(&mut chat);
        }
    }

    /// The count is the number of distinct files, matching the desktop's
    /// collapsed bursts: four reads of one file say `read 1 file`, because that
    /// is what happened. A call the fold cannot describe is still counted.
    /// A live failure is marked from the event's own structured result, so the
    /// fold cannot swallow it: the failed call keeps its row and its body.
    #[test]
    fn a_failed_call_is_not_folded_and_keeps_its_body() {
        let mut chat = new_chat();
        chat.render(W);
        chat.set_compact_activity(true);
        for path in ["/a.rs", "/b.rs"] {
            chat.add_tool_start(
                &format!("c{path}"),
                "read",
                Some(format!(r#"{{"path":"{path}"}}"#)),
            );
            chat.finish_tool(&format!("c{path}"), None, false);
        }
        assert_eq!(compact_lines(&mut chat), vec!["▸ read 2 files"]);

        chat.add_tool_start("boom", "read", Some(r#"{"path":"/c.rs"}"#.into()));
        chat.append_tool_delta("boom", "permission denied\n");
        chat.finish_tool("boom", None, true);
        let lines = compact_lines(&mut chat);
        assert_eq!(lines[0], "▸ read 2 files", "{lines:?}");
        assert!(
            lines.iter().any(|line| line.contains("read /c.rs")),
            "the failed call is its own row: {lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("permission denied")),
            "and it keeps its body: {lines:?}"
        );
        assert_matches_fresh_render(&mut chat);

        // The calls after it start a new run instead of joining the folded one.
        chat.add_tool_start("after", "read", Some(r#"{"path":"/d.rs"}"#.into()));
        chat.finish_tool("after", None, false);
        chat.add_tool_start("after2", "read", Some(r#"{"path":"/e.rs"}"#.into()));
        chat.finish_tool("after2", None, false);
        let lines = compact_lines(&mut chat);
        assert_eq!(lines[0], "▸ read 2 files", "{lines:?}");
        assert_eq!(lines.last().unwrap(), "▸ read 2 files", "{lines:?}");
        assert_matches_fresh_render(&mut chat);
    }

    #[test]
    fn compact_view_counts_distinct_files() {
        let mut chat = new_chat();
        chat.render(W);
        chat.set_compact_activity(true);
        chat.add_message(read_of("/same.rs"));
        chat.add_message(read_of("/same.rs"));
        assert_eq!(compact_lines(&mut chat), vec!["▸ read 1 file"]);

        chat.add_message(read_of("/other.rs"));
        assert_eq!(compact_lines(&mut chat), vec!["▸ read 2 files"]);

        // A call whose arguments are unreadable counts too, so the fold never
        // under-reports the work.
        let mut blind = completed_tool("read", "not json", "body\n");
        blind.tool_args = Some("not json".into());
        chat.add_message(blind);
        assert_eq!(compact_lines(&mut chat), vec!["▸ read 3 files"]);
        assert_matches_fresh_render(&mut chat);
    }

    #[test]
    fn compact_view_folds_only_same_kind_completed_runs() {
        let mut chat = new_chat();
        chat.render(W);
        chat.set_compact_activity(true);
        // read, read, shell: one fold and one lone call.
        chat.add_message(read_of("/a.rs"));
        chat.add_message(read_of("/b.rs"));
        chat.add_message(completed_tool("shell", r#"{"command":"ls"}"#, "out\n"));
        let lines = compact_lines(&mut chat);
        assert_eq!(lines, vec!["▸ read 2 files", "$ ls"], "{lines:?}");

        // A running call passes through and breaks the run around it.
        let mut chat = new_chat();
        chat.render(W);
        chat.set_compact_activity(true);
        chat.add_message(read_of("/a.rs"));
        chat.add_tool_start("live", "read", Some(r#"{"path":"/b.rs"}"#.into()));
        chat.add_message(read_of("/c.rs"));
        let lines = compact_lines(&mut chat);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert_eq!(lines[0], "read /a.rs");
        assert!(lines[1].contains("read /b.rs"), "{lines:?}");
        assert_eq!(lines[2], "read /c.rs");
        assert_matches_fresh_render(&mut chat);
    }

    /// A failure keeps its own row and its body: the reason is not something a
    /// fold may hide.
    #[test]
    fn compact_view_never_folds_a_failed_call() {
        let mut chat = new_chat();
        chat.render(W);
        chat.set_compact_activity(true);
        chat.add_message(read_of("/a.rs"));
        let mut failed = completed_tool("read", r#"{"path":"/b.rs"}"#, "boom\n");
        failed.tool_status = Some(ToolStatus::Error);
        chat.add_message(failed);
        let lines = compact_lines(&mut chat);
        assert_eq!(lines[0], "read /a.rs", "{lines:?}");
        assert!(
            lines.iter().any(|line| line.contains("read /b.rs")),
            "the failed call keeps its own row: {lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("boom")),
            "the failure body is visible: {lines:?}"
        );
        assert_matches_fresh_render(&mut chat);
    }

    #[test]
    fn compact_view_folds_thinking_to_one_marker() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("u".into(), ChatRole::User, "go"));
        let mut assistant = ChatMessage::new("a".into(), ChatRole::Assistant, "the answer");
        assistant.thinking = Some("line one\nline two\nline three".into());
        chat.add_message(assistant);
        // Without the compact view the reasoning is on screen in full.
        assert!(compact_lines(&mut chat)
            .iter()
            .any(|l| l.contains("line one")));

        chat.set_compact_activity(true);
        let lines = compact_lines(&mut chat);
        assert_eq!(lines, vec!["go", "▸ thinking", "the answer"], "{lines:?}");
        assert_matches_fresh_render(&mut chat);
    }

    #[test]
    fn compact_view_merges_consecutive_thinking_blocks() {
        let mut chat = new_chat();
        chat.render(W);
        chat.set_compact_activity(true);
        for (id, text) in [("a1", "first block"), ("a2", "second block")] {
            let mut msg = ChatMessage::new(id.into(), ChatRole::Assistant, "");
            msg.thinking = Some(text.into());
            chat.add_message(msg);
        }
        assert_eq!(compact_lines(&mut chat), vec!["▸ thinking ×2"]);
        assert_matches_fresh_render(&mut chat);
    }

    /// `ctrl+o` still wins over the compact marker: hiding thinking is an
    /// explicit request, and a marker is still thinking on screen.
    #[test]
    fn compact_view_does_not_show_hidden_thinking() {
        let mut chat = new_chat();
        chat.render(W);
        chat.set_compact_activity(true);
        chat.set_thinking_hidden(true);
        let mut msg = ChatMessage::new("a1".into(), ChatRole::Assistant, "answer");
        msg.thinking = Some("reasoning".into());
        chat.add_message(msg);
        assert_eq!(compact_lines(&mut chat), vec!["answer"]);
    }

    /// `ctrl+g` asks for every tool body; that cannot be reconciled with hiding
    /// the calls, so bodies win and the runs stay individual rows.
    #[test]
    fn expanded_tool_output_turns_folding_off() {
        let mut chat = new_chat();
        chat.render(W);
        chat.set_compact_activity(true);
        chat.add_message(read_of("/a.rs"));
        chat.add_message(read_of("/b.rs"));
        assert_eq!(compact_lines(&mut chat), vec!["▸ read 2 files"]);

        chat.set_tool_output_expanded(true);
        let lines = compact_lines(&mut chat);
        assert_eq!(lines.len(), 4, "two calls, each with its body: {lines:?}");
        assert!(lines[0].contains("read /a.rs"), "{lines:?}");
        // …and back again.
        chat.set_tool_output_expanded(false);
        assert_eq!(compact_lines(&mut chat), vec!["▸ read 2 files"]);
    }

    /// A run built the way a live run builds one — one call at a time — folds as
    /// it grows, and the transcript stays exactly what a fresh render produces.
    #[test]
    fn compact_view_folds_runs_built_incrementally() {
        let mut chat = new_chat();
        chat.render(W);
        chat.set_compact_activity(true);
        chat.add_message(ChatMessage::new("u".into(), ChatRole::User, "go"));
        for (i, path) in ["/a.rs", "/b.rs", "/c.rs", "/d.rs"].iter().enumerate() {
            let id = format!("call{i}");
            chat.add_tool_start(&id, "read", Some(format!(r#"{{"path":"{path}"}}"#)));
            // A running call is its own row…
            assert!(compact_lines(&mut chat).iter().any(|l| l.contains(path)));
            chat.append_tool_delta(&id, "body\n");
            chat.finish_tool(&id, None, false);
            assert_matches_fresh_render(&mut chat);
        }
        let lines = compact_lines(&mut chat);
        assert_eq!(lines, vec!["go", "▸ read 4 files"], "{lines:?}");
    }

    /// A think-tool-think-tool run — the shape interleaved reasoning produces —
    /// folds the calls and the reasoning, but never across a text answer.
    #[test]
    fn compact_view_folds_an_interleaved_run() {
        let mut chat = new_chat();
        chat.render(W);
        chat.set_compact_activity(true);
        chat.add_message(ChatMessage::new("u".into(), ChatRole::User, "go"));
        let mut first = ChatMessage::new("a1".into(), ChatRole::Assistant, "");
        first.thinking = Some("thinking one".into());
        chat.add_message(first);
        chat.add_message(completed_tool("shell", r#"{"command":"ls"}"#, "out\n"));
        chat.add_message(completed_tool("shell", r#"{"command":"pwd"}"#, "out\n"));
        let mut second = ChatMessage::new("a2".into(), ChatRole::Assistant, "");
        second.thinking = Some("thinking two".into());
        chat.add_message(second);
        let lines = compact_lines(&mut chat);
        assert_eq!(
            lines,
            vec!["go", "▸ thinking", "▸ $ 2 commands", "▸ thinking"],
            "{lines:?}"
        );
        assert_matches_fresh_render(&mut chat);
    }

    /// A text answer breaks a run: the folds on either side stay separate, and
    /// the answer itself is never hidden.
    #[test]
    fn compact_view_never_folds_across_an_answer() {
        let mut chat = new_chat();
        chat.render(W);
        chat.set_compact_activity(true);
        chat.add_message(read_of("/a.rs"));
        chat.add_message(read_of("/b.rs"));
        chat.add_message(ChatMessage::new("a".into(), ChatRole::Assistant, "halfway"));
        chat.add_message(read_of("/c.rs"));
        chat.add_message(read_of("/d.rs"));
        let lines = compact_lines(&mut chat);
        assert_eq!(
            lines,
            vec!["▸ read 2 files", "halfway", "▸ read 2 files"],
            "{lines:?}"
        );
        assert_matches_fresh_render(&mut chat);
    }

    /// Older history arriving above the transcript folds with what is already on
    /// screen, and keeps the reader anchored on the line they were reading.
    #[test]
    fn compact_view_folds_across_prepended_history() {
        let mut chat = new_chat();
        chat.render(W);
        chat.set_compact_activity(true);
        chat.add_message(read_of("/new.rs"));
        chat.add_message(read_of("/newer.rs"));
        assert_eq!(compact_lines(&mut chat), vec!["▸ read 2 files"]);
        chat.set_viewport_height(3);
        chat.render(W);

        // The older page's last call is the same tool, so the fold grows over
        // the seam instead of splitting into two rows.
        chat.prepend_messages(vec![read_of("/older.rs")]);
        assert_eq!(compact_lines(&mut chat), vec!["▸ read 3 files"]);
        assert_matches_fresh_render(&mut chat);
    }

    fn eager_lines(content: &str, pending: bool, width: usize) -> Vec<String> {
        let mut chat = ChatArea::new(width, None);
        chat.render(width);
        set_messages(
            &mut chat,
            vec![ChatMessage {
                id: "m".into(),
                role: ChatRole::Assistant,
                content: content.to_string(),
                pending,
                ..ChatMessage::new(String::new(), ChatRole::Assistant, "")
            }],
        );
        chat.render_all(width)
    }

    /// Stream `full` into `chat` in deterministic chunks, checking every frame.
    fn expect_streaming_matches_full_render(
        chat: &mut ChatArea,
        full: &str,
        width: usize,
    ) -> usize {
        let mut i = 0;
        let mut frames = 0;
        let mut n = 3usize;
        while i < full.len() {
            n = (n * 7 + 5) % 23 + 1;
            let end = (i + n).min(full.len());
            chat.append_to_last_message(&full[i..end]);
            i = end;
            let got = chat.render_all(width);
            let want = eager_lines(&full[..i], true, width);
            assert_eq!(got, want, "frame {frames} mismatch at offset {i}");
            frames += 1;
        }
        frames
    }

    #[test]
    fn nested_indented_fence_stream_matches_eager_render() {
        let mut chat = ChatArea::new(60, None);
        chat.render(60);
        let mut message = ChatMessage::new("m".into(), ChatRole::Assistant, "");
        message.pending = true;
        chat.add_message(message);
        expect_streaming_matches_full_render(
            &mut chat,
            "- item\n    ```ts\n    code\n\n    more code\n    ```\n\n",
            60,
        );
    }

    #[test]
    fn tiny_terminal_width_does_not_underflow() {
        for width in 0..=2 {
            let mut chat = ChatArea::new(width, None);
            chat.add_message(ChatMessage::new("u".into(), ChatRole::User, "hello"));
            let mut assistant = ChatMessage::new("a".into(), ChatRole::Assistant, "answer");
            assistant.thinking = Some("thinking".into());
            chat.add_message(assistant);
            let _ = chat.render(width);
        }
    }

    #[test]
    fn ten_queued_submissions_keep_canonical_run_ownership() {
        let mut chat = new_chat();
        chat.render(W);
        for index in 0..10 {
            let local_id = format!("local-{index}");
            chat.add_message(ChatMessage::new(
                local_id.clone(),
                ChatRole::User,
                &format!("prompt {index}"),
            ));
            chat.bind_user_run(
                &local_id,
                &format!("run-{index}"),
                if index == 0 {
                    RunState::Running
                } else {
                    RunState::Queued
                },
                None,
            );
        }
        chat.update_run_state("run-4", RunState::Running);
        chat.update_run_state("run-4", RunState::Terminal);
        assert_eq!(chat.messages.len(), 10);
        for (index, m) in chat.messages.iter().enumerate() {
            assert_eq!(m.run_id.as_deref(), Some(format!("run-{index}").as_str()));
        }
        assert_eq!(chat.messages[4].run_state, Some(RunState::Terminal));
        assert_eq!(
            chat.messages
                .iter()
                .filter(|m| m.run_state == Some(RunState::Queued))
                .count(),
            8
        );
    }

    #[test]
    fn queued_state_replay_reconstructs_bubbles_after_restart() {
        let mut chat = new_chat();
        chat.render(W);
        chat.upsert_queued_run("run-2", "second prompt", 2);
        chat.upsert_queued_run("run-1", "first prompt", 1);
        chat.upsert_queued_run("run-2", "ignored replacement", 1);
        assert_eq!(chat.messages.len(), 2);
        assert_eq!(chat.messages[0].id, "run-2");
        assert_eq!(chat.messages[0].content, "second prompt");
        assert_eq!(chat.messages[0].run_state, Some(RunState::Queued));
        assert_eq!(chat.messages[0].queue_position, Some(1));
    }

    #[test]
    fn deferred_deltas_are_not_rendered_until_flush() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("m".into(), ChatRole::Assistant, ""));
        let before = chat.render_all(W);
        chat.append_to_last_message("hello **world**");
        // Content mutated, but rendered lines stay stale until the next render.
        assert_ne!(chat.render_all(W), before);
        assert_eq!(chat.render_all(W), eager_lines("hello **world**", true, W));
    }

    #[test]
    fn incremental_prefix_cache_matches_full_render_at_every_frame() {
        let full = [
            "# Header\n\n",
            &("para **one** with `code` and more text wrapping around here. ".repeat(8) + "\n\n"),
            "```ts\nconst x = 1;\n// comment\n\nblank line inside fence\n```\n\n",
            "- item one\n- item two\n- item three\n\n",
            "| a | b |\n|---|---|\n| 1 | 2 |\n\n",
            "> a quote\n\n",
            "1. first\n2. second\n\n",
            "unclosed fence follows\n\n```python\nprint(1)\n",
        ]
        .concat();
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("m".into(), ChatRole::Assistant, ""));
        let frames = expect_streaming_matches_full_render(&mut chat, &full, W);
        assert!(frames > 50, "expected >50 frames, got {frames}");
    }

    #[test]
    fn link_reference_definitions_disable_prefix_caching_safely() {
        let full = "see [the docs] for details\n\nmore text\n\n[the docs]: https://example.com\n";
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("m".into(), ChatRole::Assistant, ""));
        expect_streaming_matches_full_render(&mut chat, full, W);
    }

    #[test]
    fn thinking_deltas_stream_incrementally_too() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage {
            id: "m".into(),
            role: ChatRole::Assistant,
            content: String::new(),
            thinking: Some(String::new()),
            ..ChatMessage::new(String::new(), ChatRole::Assistant, "")
        });
        let thinking = "reasoning **step** one\n\nreasoning step two\n\n";
        for chunk in thinking.as_bytes().chunks(5) {
            chat.append_thinking_delta(std::str::from_utf8(chunk).unwrap());
            chat.render_all(W); // must not throw; output checked at completion
        }
        chat.messages[0].pending = false;
        chat.rerender();
        let got = chat.render_all(W);

        let mut reference = new_chat();
        reference.render(W);
        set_messages(
            &mut reference,
            vec![ChatMessage {
                id: "m".into(),
                role: ChatRole::Assistant,
                content: String::new(),
                thinking: Some(thinking.to_string()),
                ..ChatMessage::new(String::new(), ChatRole::Assistant, "")
            }],
        );
        assert_eq!(got, reference.render_all(W));
    }

    #[test]
    fn width_change_mid_stream_invalidates_cached_prefix() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("m".into(), ChatRole::Assistant, ""));
        let first = "para one wraps differently at another width. ".repeat(6) + "\n\n";
        chat.append_to_last_message(&first);
        chat.render_all(W);

        let rest = "para two keeps streaming along here. ".repeat(6);
        chat.append_to_last_message(&rest);
        let narrow = chat.render_all(60);
        assert_eq!(narrow, eager_lines(&(first + &rest), true, 60));
    }

    #[test]
    fn completion_switches_to_a_clean_full_render() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("m".into(), ChatRole::Assistant, ""));
        let full = "# Title\n\nbody **bold**\n\n- a\n- b\n";
        for chunk in full.as_bytes().chunks(4) {
            chat.append_to_last_message(std::str::from_utf8(chunk).unwrap());
            chat.render_all(W);
        }
        chat.update_last_message(full);
        chat.messages[0].pending = false;
        chat.rerender();
        assert_eq!(chat.render_all(W), eager_lines(full, false, W));
    }

    #[test]
    fn duplicate_tool_start_for_the_same_id_merges_into_single_bubble() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_tool_start("call_1", "shell", Some(String::new()));
        chat.add_tool_start("call_1", "shell", Some("{\"command\":\"cd /tmp\"}".into()));
        assert_eq!(
            chat.messages
                .iter()
                .filter(|m| m.role == ChatRole::Tool)
                .count(),
            1
        );
        let lines: Vec<String> = chat
            .render_all(W)
            .into_iter()
            .filter(|l| !l.trim().is_empty())
            .collect();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains('$'));
        assert!(lines[0].contains("cd /tmp"));
    }

    #[test]
    fn distinct_tool_ids_still_render_separate_bubbles() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_tool_start("call_1", "shell", Some("{\"command\":\"cd /tmp\"}".into()));
        chat.add_tool_start("call_2", "read", Some("{\"path\":\"/etc/hosts\"}".into()));
        assert_eq!(
            chat.messages
                .iter()
                .filter(|m| m.role == ChatRole::Tool)
                .count(),
            2
        );
        let lines: Vec<String> = chat
            .render_all(W)
            .into_iter()
            .filter(|l| !l.trim().is_empty())
            .collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains('$'));
        assert!(lines[1].contains("read"));
    }

    // ─── display-fixes.test.ts ChatArea thinking cases ───────────────────

    #[test]
    fn thinking_renders_entirely_in_thinking_color() {
        let mut chat = ChatArea::new(60, None);
        chat.add_message(ChatMessage {
            id: "1".into(),
            role: ChatRole::Assistant,
            content: "answer".into(),
            thinking: Some("let me check `some code` and **bold text** first".into()),
            ..ChatMessage::new(String::new(), ChatRole::Assistant, "")
        });
        chat.render(60); // establish width
        let lines = chat.render(60);
        let thinking_lines: Vec<&String> = lines
            .iter()
            .filter(|l| l.contains("let me check") || l.contains("first"))
            .collect();
        assert!(!thinking_lines.is_empty());
        for line in thinking_lines {
            // Every ANSI reset in a thinking line must be immediately
            // followed by the thinking style prefix (italic + gray 244).
            let mut rest = line.as_str();
            while let Some(idx) = rest.find('\x1b') {
                let code_end = find_ansi_end(rest, idx);
                let code = &rest[idx..code_end];
                if code == "\x1b[0m" || code == "\x1b[m" {
                    let after = &rest[code_end..];
                    if !strip(after).trim().is_empty() {
                        assert!(
                            after.starts_with("\x1b[3m\x1b[38;5;244m"),
                            "reset not followed by thinking prefix: {line:?}"
                        );
                    }
                }
                rest = &rest[code_end..];
            }
            let plain = crate::utils::strip_ansi_codes(line);
            assert!(plain.contains("let me check some code and bold text first"));
        }
    }

    #[test]
    fn thinking_theme_styles_blockquote_and_code_block() {
        // Thinking with a blockquote and a fenced code block — exercises the
        // thinking theme's quote/code-block style closures.
        let mut chat = ChatArea::new(60, None);
        chat.add_message(ChatMessage {
            id: "1".into(),
            role: ChatRole::Assistant,
            content: "answer".into(),
            thinking: Some("> quoted thought\n\n```\ncode line\n```\n\n".into()),
            ..ChatMessage::new(String::new(), ChatRole::Assistant, "")
        });
        let lines = chat.render(60);
        let joined = lines
            .iter()
            .map(|l| crate::utils::strip_ansi_codes(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("quoted thought"));
        assert!(joined.contains("code line"));
    }

    #[test]
    fn thinking_never_leaks_markdown_accent_colors() {
        let mut chat = ChatArea::new(60, None);
        chat.add_message(ChatMessage {
            id: "1".into(),
            role: ChatRole::Assistant,
            content: "answer".into(),
            thinking: Some(
                [
                    "# Plan",
                    "check `some code` and **bold** plus [a link](https://example.com) here",
                    "- first item",
                    "```js",
                    "const x = 1;",
                    "```",
                ]
                .join("\n"),
            ),
            ..ChatMessage::new(String::new(), ChatRole::Assistant, "")
        });
        chat.render(60);
        let lines = chat.render(60);
        let needles = ["Plan", "some code", "a link", "first item", "const x = 1;"];
        let thinking_lines: Vec<&String> = lines
            .iter()
            .filter(|l| {
                needles
                    .iter()
                    .any(|n| crate::utils::strip_ansi_codes(l).contains(n))
            })
            .collect();
        for n in needles {
            assert!(
                thinking_lines
                    .iter()
                    .any(|l| crate::utils::strip_ansi_codes(l).contains(n)),
                "needle {n} missing"
            );
        }
        for line in &thinking_lines {
            // Every SGR foreground color must be the thinking gray (244).
            let mut rest = line.as_str();
            let mut colors: Vec<u32> = Vec::new();
            while let Some(idx) = rest.find("38;5;") {
                let after = &rest[idx + 5..];
                let num_end = after
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(after.len());
                if num_end > 0 {
                    colors.extend(after[..num_end].parse::<u32>().ok());
                }
                rest = &after[num_end..];
            }
            assert!(!colors.is_empty());
            for c in colors {
                assert_eq!(c, 244, "accent color leaked: {line:?}");
            }
        }
    }

    #[test]
    fn concatenates_consecutive_thinking_blocks_directly() {
        let mut chat = ChatArea::new(60, None);
        chat.add_message(ChatMessage::new("1".into(), ChatRole::Assistant, ""));
        chat.start_thinking();
        chat.append_thinking_delta("first block");
        chat.end_thinking();
        chat.start_thinking();
        chat.append_thinking_delta("second block");
        chat.end_thinking();
        let lines = chat.render(60);
        let plain = lines
            .iter()
            .map(|l| crate::utils::strip_ansi_codes(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(plain.contains("first blocksecond block"));
        assert!(!plain.contains("first block\n\nsecond block"));
    }

    #[test]
    fn shows_an_interrupted_marker_for_stopped_messages() {
        let mut chat = ChatArea::new(60, None);
        chat.add_message(ChatMessage::new(
            "1".into(),
            ChatRole::Assistant,
            "partial answer",
        ));
        chat.mark_last_assistant_stopped();
        let plain = chat
            .render(60)
            .iter()
            .map(|l| crate::utils::strip_ansi_codes(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(plain.contains("interrupted"));
    }

    #[test]
    fn preserves_blank_lines_in_user_messages() {
        let mut chat = ChatArea::new(60, None);
        chat.add_message(ChatMessage::new(
            "1".into(),
            ChatRole::User,
            "para one\n\npara two",
        ));
        let lines = chat.render(60);
        let texts: Vec<String> = lines
            .iter()
            .map(|l| crate::utils::strip_ansi_codes(l))
            .collect();
        let first = texts.iter().position(|t| t.contains("para one")).unwrap();
        let second = texts.iter().position(|t| t.contains("para two")).unwrap();
        assert!(second > first + 1, "blank line between paragraphs");
    }

    #[test]
    fn tool_call_formatting_shell() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_tool_start("c1", "shell", Some("{\"command\":\"cd /tmp\"}".into()));
        let lines: Vec<String> = chat
            .render_all(W)
            .into_iter()
            .filter(|l| !l.trim().is_empty())
            .collect();
        assert!(lines[0].contains('$'));
        assert!(lines[0].contains("cd /tmp"));
    }

    #[test]
    fn tool_call_formatting_read_with_range() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_tool_start(
            "c1",
            "read",
            Some("{\"path\":\"/etc/hosts\",\"offset\":10,\"limit\":5}".into()),
        );
        let lines: Vec<String> = chat
            .render_all(W)
            .into_iter()
            .filter(|l| !l.trim().is_empty())
            .collect();
        let plain = crate::utils::strip_ansi_codes(&lines[0]);
        assert!(plain.contains("/etc/hosts"));
        assert!(plain.contains(":10-14"));
    }

    #[test]
    fn tool_call_formatting_fallback_args() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_tool_start(
            "c1",
            "search_paper",
            Some("{\"query\":\"rust tui\"}".into()),
        );
        let lines: Vec<String> = chat
            .render_all(W)
            .into_iter()
            .filter(|l| !l.trim().is_empty())
            .collect();
        let plain = crate::utils::strip_ansi_codes(&lines[0]);
        assert!(plain.contains("search_paper"));
        assert!(plain.contains("rust tui"));
    }

    #[test]
    fn tool_call_invalid_json_renders_raw_args() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_tool_start("c1", "weird", Some("not json".into()));
        let lines: Vec<String> = chat
            .render_all(W)
            .into_iter()
            .filter(|l| !l.trim().is_empty())
            .collect();
        let plain = crate::utils::strip_ansi_codes(&lines[0]);
        assert!(plain.contains("weird"));
        assert!(plain.contains("not json"));
    }

    // ─── Tool output body + diff rendering ─────────────────────────────

    /// A tool call plus its streamed output, rendered once.
    fn tool_lines(chat: &mut ChatArea, tool: &str, args: &str, output: &str) -> Vec<String> {
        chat.render(W);
        chat.add_tool_start("c1", tool, Some(args.to_string()));
        chat.append_tool_delta("c1", output);
        chat.finish_tool("c1", None, false);
        render_trimmed(chat)
    }

    /// The same, in the state `ctrl+g` puts the transcript in — the body visible.
    fn expanded_tool_lines(
        chat: &mut ChatArea,
        tool: &str,
        args: &str,
        output: &str,
    ) -> Vec<String> {
        chat.set_tool_output_expanded(true);
        tool_lines(chat, tool, args, output)
    }

    /// `render_all` with the blank separator rows dropped.
    fn render_trimmed(chat: &mut ChatArea) -> Vec<String> {
        let mut lines = chat.render_all(W);
        while lines.last().is_some_and(|line| line.is_empty()) {
            lines.pop();
        }
        lines
    }

    fn diff_output() -> &'static str {
        "--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,4 @@\n context\n-removed\n+added\n+also added\n context\n"
    }

    #[test]
    fn a_tool_call_is_one_row_until_its_body_is_asked_for() {
        let mut chat = new_chat();
        let lines = tool_lines(&mut chat, "shell", r#"{"command":"ls"}"#, "alpha\nbeta\n");
        assert_eq!(lines.len(), 1, "the call is the whole entry: {lines:?}");
        assert!(strip(&lines[0]).contains("$ ls"));
        assert!(!lines.iter().any(|l| strip(l).contains("alpha")));

        chat.set_tool_output_expanded(true);
        let expanded = render_trimmed(&mut chat);
        assert_eq!(expanded.len(), 3, "row + 2 output lines: {expanded:?}");
        assert!(strip(&expanded[1]).contains("alpha"));
        assert!(strip(&expanded[2]).contains("beta"));
    }

    #[test]
    fn tool_output_rows_keep_the_exact_width_and_indent() {
        let mut chat = new_chat();
        let lines =
            expanded_tool_lines(&mut chat, "shell", r#"{"command":"ls"}"#, &"x".repeat(200));
        assert_eq!(visible_width_of(&lines[0]), W);
        assert!(strip(&lines[1]).starts_with("  x"));
        assert!(
            visible_width_of(&lines[1]) <= W,
            "never wider than the terminal"
        );
    }

    #[test]
    fn a_collapsed_tool_call_shows_no_preview_and_no_hint() {
        let mut chat = new_chat();
        let output: String = (0..12)
            .map(|i| format!("line {i}\n"))
            .collect::<Vec<_>>()
            .join("");
        let lines = tool_lines(&mut chat, "shell", r#"{"command":"ls"}"#, &output);
        assert_eq!(lines.len(), 1, "row only: {lines:?}");
        assert!(!lines.iter().any(|l| strip(l).contains("line 0")));
        assert!(!lines.iter().any(|l| strip(l).contains("more lines")));
        assert!(!chat.tool_output_expanded());
    }

    #[test]
    fn ctrl_g_expands_every_tool_output_body() {
        let mut chat = new_chat();
        let output: String = (0..12)
            .map(|i| format!("line {i}\n"))
            .collect::<Vec<_>>()
            .join("");
        let collapsed = tool_lines(&mut chat, "shell", r#"{"command":"ls"}"#, &output);
        assert_eq!(collapsed.len(), 1, "row only: {collapsed:?}");
        assert!(chat.toggle_tool_output_expanded());
        let lines = render_trimmed(&mut chat);
        assert_eq!(lines.len(), 13, "row + 12 output lines");
        assert!(strip(&lines[12]).contains("line 11"));
        assert!(!lines.iter().any(|l| strip(l).contains("more lines")));
        // Toggling back collapses again.
        assert!(!chat.toggle_tool_output_expanded());
        assert_eq!(render_trimmed(&mut chat).len(), 1);
        // Setting the same value is a no-op.
        chat.set_tool_output_expanded(false);
        assert!(!chat.tool_output_expanded());
    }

    #[test]
    fn a_unified_diff_body_is_parsed_and_summarised() {
        let mut chat = new_chat();
        let lines = tool_lines(&mut chat, "edit", r#"{"path":"src/a.rs"}"#, diff_output());
        // The tool row carries the +N -M badge; the body itself waits for ctrl+g.
        assert_eq!(lines.len(), 1, "row only: {lines:?}");
        let row = strip(&lines[0]);
        assert!(row.contains("+2 -1"), "{row:?}");

        // Expanded: every add/remove row is rendered and coloured.
        chat.set_tool_output_expanded(true);
        let expanded = render_trimmed(&mut chat);
        let body: Vec<String> = expanded.iter().map(|l| strip(l)).collect();
        assert!(
            body.iter().any(|l| l.contains("--- a/src/a.rs")),
            "{body:?}"
        );
        assert!(
            body.iter().any(|l| l.contains("+++ b/src/a.rs")),
            "{body:?}"
        );
        assert!(
            body.iter().any(|l| l.contains("@@ -1,3 +1,4 @@")),
            "{body:?}"
        );
        assert!(body.iter().any(|l| l.contains("+added")), "{body:?}");
        assert!(body.iter().any(|l| l.contains("+also added")), "{body:?}");
        assert!(body.iter().any(|l| l.contains("-removed")), "{body:?}");
        assert!(!body.iter().any(|l| l.contains("more lines")), "{body:?}");
        assert!(expanded
            .iter()
            .any(|l| l.contains(&format!("\x1b[38;5;{}m", DARK_THEME.success))));
        assert!(expanded
            .iter()
            .any(|l| l.contains(&format!("\x1b[38;5;{}m", DARK_THEME.error))));
    }

    #[test]
    fn a_long_diff_is_collapsed_and_expands_on_ctrl_g() {
        let mut chat = new_chat();
        let mut diff = String::from("--- a/f\n+++ b/f\n@@ -1,60 +1,60 @@\n");
        for i in 0..60 {
            diff.push_str(&format!("-old {i}\n+new {i}\n"));
        }
        let lines = tool_lines(&mut chat, "write", r#"{"path":"f"}"#, &diff);
        assert_eq!(lines.len(), 1, "collapsed diff is one row: {lines:?}");
        // Expanded shows the whole (123-line) diff — no marker left.
        chat.set_tool_output_expanded(true);
        let expanded = render_trimmed(&mut chat);
        assert_eq!(expanded.len(), 1 + 123, "every diff line is shown");
        assert!(!expanded.iter().any(|l| strip(l).contains("more lines")));

        // A diff bigger than the expanded cap is cut with a "truncated" marker.
        let mut chat = new_chat();
        let mut huge = String::from("--- a/f\n+++ b/f\n@@ -1,400 +1,400 @@\n");
        for i in 0..400 {
            huge.push_str(&format!("-old {i}\n+new {i}\n"));
        }
        tool_lines(&mut chat, "write", r#"{"path":"f"}"#, &huge);
        chat.set_tool_output_expanded(true);
        let expanded = render_trimmed(&mut chat);
        assert_eq!(expanded.len(), 1 + EXPANDED_TOOL_OUTPUT_ROWS + 1);
        let marker = strip(expanded.last().unwrap());
        assert!(marker.contains("truncated"), "{marker:?}");
        assert!(marker.contains("more lines"), "{marker:?}");
    }

    #[test]
    fn an_error_tool_body_is_coloured_with_the_error_palette() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_tool_start("c1", "shell", Some(r#"{"command":"false"}"#.into()));
        chat.append_tool_delta("c1", "boom\n");
        chat.messages.last_mut().unwrap().tool_status = Some(ToolStatus::Error);
        // A failure keeps its body while collapsed: the reason a call failed is
        // the one thing the one-row rule must not hide.
        let lines = render_trimmed(&mut chat);
        assert_eq!(lines.len(), 2, "row + the error body: {lines:?}");
        assert!(lines[1].contains(&format!("\x1b[38;5;{}m", DARK_THEME.error)));
        assert!(lines[0].contains(&format!("\x1b[48;5;{}m", DARK_THEME.tool_error_bg)));
    }

    #[test]
    fn a_long_error_body_is_still_truncated_with_its_hint() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_tool_start("c1", "shell", Some(r#"{"command":"false"}"#.into()));
        chat.append_tool_delta("c1", &"line\n".repeat(12));
        chat.messages.last_mut().unwrap().tool_status = Some(ToolStatus::Error);
        let lines = render_trimmed(&mut chat);
        assert_eq!(lines.len(), 1 + COLLAPSED_TOOL_OUTPUT_ROWS + 1, "{lines:?}");
        let marker = strip(lines.last().unwrap());
        assert!(marker.contains("8 more lines"), "{marker:?}");
        assert!(marker.contains("ctrl+g to expand"), "{marker:?}");
    }

    #[test]
    fn finish_tool_keeps_a_body_that_never_streamed() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_tool_start("c9", "read", None);
        chat.finish_tool("c9", Some("from tool_end\n"), false);
        chat.set_tool_output_expanded(true);
        let lines = render_trimmed(&mut chat);
        assert!(strip(&lines[1]).contains("from tool_end"));
        // A streamed body wins over the tool_end text.
        let mut chat = new_chat();
        chat.render(W);
        chat.add_tool_start("c9", "read", None);
        chat.append_tool_delta("c9", "streamed\n");
        chat.finish_tool("c9", Some("late\n"), false);
        chat.set_tool_output_expanded(true);
        let lines = render_trimmed(&mut chat);
        assert!(strip(&lines[1]).contains("streamed"));
        assert!(!lines.iter().any(|l| strip(l).contains("late")));
        // An unknown tool id is a no-op.
        chat.finish_tool("nope", Some("x"), false);
    }

    #[test]
    fn tool_output_ansi_is_stripped_but_the_layout_survives() {
        let mut chat = new_chat();
        let lines = expanded_tool_lines(
            &mut chat,
            "shell",
            r#"{"command":"ls --color"}"#,
            "\x1b[31mred\x1b[0m and \x1b]0;title\x07plain\n",
        );
        assert!(strip(&lines[1]).contains("red and plain"), "{:?}", lines[1]);
        assert!(!lines[1].contains("\x1b[31m"));
        assert!(visible_width_of(&lines[1]) <= W);
    }

    #[test]
    fn an_empty_tool_body_adds_no_rows() {
        let mut chat = new_chat();
        let lines = tool_lines(&mut chat, "shell", r#"{"command":"true"}"#, "   \n");
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(render_trimmed(&mut chat).len(), 1);
    }

    #[test]
    fn tool_output_parsing_handles_the_edge_shapes() {
        assert!(ToolOutput::parse("").is_empty());
        assert!(ToolOutput::parse("  \n\t\n").is_empty());
        let text = ToolOutput::parse("a\nb\n");
        assert_eq!(text.text_lines, vec!["a", "b"]);
        assert!(text.diff.is_empty());
        assert!(text.summary_ansi(&DARK_THEME).is_none());
        // A diff is sniffed, and only counted lines produce a badge.
        let unified = ToolOutput::parse(diff_output());
        assert!(!unified.diff.is_empty());
        assert!(unified.text_lines.is_empty());
        let badge = unified.summary_ansi(&DARK_THEME).unwrap();
        assert!(badge.contains("+2") && badge.contains("-1"), "{badge:?}");
        // apply_patch bodies are sniffed too.
        let patch = ToolOutput::parse(
            "*** Begin Patch\n*** Update File: a.rs\n@@\n-old\n+new\n*** End Patch\n",
        );
        assert!(!patch.diff.is_empty(), "{patch:?}");
        // A body with no added/removed lines has no badge.
        let context_only = ToolOutput::parse("--- a/f\n+++ b/f\n@@ -1 +1 @@\n same\n");
        assert!(!context_only.diff.is_empty());
        assert!(context_only.summary_ansi(&DARK_THEME).is_none());
        // Plain text is NOT sniffed as a diff (parse_unified_diff would call
        // every line a context/meta line).
        for plain in [
            "total 4\ndrwxr-xr-x  a\n",
            "hello world\n",
            "+not a diff\n",
            "--- only a dashes line\n",
        ] {
            let parsed = ToolOutput::parse(plain);
            assert!(
                parsed.diff.is_empty(),
                "{plain:?} is not a diff: {parsed:?}"
            );
            assert!(!parsed.text_lines.is_empty());
            assert!(parsed.summary_ansi(&DARK_THEME).is_none());
        }
        // Narrow widths produce no body rows rather than a panic.
        assert_eq!(unified.rows(0, false, false, &DARK_THEME), (vec![], 0));
        assert_eq!(unified.rows(2, false, false, &DARK_THEME), (vec![], 0));
        let (narrow, hidden) = text.rows(3, false, false, &DARK_THEME);
        assert_eq!(hidden, 0);
        assert_eq!(narrow.len(), 2);
        assert_eq!(crate::utils::visible_width(&narrow[0]), 3);
    }

    #[test]
    fn tool_output_rows_are_bounded_when_expanded() {
        let big: String = (0..EXPANDED_TOOL_OUTPUT_ROWS + 50)
            .map(|i| format!("line {i}\n"))
            .collect();
        let body = ToolOutput::parse(&big);
        let (rows, hidden) = body.rows(W, true, false, &DARK_THEME);
        assert_eq!(rows.len(), EXPANDED_TOOL_OUTPUT_ROWS);
        assert_eq!(hidden, 50);
        let (collapsed, hidden) = body.rows(W, false, false, &DARK_THEME);
        assert_eq!(collapsed.len(), COLLAPSED_TOOL_OUTPUT_ROWS);
        assert_eq!(
            hidden,
            EXPANDED_TOOL_OUTPUT_ROWS + 50 - COLLAPSED_TOOL_OUTPUT_ROWS
        );
    }

    /// `/theme light` must repaint markdown that is *already* on screen, not
    /// just content rendered after the switch: `set_theme` rebuilds both
    /// renderers from the new palette, so the same message text re-renders in
    /// the new colors (a cache-flag-only fix would leave the dark indices).
    #[test]
    fn set_theme_recolors_markdown_rendered_before_the_switch() {
        let mut chat = new_chat();
        chat.render(W);
        set_messages(
            &mut chat,
            vec![ChatMessage {
                id: "m".into(),
                role: ChatRole::Assistant,
                content: "# Title\n\ntext with `code` and [link](https://example.com)\n\n> quote\n"
                    .into(),
                ..ChatMessage::new(String::new(), ChatRole::Assistant, "")
            }],
        );
        let dark = render_trimmed(&mut chat).join("\n");
        assert!(dark.contains("38;5;221m"), "dark heading: {dark}");
        assert!(dark.contains("38;5;151m"), "dark code: {dark}");
        assert!(dark.contains("38;5;117m"), "dark link: {dark}");

        chat.set_theme(crate::themes::theme_by_id("light").expect("light palette"));
        let light = render_trimmed(&mut chat).join("\n");
        assert!(light.contains("38;5;130m"), "light heading: {light}");
        assert!(light.contains("38;5;30m"), "light code: {light}");
        assert!(light.contains("38;5;25m"), "light link: {light}");
        for gone in ["38;5;221m", "38;5;151m", "38;5;117m", "38;5;244m"] {
            assert!(!light.contains(gone), "still dark-themed ({gone}): {light}");
        }
        // The same text is still there — this is a repaint, not an empty render.
        assert!(strip(&light).contains("text with code and link"));
        assert!(strip(&light).contains("Title"));
    }

    #[test]
    fn set_theme_repaints_the_chat_and_rebuilds_the_markdown() {
        let mut chat = new_chat();
        let light = Theme {
            error: 9,
            tool_output: 10,
            ..DARK_THEME
        };
        assert_eq!(chat.theme(), DARK_THEME);
        chat.set_theme(light);
        assert_eq!(chat.theme(), light);
        chat.set_theme(light); // no-op, must not invalidate
        let mut chat2 = new_chat();
        chat2.render(W);
        chat2.add_tool_start("c1", "shell", Some(r#"{"command":"ls"}"#.into()));
        chat2.append_tool_delta("c1", "out\n");
        chat2.set_theme(light);
        chat2.set_tool_output_expanded(true);
        let lines = render_trimmed(&mut chat2);
        assert!(lines[1].contains("\x1b[38;5;10m"), "{:?}", lines[1]);
    }

    fn visible_width_of(line: &str) -> usize {
        crate::utils::visible_width(line)
    }

    #[test]
    fn scroll_up_down_behavior() {
        let mut chat = new_chat();
        chat.render(W);
        for i in 0..10 {
            chat.add_message(ChatMessage::new(
                format!("u{i}"),
                ChatRole::User,
                &format!("message {i}"),
            ));
        }
        chat.render(W);
        chat.set_viewport_height(5);
        chat.scroll_to_bottom();
        assert!(chat.is_at_bottom());
        assert!(chat.scroll_up(2));
        assert!(!chat.is_at_bottom());
        assert!(chat.scroll_down(2));
        assert!(chat.is_at_bottom());
        // at bottom: further scroll down returns false
        assert!(!chat.scroll_down(2));
    }

    #[test]
    fn welcome_message_renders_unprefixed() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage {
            id: "w".into(),
            role: ChatRole::System,
            content: "Welcome!\n\nGetting started".into(),
            welcome: true,
            ..ChatMessage::new(String::new(), ChatRole::System, "")
        });
        let plain = chat
            .render_all(W)
            .iter()
            .map(|l| crate::utils::strip_ansi_codes(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(plain.contains("Welcome!"));
        assert!(plain.contains("Getting started"));
    }

    #[test]
    fn system_error_line_uses_error_color() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage {
            id: "s".into(),
            role: ChatRole::System,
            content: "something failed here".into(),
            ..ChatMessage::new(String::new(), ChatRole::System, "")
        });
        let lines = chat.render_all(W);
        assert!(lines.iter().any(|l| l.contains("\x1b[38;5;204m")));
    }

    #[test]
    fn user_message_gets_background() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("u".into(), ChatRole::User, "hi there"));
        let lines = chat.render_all(W);
        assert!(lines[0].starts_with("\x1b[48;5;59m"));
    }

    #[test]
    fn queued_run_renders_queue_position() {
        let mut chat = new_chat();
        chat.render(W);
        chat.upsert_queued_run("run-1", "prompt", 3);
        let plain = chat
            .render_all(W)
            .iter()
            .map(|l| crate::utils::strip_ansi_codes(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(plain.contains("queued"));
        assert!(plain.contains("(#3)"));
    }

    #[test]
    fn queued_suffix_is_inside_dim() {
        // TS: dim(`queued${suffix}`) — the (#n) suffix is inside the dim
        // span. Parity harness caught the suffix leaking outside.
        let mut chat = new_chat();
        chat.render(W);
        chat.upsert_queued_run("run-1", "prompt", 3);
        let line = chat
            .render_all(W)
            .iter()
            .find(|l| l.contains("queued"))
            .unwrap()
            .clone();
        assert_eq!(
            line,
            format!(
                "\x1b[48;5;59m \x1b[2mqueued (#3)\x1b[0m\x1b[48;5;59m\x1b[0m\x1b[48;5;59m{}\x1b[0m",
                " ".repeat(W - 12)
            )
        );
    }

    #[test]
    fn clear_messages_resets() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("u".into(), ChatRole::User, "hi"));
        chat.clear_messages();
        assert!(chat.messages.is_empty());
        // rerender always pushes the trailing spacer line
        assert_eq!(chat.render_all(W), vec![""]);
    }

    #[test]
    fn append_to_last_message_after_tool_starts_new_assistant() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("u".into(), ChatRole::User, "prompt"));
        chat.add_tool_start("c1", "shell", Some("{\"command\":\"ls\"}".into()));
        chat.finish_tool("c1", None, false);
        chat.append_to_last_message("hello");
        // A fresh assistant message was pushed after the tool result.
        assert_eq!(chat.messages.last().unwrap().role, ChatRole::Assistant);
        assert_eq!(chat.messages.last().unwrap().content, "hello");
    }

    #[test]
    fn find_stream_cut_basic() {
        // cut = offset just past the last blank line outside any fence
        assert_eq!(find_stream_cut("para one\n\npara two\n\n"), 20);
        assert_eq!(find_stream_cut("no blank lines here"), 0);
        // blank lines inside a fence don't cut
        let fenced = "```\ncode\n\nmore\n```\n\n";
        assert_eq!(find_stream_cut(fenced), fenced.len());
        // The unterminated tail line is never a cut point — even when it
        // looks blank. Regression: "foo\n\n " used to return len+1 and the
        // incremental renderer then sliced out of bounds (exit 101).
        assert_eq!(find_stream_cut("foo\n\n "), 5);
        assert_eq!(find_stream_cut(" "), 0);
        assert_eq!(find_stream_cut("para\n\n  \n"), 9);
    }

    #[test]
    fn streaming_render_tolerates_whitespace_only_tail() {
        // End-to-end: the exact panic path from the crash log —
        // render_streaming_markdown with a trailing blank-but-unterminated
        // line must not panic.
        let mut caches = std::collections::HashMap::new();
        let mut md = MarkdownRenderer::new();
        let frames = [
            "para one\n",
            "para one\n\n",
            "para one\n\n ",
            "para one\n\n  \npara two",
        ];
        for frame in frames {
            let lines = ChatArea::render_streaming_markdown(&mut caches, "k", frame, 78, &mut md);
            let joined = lines
                .iter()
                .map(|l| crate::utils::strip_ansi_codes(l))
                .collect::<Vec<_>>()
                .join("\n");
            assert!(joined.contains("para one"), "frame {frame:?} lost content");
        }
    }

    #[test]
    fn find_stream_cut_never_exceeds_len() {
        // Blank final line with no trailing newline: cut must clamp to len
        // (previously returned len+1 → byte-index slice panic).
        let s = "para one\n\npara two\n ";
        let cut = find_stream_cut(s);
        assert!(cut <= s.len());
        // Same with multi-byte UTF-8 (e.g. CJK streamed text on Windows).
        let s = "第一段\n\n第二段\n ";
        let cut = find_stream_cut(s);
        assert!(cut <= s.len());
        assert!(s.is_char_boundary(cut));
        // Ends exactly with a newline is already at len.
        assert_eq!(find_stream_cut("a\n\n"), 3);
    }

    #[test]
    fn new_id_is_8_base36_chars() {
        for _ in 0..100 {
            let id = new_id();
            assert_eq!(id.len(), 8);
            assert!(id.chars().all(|c| c.is_ascii_alphanumeric()));
        }
    }

    /// Find the end of the ANSI escape sequence starting at `idx`.
    fn find_ansi_end(s: &str, idx: usize) -> usize {
        let rest = &s[idx..];
        let mut i = 2;
        while i < rest.len() {
            let c = rest.as_bytes()[i];
            if (0x40..=0x7e).contains(&c) {
                return idx + i + 1;
            }
            if !(0x20..=0x3f).contains(&c) {
                return idx + i;
            }
            i += 1;
        }
        idx + rest.len()
    }

    fn strip(s: &str) -> String {
        crate::utils::strip_ansi_codes(s)
    }

    // ─── RunState labels / helpers ────────────────────────────────────

    #[test]
    fn run_state_labels_match_ts() {
        assert_eq!(RunState::Queued.label(), "queued");
        assert_eq!(RunState::Running.label(), "running");
        assert_eq!(RunState::Terminal.label(), "terminal");
        assert_eq!(RunState::Failed.label(), "failed");
        assert_eq!(RunState::Cancelled.label(), "cancelled");
        assert_eq!(RunState::Superseded.label(), "superseded");
        assert_eq!(
            RunState::LostOnAgentRestart.label(),
            "lost_on_agent_restart"
        );
    }

    #[test]
    fn js_num_formats_like_js() {
        assert_eq!(js_num(5.0), "5");
        assert_eq!(js_num(5.5), "5.5");
    }

    #[test]
    fn seed_from_entropy_has_nonzero_fallback() {
        let nanos = 0x1234_5678_9abc_def0u64;
        let addr = (nanos ^ 0x9e37_79b9_7f4a_7c15u64).rotate_right(17);
        assert_eq!(seed_from_entropy(nanos, addr), 0x2545_f491_4f6c_dd1d);
        assert_eq!(seed_from_entropy(1, 0), 1u64 ^ 0x9e37_79b9_7f4a_7c15u64);
    }

    #[test]
    fn find_ansi_end_handles_invalid_and_unterminated() {
        // Invalid CSI parameter byte stops before it.
        assert_eq!(find_ansi_end("\x1b[\x01A", 0), 2);
        // Unterminated sequence consumes the rest.
        assert_eq!(find_ansi_end("x\x1b[1", 1), 4);
    }

    // ─── run binding / state update APIs ──────────────────────────────

    #[test]
    fn bind_user_run_guards_and_updates() {
        let mut chat = new_chat();
        // Unknown message id → no-op.
        chat.bind_user_run("missing", "r1", RunState::Running, None);
        // Non-user message → no-op.
        chat.add_message(ChatMessage::new("a1".into(), ChatRole::Assistant, "hi"));
        chat.bind_user_run("a1", "r1", RunState::Running, None);
        assert!(chat.messages[0].run_id.is_none());
        // User message → binds run identity.
        chat.add_message(ChatMessage::new("u1".into(), ChatRole::User, "hello"));
        chat.bind_user_run("u1", "r1", RunState::Running, Some(2));
        let m = &chat.messages[1];
        assert_eq!(m.id, "r1");
        assert_eq!(m.run_id.as_deref(), Some("r1"));
        assert_eq!(m.run_state, Some(RunState::Running));
        assert_eq!(m.queue_position, Some(2));
    }

    #[test]
    fn run_state_updates_by_run_id_and_message_id() {
        let mut chat = new_chat();
        // No matches → no-ops.
        chat.update_run_state("nope", RunState::Failed);
        chat.update_queue_position("nope", 3);
        chat.set_message_run_state("nope", RunState::Failed);

        chat.add_message(ChatMessage::new("u1".into(), ChatRole::User, "hello"));
        chat.bind_user_run("u1", "r1", RunState::Queued, Some(1));
        chat.update_run_state("r1", RunState::Running);
        assert_eq!(chat.messages[0].run_state, Some(RunState::Running));
        chat.update_queue_position("r1", 5);
        assert_eq!(chat.messages[0].queue_position, Some(5));
        chat.set_message_run_state("r1", RunState::Terminal);
        assert_eq!(chat.messages[0].run_state, Some(RunState::Terminal));
    }

    #[test]
    fn update_last_message_without_assistant_is_noop() {
        let mut chat = new_chat();
        chat.add_message(ChatMessage::new("u1".into(), ChatRole::User, "hello"));
        chat.update_last_message("ignored");
        assert_eq!(chat.messages[0].content, "hello");
    }

    // ─── on_change fan-out ────────────────────────────────────────────

    #[test]
    fn on_change_fires_across_mutation_paths() {
        use std::cell::Cell;
        use std::rc::Rc;
        let count = Rc::new(Cell::new(0));
        let mut chat = new_chat();
        let cb = Rc::clone(&count);
        chat.set_on_change(move || cb.set(cb.get() + 1));

        chat.render(W); // establishes the render width
        assert_eq!(count.get(), 0);
        // add_message fires via append_last_message AND its own callback.
        chat.add_message(ChatMessage::new("u1".into(), ChatRole::User, "q"));
        assert_eq!(count.get(), 2);
        chat.add_message(ChatMessage::new("a1".into(), ChatRole::Assistant, ""));
        assert_eq!(count.get(), 4);
        chat.append_to_last_message("delta"); // mark_message_dirty
        assert_eq!(count.get(), 5);
        chat.render(W); // flush: in-flight rerender must not re-fire
        assert_eq!(count.get(), 5);
        chat.mark_last_message_complete(); // rerender_message (not flushing)
        assert_eq!(count.get(), 6);
    }

    // ─── viewport / geometry APIs ─────────────────────────────────────

    #[test]
    fn width_viewport_and_scroll_accessors() {
        let mut chat = new_chat();
        for i in 0..5 {
            chat.add_message(ChatMessage::new(
                format!("u{i}"),
                ChatRole::User,
                "line one\nline two",
            ));
        }
        chat.render(W);
        let total = chat.render_all(W).len();
        assert!(total > 2);

        // set_width with a new width re-renders.
        chat.set_width(W - 20);
        chat.set_width(W - 20); // same width — no rerender
        chat.render(W - 20);

        // Viewport height clamp and scroll state.
        chat.set_viewport_height(2);
        chat.set_auto_scroll(true);
        assert!(!chat.is_at_top());
        assert!(chat.is_at_bottom());
        assert_eq!(chat.get_height(), 2);
        assert!(chat.scroll_up(1)); // leaves the bottom
        assert!(!chat.is_at_bottom());
        assert!(chat.scroll_down(1)); // back to the bottom → auto_scroll
        chat.set_viewport_height(10_000); // clamps viewport_top to 0
        chat.set_auto_scroll(false);
        chat.invalidate();
        assert_eq!(chat.last_render_width, -1);
    }

    #[test]
    fn component_trait_impl_delegates() {
        let mut chat = new_chat();
        chat.add_message(ChatMessage::new("u1".into(), ChatRole::User, "hi"));
        let lines = Component::render(&mut chat, W);
        assert!(!lines.is_empty());
        Component::handle_input(&mut chat, "ignored");
        Component::invalidate(&mut chat);
        assert_eq!(chat.last_render_width, -1);
        assert!(chat.as_any().downcast_ref::<ChatArea>().is_some());
        assert!(chat.as_any_mut().downcast_mut::<ChatArea>().is_some());
    }

    // ─── tool call paths ──────────────────────────────────────────────

    #[test]
    fn tool_and_thinking_before_first_render_defer() {
        // Without an established render width, mutations take the deferred
        // full-rerender path.
        let mut chat = new_chat();
        chat.start_thinking(); // empty messages → fresh thinking message
        chat.add_tool_start("c1", "shell", None);
        assert_eq!(chat.messages.len(), 2);
        assert!(chat.dirty);

        // After rendering, a thinking section following a user message is
        // appended incrementally.
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("u1".into(), ChatRole::User, "q"));
        chat.start_thinking();
        assert_eq!(chat.messages.len(), 2);
        assert_eq!(chat.messages[1].role, ChatRole::Assistant);
        assert!(chat.messages[1].thinking.is_some());
    }

    #[test]
    fn scroll_up_at_top_is_false() {
        let mut chat = new_chat();
        chat.render(W);
        assert!(!chat.scroll_up(1));
    }

    /// Prepending older history must leave the reader looking at the same line.
    ///
    /// `viewport_top` indexes `rendered_lines`, so an insert above it would
    /// otherwise scroll the transcript by the height of everything inserted —
    /// the reader would lose their place exactly when the page they asked for
    /// arrives.
    #[test]
    fn prepend_messages_anchors_the_viewport_and_reports_the_insert_height() {
        let mut chat = new_chat();
        set_messages(
            &mut chat,
            vec![
                ChatMessage::new("u1".into(), ChatRole::User, "first question"),
                ChatMessage::new("a1".into(), ChatRole::Assistant, "first answer"),
                ChatMessage::new("u2".into(), ChatRole::User, "newest question"),
                ChatMessage::new("a2".into(), ChatRole::Assistant, "newest answer"),
            ],
        );
        chat.set_viewport_height(3);
        chat.render(W);
        chat.scroll_to_bottom();
        assert!(
            chat.scroll_up(1),
            "the transcript is taller than the viewport"
        );
        assert!(
            !chat.auto_scroll,
            "scrolled up, so the view is not following"
        );
        let top_before = chat.viewport_top;
        let visible_before = chat.render(W);

        let added = chat.prepend_messages(vec![
            ChatMessage::new("u0".into(), ChatRole::User, "older question"),
            ChatMessage::new("a0".into(), ChatRole::Assistant, "older answer"),
        ]);

        assert!(added > 0, "the insert has height");
        assert_eq!(
            chat.viewport_top,
            top_before + added,
            "the viewport moved down by exactly what was inserted above it"
        );
        assert_eq!(
            chat.render(W),
            visible_before,
            "the same lines are still on screen"
        );
        assert!(
            !chat.auto_scroll,
            "prepending never drags the reader to the tail"
        );
        // The inserted rows really are above: scrolling up reveals them.
        chat.scroll_up(added + 1);
        assert_eq!(
            chat.viewport_top,
            top_before.saturating_sub(1),
            "scrolling up by the insert height lands one line above where the reader was"
        );
        chat.scroll_up(usize::MAX);
        assert_eq!(
            chat.viewport_top, 0,
            "the transcript now starts at the older page"
        );
        let revealed = crate::utils::strip_ansi_codes(&chat.render(W).join("\n"));
        assert!(revealed.contains("older question"), "{revealed}");
        assert_eq!(chat.messages[0].id, "u0");
        assert_eq!(chat.messages[1].id, "a0");
    }

    #[test]
    fn prepend_messages_with_nothing_to_add_is_a_no_op() {
        let mut chat = new_chat();
        set_messages(
            &mut chat,
            vec![ChatMessage::new("u1".into(), ChatRole::User, "q")],
        );
        chat.set_viewport_height(2);
        chat.render(W);
        chat.scroll_up(1);
        let top = chat.viewport_top;
        assert_eq!(chat.prepend_messages(Vec::new()), 0);
        assert_eq!(chat.viewport_top, top);
        assert_eq!(chat.messages.len(), 1);
    }

    /// A prepend before the first render cannot measure the insert (the layout
    /// is deferred until `render` learns the width), and neither can the reader
    /// have a scroll position yet — the next render lays the whole transcript
    /// out from its top.
    #[test]
    fn prepend_messages_before_the_first_render_defers_the_layout() {
        let mut chat = new_chat();
        let added =
            chat.prepend_messages(vec![ChatMessage::new("u0".into(), ChatRole::User, "early")]);
        assert_eq!(added, 0);
        assert!(chat.dirty);
        assert_eq!(chat.viewport_top, 0);
        let rendered = crate::utils::strip_ansi_codes(&chat.render(W).join("\n"));
        assert!(rendered.contains("early"), "{rendered}");
    }

    #[test]
    fn system_message_skips_blanks_and_dims_plain_lines() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new(
            "s1".into(),
            ChatRole::System,
            "plain note\n\nanother note",
        ));
        let plain: Vec<String> = chat.render_all(W).into_iter().map(|l| strip(&l)).collect();
        assert!(plain.iter().any(|l| l.contains("plain note")));
        assert!(plain.iter().any(|l| l.contains("another note")));
    }

    #[test]
    fn tool_start_update_delta_finish_cycle() {
        let mut chat = new_chat();
        chat.render(W);
        // Empty tool id: no dedup, always appends.
        chat.add_tool_start("", "shell", None);
        chat.add_tool_start("", "shell", None);
        assert_eq!(chat.messages.len(), 2);
        // Duplicate id updates the existing bubble (name/args fill-in).
        chat.add_tool_start("c1", "shell", None);
        chat.add_tool_start("c1", "read", Some("{\"path\":\"/x\"}".into()));
        assert_eq!(chat.messages.len(), 3);
        assert_eq!(chat.messages[2].name.as_deref(), Some("read"));
        assert_eq!(
            chat.messages[2].tool_args.as_deref(),
            Some("{\"path\":\"/x\"}")
        );
        // Streaming delta + finish.
        chat.append_tool_delta("c1", "partial");
        chat.append_tool_delta("unknown", "dropped");
        chat.finish_tool("c1", None, false);
        assert_eq!(chat.messages[2].tool_status, Some(ToolStatus::Complete));
        chat.finish_tool("unknown", None, false); // no-op
    }

    #[test]
    fn tool_error_status_uses_error_background() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_tool_start("c1", "shell", None);
        chat.messages[0].tool_status = Some(ToolStatus::Error);
        chat.invalidate();
        let lines = chat.render_all(W);
        assert!(lines.iter().any(|l| l.contains("48;5")));
    }

    #[test]
    fn tool_formatting_covers_remaining_variants() {
        let mut chat = new_chat();
        chat.render(W);
        // No args at all → bare bold tool name.
        chat.add_tool_start("c0", "shell", None);
        // shell with empty command → "..." placeholder.
        chat.add_tool_start("c1", "shell", Some("{\"command\":\"\"}".into()));
        // read with offset but no limit → open-ended range.
        chat.add_tool_start("c2", "read", Some("{\"path\":\"/f\",\"offset\":3}".into()));
        // read with no path → "..." placeholder.
        chat.add_tool_start("c3", "read", Some("{}".into()));
        // write / edit with and without paths.
        chat.add_tool_start("c4", "write", Some("{\"path\":\"/w\"}".into()));
        chat.add_tool_start("c5", "write", Some("{}".into()));
        chat.add_tool_start("c6", "edit", Some("{\"path\":\"/e\"}".into()));
        chat.add_tool_start("c7", "edit", Some("{}".into()));
        let plain: Vec<String> = chat
            .render_all(W)
            .into_iter()
            .map(|l| strip(&l))
            .filter(|l| !l.trim().is_empty())
            .collect();
        assert!(plain.iter().any(|l| l.trim() == "shell"));
        assert!(plain.iter().any(|l| l.contains("$ ...")));
        assert!(plain.iter().any(|l| l.contains("/f") && l.contains(":3")));
        assert!(plain.iter().any(|l| l.contains("read ...")));
        assert!(plain.iter().any(|l| l.contains("write /w")));
        assert!(plain.iter().any(|l| l.contains("write ...")));
        assert!(plain.iter().any(|l| l.contains("edit /e")));
        assert!(plain.iter().any(|l| l.contains("edit ...")));
    }

    // ─── thinking lifecycle ───────────────────────────────────────────

    #[test]
    fn thinking_lifecycle_edge_cases() {
        let mut chat = new_chat();
        // Deltas with no messages are dropped.
        chat.append_thinking_delta("x");
        chat.end_thinking();
        assert!(chat.messages.is_empty());

        // Thinking starts a fresh assistant message when the last message
        // is not assistant (or none exists).
        chat.add_message(ChatMessage::new("u1".into(), ChatRole::User, "q"));
        chat.start_thinking();
        assert_eq!(chat.messages.len(), 2);
        assert_eq!(chat.messages[1].role, ChatRole::Assistant);
        chat.append_thinking_delta("think");
        assert_eq!(chat.messages[1].thinking.as_deref(), Some("think"));
        chat.end_thinking();

        // A thinking delta with a user message queued after the streaming
        // assistant still lands on that assistant (not dropped, not on the
        // trailing user message).
        chat.add_message(ChatMessage::new("u2".into(), ChatRole::User, "q2"));
        chat.append_thinking_delta("dropped");
        assert_eq!(chat.messages[1].thinking.as_deref(), Some("thinkdropped"));
        assert!(chat.messages.last().unwrap().thinking.is_none());

        // start_thinking on an assistant without thinking adds the section.
        chat.append_to_last_message("answer");
        chat.start_thinking();
        let last = chat.messages.last().unwrap();
        assert!(last.thinking.is_some());
        chat.end_thinking(); // rerender with thinking present
    }

    // ─── user message run-state rendering ─────────────────────────────

    #[test]
    fn user_message_renders_terminal_run_states() {
        let mut chat = new_chat();
        chat.render(W);
        let mut msg = ChatMessage::new("u1".into(), ChatRole::User, "question");
        msg.run_state = Some(RunState::Cancelled);
        chat.add_message(msg);
        let plain: Vec<String> = chat.render_all(W).into_iter().map(|l| strip(&l)).collect();
        assert!(plain.iter().any(|l| l.contains("cancelled")));

        // Queued without a position renders a bare "queued" label.
        let mut chat = new_chat();
        chat.render(W);
        chat.upsert_queued_run("r9", "later work", 0);
        let last = chat.messages.last().unwrap();
        assert_eq!(last.run_state, Some(RunState::Queued));
        // upsert on an existing run updates state + position in place.
        let before = chat.messages.len();
        chat.upsert_queued_run("r9", "later work", 4);
        assert_eq!(chat.messages.len(), before);
        assert_eq!(chat.messages.last().unwrap().queue_position, Some(4));
    }

    // ─── streaming cache invalidation ─────────────────────────────────

    #[test]
    fn streaming_cache_drops_when_prefix_changes() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("a1".into(), ChatRole::Assistant, ""));
        chat.append_to_last_message("first paragraph\n\nsecond");
        chat.render(W);
        // A rewrite that no longer starts with the cached prefix invalidates
        // the streaming cache and still renders correctly.
        chat.update_last_message("completely different content");
        let plain: Vec<String> = chat.render(W).into_iter().map(|l| strip(&l)).collect();
        assert!(plain.iter().any(|l| l.contains("completely different")));
    }
}
