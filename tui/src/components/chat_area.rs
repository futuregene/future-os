//! ChatArea — scrollable chat view matching the TS style. 1:1 port of
//! `tui/src/components/chat-area.ts`.
//!
//! Renders messages with proper markdown, tool output, and streaming,
//! including the deferred re-render queue and the streaming prefix cache
//! (both TS streaming optimizations).

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

use regex::Regex;

use crate::components::markdown::{MarkdownRenderer, MarkdownThemePartial};
use crate::theme::{bold, dim, fg, italic, Theme, DARK_THEME};
use crate::tui::{Component, RESET};
use crate::utils::{
    apply_background_to_line, truncate_to_width, wrap_text_with_ansi, TruncateOptions,
};

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

// ─── Streaming helpers (ported regexes + findStreamCut) ───────────────────

/// `^ {0,3}(`{3,}|~{3,})` — fence opener inside stream cut scanning.
static STREAM_FENCE_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
/// `^ {0,3}\[[^\]\n]+\]:\s*\S` (m flag) — link reference definitions.
static STREAM_LINK_DEF_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();

fn stream_fence_re() -> &'static Regex {
    STREAM_FENCE_RE.get_or_init(|| Regex::new(r"^ {0,3}(`{3,}|~{3,})").unwrap())
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
    on_change: Option<Box<dyn FnMut()>>,
    message_line_ranges: Vec<(usize, i64)>,
}

impl ChatArea {
    pub fn new(max_width: usize, theme: Option<Theme>) -> Self {
        let theme = theme.unwrap_or(DARK_THEME);
        // Thinking renders entirely in the thinking gray: every markdown
        // element that would normally get an accent color is mapped to
        // thinkingText (bold/italic/underline stay attribute-only; the
        // reset-reapply pass in renderAssistantMessage restores the gray).
        let tc = theme.thinking_text as u8;
        let think_fg = move |s: &str| fg(tc, s);
        let md_thinking = MarkdownRenderer::with_theme(MarkdownThemePartial {
            heading: Some(std::rc::Rc::new(think_fg)),
            link: Some(std::rc::Rc::new(think_fg)),
            link_url: Some(std::rc::Rc::new(think_fg)),
            code: Some(std::rc::Rc::new(think_fg)),
            code_block: Some(std::rc::Rc::new(move |s: &str| fg(tc, &dim(s)))),
            code_block_border: Some(std::rc::Rc::new(move |s: &str| fg(tc, &dim(s)))),
            // (No quote/quote_border override: the renderer never invokes
            // those style fns — they exist for TS shape parity only.)
            quote_border: Some(std::rc::Rc::new(think_fg)),
            hr: Some(std::rc::Rc::new(think_fg)),
            list_bullet: Some(std::rc::Rc::new(think_fg)),
            strikethrough: Some(std::rc::Rc::new(think_fg)),
            ..Default::default()
        });

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
            md: MarkdownRenderer::new(),
            md_thinking,
            theme,
            on_change: None,
            message_line_ranges: Vec::new(),
        }
    }

    pub fn last_message(&self) -> Option<&ChatMessage> {
        self.messages.last()
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

    pub fn finish_tool(&mut self, tool_id: &str, _output: Option<&str>) {
        if let Some(idx) = self.find_tool_index(tool_id) {
            self.messages[idx].tool_status = Some(ToolStatus::Complete);
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

    fn rerender(&mut self) {
        self.pending_rerender.clear();
        self.stream_caches.clear();
        // Defer until first render() has set the correct terminal width.
        if self.last_render_width == -1 {
            self.dirty = true;
            return;
        }
        self.dirty = false;

        self.rendered_lines = Vec::new();
        self.message_line_ranges = Vec::new();
        for i in 0..self.messages.len() {
            if i > 0 {
                self.rendered_lines.push(RenderedLine {
                    text: String::new(),
                    dim: true,
                });
            }
            let start = self.rendered_lines.len();
            let msg = self.messages[i].clone();
            self.render_message(&msg);
            self.message_line_ranges
                .push((start, self.rendered_lines.len() as i64 - 1));
        }
        self.rendered_lines.push(RenderedLine {
            text: String::new(),
            dim: true,
        });
    }

    /// Re-render only the message at msg_idx, splicing its lines in-place.
    fn rerender_message(&mut self, msg_idx: usize) {
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
        let msg = self.messages[msg_idx].clone();
        self.render_message(&msg);
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
        self.rendered_lines.pop();
        if self.messages.len() > 1 {
            self.rendered_lines.push(RenderedLine {
                text: String::new(),
                dim: true,
            });
        }
        let start = self.rendered_lines.len();
        let msg = self.messages[self.messages.len() - 1].clone();
        self.render_message(&msg);
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

    fn render_message(&mut self, msg: &ChatMessage) {
        match msg.role {
            ChatRole::User => self.render_user_message(msg),
            ChatRole::Assistant => self.render_assistant_message(msg),
            ChatRole::Tool => self.render_tool_message(msg),
            ChatRole::System => self.render_system_message(msg),
        }
    }

    // ─── User message (markdown + full-width background Box) ────────────

    fn render_user_message(&mut self, msg: &ChatMessage) {
        let rendered = self.md.render_text(&msg.content, self.width - 2);
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

    fn render_assistant_message(&mut self, msg: &ChatMessage) {
        let has_thinking = msg
            .thinking
            .as_deref()
            .is_some_and(|t| !t.trim().is_empty());

        // Render thinking block FIRST (before content). Collapsed thinking
        // (ctrl+o) renders nothing — except a one-line hint while thinking is
        // actively streaming (pending, no content yet) so a live run doesn't
        // look frozen; historical thinking stays fully hidden.
        if has_thinking && !self.thinking_hidden {
            let thinking = msg.thinking.as_deref().unwrap_or("");
            let thinking_lines = if msg.pending {
                Self::render_streaming_markdown(
                    &mut self.stream_caches,
                    &format!("{}:t", msg.id),
                    thinking,
                    self.width - 2,
                    &mut self.md_thinking,
                )
            } else {
                self.md_thinking.render_text(thinking, self.width - 2)
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
        } else if has_thinking && msg.pending && msg.content.trim().is_empty() {
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
        let content_width = self.width - 2;
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
        let line = format!(" {}", self.format_tool_call(tool_name, tool_args));

        self.rendered_lines.push(RenderedLine {
            text: apply_background_to_line(&line, self.width, bg_color),
            dim: status == ToolStatus::Complete,
        });
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
        let wrap_width = std::cmp::max(1, self.width - 2);
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
        chat.finish_tool("c1", None);
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
        chat.finish_tool("c1", None);
        assert_eq!(chat.messages[2].tool_status, Some(ToolStatus::Complete));
        chat.finish_tool("unknown", None); // no-op
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
