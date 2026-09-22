//! Diff parsing and rendering for tool output (`edit`, `write`, patch tools).
//!
//! The agent streams two shapes of patch text: classic unified diffs
//! (`---`/`+++` headers, `@@ -a,b +c,d @@` hunks) and `apply_patch` envelopes
//! (`*** Begin Patch` … `*** Update File:`/`*** Add File:`/
//! `*** Delete File:` … `*** End Patch`). Both are parsed into one flat
//! [`DiffLine`] stream — a shape the chat area can render directly, count, or
//! collapse into a one-line summary.
//!
//! `apply_patch` is a *provider* tool, not one of ours: it is declared as a
//! FREEFORM custom tool with a Lark grammar (`start: begin_patch hunk+
//! end_patch`, `*** Update File:`, `*** Add File:`, `*** Delete File:`, …), and
//! the definition the provider sends is captured in this repo under
//! `tests/provider-protocol/fixtures/` — see the `custom_grammar_tool` fixtures.
//! A model that is handed that tool emits this shape through our provider layer,
//! so it has to render.
//!
//! Design notes:
//!
//! * **Hunk budgets drive classification.** A line is content only while the
//!   enclosing hunk header still expects old/new lines; once both counters hit
//!   zero the hunk is over, so a following `--- path` is a *file header* again
//!   (this is what makes multi-file unified diffs parse without guessing at
//!   `a/`/`b/` prefixes, and it also keeps a `-- sql comment` line inside a hunk
//!   from being mistaken for a header). Malformed input never panics: an
//!   unparsable `@@` becomes [`DiffLineKind::Meta`] and the parser keeps going.
//! * **`apply_patch` carries no line numbers** (`@@` there separates regions and
//!   holds a context label, not a range). We do not fabricate them — those lines
//!   get `old_no`/`new_no` of `None`, and [`render_diff`] drops the number gutter
//!   entirely when no line in the block has a number.
//! * **Rendering is ANSI-safe and column-exact.** Every produced row has a
//!   visible width of exactly `width` (using [`crate::utils`] for grapheme
//!   widths, truncation and background painting), so callers can pad, overlay or
//!   diff rows without measuring them first.
//!
//! Rendering conventions (all of them asserted by the tests below rather than
//! inherited from another project): a line-number gutter, tinted add/remove
//! backgrounds, and tabs expanded to a fixed column count. The single gutter
//! cell shows the old number for removals and the new number otherwise, which
//! is what our desktop `components/ui/DiffView.tsx` does too. The UI structure
//! here is our own: plain styled rows, not a widget from a TUI framework.

use crate::theme::C;
use crate::tui::{BOLD, CSI, RESET};
use crate::utils::{
    apply_background_to_line, replace_tabs, strip_ansi_codes, truncate_to_width, visible_width,
    TruncateOptions,
};

// ─── Constants ─────────────────────────────────────────────────────────────

/// Unified-diff trailer marking a file whose last line has no newline.
const NO_NEWLINE_MARKER: &str = "\\ No newline at end of file";
/// Columns a tab expands to. Diff output is column-sensitive, so the value has
/// to be fixed rather than dependent on the terminal: 4 keeps a deep indent from
/// eating the gutter, and it is what the source editors this renders for use.
const TAB_WIDTH: usize = 4;
/// Width of the line-number gutter cell (`   12 `).
const GUTTER_WIDTH: usize = 5;
/// Below this total width the gutter is dropped so content still fits.
const MIN_WIDTH_FOR_GUTTER: usize = 10;

const AP_BEGIN: &str = "*** Begin Patch";
const AP_END: &str = "*** End Patch";
const AP_EOF: &str = "*** End of File";
const AP_ADD: &str = "*** Add File: ";
const AP_UPDATE: &str = "*** Update File: ";
const AP_DELETE: &str = "*** Delete File: ";
const AP_MOVE: &str = "*** Move to: ";

// ─── Data model ────────────────────────────────────────────────────────────

/// What a parsed diff line is.
///
/// Terminal colours and backgrounds are keyed off this in [`render_diff`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiffLineKind {
    /// `+…` — present in the new file only.
    Add,
    /// `-…` — present in the old file only.
    Remove,
    /// ` …` (or an empty line inside a hunk) — unchanged.
    Context,
    /// `@@ -a,b +c,d @@ …` hunk header (the `@@ …` separator in `apply_patch`).
    Hunk,
    /// Metadata: `diff --git`, `index`, file modes, `*** Begin Patch`, …
    Meta,
    /// `\ No newline at end of file`.
    NoNewline,
    /// `---`/`+++` (unified) or `*** … File:` / `*** Move to:` (`apply_patch`).
    FileHeader,
}

/// One parsed diff line. `text` is the line **as written** (prefixes included),
/// CR-free, so rendering can show it verbatim.
///
/// `old_no`/`new_no` are the 1-based line numbers the line occupies in the old
/// and new file. They are `None` outside a hunk, for the marker/header lines,
/// and for every `apply_patch` line (the format has no numbers to read).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
}

impl DiffLine {
    fn plain(kind: DiffLineKind, text: &str) -> Self {
        Self {
            kind,
            text: text.to_string(),
            old_no: None,
            new_no: None,
        }
    }
}

/// 256-colour styling for a rendered diff block. `*_bg` of `None` leaves the
/// line transparent (no `48;5;` sequence is emitted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffTheme {
    pub add_fg: u8,
    pub remove_fg: u8,
    pub hunk_fg: u8,
    pub meta_fg: u8,
    pub add_bg: Option<u8>,
    pub remove_bg: Option<u8>,
}

/// Crate-palette default (`crate::theme::C`), untinted — matches the plain
/// chat-area rendering where backgrounds are reserved for selection.
pub const DEFAULT_DIFF_THEME: DiffTheme = DiffTheme {
    add_fg: C.green,
    remove_fg: C.red,
    hunk_fg: C.cyan,
    meta_fg: C.gray,
    add_bg: None,
    remove_bg: None,
};

impl Default for DiffTheme {
    fn default() -> Self {
        DEFAULT_DIFF_THEME
    }
}

/// Aggregate counts for a parsed diff block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiffStats {
    pub added: usize,
    pub removed: usize,
    pub files: usize,
}

// ─── Public parsing API ────────────────────────────────────────────────────

/// Parses a classic unified diff. Tolerates missing file headers, CRLF, a
/// trailing `\ No newline…` marker and out-of-budget lines; never panics.
pub fn parse_unified_diff(text: &str) -> Vec<DiffLine> {
    let mut out: Vec<DiffLine> = Vec::new();
    let mut in_hunk = false;
    let mut rem_old = 0u32;
    let mut rem_new = 0u32;
    let mut old_no = 0u32;
    let mut new_no = 0u32;

    for line in split_diff_lines(text) {
        if let Some(range) = parse_hunk_header(line) {
            old_no = range.old_start;
            new_no = range.new_start;
            rem_old = range.old_count;
            rem_new = range.new_count;
            // A `-0,0 +0,0` hunk is empty; do not enter hunk mode for it.
            in_hunk = rem_old > 0 || rem_new > 0;
            out.push(DiffLine {
                kind: DiffLineKind::Hunk,
                text: line.to_string(),
                old_no: Some(range.old_start),
                new_no: Some(range.new_start),
            });
            continue;
        }
        if is_no_newline_marker(line) {
            // Attaches to the previous content line: consumes no budget.
            out.push(DiffLine::plain(DiffLineKind::NoNewline, line));
            continue;
        }
        if in_hunk {
            if line.starts_with('+') && rem_new > 0 {
                out.push(DiffLine {
                    kind: DiffLineKind::Add,
                    text: line.to_string(),
                    old_no: None,
                    new_no: Some(new_no),
                });
                new_no = new_no.saturating_add(1);
                rem_new -= 1;
                continue;
            }
            if line.starts_with('-') && rem_old > 0 {
                out.push(DiffLine {
                    kind: DiffLineKind::Remove,
                    text: line.to_string(),
                    old_no: Some(old_no),
                    new_no: None,
                });
                old_no = old_no.saturating_add(1);
                rem_old -= 1;
                continue;
            }
            if (line.starts_with(' ') || line.is_empty()) && rem_old > 0 && rem_new > 0 {
                out.push(DiffLine {
                    kind: DiffLineKind::Context,
                    text: line.to_string(),
                    old_no: Some(old_no),
                    new_no: Some(new_no),
                });
                old_no = old_no.saturating_add(1);
                new_no = new_no.saturating_add(1);
                rem_old -= 1;
                rem_new -= 1;
                continue;
            }
            // Line does not fit the declared hunk counts (malformed patch, or
            // the hunk simply ended) — fall through and re-classify below.
            in_hunk = false;
        }
        if is_unified_file_header(line) {
            out.push(DiffLine::plain(DiffLineKind::FileHeader, line));
        } else if line.starts_with('+') {
            // `+++` without a path, or an add line outside any hunk: keep the
            // content, drop the number we cannot know.
            out.push(DiffLine::plain(DiffLineKind::Add, line));
        } else if line.starts_with('-') {
            out.push(DiffLine::plain(DiffLineKind::Remove, line));
        } else if !line.is_empty() {
            out.push(DiffLine::plain(DiffLineKind::Meta, line));
        }
    }
    out
}

/// Parses an `apply_patch` envelope (the FREEFORM provider tool described in
/// the module docs). Unknown `*** …` directives and stray
/// lines degrade to [`DiffLineKind::Meta`]; never panics.
pub fn parse_apply_patch(text: &str) -> Vec<DiffLine> {
    let mut out: Vec<DiffLine> = Vec::new();
    let mut in_file = false;

    for line in split_diff_lines(text) {
        if is_apply_patch_marker(line) {
            out.push(DiffLine::plain(DiffLineKind::Meta, line));
            in_file = false;
            continue;
        }
        if line.starts_with("*** ") {
            if apply_patch_directive(line).is_some() {
                out.push(DiffLine::plain(DiffLineKind::FileHeader, line));
                in_file = true;
            } else {
                out.push(DiffLine::plain(DiffLineKind::Meta, line));
            }
            continue;
        }
        if line.starts_with("@@") {
            // Region separator; in this format it carries a context label
            // (`@@ fn main() {`), never a line range.
            out.push(DiffLine::plain(DiffLineKind::Hunk, line));
            continue;
        }
        if is_no_newline_marker(line) {
            out.push(DiffLine::plain(DiffLineKind::NoNewline, line));
            continue;
        }
        if line.starts_with('+') {
            out.push(DiffLine::plain(DiffLineKind::Add, line));
            continue;
        }
        if line.starts_with('-') {
            out.push(DiffLine::plain(DiffLineKind::Remove, line));
            continue;
        }
        if line.is_empty() {
            if in_file {
                out.push(DiffLine::plain(DiffLineKind::Context, line));
            }
            continue;
        }
        if line.starts_with(' ') {
            out.push(DiffLine::plain(DiffLineKind::Context, line));
        } else {
            out.push(DiffLine::plain(DiffLineKind::Meta, line));
        }
    }
    out
}

/// Sniffs the format and dispatches to [`parse_apply_patch`] or
/// [`parse_unified_diff`]. Empty input yields an empty block.
pub fn parse_any_diff(text: &str) -> Vec<DiffLine> {
    if text.is_empty() {
        return Vec::new();
    }
    if is_apply_patch(text) {
        parse_apply_patch(text)
    } else {
        parse_unified_diff(text)
    }
}

/// True when the text contains an `apply_patch` envelope marker.
pub fn is_apply_patch(text: &str) -> bool {
    split_diff_lines(text).into_iter().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with(AP_BEGIN)
            || trimmed.starts_with(AP_UPDATE)
            || trimmed.starts_with(AP_ADD)
            || trimmed.starts_with(AP_DELETE)
    })
}

// ─── Public analysis API ───────────────────────────────────────────────────

/// Counts added/removed lines and touched files (`files` counts the distinct
/// paths from [`changed_file_paths`]).
pub fn diff_stats(lines: &[DiffLine]) -> DiffStats {
    let mut stats = DiffStats::default();
    for line in lines {
        match line.kind {
            DiffLineKind::Add => stats.added += 1,
            DiffLineKind::Remove => stats.removed += 1,
            _ => {}
        }
    }
    stats.files = changed_file_paths(lines).len();
    stats
}

/// Distinct file paths in first-appearance order.
///
/// Unified headers are paired (`---` supplies the path when `+++` is
/// `/dev/null`, i.e. a deletion), git's `a/`/`b/` prefixes and surrounding
/// quotes are stripped, and `apply_patch`'s `*** Move to:` replaces the path of
/// the `*** Update File:` it belongs to instead of adding a second entry.
pub fn changed_file_paths(lines: &[DiffLine]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut pending_old: Option<String> = None;
    let mut move_target: Option<usize> = None;

    for line in lines {
        if line.kind != DiffLineKind::FileHeader {
            continue;
        }
        if let Some(rest) = line.text.strip_prefix("--- ") {
            pending_old = clean_unified_header_path(rest);
            move_target = None;
            continue;
        }
        if let Some(rest) = line.text.strip_prefix("+++ ") {
            let path = clean_unified_header_path(rest).or_else(|| pending_old.take());
            pending_old = None;
            move_target = None;
            if let Some(path) = path {
                push_unique(&mut out, path);
            }
            continue;
        }
        let Some((marker, path)) = apply_patch_directive(&line.text) else {
            continue;
        };
        if path.is_empty() {
            continue;
        }
        if marker == AP_MOVE {
            let moved = path.to_string();
            match move_target {
                Some(index) => {
                    out[index] = moved;
                    move_target = Some(index);
                }
                None => {
                    push_unique(&mut out, moved);
                }
            }
            continue;
        }
        let index = push_unique(&mut out, path.to_string());
        move_target = (marker == AP_UPDATE).then_some(index);
    }
    out
}

/// Extracts the path carried by a [`DiffLineKind::FileHeader`] line.
pub fn file_path(line: &DiffLine) -> Option<String> {
    if line.kind != DiffLineKind::FileHeader {
        return None;
    }
    if let Some(rest) = line.text.strip_prefix("--- ") {
        return clean_unified_header_path(rest);
    }
    if let Some(rest) = line.text.strip_prefix("+++ ") {
        return clean_unified_header_path(rest);
    }
    match apply_patch_directive(&line.text) {
        Some((_, path)) if !path.is_empty() => Some(path.to_string()),
        _ => None,
    }
}

/// The single gutter number for a line: the old number for removals, the new
/// number otherwise (mirrors the desktop `DiffView`). `None` for headers/meta
/// and for formats that carry no numbering (see the module docs).
pub fn line_number(line: &DiffLine) -> Option<u32> {
    match line.kind {
        DiffLineKind::Add | DiffLineKind::Context => line.new_no,
        DiffLineKind::Remove => line.old_no,
        DiffLineKind::Hunk
        | DiffLineKind::Meta
        | DiffLineKind::NoNewline
        | DiffLineKind::FileHeader => None,
    }
}

/// `edit src/a.rs +12 -3` — the collapsed tool row for `edit`-style tools.
pub fn collapsed_summary(stats: &DiffStats, files: &[String]) -> String {
    collapsed_summary_for("edit", stats, files)
}

/// [`collapsed_summary`] with an explicit tool name (`write`, `apply_patch`, …).
pub fn collapsed_summary_for(tool: &str, stats: &DiffStats, files: &[String]) -> String {
    let tool = tool.trim();
    let counts = format!("+{} -{}", stats.added, stats.removed);
    let head = |subject: String| {
        if tool.is_empty() {
            subject
        } else {
            format!("{tool} {subject}")
        }
    };
    match files.len() {
        0 => {
            if stats.added == 0 && stats.removed == 0 {
                "no changes".to_string()
            } else {
                head(counts)
            }
        }
        1 => head(format!("{} {counts}", files[0])),
        n => head(format!("{n} files {counts}")),
    }
}

// ─── Public rendering API ──────────────────────────────────────────────────

/// Renders a parsed block into rows of exactly `width` visible columns.
///
/// Styling is 256-colour SGR; add/remove rows additionally take their theme
/// background. When `lines` is longer than `max_lines` the block is cut and a
/// `… N more lines` row is appended (so `max_lines == 1` yields only that row).
/// Empty input, `width == 0` and `max_lines == 0` yield no rows at all.
pub fn render_diff(
    lines: &[DiffLine],
    width: usize,
    theme: &DiffTheme,
    max_lines: usize,
) -> Vec<String> {
    if lines.is_empty() || width == 0 || max_lines == 0 {
        return Vec::new();
    }
    let gutter = width >= MIN_WIDTH_FOR_GUTTER && lines.iter().any(|l| line_number(l).is_some());
    let truncated = lines.len() > max_lines;
    let shown = if truncated {
        max_lines - 1
    } else {
        lines.len()
    };

    let mut out: Vec<String> = Vec::with_capacity(shown + 1);
    for line in &lines[..shown] {
        out.push(render_row(line, width, theme, gutter));
    }
    if truncated {
        let remaining = lines.len() - shown;
        out.push(style_row(
            &format!("… {remaining} more lines"),
            theme.meta_fg,
            None,
            width,
            false,
        ));
    }
    out
}

// ─── Rendering helpers ─────────────────────────────────────────────────────

fn render_row(line: &DiffLine, width: usize, theme: &DiffTheme, gutter: bool) -> String {
    let mut text = String::new();
    if gutter {
        match line_number(line) {
            Some(number) => text.push_str(&format!("{number:>4} ")),
            None => text.push_str(&" ".repeat(GUTTER_WIDTH)),
        }
    }
    text.push_str(&replace_tabs(&line.text, TAB_WIDTH));

    let (fg, bg, bold) = style_for(line.kind, theme);
    style_row(&text, fg, bg, width, bold)
}

fn style_for(kind: DiffLineKind, theme: &DiffTheme) -> (u8, Option<u8>, bool) {
    match kind {
        DiffLineKind::Add => (theme.add_fg, theme.add_bg, false),
        DiffLineKind::Remove => (theme.remove_fg, theme.remove_bg, false),
        DiffLineKind::Hunk => (theme.hunk_fg, None, false),
        DiffLineKind::FileHeader => (theme.meta_fg, None, true),
        DiffLineKind::Meta | DiffLineKind::NoNewline => (theme.meta_fg, None, false),
        DiffLineKind::Context => (C.fg, None, false),
    }
}

/// Truncates `text` to `width` columns, colours it, then pads it (with the
/// theme background when one is configured) to exactly `width` columns.
fn style_row(text: &str, fg: u8, bg: Option<u8>, width: usize, bold: bool) -> String {
    let cut = truncate_to_width(
        text,
        width,
        &TruncateOptions {
            ellipsis: true,
            pad: false,
        },
    );
    let bold_code = if bold { BOLD } else { "" };
    let styled = format!("{bold_code}{CSI}38;5;{fg}m{cut}{RESET}");
    match bg {
        Some(bg) => apply_background_to_line(&styled, width, i16::from(bg)),
        None => {
            let pad = width.saturating_sub(visible_width(&strip_ansi_codes(&styled)));
            styled + &" ".repeat(pad)
        }
    }
}

// ─── Parsing helpers ───────────────────────────────────────────────────────

/// Splits on `\n`, drops a single trailing `\r` per line (CRLF/LF mixed input)
/// and ignores the empty field produced by a final newline.
fn split_diff_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    lines
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HunkRange {
    old_start: u32,
    old_count: u32,
    new_start: u32,
    new_count: u32,
}

/// `@@ -a[,b] +c[,d] @@ optional heading`; `b`/`d` default to 1.
fn parse_hunk_header(line: &str) -> Option<HunkRange> {
    let rest = line.strip_prefix("@@")?.strip_prefix(' ')?;
    let rest = rest.strip_prefix('-')?;
    let (old_start, rest) = parse_number(rest)?;
    let (old_count, rest) = parse_count(rest)?;
    let rest = rest.strip_prefix(" +")?;
    let (new_start, rest) = parse_number(rest)?;
    let (new_count, rest) = parse_count(rest)?;
    // The terminator is required — that is what keeps stray `@@ -1,2 …` prose
    // out of hunk mode. Everything past it is a decorative label (the function
    // name git appends) and is accepted as-is.
    rest.strip_prefix(" @@")?;
    Some(HunkRange {
        old_start,
        old_count,
        new_start,
        new_count,
    })
}

fn parse_number(text: &str) -> Option<(u32, &str)> {
    let digits = text.len() - text.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return None;
    }
    // `parse` rejects out-of-range values, so absurd numbers degrade to
    // "malformed" instead of wrapping or panicking.
    text[..digits]
        .parse::<u32>()
        .ok()
        .map(|n| (n, &text[digits..]))
}

fn parse_count(text: &str) -> Option<(u32, &str)> {
    match text.strip_prefix(',') {
        Some(rest) => parse_number(rest),
        None => Some((1, text)),
    }
}

fn is_unified_file_header(line: &str) -> bool {
    let rest = match line.strip_prefix("--- ") {
        Some(rest) => rest,
        None => match line.strip_prefix("+++ ") {
            Some(rest) => rest,
            None => return false,
        },
    };
    !rest.trim().is_empty()
}

fn is_no_newline_marker(line: &str) -> bool {
    line.starts_with(NO_NEWLINE_MARKER)
}

fn is_apply_patch_marker(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed == AP_BEGIN || trimmed == AP_END || trimmed == AP_EOF
}

/// `("*** Update File: ", "src/a.rs")` for an `apply_patch` directive line.
fn apply_patch_directive(line: &str) -> Option<(&'static str, &str)> {
    for marker in [AP_ADD, AP_UPDATE, AP_DELETE, AP_MOVE] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some((marker, rest.trim()));
        }
    }
    None
}

/// Normalises a `---`/`+++` header payload: drops a `diff -u` timestamp, quotes
/// and git's `a/`/`b/` prefix; maps `/dev/null` to "no path".
fn clean_unified_header_path(rest: &str) -> Option<String> {
    let without_timestamp = rest.split('\t').next().unwrap_or("");
    let unquoted = without_timestamp.trim().trim_matches('"');
    if unquoted.is_empty() || unquoted == "/dev/null" {
        return None;
    }
    let stripped = unquoted
        .strip_prefix("a/")
        .or_else(|| unquoted.strip_prefix("b/"))
        .unwrap_or(unquoted);
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_string())
    }
}

/// Appends `path` unless already present; returns its index either way.
fn push_unique(out: &mut Vec<String>, path: String) -> usize {
    match out.iter().position(|existing| *existing == path) {
        Some(index) => index,
        None => {
            out.push(path);
            out.len() - 1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "diff --git a/src/a.rs b/src/a.rs\n\
         index 1111111..2222222 100644\n\
         --- a/src/a.rs\n\
         +++ b/src/a.rs\n\
         @@ -1,3 +1,4 @@ fn main() {\n\
         \x20line one\n\
         -old\n\
         +new\n\
         +extra\n\
         \x20tail\n";

    fn kinds(lines: &[DiffLine]) -> Vec<DiffLineKind> {
        lines.iter().map(|line| line.kind).collect()
    }

    fn numbers(lines: &[DiffLine]) -> Vec<(Option<u32>, Option<u32>)> {
        lines
            .iter()
            .map(|line| (line.old_no, line.new_no))
            .collect()
    }

    fn texts(lines: &[DiffLine]) -> Vec<String> {
        lines.iter().map(|line| line.text.clone()).collect()
    }

    /// Stripped row content without trailing pad.
    fn plain(row: &str) -> String {
        strip_ansi_codes(row).trim_end().to_string()
    }

    fn assert_rows_width(rows: &[String], width: usize) {
        for row in rows {
            assert_eq!(
                visible_width(&strip_ansi_codes(row)),
                width,
                "row {row:?} is not {width} columns"
            );
        }
    }

    // ── unified parsing ────────────────────────────────────────────────────

    #[test]
    fn parses_unified_diff_completely() {
        let lines = parse_unified_diff(SAMPLE);
        assert_eq!(
            kinds(&lines),
            vec![
                DiffLineKind::Meta,
                DiffLineKind::Meta,
                DiffLineKind::FileHeader,
                DiffLineKind::FileHeader,
                DiffLineKind::Hunk,
                DiffLineKind::Context,
                DiffLineKind::Remove,
                DiffLineKind::Add,
                DiffLineKind::Add,
                DiffLineKind::Context,
            ]
        );
        assert_eq!(
            numbers(&lines),
            vec![
                (None, None),
                (None, None),
                (None, None),
                (None, None),
                (Some(1), Some(1)),
                (Some(1), Some(1)),
                (Some(2), None),
                (None, Some(2)),
                (None, Some(3)),
                (Some(3), Some(4)),
            ]
        );
        assert_eq!(lines[4].text, "@@ -1,3 +1,4 @@ fn main() {");
        assert_eq!(texts(&lines)[7], "+new");
    }

    #[test]
    fn parses_hunk_header_with_default_new_count() {
        // `+2` (no `,count`) means exactly one line on the new side.
        let lines = parse_unified_diff("@@ -2,3 +2 @@\n-a\n-b\n a\n");
        assert_eq!(lines[0].kind, DiffLineKind::Hunk);
        assert_eq!((lines[0].old_no, lines[0].new_no), (Some(2), Some(2)));
        assert_eq!((lines[1].old_no, lines[1].new_no), (Some(2), None));
        assert_eq!((lines[3].old_no, lines[3].new_no), (Some(4), Some(2)));
        assert_eq!(lines[3].kind, DiffLineKind::Context);
        // Nothing is left over once the declared budget is consumed.
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn parses_hunk_header_without_counts() {
        let lines = parse_unified_diff("@@ -5 +7 @@\n-a\n+b\n");
        assert_eq!(lines.len(), 3);
        assert_eq!((lines[0].old_no, lines[0].new_no), (Some(5), Some(7)));
        assert_eq!((lines[1].old_no, lines[1].new_no), (Some(5), None));
        assert_eq!((lines[2].old_no, lines[2].new_no), (None, Some(7)));
    }

    #[test]
    fn parses_new_file_with_zero_count_hunk() {
        let lines =
            parse_unified_diff("--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,3 @@\n+a\n+b\n+c\n");
        assert_eq!(
            kinds(&lines),
            vec![
                DiffLineKind::FileHeader,
                DiffLineKind::FileHeader,
                DiffLineKind::Hunk,
                DiffLineKind::Add,
                DiffLineKind::Add,
                DiffLineKind::Add,
            ]
        );
        assert_eq!(
            numbers(&lines)[3..],
            [(None, Some(1)), (None, Some(2)), (None, Some(3))]
        );
    }

    #[test]
    fn parses_blank_context_line_inside_hunk() {
        let lines = parse_unified_diff("@@ -1,2 +1,2 @@\n\n-tail\n+head\n");
        assert_eq!(lines[1].kind, DiffLineKind::Context);
        assert_eq!(lines[1].text, "");
        assert_eq!((lines[1].old_no, lines[1].new_no), (Some(1), Some(1)));
        assert_eq!((lines[2].old_no, lines[2].new_no), (Some(2), None));
    }

    #[test]
    fn parses_no_newline_marker_without_consuming_budget() {
        let text = "@@ -1,1 +1,1 @@\n-old\n\\ No newline at end of file\n+new\n";
        let lines = parse_unified_diff(text);
        assert_eq!(kinds(&lines)[2], DiffLineKind::NoNewline);
        assert_eq!(numbers(&lines)[2], (None, None));
        // The `+new` still consumed the (already exhausted) new budget via the
        // degraded path, i.e. the marker did not shift numbering.
        assert_eq!(lines[3].kind, DiffLineKind::Add);
    }

    #[test]
    fn no_newline_marker_consumes_no_budget() {
        let text = "@@ -1,1 +1,2 @@\n-old\n\\ No newline at end of file\n+new\n+extra\n";
        let lines = parse_unified_diff(text);
        assert_eq!(
            numbers(&lines),
            vec![
                (Some(1), Some(1)),
                (Some(1), None),
                (None, None),
                (None, Some(1)),
                (None, Some(2)),
            ]
        );
    }

    #[test]
    fn parses_crlf_and_mixed_line_endings() {
        let text = "--- a/x\r\n+++ b/x\n@@ -1,1 +1,1 @@\r\n-a\r\n+b\r\n";
        let lines = parse_unified_diff(text);
        assert_eq!(lines.len(), 5);
        assert_eq!(
            texts(&lines),
            vec!["--- a/x", "+++ b/x", "@@ -1,1 +1,1 @@", "-a", "+b"]
        );
        assert_eq!(lines[3].text, "-a");
    }

    #[test]
    fn trailing_newline_does_not_add_a_line() {
        let with = parse_unified_diff("--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+b\n");
        let without = parse_unified_diff("--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+b");
        assert_eq!(with.len(), without.len());
    }

    #[test]
    fn malformed_hunk_header_only_at_signs_becomes_meta() {
        let lines = parse_unified_diff("@@\n+a\n");
        assert_eq!(kinds(&lines), vec![DiffLineKind::Meta, DiffLineKind::Add]);
        assert_eq!(numbers(&lines)[1], (None, None));
    }

    #[test]
    fn malformed_hunk_header_bad_numbers_become_meta() {
        for header in [
            "@@ -1,x +1,2 @@",
            "@@ -1,2 1,2 @@",
            "@@ -1,2 +1,2@",
            "@@@ -1,2 +1,2 @@",
            "@@ 1,2 +1,2 @@",
            "@@ -1,2 +1, @@",
        ] {
            let lines = parse_unified_diff(&format!("{header}\n+a\n"));
            assert_eq!(lines[0].kind, DiffLineKind::Meta, "header {header:?}");
        }
    }

    #[test]
    fn out_of_range_line_numbers_become_meta_without_panic() {
        let lines = parse_unified_diff("@@ -4294967296,1 +1,1 @@\n-a\n");
        assert_eq!(lines[0].kind, DiffLineKind::Meta);
        assert_eq!(lines[1].kind, DiffLineKind::Remove);
        assert_eq!(lines[1].old_no, None);
    }

    #[test]
    fn empty_hunk_yields_only_the_header() {
        let lines = parse_unified_diff("--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n");
        assert_eq!(kinds(&lines)[2], DiffLineKind::Hunk);
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn zero_count_both_sides_does_not_enter_hunk_mode() {
        let lines = parse_unified_diff("@@ -0,0 +0,0 @@\n--- a/x\n+++ b/x\n");
        assert_eq!(
            kinds(&lines),
            vec![
                DiffLineKind::Hunk,
                DiffLineKind::FileHeader,
                DiffLineKind::FileHeader,
            ]
        );
    }

    #[test]
    fn missing_file_header_still_numbers_hunk_lines() {
        let lines = parse_unified_diff("@@ -3,1 +3,1 @@\n-a\n+b\n");
        assert_eq!(numbers(&lines)[1], (Some(3), None));
        assert_eq!(numbers(&lines)[2], (None, Some(3)));
    }

    #[test]
    fn git_meta_lines_are_meta_and_not_files() {
        let text = "diff --git a/x b/x\n\
             index 1111111..2222222 100644\n\
             new file mode 100644\n\
             similarity index 90%\n\
             rename from x\n\
             rename to y\n\
             Binary files a/x and b/x differ\n";
        let lines = parse_unified_diff(text);
        assert!(lines.iter().all(|l| l.kind == DiffLineKind::Meta));
        assert_eq!(changed_file_paths(&lines), Vec::<String>::new());
    }

    #[test]
    fn second_file_header_after_exhausted_hunk_is_recognised() {
        let text = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-aa\n+bb\n--- a/y\n+++ b/y\n@@ -1,1 +1,1 @@\n-cc\n+dd\n";
        let lines = parse_unified_diff(text);
        assert_eq!(lines[3].kind, DiffLineKind::Remove);
        assert_eq!(lines[3].text, "-aa");
        assert_eq!(lines[4].kind, DiffLineKind::Add);
        assert_eq!(lines[5].kind, DiffLineKind::FileHeader);
        assert_eq!(lines[6].kind, DiffLineKind::FileHeader);
        assert_eq!(changed_file_paths(&lines), vec!["x", "y"]);
    }

    #[test]
    fn header_like_line_inside_hunk_budget_is_content() {
        // `-- sql` is a removal while the hunk still expects an old line.
        let text = "--- a/x.sql\n+++ b/x.sql\n@@ -1,2 +1,1 @@\n--- sql comment\n keep\n";
        let lines = parse_unified_diff(text);
        assert_eq!(lines[3].kind, DiffLineKind::Remove);
        assert_eq!(lines[3].text, "--- sql comment");
        assert_eq!(lines[3].old_no, Some(1));
        assert_eq!(lines[4].kind, DiffLineKind::Context);
    }

    #[test]
    fn extra_content_line_beyond_hunk_budget_degrades() {
        let text = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+b\n+surplus\n";
        let lines = parse_unified_diff(text);
        assert_eq!(lines.last().unwrap().kind, DiffLineKind::Add);
        assert_eq!(lines.last().unwrap().text, "+surplus");
        assert_eq!(lines.last().unwrap().new_no, None);
    }

    #[test]
    fn whitespace_only_and_unknown_lines_are_meta() {
        let lines = parse_unified_diff("not a diff at all\n\n");
        assert_eq!(kinds(&lines), vec![DiffLineKind::Meta]);
    }

    #[test]
    fn empty_input_parses_to_nothing() {
        assert!(parse_unified_diff("").is_empty());
        assert!(parse_apply_patch("").is_empty());
        assert!(parse_any_diff("").is_empty());
    }

    // ── apply_patch parsing ────────────────────────────────────────────────

    const AP_SAMPLE: &str = "*** Begin Patch\n\
         *** Update File: src/a.rs\n\
         @@ fn main() {\n\
         -let x = 1;\n\
         +let x = 2;\n\
         \x20unchanged\n\
         *** Add File: src/b.rs\n\
         +fn b() {}\n\
         *** Delete File: src/c.rs\n\
         *** End Patch\n";

    #[test]
    fn parses_apply_patch_completely() {
        let lines = parse_apply_patch(AP_SAMPLE);
        assert_eq!(
            kinds(&lines),
            vec![
                DiffLineKind::Meta,
                DiffLineKind::FileHeader,
                DiffLineKind::Hunk,
                DiffLineKind::Remove,
                DiffLineKind::Add,
                DiffLineKind::Context,
                DiffLineKind::FileHeader,
                DiffLineKind::Add,
                DiffLineKind::FileHeader,
                DiffLineKind::Meta,
            ]
        );
        // No numbering exists in this format: never fabricated.
        assert_eq!(numbers(&lines), vec![(None, None); lines.len()]);
        assert_eq!(lines[2].text, "@@ fn main() {");
        assert_eq!(
            changed_file_paths(&lines),
            vec!["src/a.rs", "src/b.rs", "src/c.rs"]
        );
        let stats = diff_stats(&lines);
        assert_eq!((stats.added, stats.removed, stats.files), (2, 1, 3));
    }

    #[test]
    fn apply_patch_is_sniffed_by_markers() {
        assert!(is_apply_patch(AP_SAMPLE));
        assert!(is_apply_patch("  *** Begin Patch\n"));
        assert!(is_apply_patch("*** Add File: x\n"));
        assert!(!is_apply_patch(SAMPLE));
        assert!(!is_apply_patch(""));
        assert_eq!(parse_any_diff(AP_SAMPLE), parse_apply_patch(AP_SAMPLE));
        assert_eq!(parse_any_diff(SAMPLE), parse_unified_diff(SAMPLE));
    }

    #[test]
    fn apply_patch_without_envelope_still_parses() {
        let lines = parse_any_diff("*** Update File: a.rs\n@@\n-a\n+b\n");
        assert_eq!(kinds(&lines)[0], DiffLineKind::FileHeader);
        assert_eq!(changed_file_paths(&lines), vec!["a.rs"]);
    }

    #[test]
    fn apply_patch_move_target_replaces_the_update_path() {
        let text = "*** Begin Patch\n\
             *** Update File: old/name.rs\n\
             *** Move to: new/name.rs\n\
             @@\n\
             -a\n\
             +b\n\
             *** End Patch\n";
        let lines = parse_apply_patch(text);
        assert_eq!(lines[2].kind, DiffLineKind::FileHeader);
        assert_eq!(file_path(&lines[2]).as_deref(), Some("new/name.rs"));
        assert_eq!(changed_file_paths(&lines), vec!["new/name.rs"]);
    }

    #[test]
    fn apply_patch_standalone_move_target_is_its_own_path() {
        let lines = parse_apply_patch("*** Move to: solo/name.rs\n");
        assert_eq!(changed_file_paths(&lines), vec!["solo/name.rs"]);
    }

    #[test]
    fn apply_patch_unknown_directives_and_markers() {
        let text =
            "*** Begin Patch\n*** Frobnicate: x\n*** End of File\nplain text\n*** End Patch\n";
        let lines = parse_apply_patch(text);
        assert_eq!(kinds(&lines), vec![DiffLineKind::Meta; 5]);
        assert_eq!(lines[0].text, "*** Begin Patch");
        assert_eq!(lines[2].text, "*** End of File");
    }

    #[test]
    fn apply_patch_crlf_blank_context_and_no_newline_marker() {
        let text = "*** Begin Patch\r\n\
             *** Update File: a.rs\r\n\
             @@\r\n\
             -a\r\n\
             +b\r\n\
             \r\n\
             \\ No newline at end of file\r\n\
             *** End Patch\r\n";
        let lines = parse_apply_patch(text);
        assert_eq!(lines[4].text, "+b");
        assert_eq!(lines[5].kind, DiffLineKind::Context);
        assert_eq!(lines[5].text, "");
        assert_eq!(lines[6].kind, DiffLineKind::NoNewline);
        assert_eq!(lines.last().unwrap().text, "*** End Patch");
    }

    #[test]
    fn apply_patch_blank_line_outside_a_file_is_skipped() {
        let lines = parse_apply_patch("*** Begin Patch\n\n*** End Patch\n");
        assert_eq!(kinds(&lines), vec![DiffLineKind::Meta, DiffLineKind::Meta]);
    }

    #[test]
    fn apply_patch_empty_directive_path_is_not_a_file() {
        let lines = parse_apply_patch("*** Update File: \n*** End Patch\n");
        assert_eq!(changed_file_paths(&lines), Vec::<String>::new());
        assert_eq!(file_path(&lines[0]), None);
    }

    // ── stats / paths / summary ────────────────────────────────────────────

    #[test]
    fn diff_stats_counts_adds_removes_and_files() {
        let stats = diff_stats(&parse_unified_diff(SAMPLE));
        assert_eq!(stats.added, 2);
        assert_eq!(stats.removed, 1);
        assert_eq!(stats.files, 1);
    }

    #[test]
    fn diff_stats_on_empty_and_meta_only_blocks() {
        assert_eq!(diff_stats(&[]), DiffStats::default());
        let meta = parse_unified_diff("index 1..2 100644\n");
        assert_eq!(diff_stats(&meta), DiffStats::default());
    }

    #[test]
    fn changed_file_paths_strips_git_prefixes_and_keeps_order() {
        let text = "diff --git a/src/a.rs b/src/a.rs\n\
             --- a/src/a.rs\n+++ b/src/a.rs\n\
             diff --git a/src/b.rs b/src/b.rs\n\
             --- a/src/b.rs\n+++ b/src/b.rs\n\
             --- a/src/a.rs\n+++ b/src/a.rs\n";
        let lines = parse_unified_diff(text);
        assert_eq!(changed_file_paths(&lines), vec!["src/a.rs", "src/b.rs"]);
    }

    #[test]
    fn changed_file_paths_falls_back_to_old_path_for_deletions() {
        let lines = parse_unified_diff("--- a/gone.txt\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-bye\n");
        assert_eq!(changed_file_paths(&lines), vec!["gone.txt"]);
    }

    #[test]
    fn changed_file_paths_handles_quoted_and_timestamped_headers() {
        let text = "--- a/my file.rs\t2024-01-01 10:00:00.000000000 +0000\n\
             +++ \"b/my file.rs\"\t2024-01-01 10:00:01.000000000 +0000\n";
        let lines = parse_unified_diff(text);
        assert_eq!(changed_file_paths(&lines), vec!["my file.rs"]);
        assert_eq!(file_path(&lines[1]).as_deref(), Some("my file.rs"));
        assert_eq!(file_path(&lines[0]).as_deref(), Some("my file.rs"));
    }

    #[test]
    fn changed_file_paths_ignores_lone_dev_null_and_garbage() {
        let lines = parse_unified_diff("+++ /dev/null\n--- \nnot a header\n");
        assert_eq!(changed_file_paths(&lines), Vec::<String>::new());
    }

    #[test]
    fn changed_file_paths_from_lone_plus_header() {
        let lines = parse_unified_diff("+++ b/only.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n");
        assert_eq!(changed_file_paths(&lines), vec!["only.rs"]);
    }

    #[test]
    fn file_path_rejects_headerless_payloads() {
        let lines = parse_unified_diff("+++ /dev/null\n--- a/\n+++ \"\"\n");
        for line in &lines {
            assert_eq!(file_path(line), None, "{line:?}");
        }
        assert_eq!(changed_file_paths(&lines), Vec::<String>::new());
    }

    #[test]
    fn hand_built_file_header_without_a_known_prefix_is_ignored() {
        let lines = vec![DiffLine {
            kind: DiffLineKind::FileHeader,
            text: "*** Frobnicate: x".to_string(),
            old_no: None,
            new_no: None,
        }];
        assert_eq!(changed_file_paths(&lines), Vec::<String>::new());
        assert_eq!(file_path(&lines[0]), None);
        assert_eq!(line_number(&lines[0]), None);
    }

    #[test]
    fn file_path_returns_none_for_non_headers() {
        let lines = parse_unified_diff(SAMPLE);
        for line in lines.iter().take(2) {
            assert_eq!(file_path(line), None);
        }
        assert_eq!(file_path(&lines[4]), None);
    }

    #[test]
    fn line_number_prefers_old_for_removals_only() {
        let lines = parse_unified_diff(SAMPLE);
        assert_eq!(line_number(&lines[5]), Some(1));
        assert_eq!(line_number(&lines[6]), Some(2));
        assert_eq!(line_number(&lines[7]), Some(2));
        assert_eq!(line_number(&lines[4]), None);
        assert_eq!(line_number(&lines[3]), None);
        assert_eq!(line_number(&lines[2]), None);
    }

    #[test]
    fn collapsed_summary_variants() {
        let one = vec!["src/a.rs".to_string()];
        let many = vec!["src/a.rs".to_string(), "src/b.rs".to_string()];
        let stats = DiffStats {
            added: 12,
            removed: 3,
            files: 1,
        };
        assert_eq!(collapsed_summary(&stats, &one), "edit src/a.rs +12 -3");
        assert_eq!(collapsed_summary(&stats, &many), "edit 2 files +12 -3");
        assert_eq!(
            collapsed_summary_for("write", &stats, &many),
            "write 2 files +12 -3"
        );
        assert_eq!(
            collapsed_summary_for("apply_patch", &stats, &one),
            "apply_patch src/a.rs +12 -3"
        );
        assert_eq!(collapsed_summary(&DiffStats::default(), &[]), "no changes");
        assert_eq!(collapsed_summary_for("", &stats, &one), "src/a.rs +12 -3");
        assert_eq!(
            collapsed_summary_for("  edit  ", &stats, &one),
            "edit src/a.rs +12 -3"
        );
    }

    #[test]
    fn collapsed_summary_without_paths_but_with_counts() {
        let stats = DiffStats {
            added: 4,
            removed: 0,
            files: 0,
        };
        assert_eq!(collapsed_summary(&stats, &[]), "edit +4 -0");
        assert_eq!(collapsed_summary_for("", &stats, &[]), "+4 -0");
    }

    // ── rendering ──────────────────────────────────────────────────────────

    #[test]
    fn render_empty_input_produces_nothing() {
        let theme = DiffTheme::default();
        assert!(render_diff(&[], 40, &theme, 10).is_empty());
    }

    #[test]
    fn render_with_zero_width_or_zero_max_lines_produces_nothing() {
        let theme = DiffTheme::default();
        let lines = parse_unified_diff(SAMPLE);
        assert!(render_diff(&lines, 0, &theme, 10).is_empty());
        assert!(render_diff(&lines, 40, &theme, 0).is_empty());
    }

    #[test]
    fn render_default_theme_matches_crate_palette() {
        assert_eq!(DiffTheme::default(), DEFAULT_DIFF_THEME);
        assert_eq!(DEFAULT_DIFF_THEME.add_fg, C.green);
        assert_eq!(DEFAULT_DIFF_THEME.remove_fg, C.red);
    }

    #[test]
    fn rendered_rows_have_exact_visible_width() {
        let theme = DiffTheme::default();
        let lines = parse_unified_diff(SAMPLE);
        for width in [1, 2, 4, 8, 9, 10, 12, 20, 40, 120] {
            let rows = render_diff(&lines, width, &theme, 100);
            assert_eq!(rows.len(), lines.len(), "width {width}");
            assert_rows_width(&rows, width);
        }
    }

    #[test]
    fn rendered_rows_have_exact_width_for_apply_patch_and_wide_glyphs() {
        let theme = DiffTheme::default();
        let text = "*** Begin Patch\n*** Update File: 中文/文件.rs\n@@ 函数\n-旧值 = 1\n+新值 = 2\n*** End Patch\n";
        let lines = parse_apply_patch(text);
        for width in [1, 3, 7, 11, 24, 60] {
            let rows = render_diff(&lines, width, &theme, 100);
            assert_rows_width(&rows, width);
        }
    }

    #[test]
    fn render_shows_gutter_signs_and_content() {
        let theme = DiffTheme::default();
        let rows = render_diff(&parse_unified_diff(SAMPLE), 40, &theme, 100);
        assert_eq!(plain(&rows[2]), "     --- a/src/a.rs");
        assert_eq!(plain(&rows[4]), "     @@ -1,3 +1,4 @@ fn main() {");
        assert_eq!(plain(&rows[5]), "   1  line one");
        assert_eq!(plain(&rows[6]), "   2 -old");
        assert_eq!(plain(&rows[7]), "   2 +new");
        assert_eq!(plain(&rows[9]), "   4  tail");
    }

    #[test]
    fn render_omits_gutter_when_no_line_has_a_number() {
        let theme = DiffTheme::default();
        let rows = render_diff(&parse_apply_patch(AP_SAMPLE), 40, &theme, 100);
        assert_eq!(plain(&rows[1]), "*** Update File: src/a.rs");
        assert_eq!(plain(&rows[3]), "-let x = 1;");
        assert_eq!(plain(&rows[4]), "+let x = 2;");
    }

    #[test]
    fn render_truncates_long_lines_with_ellipsis_keeping_width() {
        let theme = DiffTheme::default();
        let text = format!("@@ -1,1 +1,1 @@\n+{}\n", "x".repeat(200));
        let lines = parse_unified_diff(&text);
        let rows = render_diff(&lines, 20, &theme, 10);
        assert_rows_width(&rows, 20);
        let add = plain(&rows[1]);
        assert!(add.starts_with("   1 +xxx"), "{add:?}");
        assert!(add.ends_with('…'), "{add:?}");
        // 5 gutter + sign + 13 content columns, the 14th replaced by the ellipsis.
        assert_eq!(
            strip_ansi_codes(&rows[1])
                .chars()
                .filter(|c| *c == 'x')
                .count(),
            13
        );
    }

    #[test]
    fn render_expands_tabs_to_four_columns() {
        let theme = DiffTheme::default();
        let lines = parse_unified_diff("@@ -1,1 +1,1 @@\n+\tif x {\n");
        let rows = render_diff(&lines, 40, &theme, 10);
        assert_eq!(plain(&rows[1]), "   1 +    if x {");
    }

    #[test]
    fn render_truncation_marker_counts_missing_lines() {
        let theme = DiffTheme::default();
        let lines = parse_unified_diff(SAMPLE);
        let rows = render_diff(&lines, 40, &theme, 4);
        assert_eq!(rows.len(), 4);
        assert_eq!(plain(&rows[3]), "… 7 more lines");
        assert_rows_width(&rows, 40);
    }

    #[test]
    fn render_max_lines_one_shows_only_the_marker() {
        let theme = DiffTheme::default();
        let lines = parse_unified_diff(SAMPLE);
        let rows = render_diff(&lines, 40, &theme, 1);
        assert_eq!(rows.len(), 1);
        assert_eq!(plain(&rows[0]), "… 10 more lines");
        assert_rows_width(&rows, 40);
    }

    #[test]
    fn render_exact_max_lines_has_no_marker() {
        let theme = DiffTheme::default();
        let lines = parse_unified_diff(SAMPLE);
        let rows = render_diff(&lines, 40, &theme, lines.len());
        assert_eq!(rows.len(), lines.len());
        assert!(!plain(&rows[rows.len() - 1]).contains("more lines"));
    }

    #[test]
    fn render_max_lines_above_line_count_has_no_marker() {
        let theme = DiffTheme::default();
        let lines = parse_unified_diff(SAMPLE);
        let rows = render_diff(&lines, 40, &theme, 500);
        assert_eq!(rows.len(), lines.len());
    }

    #[test]
    fn render_applies_theme_colours_and_backgrounds() {
        let theme = DiffTheme {
            add_fg: 10,
            remove_fg: 20,
            hunk_fg: 30,
            meta_fg: 40,
            add_bg: Some(22),
            remove_bg: Some(52),
        };
        let rows = render_diff(&parse_unified_diff(SAMPLE), 40, &theme, 100);
        assert!(rows[6].contains("\x1b[38;5;20m"), "{:?}", rows[6]);
        assert!(rows[6].contains("\x1b[48;5;52m"), "{:?}", rows[6]);
        assert!(rows[7].contains("\x1b[38;5;10m"), "{:?}", rows[7]);
        assert!(rows[7].contains("\x1b[48;5;22m"), "{:?}", rows[7]);
        assert!(rows[4].contains("\x1b[38;5;30m"), "{:?}", rows[4]);
        assert!(!rows[4].contains("\x1b[48;5;"), "{:?}", rows[4]);
        assert!(rows[2].contains(BOLD), "{:?}", rows[2]);
        assert!(!rows[5].contains("\x1b[48;5;"), "{:?}", rows[5]);
        assert_rows_width(&rows, 40);
    }

    #[test]
    fn render_background_covers_padding_after_truncation() {
        let theme = DiffTheme {
            add_bg: Some(22),
            ..DiffTheme::default()
        };
        let text = format!("@@ -1,1 +1,1 @@\n+{}\n", "y".repeat(80));
        let rows = render_diff(&parse_unified_diff(&text), 16, &theme, 10);
        let add = &rows[1];
        assert!(plain(add).starts_with("   1 +yyy"));
        assert!(plain(add).ends_with('…'));
        assert_rows_width(&rows, 16);
        assert_eq!(plain(add).chars().count(), 16);
    }

    #[test]
    fn render_styles_the_no_newline_marker_as_meta() {
        let theme = DiffTheme {
            meta_fg: 77,
            add_bg: Some(22),
            ..DiffTheme::default()
        };
        let lines =
            parse_unified_diff("@@ -1,1 +1,1 @@\n-old\n\\ No newline at end of file\n+new\n");
        let rows = render_diff(&lines, 40, &theme, 10);
        assert_eq!(plain(&rows[2]), "     \\ No newline at end of file");
        assert!(rows[2].contains("\x1b[38;5;77m"), "{:?}", rows[2]);
        assert!(!rows[2].contains("\x1b[48;5;"), "{:?}", rows[2]);
        assert!(rows[3].contains("\x1b[48;5;22m"), "{:?}", rows[3]);
        assert_rows_width(&rows, 40);
    }

    #[test]
    fn render_marker_row_uses_meta_colour() {
        let theme = DiffTheme {
            meta_fg: 99,
            ..DiffTheme::default()
        };
        let lines = parse_unified_diff(SAMPLE);
        let rows = render_diff(&lines, 30, &theme, 3);
        assert!(rows[2].contains("\x1b[38;5;99m"), "{:?}", rows[2]);
    }

    #[test]
    fn render_rows_never_contain_raw_newlines() {
        let theme = DiffTheme::default();
        let lines = parse_unified_diff("@@ -1,1 +1,1 @@\n+a\r\n");
        let rows = render_diff(&lines, 20, &theme, 10);
        assert!(rows
            .iter()
            .all(|row| !row.contains('\n') && !row.contains('\r')));
    }
}
