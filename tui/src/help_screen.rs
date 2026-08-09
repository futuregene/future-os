//! Built-in help screen rendering — 1:1 port of `tui/src/help-screen.ts`.
//!
//! A pure function: takes a terminal width and returns the formatted help
//! card lines (ANSI-styled). The command list mirrors the slash commands
//! handled by `app.ts` (dispatch + autocomplete).

use crate::theme::{bold, fg};
use crate::utils::{truncate_to_width, visible_width, TruncateOptions};

struct HelpEntry {
    key: &'static str,
    desc: &'static str,
}

const SHORTCUTS: [HelpEntry; 9] = [
    HelpEntry {
        key: "ctrl+c",
        desc: "interrupt",
    },
    HelpEntry {
        key: "ctrl+p",
        desc: "cycle model",
    },
    HelpEntry {
        key: "ctrl+r",
        desc: "browse sessions",
    },
    HelpEntry {
        key: "ctrl+t",
        desc: "cycle thinking",
    },
    HelpEntry {
        key: "ctrl+o",
        desc: "expand/collapse thinking",
    },
    HelpEntry {
        key: "tab",
        desc: "autocomplete",
    },
    HelpEntry {
        key: "↑↓",
        desc: "scroll / navigate",
    },
    HelpEntry {
        key: "enter",
        desc: "submit / accept",
    },
    HelpEntry {
        key: "escape",
        desc: "close popup",
    },
];

const COMMANDS: [HelpEntry; 17] = [
    HelpEntry {
        key: "/model [name]",
        desc: "select model",
    },
    HelpEntry {
        key: "/new",
        desc: "start a new session",
    },
    HelpEntry {
        key: "/sessions",
        desc: "browse and switch sessions",
    },
    HelpEntry {
        key: "/compact",
        desc: "compress conversation context",
    },
    HelpEntry {
        key: "/scoped-models",
        desc: "configure model enable/disable list",
    },
    HelpEntry {
        key: "/clone",
        desc: "clone the current session",
    },
    HelpEntry {
        key: "/fork",
        desc: "fork the current session",
    },
    HelpEntry {
        key: "/tree",
        desc: "session tree with fork/clone hierarchy",
    },
    HelpEntry {
        key: "/name [n]",
        desc: "set the session name",
    },
    HelpEntry {
        key: "/status",
        desc: "session state, token usage, cost",
    },
    HelpEntry {
        key: "/stop",
        desc: "abort current generation",
    },
    HelpEntry {
        key: "/cwd",
        desc: "change the working directory",
    },
    HelpEntry {
        key: "/approve",
        desc: "approve pending tool execution",
    },
    HelpEntry {
        key: "/reject",
        desc: "reject pending tool execution",
    },
    HelpEntry {
        key: "/cancel <run-id>",
        desc: "cancel a queued run",
    },
    HelpEntry {
        key: "/reload",
        desc: "reload skills and context",
    },
    HelpEntry {
        key: "/help",
        desc: "show all commands and shortcuts",
    },
];

/// Render the help card at the given terminal width.
pub fn render_help(w: usize) -> Vec<String> {
    let dim_ = |t: &str| fg(245, t);
    let acc = |t: &str| fg(151, t);
    let bold_ = |t: &str| fg(252, &bold(t));

    let inner_w = w.saturating_sub(4); // card body width: 2 borders + 2-space gutter

    let mut lines: Vec<String> = Vec::new();
    // Push one card row: border + gutter + content + pad to body width +
    // border.
    let push = |lines: &mut Vec<String>, row: &str| {
        let clipped = if visible_width(row) > inner_w {
            truncate_to_width(row, inner_w, &TruncateOptions::default())
        } else {
            row.to_string()
        };
        lines.push(format!(
            "{}  {}{}{}",
            dim_("│"),
            clipped,
            " ".repeat(inner_w.saturating_sub(visible_width(&clipped))),
            dim_("│"),
        ));
    };

    lines.push(dim_(&format!("┌{}┐", "─".repeat(w.saturating_sub(2)))));
    lines.push(format!(
        "{}  {}  {}{}{}",
        dim_("│"),
        bold_("future-tui"),
        dim_("Terminal UI Help"),
        " ".repeat(inner_w.saturating_sub(28)),
        dim_("│"),
    ));
    lines.push(dim_(&format!("├{}┤", "─".repeat(w.saturating_sub(2)))));

    push(&mut lines, &acc("Shortcuts:"));
    for entry in SHORTCUTS.iter() {
        push(
            &mut lines,
            &dim_(&format!("{} {}", pad_end(entry.key, 8), entry.desc)),
        );
    }

    push(&mut lines, "");
    push(&mut lines, &acc("/commands:"));

    let key_w = COMMANDS
        .iter()
        .map(|c| visible_width(c.key))
        .max()
        .unwrap_or(0);
    for entry in COMMANDS.iter() {
        push(
            &mut lines,
            &dim_(&format!("{}{}", pad_end(entry.key, key_w + 2), entry.desc)),
        );
    }

    lines.push(dim_(&format!("└{}┘", "─".repeat(w.saturating_sub(2)))));
    lines
}

/// JS `str.padEnd(len)` — pads with spaces to the given length (no-op when
/// already at/over length).
fn pad_end(s: &str, len: usize) -> String {
    let visible = visible_width(s);
    if visible >= len {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(len - visible))
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::strip_ansi_codes;

    const EXPECTED_COMMANDS: [&str; 17] = [
        "/model [name]",
        "/new",
        "/sessions",
        "/compact",
        "/scoped-models",
        "/clone",
        "/fork",
        "/tree",
        "/name [n]",
        "/status",
        "/stop",
        "/cwd",
        "/approve",
        "/reject",
        "/cancel <run-id>",
        "/reload",
        "/help",
    ];

    #[test]
    fn lists_every_slash_command_handled_by_the_tui() {
        let text = render_help(80)
            .iter()
            .map(|l| strip_ansi_codes(l))
            .collect::<Vec<_>>()
            .join("\n");
        for cmd in EXPECTED_COMMANDS {
            assert!(text.contains(cmd), "missing {cmd}");
        }
    }

    #[test]
    fn renders_every_row_at_exactly_the_requested_width() {
        for width in [40usize, 60, 80, 120] {
            let rows: Vec<usize> = render_help(width)
                .iter()
                .map(|l| visible_width(l))
                .collect();
            let mut unique = rows.clone();
            unique.dedup();
            assert_eq!(unique.len(), 1, "rows not uniform at width {width}");
            assert_eq!(rows[0], width);
        }
    }

    #[test]
    fn keeps_ansi_codes_intact_no_dangling_escapes_after_truncation() {
        for width in [40usize, 80] {
            for line in render_help(width) {
                assert!(!strip_ansi_codes(&line).contains('\x1b'));
            }
        }
    }
}
