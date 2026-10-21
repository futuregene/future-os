//! The approval prompt: a modal card that asks a yes/no question about one
//! pending tool call.
//!
//! Port of the desktop's `ApprovalPrompt.tsx` as a terminal card: the frame,
//! the tool-use details and a numbered option list the cursor moves over
//! (`↑`/`↓`), the shape Claude Code's permission prompt uses. The card is
//! deliberately *modal* — the agent parks the whole run until a decision
//! arrives, so the input line must not compete for the same keys.
//!
//! Like [`crate::components::sandbox_view`], this module is I/O-free: it holds
//! the request it was handed, renders it, and reports *intent*
//! ([`ApprovalAction`]) for the app to turn into an RPC.
//!
//! Design notes:
//!
//! * **The details come from `action`, not the raw arguments.** The agent's
//!   `action` object is the curated view (`command`, `paths`, `writes`,
//!   `targets`, `justification`); `requested_action` is the raw tool arguments
//!   — a JSON object like `{"command": "…", "timeout": 30}` that reads like a
//!   config dump. The raw form is only a fallback for payloads that carry no
//!   `action`.
//! * **Every row measures exactly `width` columns** for any `width` including
//!   `0` and `1`, the card never exceeds `height`, and long content is capped
//!   so the options are always on screen.

use serde_json::Value;

use crate::theme::{bold, fg, Theme, DARK_THEME};
use crate::utils::{truncate_to_width, visible_width, wrap_text_with_ansi, TruncateOptions};

// ─── Copy ──────────────────────────────────────────────────────────────────

const HEADER: &str = "Tool use";
const QUESTION: &str = "Do you want to proceed?";
const YES: &str = "Yes";
const NO: &str = "No";
const KEYS: &str = "↑/↓ select · enter confirm · esc cancel";
/// Detail rows beyond this are dropped (with a `…` row) so a 2000-character
/// command cannot push the options off the card.
const MAX_DETAIL_ROWS: usize = 14;

// ─── Request ───────────────────────────────────────────────────────────────

/// One approval request as the card needs it. The agent's `approval_request`
/// payload and `get_state.pendingApprovals` carry the same fields.
#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalRequest {
    pub request_id: String,
    pub title: String,
    pub summary: String,
    pub kind: String,
    pub risk_level: String,
    /// The agent's curated action view (`command`, `paths`, `writes`, …).
    pub action: Option<Value>,
    /// The raw tool arguments, only used when `action` carries no detail.
    pub requested_action: Option<Value>,
}

// ─── Actions ───────────────────────────────────────────────────────────────

/// What a key press asks the caller to do. Nothing here performs I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalAction {
    /// The key did nothing.
    None,
    /// The highlight moved — redraw.
    Moved,
    /// Answer `request_id` (`approved == false` is a denial).
    Decide { request_id: String, approved: bool },
}

// ─── View ──────────────────────────────────────────────────────────────────

/// The approval card: a title row, the tool-use details and two options.
pub struct ApprovalView {
    request: ApprovalRequest,
    /// `0` = Yes, `1` = No.
    index: usize,
    /// How many requests the app still holds (shown when > 1).
    queue_len: usize,
    theme: Theme,
}

impl ApprovalView {
    pub fn new(request: ApprovalRequest, queue_len: usize) -> Self {
        ApprovalView {
            request,
            index: 0,
            queue_len,
            theme: DARK_THEME,
        }
    }

    /// Replace the request the card is asking about. A different request moves
    /// the highlight back to `Yes`; the same one (a refresh re-asking) keeps
    /// the user's cursor where they left it.
    pub fn set_request(&mut self, request: ApprovalRequest, queue_len: usize) {
        if self.request.request_id != request.request_id {
            self.index = 0;
        }
        self.request = request;
        self.queue_len = queue_len;
    }

    pub fn request_id(&self) -> &str {
        &self.request.request_id
    }

    pub fn set_theme(&mut self, theme: &Theme) {
        self.theme = *theme;
    }

    /// The highlighted option's answer.
    pub fn highlighted_approved(&self) -> bool {
        self.index == 0
    }

    pub fn handle_key(&mut self, key: &str) -> ApprovalAction {
        match key {
            "up" | "k" => {
                if self.index == 0 {
                    return ApprovalAction::None;
                }
                self.index -= 1;
                ApprovalAction::Moved
            }
            "down" | "j" => {
                if self.index + 1 >= 2 {
                    return ApprovalAction::None;
                }
                self.index += 1;
                ApprovalAction::Moved
            }
            // Both the plain keys and the option numbers answer directly —
            // one keystroke, no cursor trip.
            "1" | "y" | "Y" => self.decide(true),
            "2" | "n" | "N" => self.decide(false),
            "enter" | "space" => self.decide(self.highlighted_approved()),
            // Escape means "cancel" in the Claude Code prompt: the request is
            // denied (the run was parked; leaving it unanswered would hang
            // it). The app forwards escape here via `wants_escape`.
            "escape" => self.decide(false),
            _ => ApprovalAction::None,
        }
    }

    fn decide(&self, approved: bool) -> ApprovalAction {
        ApprovalAction::Decide {
            request_id: self.request.request_id.clone(),
            approved,
        }
    }

    /// The card at `width × height`: never more than `height` rows, each
    /// exactly `width` visible columns. Empty for `width == 0` or `height == 0`.
    pub fn render(&mut self, width: usize, height: usize) -> Vec<String> {
        if width == 0 || height == 0 {
            return Vec::new();
        }
        let inner = width.saturating_sub(2);
        let mut rows: Vec<String> = Vec::new();

        rows.push(border_row('┌', '┐', width, self.theme.dim as u8));
        rows.push(card_row(&self.header_row(), inner, &self.theme));
        rows.push(card_row("", inner, &self.theme));

        // Details first: their budget is whatever the fixed rows leave.
        // Fixed rows = top, header, blank, blank, question, 2 options, blank,
        // footer, bottom = 10.
        let detail_budget = height.saturating_sub(10).clamp(1, MAX_DETAIL_ROWS);
        let details = self.detail_rows(inner);
        let shown = details.len().min(detail_budget);
        for row in details.iter().take(shown) {
            rows.push(card_row(row, inner, &self.theme));
        }
        if details.len() > shown {
            rows.push(card_row(
                &fg(self.theme.dim as u8, "  …"),
                inner,
                &self.theme,
            ));
        }

        rows.push(card_row("", inner, &self.theme));
        rows.push(card_row(
            &fg(self.theme.fg as u8, QUESTION),
            inner,
            &self.theme,
        ));
        for (index, label) in [(0, YES), (1, NO)] {
            rows.push(self.option_row(index, label, inner, width));
        }
        rows.push(card_row("", inner, &self.theme));
        rows.push(card_row(
            &fg(self.theme.dim as u8, &self.footer_text()),
            inner,
            &self.theme,
        ));
        rows.push(border_row('└', '┘', width, self.theme.dim as u8));

        rows.truncate(height);
        rows.into_iter().map(|row| fit_row(&row, width)).collect()
    }

    // ── rows ──

    fn header_row(&self) -> String {
        let mut row = fg(self.theme.accent as u8, &bold(HEADER));
        let risk = self.request.risk_level.trim();
        if !risk.is_empty() {
            row.push_str(&fg(
                self.theme.dim as u8,
                &format!("  ·  {} risk", risk.to_uppercase()),
            ));
        }
        row
    }

    fn option_row(&self, index: usize, label: &str, _inner: usize, _width: usize) -> String {
        let highlighted = index == self.index;
        // The cursor marks the selection; the row keeps the card's frame. A
        // full-width selection bar would paint over the side borders and read
        // as a separate widget, not as one of the card's options.
        let text = if highlighted {
            format!(
                "{} {}. {}",
                fg(self.theme.accent as u8, "❯"),
                index + 1,
                fg(self.theme.fg as u8, label)
            )
        } else {
            format!("  {}. {}", index + 1, fg(self.theme.dim as u8, label))
        };
        card_row(&text, _inner, &self.theme)
    }

    fn footer_text(&self) -> String {
        if self.queue_len > 1 {
            format!("{KEYS} · {} pending", self.queue_len)
        } else {
            KEYS.to_string()
        }
    }

    /// The tool-use details, wrapped to the card's inner width. Each entry is
    /// already styled; the caller counts rows, not lines.
    fn detail_rows(&self, inner: usize) -> Vec<String> {
        let text_width = inner.saturating_sub(DETAIL_INDENT);
        if text_width == 0 {
            return Vec::new();
        }
        let indent = " ".repeat(DETAIL_INDENT);
        let mut rows: Vec<String> = Vec::new();
        let mut push = |text: &str, colour: u8| {
            let trimmed = text.trim_end();
            if trimmed.trim().is_empty() {
                return;
            }
            for line in wrap_text_with_ansi(trimmed, text_width) {
                rows.push(format!("{indent}{}", fg(colour, &line)));
            }
        };

        // The agent's title ("Approve shell command", "Modify /etc/hosts")
        // says what is being approved even when the action carries no detail.
        push(&self.request.title, self.theme.fg as u8);

        if let Some(command) = self.command_text() {
            push(&format!("$ {command}"), self.theme.success as u8);
        }

        for path in self.paths() {
            push(&format!("· {path}"), self.theme.dim as u8);
        }

        push(&self.request.summary, self.theme.dim as u8);

        if let Some(justification) = self.justification() {
            push(&justification, self.theme.dim as u8);
        }
        rows
    }

    /// The command the request is about, from the curated `action` first.
    fn command_text(&self) -> Option<String> {
        if let Some(value) = self.request.action.as_ref() {
            if let Some(command) = value.get("command").and_then(Value::as_str) {
                let trimmed = command.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
        match self.request.requested_action.as_ref()? {
            Value::String(raw) => {
                let trimmed = raw.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            }
            Value::Object(map) => map
                .get("command")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|command| !command.is_empty())
                .map(str::to_string),
            _ => None,
        }
    }

    /// `paths` from the action (the raw arguments carry none this readable).
    fn paths(&self) -> Vec<String> {
        let Some(action) = self.request.action.as_ref() else {
            return Vec::new();
        };
        let array = action
            .get("paths")
            .or_else(|| action.get("targets"))
            .and_then(Value::as_array);
        let Some(array) = array else {
            return Vec::new();
        };
        let mut paths: Vec<String> = array
            .iter()
            .filter_map(|entry| {
                entry.as_str().map(str::to_string).or_else(|| {
                    entry
                        .get("path")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
            })
            .filter(|path| !path.trim().is_empty())
            .take(5)
            .collect();
        if array.len() > paths.len() {
            paths.push(format!("… {} more", array.len() - paths.len()));
        }
        paths
    }

    fn justification(&self) -> Option<String> {
        let action = self.request.action.as_ref()?;
        let text = action.get("justification").and_then(Value::as_str)?.trim();
        if text.is_empty() || text == self.request.summary.trim() {
            return None;
        }
        Some(text.to_string())
    }
}

// ─── Helpers ───────────────────────────────────────────────────────────────

/// Indent of the detail rows (the options and the question sit at the gutter).
const DETAIL_INDENT: usize = 2;

fn border_row(left: char, right: char, width: usize, colour: u8) -> String {
    if width == 0 {
        return String::new();
    }
    if width == 1 {
        return fg(colour, &left.to_string());
    }
    fg(colour, &format!("{left}{}{right}", "─".repeat(width - 2)))
}

/// `│ {content}` padded so the row lines up with the frame.
///
/// `inner` is the space between the two border columns; the content starts
/// after the border **and** one gutter space, so its budget is `inner - 1`.
/// Getting this wrong by one column is what clipped the right border off
/// every row (the final `fit_row` then ate the last character).
fn card_row(content: &str, inner: usize, theme: &Theme) -> String {
    let budget = inner.saturating_sub(1);
    let clipped = truncate_to_width(content, budget, &TruncateOptions::default());
    let padding = budget.saturating_sub(visible_width(&clipped));
    format!(
        "{}{clipped}{}{}",
        fg(theme.dim as u8, "│ "),
        " ".repeat(padding),
        fg(theme.dim as u8, "│"),
    )
}

/// Pad/truncate a row to exactly `width` visible columns.
fn fit_row(content: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let clipped = if visible_width(content) > width {
        truncate_to_width(content, width, &TruncateOptions::default())
    } else {
        content.to_string()
    };
    let padding = width.saturating_sub(visible_width(&clipped));
    if padding == 0 {
        clipped
    } else {
        format!("{clipped}{}", " ".repeat(padding))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::strip_ansi_codes;

    fn request(action: Option<Value>, requested_action: Option<Value>) -> ApprovalRequest {
        ApprovalRequest {
            request_id: "approval_1".into(),
            title: "Approve shell command".into(),
            summary: "Agent wants to run a shell command.".into(),
            kind: "shell_command".into(),
            risk_level: "medium".into(),
            action,
            requested_action,
        }
    }

    fn shell_request() -> ApprovalRequest {
        request(
            Some(serde_json::json!({
                "tool": "shell",
                "category": "shell_command",
                "summary": "curl -s https://example.com | head -c 2000",
                "command": "curl -s https://example.com | head -c 2000",
                "scope": {"cwd": "/w", "inside_workspace": true}
            })),
            Some(serde_json::json!({
                "command": "curl -s https://example.com | head -c 2000",
                "timeout": 30
            })),
        )
    }

    fn lines(view: &mut ApprovalView, width: usize, height: usize) -> Vec<String> {
        view.render(width, height)
            .iter()
            .map(|row| strip_ansi_codes(row).trim_end().to_string())
            .collect()
    }

    #[test]
    fn the_card_shows_the_command_and_the_two_options() {
        let mut view = ApprovalView::new(shell_request(), 1);
        let rows = lines(&mut view, 80, 30);
        let text = rows.join("\n");
        assert!(text.contains("Tool use"), "{text}");
        assert!(text.contains("MEDIUM risk"), "{text}");
        assert!(
            text.contains("$ curl -s https://example.com | head -c 2000"),
            "{text}"
        );
        assert!(text.contains("Do you want to proceed?"), "{text}");
        assert!(text.contains("❯ 1. Yes"), "{text}");
        assert!(text.contains("  2. No"), "{text}");
        assert!(
            text.contains("↑/↓ select · enter confirm · esc cancel"),
            "{text}"
        );
        // The raw arguments (`timeout: 30`) never leak into the card.
        assert!(!text.contains("timeout"), "{text}");
        assert!(rows[0].starts_with('┌'), "{rows:?}");
        assert!(rows[rows.len() - 1].starts_with('└'), "{rows:?}");
    }

    #[test]
    fn the_arrow_keys_move_the_selection_and_enter_answers_it() {
        let mut view = ApprovalView::new(shell_request(), 1);
        assert!(view.highlighted_approved());
        assert_eq!(view.handle_key("down"), ApprovalAction::Moved);
        assert!(!view.highlighted_approved());
        assert_eq!(view.handle_key("down"), ApprovalAction::None);
        assert_eq!(
            view.handle_key("enter"),
            ApprovalAction::Decide {
                request_id: "approval_1".into(),
                approved: false
            }
        );
        assert_eq!(view.handle_key("up"), ApprovalAction::Moved);
        assert_eq!(
            view.handle_key("space"),
            ApprovalAction::Decide {
                request_id: "approval_1".into(),
                approved: true
            }
        );
        assert_eq!(view.handle_key("up"), ApprovalAction::None);
    }

    #[test]
    fn numbers_letters_and_escape_answer_directly() {
        let mut view = ApprovalView::new(shell_request(), 1);
        for key in ["1", "y", "Y"] {
            assert!(matches!(
                view.handle_key(key),
                ApprovalAction::Decide { approved: true, .. }
            ));
        }
        for key in ["2", "n", "N", "escape"] {
            assert!(
                matches!(
                    view.handle_key(key),
                    ApprovalAction::Decide {
                        approved: false,
                        ..
                    }
                ),
                "{key}"
            );
        }
        assert_eq!(view.handle_key("x"), ApprovalAction::None);
    }

    #[test]
    fn file_approvals_list_their_paths() {
        let mut req = request(
            Some(serde_json::json!({
                "tool": "write",
                "category": "outside_workspace_write",
                "summary": "Modify /etc/hosts",
                "paths": ["/etc/hosts"],
                "writes": [{"path": "/etc/hosts", "preview": "a\nb"}]
            })),
            None,
        );
        req.title = "Approve outside-workspace write".into();
        let mut view = ApprovalView::new(req, 1);
        let text = lines(&mut view, 80, 30).join("\n");
        assert!(text.contains("Approve outside-workspace write"), "{text}");
        assert!(text.contains("· /etc/hosts"), "{text}");
    }

    #[test]
    fn a_raw_string_requested_action_still_gives_the_command() {
        let mut view =
            ApprovalView::new(request(None, Some(Value::String("rm -rf build".into()))), 1);
        let text = lines(&mut view, 80, 30).join("\n");
        assert!(text.contains("$ rm -rf build"), "{text}");
    }

    #[test]
    fn the_queue_size_is_announced() {
        let mut view = ApprovalView::new(shell_request(), 3);
        let text = lines(&mut view, 100, 30).join("\n");
        assert!(text.contains("3 pending"), "{text}");
    }

    #[test]
    fn every_row_is_exactly_the_requested_width_and_never_overflows() {
        for width in 0..=40 {
            for height in 0..=16 {
                let mut view = ApprovalView::new(shell_request(), 1);
                let rows = view.render(width, height);
                assert!(rows.len() <= height, "{width}x{height}");
                if width == 0 || height == 0 {
                    assert!(rows.is_empty());
                }
                for row in &rows {
                    assert_eq!(visible_width(row), width, "{width}x{height}: {row:?}");
                }
            }
        }
    }

    #[test]
    fn a_long_command_is_capped_so_the_options_stay_visible() {
        let long = format!("echo {}", "x".repeat(2000));
        let mut view = ApprovalView::new(
            request(
                Some(serde_json::json!({"command": long, "summary": "s"})),
                None,
            ),
            1,
        );
        let rows = view.render(60, 24);
        let text: String = rows
            .iter()
            .map(|row| strip_ansi_codes(row))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("❯ 1. Yes"), "options must survive the cap");
        assert!(text.contains("2. No"), "{text}");
        assert!(text.contains('…'), "the cut is visible: {text}");
        assert!(rows.len() <= 24);
    }

    #[test]
    fn set_request_resets_the_highlight_to_yes() {
        let mut view = ApprovalView::new(shell_request(), 2);
        view.handle_key("down");
        assert!(!view.highlighted_approved());
        let mut next = shell_request();
        next.request_id = "approval_2".into();
        view.set_request(next, 1);
        assert_eq!(view.request_id(), "approval_2");
        assert!(view.highlighted_approved());
    }
}
