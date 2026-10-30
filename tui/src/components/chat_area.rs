//! ChatArea — scrollable chat view matching the TS style. 1:1 port of
//! `tui/src/components/chat-area.ts`.
//!
//! Renders messages with proper markdown, tool output, and streaming,
//! including the deferred re-render queue and the streaming prefix cache
//! (both TS streaming optimizations).

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use regex::Regex;

use crate::components::diff::{self, DiffTheme};
use crate::components::markdown::{MarkdownRenderer, MarkdownTheme, MarkdownThemePartial};
use crate::theme::{
    bold, dim, dim_raw, fg, fg_raw, italic, italic_raw, Chrome, Color, Theme, DARK_THEME,
};
use crate::tui::{Component, RESET};
use crate::utils::{
    apply_background_to_line, strip_ansi_codes, truncate_to_width, visible_width,
    wrap_text_with_ansi, TruncateOptions,
};

/// Body rows one call may print — the hard cap an expanded body gets, so a
/// `cat` of a huge file cannot make the diff renderer walk megabytes on every
/// frame.
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

/// How much of a settled reply the transcript keeps — the desktop's two
/// `replyDetail` levels, and only those two: `default` and `detailed`.
///
/// Neither level ever shows a tool call's body or a reasoning block's text
/// (`/reply-detail` is not what opens those; the row itself is — see
/// [`RowAction`]). What the level decides is whether a *settled* reply keeps
/// its process where it happened or folds it into one row above the answer it
/// ended on.
///
/// The terminal starts at [`Default`](Self::Default) every time: `/reply-detail`
/// is a view toggle for the run it is used in, not a preference written back to
/// `settings.json`. A reader who peeks at the process used to find every later
/// start expanded, because the peek *became* the stored level — the one thing a
/// terminal must not do with a verb that reads as "show me this once". The
/// level applies to every reply in the conversation at once: one use re-reads
/// the whole transcript, not the reply under the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReplyDetail {
    /// The desktop's `detailed`: every slice of a settled reply keeps its own
    /// row, in the order it happened — a tool call is one row, reasoning a
    /// `▸ thinking` marker, and every piece of prose the model wrote (the notes
    /// between its calls included) stays on screen. A reply that is still
    /// running uses the same shape: nothing folds while the reader is watching
    /// it arrive.
    Detailed,
    /// The desktop's `default`: a settled reply keeps only the answer it ended
    /// on, with the reasoning, calls and notes that produced it folded into one
    /// summary row above that answer. A reply that is still running is never
    /// folded this far — it reads like [`Detailed`](Self::Detailed) until
    /// `agent_end`, so the rows do not collapse and reopen while the model
    /// works (see [`ChatArea::begin_reply_run`]). That streaming shape is the
    /// desktop's too: all of the model's output stays on screen while work is
    /// folded.
    #[default]
    Default,
}

impl ReplyDetail {
    /// The level one `/reply-detail` moves to: the whole transcript flips
    /// between the answer alone and the answer with its process where it
    /// happened.
    pub fn next(self) -> Self {
        match self {
            Self::Detailed => Self::Default,
            Self::Default => Self::Detailed,
        }
    }
}

/// One rendered row the reader can stop on with `tab` and act on with `enter`
/// (or a click).
///
/// This is the terminal's answer to what the desktop does with a pointer: a
/// tool call and a reasoning block are rows that open when pressed, and a
/// folded stretch of work is a row that opens into the rows it stood for. The
/// terminal has no pointer to hover, so the rows are `tab` stops instead — the
/// same walk that already steps through links and pictures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowAction {
    /// A settled reply's whole process, folded into one summary row above its
    /// answer (`▸ 61 tool calls · 30 thoughts`). Opening it replays the reply's
    /// slices in place, their own rows still folded.
    Process { msg: usize },
    /// A burst of finished work folded where it happened (`▸ read 3 files`).
    /// Opening it puts the calls and thoughts it stood for back on screen, one
    /// row each.
    Burst { msg: usize },
    /// A tool call's own row: opening it shows the call's body.
    Tool { msg: usize },
    /// A `▸ thinking` marker: opening it shows the reasoning text.
    Thinking { msg: usize },
    /// The **copy** control under a message: this message's own text goes to the
    /// clipboard. A control, not a fold — pressing it changes nothing on screen,
    /// so it has no [`RowKind`] and never opens or closes a row.
    Copy { msg: usize },
    /// The **fork** control under a reply: branch the conversation at the user
    /// message that produced it, which is what the desktop's own fork button
    /// does (see `Live::fork_message`).
    Fork { msg: usize },
}

impl RowAction {
    /// Which open/close flag this row addresses, or `None` for a control.
    fn kind(&self) -> Option<RowKind> {
        match self {
            Self::Process { .. } => Some(RowKind::Process),
            Self::Burst { .. } => Some(RowKind::Burst),
            Self::Tool { .. } => Some(RowKind::Tool),
            Self::Thinking { .. } => Some(RowKind::Thinking),
            Self::Copy { .. } | Self::Fork { .. } => None,
        }
    }

    /// The message the row belongs to.
    ///
    /// For a fold this is the message the reveal flag lives on, and not always
    /// the message that *draws* the row: an opened fold's header is the first
    /// row of the reply's tree, while the fold itself is remembered against the
    /// reply's head — one flag, whichever of the two rows the reader presses.
    fn message(&self) -> usize {
        match self {
            Self::Process { msg }
            | Self::Burst { msg }
            | Self::Tool { msg }
            | Self::Thinking { msg }
            | Self::Copy { msg }
            | Self::Fork { msg } => *msg,
        }
    }
}

/// Which of a message's rows a reader opened. A message draws at most one row
/// per kind, so the pair (message id, kind) is the row's identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RowKind {
    /// A settled reply's whole process, folded above its answer.
    Process,
    /// A burst of finished work, folded where it happened.
    Burst,
    /// One tool call: its body waits behind its row.
    Tool,
    /// One reasoning block: its text waits behind its marker.
    Thinking,
}

/// Which rows of one message are open. Four flags rather than four sets: a
/// lookup on the render path is one `HashMap` probe, and a row that moves (a
/// reply above it grew) keeps its state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct OpenRows {
    process: bool,
    burst: bool,
    tool: bool,
    thinking: bool,
}

impl OpenRows {
    fn get(self, kind: RowKind) -> bool {
        match kind {
            RowKind::Process => self.process,
            RowKind::Burst => self.burst,
            RowKind::Tool => self.tool,
            RowKind::Thinking => self.thinking,
        }
    }

    fn toggle(&mut self, kind: RowKind) {
        let slot = match kind {
            RowKind::Process => &mut self.process,
            RowKind::Burst => &mut self.burst,
            RowKind::Tool => &mut self.tool,
            RowKind::Thinking => &mut self.thinking,
        };
        *slot = !*slot;
    }
}

/// One stop of the transcript's own walk (`tab`): the rendered row that draws
/// it, and what pressing `enter` on it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowStop {
    /// Index into the rendered rows (`WorkbenchData::messages`).
    pub row: usize,
    /// First display column of the row's label, and one past its last. The
    /// label, not the whole padded row: what is highlighted and what answers a
    /// click have to be the thing that reads as pressable.
    pub start: usize,
    pub end: usize,
    pub action: RowAction,
    /// Is the row currently open (its hidden rows are on screen)?
    pub expanded: bool,
}

/// What a folded stretch of a reply hides, as the summary row counts it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct FoldSummary {
    /// Completed tool calls folded away.
    tools: usize,
    /// Reasoning blocks folded away.
    thoughts: usize,
}

/// A summary row standing in for a settled reply's folded work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReplyFold {
    /// What the row counts.
    summary: FoldSummary,
}

/// The words a transcript's own process rows are drawn in.
///
/// The counts and markers of a folded reply are the terminal's chrome, not the
/// model's words, so they follow the interface language like every other label
/// on the surface: a Chinese session says `工具调用 61 次 · 思考 30 次` and a
/// `▸ 思考` marker, an English one `61 tool calls · 30 thoughts` and
/// `▸ thinking`. The words arrive from the workbench's copy rather than being
/// looked up here, because a component has no business reading the settings
/// file — and the English defaults keep a caller that never sets them (a unit
/// test, a transcript drawn off-screen) rendering exactly what it always did.
///
/// The counts are `{}` templates rather than strings built here, because the
/// order differs by language: English puts the noun last (`61 tool calls`),
/// Chinese puts it first (`工具调用 61 次`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FoldWords {
    /// One call: `{} tool call` / `工具调用 {} 次`.
    pub tool_call: &'static str,
    /// Several: `{} tool calls` / `工具调用 {} 次`.
    pub tool_calls: &'static str,
    /// One thought: `{} thought` / `思考 {} 次`.
    pub thought: &'static str,
    /// Several: `{} thoughts` / `思考 {} 次`.
    pub thoughts: &'static str,
    /// The marker standing in for hidden reasoning (`思考` / `thinking`).
    pub thinking: &'static str,
    /// The label a summary row falls back on when it counts nothing (a fold
    /// that only hid prose).
    pub process: &'static str,
    /// The copy control under a message (`[复制]` / `[copy]`).
    pub copy: &'static str,
    /// The fork control under a reply (`[分支]` / `[fork]`).
    pub fork: &'static str,
}

impl Default for FoldWords {
    fn default() -> Self {
        Self {
            tool_call: "{} tool call",
            tool_calls: "{} tool calls",
            thought: "{} thought",
            thoughts: "{} thoughts",
            thinking: "thinking",
            process: "process",
            copy: "copy",
            fork: "fork",
        }
    }
}

impl FoldWords {
    /// One counted word with `{}` replaced by `count`.
    fn counted(template: &'static str, count: usize) -> String {
        template.replace("{}", &count.to_string())
    }
}

/// The rows a transcript's folds resolve to: per message, the message that
/// renders it, the summary rows that stand in for folded work, and any folds
/// the reader opened.
#[derive(Debug, Default)]
struct FoldPlan {
    /// One entry per message: the index of the message that draws its rows
    /// (itself when nothing is folded into it).
    heads: Vec<usize>,
    /// Summary rows, keyed by the message that draws them.
    folds: HashMap<usize, ReplyFold>,
    /// Folds the reader opened, keyed by the message that draws each of their
    /// rows (see [`OpenedTree`]).
    trees: HashMap<usize, OpenedTree>,
}

/// A fold the reader opened: every row it stood for is back, indented under one
/// header row that folds them away again.
///
/// An opened fold has to keep a row of its own, or opening one would be a state
/// with no way back: the summary row used to *become* the rows it hid, so the
/// only thing that could close them was gone. The header is that row — the same
/// counts, one press away from folding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OpenedTree {
    /// The message the fold's reveal flag lives on: the reply's head for a
    /// settled reply's process, the burst's first slice for a burst.
    head: usize,
    /// Which flag that is — [`RowKind::Process`] or [`RowKind::Burst`].
    kind: RowKind,
    /// What the header counts. Only the fold's first row draws it.
    banner: Option<FoldSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Queued,
    Running,
    Terminal,
    Failed,
    Cancelled,
    Superseded,
    /// Absorbed into the run ahead of it (the agent's `merged` terminal ack):
    /// the submission's text is part of another run's prompt, so no run of its
    /// own ever starts.
    Merged,
    LostOnAgentRestart,
    /// The prompt was written but no acknowledgement came back. It may or may
    /// not have reached the agent; a blind retry could run it twice.
    NotAcknowledged,
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
            RunState::Merged => "merged into the previous run",
            RunState::LostOnAgentRestart => "lost_on_agent_restart",
            RunState::NotAcknowledged => "delivery not acknowledged",
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
    /// The journal entry this message was read from — the id `fork` branches at,
    /// and the reason a *live* message becomes forkable the moment the agent
    /// says which entry it wrote (see `ChatArea::bind_user_entry`).
    ///
    /// `None` for a row the agent has not persisted yet: a reply whose prompt is
    /// still being acknowledged is on screen before the entry exists, and a
    /// control that would silently fork the wrong place is worse than one that
    /// says it cannot yet.
    pub entry_id: Option<String>,
    /// Pictures this message carries, in the order they were attached.
    ///
    /// The journal keeps a picture as a *path* — no bytes travel with an entry
    /// — so these name files rather than hold them, and a path that has since
    /// been deleted paints as its caption alone.
    pub pictures: Vec<ChatPicture>,
}

/// A picture a message carries: the file, and the name its caption shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatPicture {
    /// The file, as the entry's metadata recorded it (an absolute path).
    pub path: String,
    /// The caption, already safe to sit inside markdown's `[…](…)` — see
    /// `session_rows::rows_to_messages`.
    pub name: String,
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
            entry_id: None,
            pictures: Vec::new(),
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

/// Marks the header of a fold the reader opened: the same row as
/// [`FOLD_MARKER`], turned down, and pressing it folds the work away again.
const OPEN_MARKER: &str = "▾";

/// The gutter an opened fold's rows hang under. Two columns — a space and the
/// rail — so the rail lands in column 2, directly under the header's `▸`, and
/// every row's text starts in the column the header's label does.
const TREE_RAIL: &str = "│";
const TREE_GUTTER: usize = 2;

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

/// Put `gutter` in front of a rendered row — outside the row's background when
/// it has one, so a call's band starts at the tree's text column and the rail
/// keeps the terminal's own background. A band painted across the rail read as
/// a break in the spine: the rail is the tree's edge, and a row's fill belongs
/// inside it.
///
/// The row is laid out one gutter narrower (see `render_message`), so the result
/// is exactly as wide as any other row.
fn with_gutter(text: &str, gutter: &str) -> String {
    format!("{gutter}{text}")
}

/// A row that carries nothing but the tree: a blank row, or one holding only
/// the rails that hang it (a block's trailing paragraph gap).
///
/// A block's padding at its end is these rows, and a spine that keeps running
/// below the last row it holds hangs over nothing — so `hang_under_rail` drops
/// them, while the same row *between* two of the block's rows keeps its rails
/// (that is what stops the spine breaking at a paragraph gap).
fn is_rail_only(text: &str) -> bool {
    let glyph = TREE_RAIL.chars().next().unwrap_or_default();
    strip_ansi_codes(text)
        .chars()
        .all(|c| c == ' ' || c == glyph)
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

/// Does this message carry reasoning text?
fn has_reasoning(msg: &ChatMessage) -> bool {
    msg.thinking
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty())
}

/// A reasoning block with no answer text of its own. It is neither work a
/// summary counts nor a slice that gets folded away: when one heads a fold (the
/// newest slice of a reply that has not written prose yet) its `▸ thinking`
/// marker is the row the reader sees.
fn reasoning_only(msg: &ChatMessage) -> bool {
    msg.role == ChatRole::Assistant && msg.content.trim().is_empty() && has_reasoning(msg)
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

/// Is `fresh` the fold layout `old` describes, plus standalone messages at the end?
///
/// Appending a message that renders its own rows leaves every message already
/// on screen exactly as it was, so the incremental paths can append it. A fold —
/// or a fold dissolving — rewrites an *earlier* message's rows, which the
/// incremental paths cannot express.
fn fold_layout_extends(old: &[usize], fresh: &[usize]) -> bool {
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
        diff_badge(&self.diff, theme)
    }

    /// Body rows plus the number of rows left out.
    ///
    /// Only an expanded body asks for these: collapsed, a call is one row and
    /// this is never called (see `render_tool_message`).
    fn rows(&self, width: usize, theme: &Theme) -> (Vec<String>, usize) {
        if self.is_empty() || width <= TOOL_OUTPUT_INDENT {
            return (Vec::new(), 0);
        }
        let limit = EXPANDED_TOOL_OUTPUT_ROWS;

        if !self.diff.is_empty() {
            return diff_body_rows(&self.diff, width, theme);
        }

        let inner = width - TOOL_OUTPUT_INDENT;
        let total = self.text_lines.len();
        let colour = theme.tool_output as u8;
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

/// `+N -M` in the row's colours, for a block that has any change to count.
///
/// A side that changed nothing is left out: a pure insertion is `+1`, not
/// `+1 -0` — the zero carries no information and takes a column.
fn diff_badge(lines: &[diff::DiffLine], theme: &Theme) -> Option<String> {
    if lines.is_empty() {
        return None;
    }
    let stats = diff::diff_stats(lines);
    if stats.added == 0 && stats.removed == 0 {
        return None;
    }
    let mut badge = String::new();
    if stats.added > 0 {
        badge.push_str(&fg(theme.success as u8, &format!("+{}", stats.added)));
    }
    if stats.removed > 0 {
        if !badge.is_empty() {
            badge.push(' ');
        }
        badge.push_str(&fg(theme.error as u8, &format!("-{}", stats.removed)));
    }
    Some(badge)
}

/// One rendered diff body: rows indented to [`TOOL_OUTPUT_INDENT`] plus the
/// number of lines the [`EXPANDED_TOOL_OUTPUT_ROWS`] budget left out.
fn diff_body_rows(lines: &[diff::DiffLine], width: usize, theme: &Theme) -> (Vec<String>, usize) {
    if lines.is_empty() || width <= TOOL_OUTPUT_INDENT {
        return (Vec::new(), 0);
    }
    let inner = width - TOOL_OUTPUT_INDENT;
    let limit = EXPANDED_TOOL_OUTPUT_ROWS;
    // `render_diff` reserves a row for its own `… N more lines` marker
    // whenever it truncates, so ask for one more row than the budget and drop
    // the marker (the caller owns the marker text).
    let mut rows = diff::render_diff(lines, inner, &diff_theme(theme), limit.saturating_add(1));
    let hidden = lines.len().saturating_sub(limit);
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
    (rows, hidden)
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

// ─── An `edit` call's body ────────────────────────────────────────────────

/// Unchanged lines kept on each side of a change in an opened `edit` call.
///
/// The model supplies its own context in `oldText` (enough to match the spot
/// uniquely), so this only *trims* — a call that replaces one line inside a
/// forty-line block would otherwise draw forty `-` rows and forty `+` rows for
/// a one-line change. Three is the same context a unified diff carries.
const EDIT_CONTEXT_LINES: usize = 3;

/// Argument keys that hold the text being replaced, and its replacement.
///
/// `edit` accepts the camelCase spelling the tool schema declares and the
/// snake_case aliases its handler does (`agent/src/tools/mod.rs`); a replay of
/// an older journal carries whichever the model emitted.
const EDIT_OLD_KEYS: [&str; 4] = ["oldText", "old_text", "oldString", "old_string"];
const EDIT_NEW_KEYS: [&str; 4] = ["newText", "new_text", "newString", "new_string"];

/// A string literal read off a tool-argument blob, told apart by whether the
/// document has a `:` after it.
#[derive(Debug, PartialEq, Eq)]
enum JsonChunk {
    Key(String),
    Value(String),
}

/// Every complete string literal in `text`, in order.
///
/// Arguments arrive as a stream of snapshots (`tool_delta`), so the blob is a
/// whole document only once the call is over. Parsing it strictly would show
/// the raw JSON for the entire call and switch to the patch at the very end;
/// string literals, by contrast, are self-delimiting, so scanning for them
/// renders the change while it is still being written. The unterminated
/// literal at the tail is simply dropped until its closing quote arrives.
fn json_string_chunks(text: &str) -> Vec<JsonChunk> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'"' {
            index += 1;
            continue;
        }
        // The closing quote, honouring escapes. Only ASCII bytes are compared,
        // and an escape is ASCII in valid JSON — a stray backslash before a
        // multi-byte character is caught by the char-boundary check below.
        let mut end = index + 1;
        let mut closed = false;
        while end < bytes.len() {
            match bytes[end] {
                b'\\' => end += 2,
                b'"' => {
                    closed = true;
                    break;
                }
                _ => end += 1,
            }
        }
        let Some(literal) = closed.then(|| text.get(index..=end)).flatten() else {
            break;
        };
        let Ok(decoded) = serde_json::from_str::<String>(literal) else {
            index = end + 1;
            continue;
        };
        let mut next = end + 1;
        while next < bytes.len() && bytes[next].is_ascii_whitespace() {
            next += 1;
        }
        out.push(if next < bytes.len() && bytes[next] == b':' {
            JsonChunk::Key(decoded)
        } else {
            JsonChunk::Value(decoded)
        });
        index = end + 1;
    }
    out
}

/// The `(old, new)` text pairs of an `edit` call's arguments, in the order the
/// model wrote them.
///
/// Handles both shapes the tool accepts — `oldText`/`newText` at the top level
/// and a batch of them under `edits` — and pairs them positionally: an old text
/// opens a pair, the next new text fills it. A trailing old text (or a lone new
/// one) is a deletion (an insertion) and keeps its empty other side.
fn edit_text_pairs(args: &str) -> Vec<(Option<String>, Option<String>)> {
    let chunks = json_string_chunks(args);
    let mut pairs: Vec<(Option<String>, Option<String>)> = Vec::new();
    for (index, chunk) in chunks.iter().enumerate() {
        let JsonChunk::Key(key) = chunk else {
            continue;
        };
        let Some(JsonChunk::Value(value)) = chunks.get(index + 1) else {
            continue;
        };
        if EDIT_OLD_KEYS.contains(&key.as_str()) {
            pairs.push((Some(value.clone()), None));
        } else if EDIT_NEW_KEYS.contains(&key.as_str()) {
            match pairs.last_mut() {
                Some(slot) if slot.1.is_none() => slot.1 = Some(value.clone()),
                _ => pairs.push((None, Some(value.clone()))),
            }
        }
    }
    pairs
}

/// A block's lines, with a terminating newline ending the last line rather
/// than starting an empty one, and CRLF reduced to LF.
fn split_block(text: Option<&str>) -> Vec<&str> {
    let Some(text) = text else {
        return Vec::new();
    };
    let mut lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();
    if lines.len() > 1 && lines.last() == Some(&"") {
        lines.pop();
    }
    lines
}

fn diff_line(kind: diff::DiffLineKind, text: String) -> diff::DiffLine {
    diff::DiffLine {
        kind,
        text,
        old_no: None,
        new_no: None,
    }
}

/// `… N lines`, the marker for context an opened call trimmed away.
fn elided_line(count: usize) -> diff::DiffLine {
    diff_line(
        diff::DiffLineKind::Meta,
        format!("… {count} line{}", if count == 1 { "" } else { "s" }),
    )
}

/// Appends one `(old, new)` pair to `out` as a small unified-diff block:
/// up to [`EDIT_CONTEXT_LINES`] unchanged lines either side of the change
/// (`… N lines` where more were dropped), the replaced lines, the
/// replacement's.
///
/// Common leading and trailing lines are what the model passed as context; the
/// middle is the change whether or not it is a clean single-line swap. Numbers
/// are not fabricated — the call carries no offsets into the file, so the rows
/// render without a gutter.
fn push_edit_pair(old: Option<&str>, new: Option<&str>, out: &mut Vec<diff::DiffLine>) {
    let old_lines = split_block(old);
    let new_lines = split_block(new);

    let mut head = 0;
    while head < old_lines.len() && head < new_lines.len() && old_lines[head] == new_lines[head] {
        head += 1;
    }
    let mut tail = 0;
    while tail < old_lines.len() - head
        && tail < new_lines.len() - head
        && old_lines[old_lines.len() - 1 - tail] == new_lines[new_lines.len() - 1 - tail]
    {
        tail += 1;
    }

    let keep_head = head.min(EDIT_CONTEXT_LINES);
    if head > keep_head {
        out.push(elided_line(head - keep_head));
    }
    for line in &old_lines[head - keep_head..head] {
        out.push(diff_line(diff::DiffLineKind::Context, format!(" {line}")));
    }
    for line in &old_lines[head..old_lines.len() - tail] {
        out.push(diff_line(diff::DiffLineKind::Remove, format!("-{line}")));
    }
    for line in &new_lines[head..new_lines.len() - tail] {
        out.push(diff_line(diff::DiffLineKind::Add, format!("+{line}")));
    }
    let keep_tail = tail.min(EDIT_CONTEXT_LINES);
    for line in &old_lines[old_lines.len() - keep_tail..] {
        out.push(diff_line(diff::DiffLineKind::Context, format!(" {line}")));
    }
    if tail > keep_tail {
        out.push(elided_line(tail - keep_tail));
    }
}

/// The patch an `edit` call *is*, as diff rows — empty for every other tool,
/// for arguments that carry no replacement text, and for a call whose text
/// replaces itself with itself.
///
/// `edit` is the one tool whose arguments are the change: `oldText`/`newText`
/// pairs, or a batch of them. Opening the call used to show the raw argument
/// JSON — a wall of escaped `\n` in which the actual edit had to be
/// reassembled by eye, and whose newlines the terminal cannot show at all.
/// Rendering it as the patch it is says the same thing in as many rows as the
/// change has lines.
fn edit_args_diff(tool_name: &str, tool_args: Option<&str>) -> Vec<diff::DiffLine> {
    if tool_name != "edit" {
        return Vec::new();
    }
    let Some(args) = tool_args else {
        return Vec::new();
    };
    let pairs = edit_text_pairs(args);
    if pairs.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    for (index, (old, new)) in pairs.iter().enumerate() {
        // A batch is several changes to one file: the separators say where one
        // ends and the next begins, the way a hunk header does.
        if pairs.len() > 1 {
            out.push(diff_line(
                diff::DiffLineKind::Hunk,
                format!("@@ {}/{}", index + 1, pairs.len()),
            ));
        }
        push_edit_pair(old.as_deref(), new.as_deref(), &mut out);
    }
    // Nothing replaced anything: the arguments are not a readable patch, so
    // leave the call to its ordinary body rather than drawing context alone.
    if !out.iter().any(|line| {
        matches!(
            line.kind,
            diff::DiffLineKind::Add | diff::DiffLineKind::Remove
        )
    }) {
        return Vec::new();
    }
    out
}

// ─── ChatArea ─────────────────────────────────────────────────────────────

pub struct ChatArea {
    messages: Vec<ChatMessage>,
    viewport_top: usize,
    viewport_height: usize,
    rendered_lines: Vec<RenderedLine>,
    /// The full transcript as flat owned rows, shared with the frame. Rebuilt
    /// only when `rendered_lines` changes, so composing several frames for one
    /// keystroke neither copies nor re-flattens the conversation.
    flat_cache: Option<Arc<[String]>>,
    auto_scroll: bool,
    width: usize,
    /// The rows the reader opened, by message id and row kind: a settled
    /// reply's folded process, a burst of work, a tool call's body, a reasoning
    /// block's text. Keyed by kind because one message can draw two rows (a
    /// settled reply whose head is a lone reasoning block keeps both its
    /// summary row and its `▸ thinking` marker), and the two open separately.
    ///
    /// A view state, not a preference: it lives as long as the transcript does
    /// and is dropped when the conversation is left.
    revealed: std::collections::HashMap<String, OpenRows>,
    last_render_width: i64,
    dirty: bool,
    pending_rerender: BTreeSet<usize>,
    flushing: bool,
    /// How many times the whole transcript has been rebuilt from scratch (see
    /// [`Self::full_renders`]).
    full_renders: u64,
    stream_caches: std::collections::HashMap<String, StreamRenderCache>,

    md: MarkdownRenderer,
    md_thinking: MarkdownRenderer,
    theme: Theme,
    /// `ctrl+d`: fold runs of consecutive tool calls and thinking blocks into
    /// compact summary rows (see [`Self::compact_heads`]). Off by default: the
    /// transcript renders exactly as it does without it.
    compact_activity: bool,
    /// `/reply-detail`: how much of a settled reply stays on screen (see
    /// [`ReplyDetail`]). `default` keeps the answer a reply ends on and folds
    /// the process that produced it into one summary row; `detailed` keeps
    /// every slice where it happened. Neither level shows a call's body or a
    /// reasoning block's text — the rows do, one at a time (`/reply-detail`
    /// moves the level; it has no chord).
    reply_detail: ReplyDetail,
    /// The words the process rows are drawn in — the reader's interface
    /// language, not the model's output (see [`FoldWords`]).
    fold_words: FoldWords,
    /// Is a run writing the last reply right now? Set by
    /// [`Self::begin_reply_run`] and cleared by [`Self::end_reply_run`].
    ///
    /// The reading level needs this and cannot infer it from the messages: a
    /// slice stops growing (and loses `pending`) every time the model moves on
    /// to a call or a new thought, so between two slices a reply in flight
    /// looks exactly like a settled one. Without the run's own state the whole
    /// reply folded and unfolded on every step — the rows collapsed while the
    /// answer was still being written, which is what the desktop never does.
    run_in_flight: bool,
    /// The compact layout the rendered lines were built with — one entry per
    /// message, the index of the message that renders it (itself when it
    /// renders its own rows). Compared against a freshly computed layout to
    /// notice when a fold *appeared* (a tool completing into the run above it,
    /// which changes an earlier message's rows) and the transcript has to be
    /// rebuilt. Only meaningful while a fold is possible at all.
    compact_layout: Vec<usize>,
    /// Per message index of the message that renders a settled reply's folded
    /// work. Keyed by the message that *renders* the summary, and empty for
    /// messages that fold nothing.
    reply_folds: HashMap<usize, ReplyFold>,
    /// Folds the reader opened, keyed by the message drawing each of their rows
    /// (see [`OpenedTree`]): their rows hang under one header that folds them
    /// away again. Kept beside [`Self::reply_folds`] — the two are halves of one
    /// layout, so a row that moves or a level that flips keeps its state.
    opened_trees: HashMap<usize, OpenedTree>,
    /// Is the render in flight drawing a row that hangs inside an opened fold?
    /// Set around one message's own rows, and what makes a detail (a call's
    /// body, a thought's text) hang one level deeper than its parent row.
    nested: bool,
    /// One bitfield per message holding the only message state a fold can
    /// depend on (see [`fold_flags`]). A streaming delta changes a message's
    /// text but not these bits, which is what lets the hot path skip the
    /// layout pass instead of recomputing it per character.
    fold_bits: Vec<u8>,
    on_change: Option<Box<dyn FnMut()>>,
    message_line_ranges: Vec<(usize, i64)>,
    /// The actionable rows each message draws, by message index: for each, its
    /// offset inside the message's own rows, the action, and the label's column
    /// span. One message can draw two (a settled reply whose head is a lone
    /// reasoning block keeps both its summary row and its `▸ thinking` marker).
    /// Derived rather than stored per rendered line so that the incremental
    /// paths (`rerender_message`, `append_last_message`) keep it correct for
    /// free: rows shift, offsets do not.
    row_meta: HashMap<usize, Vec<(usize, RowAction, usize, usize)>>,
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
            flat_cache: None,
            auto_scroll: true,
            width: max_width,
            // Every row starts closed, like the desktop: a streamed reply
            // would otherwise spend the screen on its own thinking and push
            // the answer it just wrote off the bottom.
            revealed: std::collections::HashMap::new(),
            last_render_width: -1,
            dirty: false,
            pending_rerender: BTreeSet::new(),
            flushing: false,
            full_renders: 0,
            stream_caches: std::collections::HashMap::new(),
            md,
            md_thinking,
            theme,
            compact_activity: false,
            reply_detail: ReplyDetail::Default,
            fold_words: FoldWords::default(),
            run_in_flight: false,
            compact_layout: Vec::new(),
            reply_folds: HashMap::new(),
            opened_trees: HashMap::new(),
            nested: false,
            fold_bits: Vec::new(),
            on_change: None,
            message_line_ranges: Vec::new(),
            row_meta: HashMap::new(),
        }
    }

    /// An empty transcript that renders exactly like this one.
    ///
    /// A session switch swaps whole [`ChatArea`]s (the outgoing conversation's
    /// rows are kept for the way back — see `Live::begin_switch`), so the
    /// incoming side has to start empty *without* losing the settings the
    /// transcript does not share with the conversation: the theme, the
    /// reading level (`/reply-detail`) and `ctrl+d`'s folding belong to the
    /// reader, not to the conversation they are leaving.
    pub fn empty_like(&self) -> Self {
        let mut fresh = Self::new(self.width.max(1), Some(self.theme));
        fresh.reply_detail = self.reply_detail;
        fresh.fold_words = self.fold_words;
        fresh.compact_activity = self.compact_activity;
        fresh.viewport_height = self.viewport_height;
        fresh
    }

    /// The words the process rows are drawn in (the interface language).
    ///
    /// Set from the workbench whenever the language changes; a transcript that
    /// never receives them draws the English defaults.
    pub fn set_fold_words(&mut self, words: FoldWords) {
        if self.fold_words == words {
            return;
        }
        self.fold_words = words;
        self.rerender();
    }

    pub fn fold_words(&self) -> FoldWords {
        self.fold_words
    }

    /// A run has started writing the transcript's last reply: keep every
    /// thought, call and piece of prose on screen until it ends, instead of
    /// folding the reply each time a slice stops growing.
    ///
    /// Idempotent — the workbench calls it from `agent_start` and again on the
    /// first event of a run it attached to mid-flight.
    pub fn begin_reply_run(&mut self) {
        if self.run_in_flight {
            return;
        }
        self.run_in_flight = true;
        self.replan();
    }

    /// The run is over: the reply it wrote is settled, and the reading level
    /// may fold it onto its answer (see [`ReplyDetail::Default`]).
    ///
    /// This is the one moment a reply collapses — the desktop's rule.
    pub fn end_reply_run(&mut self) {
        if !self.run_in_flight {
            return;
        }
        self.run_in_flight = false;
        self.replan();
    }

    pub fn run_in_flight(&self) -> bool {
        self.run_in_flight
    }

    /// Recompute the fold layout and rebuild the transcript when it moved.
    ///
    /// For a state change no message carries (a run starting or ending, which
    /// flips `run_in_flight` rather than a message flag, so `sync_compact_layout`
    /// cannot see it).
    fn replan(&mut self) {
        if self.last_render_width == -1 {
            self.dirty = true;
            return;
        }
        self.rerender();
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

    /// The reading level of a settled reply (`/reply-detail`).
    pub fn reply_detail(&self) -> ReplyDetail {
        self.reply_detail
    }

    /// Set the reading level (`/reply-detail`, the only door: it has no chord).
    ///
    /// One level for the whole transcript: every reply re-reads at once, and
    /// no row's open state is discarded by the flip — a burst the reader
    /// opened stays open whichever level draws it.
    pub fn set_reply_detail(&mut self, detail: ReplyDetail) {
        if self.reply_detail == detail {
            return;
        }
        self.reply_detail = detail;
        self.rerender();
    }

    /// Flip the reading level (`/reply-detail`); returns the new one. The
    /// transcript's height changes by however much was folded, so the view
    /// jumps to the bottom like the other view toggles. The cycle runs
    /// `default ↔ detailed` — the desktop's two levels, and nothing between
    /// them (see [`ReplyDetail::next`]).
    pub fn toggle_reply_detail(&mut self) -> ReplyDetail {
        let next = self.reply_detail.next();
        self.set_reply_detail(next);
        self.set_auto_scroll(true);
        next
    }

    /// Fold an uninterrupted run of *completed* calls to the same tool?
    fn folds_tool_runs(&self) -> bool {
        self.compact_activity
    }

    pub fn last_message(&self) -> Option<&ChatMessage> {
        self.messages.last()
    }

    /// The pictures the transcript carries, in the order it carries them.
    ///
    /// The preview box steps through these when the reader opened one of them:
    /// a session's pictures are its own, and a step from one should not land in
    /// another conversation's draft.
    pub fn pictures(&self) -> impl Iterator<Item = &ChatPicture> {
        self.messages
            .iter()
            .flat_map(|message| message.pictures.iter())
    }

    /// The text a message carries, for its copy control: the message covering
    /// `row`, trailing whitespace trimmed. `None` when the row covers no message
    /// or the message has no prose of its own (a reply that only called tools,
    /// a prompt that was only pictures) — the chip is not drawn there either.
    pub fn copyable_text_at(&self, row: usize) -> Option<String> {
        let index = self.message_at_row(row)?;
        let message = self.messages.get(index)?;
        let text = message.content.trim_end();
        (!text.trim().is_empty()).then(|| text.to_string())
    }

    /// Which rendered rows are the per-message controls (`[复制]`, `[分支]`).
    ///
    /// The rows are derived from the same stops the walk uses, so a frame that
    /// keeps its controls keeps them in one place — the frame tests read a
    /// transcript's *own* rows by taking these out, and a test that guessed at
    /// the chips' spelling would break the moment the language changed.
    pub fn control_rows(&self) -> Vec<usize> {
        let mut rows: Vec<usize> = self
            .row_stops()
            .into_iter()
            .filter(|stop| matches!(stop.action, RowAction::Copy { .. } | RowAction::Fork { .. }))
            .map(|stop| stop.row)
            .collect();
        // A reply's two chips share one row, and a row is one row: the list
        // names each of them once (`row_stops` is sorted by row).
        rows.dedup();
        rows
    }

    /// The journal entry of the prompt behind the message covering `row` — the
    /// point `fork` branches at.
    ///
    /// A reply is not forkable on its own: the fork means "branch where this
    /// question was asked", so the walk goes back to the nearest user message
    /// rather than to the reply itself (the desktop's fork button resolves its
    /// target the same way). `None` when there is no such prompt above the row,
    /// or when its entry is not written yet — a submission that was
    /// acknowledged but never persisted, or a row from an older agent that
    /// sends no `user_message` announcement.
    pub fn preceding_user_entry(&self, row: usize) -> Option<String> {
        let index = self.message_at_row(row)?;
        self.messages[..=index]
            .iter()
            .rev()
            .find(|msg| msg.role == ChatRole::User && !msg.welcome)
            .and_then(|msg| msg.entry_id.clone())
    }

    /// Learn the journal entry a prompt was written to, after the fact.
    ///
    /// The agent announces it on its `user_message` event, which can arrive on
    /// either side of the acknowledgement that binds the bubble to its run — and
    /// a prompt the agent accepted straight away is never bound by id at all
    /// (`bind_user_run` is the *queued* road). So the match tries, in order: the
    /// run id (the queued case), then the newest prompt with no entry whose text
    /// is the one announced. `text` is the announcement's display text; an
    /// announcement that matches nothing is dropped rather than guessed at —
    /// binding the wrong entry would fork the wrong place, and a missed one only
    /// means the fork chip says it cannot yet.
    pub fn bind_user_entry(&mut self, run_id: &str, entry_id: &str, text: &str) {
        if entry_id.is_empty() {
            return;
        }
        if !run_id.is_empty() {
            if let Some(msg) = self.messages.iter_mut().find(|msg| {
                msg.role == ChatRole::User
                    && (msg.run_id.as_deref() == Some(run_id) || msg.id == run_id)
            }) {
                msg.entry_id = Some(entry_id.to_string());
                return;
            }
        }
        let text = text.trim();
        if let Some(msg) = self
            .messages
            .iter_mut()
            .rev()
            .find(|msg| msg.role == ChatRole::User && msg.entry_id.is_none())
        {
            if text.is_empty() || msg.content.trim() == text {
                msg.entry_id = Some(entry_id.to_string());
            }
        }
    }

    /// The content of the most recent assistant message (thinking excluded) —
    /// what `/copy` puts on the clipboard. `None` when the transcript has no
    /// assistant text yet.
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
        let was_queued = self.messages[index].run_state == Some(RunState::Queued);
        self.messages[index].id = run_id.to_string();
        self.messages[index].run_id = Some(run_id.to_string());
        self.messages[index].run_state = Some(run_state);
        self.messages[index].queue_position = queue_position;
        // A queued prompt is not in the transcript at all (see
        // [`Self::waits_in_queue`]), so entering or leaving that state changes
        // whether the message has rows *and* whether the separator above it
        // belongs there: rebuild instead of splicing one message's lines.
        if was_queued || run_state == RunState::Queued {
            self.rerender();
        } else {
            self.rerender_message(index);
        }
    }

    pub fn update_run_state(&mut self, run_id: &str, run_state: RunState) {
        let Some(index) = self
            .messages
            .iter()
            .position(|m| m.run_id.as_deref() == Some(run_id))
        else {
            return;
        };
        let was_queued = self.messages[index].run_state == Some(RunState::Queued);
        self.messages[index].run_state = Some(run_state);
        if was_queued || run_state == RunState::Queued {
            self.rerender();
        } else {
            self.rerender_message(index);
        }
    }

    /// Forget the submission bound to `run_id` — a queued prompt that was
    /// cancelled: no run of its own happened, so it does not belong in the
    /// transcript.
    pub fn drop_run(&mut self, run_id: &str) {
        let before = self.messages.len();
        self.messages
            .retain(|message| message.run_id.as_deref() != Some(run_id));
        if self.messages.len() != before {
            self.rerender();
        }
    }

    /// Fold a submission's text into the card of the run that absorbed it, and
    /// forget the folded card.
    ///
    /// The agent folds the prompts queued directly behind a run at the turn
    /// boundary (`enqueue_coalescing`): their texts are appended to that run's
    /// message in queue order, and each of them comes back as a terminal
    /// `merged` acknowledgement, never as a run of its own. The card ahead is
    /// therefore the one that has to grow — dropping the folded card alone would
    /// leave the transcript showing a message shorter than the one the agent was
    /// asked. Returns whether there was a card of this client's to grow: a fold
    /// that happened inside another client's submission leaves nothing here to
    /// change.
    pub fn fold_run_into_previous(&mut self, folded_run: &str) -> bool {
        let Some(index) = self
            .messages
            .iter()
            .position(|message| message.run_id.as_deref() == Some(folded_run))
        else {
            return false;
        };
        let folded = self.messages[index].content.clone();
        // The agent folds the images too (`payload.images.extend`), so the card
        // ahead keeps showing them.
        let pictures = self.messages[index].pictures.clone();
        // The card ahead is the nearest earlier submission bound to a run, and
        // not itself folded away: the agent's fold is a run of consecutive
        // requests, so the one before this card is where the text went.
        let ahead = (0..index).rev().find(|at| {
            self.messages[*at].run_id.is_some()
                && self.messages[*at].run_state != Some(RunState::Merged)
        });
        self.messages.remove(index);
        let grew = ahead.map(|at| {
            let target = &mut self.messages[at];
            if !target.content.is_empty() && !folded.is_empty() {
                target.content.push_str("\n\n");
            }
            target.content.push_str(&folded);
            target.pictures.extend(pictures);
        });
        self.rerender();
        grew.is_some()
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
            self.rerender();
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

    /// Mark the newest message — the one a submission just appended — after an
    /// acknowledgement that never arrived.
    pub fn mark_last_message_run_state(&mut self, run_state: RunState) {
        let Some(index) = self.messages.len().checked_sub(1) else {
            return;
        };
        self.messages[index].run_state = Some(run_state);
        self.rerender_message(index);
    }

    pub fn set_on_change(&mut self, cb: impl FnMut() + 'static) {
        self.on_change = Some(Box::new(cb));
    }

    pub fn update_last_message(&mut self, content: &str) {
        // The run's final text belongs below the activity that produced it. A
        // tool result or an approval decision row is a *later* message than the
        // assistant message that started the turn, so writing the text back
        // into `find_assistant_index()` would paint the answer above those
        // rows — the manual-approval case ("Allowed: …" is a `System` row
        // pushed between the tool calls and the final answer).
        if self.starts_new_assistant_segment() {
            if !content.is_empty() {
                self.add_message(ChatMessage::new(new_id(), ChatRole::Assistant, content));
            }
            return;
        }
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
        // Text that follows this turn's activity starts a fresh assistant
        // message, so it renders after the tool/approval rows instead of being
        // appended to the turn's opening assistant message above them.
        if self.starts_new_assistant_segment() {
            self.settle_tail();
            self.add_message(ChatMessage::new(new_id(), ChatRole::Assistant, ""));
        }
        if let Some(idx) = self.find_assistant_index() {
            self.messages[idx].content.push_str(delta);
            self.messages[idx].pending = true;
            self.mark_message_dirty(idx);
        }
    }

    /// Whether the transcript's tail is activity rows this turn owns (`Tool`
    /// result, `System` notice such as an approval decision) rather than the
    /// assistant message that the next text continues.
    ///
    /// A trailing `User` message is deliberately *not* activity: it is a prompt
    /// queued for the next turn (`enqueue_if_busy`), and the running turn's
    /// text still belongs to the assistant message above it.
    /// The tail message is no longer growing: the run has moved on to another
    /// message, so a reply that ends there is settled. Without this a message
    /// that only carried reasoning stayed `pending` for the rest of the run
    /// (nothing ever came back to finish it), which kept its reply out of the
    /// reading-level fold and left `thinking…` showing next to finished work.
    fn settle_tail(&mut self) {
        let Some(last) = self.messages.last_mut() else {
            return;
        };
        if last.role == ChatRole::Assistant && last.pending {
            last.pending = false;
        }
    }

    fn starts_new_assistant_segment(&self) -> bool {
        self.messages
            .last()
            .is_some_and(|m| matches!(m.role, ChatRole::Tool | ChatRole::System))
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
        // The assistant message that requested this call is finished: the
        // call is the run's next slice, not more of that message. Without
        // this it stays `pending` for the rest of the run (nothing ever comes
        // back to finish it), keeping its reply out of the reading-level fold
        // and leaving `thinking…` beside work that has already stopped.
        self.settle_tail();
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
    /// one `✗` on its row — the same way a replayed one is.
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
        // Subsequent thinking blocks in the same turn are concatenated directly
        // into one thinking section — but only while that slice has written no
        // prose. A thought that begins after the model has already written an
        // answer is its own slice: it happened *after* that prose, and drawing
        // it on the same message would put the `▸ thinking` marker above text
        // it followed. The desktop gets this for free (reasoning is its own
        // segment); here it is one message per slice, so the slice is new.
        let joins_last = self.messages[last_idx].role == ChatRole::Assistant
            && self.messages[last_idx].content.trim().is_empty();
        if joins_last {
            let last = &mut self.messages[last_idx];
            if last.thinking.is_none() {
                last.thinking = Some(String::new());
            }
            last.pending = true;
            self.rerender_message(last_idx);
        } else {
            // The slice that wrote prose before this thought has stopped
            // growing: leaving it `pending` would keep the whole reply reading
            // as "still running" for the rest of the turn.
            self.settle_tail();
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
        if self.messages[idx].thinking.is_none() {
            return;
        }
        // The block finished: it is not growing any more, and the answer that
        // follows is a new slice of the same reply. A slice that has already
        // written prose stays pending — clearing the flag there would read as
        // "this reply settled" while the run is still working, and the reading
        // level would fold the prose away until the next delta put it back.
        if self.messages[idx].content.trim().is_empty() {
            let changed = self.messages[idx].pending;
            self.messages[idx].pending = false;
            if changed {
                self.rerender_message(idx);
            } else {
                self.mark_message_dirty(idx);
            }
        } else {
            self.mark_message_dirty(idx);
        }
    }

    // ─── Opening a row (tab + enter, and a click) ───────────────────────

    /// The rows `tab` stops on: every row that opens something, in reading
    /// order — a settled reply's process, a burst of work, a tool call, a
    /// reasoning marker.
    ///
    /// Derived from the per-message offsets the render wrote (`row_meta`) and
    /// the message ranges it wrote them into, so the list is always consistent
    /// with the lines on screen even after an incremental re-render.
    pub fn row_stops(&self) -> Vec<RowStop> {
        let mut stops: Vec<RowStop> = Vec::new();
        for (msg, rows) in &self.row_meta {
            let Some((from, to)) = self.message_line_ranges.get(*msg).copied() else {
                continue;
            };
            if to < from as i64 {
                continue;
            }
            for (offset, action, start, end) in rows {
                // The reveal flag lives on the message the action names: a
                // fold's header hangs off the first row of its tree, but it
                // opens and closes the reply it stands for.
                let open = self
                    .messages
                    .get(action.message())
                    .and_then(|msg| self.revealed.get(msg.id.as_str()))
                    .copied()
                    .unwrap_or_default();
                stops.push(RowStop {
                    row: from + offset,
                    start: *start,
                    end: *end,
                    // A control (copy, fork) has no open/closed state, so it is
                    // never "expanded" — it is still a stop the cursor lands on.
                    expanded: action.kind().is_some_and(|kind| open.get(kind)),
                    action: action.clone(),
                });
            }
        }
        stops.sort_by_key(|stop| stop.row);
        stops
    }

    /// Open or close the row drawn at rendered row `row`; returns the row the
    /// cursor should land on afterwards (`None` when that row draws nothing
    /// actionable any more).
    ///
    /// Opening is per row, and the state is kept by message id, so a row that
    /// moves (a reply above it grew) or a level flip does not lose it. The
    /// returned row is where the *same* row went: opening a fold moves its
    /// header to the top of the work it stands for (and closing it moves the
    /// header back), and a reader who pressed `enter` expects the cursor to
    /// follow what they pressed rather than stay on a line that now says
    /// something else.
    pub fn toggle_row(&mut self, row: usize) -> Option<usize> {
        let msg = self.message_at_row(row)?;
        let (from, _) = self.message_line_ranges[msg];
        let action = self.row_meta.get(&msg).and_then(|rows| {
            rows.iter()
                .find(|(offset, _, _, _)| from + offset == row)
                .map(|(_, action, _, _)| action.clone())
        })?;
        let id = self
            .messages
            .get(action.message())
            .map(|msg| msg.id.clone())?;
        // A control is not a fold: there is no state to toggle, and the buttons
        // are answered by the driver (`WbAction::CopyMessage`/`ForkMessage`).
        let kind = action.kind()?;
        self.revealed.entry(id).or_default().toggle(kind);
        // Opening draws what the row stood for *below* it and closing takes it
        // away again, so every row above the pressed one is exactly where it
        // was: the viewport (a line index from the top) still names the same
        // rows and is left alone. This used to follow the tail, which threw the
        // reader's place away and left the rows they just opened off screen.
        self.rerender();
        let stops = self.row_stops();
        stops
            .iter()
            .find(|stop| stop.action == action)
            .or_else(|| stops.iter().find(|stop| stop.row >= row))
            .map(|stop| stop.row)
    }

    /// The message whose rendered rows cover `row`.
    pub fn message_at_row(&self, row: usize) -> Option<usize> {
        self.message_line_ranges
            .iter()
            .position(|(from, to)| *from <= row && (row as i64) <= *to)
    }

    /// Is the row `kind` of message `id` open?
    fn is_open(&self, id: &str, kind: RowKind) -> bool {
        self.revealed.get(id).is_some_and(|rows| rows.get(kind))
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
        self.render_all_shared(width).as_ref().to_vec()
    }

    /// The whole transcript as a shared snapshot. [`Self::render_all`] owns a
    /// copy for callers that mutate the rows; the workbench borrows the snapshot
    /// and must not pay that copy once per `WorkbenchData`.
    pub fn render_all_shared(&mut self, width: usize) -> Arc<[String]> {
        if width as i64 != self.last_render_width || self.dirty {
            self.last_render_width = width as i64;
            self.width = width;
            self.dirty = false;
            self.rerender();
        }
        self.flush_pending_rerenders();
        if self.flat_cache.is_none() {
            self.flat_cache = Some(
                self.rendered_lines
                    .iter()
                    .map(|line| line.text.clone())
                    .collect::<Vec<_>>()
                    .into(),
            );
        }
        self.flat_cache
            .as_ref()
            .cloned()
            .unwrap_or_else(|| Arc::from(Vec::new()))
    }

    /// How many times this area has rebuilt the whole transcript — the count
    /// [`Self::rerender`] pays when the width changes or a message is
    /// re-rendered wholesale, and the meter the driver loop's contract is
    /// measured with.
    ///
    /// That contract: a frame is composed at one width, and asking for another
    /// (the loop used to build a whole frame at width `1` just to read the
    /// activity kind) misses the per-width cache and re-wraps *every* message,
    /// which is what made a long session slow under a streamed reply. A caller
    /// that keeps one width sees this climb once per real change, not once per
    /// frame.
    pub fn full_renders(&self) -> u64 {
        self.full_renders
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

    /// The reading-level fold for settled replies.
    ///
    /// A **settled** reply keeps the answer it ended on: the calls, the
    /// reasoning and the notes it wrote between them fold into one summary row
    /// above that answer. A **running** reply never folds: every thought, call
    /// and piece of prose keeps its row until the run ends. Folding a finished
    /// burst while the next slice was arriving made rows disappear and reappear,
    /// and changed the viewport height throughout the run.
    ///
    /// Whether a reply is still running is the *run's* answer, not its
    /// messages': `agent_start` opens a run and `agent_end` closes it, and in
    /// between a slice routinely stops growing while the model thinks about the
    /// next step. Reading "settled" off the messages made the whole reply
    /// collapse onto its answer between two slices and reopen on the next event.
    ///
    /// A reply is the stretch of messages after a user or system row: the same
    /// unit the desktop and the phone call a reply, drawn from the transcript
    /// this client builds (one message per assistant/tool slice). Every reply is
    /// decided on its own, so an older turn stays folded while the newest one
    /// streams.
    ///
    /// `fold_settled` is what separates the two levels: [`ReplyDetail::Default`]
    /// passes `true` (a finished reply becomes its answer plus one count row),
    /// [`ReplyDetail::Detailed`] passes `false` (a finished reply keeps every
    /// slice). Neither level folds a reply that is still running.
    fn reply_fold_layout(
        messages: &[ChatMessage],
        run_in_flight: bool,
        fold_settled: bool,
        revealed: &std::collections::HashMap<String, OpenRows>,
    ) -> FoldPlan {
        let mut plan = FoldPlan {
            heads: (0..messages.len()).collect(),
            ..FoldPlan::default()
        };
        let live = if run_in_flight {
            Self::live_segment_start(messages)
        } else {
            None
        };
        let mut start = 0usize;
        let mut cursor = 0usize;
        loop {
            let boundary = cursor == messages.len()
                || matches!(messages[cursor].role, ChatRole::User | ChatRole::System);
            if boundary {
                Self::fold_reply_segment(
                    messages,
                    start,
                    cursor,
                    live == Some(start),
                    fold_settled,
                    revealed,
                    &mut plan,
                );
                start = cursor + 1;
            }
            if cursor == messages.len() {
                break;
            }
            cursor += 1;
        }
        plan
    }

    /// The reply a live run is writing: the start of the last segment that
    /// holds an assistant or tool row.
    ///
    /// The *last* segment is not enough. A prompt queued mid-run (`ctrl+enter`)
    /// lands as a user row below the reply being written, and the reply above it
    /// is still the one streaming.
    fn live_segment_start(messages: &[ChatMessage]) -> Option<usize> {
        let mut start = 0usize;
        let mut live = None;
        for (index, msg) in messages.iter().enumerate() {
            match msg.role {
                ChatRole::User | ChatRole::System => start = index + 1,
                ChatRole::Assistant | ChatRole::Tool => live = Some(start),
            }
        }
        live
    }

    /// Fold one settled reply (`start..end`, exclusive) above its answer.
    ///
    /// A reply still running keeps every row on screen. Once the reply settles
    /// the whole reply folds — the work that produced the
    /// answer *and* whatever ran after it. Before this, only the stretch above
    /// the answer folded, so a reply whose last prose came before its last few
    /// calls (an interrupted run, a turn that ended on a tool call) kept those
    /// rows: the folded level then read as "only part of it folded", and
    /// scrolling up the unfolded calls were the first thing in sight. A settled
    /// reply is one
    /// unit, and its summary row counts all of the work.
    ///
    /// `live` says the run is still writing *this* reply (see
    /// [`Self::live_segment_start`]) and `fold_settled` picks what a finished
    /// reply does.
    fn fold_reply_segment(
        messages: &[ChatMessage],
        start: usize,
        end: usize,
        live: bool,
        fold_settled: bool,
        revealed: &std::collections::HashMap<String, OpenRows>,
        plan: &mut FoldPlan,
    ) {
        if start >= end {
            return;
        }
        let running = live
            || messages[start..end]
                .iter()
                .any(|msg| msg.pending || msg.tool_status == Some(ToolStatus::Running));
        if running {
            // Streaming rows are allowed to grow or update in place, never to
            // disappear into an automatic fold. `agent_end` is the one moment
            // the default reading level may collapse the reply.
            return;
        }
        if !fold_settled {
            // `detailed`: every slice keeps its own row. Nothing is folded, so
            // there is no summary row to open; the calls and thoughts are
            // already on screen, each opening its own detail.
            return;
        }
        // The slice the summary rides on is the one the reader keeps in sight:
        // the reply's last prose, else its newest slice (a turn that never wrote
        // an answer) — a slice that draws a row, so an assistant placeholder
        // that is still empty never becomes the only thing left on screen.
        let head = (start..end)
            .rev()
            .find(|&i| {
                messages[i].role == ChatRole::Assistant && !messages[i].content.trim().is_empty()
            })
            .or_else(|| {
                (start..end)
                    .rev()
                    .find(|&i| messages[i].role == ChatRole::Tool || has_reasoning(&messages[i]))
            })
            .unwrap_or(end - 1);
        let mut summary = FoldSummary::default();
        let mut folded: Vec<usize> = Vec::new();
        for (offset, msg) in messages[start..end].iter().enumerate() {
            let index = start + offset;
            if index == head {
                continue;
            }
            match msg.role {
                ChatRole::Tool => summary.tools += 1,
                ChatRole::Assistant if has_reasoning(msg) => summary.thoughts += 1,
                _ => {}
            }
            folded.push(index);
        }
        // The head's own reasoning counts too when the row below it is prose —
        // a reasoning-only slice keeps its marker instead and is not counted
        // (see `reasoning_only`).
        if has_reasoning(&messages[head]) && !reasoning_only(&messages[head]) {
            summary.thoughts += 1;
        }
        if summary.tools == 0 && summary.thoughts == 0 {
            // The reply is already just its output: nothing to stand in for.
            return;
        }
        // The reader opened this reply's process row: it replays as a tree
        // under a header that folds it away again (see [`Self::open_tree`]).
        if revealed
            .get(messages[head].id.as_str())
            .is_some_and(|open| open.get(RowKind::Process))
        {
            Self::open_tree(
                &mut plan.trees,
                messages,
                start,
                end,
                head,
                RowKind::Process,
                summary,
            );
            return;
        }
        for index in folded {
            plan.heads[index] = head;
        }
        plan.folds.insert(head, ReplyFold { summary });
    }

    /// Lay a fold the reader opened out as a tree.
    ///
    /// Every row the fold stood for gets its own row back, hung under one header
    /// that counts them and folds them away again — an opened fold with no
    /// header is a state with no way out, because the summary row used to *turn
    /// into* the rows it hid.
    ///
    /// `head` is the slice the fold rode on. An **answer** stays outside the
    /// tree, at the transcript's own column: it is the reply's output rather
    /// than part of its process, and the answer's own reasoning then reads as
    /// its `▸ thinking` marker again, so the header stops counting it. A reply
    /// that never wrote prose has no such row — its head is just the newest
    /// slice, which joins the tree like every other one.
    fn open_tree(
        trees: &mut HashMap<usize, OpenedTree>,
        messages: &[ChatMessage],
        start: usize,
        end: usize,
        head: usize,
        kind: RowKind,
        summary: FoldSummary,
    ) {
        let answer_outside =
            messages[head].role == ChatRole::Assistant && !messages[head].content.trim().is_empty();
        let mut banner = summary;
        if answer_outside && has_reasoning(&messages[head]) && !reasoning_only(&messages[head]) {
            // The answer's reasoning is a marker of its own again (it is outside
            // the tree), so it is no longer one of the header's counts.
            banner.thoughts = banner.thoughts.saturating_sub(1);
        }
        let mut first = true;
        for index in start..end {
            if index == head && answer_outside {
                continue;
            }
            trees.insert(
                index,
                OpenedTree {
                    head,
                    kind,
                    banner: if first { Some(banner) } else { None },
                },
            );
            first = false;
        }
    }

    /// The layout in effect for the rendered lines: the reading-level fold,
    /// with the compact view's runs folded on top of whatever it left standing.
    fn layout(&self) -> FoldPlan {
        // `default` folds a settled reply onto the answer it ended on;
        // `detailed` keeps every slice where it happened. A reply that is still
        // running keeps every row at either level, and neither level shows a
        // body or reasoning text — opened rows are the reader's.
        let fold_settled = self.reply_detail == ReplyDetail::Default;
        let mut plan = Self::reply_fold_layout(
            &self.messages,
            self.run_in_flight,
            fold_settled,
            &self.revealed,
        );
        if self.compact_activity {
            let mut run: Option<(FoldKind, usize)> = None;
            for (i, msg) in self.messages.iter().enumerate() {
                // A message the reading level already folded is out of play:
                // its rows do not exist, and a run never spans across one.
                if plan.heads[i] != i {
                    run = None;
                    continue;
                }
                // …and a message a reply's summary rides on is not free to
                // join a run either: folding it into a later row would take
                // the summary off screen with it.
                if plan.folds.contains_key(&i) {
                    run = None;
                    continue;
                }
                // A row hanging under an opened fold's header is out of play
                // for the same reason: a compact run would swallow the tree's
                // rows into a row of its own, outside the header that counts
                // them.
                if plan.trees.contains_key(&i) {
                    run = None;
                    continue;
                }
                let Some(kind) = fold_kind(msg, self.folds_tool_runs()) else {
                    run = None;
                    continue;
                };
                match run {
                    Some((ref run_kind, head)) if *run_kind == kind => plan.heads[i] = head,
                    _ => run = Some((kind, i)),
                }
            }
        }
        plan
    }

    /// The message state a fold depends on, as one bitfield: whether the reply
    /// it belongs to is still running, whether it has prose, whether it has
    /// reasoning, and its tool status. Everything else a message holds — its
    /// text especially — cannot change where a fold boundary sits, so a
    /// content delta leaves these equal and the layout pass is skipped.
    fn fold_flags(messages: &[ChatMessage]) -> Vec<u8> {
        messages
            .iter()
            .map(|msg| {
                let status = match msg.tool_status {
                    None => 0u8,
                    Some(ToolStatus::Running) => 1,
                    Some(ToolStatus::Complete) => 2,
                    Some(ToolStatus::Error) => 3,
                };
                (msg.pending as u8)
                    | ((!msg.content.trim().is_empty() as u8) << 1)
                    | ((msg
                        .thinking
                        .as_deref()
                        .is_some_and(|text| !text.trim().is_empty()) as u8)
                        << 2)
                    | (status << 3)
            })
            .collect()
    }

    /// Is `msg_idx` folded into an earlier message (and therefore rendering no
    /// rows of its own)?
    fn is_folded(&self, msg_idx: usize) -> bool {
        self.compact_layout
            .get(msg_idx)
            .is_some_and(|head| *head != msg_idx)
    }

    /// The submissions still waiting in the agent's queue, in transcript order.
    ///
    /// The queue itself is the agent's (`get_state`'s `queuedRuns`); this is
    /// the subset *this* client submitted, which is the only part it holds a
    /// card for — see `workbench::live::reconcile_queue`.
    pub fn queued_run_ids(&self) -> Vec<String> {
        self.messages
            .iter()
            .filter(|message| message.run_state == Some(RunState::Queued))
            .filter_map(|message| message.run_id.clone())
            .collect()
    }

    /// Is `msg_idx` a submission still waiting in the agent's queue? Such a
    /// prompt has not been sent yet, so it draws no rows here: the frame lists
    /// it above the composer instead (one row per prompt, `workbench::screen`),
    /// which is what keeps "queued" from looking like "sent".
    fn waits_in_queue(&self, msg_idx: usize) -> bool {
        self.messages
            .get(msg_idx)
            .is_some_and(|msg| msg.run_state == Some(RunState::Queued))
    }

    /// Recompute the layout and, when the rendered lines no longer match it,
    /// rebuild the transcript. Returns `true` when the caller must not assume
    /// `msg_idx` rendered on its own.
    ///
    /// Growth that adds *standalone* messages (the common case: a user turn, an
    /// assistant reply, the first call of a run) leaves every existing message's
    /// rows untouched, so the incremental paths stay in charge. A fold appearing
    /// changes an earlier message's row (a run's count) — and, when it dissolves
    /// again, brings rows back — so the transcript is rebuilt: correctness over
    /// a splice that would have to reason about both sides of the fold.
    fn sync_compact_layout(&mut self) -> bool {
        // `default` folds settled replies, so there is no "nothing folds" fast
        // path; the bit comparison below is what keeps the common case cheap.
        // A fold can only appear or dissolve when one of these bits moved, so
        // the common case (a delta arriving in the reply being streamed) costs
        // one linear scan of cheap comparisons instead of a layout rebuild.
        let bits = Self::fold_flags(&self.messages);
        if bits == self.fold_bits
            && self.compact_layout.len() == self.messages.len()
            && self
                .reply_folds
                .keys()
                .all(|idx| *idx < self.messages.len())
            && self
                .opened_trees
                .keys()
                .all(|idx| *idx < self.messages.len())
        {
            return false;
        }
        let plan = self.layout();
        self.fold_bits = bits;
        // A fold's *counts* can move while its structure stands: a prose slice's
        // reasoning joins the counts above it, and the row that counts them is
        // drawn from them (`push_reply_summary`). `heads` alone would say
        // "nothing moved" and leave the old numbers, and a marker the counts
        // just took over would stay on screen. Those are earlier messages' rows
        // too, so the transcript is rebuilt rather than spliced.
        if fold_layout_extends(&self.compact_layout, &plan.heads)
            && plan.folds == self.reply_folds
            && plan.trees == self.opened_trees
        {
            self.compact_layout = plan.heads;
            self.reply_folds = plan.folds;
            self.opened_trees = plan.trees;
            return false;
        }
        self.rerender();
        true
    }

    fn rerender(&mut self) {
        self.pending_rerender.clear();
        self.stream_caches.clear();
        self.flat_cache = None;
        // Defer until first render() has set the correct terminal width.
        if self.last_render_width == -1 {
            self.dirty = true;
            return;
        }
        self.dirty = false;
        self.full_renders = self.full_renders.saturating_add(1);

        let plan = self.layout();
        self.compact_layout = plan.heads;
        self.reply_folds = plan.folds;
        self.opened_trees = plan.trees;
        self.fold_bits = Self::fold_flags(&self.messages);
        self.rendered_lines = Vec::new();
        self.message_line_ranges = Vec::new();
        // A folded message renders nothing and takes no separator with it, so
        // the transcript stays as tight as the summary row implies.
        let mut rendered_any = false;
        let mut above: Option<usize> = None;
        for i in 0..self.messages.len() {
            if self.is_folded(i) || self.waits_in_queue(i) {
                let at = self.rendered_lines.len();
                self.message_line_ranges.push((at, at as i64 - 1));
                continue;
            }
            if rendered_any {
                if let Some(above) = above {
                    self.push_message_separator(i, above);
                }
            }
            let start = self.rendered_lines.len();
            self.render_message(i);
            self.message_line_ranges
                .push((start, self.rendered_lines.len() as i64 - 1));
            rendered_any = true;
            above = Some(i);
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
        // A prompt waiting in the queue has no rows at all; the frame shows it
        // above the composer (the band in `workbench::screen`), so a state
        // change is what re-renders the transcript, not this splice.
        if self.waits_in_queue(msg_idx) {
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
        self.flat_cache = None;
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
        // A queued prompt draws no rows (it waits above the composer, not in
        // the transcript), so there is nothing to append — and the trailing
        // blank row the append consumes must stay where it is.
        if self.waits_in_queue(self.messages.len().saturating_sub(1)) {
            return;
        }
        self.rendered_lines.pop();
        if self.messages.len() > 1 {
            self.push_message_separator(self.messages.len() - 1, self.messages.len() - 2);
        }
        let start = self.rendered_lines.len();
        self.render_message(self.messages.len() - 1);
        self.message_line_ranges
            .push((start, self.rendered_lines.len() as i64 - 1));
        self.rendered_lines.push(RenderedLine {
            text: String::new(),
            dim: true,
        });
        self.flat_cache = None;
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

    fn find_tool_index(&self, tool_id: &str) -> Option<usize> {
        self.messages
            .iter()
            .rposition(|m| m.role == ChatRole::Tool && m.tool.as_deref() == Some(tool_id))
    }

    fn render_message(&mut self, msg_idx: usize) {
        // A folded run renders as one summary row, written by its first message
        // (the rest render nothing at all — see `compact_heads`).
        self.row_meta.remove(&msg_idx);
        // Where this message's own rows start: the actionable rows it draws are
        // remembered as offsets from here, so a splice that shifts them costs
        // nothing (see `row_stops`).
        let origin = self.rendered_lines.len();
        let run = self.compact_run(msg_idx);
        let fold = self.reply_folds.get(&msg_idx).copied();
        let tree = self.opened_trees.get(&msg_idx).copied();
        // An opened fold draws its header first — the row that folds it away
        // again — then a gap that carries the rail on down, and hangs every row
        // it stands for beneath that: the tree is a block of its own, not the
        // title's next line.
        if let Some(tree) = tree {
            if let Some(summary) = tree.banner {
                self.push_fold_header(msg_idx, summary, tree, origin);
                self.push_rail_row();
            }
        }
        // The rows below the header are laid out one gutter narrower and hung
        // under the rail once they are drawn: markdown wrapping, a call's band
        // and its body all have to fit inside the tree's column.
        let body = self.rendered_lines.len();
        let width = self.width;
        if tree.is_some() {
            self.width = width.saturating_sub(TREE_GUTTER).max(1);
        }
        // Rows drawn *inside* this one (a call's body, a thought's text) hang
        // one level deeper, with a rail of their own.
        self.nested = tree.is_some();
        let msg = self.messages[msg_idx].clone();
        match msg.role {
            ChatRole::User => self.render_user_message(&msg),
            ChatRole::Assistant => self.render_assistant_message(msg_idx, &msg, run, fold, origin),
            ChatRole::Tool if fold.is_some() => {
                let fold = fold.unwrap();
                self.push_reply_summary(msg_idx, fold.summary, origin);
                // The settled summary rides on a call the reader keeps in
                // sight, so that call still draws its own row.
                if run > 1 {
                    self.render_tool_run(msg_idx, run, origin);
                } else {
                    self.render_tool_message(msg_idx, &msg, origin);
                }
            }
            ChatRole::Tool if run > 1 => self.render_tool_run(msg_idx, run, origin),
            ChatRole::Tool => self.render_tool_message(msg_idx, &msg, origin),
            ChatRole::System => self.render_system_message(&msg),
        }
        // Everything but the user card draws its pictures after its own rows:
        // the card renders them inside itself (see `render_user_message`), so a
        // picture sits on the bubble rather than under it.
        if !matches!(msg.role, ChatRole::User) {
            self.render_pictures(&msg);
        }
        // The controls sit under everything the message drew — its pictures'
        // captions included, so the last thing in a message is not the only
        // thing the reader can press — and *outside* the user card, which is
        // what keeps `[复制]` a control rather than something the reader
        // typed.
        self.push_message_controls(msg_idx, &msg, origin);
        self.nested = false;
        self.width = width;
        if tree.is_some() {
            // The header is the tree's own row; everything after it hangs.
            self.hang_under_rail(body, msg_idx, body - origin);
        }
    }

    /// Where a row drawn *inside* another one goes, and how wide: one gutter in,
    /// with a rail of its own, when the row it belongs to is inside an opened
    /// fold (a call's body, a thought's text).
    ///
    /// A detail that sat at its parent's column read as another sibling of it;
    /// the second level is what makes the fold a tree rather than a flat list of
    /// rows that happen to be beside a rail.
    fn detail_place(&self) -> (usize, String) {
        if self.nested {
            (self.width.saturating_sub(TREE_GUTTER).max(1), self.rail())
        } else {
            (self.width, String::new())
        }
    }

    /// Look the rows just drawn hang under: the rail in column 1 of each, with
    /// the rows' own widths already laid out one gutter narrower.
    ///
    /// A blank row inside the block (a paragraph break in reasoning, a gap
    /// between two notes) carries the rail and nothing else. Leaving it empty
    /// cut the spine in two: the rail is what says these rows are one tree, and
    /// a break in the middle of it reads as the end of the fold. Only the
    /// block's trailing padding rows are dropped, because a rail dangling below
    /// the last row hangs over nothing.
    fn hang_under_rail(&mut self, from: usize, msg_idx: usize, skipped: usize) {
        let rail = self.rail();
        let spine = rail.trim_end().to_string();
        let end = self.rendered_lines[from..]
            .iter()
            .rposition(|row| !is_rail_only(&row.text))
            .map_or(from, |last| from + last + 1);
        self.rendered_lines.truncate(end);
        for row in &mut self.rendered_lines[from..] {
            row.text = if row.text.is_empty() {
                spine.clone()
            } else {
                with_gutter(&row.text, &rail)
            };
        }
        if let Some(rows) = self.row_meta.get_mut(&msg_idx) {
            for (offset, _, start, end) in rows.iter_mut() {
                if *offset < skipped {
                    continue;
                }
                *start += TREE_GUTTER;
                *end += TREE_GUTTER;
            }
        }
    }

    /// The gutter an opened fold's rows hang under, in the chrome's faint ink:
    /// a space and the rail, so the rail sits in column 2 — directly under the
    /// header's marker — while the row's own one-column inset still starts the
    /// text in the header label's column.
    ///
    /// The foreground is given back with `39` rather than a full reset: a reset
    /// would drop the background a call's band is painted on.
    fn rail(&self) -> String {
        format!(
            " \x1b[38;5;{}m{TREE_RAIL}\x1b[39m",
            Chrome::from_theme(&self.theme).muted
        )
    }

    /// The row that separates the message at `index` from the one above it: a
    /// blank, or the tree's rail when both hang under one opened fold's header —
    /// a blank row there would cut the tree in two.
    fn push_message_separator(&mut self, index: usize, above: usize) {
        let same_tree = match (self.opened_trees.get(&above), self.opened_trees.get(&index)) {
            (Some(above), Some(index)) => above.head == index.head,
            _ => false,
        };
        if same_tree {
            self.push_rail_row();
        } else {
            self.rendered_lines.push(RenderedLine {
                text: String::new(),
                dim: true,
            });
        }
    }

    /// A row that carries the tree's rail and nothing else: the spine keeps
    /// running where the transcript has a gap (between two rows of one fold, and
    /// under its header).
    fn push_rail_row(&mut self) {
        let text = self.rail().trim_end().to_string();
        self.rendered_lines.push(RenderedLine { text, dim: true });
    }

    /// Record one span of the row just pushed as a stop: `start`..`end` are
    /// display columns of that row, so a row carrying two controls has two
    /// targets that each answer their own column.
    fn mark_row_span(
        &mut self,
        msg_idx: usize,
        origin: usize,
        start: usize,
        end: usize,
        action: RowAction,
    ) {
        let Some(last) = self.rendered_lines.len().checked_sub(1) else {
            return;
        };
        let offset = last.saturating_sub(origin);
        self.row_meta
            .entry(msg_idx)
            .or_default()
            .push((offset, action, start, end));
    }

    /// The row of controls under a message: `[复制]`, and on a reply `[分支]`.
    ///
    /// Every client carries these under its messages — the desktop puts the
    /// user's copy in the bubble's footer and the reply's copy beside its fork
    /// control — but the terminal cannot copy the desktop's *reveal*: a control
    /// painted only while the pointer rests on a message does not exist on a
    /// keyboard surface. So the row is always there, one row per message, and
    /// it is the message's last one: `tab` reaches it after everything the
    /// message says, and after its pictures' captions.
    ///
    /// Two rules decide whether a chip is drawn at all:
    ///
    /// * **Copy** needs text. A reply that only called tools has nothing to
    ///   copy, and a button that copies nothing is a button that lies.
    /// * **Fork** belongs to a *settled* reply — it branches at the user
    ///   message that produced the reply, which is the desktop's own meaning of
    ///   its fork button (`Live::fork_message`). While the reply is still
    ///   arriving there is nothing to branch on yet, so the chip waits.
    ///
    /// Nothing is drawn for a welcome banner, for a prompt still waiting in the
    /// agent's queue (it has no rows at all, see [`Self::render_user_message`]),
    /// or for a row inside an opened fold — the tree is the fold's body, not a
    /// message the reader acts on.
    fn push_message_controls(&mut self, msg_idx: usize, msg: &ChatMessage, origin: usize) {
        if msg.welcome || msg.pending || msg.run_state == Some(RunState::Queued) || self.nested {
            return;
        }
        // Only the two roles a reader writes or reads get controls: a tool
        // call's row and a compaction divider are the reply's *work*, not
        // messages of their own, and a copy chip under every call would double
        // the height of a folded run.
        let (wants_copy, wants_fork) = match msg.role {
            ChatRole::User => (true, false),
            ChatRole::Assistant => (true, true),
            ChatRole::Tool | ChatRole::System => return,
        };
        let mut chips: Vec<(&'static str, RowAction)> = Vec::new();
        if wants_copy && !msg.content.trim().is_empty() {
            chips.push((self.fold_words.copy, RowAction::Copy { msg: msg_idx }));
        }
        if wants_fork {
            chips.push((self.fold_words.fork, RowAction::Fork { msg: msg_idx }));
        }
        if chips.is_empty() {
            return;
        }
        // `[复制]`: the brackets are this surface's spelling for something
        // pressable — a panel's `[× 关闭]`, the header's `[ctrl+r] 会话`.
        let labels: Vec<String> = chips.iter().map(|(word, _)| format!("[{word}]")).collect();
        // One space between chips, which is also what the spans' arithmetic
        // below assumes.
        let total = labels
            .iter()
            .map(|label| visible_width(label))
            .sum::<usize>()
            + labels.len().saturating_sub(1);
        // Nothing fits in a frame this narrow: the chips would spill past the
        // last column, and a control the reader cannot read is not a control.
        if total + 1 > self.width {
            return;
        }
        // The user's controls sit under the right edge of their own bubble, the
        // way the desktop's do; a reply's sit under its own first column.
        let lead = match msg.role {
            ChatRole::User => self.width.saturating_sub(total + 1),
            _ => 1,
        };
        let mut text = " ".repeat(lead);
        let mut column = lead;
        let mut spans: Vec<(usize, usize, RowAction)> = Vec::new();
        for (position, (label, (_, action))) in labels.iter().zip(chips.iter()).enumerate() {
            if position > 0 {
                text.push(' ');
                column += 1;
            }
            let end = column + visible_width(label);
            spans.push((column, end, action.clone()));
            text.push_str(&dim_raw(label));
            column = end;
        }
        text.push_str(RESET);
        self.rendered_lines.push(RenderedLine { text, dim: true });
        for (start, end, action) in spans {
            self.mark_row_span(msg_idx, origin, start, end, action);
        }
    }

    /// Record that the row just pushed is the one `action` opens, spanning the
    /// label the renderer wrote (from column 1, the transcript's gutter).
    fn mark_last_row(&mut self, msg_idx: usize, origin: usize, action: RowAction) {
        let Some(last) = self.rendered_lines.len().checked_sub(1) else {
            return;
        };
        let end = visible_width(&self.rendered_lines[last].text).max(1);
        let start = if end > 1 { 1 } else { 0 };
        let offset = last.saturating_sub(origin);
        self.row_meta
            .entry(msg_idx)
            .or_default()
            .push((offset, action, start, end));
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

    // ─── User message (markdown on a padded, full-width surface) ────────

    /// Columns between the user block's coloured edge and its text. The wrap
    /// width loses twice this, so a full line ends with the same inset on the
    /// right that it starts with on the left and the text never hugs the edge.
    const USER_CARD_INSET: usize = 2;

    fn render_user_message(&mut self, msg: &ChatMessage) {
        // A prompt that is still waiting in the agent's queue is not part of the
        // conversation yet: the frame draws it in the band above the composer
        // (`workbench::screen::queue_band`), and the transcript must not show a
        // card that reads like a message already sent.
        if msg.run_state == Some(RunState::Queued) {
            return;
        }
        let bg = self.theme.user_bg;
        // The block is a surface, not a tinted row: one empty row of its own
        // colour above and below the text. Those rows are the card's padding —
        // the transcript's blank separator rows stay terminal background and
        // sit outside it.
        self.push_user_card_row("", bg);
        // Below four columns the card's inset has nowhere to go: those layouts
        // bottom out at the one-column gutter every other row keeps.
        let inset = if self.width >= Self::USER_CARD_INSET * 2 {
            Self::USER_CARD_INSET
        } else {
            1
        };
        let indent = " ".repeat(inset);
        let rendered = self
            .md
            .render_text(&msg.content, self.width.saturating_sub(inset * 2).max(1));
        for line in rendered {
            self.push_user_card_row(&format!("{indent}{line}"), bg);
        }
        if matches!(
            msg.run_state,
            Some(RunState::Cancelled)
                | Some(RunState::Superseded)
                | Some(RunState::Merged)
                | Some(RunState::LostOnAgentRestart)
                | Some(RunState::Failed)
                | Some(RunState::NotAcknowledged)
        ) {
            let label = msg.run_state.unwrap().label();
            let t = format!("{indent}{}", dim(label));
            self.push_user_card_row(&t, bg);
        }
        // The pictures the reader attached, inside the bubble they came with —
        // and before the card's closing padding row, so the card keeps its
        // bottom edge under the caption rather than under a picture.
        self.render_pictures(msg);
        self.push_user_card_row("", bg);
    }

    /// One row of the user block: `text` on the block's background, padded out
    /// to the full width so the colour runs edge to edge and every row of a
    /// message — text, padding, run state — carries the same surface.
    fn push_user_card_row(&mut self, text: &str, bg: Color) {
        // `.max(3)`: below three columns the layout bottoms out (one gutter
        // column plus one content column), and the card's empty rows follow the
        // same floor so every degenerate width renders identical rows.
        self.rendered_lines.push(RenderedLine {
            text: apply_background_to_line(text, self.width.max(3), bg),
            dim: false,
        });
    }

    // ─── Pictures ───────────────────────────────────────────────────────

    /// The pictures a message carried: the file's name, linked to its path.
    ///
    /// The picture itself is not painted here — not as a thumbnail either. A
    /// text surface has no pixels of its own (the mosaic of half blocks this
    /// renderer used to try was removed for being a guess at a picture rather
    /// than a picture, see `workbench::image_view`), and the blank block a
    /// bitmap was then placed over cost every message four rows of height to
    /// say "there is a picture here" less clearly than its own name does. The
    /// picture is one `enter` (or one click) away in the preview box, where it
    /// is drawn at full size; the transcript line is the door to it.
    ///
    /// The caption is markdown's link, so `tab` + `enter` and a click both open
    /// it — the same two ways every other link in the conversation opens.
    fn render_pictures(&mut self, msg: &ChatMessage) {
        if msg.pictures.is_empty() {
            return;
        }
        // A picture inside the user bubble is part of the bubble: that is where
        // the reader put it, and a full-width block hanging under it would read
        // as a separate message.
        let card = matches!(msg.role, ChatRole::User);
        let bg = self.theme.user_bg;
        let inset = if self.width >= Self::USER_CARD_INSET * 2 {
            Self::USER_CARD_INSET
        } else {
            1
        };
        let indent = " ".repeat(inset);
        let content_width = if card {
            self.width.saturating_sub(inset * 2).max(1)
        } else {
            self.width.saturating_sub(2).max(1)
        };
        for picture in &msg.pictures {
            // Markdown rather than a hand-built row: `[…](…)` is exactly the
            // link styling the transcript paints every other link in, and the
            // OSC 8 wrapper it writes is what `tab` and a click find.
            let label = if picture.name.trim().is_empty() {
                file_name_of(&picture.path)
            } else {
                picture.name.clone()
            };
            // The caption is markdown's link text: a bracket or a newline in a
            // file name would close the row's own syntax early.
            let label = label.replace(['[', ']', '\n', '\r'], " ");
            let caption = format!(
                "[{label}]({})",
                crate::terminal_image::path_href(&picture.path)
            );
            for line in self.md.render_text(&caption, content_width) {
                if card {
                    self.push_user_card_row(&format!("{indent}{line}"), bg);
                } else if line.is_empty() {
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
        }
    }

    // ─── Assistant message (markdown, thinking first) ───────────────────

    fn render_assistant_message(
        &mut self,
        msg_idx: usize,
        msg: &ChatMessage,
        run: usize,
        fold: Option<ReplyFold>,
        origin: usize,
    ) {
        let has_thinking = msg
            .thinking
            .as_deref()
            .is_some_and(|t| !t.trim().is_empty())
            // Reasoning that has started but not sent its first token yet is
            // still the row the reader is watching: drawing nothing until the
            // first delta made the marker arrive a frame late, on a line the
            // transcript had already counted (a blank) — one more row appearing
            // under a bottom-anchored transcript, one more jump.
            || (msg.pending && msg.content.trim().is_empty() && msg.thinking.is_some());

        // The reading level folded this reply's work (and this answer's own
        // reasoning) away: one summary row stands in for it, above the answer.
        let summary_drawn = fold.is_some();
        if let Some(fold) = fold {
            self.push_reply_summary(msg_idx, fold.summary, origin);
        }

        // Render thinking block FIRST (before content). Reasoning is never its
        // text on this surface: a one-line marker says the model reasoned — the
        // row the desktop's `Thinking` block and the phone's thought row use —
        // and opening it (`enter` on the marker) shows the text beneath it.
        // `/reply-detail` is *not* what opens reasoning: no level of it shows a
        // body's text or a call's output (see [`ReplyDetail`]).
        //
        // A folded reply hides the marker too: its summary row already counts
        // the thought, except when reasoning is the reply's whole output and
        // the marker is the only row saying the model thought at all.
        let shows_marker = match fold {
            None => true,
            Some(_) => reasoning_only(msg),
        };
        if has_thinking && shows_marker {
            self.push_thinking_marker(msg_idx, msg, run, origin);
            if self.is_open(&msg.id, RowKind::Thinking) {
                self.push_thinking_text(msg);
            }
        }

        // Spacer between the reasoning and the answer, opened or not: the
        // marker (`▸ 思考`) is a row *about* the reasoning, so wedged against
        // the prose it reads as the answer's first line — the same complaint the
        // summary row below answers (reported 2026-10-30 against a collapsed
        // marker and the answer right under it). A reply that has only reasoned
        // so far has no content, so the spacer never appears mid-stream before
        // there is an answer to separate.
        if has_thinking && shows_marker && !msg.content.trim().is_empty() {
            self.rendered_lines.push(RenderedLine {
                text: String::new(),
                dim: true,
            });
        }

        // …and the same row above a folded reply's answer. The counts are chrome
        // that heads the answer, so wedged against the prose they read as its
        // first line (`▸ 工具调用 62 次 · 思考 39 次` and `是缓存命中率…` on
        // consecutive rows). This never stacks with the spacer above: a folded
        // reply draws its own marker only when the reasoning is the whole
        // output (`reasoning_only`), which leaves no content to space out here.
        if summary_drawn && !msg.content.trim().is_empty() {
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
    /// which tool, and how much of it — and nothing else: no bodies (a folded
    /// run has no row of its own to open, and `/export` is where a record's
    /// bodies live), no targets (twenty paths is what the fold exists to
    /// avoid).
    /// The `▸` marks it as a fold rather than a call.
    fn render_tool_run(&mut self, msg_idx: usize, run: usize, origin: usize) {
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
        self.mark_last_row(msg_idx, origin, RowAction::Burst { msg: msg_idx });
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
    /// them): that the model reasoned, and how many blocks — not the text. The
    /// ellipsis marks the reasoning *itself* still arriving (`pending` alone is
    /// not enough: the flag also covers the answer streaming after it).
    fn push_thinking_marker(
        &mut self,
        msg_idx: usize,
        msg: &ChatMessage,
        run: usize,
        origin: usize,
    ) {
        let mut label = self.fold_words.thinking.to_string();
        if run > 1 {
            label.push_str(&format!(" ×{run}"));
        }
        if msg.pending && msg.content.trim().is_empty() {
            label.push('…');
        }
        // The reasoning ink, and italic (the marker keeps the softer slant it
        // has always had) — both stated once, for the whole row, by
        // `push_quiet_row`.
        let ink = self.theme.thinking_text as u8;
        self.push_quiet_row(FOLD_MARKER, &label, ink, true);
        self.mark_last_row(msg_idx, origin, RowAction::Thinking { msg: msg_idx });
    }

    /// The reasoning text, drawn under its marker once the reader opens it.
    ///
    /// The same italic ink and inset the desktop's revealed thought uses: one
    /// step in from the marker, so a reasoning block and a tool call's body read
    /// the same way.
    fn push_thinking_text(&mut self, msg: &ChatMessage) {
        let thinking = msg.thinking.as_deref().unwrap_or("");
        let (detail_width, detail_rail) = self.detail_place();
        let width = detail_width.saturating_sub(2).max(1);
        let thinking_lines = if msg.pending {
            Self::render_streaming_markdown(
                &mut self.stream_caches,
                &format!("{}:t", msg.id),
                thinking,
                width,
                &mut self.md_thinking,
            )
        } else {
            self.md_thinking.render_text(thinking, width)
        };
        let think_prefix = format!("\x1b[3m\x1b[38;5;{}m", self.theme.thinking_text);
        for line in thinking_lines {
            if line.is_empty() {
                // A paragraph break keeps the frame: the row's rail is its
                // depth, not its text, so a blank line shows only that (the
                // block's own rail here, `hang_under_rail` adds the fold's).
                self.rendered_lines.push(RenderedLine {
                    text: detail_rail.clone(),
                    dim: true,
                });
            } else {
                // Re-apply thinking style after EVERY ANSI reset.
                let styled = reapply_style(&format!(" {line}"), &think_prefix);
                self.rendered_lines.push(RenderedLine {
                    text: format!("{detail_rail}{think_prefix}{styled}{RESET}"),
                    dim: true,
                });
            }
        }
    }

    /// One of the transcript's own quiet rows: `" ▸ label"` in a single ink,
    /// with the fold marker dimmed.
    ///
    /// The dim is switched off with `22` and the row's only full reset comes at
    /// its end. That is not pedantry: a reset in the *middle* of a row drops the
    /// colour it was given, and the frame's row painter re-arms the body ink
    /// after it — so the counts that followed a `dim(▸)` used to arrive in the
    /// bright body colour while only the marker wore the faint ink.
    fn push_quiet_row(&mut self, marker: &str, label: &str, ink: u8, italic_label: bool) {
        // A row is text like any other: a terminal too narrow for it clips it
        // instead of spilling past the last column. The marker and its two
        // spaces stay whole; the label takes what is left.
        let label = truncate_to_width(
            label,
            self.width.saturating_sub(3),
            &TruncateOptions::default(),
        );
        let mut text = fg_raw(ink, " ");
        text.push_str(&dim_raw(marker));
        text.push_str("\x1b[22m ");
        if italic_label {
            text.push_str(&italic_raw(&label));
        } else {
            text.push_str(&label);
        }
        text.push_str(RESET);
        self.rendered_lines.push(RenderedLine { text, dim: true });
    }

    /// The row a folded reply's work leaves behind: how much ran, not what it
    /// said. The same two counts — and the same order — the desktop's process
    /// fold and the phone's step summary use, so one reply reads the same on
    /// every client. It rides on the slice below it: the answer, or the call the
    /// reply stopped at.
    ///
    /// The wording comes from [`FoldWords`], which is what lets a Chinese frame
    /// say `工具调用 61 次 · 思考 30 次` while an English one says
    /// `61 tool calls · 30 thoughts`.
    fn push_reply_summary(&mut self, msg_idx: usize, summary: FoldSummary, origin: usize) {
        let label = self.fold_summary_label(summary);
        // Upright, not italic: the desktop's counts are plain, and slanted
        // digits would pull the eye to the one row that is meant to sit behind
        // the prose.
        //
        // The faintest ink the chrome has (the default palette's `241`, one step
        // under the `245` of body dim text): the row is a note about work that
        // already happened, and it should recede behind the answer it stands
        // above.
        let ink = Chrome::from_theme(&self.theme).muted;
        self.push_quiet_row(FOLD_MARKER, &label, ink, false);
        self.mark_last_row(msg_idx, origin, RowAction::Process { msg: msg_idx });
    }

    /// The header an opened fold's rows hang under: the same counts as the
    /// closed summary row, with the marker turned down. Pressing it (`enter`, or
    /// a click) folds the tree away again.
    fn push_fold_header(
        &mut self,
        msg_idx: usize,
        summary: FoldSummary,
        tree: OpenedTree,
        origin: usize,
    ) {
        let label = self.fold_summary_label(summary);
        let ink = Chrome::from_theme(&self.theme).muted;
        self.push_quiet_row(OPEN_MARKER, &label, ink, false);
        let action = match tree.kind {
            RowKind::Burst => RowAction::Burst { msg: tree.head },
            _ => RowAction::Process { msg: tree.head },
        };
        self.mark_last_row(msg_idx, origin, action);
    }

    /// `12 tool calls · 3 thoughts` / `工具调用 12 次 · 思考 3 次`, in the order
    /// [`FoldWords`] gives them; a fold that counted nothing (a burst of notes)
    /// falls back on the plain name for what it hides.
    fn fold_summary_label(&self, summary: FoldSummary) -> String {
        let words = self.fold_words;
        let mut parts: Vec<String> = Vec::new();
        if summary.tools > 0 {
            parts.push(FoldWords::counted(
                if summary.tools == 1 {
                    words.tool_call
                } else {
                    words.tool_calls
                },
                summary.tools,
            ));
        }
        if summary.thoughts > 0 {
            parts.push(FoldWords::counted(
                if summary.thoughts == 1 {
                    words.thought
                } else {
                    words.thoughts
                },
                summary.thoughts,
            ));
        }
        if parts.is_empty() {
            words.process.to_string()
        } else {
            parts.join(" · ")
        }
    }

    fn render_tool_message(&mut self, msg_idx: usize, msg: &ChatMessage, origin: usize) {
        let tool_name = msg
            .name
            .as_deref()
            .unwrap_or(msg.tool.as_deref().unwrap_or("tool"));
        let status = msg.tool_status.unwrap_or(ToolStatus::Running);

        let bg_color = match status {
            ToolStatus::Complete => self.theme.tool_success_bg,
            // A failure is *not* painted red: a call that came back an error is
            // an ordinary step of the work, and a red band across the screen
            // made a stopped `cargo test` the loudest thing in the transcript.
            // The neutral band plus the `✗` below says the same thing quietly.
            ToolStatus::Error => self.theme.tool_pending_bg,
            ToolStatus::Running => self.theme.tool_pending_bg,
        };

        let tool_args = msg.tool_args.as_deref();
        let body = ToolOutput::parse(&msg.content);
        // An `edit` call's arguments are the change itself; every other call's
        // body is its output. The badge counts whichever block the row will
        // open onto, so the two can never disagree.
        let args_diff = edit_args_diff(tool_name, tool_args);
        let badge = body
            .summary_ansi(&self.theme)
            .or_else(|| diff_badge(&args_diff, &self.theme));
        // The call itself is shortened around the badge rather than under it:
        // at the width where a long path fills the row, `+N -M` is the half the
        // reader cannot get back by opening the row.
        let reserve = badge
            .as_ref()
            .map_or(0, |summary| visible_width_of(summary) + 1);
        let mut line = format!(" {}", self.format_tool_call(tool_name, tool_args, reserve));
        if let Some(summary) = badge {
            line.push(' ');
            line.push_str(&summary);
        }
        // The one mark a failed row carries. No red, and no body: whatever went
        // wrong is in the output, which is `/export` — or the desktop — away.
        if status == ToolStatus::Error {
            line.push_str(&fg(self.theme.dim as u8, " ✗"));
        }

        self.rendered_lines.push(RenderedLine {
            text: apply_background_to_line(&line, self.width, bg_color),
            dim: status != ToolStatus::Running,
        });
        self.mark_last_row(msg_idx, origin, RowAction::Tool { msg: msg_idx });

        // A call is one row — the call itself, however it ended. The body waits
        // behind that row: opened one at a time by `enter` on it (or a click).
        // A transcript of twenty calls is twenty rows rather than twenty
        // previews, and a failure is one of the twenty.
        if !self.is_open(&msg.id, RowKind::Tool) {
            return;
        }

        let (detail_width, detail_rail) = self.detail_place();
        let (rows, hidden) = if args_diff.is_empty() {
            body.rows(detail_width, &self.theme)
        } else {
            diff_body_rows(&args_diff, detail_width, &self.theme)
        };
        self.rendered_lines.extend(rows.into_iter().map(|text| {
            let text = if text.is_empty() {
                text
            } else {
                format!("{detail_rail}{text}")
            };
            RenderedLine { text, dim: false }
        }));
        if hidden > 0 {
            let more = truncate_to_width(
                &format!(
                    "  … {hidden} more line{plural} · truncated",
                    plural = if hidden == 1 { "" } else { "s" },
                ),
                detail_width,
                &TruncateOptions::default(),
            );
            self.rendered_lines.push(RenderedLine {
                text: fg(self.theme.dim as u8, &format!("{detail_rail}{more}")),
                dim: true,
            });
        }
    }

    /// Format tool call display per tool type.
    ///
    /// `reserve` is the width the row's `+N -M` badge will take after it, so a
    /// long path is shortened to leave room instead of pushing the badge off
    /// the edge of the terminal.
    fn format_tool_call(
        &mut self,
        tool_name: &str,
        tool_args: Option<&str>,
        reserve: usize,
    ) -> String {
        let max_for = |prefix_len: usize| -> usize {
            10usize.max(
                self.width
                    .saturating_sub(2)
                    .saturating_sub(prefix_len)
                    .saturating_sub(reserve),
            )
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

/// The file's own name, for a picture whose caption arrived empty.
fn file_name_of(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path).trim();
    if name.is_empty() {
        path.to_string()
    } else {
        name.to_string()
    }
}

/// Build the assistant and thinking markdown renderers for `theme`.///
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

    /// The same transcript at the *detailed* reading level — the desktop's
    /// `detailed`: every slice keeps its own row, the work folded where it
    /// happened and the prose in order. Tests about what a slice looks like (a
    /// call's row, a marker's ink) render at this level so the *settled-reply*
    /// fold does not stand between them and the row under test. The compact-view
    /// tests use it too: at the default level a reply's own summary covers its
    /// calls first, so the compact fold is only on its own there.
    fn detailed_chat() -> ChatArea {
        let mut chat = ChatArea::new(W, None);
        chat.set_reply_detail(ReplyDetail::Detailed);
        chat
    }

    /// The row stop whose action opens the reasoning of message `id`.
    fn thinking_stop(chat: &mut ChatArea, id: &str) -> usize {
        let msg = chat
            .messages
            .iter()
            .position(|msg| msg.id == id)
            .expect("the message is in the transcript");
        chat.row_stops()
            .into_iter()
            .find(|stop| stop.action == RowAction::Thinking { msg })
            .expect("the reasoning block draws a marker")
            .row
    }

    #[test]
    fn a_marker_opens_its_reasoning_text_and_closes_again() {
        let mut chat = detailed_chat();
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

        // Closed by default: the reason said that it thought, never what it
        // thought — at *both* reading levels (see `ReplyDetail`).
        let closed = chat.render_all(W);
        assert!(!closed.iter().any(|l| l.contains("step by step reasoning")));
        assert!(closed.iter().any(|l| l.contains("final answer")));

        // `enter` on the marker: the text arrives under it, the view sticks to
        // the bottom.
        let row = thinking_stop(&mut chat, "m");
        assert!(chat.toggle_row(row).is_some());
        let open = chat.render_all(W);
        assert!(open.iter().any(|l| l.contains("step by step reasoning")));
        assert!(open.iter().any(|l| l.contains("final answer")));
        assert!(chat.auto_scroll);
        assert_matches_fresh_render(&mut chat);

        // `enter` again puts it back.
        let row = thinking_stop(&mut chat, "m");
        chat.toggle_row(row).expect("the marker is still a row");
        let again = chat.render_all(W);
        assert!(!again.iter().any(|l| l.contains("step by step reasoning")));
        assert_matches_fresh_render(&mut chat);
    }

    #[test]
    fn collapsed_thinking_shows_a_marker_never_its_text() {
        let mut chat = new_chat();
        chat.render(W);

        // Reasoning streaming (start_thinking marks the message pending): a
        // one-line marker that it is thinking, never the reasoning text. The
        // default level starts reasoning collapsed, like the desktop.
        chat.start_thinking();
        chat.append_thinking_delta("secret reasoning");
        let lines = chat.render_all(W);
        assert!(lines.iter().any(|l| l.contains("thinking")), "{lines:?}");
        assert!(
            !lines.iter().any(|l| l.contains("secret reasoning")),
            "{lines:?}"
        );

        // Content starts: the answer is its own output and stays on screen,
        // with the thought above it still a collapsed marker — the reasoning
        // text never reaches the screen.
        chat.append_to_last_message("answer");
        let lines = chat.render_all(W);
        assert!(
            lines.iter().any(|l| l.contains("thinking")),
            "the thought keeps its marker while the reply runs: {lines:?}"
        );
        assert!(
            !lines.iter().any(|l| l.contains("secret reasoning")),
            "{lines:?}"
        );
        assert!(lines.iter().any(|l| l.contains("answer")));

        // Run complete: the reply folds to its answer, with the thought
        // counted in the summary row — not shown.
        chat.mark_last_message_complete();
        let lines = chat.render_all(W);
        assert!(
            !lines.iter().any(|l| l.contains("secret reasoning")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains("1 thought")),
            "the fold counts the thought: {lines:?}"
        );
        assert!(lines.iter().any(|l| l.contains("answer")));
    }

    /// The reasoning-only shape (a thought with no prose of its own) keeps its
    /// marker after it settles — the reply has no answer to fold into, so the
    /// row is the only sign the model thought at all.
    #[test]
    fn a_settled_reasoning_only_block_keeps_its_marker() {
        let mut chat = new_chat();
        chat.render(W);
        chat.start_thinking();
        chat.append_thinking_delta("hmm");
        chat.end_thinking();
        let lines = chat.render_all(W);
        let plain = lines
            .iter()
            .map(|l| crate::utils::strip_ansi_codes(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(plain.contains("▸ thinking"), "{plain:?}");
        assert!(!plain.contains("hmm"), "{plain:?}");
    }

    /// No reading level shows the reasoning text, and none of them ever did: it
    /// flips between the answer alone and the reply with its process, and the
    /// thought stays a marker at both. Opening the marker is the row's own
    /// door (see `a_marker_opens_its_reasoning_text_and_closes_again`).
    #[test]
    fn no_reading_level_shows_the_reasoning_text() {
        let mut chat = new_chat();
        chat.render(W);
        chat.start_thinking();
        chat.append_thinking_delta("secret reasoning");
        chat.end_thinking();
        chat.append_to_last_message("answer");
        chat.mark_last_message_complete();

        for level in [ReplyDetail::Default, ReplyDetail::Detailed] {
            chat.set_reply_detail(level);
            let lines = chat.render_all(W);
            assert!(
                !lines.iter().any(|l| l.contains("secret reasoning")),
                "{level:?} leaked the reasoning: {lines:?}"
            );
            assert!(lines.iter().any(|l| l.contains("answer")), "{lines:?}");
        }
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

    /// Open every row the transcript draws (each one by the `enter` a reader
    /// would press on it), so a test that inspects a body or a reasoning text
    /// sees it. The walk is the reader's own: a summary row opens into its
    /// slices, and each of those opens its detail.
    fn open_all_rows(chat: &mut ChatArea) {
        for _ in 0..128 {
            // The controls are never open and never toggle: skipping them is
            // what keeps the walk from stopping on the first message's
            // `[copy]` with every fold still shut.
            let Some(stop) = chat
                .row_stops()
                .into_iter()
                .find(|stop| !stop.expanded && !is_control(&stop.action))
            else {
                return;
            };
            if chat.toggle_row(stop.row).is_none() {
                return;
            }
        }
        panic!("rows kept appearing: the walk did not settle");
    }

    /// Is this a control row (`[copy]`, `[fork]`) rather than a fold?
    fn is_control(action: &RowAction) -> bool {
        matches!(action, RowAction::Copy { .. } | RowAction::Fork { .. })
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

    /// [`assert_matches_fresh_render`] at an arbitrary width, for the resizing
    /// case: a wrap change re-measures every row, so the cached ranges have to
    /// equal what a fresh layout at that width produces.
    fn assert_matches_fresh_render_at(chat: &mut ChatArea, width: usize) {
        let incremental = chat.render_all(width);
        let mut fresh = ChatArea::new(width, None);
        fresh.revealed = chat.revealed.clone();
        fresh.compact_activity = chat.compact_activity;
        fresh.reply_detail = chat.reply_detail;
        set_messages(&mut fresh, chat.messages.clone());
        let expected = fresh.render_all(width);
        assert_eq!(
            incremental, expected,
            "width {width}: incremental rendering drifted from a fresh layout"
        );
    }

    /// The invariant the compact folds must not break: whatever the incremental
    /// paths did (appends, splices, folds), the transcript is exactly what a
    /// from-scratch layout of the same messages would produce.
    fn assert_matches_fresh_render(chat: &mut ChatArea) {
        let incremental = chat.render_all(W);
        // The ground truth is a full re-layout of the same messages, not
        // another walk through the incremental paths.
        let mut fresh = ChatArea::new(W, None);
        fresh.revealed = chat.revealed.clone();
        fresh.fold_words = chat.fold_words;
        fresh.compact_activity = chat.compact_activity;
        fresh.reply_detail = chat.reply_detail;
        fresh.run_in_flight = chat.run_in_flight;
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
            // Bound eagerly: `assert!`'s message is evaluated only on failure, so
            // calling `len()` inside it would leave this line permanently
            // "uncovered" and hide a real gap behind an artifact.
            let line_count = chat.rendered_lines.len();
            assert!(
                *end < line_count as i64,
                "range ({start}, {end}) is outside {line_count} lines"
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
        let rendered = render_trimmed(chat);
        let lines = no_controls(chat, rendered);
        lines
            .iter()
            .map(|line| strip(line).trim().to_string())
            .filter(|line| !line.is_empty())
            .collect()
    }

    /// The transcript's rows with the per-message controls taken out.
    ///
    /// Every settled message ends in one (`ChatArea::push_message_controls`).
    /// The cases that read a transcript as a list of *rows it says* take those
    /// out instead of restating them in every expected vector: the control row
    /// is the same under every message, and what those cases are about is the
    /// text, the folds and the markers around it. The rows come from
    /// [`ChatArea::control_rows`], not from the chips' spelling, so a localized
    /// frame is read the same way.
    fn no_controls(chat: &mut ChatArea, lines: Vec<String>) -> Vec<String> {
        let controls = chat.control_rows();
        lines
            .into_iter()
            .enumerate()
            .filter(|(index, _)| !controls.contains(index))
            .map(|(_, line)| line)
            .collect()
    }

    /// [`compact_lines`] with the tree's rail taken off, so a test can read an
    /// opened fold as the rows it stands for.
    fn rail_stripped(chat: &mut ChatArea) -> Vec<String> {
        let rendered = render_trimmed(chat);
        let lines = no_controls(chat, rendered);
        lines
            .iter()
            .map(|line| strip(line))
            .map(|line| {
                line.trim_start()
                    .strip_prefix(TREE_RAIL)
                    .unwrap_or(line.trim_start())
                    .trim()
                    .to_string()
            })
            .filter(|line| !line.is_empty())
            .collect()
    }

    #[test]
    fn compact_view_folds_a_run_of_calls_to_one_row() {
        let mut chat = detailed_chat();
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
            let mut chat = detailed_chat();
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
    /// fold cannot swallow it: the failed call keeps its row — one row, like
    /// every other call, its body behind that row.
    #[test]
    fn a_failed_call_is_not_folded() {
        let mut chat = detailed_chat();
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
            !lines.iter().any(|line| line.contains("permission denied")),
            "and its body waits for its own row like every other call: {lines:?}"
        );
        open_all_rows(&mut chat);
        let expanded = compact_lines(&mut chat);
        assert!(
            expanded
                .iter()
                .any(|line| line.contains("permission denied")),
            "the reason is one row away: {expanded:?}"
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
        let mut chat = detailed_chat();
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
        let mut chat = detailed_chat();
        chat.render(W);
        chat.set_compact_activity(true);
        // read, read, shell: one fold and one lone call.
        chat.add_message(read_of("/a.rs"));
        chat.add_message(read_of("/b.rs"));
        chat.add_message(completed_tool("shell", r#"{"command":"ls"}"#, "out\n"));
        let lines = compact_lines(&mut chat);
        assert_eq!(lines, vec!["▸ read 2 files", "$ ls"], "{lines:?}");

        // A running call passes through and breaks the run around it.
        let mut chat = detailed_chat();
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

    /// A failure keeps its own row — the fold may not swallow it — and, like
    /// every other call, one row is all it shows until that row opens.
    #[test]
    fn compact_view_never_folds_a_failed_call() {
        let mut chat = detailed_chat();
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
            !lines.iter().any(|line| line.contains("boom")),
            "one row is all it shows: {lines:?}"
        );
        open_all_rows(&mut chat);
        let expanded = compact_lines(&mut chat);
        assert!(
            expanded.iter().any(|line| line.contains("boom")),
            "and the reason is one row away: {expanded:?}"
        );
        assert_matches_fresh_render(&mut chat);
    }

    #[test]
    fn a_marker_keeps_its_place_while_the_compact_view_folds_it() {
        // A lone thought is already the marker the compact view would draw, so
        // the toggle changes nothing about it — and its text opens from the
        // marker itself, at either setting.
        let mut chat = detailed_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("u".into(), ChatRole::User, "go"));
        let mut assistant = ChatMessage::new("a".into(), ChatRole::Assistant, "the answer");
        assistant.thinking = Some("line one\nline two".into());
        chat.add_message(assistant);
        assert_eq!(
            compact_lines(&mut chat),
            vec!["go", "▸ thinking", "the answer"]
        );

        chat.set_compact_activity(true);
        let lines = compact_lines(&mut chat);
        assert_eq!(lines, vec!["go", "▸ thinking", "the answer"], "{lines:?}");

        open_all_rows(&mut chat);
        let opened = compact_lines(&mut chat);
        assert!(
            opened.iter().any(|line| line.contains("line one")),
            "the reasoning opens from its marker — {opened:?}"
        );
        assert_matches_fresh_render(&mut chat);
    }

    #[test]
    fn compact_view_merges_consecutive_thinking_blocks() {
        let mut chat = detailed_chat();
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

    /// Hiding reasoning keeps the reasoning *text* off screen, but the marker
    /// row stays: it is the one row that says a thought happened, and the
    /// compact view is where a run of them is counted. This is the same row the
    /// default level draws (reasoning starts collapsed there too).
    #[test]
    fn hidden_thinking_keeps_its_marker_and_loses_its_text() {
        // `ctrl+d`'s compact view folds the thought into its marker — the same
        // row the reader sees at every other level, never its text.
        let mut chat = detailed_chat();
        chat.render(W);
        chat.set_compact_activity(true);
        let mut msg = ChatMessage::new("a1".into(), ChatRole::Assistant, "answer");
        msg.thinking = Some("reasoning".into());
        chat.add_message(msg);
        let lines = compact_lines(&mut chat);
        assert_eq!(lines, vec!["▸ thinking", "answer"], "{lines:?}");
    }

    /// A run built the way a live run builds one — one call at a time — folds as
    /// it grows, and the transcript stays exactly what a fresh render produces.
    #[test]
    fn compact_view_folds_runs_built_incrementally() {
        let mut chat = detailed_chat();
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
        let mut chat = detailed_chat();
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
        let mut chat = detailed_chat();
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
        let mut chat = detailed_chat();
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

    // ─── Reading level (/reply-detail) ───────────────────────────────────

    fn assistant_with_thinking(id: &str, thinking: &str, content: &str) -> ChatMessage {
        ChatMessage {
            id: id.into(),
            role: ChatRole::Assistant,
            thinking: (!thinking.is_empty()).then(|| thinking.to_string()),
            content: content.into(),
            ..ChatMessage::new(String::new(), ChatRole::Assistant, "")
        }
    }

    /// A settled turn: the work it ran, in the shape the transcript builds it —
    /// a reasoning-only assistant message, the calls, an intermediate note, and
    /// the answer it ends on.
    fn settled_turn_messages() -> Vec<ChatMessage> {
        vec![
            ChatMessage::new("u1".into(), ChatRole::User, "fix the parser"),
            assistant_with_thinking("a1", "weighing the options", ""),
            read_of("/src/parser.rs"),
            ChatMessage::new("a2".into(), ChatRole::Assistant, "I read the parser."),
            ChatMessage::new(
                "a3".into(),
                ChatRole::Assistant,
                "**Fixed** the off-by-one.",
            ),
        ]
    }

    #[test]
    fn the_default_level_keeps_the_answer_and_counts_the_work() {
        let mut chat = new_chat();
        chat.render(W);
        set_messages(&mut chat, settled_turn_messages());
        assert_eq!(chat.reply_detail(), ReplyDetail::Default);

        let lines = compact_lines(&mut chat);
        assert_eq!(
            lines,
            vec![
                "fix the parser",
                "▸ 1 tool call · 1 thought",
                "Fixed the off-by-one.",
            ],
            "{lines:?}"
        );
        assert_matches_fresh_render(&mut chat);
    }

    #[test]
    /// The detailed level is the desktop's `detailed`: every slice stays where
    /// it happened, interleaved in order — and a body or a reasoning text is
    /// still nobody's default (`/reply-detail` opens neither; the rows do).
    fn the_detailed_level_keeps_every_slice_without_its_detail() {
        let mut chat = new_chat();
        chat.render(W);
        chat.set_reply_detail(ReplyDetail::Detailed);
        set_messages(&mut chat, settled_turn_messages());

        let lines = compact_lines(&mut chat);
        // The thought is a marker, never its text — at every level.
        assert!(
            !lines.iter().any(|l| l.contains("weighing the options")),
            "detailed does not print reasoning — {lines:?}"
        );
        // Every slice keeps its own row, in order: no summary row at all.
        assert!(
            lines.iter().any(|l| l.contains("read /src/parser.rs")),
            "{lines:?}"
        );
        assert!(
            lines.iter().all(|l| !l.contains("tool call")),
            "detailed draws no count row — {lines:?}"
        );
        // Every piece of prose the model wrote stays, in order.
        assert!(
            lines.iter().any(|l| l.contains("I read the parser.")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains("Fixed the off-by-one.")),
            "{lines:?}"
        );
        // The call's body stays behind its own row at either
        // level.
        assert!(!lines.iter().any(|l| l.contains("body")), "{lines:?}");
        assert_matches_fresh_render(&mut chat);
    }

    /// An opened process is a tree: one header that folds it again, and every
    /// slice it stood for hung under a rail beneath it.
    #[test]
    fn an_opened_process_hangs_its_rows_under_one_header() {
        let mut chat = new_chat();
        chat.render(W);
        set_messages(&mut chat, settled_turn_messages());
        let stop = chat
            .row_stops()
            .into_iter()
            .find(|stop| matches!(stop.action, RowAction::Process { .. }))
            .expect("the fold draws a stop");
        chat.toggle_row(stop.row).expect("the fold opens");

        // The rows the fold stood for, in reply order, each under the rail.
        assert_eq!(
            rail_stripped(&mut chat),
            vec![
                "fix the parser",
                "▾ 1 tool call · 1 thought",
                "▸ thinking",
                "read /src/parser.rs",
                "I read the parser.",
                "Fixed the off-by-one.",
            ],
        );
        // …and the answer is outside the tree: it is the reply's output, not
        // part of its process. Everything between the header and it hangs.
        let rows: Vec<String> = render_trimmed(&mut chat)
            .iter()
            .map(|line| strip(line))
            .collect();
        let header = rows.iter().position(|row| row.contains("▾")).unwrap();
        let answer = rows
            .iter()
            .position(|row| row.contains("off-by-one"))
            .unwrap();
        assert!(
            rows[header + 1].trim() == TREE_RAIL,
            "a gap under the header carries the rail on down (and nothing else): {rows:?}"
        );
        assert!(
            rows[header + 2..answer - 1]
                .iter()
                .filter(|row| !row.trim().is_empty())
                .all(|row| row.trim_start().starts_with(TREE_RAIL)),
            "every row under the header hangs: {rows:?}"
        );
        // …in the column the header's own marker occupies: the rows hang *from*
        // the title, not off to the left of it.
        let indent = |row: &str| row.chars().take_while(|c| *c == ' ').count();
        assert_eq!(indent(&rows[header]), 1, "{rows:?}");
        for row in rows[header + 2..answer - 1]
            .iter()
            .filter(|row| !row.trim().is_empty())
        {
            assert_eq!(
                indent(row),
                1,
                "a tree row puts its rail in the marker's column: {rows:?}"
            );
        }
        assert!(
            rows[answer - 1].trim().is_empty(),
            "a blank row separates the tree from the answer: {rows:?}"
        );
        assert_matches_fresh_render(&mut chat);

        // The rail is paid for by the layout, not added on top of it: no row of
        // the tree is wider than the terminal, and a call's band starts after
        // the rail it hangs from — the spine keeps the terminal's own
        // background — and still runs out to the right edge.
        for width in [W, 64, 40] {
            let rendered = chat.render_all(width);
            for row in &rendered {
                assert!(
                    visible_width_of(row) <= width,
                    "a tree row is wider than the terminal ({width}): {row:?}"
                );
            }
            let band = rendered
                .iter()
                .find(|row| strip(row).contains("/src/parser.rs"))
                .expect("the call is on screen");
            let band_at = band.find("\x1b[48;5;").expect("the call is banded");
            let before = strip(&band[..band_at]);
            assert_eq!(
                before,
                format!(" {TREE_RAIL}"),
                "the band begins inside the tree's text column, not across its rail ({width}): {band:?}"
            );
            assert_eq!(
                visible_width_of(band),
                width,
                "the band runs to the right edge ({width})"
            );
            assert_matches_fresh_render_at(&mut chat, width);
        }
    }

    /// A gap *inside* the work a fold opened keeps the spine.
    ///
    /// Reasoning and a call's body both come in paragraphs, and the blank row
    /// between two of them used to be empty: the rail stopped there, and a fold
    /// that had been one block read as two that happened to be indented alike.
    /// The gap therefore carries the rails of the rows it sits between — and the
    /// block's *trailing* gap is dropped instead, because a rail below the last
    /// row it holds hangs over nothing.
    #[test]
    fn a_gap_inside_an_opened_fold_keeps_the_spine() {
        let mut chat = new_chat();
        chat.render(W);
        set_messages(
            &mut chat,
            vec![
                ChatMessage::new("u1".into(), ChatRole::User, "fix the parser"),
                assistant_with_thinking("a1", "first thought\n\nsecond thought\n\n", ""),
                completed_tool("shell", r#"{"command":"ls"}"#, "alpha\n\nbeta\n\n"),
                ChatMessage::new("a3".into(), ChatRole::Assistant, "Fixed."),
            ],
        );
        open_all_rows(&mut chat);
        let rows: Vec<String> = render_trimmed(&mut chat)
            .iter()
            .map(|line| strip(line))
            .collect();
        let at = |needle: &str| {
            rows.iter()
                .position(|row| row.contains(needle))
                .unwrap_or_else(|| panic!("{needle} is on screen: {rows:?}"))
        };

        // Reasoning: one row, a gap that is the two rails it hangs from, one
        // row — the fold still reads as a single block.
        let first = at("first thought");
        assert_eq!(at("second thought"), first + 2, "{rows:?}");
        assert_eq!(
            rows[first + 1],
            format!(" {TREE_RAIL} {TREE_RAIL}"),
            "a paragraph gap in reasoning is its rails and nothing else: {rows:?}"
        );

        // A call's body is the same: one rail for the fold, one for the body.
        let alpha = at("alpha");
        assert_eq!(at("beta"), alpha + 2, "{rows:?}");
        assert_eq!(
            rows[alpha + 1].trim_end(),
            format!(" {TREE_RAIL} {TREE_RAIL}"),
            "a paragraph gap in a body keeps both rails: {rows:?}"
        );

        // …and the tree ends on the last row with something on it: the gap the
        // body ended with is gone, so the answer is separated by the blank
        // separator row and no rail hangs over it.
        assert_eq!(rows[at("beta") + 1], "", "{rows:?}");
        assert_eq!(rows[at("beta") + 2], " Fixed.", "{rows:?}");
        assert_matches_fresh_render(&mut chat);
    }

    /// The header is the way back out: pressing it folds the tree onto its
    /// counts again. Before it existed an opened fold had no exit at all — the
    /// summary row *became* the rows it hid, so the only row that could close
    /// them was gone.
    #[test]
    fn the_tree_header_folds_its_rows_away_again() {
        let mut chat = new_chat();
        chat.render(W);
        set_messages(&mut chat, settled_turn_messages());
        let folded = compact_lines(&mut chat);

        let summary = chat
            .row_stops()
            .into_iter()
            .find(|stop| matches!(stop.action, RowAction::Process { .. }))
            .expect("the fold draws a stop");
        chat.toggle_row(summary.row).expect("the fold opens");

        let header = chat
            .row_stops()
            .into_iter()
            .find(|stop| matches!(stop.action, RowAction::Process { .. }))
            .expect("the opened fold keeps a row to close it");
        assert!(header.expanded, "the header reports the fold open");
        let landed = chat.toggle_row(header.row).expect("the header closes it");
        assert_eq!(
            compact_lines(&mut chat),
            folded,
            "closing the tree restores the folded transcript"
        );
        // The cursor stays on the fold rather than on whatever line it used to
        // occupy: closing moves the header down onto the answer.
        assert_eq!(
            landed,
            chat.row_stops()
                .into_iter()
                .find(|stop| matches!(stop.action, RowAction::Process { .. }))
                .expect("the fold draws a stop")
                .row
        );
        assert_matches_fresh_render(&mut chat);
    }

    /// A reply that never wrote prose has no answer to keep outside the tree: its
    /// head is just the newest slice, so every row hangs under the header.
    #[test]
    fn an_opened_reply_without_an_answer_hangs_every_row() {
        let mut chat = new_chat();
        chat.render(W);
        set_messages(
            &mut chat,
            vec![
                ChatMessage::new("u1".into(), ChatRole::User, "go"),
                read_of("/a.rs"),
                read_of("/b.rs"),
            ],
        );
        let stop = chat
            .row_stops()
            .into_iter()
            .find(|stop| matches!(stop.action, RowAction::Process { .. }))
            .expect("the fold draws a stop");
        chat.toggle_row(stop.row).expect("the fold opens");
        assert_eq!(
            rail_stripped(&mut chat),
            vec!["go", "▾ 1 tool call", "read /a.rs", "read /b.rs"],
        );
        let rows: Vec<String> = render_trimmed(&mut chat)
            .iter()
            .map(|line| strip(line))
            .collect();
        let header = rows.iter().position(|row| row.contains("▾")).unwrap();
        assert!(
            rows[header + 1..]
                .iter()
                .filter(|row| !row.trim().is_empty())
                .all(|row| row.trim_start().starts_with(TREE_RAIL)),
            "nothing follows the tree when the reply has no answer: {rows:?}"
        );
        assert_matches_fresh_render(&mut chat);
    }

    /// A call opened inside a tree keeps its body under it, indented like the
    /// rest of the tree.
    #[test]
    fn a_call_inside_a_tree_opens_its_body_in_place() {
        let mut chat = new_chat();
        chat.render(W);
        set_messages(&mut chat, settled_turn_messages());
        let summary = chat
            .row_stops()
            .into_iter()
            .find(|stop| matches!(stop.action, RowAction::Process { .. }))
            .expect("the fold draws a stop");
        chat.toggle_row(summary.row).expect("the fold opens");

        let call = chat
            .row_stops()
            .into_iter()
            .find(|stop| matches!(stop.action, RowAction::Tool { .. }))
            .expect("the call is a row of the tree");
        assert!(call.start >= TREE_GUTTER, "the row hangs under the rail");
        chat.toggle_row(call.row).expect("the call opens");

        let rows: Vec<String> = render_trimmed(&mut chat)
            .iter()
            .map(|line| strip(line))
            .collect();
        let body = rows
            .iter()
            .position(|row| row.contains("body"))
            .unwrap_or_else(|| panic!("the call's body is on screen: {rows:?}"));
        assert!(
            rows[body]
                .trim_end()
                .starts_with(&format!(" {TREE_RAIL} {TREE_RAIL}")),
            "the body hangs one level in, under a rail of its own: {rows:?}"
        );
        assert_matches_fresh_render(&mut chat);
        // A body is laid out in the tree's column too: no row spills past the
        // terminal, whatever the width.
        for width in [W, 40] {
            for row in &chat.render_all(width) {
                assert!(
                    visible_width_of(row) <= width,
                    "a body row is wider than the terminal ({width}): {row:?}"
                );
            }
            assert_matches_fresh_render_at(&mut chat, width);
        }
    }

    /// A reasoning block inside an opened fold nests the same way its text's
    /// sibling — a call's body — does: one level in, under a rail of its own.
    #[test]
    fn a_thought_inside_a_tree_nests_one_level_in() {
        let mut chat = new_chat();
        chat.render(W);
        set_messages(&mut chat, settled_turn_messages());
        let summary = chat
            .row_stops()
            .into_iter()
            .find(|stop| matches!(stop.action, RowAction::Process { .. }))
            .expect("the fold draws a stop");
        chat.toggle_row(summary.row).expect("the fold opens");
        let marker = chat
            .row_stops()
            .into_iter()
            .find(|stop| matches!(stop.action, RowAction::Thinking { .. }))
            .expect("the thought is a row of the tree");
        // The marker itself is a level-1 row (one rail); its text is level 2.
        let marker_row = render_trimmed(&mut chat)
            .iter()
            .map(|line| strip(line))
            .find(|row| row.contains("thinking"))
            .expect("the marker is on screen");
        assert!(
            marker_row.starts_with(&format!(" {TREE_RAIL}"))
                && !marker_row.starts_with(&format!(" {TREE_RAIL} {TREE_RAIL}")),
            "the marker is one rail in: {marker_row:?}"
        );

        chat.toggle_row(marker.row).expect("the marker opens");
        let rows: Vec<String> = render_trimmed(&mut chat)
            .iter()
            .map(|line| strip(line))
            .collect();
        let text = rows
            .iter()
            .position(|row| row.contains("weighing the options"))
            .unwrap_or_else(|| panic!("the reasoning text is on screen: {rows:?}"));
        assert!(
            rows[text]
                .trim_end()
                .starts_with(&format!(" {TREE_RAIL} {TREE_RAIL}")),
            "the reasoning text hangs one level in: {rows:?}"
        );
        assert_matches_fresh_render(&mut chat);
    }

    /// The rail goes in front of a row and *outside* its band: a call's fill
    /// belongs to the tree's text column, so it starts after the rail and runs
    /// to the right edge. A band painted across the rail broke the spine the
    /// fold is drawn with.
    #[test]
    fn the_rail_is_painted_beside_a_call_band() {
        assert_eq!(
            with_gutter("\x1b[48;5;236m read a.rs \x1b[0m", " │"),
            " │\x1b[48;5;236m read a.rs \x1b[0m"
        );
        assert_eq!(with_gutter(" ▸ thinking", " │"), " │ ▸ thinking");
    }

    /// Opening the settled summary replays the reply's process in place: the
    /// notes come back, the work still folded where it happened, and every row
    /// it reveals opens on its own.
    #[test]
    fn opening_the_process_replays_the_reply_in_place() {
        let mut chat = new_chat();
        chat.render(W);
        set_messages(&mut chat, settled_turn_messages());

        let folded = compact_lines(&mut chat);
        assert_eq!(
            folded,
            vec![
                "fix the parser",
                "▸ 1 tool call · 1 thought",
                "Fixed the off-by-one.",
            ],
            "{folded:?}"
        );

        // The summary row is a `tab` stop whose action opens this reply.
        let stop = chat
            .row_stops()
            .into_iter()
            .find(|stop| matches!(stop.action, RowAction::Process { .. }))
            .expect("the fold draws a stop");
        assert!(!stop.expanded);
        chat.toggle_row(stop.row).expect("the stop opens");

        let open = compact_lines(&mut chat);
        assert!(
            open.iter().any(|line| line.contains("I read the parser.")),
            "the note is back — {open:?}"
        );
        assert!(
            !open
                .iter()
                .any(|line| line.contains("weighing the options")),
            "opening the process does not print reasoning — {open:?}"
        );
        // Opening a fold is not a level change: the rest of the transcript
        // stays as folded as it was.
        assert_eq!(chat.reply_detail(), ReplyDetail::Default);
        assert_matches_fresh_render(&mut chat);
    }

    /// A call's body and a reasoning block's text open one at a time — the
    /// desktop's rows, reached from the keyboard.
    #[test]
    fn a_call_and_a_thought_open_their_own_detail() {
        let mut chat = detailed_chat();
        chat.render(W);
        set_messages(
            &mut chat,
            vec![
                ChatMessage::new("u".into(), ChatRole::User, "go"),
                ChatMessage {
                    id: "t".into(),
                    role: ChatRole::Assistant,
                    thinking: Some("the reasoning".into()),
                    content: "answer".into(),
                    ..ChatMessage::new(String::new(), ChatRole::Assistant, "")
                },
                completed_tool("read", r#"{"path":"/a.rs"}"#, "the call's body"),
            ],
        );

        // Both rows are stops, and neither detail is on screen yet. The
        // messages' control rows are stops too (they are presses as well), so
        // they are counted out of this one: what this case is about is the two
        // details.
        let stops: Vec<_> = chat
            .row_stops()
            .into_iter()
            .filter(|stop| !is_control(&stop.action))
            .collect();
        assert_eq!(stops.len(), 2, "{stops:?}");
        assert!(!stops.iter().any(|stop| stop.expanded));
        let plain = compact_lines(&mut chat).join("\n");
        assert!(!plain.contains("the reasoning"), "{plain}");
        assert!(!plain.contains("the call's body"), "{plain}");

        // `enter` on the marker...
        let marker = thinking_stop(&mut chat, "t");
        chat.toggle_row(marker).expect("the marker opens");
        let thought = compact_lines(&mut chat).join("\n");
        assert!(thought.contains("the reasoning"), "{thought}");
        // ...and on the call.
        let call = chat
            .row_stops()
            .into_iter()
            .find(|stop| matches!(stop.action, RowAction::Tool { .. }))
            .expect("the call draws a row")
            .row;
        chat.toggle_row(call).expect("the call opens");
        let both = compact_lines(&mut chat).join("\n");
        assert!(both.contains("the reasoning"), "{both}");
        assert!(both.contains("the call's body"), "{both}");
        assert_matches_fresh_render(&mut chat);
    }

    /// Every message the reader wrote or read ends in its controls; a call's row
    /// and a compaction divider end in nothing.
    ///
    /// This is the desktop's pair of footer controls (`MessageBlock`'s copy
    /// button and its fork button beside it), made permanent: a terminal has no
    /// hover to reveal them under, so the row is always there — one row per
    /// message, the last one, where `tab` reaches it after the message's own
    /// rows.
    #[test]
    fn the_controls_row_follows_each_message() {
        let mut chat = detailed_chat();
        chat.render(W);
        set_messages(
            &mut chat,
            vec![
                ChatMessage::new("u".into(), ChatRole::User, "fix it"),
                ChatMessage::new("a".into(), ChatRole::Assistant, "fixed"),
                completed_tool("read", r#"{"path":"/a.rs"}"#, "the body"),
            ],
        );
        let rows: Vec<String> = render_trimmed(&mut chat)
            .iter()
            .map(|line| strip(line))
            .collect();
        let controls = chat.control_rows();
        assert_eq!(controls.len(), 2, "one per message, none for the call");
        // The prompt's sits under the right edge of her own card, outside its
        // surface; the reply's sits at its own first column.
        assert_eq!(rows[controls[0]].trim(), "[copy]", "{rows:?}");
        assert!(
            rows[controls[0]].starts_with("     "),
            "the prompt's control is right-aligned: {:?}",
            rows[controls[0]]
        );
        assert_eq!(rows[controls[1]].trim(), "[copy] [fork]", "{rows:?}");
        assert!(
            rows[controls[1]].starts_with(" [copy]"),
            "a reply's controls lead at its first column: {:?}",
            rows[controls[1]]
        );
        // Nothing else in the transcript carries one.
        assert!(rows
            .iter()
            .enumerate()
            .all(|(index, row)| controls.contains(&index) || !row.contains("[copy]")));
    }

    /// The two controls are two stops on one row, each over its own label — what
    /// keeps a click on `[分支]` from running `[复制]`.
    #[test]
    fn the_two_controls_are_separate_stops_on_one_row() {
        let mut chat = detailed_chat();
        chat.render(W);
        set_messages(
            &mut chat,
            vec![
                ChatMessage::new("u".into(), ChatRole::User, "go"),
                ChatMessage::new("a".into(), ChatRole::Assistant, "done"),
            ],
        );
        let stops: Vec<_> = chat
            .row_stops()
            .into_iter()
            .filter(|stop| is_control(&stop.action))
            .collect();
        let copy = stops
            .iter()
            .find(|stop| stop.action == RowAction::Copy { msg: 1 })
            .expect("the reply has a copy control");
        let fork = stops
            .iter()
            .find(|stop| stop.action == RowAction::Fork { msg: 1 })
            .expect("the reply has a fork control");
        assert_eq!(copy.row, fork.row, "one row carries both");
        assert!(copy.end < fork.start, "the chips split the row: {copy:?}");
        let rows = render_trimmed(&mut chat);
        let row = strip(&rows[copy.row]);
        assert_eq!(&row[copy.start..copy.end], "[copy]");
        assert_eq!(&row[fork.start..fork.end], "[fork]");
        // Neither is ever "open": a control has nothing to unfold.
        assert!(!copy.expanded && !fork.expanded);
    }

    /// A reply that is still arriving has no controls yet: the fork branches at
    /// a prompt that is still being answered, and the desktop hides both buttons
    /// while it streams for the same reason.
    #[test]
    fn a_streaming_reply_has_no_controls_yet() {
        let mut chat = new_chat();
        chat.render(W);
        let mut reply = ChatMessage::new("a".into(), ChatRole::Assistant, "half an ans");
        reply.pending = true;
        set_messages(
            &mut chat,
            vec![ChatMessage::new("u".into(), ChatRole::User, "go"), reply],
        );
        let rows = chat.control_rows();
        assert_eq!(rows.len(), 1, "only the settled prompt has a row: {rows:?}");
        // …and it appears under the reply the moment it settles.
        chat.messages[1].pending = false;
        chat.mark_message_dirty(1);
        chat.render(W);
        assert_eq!(chat.control_rows().len(), 2);
    }

    /// A prompt learns the journal entry it was written to, which is what the
    /// fork control points at.
    #[test]
    fn a_prompt_learns_the_entry_it_was_written_to() {
        let mut chat = detailed_chat();
        chat.render(W);
        set_messages(
            &mut chat,
            vec![
                ChatMessage::new("u".into(), ChatRole::User, "go"),
                ChatMessage::new("a".into(), ChatRole::Assistant, "done"),
            ],
        );
        let reply_row = *chat.control_rows().last().expect("the reply's controls");
        // A row that came from history knows its entry; a live one does not
        // until the agent names it.
        assert_eq!(chat.preceding_user_entry(reply_row), None);
        chat.bind_user_entry("run-1", "entry-9", "go");
        assert_eq!(
            chat.preceding_user_entry(reply_row).as_deref(),
            Some("entry-9")
        );
        // An announcement whose text is not the prompt's is not taken: binding
        // the wrong entry would fork the wrong place.
        let mut other = detailed_chat();
        other.render(W);
        set_messages(
            &mut other,
            vec![
                ChatMessage::new("u".into(), ChatRole::User, "go"),
                ChatMessage::new("a".into(), ChatRole::Assistant, "done"),
            ],
        );
        other.bind_user_entry("run-2", "entry-10", "something else");
        assert_eq!(other.preceding_user_entry(reply_row), None);
    }

    /// A running reply keeps every thought, call and piece of prose on screen.
    /// Settling is what folds the process away, leaving the answer the reply
    /// ended on.
    #[test]
    fn a_streaming_reply_keeps_the_work_and_prose_until_it_settles() {
        let mut chat = new_chat();
        chat.render(W);
        let mut messages = settled_turn_messages();
        messages[4].content = "Fixed".into();
        messages[4].pending = true;
        set_messages(&mut chat, messages);

        let lines = compact_lines(&mut chat);
        assert_eq!(
            lines,
            vec![
                "fix the parser",
                "▸ thinking",
                "read /src/parser.rs",
                "I read the parser.",
                "Fixed",
            ],
            "{lines:?}"
        );
        assert!(!lines.iter().any(|l| l.contains("weighing the options")));

        // Settling folds the notes the answer replaced: one summary row above
        // the answer the reply ends on.
        chat.messages[4].pending = false;
        chat.messages[4].content = "Fixed the off-by-one.".into();
        chat.rerender();
        let lines = compact_lines(&mut chat);
        assert_eq!(
            lines,
            vec![
                "fix the parser",
                "▸ 1 tool call · 1 thought",
                "Fixed the off-by-one.",
            ],
            "{lines:?}"
        );
        assert_matches_fresh_render(&mut chat);
    }

    /// The output and work between calls keep their individual rows while the
    /// reply runs. Settling is what folds those notes and calls away.
    #[test]
    fn a_live_reply_keeps_the_notes_it_wrote_between_its_calls() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("u".into(), ChatRole::User, "go"));
        chat.add_message(ChatMessage::new("a0".into(), ChatRole::Assistant, ""));
        chat.start_thinking();
        chat.append_thinking_delta("which file holds the fold?");
        chat.end_thinking();
        chat.add_tool_start("c1", "read", Some(r#"{"path":"/src/a.rs"}"#.into()));
        chat.finish_tool("c1", None, false);
        chat.append_to_last_message("Let me compare both files.");
        chat.start_thinking();
        chat.append_thinking_delta("the second one matters");
        chat.end_thinking();
        chat.add_tool_start("c2", "read", Some(r#"{"path":"/src/b.rs"}"#.into()));
        chat.finish_tool("c2", None, false);
        chat.append_to_last_message("Both fold the same way.");

        let lines = compact_lines(&mut chat);
        assert_eq!(
            lines,
            vec![
                "go",
                "▸ thinking",
                "read /src/a.rs",
                "Let me compare both files.",
                "▸ thinking",
                "read /src/b.rs",
                "Both fold the same way.",
            ],
            "{lines:?}"
        );

        // Settling leaves the answer the reply ended on, with the notes it
        // replaced counted in one row above it.
        chat.mark_last_message_complete();
        let lines = compact_lines(&mut chat);
        assert_eq!(
            lines,
            vec![
                "go",
                "▸ 2 tool calls · 2 thoughts",
                "Both fold the same way.",
            ],
            "{lines:?}"
        );
        assert_matches_fresh_render(&mut chat);
    }

    /// A lone finished call and thought keep their own rows for the whole run.
    #[test]
    fn a_live_reply_keeps_single_steps_visible() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("u".into(), ChatRole::User, "go"));
        chat.add_message(ChatMessage::new("a0".into(), ChatRole::Assistant, ""));
        chat.start_thinking();
        chat.append_thinking_delta("where to look");
        chat.end_thinking();
        chat.append_to_last_message("First look.");
        chat.add_tool_start("c1", "read", Some(r#"{"path":"/b.rs"}"#.into()));
        chat.finish_tool("c1", None, false);
        chat.append_to_last_message("Second look.");

        let lines = compact_lines(&mut chat);
        assert_eq!(
            lines,
            vec![
                "go",
                "▸ thinking",
                "First look.",
                "read /b.rs",
                "Second look.",
            ],
            "{lines:?}"
        );
        assert_matches_fresh_render(&mut chat);
    }

    /// Reasoning that begins *after* the model wrote prose is its own slice,
    /// below that prose. Drawing it on the prose message would put the
    /// `▸ thinking` marker above text it followed — the transcripts this client
    /// builds are one message per slice, so a late thought starts one.
    #[test]
    fn a_late_thought_lands_below_the_prose_it_followed() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("u".into(), ChatRole::User, "go"));
        chat.add_message(ChatMessage::new("a0".into(), ChatRole::Assistant, ""));
        chat.append_to_last_message("first paragraph");
        chat.start_thinking();
        chat.append_thinking_delta("reconsidering");
        chat.end_thinking();
        chat.append_to_last_message("second paragraph");

        let lines = compact_lines(&mut chat);
        let at = |needle: &str| {
            lines
                .iter()
                .position(|line| line.contains(needle))
                .unwrap_or_else(|| panic!("{needle} missing — {lines:?}"))
        };
        assert!(at("first paragraph") < at("thinking"), "{lines:?}");
        assert!(at("thinking") < at("second paragraph"), "{lines:?}");
        assert_matches_fresh_render(&mut chat);
    }

    /// The marker the reader is watching never leaves the screen when the
    /// thought it announces stops growing: it drops its ellipsis and stays.
    ///
    /// Running replies never fold, so the bottom-anchored transcript cannot
    /// pull every line above the marker down when reasoning ends.
    #[test]
    fn the_live_marker_keeps_its_row_when_the_thought_ends() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("u".into(), ChatRole::User, "go"));
        chat.begin_reply_run();

        // Reasoning that has started but sent no token yet is already the row
        // the reader is waiting on: it appears with the thought, not with its
        // first byte.
        chat.start_thinking();
        assert_eq!(compact_lines(&mut chat), vec!["go", "▸ thinking…"]);
        chat.append_thinking_delta("weighing it");
        let live = compact_lines(&mut chat);
        assert_eq!(live, vec!["go", "▸ thinking…"], "{live:?}");
        let rows = chat.render_all(W).len();

        chat.end_thinking();
        let ended = compact_lines(&mut chat);
        assert_eq!(ended, vec!["go", "▸ thinking"], "{ended:?}");
        // The transcript never *loses* a row here: the marker keeps its place,
        // and the reply's control row arrives beneath it now that the slice has
        // settled (a streaming reply has none — see `push_message_controls`).
        assert!(
            chat.render_all(W).len() >= rows,
            "the marker left the screen: every line above it would jump down"
        );
        assert_matches_fresh_render(&mut chat);
    }

    /// A running reply's transcript only ever grows.
    ///
    /// The view is bottom-anchored — its window is the transcript's last rows —
    /// so a row that *disappears* at the tail pulls every line above it down,
    /// and the next event pushes them back up. Folding the step the reader was
    /// watching the instant it finished did that once per step.
    #[test]
    fn a_running_reply_only_ever_grows() {
        let mut chat = new_chat();
        chat.render(W);
        let mut rows = vec![chat.render_all(W).len()];
        chat.add_message(ChatMessage::new("u".into(), ChatRole::User, "go"));
        rows.push(chat.render_all(W).len());
        chat.begin_reply_run();
        rows.push(chat.render_all(W).len());
        for i in 0..4 {
            chat.start_thinking();
            rows.push(chat.render_all(W).len());
            chat.append_thinking_delta("weighing it");
            rows.push(chat.render_all(W).len());
            chat.end_thinking();
            rows.push(chat.render_all(W).len());
            let id = format!("c{i}");
            chat.add_tool_start(&id, "read", Some(r#"{"path":"/a.rs"}"#.into()));
            rows.push(chat.render_all(W).len());
            chat.finish_tool(&id, None, false);
            rows.push(chat.render_all(W).len());
        }
        chat.start_thinking();
        chat.append_thinking_delta("one more look");
        chat.end_thinking();
        rows.push(chat.render_all(W).len());
        chat.append_to_last_message("the answer");
        rows.push(chat.render_all(W).len());

        for pair in rows.windows(2) {
            assert!(
                pair[1] >= pair[0],
                "a row left the tail, so every line above it moved: {rows:?}"
            );
        }
        assert_matches_fresh_render(&mut chat);
    }

    /// Finished calls and the call the reader is waiting on all stay visible.
    #[test]
    fn a_live_run_keeps_finished_and_running_calls_visible() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("u".into(), ChatRole::User, "go"));
        chat.start_thinking();
        chat.append_thinking_delta("weighing it");
        chat.add_tool_start("c1", "read", Some(r#"{"path":"/a.rs"}"#.into()));
        chat.finish_tool("c1", None, false);
        chat.add_tool_start("c2", "read", Some(r#"{"path":"/b.rs"}"#.into()));

        let live = compact_lines(&mut chat);
        assert_eq!(
            live,
            vec!["go", "▸ thinking", "read /a.rs", "read /b.rs"],
            "{live:?}"
        );

        chat.finish_tool("c2", None, false);
        chat.append_to_last_message("the answer");
        let lines = compact_lines(&mut chat);
        assert_eq!(
            lines,
            vec!["go", "▸ thinking", "read /a.rs", "read /b.rs", "the answer",],
            "{lines:?}"
        );
        assert_matches_fresh_render(&mut chat);
    }

    /// The counts head the answer instead of running into it: one blank row
    /// stands between a folded reply's summary and the prose under it. Wedged
    /// together, `▸ 工具调用 62 次 · 思考 39 次` read as the answer's first line
    /// (reported 2026-10-29 against a reply whose table followed right below).
    #[test]
    fn a_folded_reply_keeps_a_blank_row_above_its_answer() {
        let mut chat = new_chat();
        chat.render(W);
        set_messages(
            &mut chat,
            vec![
                ChatMessage::new("u1".into(), ChatRole::User, "go"),
                read_of("/a.rs"),
                read_of("/b.rs"),
                ChatMessage::new("a1".into(), ChatRole::Assistant, "the answer"),
            ],
        );
        let rows: Vec<String> = render_trimmed(&mut chat)
            .iter()
            .map(|line| strip(line).trim().to_string())
            .collect();
        let summary = rows
            .iter()
            .position(|row| row.contains("2 tool calls"))
            .unwrap_or_else(|| panic!("no summary row: {rows:?}"));
        let answer = rows
            .iter()
            .position(|row| row == "the answer")
            .unwrap_or_else(|| panic!("no answer row: {rows:?}"));
        assert_eq!(
            answer,
            summary + 2,
            "exactly one blank row between the counts and the answer: {rows:?}"
        );
        assert!(rows[summary + 1].is_empty(), "{rows:?}");
    }

    /// The collapsed reasoning marker heads the answer too, so it gets the same
    /// blank row: `▸ 思考` wedged against `大致可以分成六块…` read as that
    /// paragraph's opening line (reported 2026-10-30). The marker says the model
    /// reasoned — it is not the first line of what it reasoned to.
    ///
    /// `Detailed` is where the marker is drawn for a settled reply (at the
    /// default level the work folds into a count row, which has its own spacer
    /// above), and a live reply draws it at either level.
    #[test]
    fn a_collapsed_thought_marker_keeps_a_blank_row_above_its_answer() {
        let mut chat = new_chat();
        chat.render(W);
        chat.set_reply_detail(ReplyDetail::Detailed);
        set_messages(
            &mut chat,
            vec![
                ChatMessage::new("u1".into(), ChatRole::User, "go"),
                ChatMessage {
                    thinking: Some("weighed the options".into()),
                    ..ChatMessage::new("a1".into(), ChatRole::Assistant, "the answer")
                },
            ],
        );
        let rows: Vec<String> = render_trimmed(&mut chat)
            .iter()
            .map(|line| strip(line).trim().to_string())
            .collect();
        let marker = rows
            .iter()
            .position(|row| row.contains("thinking"))
            .unwrap_or_else(|| panic!("no marker row: {rows:?}"));
        let answer = rows
            .iter()
            .position(|row| row == "the answer")
            .unwrap_or_else(|| panic!("no answer row: {rows:?}"));
        assert_eq!(
            answer,
            marker + 2,
            "exactly one blank row between the marker and the answer: {rows:?}"
        );
        assert!(rows[marker + 1].is_empty(), "{rows:?}");
        // The reasoning text stays folded away: the spacer separates the marker
        // from the answer, it does not reveal the thought.
        assert!(
            !rows.iter().any(|row| row.contains("weighed the options")),
            "{rows:?}"
        );
    }

    /// The blank row is an answer's spacer, not a summary's: a fold that rides
    /// on a call (`▸ 1 tool call` over the call it stands above) keeps the call
    /// on the very next row — the band already separates them, and a blank
    /// there would just cost a row.
    #[test]
    fn a_fold_that_rides_on_a_call_keeps_it_on_the_next_row() {
        let mut chat = new_chat();
        chat.render(W);
        set_messages(
            &mut chat,
            vec![
                ChatMessage::new("u1".into(), ChatRole::User, "go"),
                read_of("/a.rs"),
                read_of("/b.rs"),
            ],
        );
        let rows: Vec<String> = render_trimmed(&mut chat)
            .iter()
            .map(|line| strip(line).trim().to_string())
            .collect();
        let summary = rows
            .iter()
            .position(|row| row.contains("tool call"))
            .unwrap_or_else(|| panic!("no summary row: {rows:?}"));
        assert!(
            rows.get(summary + 1)
                .is_some_and(|row| row.contains("/b.rs")),
            "the call the fold rides on follows the counts directly: {rows:?}"
        );
    }

    #[test]
    fn a_reply_without_prose_folds_onto_its_last_slice() {
        let mut chat = new_chat();
        chat.render(W);
        set_messages(
            &mut chat,
            vec![
                ChatMessage::new("u".into(), ChatRole::User, "look around"),
                assistant_with_thinking("a", "where to look", ""),
                read_of("/a.rs"),
            ],
        );
        let lines = compact_lines(&mut chat);
        // No answer to ride on, so the summary rides on the call the turn
        // stopped at; the reasoning is counted rather than drawn as a marker.
        assert!(lines.iter().any(|l| l == "▸ 1 thought"), "{lines:?}");
        assert!(!lines.iter().any(|l| l.contains("where to look")));
        assert!(lines.iter().any(|l| l.contains("read /a.rs")), "{lines:?}");
        assert!(
            !lines.iter().any(|l| l.contains("tool call")),
            "the call keeps its own row, so the count is the thought alone — {lines:?}"
        );
    }

    #[test]
    fn an_older_turn_stays_folded_while_the_newest_streams() {
        let mut chat = new_chat();
        chat.render(W);
        let mut messages = settled_turn_messages();
        messages.push(ChatMessage::new(
            "u2".into(),
            ChatRole::User,
            "now the tests",
        ));
        let mut live = assistant_with_thinking("a4", "running the suite", "Tests");
        live.pending = true;
        messages.push(live);
        set_messages(&mut chat, messages);

        let lines = compact_lines(&mut chat);
        // The newest reply is still streaming, so it is not folded: its call is
        // a row, and its reasoning is the collapsed marker.
        assert!(
            lines.iter().any(|l| l == "▸ 1 tool call · 1 thought"),
            "{lines:?}"
        );
        assert!(
            !lines.iter().any(|l| l.contains("running the suite")),
            "{lines:?}"
        );
        assert!(lines.iter().any(|l| l.contains("Tests")), "{lines:?}");
        assert_matches_fresh_render(&mut chat);
    }

    #[test]
    fn a_folded_answer_hides_its_own_reasoning_and_counts_it() {
        let mut chat = new_chat();
        chat.render(W);
        set_messages(
            &mut chat,
            vec![
                ChatMessage::new("u".into(), ChatRole::User, "hi"),
                assistant_with_thinking("a", "thinking it over", "Hello!"),
            ],
        );
        let lines = compact_lines(&mut chat);
        assert_eq!(lines, vec!["hi", "▸ 1 thought", "Hello!"], "{lines:?}");
    }

    #[test]
    fn the_toggle_switches_levels_and_restores_the_transcript() {
        let mut chat = new_chat();
        chat.render(W);
        set_messages(&mut chat, settled_turn_messages());
        let folded = chat.render_all(W).len();

        // One press: the reply keeps its process where it happened — the work
        // still folded, the prose back — and the desktop's `detailed` is what
        // that is. The press after it returns to the transcript the reader
        // started from.
        assert_eq!(chat.toggle_reply_detail(), ReplyDetail::Detailed);
        let expanded = chat.render_all(W);
        assert!(expanded.len() > folded, "detail adds the folded rows back");
        let detail = compact_lines(&mut chat);
        assert!(
            detail
                .iter()
                .any(|line| line.contains("read /src/parser.rs")),
            "detailed keeps the work it ran — {detail:?}"
        );
        assert!(
            detail
                .iter()
                .any(|line| line.contains("I read the parser.")),
            "…and keeps the notes it wrote — {detail:?}"
        );

        assert_eq!(chat.toggle_reply_detail(), ReplyDetail::Default);
        assert_eq!(chat.render_all(W).len(), folded);
    }

    /// A live run keeps its work visible through the incremental paths — every
    /// event the workbench feeds (thinking, a call, the answer) — and folds only
    /// when the reply settles.
    #[test]
    fn a_live_run_keeps_work_visible_until_it_settles() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("u".into(), ChatRole::User, "go"));
        chat.begin_reply_run();
        chat.start_thinking();
        chat.append_thinking_delta("weighing it");
        // A reasoning block with nothing else yet: the marker is the only row
        // saying what is happening, so it stays (and is not counted).
        assert_eq!(compact_lines(&mut chat), vec!["go", "▸ thinking…"]);

        chat.end_thinking();
        chat.add_tool_start("c1", "read", Some(r#"{"path":"/a.rs"}"#.into()));
        chat.append_tool_delta("c1", "body\n");
        let height_before_finish = chat.render_all(W).len();
        chat.finish_tool("c1", None, false);
        assert_eq!(
            chat.render_all(W).len(),
            height_before_finish,
            "finishing adjacent work must not collapse rows during the run"
        );
        chat.append_to_last_message("the answer");

        let live = compact_lines(&mut chat);
        assert_eq!(
            live,
            vec!["go", "▸ thinking", "read /a.rs", "the answer"],
            "{live:?}"
        );
        assert!(
            !live.iter().any(|line| line.contains("weighing it")),
            "{live:?}"
        );
        let live_height = chat.render_all(W).len();

        chat.mark_last_message_complete();
        chat.end_reply_run();
        let lines = compact_lines(&mut chat);
        assert!(
            chat.render_all(W).len() < live_height,
            "settling is the first point where the reply may collapse"
        );
        assert_eq!(
            lines,
            vec!["go", "▸ 1 tool call · 1 thought", "the answer"],
            "{lines:?}"
        );
        assert_matches_fresh_render(&mut chat);
    }

    // ── A run in flight never folds like a settled reply ─────────────────
    //
    // The reading level may only collapse a reply once the run writing it has
    // ended — the desktop's rule, and the reason its rows never flicker. The
    // terminal used to read "settled" off the *messages*, and a reply's slices
    // stop growing (`pending` clears) every time the model moves on to a call or
    // a fresh thought: between two events the last reply looked finished, the
    // notes it wrote folded onto its newest prose, and the next event unfolded
    // them again.

    /// The gap between two slices is not the end of the reply: prose written
    /// earlier stays on screen until the run itself ends.
    #[test]
    fn a_reply_does_not_fold_while_its_run_is_in_flight() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("u".into(), ChatRole::User, "go"));
        chat.begin_reply_run();
        chat.start_thinking();
        chat.append_thinking_delta("which file holds the fold?");
        chat.end_thinking();
        chat.add_tool_start("c1", "read", Some(r#"{"path":"/a.rs"}"#.into()));
        chat.finish_tool("c1", None, false);
        chat.append_to_last_message("Looking at a.rs.");
        chat.add_tool_start("c2", "read", Some(r#"{"path":"/b.rs"}"#.into()));
        chat.finish_tool("c2", None, false);
        chat.append_to_last_message("Both fold the same way.");

        // Every slice has stopped growing; the run has not.
        let live = compact_lines(&mut chat);
        assert!(
            live.iter().any(|line| line.contains("Looking at a.rs.")),
            "the note the reply wrote stays while the run is in flight — {live:?}"
        );
        assert!(
            live.iter()
                .any(|line| line.contains("Both fold the same way.")),
            "{live:?}"
        );

        // `agent_end`: now it is one reply with one answer, and the desk folds
        // it into the counts above that answer — once.
        chat.mark_last_message_complete();
        chat.end_reply_run();
        let settled = compact_lines(&mut chat);
        assert_eq!(
            settled,
            vec![
                "go",
                "▸ 2 tool calls · 1 thought",
                "Both fold the same way.",
            ],
            "{settled:?}"
        );
        assert_matches_fresh_render(&mut chat);
    }

    /// …and the panic the old shape read as: the moment a run ends is the only
    /// moment the transcript's height shrinks. Nothing before it does.
    #[test]
    fn a_reply_only_shrinks_when_its_run_ends() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("u".into(), ChatRole::User, "go"));
        chat.begin_reply_run();
        let mut rows = Vec::new();
        for step in 0..3 {
            let call = format!("c{step}");
            chat.add_tool_start(&call, "read", Some(r#"{"path":"/a.rs"}"#.into()));
            chat.finish_tool(&call, None, false);
            chat.append_to_last_message(&format!("step {step}"));
            rows.push(compact_lines(&mut chat).len());
        }
        // The prose the reply wrote so far only ever adds rows while it runs.
        for pair in rows.windows(2) {
            assert!(pair[1] >= pair[0], "the live reply shrank: {rows:?}");
        }
        let live = compact_lines(&mut chat);
        for step in 0..3 {
            assert!(
                live.iter()
                    .any(|line| line.contains(&format!("step {step}"))),
                "every note stays while the run is in flight — {live:?}"
            );
        }

        chat.mark_last_message_complete();
        chat.end_reply_run();
        let settled = compact_lines(&mut chat);
        assert!(
            settled.len() < live.len(),
            "ending the run is what folds the reply — {settled:?} vs {live:?}"
        );
        assert_eq!(
            settled,
            vec!["go", "▸ 3 tool calls", "step 2"],
            "{settled:?}"
        );
        assert_matches_fresh_render(&mut chat);
    }

    /// A thought that shares its slice with prose keeps its marker while the
    /// reply runs, then joins the settled reply's total.
    #[test]
    fn a_thought_written_with_prose_keeps_its_marker_until_settling() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("u".into(), ChatRole::User, "go"));
        chat.begin_reply_run();
        chat.start_thinking();
        chat.append_thinking_delta("which file holds the fold?");
        chat.end_thinking();
        chat.add_tool_start("c1", "read", Some(r#"{"path":"/a.rs"}"#.into()));
        chat.finish_tool("c1", None, false);
        // The model thinks again — a new slice, because the tail is the call —
        // and then writes into that same slice.
        chat.start_thinking();
        chat.append_thinking_delta("now the second file");
        chat.end_thinking();
        chat.append_to_last_message("Second file next.");

        let live = compact_lines(&mut chat);
        assert_eq!(
            live,
            vec![
                "go",
                "▸ thinking",
                "read /a.rs",
                "▸ thinking",
                "Second file next.",
            ],
            "both thoughts keep their rows until settling — {live:?}"
        );

        // The same counts once the run ends and the reply folds.
        chat.mark_last_message_complete();
        chat.end_reply_run();
        let settled = compact_lines(&mut chat);
        assert_eq!(
            settled,
            vec!["go", "▸ 1 tool call · 2 thoughts", "Second file next."],
            "{settled:?}"
        );
    }

    /// A thought behind prose keeps its own marker: nothing adjacent folds it,
    /// and the desktop draws a lone reasoning step the same way. The count rows
    /// add up over the work they stand for — a marker is a row, not a count.
    #[test]
    fn a_thought_behind_prose_keeps_its_marker() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("u".into(), ChatRole::User, "go"));
        chat.begin_reply_run();
        chat.start_thinking();
        chat.append_thinking_delta("which file holds the fold?");
        chat.end_thinking();
        chat.add_tool_start("c1", "read", Some(r#"{"path":"/a.rs"}"#.into()));
        chat.finish_tool("c1", None, false);
        chat.append_to_last_message("Looking at a.rs.");
        chat.start_thinking();
        chat.append_thinking_delta("now the second file");
        chat.end_thinking();
        chat.append_to_last_message("Second file next.");

        let live = compact_lines(&mut chat);
        assert_eq!(
            live,
            vec![
                "go",
                "▸ thinking",
                "read /a.rs",
                "Looking at a.rs.",
                "▸ thinking",
                "Second file next.",
            ],
            "{live:?}"
        );

        // Settling is where every thought reaches the counts: one row above the
        // answer, with both of them in it.
        chat.mark_last_message_complete();
        chat.end_reply_run();
        let settled = compact_lines(&mut chat);
        assert_eq!(
            settled,
            vec!["go", "▸ 1 tool call · 2 thoughts", "Second file next."],
            "{settled:?}"
        );
    }

    /// The detailed level: every slice keeps its own row, in the order it
    /// happened — no summary row, and still no reasoning text.
    #[test]
    fn the_detailed_level_keeps_every_reply_in_place() {
        let mut chat = new_chat();
        chat.render(W);
        chat.set_reply_detail(ReplyDetail::Detailed);
        set_messages(&mut chat, settled_turn_messages());

        let lines = compact_lines(&mut chat);
        assert_eq!(
            lines,
            vec![
                "fix the parser",
                "▸ thinking",
                "read /src/parser.rs",
                "I read the parser.",
                "Fixed the off-by-one.",
            ],
            "{lines:?}"
        );
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("weighing the options")),
            "reasoning text is still behind its marker — {lines:?}"
        );
        assert_matches_fresh_render(&mut chat);
    }

    /// What the process rows say in each language: Chinese words the counts
    /// with 次 and puts the noun first, English pluralizes and puts the number
    /// first — so the row is generated from templates, never assembled here.
    #[test]
    fn the_process_rows_follow_the_interface_language() {
        let mut chat = new_chat();
        chat.render(W);
        chat.set_fold_words(FoldWords {
            tool_call: "工具调用 {} 次",
            tool_calls: "工具调用 {} 次",
            thought: "思考 {} 次",
            thoughts: "思考 {} 次",
            thinking: "思考",
            process: "过程",
            copy: "复制",
            fork: "分支",
        });
        chat.add_message(ChatMessage::new("u".into(), ChatRole::User, "开始"));
        chat.begin_reply_run();
        chat.start_thinking();
        chat.append_thinking_delta("先看看");
        assert_eq!(compact_lines(&mut chat), vec!["开始", "▸ 思考…"]);

        chat.end_thinking();
        chat.add_tool_start("c1", "read", Some(r#"{"path":"/a.rs"}"#.into()));
        chat.finish_tool("c1", None, false);
        chat.add_tool_start("c2", "read", Some(r#"{"path":"/b.rs"}"#.into()));
        chat.finish_tool("c2", None, false);
        chat.append_to_last_message("看完了");
        assert_eq!(
            compact_lines(&mut chat),
            vec!["开始", "▸ 思考", "read /a.rs", "read /b.rs", "看完了"]
        );
        chat.mark_last_message_complete();
        chat.end_reply_run();

        let lines = compact_lines(&mut chat);
        assert_eq!(
            lines,
            vec!["开始", "▸ 工具调用 2 次 · 思考 1 次", "看完了"],
            "{lines:?}"
        );
        assert_matches_fresh_render(&mut chat);

        // The English defaults count the same work and word it their own way.
        let mut english = new_chat();
        english.render(W);
        english.add_message(ChatMessage::new("u".into(), ChatRole::User, "go"));
        english.begin_reply_run();
        english.start_thinking();
        english.append_thinking_delta("looking");
        assert_eq!(compact_lines(&mut english), vec!["go", "▸ thinking…"]);
    }

    /// The count row is the faintest ink the chrome has — one step under the
    /// body's dim text — and upright: it is a note about work that already
    /// happened, and it must recede behind the answer under it.
    #[test]
    fn the_count_row_sits_behind_the_body_ink() {
        let mut chat = new_chat();
        chat.render(W);
        set_messages(&mut chat, settled_turn_messages());
        let row = chat
            .render_all(W)
            .into_iter()
            .find(|line| strip(line).contains("1 tool call"))
            .unwrap_or_else(|| panic!("no count row"));
        let faint = Chrome::from_theme(&DARK_THEME).muted;
        assert!(
            row.contains(&format!("38;5;{faint}m")),
            "the row is drawn in the chrome's faint ink ({faint}); got {row:?}"
        );
        // The ink has to cover the *whole* row, not just its first cell: a
        // reset in the middle drops the colour and the frame's row painter
        // re-arms the body ink after it, which is how this row arrived bright
        // white apart from its marker.
        assert_eq!(
            row.matches("38;5;").count(),
            1,
            "one colour, stated once, for the whole row — {row:?}"
        );
        assert_eq!(
            row.matches("\x1b[0m").count() + row.matches("\x1b[m").count(),
            1,
            "…and one reset, at the end — {row:?}"
        );
        assert!(
            row.contains("\x1b[22m"),
            "the marker's dim is switched off, not reset away — {row:?}"
        );
        assert!(
            faint < DARK_THEME.dim as u8,
            "…which is darker than the body's dim text"
        );
        assert!(
            !row.contains("\x1b[3m") && !row.contains("\x1b[;3m"),
            "upright, not italic: {row:?}"
        );
    }

    /// One turn as the workbench builds it from the event stream: the empty
    /// assistant a run opens with, its reasoning, a call and its result, and
    /// the answer the turn settles on.
    fn live_turn(chat: &mut ChatArea, turn: &str, prompt: &str, path: &str, answer: &str) {
        let call = format!("call-{turn}");
        chat.add_message(ChatMessage::new(
            format!("u-{turn}"),
            ChatRole::User,
            prompt,
        ));
        chat.add_message(ChatMessage::new(
            format!("a0-{turn}"),
            ChatRole::Assistant,
            "",
        ));
        chat.start_thinking();
        chat.append_thinking_delta(&format!("reasoning about {turn}"));
        chat.end_thinking();
        chat.add_tool_start(&call, "read", Some(format!(r#"{{"path":"{path}"}}"#)));
        chat.append_tool_delta(&call, "body\n");
        chat.finish_tool(&call, None, false);
        chat.append_to_last_message(answer);
        chat.mark_last_message_complete();
    }

    /// A page of an older conversation in the shape the session store hands
    /// back (see `tui/src/session_rows.rs`): the `tool_call` step that renders
    /// nothing, the call's result, and the answer.
    fn stored_page() -> Vec<ChatMessage> {
        let rows = serde_json::json!([
            {"id": "h-u", "role": "user", "blocks": [{"kind": "text", "text": "older prompt"}]},
            {"id": "h-a0", "role": "assistant", "blocks": [{"kind": "reasoning", "text": "older reasoning"}]},
            {"id": "h-step", "role": "assistant", "blocks": [{"kind": "tool_call", "toolCallId": "c-old", "name": "read", "arguments": "{\"path\":\"/src/old.rs\"}"}]},
            {"id": "h-t", "role": "tool", "blocks": [{"kind": "tool_result", "toolCallId": "c-old", "text": "old body"}]},
            {"id": "h-a1", "role": "assistant", "blocks": [{"kind": "text", "text": "Older answer."}]}
        ]);
        crate::session_rows::rows_to_messages(rows.as_array().unwrap())
    }

    /// The reading level applies to the *whole* transcript, not to whatever
    /// arrived most recently: a page of history prepended from the session
    /// store folds with the live turns already on screen, and `/reply-detail`
    /// at any point leaves every settled reply as one summary row.
    #[test]
    fn the_reading_level_folds_history_loaded_from_the_store() {
        const FOLDED_TURN: &str = "▸ 1 tool call · 1 thought";
        fn assert_folded(chat: &mut ChatArea, turns: usize, stage: &str) {
            let lines = compact_lines(chat);
            assert_eq!(
                lines.iter().filter(|line| **line == FOLDED_TURN).count(),
                turns,
                "{stage}: every settled reply needs one summary row — {lines:?}"
            );
            for gone in ["read /src/a.rs", "read /src/b.rs", "read /src/old.rs"] {
                assert!(
                    !lines.iter().any(|line| line.contains(gone)),
                    "{stage}: {gone} must be folded — {lines:?}"
                );
            }
            for kept in ["First done.", "Second done.", "Older answer."] {
                assert!(
                    lines.iter().any(|line| line.contains(kept)),
                    "{stage}: the answer itself stays — {lines:?}"
                );
            }
        }

        let mut chat = new_chat();
        chat.render(W);
        live_turn(&mut chat, "1", "first prompt", "/src/a.rs", "First done.");
        live_turn(&mut chat, "2", "second prompt", "/src/b.rs", "Second done.");
        // An older page lands above the live turns, the way the workbench's
        // `load_older` prepends it.
        chat.prepend_messages(stored_page());
        assert_folded(&mut chat, 3, "as loaded");

        // `/reply-detail` round-trips: `default` has to come back from a
        // transcript that was rendered unfolded. The cycle is default ↔ detailed.
        assert_eq!(chat.toggle_reply_detail(), ReplyDetail::Detailed);
        let open = compact_lines(&mut chat);
        assert!(
            open.iter().any(|line| line.contains("read /src/old.rs")),
            "detail keeps the call — {open:?}"
        );
        assert_eq!(chat.toggle_reply_detail(), ReplyDetail::Default);
        assert_folded(&mut chat, 3, "after a toggle round-trip");

        // …and history arriving while the level is already folded.
        chat.prepend_messages(stored_page());
        assert_folded(&mut chat, 4, "after a later prepend");
        assert_matches_fresh_render(&mut chat);
    }

    /// One settled turn, built the way `live.rs` builds it from the event
    /// stream: a user row, the assistant row a run opens with, its reasoning, a
    /// call and its result, then the answer.
    fn streamed_turn(chat: &mut ChatArea, turn: &str, path: &str) {
        let call = format!("call-{turn}");
        chat.add_message(ChatMessage::new(
            format!("u-{turn}"),
            ChatRole::User,
            &format!("prompt {turn}"),
        ));
        chat.add_message(ChatMessage::new(
            format!("a0-{turn}"),
            ChatRole::Assistant,
            "",
        ));
        chat.start_thinking();
        chat.append_thinking_delta(&format!("thinking about {turn}"));
        chat.end_thinking();
        chat.add_tool_start(&call, "read", Some(format!(r#"{{"path":"{path}"}}"#)));
        chat.append_tool_delta(&call, "body\n");
        chat.finish_tool(&call, None, false);
        chat.append_to_last_message(&format!("answer {turn}"));
        chat.mark_last_message_complete();
    }

    /// A reply that never produced prose — the shape an **interrupted** turn
    /// leaves behind: the run opened an assistant row, reasoned, maybe ran a
    /// tool, and then the answer never arrived.
    fn interrupted_turn(chat: &mut ChatArea, turn: &str, path: &str) {
        let call = format!("call-{turn}");
        chat.add_message(ChatMessage::new(
            format!("u-{turn}"),
            ChatRole::User,
            &format!("prompt {turn}"),
        ));
        chat.add_message(ChatMessage::new(
            format!("a0-{turn}"),
            ChatRole::Assistant,
            "",
        ));
        chat.start_thinking();
        chat.append_thinking_delta(&format!("thinking about {turn}"));
        chat.end_thinking();
        chat.add_tool_start(&call, "read", Some(format!(r#"{{"path":"{path}"}}"#)));
        chat.append_tool_delta(&call, "body\n");
        chat.finish_tool(&call, None, false);
        // `agent_end` still settles the message — `live.rs` calls this
        // unconditionally — but no answer text ever came.
        chat.mark_last_message_complete();
    }

    fn folded_rows(chat: &mut ChatArea) -> usize {
        const FOLDED: &str = "▸ 1 tool call · 1 thought";
        compact_lines(chat)
            .iter()
            .filter(|line| line.as_str() == FOLDED)
            .count()
    }

    /// The incremental paths, which the workspace-level test never reaches:
    /// `sync_compact_layout` / `fold_layout_extends` / `rerender_message` /
    /// `append_last_message`. The width is established once with `render`, then
    /// every later step must ride the splice path — asking for a new width, or
    /// a dirty flag, would rerender everything and hide the bug.
    #[test]
    fn incremental_streaming_keeps_every_turn_folded() {
        let mut chat = new_chat();
        chat.render(W);
        assert_eq!(chat.reply_detail(), ReplyDetail::Default);

        for turn in 0..5 {
            streamed_turn(&mut chat, &format!("{turn}"), &format!("/src/{turn}.rs"));
            let lines = compact_lines(&mut chat);
            assert_eq!(
                folded_rows(&mut chat),
                turn + 1,
                "turn {turn}: every settled reply folded — {lines:?}"
            );
            let gone = format!("read /src/{turn}.rs");
            assert!(
                !lines.iter().any(|line| line.contains(&gone)),
                "turn {turn}: {gone} still painted — {lines:?}"
            );
            assert!(
                !lines.iter().any(|line| line.contains("thinking about")),
                "turn {turn}: reasoning still painted — {lines:?}"
            );
        }
        assert_matches_fresh_render(&mut chat);
    }

    /// …and the same incremental stream, with the level flipped *after* it.
    #[test]
    fn toggling_after_incremental_streaming_folds_everything() {
        let mut chat = detailed_chat();
        chat.render(W);
        for turn in 0..4 {
            streamed_turn(&mut chat, &format!("{turn}"), &format!("/src/{turn}.rs"));
        }
        // Detailed: every slice is on screen.
        let open = compact_lines(&mut chat);
        assert!(
            open.iter().any(|line| line.contains("read /src/2.rs")),
            "detail keeps the call — {open:?}"
        );

        // Two presses of the cycle: the work folds onto the answer, then the
        // prose comes back with it folded in place.
        assert_eq!(chat.toggle_reply_detail(), ReplyDetail::Default);
        let lines = compact_lines(&mut chat);
        assert_eq!(folded_rows(&mut chat), 4, "after the toggle — {lines:?}");
        for turn in 0..4 {
            let gone = format!("read /src/{turn}.rs");
            assert!(
                !lines.iter().any(|line| line.contains(&gone)),
                "{gone} still painted after the toggle — {lines:?}"
            );
        }
        assert_eq!(chat.toggle_reply_detail(), ReplyDetail::Detailed);
        assert_matches_fresh_render(&mut chat);
    }

    /// The transcript's visible rows at an arbitrary width.
    ///
    /// `compact_lines` is pinned to `W`; using it inside a width loop measures
    /// the same width every iteration and the loop proves nothing.
    fn compact_lines_at(chat: &mut ChatArea, width: usize) -> Vec<String> {
        let mut lines = chat.render_all(width);
        while lines.last().is_some_and(|line| line.is_empty()) {
            lines.pop();
        }
        let lines = no_controls(chat, lines);
        lines
            .iter()
            .map(|line| strip(line).trim().to_string())
            .filter(|line| !line.is_empty())
            .collect()
    }

    /// A width change re-wraps every row, so the cached line ranges have to be
    /// rebuilt; a fold whose summary row was measured at another width must not
    /// survive as a stale offset.
    #[test]
    fn resizing_while_folded_keeps_every_turn_folded() {
        for width in [W, 64, 40, 100] {
            let mut chat = new_chat();
            chat.render(W);
            for turn in 0..4 {
                streamed_turn(&mut chat, &format!("{turn}"), &format!("/src/{turn}.rs"));
            }
            let lines = compact_lines_at(&mut chat, width);
            assert_eq!(
                lines
                    .iter()
                    .filter(|line| line.starts_with('\u{25b8}'))
                    .count(),
                4,
                "width {width}: every reply keeps its summary — {lines:?}"
            );
            for turn in 0..4 {
                let gone = format!("read /src/{turn}.rs");
                assert!(
                    !lines.iter().any(|line| line.contains(&gone)),
                    "width {width}: {gone} painted — {lines:?}"
                );
            }
            assert_matches_fresh_render_at(&mut chat, width);
        }
    }

    // ── The interrupted turn: it folds to its counts too ───────────────────
    //
    // A turn that never produced an answer — an interrupted or cancelled run,
    // whose `agent_end` settles the messages without carrying text — has no
    // answer to isolate, so the fold rides on its newest slice instead and the
    // work above it becomes a count. That is the same row a running reply
    // shows, which is what keeps the transcript from unfolding at the moment a
    // run ends (the report behind this: the terminal was the one client that
    // painted every call while the desktop showed its counts (the numbers the
    // wrench and brain glyphs now carry).
    // The call that ends the turn keeps its row — one row is what tells a reader
    // the work stopped there — and `/reply-detail` still replays every slice.

    /// A settled turn with no prose folds onto its last slice, live path.
    #[test]
    fn an_interrupted_turn_folds_onto_its_last_slice() {
        let mut chat = new_chat();
        chat.render(W);
        streamed_turn(&mut chat, "done", "/src/done.rs");
        interrupted_turn(&mut chat, "cut", "/src/cut.rs");

        let lines = compact_lines(&mut chat);
        // Both turns read as counts: the answered one onto its answer, the
        // unanswered one onto the call it stopped at.
        assert_eq!(folded_rows(&mut chat), 1, "{lines:?}");
        assert!(
            lines.iter().any(|l| l.contains("▸ 1 thought")),
            "the unanswered turn counts its reasoning: {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains("read /src/cut.rs")),
            "the slice the turn stopped at keeps its row: {lines:?}"
        );
        assert!(!lines.iter().any(|l| l.contains("thinking about cut")));

        // `/reply-detail` replays the whole thing: every slice, each with its
        // own row —
        // and the reasoning still behind its marker, as at every level.
        chat.set_reply_detail(ReplyDetail::Detailed);
        let open = compact_lines(&mut chat);
        assert!(
            open.iter().any(|l| l.contains("read /src/cut.rs")),
            "{open:?}"
        );
        assert!(!open.iter().any(|l| l.contains("thinking about cut")));
        // Opening the marker is how the reasoning text is reached.
        open_all_rows(&mut chat);
        let opened = compact_lines(&mut chat);
        assert!(
            opened.iter().any(|l| l.contains("thinking about cut")),
            "{opened:?}"
        );
    }

    /// The same shape arriving from the session store.
    #[test]
    fn an_interrupted_stored_page_folds_onto_its_last_slice() {
        let rows = serde_json::json!([
            {"id": "c-u", "role": "user", "blocks": [{"kind": "text", "text": "cancelled work"}]},
            {"id": "c-a0", "role": "assistant", "blocks": [{"kind": "reasoning", "text": "stored reasoning"}]},
            {"id": "c-step", "role": "assistant", "blocks": [{"kind": "tool_call", "toolCallId": "cc-1", "name": "read", "arguments": "{\"path\":\"/src/gone.rs\"}"}]},
            {"id": "c-t", "role": "tool", "blocks": [{"kind": "tool_result", "toolCallId": "cc-1", "text": "partial body"}]}
        ]);
        let mut chat = new_chat();
        chat.render(W);
        chat.prepend_messages(crate::session_rows::rows_to_messages(
            rows.as_array().unwrap(),
        ));
        let lines = compact_lines(&mut chat);
        assert!(
            lines.iter().any(|line| line.contains("/src/gone.rs")),
            "the slice the stored turn stopped at keeps its row — {lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("▸ 1 thought")),
            "its stored reasoning is a count — {lines:?}"
        );
    }

    /// The premise those two rest on: the store always settles its tool rows, so
    /// a stored page never leaves a turn unfolded for *that* reason.
    /// `tool_status` is `Complete` or `Error`, never `Running`.
    #[test]
    fn a_stored_tool_row_is_never_running() {
        let rows = serde_json::json!([
            {"id": "s-u", "role": "user", "blocks": [{"kind": "text", "text": "ask"}]},
            {"id": "s-step", "role": "assistant", "blocks": [{"kind": "tool_call", "toolCallId": "c-1", "name": "read", "arguments": "{}"}]},
            {"id": "s-ok", "role": "tool", "blocks": [{"kind": "tool_result", "toolCallId": "c-1", "text": "fine"}]},
            {"id": "s-bad", "role": "tool", "blocks": [{"kind": "tool_result", "toolCallId": "c-2", "text": "broke", "isError": true}]}
        ]);
        let messages = crate::session_rows::rows_to_messages(rows.as_array().unwrap());
        for message in &messages {
            if message.role == ChatRole::Tool {
                assert!(
                    !matches!(message.tool_status, Some(ToolStatus::Running)),
                    "a stored tool row must be settled, saw {:?}",
                    message.tool_status
                );
                assert!(!message.pending, "a stored row is never pending");
            }
        }
    }

    /// A settled reply is one unit: the work *after* its last prose folds with
    /// the work before it. A run that was interrupted (or that ended on a tool
    /// call) leaves calls below its last answer, and folding only the stretch
    /// above that answer is what made the folded level read as "only part of the
    /// transcript collapsed" — scrolling up, those calls were the first rows in
    /// sight. Measured on a real interrupted session (2026-10-26): 89 of a
    /// reply's rows stayed while the other 296 folded.
    #[test]
    fn a_settled_reply_folds_the_work_that_follows_its_answer() {
        let mut chat = new_chat();
        chat.render(W);
        set_messages(
            &mut chat,
            vec![
                ChatMessage::new("u1".into(), ChatRole::User, "fix the parser"),
                read_of("/src/parser.rs"),
                ChatMessage::new("a1".into(), ChatRole::Assistant, "Fixed the off-by-one."),
                // …and the run kept working, without producing any further prose.
                read_of("/src/lib.rs"),
                ChatMessage::new("a2".into(), ChatRole::Assistant, ""),
                read_of("/tests/parser.rs"),
            ],
        );
        assert_eq!(
            compact_lines(&mut chat),
            vec!["fix the parser", "▸ 3 tool calls", "Fixed the off-by-one.",]
        );
        assert_matches_fresh_render(&mut chat);

        // The detailed level still keeps every row, including the trailing work.
        chat.set_reply_detail(ReplyDetail::Detailed);
        let open = compact_lines(&mut chat);
        for path in ["/src/parser.rs", "/src/lib.rs", "/tests/parser.rs"] {
            assert!(open.iter().any(|line| line.contains(path)), "{open:?}");
        }
    }

    /// Just an answer with nothing around it folds nothing — there is no work to
    /// stand in for, and a summary row counting zero calls would be noise.
    #[test]
    fn an_answer_with_no_work_around_it_gains_no_summary_row() {
        let mut chat = new_chat();
        chat.render(W);
        set_messages(
            &mut chat,
            vec![
                ChatMessage::new("u1".into(), ChatRole::User, "hello"),
                ChatMessage::new("a1".into(), ChatRole::Assistant, "hi"),
            ],
        );
        assert_eq!(compact_lines(&mut chat), vec!["hello", "hi"]);
    }

    /// The level a transcript opens at is the folded one — the state
    /// `/reply-detail` produces at its default — so a fresh terminal and a
    /// restarted one look the same. There
    /// is no stored level to disagree with it any more (see `ReplyDetail`).
    #[test]
    fn a_new_transcript_opens_folded() {
        assert_eq!(ReplyDetail::default(), ReplyDetail::Default);
        let chat = ChatArea::new(W, None);
        assert_eq!(chat.reply_detail(), ReplyDetail::Default);
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

    /// A terminal too narrow for the layout's minimum — one content column
    /// plus the one-column gutter — must clamp the width, not underflow it.
    /// An underflowing `width - 2` would pad every row out to ~`usize::MAX`
    /// cells (`apply_background_to_line` pads with `" ".repeat(padding)`), and
    /// an underflowing viewport index would slice out of range.
    ///
    /// The observable bound is `width.max(3)`: at or below two columns the
    /// layout bottoms out at gutter + one content column, and a wide (CJK)
    /// glyph fills both of those columns, so even a 0-column terminal may emit
    /// a 3-cell row. From three columns up each row is padded to exactly
    /// `width`. `usize::MAX` itself is not exercised: a genuine underflow there
    /// would try to allocate that many spaces, so the largest sane width is
    /// the useful end of the table.
    #[test]
    fn tiny_terminal_width_does_not_underflow() {
        let mut clamped_rows: Vec<Vec<String>> = Vec::new();
        for width in [0usize, 1, 2, 3, 4, 40, 1000] {
            let mut chat = ChatArea::new(width, None);
            // Every slice on screen, so the width contract is exercised on the
            // thinking row too; the reading-level summary row is covered below.
            chat.set_reply_detail(ReplyDetail::Detailed);
            chat.add_message(ChatMessage::new("u".into(), ChatRole::User, "hello world"));
            let mut assistant = ChatMessage::new("a".into(), ChatRole::Assistant, "答案 ok");
            assistant.thinking = Some("thinking".into());
            chat.add_message(assistant);
            chat.render_all(width);
            open_all_rows(&mut chat);

            let all = chat.render_all(width);
            let view = chat.render(width);

            assert!(!all.is_empty(), "width {width}: no rows at all");
            assert!(
                !view.is_empty(),
                "width {width}: the viewport rendered no rows"
            );
            // The viewport is a window onto the full transcript: an underflowed
            // `viewport_top` would slice out of range and return nothing, or
            // rows that never existed.
            assert!(
                all.len() >= view.len()
                    && all
                        .windows(view.len())
                        .any(|window| window == view.as_slice()),
                "width {width}: viewport {view:?} is not a window of the transcript"
            );

            for line in &all {
                let plain = crate::utils::strip_ansi_codes(line);
                let cols = visible_width_of(&plain);
                assert!(
                    cols <= width.max(3),
                    "width {width}: row is {cols} cells wide, expected <= {}: {plain:?}",
                    width.max(3)
                );
                assert!(
                    plain.is_empty() || plain.starts_with(' '),
                    "width {width}: row lost its 1-column gutter: {plain:?}"
                );
            }

            // Clamping wraps the text, it never drops or reorders it: squashing
            // the rows back together must recover every message.
            let squashed = all
                .iter()
                .map(|line| crate::utils::strip_ansi_codes(line))
                .collect::<Vec<_>>()
                .join("");
            let squashed: String = squashed.split_whitespace().collect();
            for needle in ["helloworld", "thinking", "答案ok"] {
                assert!(
                    squashed.contains(needle),
                    "width {width}: {needle:?} missing from the rendered rows: {squashed:?}"
                );
            }

            if width <= 2 {
                clamped_rows.push(all);
            }
        }

        // Below the 3-column floor every width lays out identically, because
        // the content column clamps to 1 — that is what shows the width is
        // clamped in the maths rather than silently propagated (or truncated).
        assert_eq!(
            clamped_rows[0], clamped_rows[1],
            "0- and 1-column layouts must clamp to the same rows"
        );
        assert_eq!(
            clamped_rows[1], clamped_rows[2],
            "1- and 2-column layouts must clamp to the same rows"
        );
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

    /// The cost the driver loop must not pay: a second width within one
    /// keystroke or one streamed token throws away the rendered rows and
    /// re-wraps (and re-parses) every message. `full_renders` is the meter.
    #[test]
    fn a_probe_width_is_a_full_rebuild_and_the_frame_width_is_free() {
        let mut chat = new_chat();
        for i in 0..8 {
            chat.add_message(ChatMessage::new(
                format!("u{i}"),
                ChatRole::User,
                "question",
            ));
            chat.add_message(ChatMessage::new(
                format!("a{i}"),
                ChatRole::Assistant,
                "an answer with **markup** and a `code` span",
            ));
        }

        // The session's first frame renders once.
        chat.render_all(W);
        let first = chat.full_renders();

        // Steady state: the same width is a cache hit, every time.
        for _ in 0..20 {
            chat.render_all(W);
        }
        assert_eq!(
            chat.full_renders(),
            first,
            "a frame at the last frame's width must not rebuild the transcript"
        );

        // A frame at any other width rebuilds — which is why a caller must not
        // probe one. The driver used to do this twice per event.
        chat.render_all(1);
        assert_eq!(chat.full_renders(), first + 1);
    }

    /// Streaming at the frame width stays incremental: the growing message is
    /// re-rendered, the rest of the transcript is not.
    #[test]
    fn a_streamed_delta_at_one_width_does_not_rebuild_the_transcript() {
        let mut chat = new_chat();
        for i in 0..8 {
            chat.add_message(ChatMessage::new(
                format!("u{i}"),
                ChatRole::User,
                "question",
            ));
            chat.add_message(ChatMessage::new(
                format!("a{i}"),
                ChatRole::Assistant,
                "an answer with **markup** and a `code` span",
            ));
        }
        chat.render_all(W);
        // The live turn starts the way a run does: the reader's prompt, then the
        // assistant row its deltas land in. Without the prompt the new message
        // would be the *tail* of the previous reply, and the first delta would
        // legitimately re-fold that reply — a rebuild the steady state never sees.
        chat.add_message(ChatMessage::new("u-live".into(), ChatRole::User, "next"));
        chat.add_message(ChatMessage::new("live".into(), ChatRole::Assistant, ""));
        let before = chat.full_renders();

        for _ in 0..40 {
            chat.append_to_last_message("streamed ");
            chat.render_all(W);
        }
        assert_eq!(
            chat.full_renders(),
            before,
            "a delta must splice the streaming message, not rebuild everything"
        );
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
        let mut chat = detailed_chat();
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
        let mut chat = {
            let mut chat = ChatArea::new(60, None);
            chat.set_reply_detail(ReplyDetail::Detailed);
            chat
        };
        chat.add_message(ChatMessage {
            id: "1".into(),
            role: ChatRole::Assistant,
            content: "answer".into(),
            thinking: Some("let me check `some code` and **bold text** first".into()),
            ..ChatMessage::new(String::new(), ChatRole::Assistant, "")
        });
        chat.render(60); // establish width
        open_all_rows(&mut chat);
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
        let mut chat = {
            let mut chat = ChatArea::new(60, None);
            chat.set_reply_detail(ReplyDetail::Detailed);
            chat
        };
        chat.add_message(ChatMessage {
            id: "1".into(),
            role: ChatRole::Assistant,
            content: "answer".into(),
            thinking: Some("> quoted thought\n\n```\ncode line\n```\n\n".into()),
            ..ChatMessage::new(String::new(), ChatRole::Assistant, "")
        });
        chat.render(60);
        open_all_rows(&mut chat);
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
        let mut chat = {
            let mut chat = ChatArea::new(60, None);
            chat.set_reply_detail(ReplyDetail::Detailed);
            chat
        };
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
        open_all_rows(&mut chat);
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
        // The reasoning text is the full-process level's (`/reply-detail`
        // Detailed); the default starts it collapsed.
        chat.set_reply_detail(ReplyDetail::Detailed);
        chat.add_message(ChatMessage::new("1".into(), ChatRole::Assistant, ""));
        chat.start_thinking();
        chat.append_thinking_delta("first block");
        chat.end_thinking();
        chat.start_thinking();
        chat.append_thinking_delta("second block");
        chat.end_thinking();
        chat.render(60);
        open_all_rows(&mut chat);
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

    /// The same, with the call's own row open — the body visible.
    fn expanded_tool_lines(
        chat: &mut ChatArea,
        tool: &str,
        args: &str,
        output: &str,
    ) -> Vec<String> {
        let _ = tool_lines(chat, tool, args, output);
        open_all_rows(chat);
        render_trimmed(chat)
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

        open_all_rows(&mut chat);
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
    }

    #[test]
    fn a_unified_diff_body_is_parsed_and_summarised() {
        let mut chat = new_chat();
        let lines = tool_lines(&mut chat, "edit", r#"{"path":"src/a.rs"}"#, diff_output());
        // The tool row carries the +N -M badge; the body itself waits for the
        // row to open.
        assert_eq!(lines.len(), 1, "row only: {lines:?}");
        let row = strip(&lines[0]);
        assert!(row.contains("+2 -1"), "{row:?}");

        // Opened: every add/remove row is rendered and coloured.
        open_all_rows(&mut chat);
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
    fn a_long_diff_is_collapsed_and_opens_with_its_row() {
        let mut chat = new_chat();
        let mut diff = String::from("--- a/f\n+++ b/f\n@@ -1,60 +1,60 @@\n");
        for i in 0..60 {
            diff.push_str(&format!("-old {i}\n+new {i}\n"));
        }
        let lines = tool_lines(&mut chat, "write", r#"{"path":"f"}"#, &diff);
        assert_eq!(lines.len(), 1, "collapsed diff is one row: {lines:?}");
        // Opened, the whole (123-line) diff is shown — no marker left.
        open_all_rows(&mut chat);
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
        open_all_rows(&mut chat);
        let expanded = render_trimmed(&mut chat);
        assert_eq!(expanded.len(), 1 + EXPANDED_TOOL_OUTPUT_ROWS + 1);
        let marker = strip(expanded.last().unwrap());
        assert!(marker.contains("truncated"), "{marker:?}");
        assert!(marker.contains("more lines"), "{marker:?}");
    }

    // ─── An `edit` call's body ─────────────────────────────────────────

    /// The row of one `edit` call and the body rows opening it adds.
    fn opened_edit_body(args: &str, streamed: &str) -> (String, Vec<String>) {
        let mut chat = new_chat();
        tool_lines(&mut chat, "edit", args, streamed);
        let row = strip(&render_trimmed(&mut chat)[0]);
        open_all_rows(&mut chat);
        let mut body: Vec<String> = render_trimmed(&mut chat)
            .iter()
            .map(|line| strip(line))
            .collect();
        // The call's own row comes first; the body is what opening added.
        body.remove(0);
        (row, body)
    }

    /// `oldText`/`newText` open as the patch they are — not as the argument
    /// JSON the model streamed, which is what the body used to hold.
    #[test]
    fn an_edit_call_opens_as_its_patch() {
        let args =
            r#"{"path":"src/a.rs","oldText":"one\ntwo\nthree","newText":"one\ntwo b\nthree"}"#;
        let (row, body) = opened_edit_body(args, args);

        // The row counts the change, so its size is visible before opening.
        assert!(row.contains("+1 -1"), "{row:?}");
        assert!(body.iter().any(|line| line.contains("-two")), "{body:?}");
        assert!(body.iter().any(|line| line.contains("+two b")), "{body:?}");
        // The unchanged lines around it stay, without a `-`/`+` of their own.
        assert!(
            body.iter().any(|line| line.trim_end().ends_with(" one")),
            "{body:?}"
        );
        assert!(
            body.iter().any(|line| line.trim_end().ends_with(" three")),
            "{body:?}"
        );
        // Nothing of the argument JSON is left on screen below the row.
        assert!(
            !body.iter().any(|line| line.contains("oldText")),
            "{body:?}"
        );
        assert!(
            !body.iter().any(|line| line.contains("src/a.rs")),
            "{body:?}"
        );
    }

    /// A call that is still streaming draws its patch as it arrives: the
    /// snapshot ends mid-literal, and what is already complete renders.
    #[test]
    fn an_edit_still_streaming_shows_the_pairs_it_has() {
        let args =
            r#"{"path":"src/a.rs","edits":[{"oldText":"alpha","newText":"one"},{"oldText":"gam"#;
        let (_, body) = opened_edit_body(args, args);
        assert!(body.iter().any(|line| line.contains("-alpha")), "{body:?}");
        assert!(body.iter().any(|line| line.contains("+one")), "{body:?}");
        assert!(
            !body.iter().any(|line| line.contains("oldText")),
            "{body:?}"
        );
    }

    /// A batch is several changes to one file: each gets its own separator.
    #[test]
    fn a_batch_edit_separates_its_changes() {
        let args = r#"{"path":"f","edits":[{"oldText":"alpha","newText":"one"},{"oldText":"gamma","newText":"three"}]}"#;
        let (row, body) = opened_edit_body(args, args);
        assert!(row.contains("+2 -2"), "{row:?}");
        assert!(body.iter().any(|line| line.contains("@@ 1/2")), "{body:?}");
        assert!(body.iter().any(|line| line.contains("@@ 2/2")), "{body:?}");
        assert!(body.iter().any(|line| line.contains("-gamma")), "{body:?}");
        assert!(body.iter().any(|line| line.contains("+three")), "{body:?}");
    }

    /// The model's own context is trimmed: a one-line change inside ten lines
    /// of untouched text is a few rows, not twenty.
    #[test]
    fn a_long_edited_block_keeps_its_context_and_drops_the_rest() {
        let old: Vec<String> = (0..10).map(|i| format!("a{i}")).collect();
        let mut new = old.clone();
        new[5] = "changed".into();
        let args = serde_json::json!({
            "path": "f",
            "oldText": old.join("\n"),
            "newText": new.join("\n"),
        })
        .to_string();
        let (row, body) = opened_edit_body(&args, &args);

        assert!(row.contains("+1 -1"), "{row:?}");
        assert!(body.iter().any(|line| line.contains("-a5")), "{body:?}");
        assert!(
            body.iter().any(|line| line.contains("+changed")),
            "{body:?}"
        );
        // Three context lines either side, and a marker for the dropped ones.
        assert_eq!(
            body.iter().filter(|line| line.contains('…')).count(),
            2,
            "{body:?}"
        );
        // The lines beyond that context are gone, not drawn as a 20-row block.
        assert!(
            !body.iter().any(|line| line.trim_end().ends_with("a0")),
            "{body:?}"
        );
        assert!(body.len() <= 10, "trimmed, not the whole block: {body:?}");
    }

    /// Only `edit` carries a patch in its arguments; another tool's arguments
    /// are still shown as they are.
    #[test]
    fn another_tool_keeps_its_argument_body() {
        let mut chat = new_chat();
        tool_lines(
            &mut chat,
            "write",
            r#"{"path":"f","oldText":"a","newText":"b"}"#,
            r#"{"path":"f","oldText":"a","newText":"b"}"#,
        );
        open_all_rows(&mut chat);
        let body: Vec<String> = render_trimmed(&mut chat)
            .iter()
            .map(|line| strip(line))
            .collect();
        assert!(body.iter().any(|line| line.contains("oldText")), "{body:?}");
        assert!(!body.iter().any(|line| line.contains("-a")), "{body:?}");
    }

    #[test]
    fn a_failed_call_is_one_quiet_row_and_its_body_waits_for_the_toggle() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_tool_start("c1", "shell", Some(r#"{"command":"false"}"#.into()));
        chat.append_tool_delta("c1", "boom\n");
        chat.messages.last_mut().unwrap().tool_status = Some(ToolStatus::Error);

        // Collapsed, a failure is the same one row as any other call: a `✗`,
        // the reason in the output, and no red band across the transcript.
        let lines = render_trimmed(&mut chat);
        assert_eq!(lines.len(), 1, "one row, no body: {lines:?}");
        assert!(strip(&lines[0]).contains("false"), "{lines:?}");
        assert!(
            strip(&lines[0]).contains('✗'),
            "marked as failed: {lines:?}"
        );
        assert!(
            !lines[0].contains(&format!("\x1b[48;5;{}m", DARK_THEME.tool_error_bg)),
            "not painted red: {lines:?}"
        );
        assert!(
            lines[0].contains(&format!("\x1b[48;5;{}m", DARK_THEME.tool_pending_bg)),
            "the neutral band: {lines:?}"
        );

        // Opening the row shows the reason — in the body colour, not the error
        // one: what went wrong is text to read, not an alarm.
        open_all_rows(&mut chat);
        let lines = render_trimmed(&mut chat);
        assert_eq!(lines.len(), 2, "row + the body: {lines:?}");
        assert!(strip(&lines[1]).contains("boom"));
        assert!(lines[1].contains(&format!("\x1b[38;5;{}m", DARK_THEME.tool_output)));
        assert!(!lines[1].contains(&format!("\x1b[38;5;{}m", DARK_THEME.error)));
    }

    /// A failed call's long body is bounded by the same cap as any other, and
    /// the marker says the rest is cut — there is no collapsed preview left to
    /// open from, so it never advertises it.
    #[test]
    fn a_long_error_body_is_truncated_at_the_same_cap() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_tool_start("c1", "shell", Some(r#"{"command":"false"}"#.into()));
        chat.append_tool_delta("c1", &"line\n".repeat(EXPANDED_TOOL_OUTPUT_ROWS + 12));
        chat.messages.last_mut().unwrap().tool_status = Some(ToolStatus::Error);
        open_all_rows(&mut chat);
        let lines = render_trimmed(&mut chat);
        assert_eq!(lines.len(), 1 + EXPANDED_TOOL_OUTPUT_ROWS + 1, "{lines:?}");
        let marker = strip(lines.last().unwrap());
        assert!(marker.contains("12 more lines"), "{marker:?}");
        assert!(marker.contains("truncated"), "{marker:?}");
    }

    #[test]
    fn finish_tool_keeps_a_body_that_never_streamed() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_tool_start("c9", "read", None);
        chat.finish_tool("c9", Some("from tool_end\n"), false);
        open_all_rows(&mut chat);
        let lines = render_trimmed(&mut chat);
        assert!(strip(&lines[1]).contains("from tool_end"));
        // A streamed body wins over the tool_end text.
        let mut chat = new_chat();
        chat.render(W);
        chat.add_tool_start("c9", "read", None);
        chat.append_tool_delta("c9", "streamed\n");
        chat.finish_tool("c9", Some("late\n"), false);
        open_all_rows(&mut chat);
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
        assert_eq!(unified.rows(0, &DARK_THEME), (vec![], 0));
        assert_eq!(unified.rows(2, &DARK_THEME), (vec![], 0));
        let (narrow, hidden) = text.rows(3, &DARK_THEME);
        assert_eq!(hidden, 0);
        assert_eq!(narrow.len(), 2);
        assert_eq!(crate::utils::visible_width(&narrow[0]), 3);
    }

    #[test]
    fn tool_output_rows_are_bounded() {
        let big: String = (0..EXPANDED_TOOL_OUTPUT_ROWS + 50)
            .map(|i| format!("line {i}\n"))
            .collect();
        let body = ToolOutput::parse(&big);
        let (rows, hidden) = body.rows(W, &DARK_THEME);
        assert_eq!(rows.len(), EXPANDED_TOOL_OUTPUT_ROWS);
        assert_eq!(hidden, 50);
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
        open_all_rows(&mut chat2);
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
        let bg = format!("\x1b[48;5;{}m", DARK_THEME.user_bg);
        // Card padding row, the text row, and the padding row under it — then
        // the message's control row and the transcript's trailing spacer.
        let (card, tail) = lines.split_at(3);
        assert_eq!(tail.len(), 2, "{lines:?}");
        assert_eq!(
            chat.control_rows(),
            vec![3],
            "the controls sit under the card, not inside it"
        );
        assert!(!tail[0].starts_with(&bg), "…outside the surface: {tail:?}");
        for line in card {
            assert!(line.starts_with(&bg), "every block row carries the surface");
        }
        // The text is inset from the surface's left edge — the block is a card,
        // not a highlighted row.
        assert!(strip(&card[1]).starts_with("  hi there"));
    }

    /// A prompt still waiting in the agent's queue draws nothing here: it is
    /// listed above the composer instead (`workbench::screen::queue_band`), so
    /// the transcript only ever holds turns that were actually sent.
    #[test]
    fn a_queued_prompt_takes_no_rows_in_the_transcript() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("a1".into(), ChatRole::Assistant, "answer"));
        let sent = chat.render_all(W).len();
        chat.upsert_queued_run("run-1", "later work", 3);
        let plain = chat
            .render_all(W)
            .iter()
            .map(|l| crate::utils::strip_ansi_codes(l))
            .collect::<Vec<_>>();
        assert_eq!(plain.len(), sent, "a queued prompt added rows: {plain:?}");
        assert!(!plain.iter().any(|l| l.contains("later work")));
        assert!(!plain.iter().any(|l| l.contains("queued")));
    }

    /// The queued row is not the only thing that moves: the transcript is
    /// rebuilt when the run starts, so the submission comes back as a card at
    /// the point in the conversation where its turn actually began — and the
    /// separator above it comes with it.
    #[test]
    fn a_queued_prompt_appears_once_its_run_starts() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("a1".into(), ChatRole::Assistant, "answer"));
        chat.add_message(ChatMessage::new("u1".into(), ChatRole::User, "later work"));
        chat.bind_user_run("u1", "run-1", RunState::Queued, Some(1));
        let hidden = chat.render_all(W).len();
        assert!(!chat
            .render_all(W)
            .iter()
            .any(|line| strip(line).contains("later work")));
        chat.update_run_state("run-1", RunState::Running);
        let shown = chat.render_all(W);
        assert!(shown.iter().any(|line| strip(line).contains("later work")));
        assert!(shown.len() > hidden);
    }

    /// The agent folds a burst of queued submissions into the run ahead of
    /// them (`enqueue_coalescing`), so the card that *ran* has to carry the
    /// folded text: dropping the absorbed card alone would leave the transcript
    /// showing a message shorter than the one the agent was asked.
    #[test]
    fn a_merged_submission_folds_its_text_into_the_card_ahead() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("u1".into(), ChatRole::User, "first"));
        chat.bind_user_run("u1", "r-1", RunState::Running, None);
        chat.add_message(ChatMessage::new("u2".into(), ChatRole::User, "second"));
        chat.bind_user_run("u2", "r-2", RunState::Queued, Some(1));
        assert!(chat.fold_run_into_previous("r-2"));
        assert_eq!(chat.messages.len(), 1, "the folded card is gone");
        // The agent joins the two with a blank line (`start_next_scheduled`), and
        // the card shows exactly that: one message, both paragraphs.
        assert_eq!(chat.messages[0].content, "first\n\nsecond");
        let plain: Vec<String> = chat.render_all(W).into_iter().map(|l| strip(&l)).collect();
        assert!(plain.iter().any(|l| l.contains("first")));
        assert!(plain.iter().any(|l| l.contains("second")));
        // A fold with no card of ours ahead: the folded submission is dropped
        // and the caller is told nothing of this client's grew.
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("u1".into(), ChatRole::User, "only"));
        chat.bind_user_run("u1", "r-7", RunState::Queued, Some(1));
        assert!(!chat.fold_run_into_previous("r-8"));
        assert_eq!(chat.messages.len(), 1, "an unknown run changes nothing");
    }

    /// `drop_run` is the cancel path: the submission never ran, so its card
    /// leaves the transcript instead of sitting there as "cancelled".
    #[test]
    fn drop_run_removes_the_dropped_submission() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("u1".into(), ChatRole::User, "never sent"));
        chat.bind_user_run("u1", "run-1", RunState::Queued, Some(1));
        chat.drop_run("run-1");
        assert!(chat.messages.is_empty());
        assert!(!chat
            .render_all(W)
            .iter()
            .any(|line| strip(line).contains("never sent")));
        // Dropping a run this chat does not hold is a no-op, not a panic.
        chat.drop_run("run-404");
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

    /// A manual-approval session interleaves a `System` row ("Allowed: …")
    /// between the tool call and the text that follows it. The text has to land
    /// *after* that row, not in the turn's opening assistant message above the
    /// call — otherwise the final answer renders before the calls that produced
    /// it (measured on a `[manual]` session, 2026-10-22).
    #[test]
    fn append_to_last_message_after_an_approval_row_starts_new_assistant() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("u".into(), ChatRole::User, "prompt"));
        chat.add_message(ChatMessage::new("a".into(), ChatRole::Assistant, ""));
        chat.add_tool_start("c1", "shell", None);
        chat.finish_tool("c1", None, false);
        chat.add_message(ChatMessage::new(
            "s1".into(),
            ChatRole::System,
            "Allowed: Approve shell command",
        ));
        chat.append_to_last_message("final answer");
        assert_eq!(chat.messages.last().unwrap().role, ChatRole::Assistant);
        assert_eq!(chat.messages.last().unwrap().content, "final answer");
        assert_eq!(
            chat.messages[1].content, "",
            "the assistant message above the tool call must not collect the answer"
        );
    }

    /// `agent_end` carries the run's final text; it takes the same position
    /// decision as a streamed delta.
    #[test]
    fn the_final_text_lands_after_the_approval_rows_it_followed() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("u".into(), ChatRole::User, "prompt"));
        chat.add_message(ChatMessage::new("a".into(), ChatRole::Assistant, ""));
        chat.add_tool_start("c1", "shell", None);
        chat.finish_tool("c1", None, false);
        chat.add_message(ChatMessage::new(
            "s1".into(),
            ChatRole::System,
            "Allowed: Approve shell command",
        ));
        chat.update_last_message("final answer");
        assert_eq!(chat.messages.last().unwrap().role, ChatRole::Assistant);
        assert_eq!(chat.messages.last().unwrap().content, "final answer");
        assert_eq!(chat.messages[1].content, "");
    }

    /// A prompt queued mid-stream (`enqueue_if_busy`) is the *next* turn: it
    /// must not split the running turn, whose remaining text still belongs to
    /// the assistant message above the queued row.
    #[test]
    fn a_queued_prompt_does_not_split_the_running_turn() {
        let mut chat = new_chat();
        chat.render(W);
        chat.add_message(ChatMessage::new("a".into(), ChatRole::Assistant, "stream"));
        chat.add_message(ChatMessage::new("u2".into(), ChatRole::User, "queued"));
        chat.append_to_last_message(" tail");
        assert_eq!(chat.messages[0].content, "stream tail");
        assert_eq!(chat.messages.last().unwrap().role, ChatRole::User);
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
    fn a_shared_render_is_reused_until_the_transcript_changes() {
        let mut chat = ChatArea::new(40, None);
        chat.add_message(ChatMessage::new("u1".into(), ChatRole::User, "hello"));
        let first = chat.render_all_shared(40);
        let second = chat.render_all_shared(40);
        assert!(
            std::sync::Arc::ptr_eq(&first, &second),
            "an unchanged transcript must not be flattened again"
        );
        chat.add_message(ChatMessage::new("a1".into(), ChatRole::Assistant, "world"));
        let third = chat.render_all_shared(40);
        assert!(!std::sync::Arc::ptr_eq(&second, &third));
        assert!(third.iter().any(|row| row.contains("world")));
    }

    #[test]
    fn run_state_labels_match_ts() {
        assert_eq!(RunState::Queued.label(), "queued");
        assert_eq!(RunState::Running.label(), "running");
        assert_eq!(RunState::Terminal.label(), "terminal");
        assert_eq!(RunState::Failed.label(), "failed");
        assert_eq!(RunState::Cancelled.label(), "cancelled");
        assert_eq!(RunState::Superseded.label(), "superseded");
        assert_eq!(RunState::Merged.label(), "merged into the previous run");
        assert_eq!(
            RunState::LostOnAgentRestart.label(),
            "lost_on_agent_restart"
        );
        assert_eq!(
            RunState::NotAcknowledged.label(),
            "delivery not acknowledged"
        );
    }

    #[test]
    fn merged_submission_renders_its_own_line() {
        // A submission folded into the run ahead of it never starts, so the
        // composer has to say so instead of leaving it at `queued (#n)`.
        let mut chat = new_chat();
        chat.add_message(ChatMessage {
            run_state: Some(RunState::Merged),
            ..ChatMessage::new("m1".into(), ChatRole::User, "a supplement")
        });
        let rendered = chat.render_all(W).join("\n");
        let rendered = strip(&rendered);
        assert!(
            rendered.contains("merged into the previous run"),
            "{rendered}"
        );
        assert!(!rendered.contains("queued"), "{rendered}");
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
        let mut chat = detailed_chat();
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

        // Queued without a position keeps the message's state, and (like every
        // queued prompt) draws no rows.
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

    /// `set_compact_activity` is a *setter*, called from the key handler on
    /// every toggle and from state restoration on every layout pass. Re-setting
    /// it to the value it already holds must leave the layout exactly as it is,
    /// while a real change must refold the run. Asserted on the rendered rows
    /// (the only thing a caller can see), not on an internal counter.
    #[test]
    fn set_compact_activity_ignores_a_repeated_value_and_refolds_on_a_change() {
        let mut chat = detailed_chat();
        chat.render(W);
        for path in ["/a.rs", "/b.rs", "/c.rs"] {
            chat.add_message(read_of(path));
        }
        let call_rows = |chat: &mut ChatArea| {
            compact_lines(chat)
                .iter()
                .filter(|line| line.contains("read"))
                .count()
        };

        assert_eq!(call_rows(&mut chat), 3, "unfolded by default");

        chat.set_compact_activity(true);
        assert!(chat.compact_activity());
        let folded = call_rows(&mut chat);
        assert_eq!(folded, 1, "a run of three calls folds to one row");

        // Setting the same value again must not undo or duplicate the fold.
        chat.set_compact_activity(true);
        assert!(chat.compact_activity());
        assert_eq!(call_rows(&mut chat), folded, "a repeated set is a no-op");

        // A real change refolds in the other direction, and repeats no-op too.
        chat.set_compact_activity(false);
        assert!(!chat.compact_activity());
        assert_eq!(call_rows(&mut chat), 3, "unfolding restores every call");
        chat.set_compact_activity(false);
        assert!(!chat.compact_activity());
        assert_eq!(call_rows(&mut chat), 3, "a repeated unset is a no-op");
    }

    /// Prepending history shifts the viewport by the height of the inserted
    /// block, and that shift is a **layout change**: the app that owns the
    /// scrollback has to be told, or its own idea of the visible window drifts
    /// from the rendered one. Asserted on the callback the app registers plus
    /// the returned height.
    #[test]
    fn prepending_history_shifts_the_viewport_and_notifies_the_owner() {
        use std::cell::Cell;
        use std::rc::Rc;
        let mut chat = detailed_chat();
        chat.render(W);
        chat.add_message(read_of("/later.rs"));
        let rendered_before = chat.render(W).len();

        let notified = Rc::new(Cell::new(0usize));
        let counter = Rc::clone(&notified);
        chat.set_on_change(move || counter.set(counter.get() + 1));

        let height = chat.prepend_messages(vec![read_of("/older.rs")]);
        assert!(height > 0, "the inserted block occupies rows");
        assert_eq!(
            notified.get(),
            1,
            "the owner must be notified exactly once per prepend"
        );

        let after = chat.render(W);
        let after_len = after.len();
        assert!(
            after_len > rendered_before,
            "the older message added rows: {rendered_before} -> {after_len}"
        );
        let plain = after.iter().map(|l| strip(l)).collect::<Vec<_>>();
        let older = plain
            .iter()
            .position(|l| l.contains("older.rs"))
            .expect("the prepended message is visible");
        let later = plain
            .iter()
            .position(|l| l.contains("later.rs"))
            .expect("the original message is still there");
        assert!(older < later, "prepended history goes above");

        // Prepending nothing is not a change and must not notify.
        assert_eq!(chat.prepend_messages(Vec::new()), 0);
        assert_eq!(notified.get(), 1, "an empty prepend notifies nobody");
    }

    /// The folded thinking marker carries an in-progress ellipsis *only* while
    /// the message is still pending: in the compact view the marker is the only
    /// signal that reasoning is still arriving, so it must appear while
    /// streaming and disappear the moment the run completes (otherwise a
    /// finished run still looks busy).
    #[test]
    fn the_thinking_marker_shows_an_ellipsis_only_while_streaming() {
        let mut chat = detailed_chat();
        chat.render(W);
        chat.set_compact_activity(true);
        chat.start_thinking();
        chat.append_thinking_delta("reasoning in flight");

        let streaming = chat.render_all(W).join("\n");
        assert!(
            streaming.contains("thinking\u{2026}"),
            "an in-flight block must be marked in progress: {streaming:?}"
        );

        chat.append_to_last_message("answer");
        chat.mark_last_message_complete();
        let done = chat.render_all(W).join("\n");
        assert!(
            done.contains("thinking"),
            "the finished run still keeps the marker row: {done:?}"
        );
        assert!(
            !done.contains("thinking\u{2026}"),
            "a completed run must not look busy: {done:?}"
        );
    }

    /// In the compact view a folded run's rows live in the run's *summary* row,
    /// so a member message has no rows of its own to splice — re-rendering one
    /// must leave the transcript untouched rather than inserting or dropping
    /// lines for a message that renders nothing.
    #[test]
    fn rerendering_a_folded_member_changes_no_rows() {
        let mut chat = detailed_chat();
        chat.render(W);
        chat.set_compact_activity(true);
        for path in ["/a.rs", "/b.rs", "/c.rs"] {
            chat.add_message(read_of(path));
        }
        let before = chat.render_all(W);
        let last = chat.messages.len() - 1;
        assert!(
            chat.is_folded(last),
            "a member of the folded run must report as folded"
        );

        chat.rerender_message(last);
        let after = chat.render_all(W);
        assert_eq!(
            before, after,
            "a folded member has no rows, so re-rendering it changes nothing"
        );

        // The run is still one summary row, not one row per call.
        let summary_rows = after.iter().filter(|line| line.contains("read")).count();
        assert_eq!(summary_rows, 1, "{after:?}");
    }

    /// Shrinking the transcript (folding a run, `/clear`) can leave the
    /// viewport scrolled past the new end. `render` must clamp it, or the next
    /// frame slices out of range and the view jumps to a line that no longer
    /// exists.
    #[test]
    fn render_clamps_a_viewport_left_past_the_end_by_shrinking_content() {
        let mut chat = detailed_chat();
        // Enough messages that the transcript is taller than the viewport, so
        // scrolling to the bottom really moves `viewport_top`.
        for index in 0..24 {
            chat.add_message(read_of(&format!("/file-{index}.rs")));
        }
        chat.render(W);
        let (line_count, viewport_rows) = (chat.rendered_lines.len(), chat.viewport_height);
        assert!(
            line_count > viewport_rows,
            "the transcript must exceed the viewport ({line_count} lines, {viewport_rows} rows)"
        );
        chat.scroll_to_bottom();
        assert!(
            chat.viewport_top > 0,
            "the transcript must be longer than one screen for this test"
        );

        // Compacting folds the run of calls into a single summary row, which
        // shortens the transcript under the scrolled viewport.
        chat.set_compact_activity(true);
        let lines = chat.render(W);
        assert!(!lines.is_empty(), "the clamped view still has content");
        // The clamp is observable: the viewport is back inside the transcript,
        // so the rendered window is real content rather than an out-of-range
        // slice. `viewport_top` is unsigned, so only the upper bound needs
        // checking.
        let (top, total) = (chat.viewport_top, chat.rendered_lines.len());
        assert!(
            top <= total,
            "viewport_top {top} must be inside {total} lines"
        );
    }

    // ─── Pictures ───────────────────────────────────────────────────────

    /// A message carrying a picture, written to a scratch file so the caption's
    /// link resolves the way it does in a real session.
    fn picture_message(dir: &std::path::Path) -> ChatMessage {
        let path = dir.join("a shot.png");
        std::fs::write(&path, b"\x89PNG\r\n\x1a\n").unwrap();
        let mut message = ChatMessage::new("m-pic".into(), ChatRole::User, "look at this");
        message.pictures.push(ChatPicture {
            path: path.to_string_lossy().into_owned(),
            name: "a shot.png".into(),
        });
        message
    }

    /// A picture is one row in the transcript: its name, as a link.
    ///
    /// Nothing is reserved for a thumbnail — not on an image-capable terminal
    /// either — because the picture is drawn in the preview box and nowhere
    /// else. The row that remains is the door: the caption is a real link, so
    /// `tab` + `enter` and a click both open it.
    #[test]
    fn a_picture_is_its_linked_caption() {
        let _caps = crate::terminal_image::CapabilitiesGuard::pin(
            crate::terminal_image::ImageProtocol::Kitty,
        );
        let dir = std::env::temp_dir().join(format!("future-chat-pic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let message = picture_message(&dir);

        let mut chat = new_chat();
        chat.add_message(message);
        // The raw rows: the caption is a hyperlink, and its target lives in the
        // OSC 8 wrapper that stripping would remove.
        let rows = chat.render(W);
        let plain: Vec<String> = rows.iter().map(|row| strip_ansi_codes(row)).collect();
        let text_row = plain
            .iter()
            .position(|row| row.contains("look at this"))
            .unwrap_or_else(|| panic!("the text is there: {plain:?}"));
        let caption_row = plain
            .iter()
            .position(|row| row.contains("a shot.png"))
            .unwrap_or_else(|| panic!("the caption is there: {plain:?}"));
        // One row between them: the block's own padding, never a blank slot.
        assert_eq!(
            caption_row,
            text_row + 1,
            "no thumbnail block above the caption: {plain:?}"
        );
        let spans = crate::workbench::links::scan(
            std::slice::from_ref(&rows[caption_row]),
            &dir.to_string_lossy(),
            &mut Default::default(),
        );
        assert_eq!(spans.len(), 1, "{spans:?}");
        assert!(
            matches!(&spans[0].target, crate::workbench::links::LinkTarget::File(path)
                if path.ends_with("a shot.png")),
            "{:?}",
            spans[0].target
        );
        // A copy read as text is the same transcript: there is no slot to drop.
        let copy: Vec<String> = chat
            .render_all(W)
            .iter()
            .map(|row| strip_ansi_codes(row))
            .collect();
        let copied = copy
            .iter()
            .position(|row| row.contains("a shot.png"))
            .unwrap_or_else(|| panic!("the caption is there: {copy:?}"));
        assert!(
            copy[copied - 1].contains("look at this"),
            "the copy keeps the same rows: {copy:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A picture with no name falls back to its file's own.
    #[test]
    fn a_nameless_picture_names_its_file() {
        let dir = std::env::temp_dir().join(format!("future-chat-noname-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("logo.png");
        std::fs::write(&path, b"\x89PNG").unwrap();
        let mut message = ChatMessage::new("m-pic2".into(), ChatRole::Assistant, "here");
        message.pictures.push(ChatPicture {
            path: path.to_string_lossy().into_owned(),
            name: String::new(),
        });
        let mut chat = new_chat();
        chat.add_message(message);
        let text = chat
            .render(W)
            .iter()
            .map(|row| strip_ansi_codes(row))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("logo.png"), "{text}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
