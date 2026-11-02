//! Pure state and formatting for the workflow overlays.

use serde_json::Value;

use super::i18n::{t, Key, Language};
use super::state::{ApprovalView, WorkbenchData};

fn pretty(value: Option<&Value>) -> String {
    value
        .filter(|value| !value.is_null())
        .map(|value| serde_json::to_string_pretty(value).unwrap_or_default())
        .unwrap_or_default()
}

fn rules(value: Option<&Value>) -> Vec<super::state::AllowRule> {
    let Some(value) = value else {
        return Vec::new();
    };
    if let Some(rules) = value.get("rules").and_then(Value::as_array) {
        return rules
            .iter()
            .filter_map(|rule| {
                Some(super::state::AllowRule {
                    path: rule.get("path")?.as_str()?.to_string(),
                    access: rule.get("access")?.as_str()?.to_string(),
                })
            })
            .collect();
    }
    match (
        value.get("path").and_then(Value::as_str),
        value.get("access").and_then(Value::as_str),
    ) {
        (Some(path), Some(access)) => vec![super::state::AllowRule {
            path: path.to_string(),
            access: access.to_string(),
        }],
        _ => Vec::new(),
    }
}

pub fn approval_of(value: &Value) -> ApprovalView {
    ApprovalView {
        request_id: value
            .get("approval_request_id")
            .or_else(|| value.get("approvalRequestId"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        tool: value
            .get("tool_name")
            .or_else(|| value.get("toolName"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        kind: value
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        risk: value
            .get("risk_level")
            .or_else(|| value.get("riskLevel"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        title: value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        summary: value
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        action: pretty(value.get("action")),
        requested_action: pretty(
            value
                .get("requested_action")
                .or_else(|| value.get("requestedAction")),
        ),
        sandbox_boundary: pretty(
            value
                .get("sandbox_boundary")
                .or_else(|| value.get("sandboxBoundary")),
        ),
        rules: rules(
            value
                .get("save_suggestion")
                .or_else(|| value.get("saveSuggestion")),
        ),
    }
}

pub fn detail_lines(data: &WorkbenchData, lang: Language, overlay: &str) -> Vec<String> {
    match overlay {
        "approval" => {
            let Some(approval) = &data.approval else {
                return vec![t(lang, Key::ApprovalNone).into()];
            };
            let mut lines = vec![
                approval.title.clone(),
                approval.summary.clone(),
                format!("{} · {} · {}", approval.tool, approval.kind, approval.risk),
                String::new(),
            ];
            for (label, body) in [
                (t(lang, Key::ApprovalAction), &approval.action),
                (t(lang, Key::ApprovalRequested), &approval.requested_action),
                (t(lang, Key::ApprovalBoundary), &approval.sandbox_boundary),
            ] {
                if !body.is_empty() {
                    lines.push(label.to_string());
                    lines.extend(body.lines().map(str::to_string));
                }
            }
            if !approval.rules.is_empty() {
                lines.push(t(lang, Key::ApprovalRules).into());
                for rule in &approval.rules {
                    lines.push(format!("{} · {}", rule.access, rule.path));
                }
            }
            lines
        }
        "diff" => {
            let Some(diff) = &data.diff else {
                return Vec::new();
            };
            if let Some(error) = &diff.error {
                return vec![error.clone()];
            }
            if diff.files.is_empty() {
                return vec![t(lang, Key::DiffClean).into()];
            }
            let mut lines = Vec::new();
            for (index, file) in diff.files.iter().enumerate() {
                let marker = if index == diff.selected { "›" } else { " " };
                lines.push(format!("{marker} {} {}", file.status, file.path));
            }
            lines.push(String::new());
            lines.extend(diff.patch.lines().map(str::to_string));
            lines
        }
        "worktree" => {
            if data.worktrees.is_empty() {
                return vec![t(lang, Key::WorktreeEmpty).into()];
            }
            data.worktrees
                .iter()
                .map(|row| {
                    format!(
                        "{}{} · {} · {}",
                        if row.current { "● " } else { "  " },
                        row.label,
                        row.description,
                        row.path
                    )
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

/// One selectable record, on exactly one screen row.
///
/// A list overlay is navigated with the arrow keys, so what a reader counts is
/// *rows*, not records: a title long enough to fold into two rows makes one
/// `down` move the marker two rows down the screen and the entries appear to
/// skip. Every record therefore owns one row by construction.
///
/// The pieces after the first are dropped **whole** when they no longer fit,
/// the same rule the usage panel follows: a workspace truncated mid-path reads
/// as a different directory, and half an id reads as a different id. Only the
/// first piece (the title, or the fork point's text) is shortened, and it ends
/// in an ellipsis so the cut is visible.
pub fn record_line(marker_on: bool, prefix: &str, pieces: &[&str], width: usize) -> String {
    let width = width.max(1);
    let marker = if marker_on { "› " } else { "  " };
    let first = pieces.first().copied().unwrap_or("").replace('\n', " ↵ ");
    let mut line = format!("{marker}{prefix}{first}");
    if crate::workbench::ansi::width(&line) > width {
        // The lead piece alone is too wide: shorten it, and keep the ellipsis
        // inside the row so the marker and its number stay put.
        return crate::workbench::ansi::fit(&line, width);
    }
    for piece in &pieces[1..] {
        let candidate = format!("{line} · {piece}");
        if crate::workbench::ansi::width(&candidate) > width {
            break;
        }
        line = candidate;
    }
    line
}

/// Soft-wrap by display cells and clamp the horizontal window to a byte
/// boundary. Long JSON and diff lines stay readable instead of being clipped.
pub fn wrapped(lines: &[String], width: usize, scroll: usize, height: usize) -> Vec<String> {
    wrapped_indexed(lines, width)
        .into_iter()
        .skip(scroll)
        .take(height)
        .map(|(_, row)| row)
        .collect()
}

/// Every row `lines` has at `width`, paired with the index of the line it was
/// wrapped from.
///
/// The pairing is what lets a pane paint a *wrapped* row by what its source line
/// is: a patch's `+` is not visible in the tail a wrap cut off, and the detail
/// pane would colour that tail as ordinary text.
pub fn wrapped_indexed(lines: &[String], width: usize) -> Vec<(usize, String)> {
    let width = width.max(1);
    let mut rows = Vec::new();
    for (source, line) in lines.iter().enumerate() {
        if line.is_empty() {
            rows.push((source, String::new()));
            continue;
        }
        let mut current = String::new();
        let mut used = 0;
        for grapheme in unicode_segmentation::UnicodeSegmentation::graphemes(line.as_str(), true) {
            let cells = crate::utils::grapheme_width(grapheme);
            if used + cells > width && !current.is_empty() {
                rows.push((source, std::mem::take(&mut current)));
                used = 0;
            }
            current.push_str(grapheme);
            used += cells;
        }
        if !current.is_empty() {
            rows.push((source, current));
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_is_one_row_and_drops_whole_pieces_before_shortening_one() {
        use crate::workbench::ansi::width;
        let pieces = [
            "refactor the overlay layout",
            "D:/future-os/.worktrees/tui-overlay",
            "20261026-000342-ebfc7c89507d4867b57705bd",
        ];
        let wide = record_line(true, "", &pieces, 200);
        assert_eq!(
            wide,
            "\u{203a} refactor the overlay layout \u{00b7} D:/future-os/.worktrees/tui-overlay \u{00b7} 20261026-000342-ebfc7c89507d4867b57705bd"
        );
        assert_eq!(width(&wide), wide.chars().count());

        // Tight: the workspace survives, the id is dropped whole — never half.
        let tight = record_line(false, "", &pieces, 80);
        assert!(tight.contains("tui-overlay"), "{tight}");
        assert!(!tight.contains("20261026"), "the id must go whole: {tight}");
        assert!(width(&tight) <= 80, "{tight}");
        assert!(
            tight.starts_with("  "),
            "an unselected row keeps its gutter"
        );

        // Tighter still: only the title, shortened with a visible ellipsis.
        let narrow = record_line(true, "", &pieces, 20);
        assert!(width(&narrow) <= 20, "{narrow}");
        assert!(narrow.contains('\u{2026}'), "the cut is visible: {narrow}");

        // A prefix (a fork number, a tree indent, a badge) is kept.
        let numbered = record_line(true, "12. ", &["short text"], 40);
        assert_eq!(numbered, "\u{203a} 12. short text");
    }

    #[test]
    fn approval_keeps_details_and_both_rule_shapes() {
        let approval = approval_of(&serde_json::json!({
            "approval_request_id": "a1", "tool_name": "shell", "risk_level": "high",
            "action": {"command": "git status", "paths": ["a b"]},
            "save_suggestion": {"rules": [{"path": "src/**", "access": "write"}]}
        }));
        assert_eq!(approval.request_id, "a1");
        assert!(approval.action.contains("git status"));
        assert_eq!(approval.rules[0].path, "src/**");
        let one =
            approval_of(&serde_json::json!({"saveSuggestion": {"path": "a", "access": "read"}}));
        assert_eq!(one.rules.len(), 1);
    }

    #[test]
    fn wrapping_never_splits_cjk_or_drops_the_tail() {
        let rows = wrapped(&["中文中文".into()], 4, 0, 10);
        assert_eq!(rows, vec!["中文", "中文"]);
    }
}
