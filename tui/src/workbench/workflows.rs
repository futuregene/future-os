//! Pure state and formatting for the workflow overlays.

use serde_json::Value;

use super::i18n::{t, Key, Language};
use super::state::{ApprovalView, SessionRow, WorkbenchData};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryMode {
    Search,
    Tree,
}

pub fn filtered_sessions<'a>(sessions: &'a [SessionRow], query: &str) -> Vec<&'a SessionRow> {
    let needle = query.trim().to_lowercase();
    sessions
        .iter()
        .filter(|row| {
            needle.is_empty()
                || [row.id.as_str(), row.title.as_str(), row.workspace.as_str()]
                    .iter()
                    .any(|text| text.to_lowercase().contains(&needle))
        })
        .collect()
}

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

/// A list of conversations as a **table**: the title, the workspace it belongs
/// to and its age, each in a column of its own.
///
/// [`record_line`] appends a value only while it fits, which is right for a
/// diff's metadata (where a cut value stays readable) and wrong for a list of
/// conversations: every row's workspace started wherever its title happened to
/// end, so the column a reader scans down was ragged — `#`'s session list read
/// as misaligned text rather than as a list. Here the two right-hand columns are
/// measured across the *whole* list and every row is padded into them.
///
/// Widths are the list's own, so a list of short titles gives the title more
/// room. The workspace column takes at most a third of the row: its name is a
/// hint about where a conversation lives, and the title is what the reader
/// came to read.
pub fn session_table(rows: &[&SessionRow], selected: usize, width: usize) -> Vec<String> {
    let width = width.max(1);
    let marker = 2usize;
    let age_width = rows
        .iter()
        .map(|row| crate::workbench::ansi::width(&row.age))
        .max()
        .unwrap_or(0);
    let workspace_width = rows
        .iter()
        .map(|row| crate::workbench::ansi::width(&row.workspace))
        .max()
        .unwrap_or(0)
        .min((width / 3).max(4));
    // The title gets what the two right-hand columns and their separators leave,
    // and at least a few cells of its own.
    let title_width = width
        .saturating_sub(marker + age_width + workspace_width + 2)
        .max(4);
    rows.iter()
        .enumerate()
        .map(|(index, row)| {
            let lead = if index == selected { "› " } else { "  " };
            let title = crate::workbench::ansi::clip(&row.title.replace('\n', " ↵ "), title_width);
            let title_pad =
                " ".repeat(title_width.saturating_sub(crate::workbench::ansi::width(&title)));
            // Right-aligned, so the last cell of every workspace name and every
            // age is the same cell.
            let workspace = crate::workbench::ansi::clip(&row.workspace, workspace_width);
            let workspace_pad = " "
                .repeat(workspace_width.saturating_sub(crate::workbench::ansi::width(&workspace)));
            let age_pad =
                " ".repeat(age_width.saturating_sub(crate::workbench::ansi::width(&row.age)));
            let line = format!(
                "{lead}{title}{title_pad} {workspace_pad}{workspace} {age_pad}{}",
                row.age
            );
            crate::workbench::ansi::fit(&line, width)
        })
        .collect()
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

    /// The session list is a *table*: the workspace and age of every row start
    /// in the same cell, whatever the title's length (and whatever its width in
    /// cells — a CJK title is two cells per character).
    #[test]
    fn a_session_table_lines_its_columns_up() {
        use crate::workbench::ansi::width;
        let owned: Vec<SessionRow> = [
            ("修复 dev 分支 CI", "future-os", "5m"),
            ("short", "serious_works", "3h"),
            ("a much longer title than the others", "future-os", "1w"),
            ("对话", "", "now"),
        ]
        .iter()
        .map(|(title, workspace, age)| SessionRow {
            id: title.to_string(),
            title: title.to_string(),
            workspace: workspace.to_string(),
            age: age.to_string(),
            ..SessionRow::default()
        })
        .collect();
        let rows: Vec<&SessionRow> = owned.iter().collect();
        let lines = session_table(&rows, 1, 60);
        assert_eq!(lines.len(), rows.len());
        for line in &lines {
            assert_eq!(width(line), 60, "every row is the box's width: {line:?}");
        }
        // The age ends flush with the row's right edge on every line, which is
        // what makes the column line up (byte offsets inside a CJK title are
        // not cells, so the tail is measured rather than indexed).
        for (line, row) in lines.iter().zip(rows.iter()) {
            let at = line.find(row.age.as_str()).expect("the age is painted");
            assert_eq!(
                width(&line[at..]),
                width(&row.age),
                "the age is not flush: {line:?}"
            );
        }
        // And the marker sits on the selected row only.
        assert!(lines[1].starts_with("› "), "{:?}", lines[1]);
        assert!(!lines[0].starts_with("› "), "{:?}", lines[0]);
    }

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
    fn history_search_covers_title_workspace_and_id_case_insensitively() {
        let rows = vec![
            SessionRow {
                id: "ABC".into(),
                title: "Fix parser".into(),
                workspace: "repo".into(),
                ..SessionRow::default()
            },
            SessionRow {
                id: "x".into(),
                title: "Other".into(),
                workspace: "articles".into(),
                ..SessionRow::default()
            },
        ];
        assert_eq!(filtered_sessions(&rows, "abc").len(), 1);
        assert_eq!(filtered_sessions(&rows, "PARSER").len(), 1);
        assert_eq!(filtered_sessions(&rows, "articles").len(), 1);
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
