//! Built-in help screen rendering — 1:1 port of `tui/src/help-screen.ts`.
//!
//! A pure function: takes a terminal width and returns the formatted help
//! card lines (ANSI-styled). The command list mirrors the slash commands
//! handled by `app.ts` (dispatch + autocomplete).
//!
//! The card is 65 rows tall — taller than a default 80×36 pane — and the
//! original port rendered all of it and let the overlay compositor clip the
//! tail, so every command below the fold was unreachable *and* invisible. The
//! card therefore also has a viewport ([`render_help_window`]): the frame rows
//! (top border, header, separator, bottom border) stay pinned, the body
//! scrolls, and a hint row says how much body is left in whichever direction
//! the user is not looking.

use crate::theme::{bold, fg};
use crate::utils::{truncate_to_width, visible_width, TruncateOptions};

struct HelpEntry {
    key: &'static str,
    desc: &'static str,
}

const SHORTCUTS: [HelpEntry; 13] = [
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
        key: "ctrl+g",
        desc: "expand/collapse tool output",
    },
    HelpEntry {
        key: "ctrl+d",
        desc: "compact view (fold tool runs and thinking)",
    },
    HelpEntry {
        key: "ctrl+x",
        desc: "copy the last answer",
    },
    HelpEntry {
        key: "ctrl+v",
        desc: "paste clipboard (image or text)",
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

/// Rows the card spends on its frame: the top border, the header row, the
/// separator and the bottom border. The rest of the card is body.
pub const HELP_FRAME_ROWS: usize = 4;

/// Two-space gutter between the card's border and its text.
const GUTTER: usize = 2;

/// The `/commands:` block, **in alphabetical order by command name**.
///
/// The name is the first whitespace-separated token of `key`
/// (`"/model [name]"` → `"/model"`), never the whole string: sorting on the
/// whole `key` would order `/model` against its own argument text, so the
/// relative position of two entries would depend on their parameters. The
/// order is a contract, not a convenience — a user looking a command up reads
/// this list top-down — so `command_card_is_sorted_by_command_name` fails
/// when an entry is inserted out of place.
///
/// [`SHORTCUTS`] above is deliberately *not* sorted: it is grouped by what the
/// keys act on (interrupt first, then the cycling keys, then paging), which is
/// how a key table is read.
const COMMANDS: [HelpEntry; 43] = [
    HelpEntry {
        key: "/agent",
        desc: "agent version and instance info",
    },
    HelpEntry {
        key: "/approve",
        desc: "approve pending tool execution",
    },
    HelpEntry {
        key: "/autocompact [on|off]",
        desc: "toggle automatic compaction",
    },
    HelpEntry {
        key: "/autoretry [on|off]",
        desc: "toggle automatic retry",
    },
    HelpEntry {
        key: "/cancel <run-id>",
        desc: "cancel a queued run",
    },
    HelpEntry {
        key: "/clone",
        desc: "clone the current session",
    },
    HelpEntry {
        key: "/compact",
        desc: "compress conversation context",
    },
    HelpEntry {
        key: "/context [on|off]",
        desc: "list or toggle the workspace context files",
    },
    HelpEntry {
        key: "/copy",
        desc: "copy the last assistant message",
    },
    HelpEntry {
        key: "/cwd",
        desc: "change the working directory",
    },
    HelpEntry {
        key: "/delete [--yes]",
        desc: "delete the current session",
    },
    HelpEntry {
        key: "/editor",
        desc: "edit the draft in $VISUAL/$EDITOR",
    },
    HelpEntry {
        key: "/export",
        desc: "export the session to an HTML file",
    },
    HelpEntry {
        key: "/fork",
        desc: "fork the current session",
    },
    HelpEntry {
        key: "/help",
        desc: "show all commands and shortcuts",
    },
    HelpEntry {
        key: "/history <query>",
        desc: "search the session history",
    },
    HelpEntry {
        key: "/keymap",
        desc: "view and rebind keyboard shortcuts",
    },
    HelpEntry {
        key: "/metrics",
        desc: "runtime metrics of this session",
    },
    HelpEntry {
        key: "/model [name]",
        desc: "select model",
    },
    HelpEntry {
        key: "/models",
        desc: "model scope (enter) / default",
    },
    HelpEntry {
        key: "/name [n]",
        desc: "set the session name",
    },
    HelpEntry {
        key: "/new",
        desc: "start a new session",
    },
    HelpEntry {
        key: "/permission [level]",
        desc: "tool permissions; or set the level",
    },
    HelpEntry {
        key: "/provider-key <id>",
        desc: "set or clear a provider API key",
    },
    HelpEntry {
        key: "/providers",
        desc: "providers, API keys, model sync",
    },
    HelpEntry {
        key: "/reject",
        desc: "reject pending tool execution",
    },
    HelpEntry {
        key: "/reload",
        desc: "reload skills and context",
    },
    HelpEntry {
        key: "/sandbox",
        desc: "approval tier + tool permissions (one card)",
    },
    HelpEntry {
        key: "/scoped-models",
        desc: "configure model enable/disable list",
    },
    HelpEntry {
        key: "/sessions",
        desc: "browse and switch sessions",
    },
    HelpEntry {
        key: "/shell <cmd>",
        desc: "run a shell command via the agent",
    },
    HelpEntry {
        key: "/skill-recommend",
        desc: "offer a fitting skill before sending (on|off)",
    },
    HelpEntry {
        key: "/skills",
        desc: "browse, insert and install skills",
    },
    HelpEntry {
        key: "/snapshot",
        desc: "diagnostic snapshot of the current run",
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
        key: "/theme",
        desc: "pick a color theme",
    },
    HelpEntry {
        key: "/title [zh|en]",
        desc: "generate and apply a session title",
    },
    HelpEntry {
        key: "/tool-output [call-id]",
        desc: "list tool calls or show one output",
    },
    HelpEntry {
        key: "/tools",
        desc: "enable or disable tools",
    },
    HelpEntry {
        key: "/transcript",
        desc: "full transcript with search and copy",
    },
    HelpEntry {
        key: "/tree",
        desc: "session tree with fork/clone hierarchy",
    },
    HelpEntry {
        key: "/usage",
        desc: "tokens, cost and context usage",
    },
];

/// Render the help card at the given terminal width.
pub fn render_help(w: usize) -> Vec<String> {
    let acc = |t: &str| fg(151, t);
    let bold_ = |t: &str| fg(252, &bold(t));

    let inner_w = card_body_width(w);

    let mut lines: Vec<String> = Vec::new();
    // Push one card row: border + gutter + content + pad to body width +
    // border.
    let push = |lines: &mut Vec<String>, row: &str| lines.push(card_row(row, inner_w));

    lines.push(dim(&format!("┌{}┐", "─".repeat(w.saturating_sub(2)))));
    lines.push(card_row(
        &format!("{}  {}", bold_("future-tui"), dim("Terminal UI Help")),
        inner_w,
    ));
    lines.push(dim(&format!("├{}┤", "─".repeat(w.saturating_sub(2)))));

    push(&mut lines, &acc("Shortcuts:"));
    for entry in SHORTCUTS.iter() {
        push(
            &mut lines,
            &dim(&format!("{} {}", pad_end(entry.key, 8), entry.desc)),
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
            &dim(&format!("{}{}", pad_end(entry.key, key_w + 2), entry.desc)),
        );
    }

    lines.push(dim(&format!("└{}┘", "─".repeat(w.saturating_sub(2)))));
    lines
}

/// One card row: left border, gutter, `content`, pad to the body width,
/// right border.
///
/// `content` may carry ANSI; only its *visible* width is measured. Content
/// wider than the body is truncated (never spilled past the right border) and
/// the ANSI reset stays balanced (`truncate_to_width` owns that).
fn card_row(content: &str, inner_w: usize) -> String {
    let clipped = truncate_to_width(content, inner_w, &TruncateOptions::default());
    format!(
        "{}  {}{}{}",
        dim("│"),
        clipped,
        " ".repeat(inner_w.saturating_sub(visible_width(&clipped))),
        dim("│"),
    )
}

/// The card's body width: the terminal width minus the two borders and the
/// two-space gutter.
fn card_body_width(w: usize) -> usize {
    w.saturating_sub(GUTTER + 2)
}

/// Card row color (the dim gray every non-accent row uses).
fn dim(t: &str) -> String {
    fg(245, t)
}

// ─── Viewport ─────────────────────────────────────────────────────────────

/// One screenful of the card: the rows to paint plus where they were taken
/// from, so the caller can keep its own scroll position in range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpWindow {
    /// Exactly the rows to paint (never more than the requested height).
    pub lines: Vec<String>,
    /// First body row shown, clamped into `0..=max_scroll`.
    pub scroll: usize,
    /// Largest legal scroll for this height.
    pub max_scroll: usize,
    /// Body rows the viewport shows at this height (`0` when only the frame
    /// and the hint row fit).
    pub page: usize,
}

/// The card clipped to `height` rows, scrolled to body row `scroll`.
///
/// * A card that fits is returned whole, unscrolled: the overlay must not
///   shift or re-frame a terminal that can already show everything.
/// * Otherwise the frame stays put — top border, header and separator are
///   always the first three rows, the bottom border is always the last — and
///   the body between them is a window. One row is spent on the
///   [`more_row`] hint, because a window with no way to tell that it is a
///   window is exactly the defect this viewport exists to fix.
/// * At `height <= HELP_FRAME_ROWS` there is no room for both a body row and
///   the hint, so the hint is dropped and the frame is drawn alone (the
///   compositor clips it further; a viewport cannot do better than that).
pub fn render_help_window(width: usize, height: usize, scroll: usize) -> HelpWindow {
    let card = render_help(width);
    // `render_help` always emits the three frame rows, at least one body row
    // (the shortcut list) and the bottom border, so the split below is safe
    // for every width, `0` included.
    let body_len = card.len().saturating_sub(HELP_FRAME_ROWS);
    if card.len() <= height {
        return HelpWindow {
            lines: card,
            scroll: 0,
            max_scroll: 0,
            page: body_len,
        };
    }
    let body: &[String] = &card[HELP_FRAME_ROWS - 1..card.len() - 1];
    let inner_w = card_body_width(width);
    let page = height.saturating_sub(HELP_FRAME_ROWS + 1);
    // `page.max(1)` keeps the window scrollable on a frame-only viewport
    // (where `page` is 0): the hint row still counts the body around it.
    let max_scroll = body.len().saturating_sub(page.max(1));
    let scroll = scroll.min(max_scroll);
    let shown = &body[scroll..(scroll + page).min(body.len())];
    let mut lines: Vec<String> = card[..HELP_FRAME_ROWS - 1].to_vec();
    lines.extend(shown.iter().cloned());
    if height > HELP_FRAME_ROWS {
        lines.push(more_row(inner_w, scroll, body.len() - scroll - shown.len()));
    }
    lines.push(card[card.len() - 1].clone());
    HelpWindow {
        lines,
        scroll,
        max_scroll,
        page,
    }
}

/// The card row that stands in for the body rows the window does not show.
///
/// `above`/`below` count body rows on either side of the window, so the hint
/// always says which direction still has content — the arrow alone cannot:
/// scrolled to the end, `↓` would promise rows that do not exist.
fn more_row(inner_w: usize, above: usize, below: usize) -> String {
    let hint = match (above, below) {
        (0, below) => format!("  ↓ {below} more · ↑↓ scroll"),
        (above, 0) => format!("  ↑ {above} above · ↑↓ scroll"),
        (above, below) => format!("  ↑ {above} above · ↓ {below} more · ↑↓ scroll"),
    };
    card_row(&dim(&hint), inner_w)
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

    /// The commands the card must list, as a set the test maintains by hand so
    /// it cannot silently follow a change to `COMMANDS` itself. Membership
    /// only — the order is `command_card_is_sorted_by_command_name`'s business.
    const EXPECTED_COMMANDS: [&str; 43] = [
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
        "/theme",
        "/providers",
        "/models",
        "/skills",
        "/skill-recommend",
        "/tools",
        "/usage",
        "/transcript",
        "/copy",
        "/editor",
        "/provider-key <id>",
        "/export",
        "/agent",
        "/history <query>",
        "/autocompact [on|off]",
        "/autoretry [on|off]",
        "/tool-output [call-id]",
        "/sandbox",
        "/permission [level]",
        "/shell <cmd>",
        "/title [zh|en]",
        "/metrics",
        "/snapshot",
        "/context [on|off]",
        "/delete [--yes]",
        "/keymap",
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

    /// The key a card row is *sorted* by: the command name alone, never the
    /// argument hint that follows it.
    ///
    /// `"/model [name]"` → `"/model"`. Sorting on the whole `key` would put
    /// a command carrying a parameter wherever its parameter text lands
    /// relative to its neighbours, which is not an order a reader can use.
    fn sort_key(key: &str) -> &str {
        key.split_whitespace().next().unwrap_or("")
    }

    /// The `/commands:` block is in alphabetical order, and the order is a
    /// contract — not a by-product of the order the entries were written in.
    ///
    /// Checked over the *whole* array rather than at its ends: `first < last`
    /// holds for almost any permutation, so it proves nothing about the 43
    /// rows between them, while a single entry inserted in the wrong place is
    /// exactly the failure a user notices (they scan the list top-down).
    #[test]
    fn command_card_is_sorted_by_command_name() {
        let names: Vec<&str> = COMMANDS.iter().map(|c| sort_key(c.key)).collect();
        // The offending pair is collected rather than formatted inside the
        // `assert!`: a panic-message argument is evaluated only when the
        // assertion fails, so it would sit in the coverage report as a line no
        // test can ever reach. Collecting runs on every call, and the message
        // stays just as specific.
        let out_of_order: Vec<[&str; 2]> = names
            .windows(2)
            .filter(|pair| pair[0] > pair[1])
            .map(|pair| [pair[0], pair[1]])
            .collect();
        assert!(out_of_order.is_empty(), "out of order: {out_of_order:?}");
        // Two rows with the same command name render as the same command
        // twice, and one of them is dead: the user cannot tell which.
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "duplicate command: {names:?}");
        // The card covers exactly the hand-maintained set, so a row added to
        // one list and not the other fails here instead of shipping silently.
        // Both sides are reduced to the sort key first: the argument hints are
        // the row's text, not its identity.
        let mut expected: Vec<&str> = EXPECTED_COMMANDS.iter().map(|key| sort_key(key)).collect();
        expected.sort_unstable();
        assert_eq!(names, expected, "card and EXPECTED_COMMANDS disagree");
    }

    #[test]
    fn lists_the_new_shortcuts() {
        let text = render_help(80)
            .iter()
            .map(|l| strip_ansi_codes(l))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("ctrl+g"), "{text}");
        assert!(text.contains("expand/collapse tool output"), "{text}");
        assert!(text.contains("ctrl+x"), "{text}");
        assert!(text.contains("copy the last answer"), "{text}");
        // The paste key belongs here too: it is the only way to attach a
        // clipboard image, and the card is where a user looks it up.
        assert!(text.contains("ctrl+v"), "{text}");
        assert!(text.contains("paste clipboard (image or text)"), "{text}");
    }

    /// `/skills` opens a browser that inserts *and* installs, so its help row
    /// has to say so — the card is where a user looks up what the panel can do.
    #[test]
    fn skills_row_mentions_installing() {
        let text = render_help(80)
            .iter()
            .map(|l| strip_ansi_codes(l))
            .collect::<Vec<_>>()
            .join("\n");
        let row = text
            .lines()
            .find(|line| line.contains("/skills"))
            .expect("the command card lists /skills");
        assert!(
            row.contains("install"),
            "the row says what the panel does: {row}"
        );
        assert!(row.contains("insert"), "and it still browses: {row}");
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

    /// All of a window's rows as one ANSI-free string (what the user reads).
    fn plain_window(width: usize, height: usize, scroll: usize) -> String {
        render_help_window(width, height, scroll)
            .lines
            .iter()
            .map(|row| strip_ansi_codes(row))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The card is 65 rows tall and a default pane is 36: the viewport is what
    /// decides how much of it exists. A terminal that can already show the
    /// whole card must get it back untouched — no hint row, no re-framing.
    #[test]
    fn a_card_that_fits_is_returned_whole() {
        let card = render_help(80);
        let window = render_help_window(80, card.len(), 0);
        assert_eq!(window.lines, card);
        assert_eq!((window.scroll, window.max_scroll), (0, 0));
        assert_eq!(window.page, card.len() - HELP_FRAME_ROWS);
        // A taller terminal, and a stale scroll position, are both a no-op.
        assert_eq!(render_help_window(80, 400, 3).lines, render_help(80));
    }

    /// 80×36 — the pane both the screenshots and the tmux gate use. The frame
    /// is pinned, the tail is not silently gone, and the hint says how much
    /// body is left below.
    #[test]
    fn a_short_viewport_pins_the_frame_and_counts_what_is_left() {
        let card = render_help(80);
        let window = render_help_window(80, 36, 0);
        assert_eq!(window.lines.len(), 36, "exactly the pane's height");
        assert_eq!(&window.lines[..3], &card[..3], "the frame stays pinned");
        assert_eq!(window.lines.last(), card.last(), "…bottom border included");
        assert_eq!(window.page, 36 - HELP_FRAME_ROWS - 1, "one row is the hint");
        assert_eq!(window.scroll, 0);
        let hint = strip_ansi_codes(&window.lines[34]);
        assert!(hint.contains("more · ↑↓ scroll"), "{hint}");
        assert!(
            !hint.contains("above"),
            "nothing is above the first window: {hint}"
        );
    }

    /// Scrolling to the end reaches the commands this branch added — the whole
    /// reason the card has a viewport — and the hint stops promising rows that
    /// do not exist.
    #[test]
    fn scrolling_to_the_end_reaches_the_last_commands() {
        let top = render_help_window(80, 36, 0);
        let bottom = render_help_window(80, 36, top.max_scroll);
        assert_eq!(bottom.scroll, top.max_scroll);
        assert_eq!(bottom.max_scroll, top.max_scroll);
        let last = plain_window(80, 36, top.max_scroll);
        assert!(last.contains("/usage"), "{last}");
        assert!(last.contains("/tree"), "{last}");
        assert!(
            !plain_window(80, 36, 0).contains("/usage"),
            "the first window cannot see the tail"
        );
        let hint = strip_ansi_codes(&bottom.lines[34]).to_string();
        assert!(hint.contains("above · ↑↓ scroll"), "{hint}");
        assert!(!hint.contains("more"), "no rows are left below: {hint}");
    }

    /// A window in the middle of the body counts both directions, so `↑` and
    /// `↓` never lie about where the rest of the card is.
    #[test]
    fn a_middle_window_counts_both_directions() {
        let middle = render_help_window(80, 36, 15);
        assert_eq!(middle.scroll, 15);
        let hint = strip_ansi_codes(&middle.lines[34]);
        assert!(hint.contains("↑ 15 above"), "{hint}");
        // The "below" count is derived from the card, not spelled out: the card
        // grows whenever a command is added (`/keymap` was the most recent one),
        // and a hard-coded number here only measures the last edit to this file.
        // The window knows what it shows, so what is left below is arithmetic.
        let body_rows = render_help(80).len() - HELP_FRAME_ROWS;
        let below = body_rows - middle.scroll - middle.page;
        assert!(hint.contains(&format!("↓ {below} more")), "{hint}");
        assert!(hint.contains("↑↓ scroll"), "{hint}");
        // Scrolling past the end is a clamp, not a panic and not a blank page.
        let clamped = render_help_window(80, 36, 9_999);
        assert_eq!(clamped.scroll, clamped.max_scroll);
        assert!(plain_window(80, 36, 9_999).contains("/usage"));
    }

    /// A viewport too short for even one body row drops the hint (rather than
    /// rendering a row past its height) and still closes the frame.
    #[test]
    fn a_frame_only_viewport_drops_the_hint_but_keeps_the_border() {
        let card = render_help(80);
        let window = render_help_window(80, HELP_FRAME_ROWS, 0);
        assert_eq!(window.lines.len(), HELP_FRAME_ROWS);
        assert_eq!(window.page, 0);
        assert_eq!(window.lines.last(), card.last(), "the frame is closed");
        assert!(!strip_ansi_codes(&window.lines.join("\n")).contains("more"));
        // One row more is enough for the hint.
        let taller = render_help_window(80, HELP_FRAME_ROWS + 1, 0);
        assert_eq!(taller.lines.len(), HELP_FRAME_ROWS + 1);
        assert!(strip_ansi_codes(&taller.lines[3]).contains("more"));
    }

    /// Every window row measures exactly the requested width, whatever the
    /// height, scroll or width — the overlay compositor paints them as-is.
    #[test]
    fn every_window_row_measures_the_requested_width() {
        for width in [40usize, 80, 120] {
            for height in [0usize, 3, 4, 5, 12, 36, 100] {
                for scroll in [0usize, 7, 9_999] {
                    for row in render_help_window(width, height, scroll).lines {
                        assert_eq!(
                            visible_width(&row),
                            width,
                            "{width}x{height} scrolled to {scroll}"
                        );
                    }
                }
            }
        }
    }

    /// `/sandbox` and `/permission` are two doors into one card, and the card
    /// is the only place that can say so — the two rows used to describe two
    /// different panels.
    #[test]
    fn the_sandbox_and_permission_rows_name_the_shared_card() {
        let text = render_help(80)
            .iter()
            .map(|l| strip_ansi_codes(l))
            .collect::<Vec<_>>()
            .join("\n");
        let row = |cmd: &str| {
            text.lines()
                .find(|line| line.contains(cmd))
                .unwrap_or_else(|| panic!("no {cmd} row in the card"))
                .to_string()
        };
        let sandbox = row("/sandbox");
        assert!(sandbox.contains("one card"), "{sandbox}");
        assert!(sandbox.contains("tool permissions"), "{sandbox}");
        let permission = row("/permission");
        assert!(permission.contains("tool permissions"), "{permission}");
        assert!(permission.contains("level"), "{permission}");
    }

    #[test]
    fn pad_end_pads_and_leaves_long_strings_untouched() {
        assert_eq!(pad_end("ab", 4), "ab  ");
        assert_eq!(pad_end("abcde", 5), "abcde");
        assert_eq!(pad_end("abcdef", 3), "abcdef");
    }
}
