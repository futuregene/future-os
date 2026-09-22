//! Desktop notifications (BEL / OSC 9) and terminal-title (OSC 0) output.
//!
//! The TUI has to tell the user that a run finished, failed, or is blocked on
//! them even when the terminal window is not focused. No portable notification
//! *API* exists, so this module speaks the two portable escape-sequence
//! channels every terminal understands:
//!
//! * the bell (`BEL`, U+0007) — most terminals map it to a sound or a window
//!   badge;
//! * `OSC 9` (`ESC ] 9 ; message BEL`) — iTerm2 / Ghostty / WezTerm / kitty /
//!   warp turn it into a real desktop notification. The framing is the one those
//!   terminals document, so it is reproduced exactly: `ESC ] 9 ;`, the message
//!   with no separator, then `BEL`.
//!
//! The window title (`OSC 0; title BEL`) is the third surface: the shell, tab
//! bar or window manager shows which directory / model / session a `future`
//! run belongs to. [`terminal_title`] builds that string; the caller decides
//! when to write it.
//!
//! Design constraints, all of them pinned by tests below:
//!
//! * **Untrusted text never reaches the terminal unchanged.** Event titles and
//!   bodies are assembled from model output, session names, file paths and
//!   config. A raw `BEL` or `ESC \` inside that text would terminate the escape
//!   sequence early and let the rest of the string be interpreted as terminal
//!   *commands* (arbitrary title changes, clipboard writes, …). So
//!   [`sanitize_escape_text`] parses CSI / OSC / DCS / PM / APC sequences and
//!   drops them *whole* — dropping only the `ESC` byte would leave `[31m`
//!   visible — then drops every remaining control character and the
//!   invisible/bidi formatting characters used by Trojan-Source style
//!   spoofing, collapses whitespace runs, and bounds the length.
//! * **Never emit something the terminal cannot consume.** When stdout is not
//!   a TTY (pipes, `--print`, CI, tests) or notifications are switched off,
//!   [`sequences_for`] returns an empty vector, so nothing — not even a `BEL` —
//!   leaks into captured output.
//! * **The module is pure.** Every function returns the exact bytes to write;
//!   none of them touch stdout, which keeps the whole module testable
//!   byte-for-byte. Writing is `app.rs`'s job.
//!
//! `NotifyConfig` is serde-ready (camelCase keys, missing fields default) so it
//! can be persisted to `~/.future/tui/settings.json` without a hand-written
//! mapping.

use serde::{Deserialize, Serialize};

use crate::utils::truncate_to_width;
use crate::utils::TruncateOptions;

// ─── Constants ─────────────────────────────────────────────────────────────

/// App name used in generated titles and as the fallback when a title has no
/// usable parts.
pub const APP_NAME: &str = "future";

/// Upper bound, in `char`s, on sanitized notification text and title payloads.
///
/// Terminals silently truncate long titles, and a notification body longer
/// than a couple of lines is unreadable in a toast. 240 leaves room for a
/// path, a model id and a state marker, and still leaves headroom inside the
/// OSC framing bytes so the sequence cannot grow past what a terminal accepts.
pub const MAX_SANITIZED_TEXT_CHARS: usize = 240;

/// Upper bound, in terminal *columns*, on a rendered terminal title.
///
/// `terminal_title` truncates with a stable `…` marker so tab bars and window
/// lists stay readable; the streaming marker sits at the front and therefore
/// survives truncation.
pub const MAX_TERMINAL_TITLE_WIDTH: usize = 100;

/// Marker prefixed to the terminal title while a run is streaming, in the same
/// spirit as the reference's `[ ! ] Action Required` prefix. It is a constant
/// (not an animation frame) so titles stay stable between renders.
pub const STREAMING_PREFIX: &str = "[>] ";

/// `BEL` — the terminal bell.
const BEL: char = '\u{7}';
/// `ESC` — introduces every escape sequence.
const ESC: char = '\u{1b}';
/// 8-bit `CSI` (single-byte form of `ESC [`).
const CSI_8BIT: char = '\u{9b}';
/// 8-bit `OSC` (single-byte form of `ESC ]`).
const OSC_8BIT: char = '\u{9d}';
/// `ST` — string terminator for OSC/DCS/PM/APC payloads.
const ST_8BIT: char = '\u{9c}';

// ─── Config ────────────────────────────────────────────────────────────────

/// Which notification channels are switched on.
///
/// `enabled` is the master switch (so a user can silence everything with one
/// key); the per-channel flags let the user keep, say, the visual OSC 9 toast
/// while disabling the audible bell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NotifyConfig {
    /// Master switch. `false` silences every channel.
    pub enabled: bool,
    /// Emit `BEL`.
    pub bell: bool,
    /// Emit an `OSC 9` desktop notification.
    pub osc9: bool,
    /// Also set the window title from the event, instead of leaving the title
    /// to the streaming status line. Off by default: the TUI already owns the
    /// title (`terminal_title`), and two writers would fight over it.
    pub title: bool,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        default_config()
    }
}

/// The default configuration: enabled, with the bell and the desktop
/// notification on, and title-on-event off.
pub fn default_config() -> NotifyConfig {
    NotifyConfig {
        enabled: true,
        bell: true,
        osc9: true,
        title: false,
    }
}

/// Returns `true` when the config would emit nothing at all, whatever the
/// event — used by callers to skip building an event (and by tests).
pub fn is_silent(config: &NotifyConfig) -> bool {
    !config.enabled || (!config.bell && !config.osc9 && !config.title)
}

// ─── Events ────────────────────────────────────────────────────────────────

/// Why a notification is being raised. The kind drives the default title and
/// body so every call site does not have to invent wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NotifyKind {
    /// A run finished normally.
    Completed,
    /// A run ended in an error.
    Failed,
    /// A tool call is waiting for the user's approval.
    ApprovalNeeded,
    /// The agent asked the user a question.
    InputNeeded,
}

impl NotifyKind {
    /// Short human-readable label, e.g. `run completed`.
    pub fn label(self) -> &'static str {
        match self {
            NotifyKind::Completed => "run completed",
            NotifyKind::Failed => "run failed",
            NotifyKind::ApprovalNeeded => "approval needed",
            NotifyKind::InputNeeded => "input needed",
        }
    }

    /// Default notification title for this kind, e.g. `future: run failed`.
    pub fn default_title(self) -> String {
        format!("{APP_NAME}: {}", self.label())
    }

    /// Default notification body for this kind.
    pub fn default_body(self) -> &'static str {
        match self {
            NotifyKind::Completed => "The agent finished this turn.",
            NotifyKind::Failed => "The agent hit an error; open the session for details.",
            NotifyKind::ApprovalNeeded => "A tool call is waiting for your approval.",
            NotifyKind::InputNeeded => "The agent is waiting on your answer.",
        }
    }
}

/// One notification to raise. `title`/`body` may contain anything (paths,
/// model output); they are sanitized on the way out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyEvent {
    pub kind: NotifyKind,
    pub title: String,
    pub body: String,
}

impl NotifyEvent {
    /// Builds an event with explicit wording.
    pub fn new(kind: NotifyKind, title: impl Into<String>, body: impl Into<String>) -> Self {
        NotifyEvent {
            kind,
            title: title.into(),
            body: body.into(),
        }
    }

    /// Builds an event with the default wording for `kind`.
    pub fn for_kind(kind: NotifyKind) -> Self {
        NotifyEvent::new(kind, kind.default_title(), kind.default_body())
    }
}

// ─── Sequence builders ─────────────────────────────────────────────────────

/// `BEL` — the plain bell.
pub fn bel_sequence() -> String {
    BEL.to_string()
}

/// `OSC 9` desktop notification: `ESC ] 9 ; <text> BEL`.
///
/// `title` and `body` are sanitized and joined with `": "`; either may be
/// empty. A payload that sanitizes away entirely still yields a well-formed
/// (empty-message) sequence — terminal that ignore empty OSC 9 messages will
/// ignore it — but [`sequences_for`] skips such events altogether so callers
/// do not emit noise.
pub fn osc9_sequence(title: &str, body: &str) -> String {
    format!("{ESC}]9;{}{BEL}", notification_text(title, body))
}

/// `OSC 0` window title: `ESC ] 0 ; <sanitized title> BEL`.
///
/// An empty (or fully sanitized-away) title emits `ESC ] 0 ; BEL`, which clears
/// the title the TUI manages. Callers that want "leave the old title alone"
/// should skip the write instead; see the reference
/// `SetTerminalTitleResult::NoVisibleContent` for the same policy split.
pub fn set_title_sequence(title: &str) -> String {
    format!("{ESC}]0;{}{BEL}", sanitize_escape_text(title))
}

/// Sanitized `title: body` text for a notification payload.
fn notification_text(title: &str, body: &str) -> String {
    let title = sanitize_escape_text(title);
    let body = sanitize_escape_text(body);
    let joined = match (title.is_empty(), body.is_empty()) {
        (false, false) => format!("{title}: {body}"),
        (false, true) => title,
        (true, false) => body,
        (true, true) => String::new(),
    };
    cap_chars(&joined, MAX_SANITIZED_TEXT_CHARS)
}

/// All sequences to write for `event`, in the stable order
/// title → OSC 9 → BEL.
///
/// Not a TTY, `config.enabled == false`, or every channel switched off ⇒ empty
/// vector (the caller writes nothing and the captured/redirected output stays
/// clean). The order puts the silent title update first and the audible bell
/// last, so a terminal that coalesces or drops later writes loses the least
/// informative channel first.
pub fn sequences_for(config: &NotifyConfig, event: &NotifyEvent, is_tty: bool) -> Vec<String> {
    if !is_tty || !config.enabled {
        return Vec::new();
    }

    let title = sanitize_escape_text(&event.title);
    let body = sanitize_escape_text(&event.body);
    if title.is_empty() && body.is_empty() {
        // Nothing survived sanitization: emitting an empty toast plus a bell
        // would only be noise.
        return Vec::new();
    }
    let title_text = if title.is_empty() { body } else { title };

    let mut sequences = Vec::new();
    if config.title {
        sequences.push(set_title_sequence(&title_text));
    }
    if config.osc9 {
        sequences.push(osc9_sequence(&event.title, &event.body));
    }
    if config.bell {
        sequences.push(bel_sequence());
    }
    sequences
}

// ─── Terminal title ────────────────────────────────────────────────────────

/// Builds the window/tab title describing the current run:
/// `<project> | <model> | <session name>`, prefixed with [`STREAMING_PREFIX`]
/// while a run is streaming.
///
/// Empty parts are omitted (`cwd`/`model` blank, an unnamed session), and a
/// title with no usable part at all falls back to [`APP_NAME`]. The result is
/// sanitized (no control characters can escape from a path or session name)
/// and truncated to [`MAX_TERMINAL_TITLE_WIDTH`] columns from the *front*, so
/// the stable parts — streaming marker, project, model — survive and only the
/// least important tail is replaced by `…`.
pub fn terminal_title(
    cwd: &str,
    model: &str,
    session_name: Option<&str>,
    streaming: bool,
) -> String {
    let mut parts: Vec<String> = Vec::new();

    let project = project_name(cwd);
    if !project.is_empty() {
        parts.push(project);
    }
    let model = sanitize_escape_text(model);
    if !model.is_empty() {
        parts.push(model);
    }
    if let Some(name) = session_name {
        let name = sanitize_escape_text(name);
        if !name.is_empty() {
            parts.push(name);
        }
    }

    let joined = if parts.is_empty() {
        APP_NAME.to_string()
    } else {
        parts.join(" | ")
    };
    let text = if streaming {
        format!("{STREAMING_PREFIX}{joined}")
    } else {
        joined
    };
    truncate_to_width(
        &text,
        MAX_TERMINAL_TITLE_WIDTH,
        &TruncateOptions {
            ellipsis: true,
            pad: false,
        },
    )
}

/// Last path component of `cwd`, used as the project name in the title.
///
/// Accepts both `/` and `\` on every platform (so Windows paths still render
/// correctly in a title written on a Unix box and vice versa), tolerates
/// trailing separators, and falls back to the sanitized input when there is no
/// component to take (a bare `/`, a drive root). Sanitization means a path
/// cannot smuggle control characters into a title.
pub fn project_name(cwd: &str) -> String {
    let sanitized = sanitize_escape_text(cwd);
    let trimmed = sanitized.trim_end_matches(['/', '\\']);
    match trimmed.rsplit(['/', '\\']).next() {
        Some(component) if !component.is_empty() => component.to_string(),
        // A bare root ("\\", "C:\\" handled above) or input that sanitized
        // down to separators only: keep the sanitized path as the identifier.
        _ => sanitized,
    }
}

// ─── Sanitization ──────────────────────────────────────────────────────────

/// Normalizes untrusted text into a single bounded display line safe to embed
/// inside an OSC payload.
///
/// Whole escape sequences are removed (CSI, OSC, DCS, PM, APC, both the 7-bit
/// `ESC`-prefixed and the 8-bit C1 forms), then every remaining control and
/// invisible formatting character is dropped, then whitespace runs — including
/// newlines — collapse to single spaces, and the result is capped at
/// [`MAX_SANITIZED_TEXT_CHARS`] characters. The guarantee the tests pin down:
/// the returned string contains no `ESC`, no `BEL`, no other control character
/// and no bidi/invisible formatting character, whatever the input.
pub fn sanitize_escape_text(text: &str) -> String {
    let stripped = strip_terminal_sequences(text);

    let mut out = String::with_capacity(stripped.len().min(MAX_SANITIZED_TEXT_CHARS));
    let mut written = 0usize;
    let mut pending_space = false;

    for ch in stripped.chars() {
        if ch.is_whitespace() {
            // Leading whitespace never sets the flag, which trims for free.
            pending_space = !out.is_empty();
            continue;
        }
        if is_disallowed_char(ch) {
            continue;
        }
        if written >= MAX_SANITIZED_TEXT_CHARS {
            break;
        }
        if pending_space {
            // Only spend a character on the space when the visible character
            // after it still fits — a truncated title should end with a letter.
            if written + 1 < MAX_SANITIZED_TEXT_CHARS {
                out.push(' ');
                written += 1;
            }
            pending_space = false;
        }
        out.push(ch);
        written += 1;
    }

    out
}

/// Cuts `text` to at most `max` characters (no ellipsis; sanitization is not a
/// display concern).
fn cap_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect()
}

/// Drops every escape sequence, keeping the rest — controls are removed later
/// by [`sanitize_escape_text`].
fn strip_terminal_sequences(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            ESC => skip_escape(&mut chars),
            CSI_8BIT => skip_csi(&mut chars),
            OSC_8BIT => skip_control_string(&mut chars),
            _ => out.push(ch),
        }
    }
    out
}

/// Skips the body of an `ESC`-introduced sequence. `peekable` is positioned
/// just after the `ESC`.
fn skip_escape<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) {
    match chars.peek().copied() {
        Some('[') => {
            chars.next();
            skip_csi(chars);
        }
        Some(']') => {
            chars.next();
            skip_control_string(chars);
        }
        // DCS / SOS / PM / APC: all terminated by ST.
        Some('P') | Some('X') | Some('^') | Some('_') => {
            chars.next();
            skip_control_string(chars);
        }
        // Any other escape: optional intermediate bytes (0x20..=0x2F, at most
        // two) followed by one final byte (0x30..=0x7E). If the sequence is
        // unterminated (end of input) we simply consume what is there.
        _ => {
            let mut intermediates = 0;
            while intermediates < 2 {
                match chars.peek().copied() {
                    Some(c) if is_intermediate_byte(c) => {
                        chars.next();
                        intermediates += 1;
                    }
                    _ => break,
                }
            }
            if let Some(c) = chars.peek().copied() {
                if is_final_byte(c) {
                    chars.next();
                }
            }
        }
    }
}

/// Skips `params` (0x30..=0x3F), `intermediates` (0x20..=0x2F) and the final
/// byte (0x40..=0x7E) of a CSI sequence; `peekable` sits just after `ESC [`.
fn skip_csi<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) {
    while let Some(c) = chars.peek().copied() {
        if is_parameter_byte(c) {
            chars.next();
        } else {
            break;
        }
    }
    let mut intermediates = 0;
    while intermediates < 2 {
        match chars.peek().copied() {
            Some(c) if is_intermediate_byte(c) => {
                chars.next();
                intermediates += 1;
            }
            _ => break,
        }
    }
    if let Some(c) = chars.peek().copied() {
        if (0x40..=0x7E).contains(&(c as u32)) {
            chars.next();
        }
    }
}

/// Skips an OSC/DCS/PM/APC payload up to `BEL`, `ST` (`ESC \`) or the 8-bit
/// `ST`, whichever comes first; `peekable` sits just after the introducer.
fn skip_control_string<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) {
    while let Some(c) = chars.next() {
        if c == BEL || c == ST_8BIT {
            return;
        }
        if c == ESC {
            if chars.peek().copied() == Some('\\') {
                chars.next();
            }
            return;
        }
    }
}

fn is_parameter_byte(c: char) -> bool {
    (0x30..=0x3F).contains(&(c as u32))
}

fn is_intermediate_byte(c: char) -> bool {
    (0x20..=0x2F).contains(&(c as u32))
}

fn is_final_byte(c: char) -> bool {
    (0x30..=0x7E).contains(&(c as u32))
}

/// Whether `ch` must not survive into a terminal payload.
///
/// Covers every control character (C0, `DEL` and C1) plus the invisible and
/// bidi formatting characters that allow visually reordering or hiding text
/// (the Trojan-Source class of attack, CVE-2021-42574). The ranges below come
/// from the Unicode standard's own categories — the default-ignorable code
/// points plus the explicit bidi embedding/override/isolate controls and the
/// variation-selector and tag blocks — not from any particular program, so the
/// set is the one the standards define rather than a snapshot of another
/// implementation.
fn is_disallowed_char(ch: char) -> bool {
    if ch.is_control() {
        return true;
    }
    matches!(
        ch,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{E0100}'..='\u{E01EF}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::visible_width;

    fn count_char(haystack: &str, needle: char) -> usize {
        haystack.chars().filter(|c| *c == needle).count()
    }

    // ─── sanitize_escape_text: injection ──────────────────────────────────

    #[test]
    fn sanitize_strips_csi_sequences_whole() {
        assert_eq!(sanitize_escape_text("\u{1b}[31mred\u{1b}[0m"), "red");
        assert_eq!(sanitize_escape_text("\u{1b}[1;38;5;196mbold"), "bold");
        // 8-bit CSI, and a CSI with a final byte of another kind.
        assert_eq!(sanitize_escape_text("\u{9b}31mred"), "red");
        assert_eq!(sanitize_escape_text("\u{1b}[1qtext"), "text");
        // A CSI with intermediate bytes (0x20..=0x2F) after its parameters:
        // `ESC [ 1 SP q` (a DECSCUSR-style cursor-shape sequence), and one
        // that uses the full two-intermediate allowance.
        assert_eq!(sanitize_escape_text("\u{1b}[1 qtext"), "text");
        assert_eq!(sanitize_escape_text("\u{1b}[  qtext"), "text");
        // Intermediate bytes at the end of the input are still swallowed.
        assert_eq!(sanitize_escape_text("keep\u{1b}[1 "), "keep");
    }

    #[test]
    fn sanitize_strips_osc_payloads_terminated_by_bel_or_st() {
        assert_eq!(sanitize_escape_text("\u{1b}]9;evil\u{7}after"), "after");
        assert_eq!(sanitize_escape_text("\u{1b}]0;title\u{1b}\\after"), "after");
        // 8-bit OSC introducer and 8-bit ST.
        assert_eq!(sanitize_escape_text("\u{9d}0;pwned\u{9c}after"), "after");
        // Unterminated OSC swallows the remainder rather than leaking it.
        assert_eq!(sanitize_escape_text("keep\u{1b}]8;;http://x"), "keep");
    }

    #[test]
    fn sanitize_strips_dcs_and_other_escape_forms() {
        assert_eq!(
            sanitize_escape_text("\u{1b}Ptmux;\u{1b}\u{1b}]9;x\u{7}\u{1b}\\ok"),
            "ok"
        );
        // Two-character escape (save cursor) and charset designator.
        assert_eq!(sanitize_escape_text("\u{1b}7save"), "save");
        assert_eq!(sanitize_escape_text("\u{1b}(Btext"), "text");
        // Bare ESC at end of input.
        assert_eq!(sanitize_escape_text("done\u{1b}"), "done");
        assert_eq!(sanitize_escape_text("done\u{1b}["), "done");
    }

    #[test]
    fn sanitize_drops_raw_control_characters() {
        assert_eq!(sanitize_escape_text("a\u{7}b\u{0}c\u{7f}d\u{9c}e"), "abcde");
        assert_eq!(sanitize_escape_text("\u{1b}\u{7}\u{0}"), "");
        assert_eq!(sanitize_escape_text("bell\u{7}"), "bell");
    }

    #[test]
    fn sanitize_collapses_newlines_and_whitespace_runs() {
        assert_eq!(sanitize_escape_text("  a \n\t b\r\n\n  c  "), "a b c");
        assert_eq!(sanitize_escape_text("\n\n\n"), "");
        assert_eq!(sanitize_escape_text("a\n"), "a");
    }

    #[test]
    fn sanitize_strips_invisible_and_bidi_formatting() {
        assert_eq!(
            sanitize_escape_text(
                "Pro\u{202E}j\u{2066}e\u{200F}c\u{061C}t\u{200B} \u{feff}T\u{2060}itle"
            ),
            "Project Title"
        );
        // Variation Selector Supplement (U+E0100..=U+E01EF), both ends of the
        // range: invisible even though they are not `char::is_control`.
        assert_eq!(sanitize_escape_text("a\u{E0100}b\u{E01EF}c"), "abc");
    }

    #[test]
    fn sanitize_keeps_printable_unicode_and_cjk() {
        assert_eq!(sanitize_escape_text("模型 ok 🚀"), "模型 ok 🚀");
    }

    #[test]
    fn sanitize_truncates_without_breaking_on_a_pending_space() {
        let input = "a".repeat(MAX_SANITIZED_TEXT_CHARS + 40);
        let out = sanitize_escape_text(&input);
        assert_eq!(out.chars().count(), MAX_SANITIZED_TEXT_CHARS);

        // A space that would consume the last slot yields to the visible char.
        let input = format!("{}\n b", "a".repeat(MAX_SANITIZED_TEXT_CHARS - 1));
        let out = sanitize_escape_text(&input);
        assert_eq!(out.chars().count(), MAX_SANITIZED_TEXT_CHARS);
        assert_eq!(out.chars().last(), Some('b'));

        // Long input whose surplus is only whitespace still respects the cap.
        let input = format!("{}   ", "b".repeat(MAX_SANITIZED_TEXT_CHARS + 5));
        assert_eq!(
            sanitize_escape_text(&input).chars().count(),
            MAX_SANITIZED_TEXT_CHARS
        );
    }

    #[test]
    fn sanitize_never_leaves_an_escape_or_bell_behind() {
        for hostile in [
            "\u{1b}]9;pwned\u{7}",
            "\u{1b}]0;title\u{1b}\\",
            "\u{1b}[31mred\u{7}",
            "a\u{9d}0;x\u{9c}b\u{9b}1m",
        ] {
            let out = sanitize_escape_text(hostile);
            assert!(!out.contains('\u{1b}'), "{hostile:?} -> {out:?}");
            assert!(!out.contains('\u{7}'), "{hostile:?} -> {out:?}");
            assert!(
                !out.chars().any(is_disallowed_char),
                "{hostile:?} -> {out:?}"
            );
        }
    }

    // ─── Sequence framing (byte-exact) ────────────────────────────────────

    #[test]
    fn bel_sequence_is_a_single_bel() {
        assert_eq!(bel_sequence(), "\u{7}");
        assert_eq!(bel_sequence().as_bytes(), &[0x07]);
    }

    #[test]
    fn osc9_sequence_frames_title_and_body_with_bel() {
        assert_eq!(
            osc9_sequence("future: run completed", "The agent finished this turn."),
            "\u{1b}]9;future: run completed: The agent finished this turn.\u{7}"
        );
        assert_eq!(osc9_sequence("done", ""), "\u{1b}]9;done\u{7}");
        assert_eq!(
            osc9_sequence("", "run finished"),
            "\u{1b}]9;run finished\u{7}"
        );
        assert_eq!(osc9_sequence("", ""), "\u{1b}]9;\u{7}");
        assert_eq!(
            osc9_sequence("", "").as_bytes(),
            &[0x1b, b']', b'9', b';', 0x07]
        );
    }

    #[test]
    fn osc9_sequence_sanitizes_its_payload() {
        let seq = osc9_sequence("done", "evil\u{7}\u{1b}]0;pwned\u{7}");
        assert_eq!(seq, "\u{1b}]9;done: evil\u{7}");
        assert_eq!(count_char(&seq, '\u{7}'), 1, "{seq:?}");
        // The second ESC would have started an injected title change.
        assert_eq!(count_char(&seq, '\u{1b}'), 1, "{seq:?}");
        assert!(!seq.contains("pwned"), "{seq:?}");
    }

    #[test]
    fn osc9_sequence_bounds_the_payload() {
        let seq = osc9_sequence(&"t".repeat(400), &"b".repeat(400));
        // ESC ] 9 ; <payload> BEL
        assert_eq!(seq.chars().count(), 4 + MAX_SANITIZED_TEXT_CHARS + 1);
        assert!(seq.ends_with('\u{7}'));
    }

    #[test]
    fn set_title_sequence_frames_osc0_with_bel() {
        assert_eq!(set_title_sequence("hello"), "\u{1b}]0;hello\u{7}");
        assert_eq!(
            set_title_sequence("hello").as_bytes(),
            &[0x1b, b']', b'0', b';', b'h', b'e', b'l', b'l', b'o', 0x07]
        );
        assert_eq!(set_title_sequence(""), "\u{1b}]0;\u{7}");
    }

    #[test]
    fn set_title_sequence_cannot_be_escaped_out_of() {
        assert_eq!(set_title_sequence("a\u{1b}]0;b"), "\u{1b}]0;a\u{7}");
        assert_eq!(set_title_sequence("x\u{7}y"), "\u{1b}]0;xy\u{7}");
        assert_eq!(count_char(&set_title_sequence("x\u{7}y"), '\u{7}'), 1);
    }

    // ─── sequences_for: gates ─────────────────────────────────────────────

    #[test]
    fn sequences_for_is_empty_when_not_a_tty() {
        let config = NotifyConfig {
            enabled: true,
            bell: true,
            osc9: true,
            title: true,
        };
        assert!(sequences_for(
            &config,
            &NotifyEvent::for_kind(NotifyKind::Completed),
            false
        )
        .is_empty());
    }

    #[test]
    fn sequences_for_is_empty_when_disabled() {
        let config = NotifyConfig {
            enabled: false,
            bell: true,
            osc9: true,
            title: true,
        };
        assert!(
            sequences_for(&config, &NotifyEvent::for_kind(NotifyKind::Failed), true).is_empty()
        );
    }

    #[test]
    fn sequences_for_is_empty_when_every_channel_is_off() {
        let config = NotifyConfig {
            enabled: true,
            bell: false,
            osc9: false,
            title: false,
        };
        assert!(is_silent(&config));
        assert!(sequences_for(
            &config,
            &NotifyEvent::for_kind(NotifyKind::InputNeeded),
            true
        )
        .is_empty());
    }

    #[test]
    fn sequences_for_skips_an_event_with_no_visible_text() {
        let config = NotifyConfig {
            enabled: true,
            bell: true,
            osc9: true,
            title: true,
        };
        let event = NotifyEvent::new(NotifyKind::Failed, "\u{1b}]0;x\u{7}", "\u{7}\u{1b}[2J");
        assert!(sequences_for(&config, &event, true).is_empty());
    }

    #[test]
    fn is_silent_reports_the_master_switch_and_channel_state() {
        assert!(!is_silent(&default_config()));
        assert!(is_silent(&NotifyConfig {
            enabled: false,
            ..default_config()
        }));
        assert!(!is_silent(&NotifyConfig {
            enabled: true,
            bell: false,
            osc9: false,
            title: true,
        }));
    }

    // ─── sequences_for: channel combinations ─────────────────────────────

    #[test]
    fn sequences_for_orders_title_then_osc9_then_bell() {
        let config = NotifyConfig {
            enabled: true,
            bell: true,
            osc9: true,
            title: true,
        };
        let event = NotifyEvent::for_kind(NotifyKind::Completed);
        assert_eq!(
            sequences_for(&config, &event, true),
            vec![
                "\u{1b}]0;future: run completed\u{7}".to_string(),
                "\u{1b}]9;future: run completed: The agent finished this turn.\u{7}".to_string(),
                "\u{7}".to_string(),
            ]
        );
    }

    #[test]
    fn sequences_for_honours_each_channel_individually() {
        let event = NotifyEvent::new(
            NotifyKind::ApprovalNeeded,
            "future: approval needed",
            "Waiting.",
        );

        let bell_only = NotifyConfig {
            enabled: true,
            bell: true,
            osc9: false,
            title: false,
        };
        assert_eq!(
            sequences_for(&bell_only, &event, true),
            vec!["\u{7}".to_string()]
        );

        let osc9_only = NotifyConfig {
            enabled: true,
            bell: false,
            osc9: true,
            title: false,
        };
        assert_eq!(
            sequences_for(&osc9_only, &event, true),
            vec!["\u{1b}]9;future: approval needed: Waiting.\u{7}".to_string()]
        );

        let title_only = NotifyConfig {
            enabled: true,
            bell: false,
            osc9: false,
            title: true,
        };
        assert_eq!(
            sequences_for(&title_only, &event, true),
            vec!["\u{1b}]0;future: approval needed\u{7}".to_string()]
        );
    }

    #[test]
    fn title_channel_falls_back_to_the_body_when_the_title_is_empty() {
        let config = NotifyConfig {
            enabled: true,
            bell: false,
            osc9: false,
            title: true,
        };
        let event = NotifyEvent::new(NotifyKind::Failed, "\u{1b}[0m", "Run failed.");
        assert_eq!(
            sequences_for(&config, &event, true),
            vec!["\u{1b}]0;Run failed.\u{7}".to_string()]
        );
    }

    #[test]
    fn default_config_enables_bell_and_osc9_but_not_title() {
        let config = default_config();
        assert_eq!(config, NotifyConfig::default());
        assert!(config.enabled);
        assert!(config.bell);
        assert!(config.osc9);
        assert!(!config.title);
        assert_eq!(
            sequences_for(&config, &NotifyEvent::for_kind(NotifyKind::Completed), true).len(),
            2
        );
    }

    #[test]
    fn notify_config_round_trips_through_json_with_defaults() {
        let config: NotifyConfig = serde_json::from_str(r#"{"bell":false}"#).expect("parse");
        assert_eq!(
            config,
            NotifyConfig {
                enabled: true,
                bell: false,
                osc9: true,
                title: false,
            }
        );
        let json = serde_json::to_value(default_config()).expect("serialize");
        assert_eq!(json["enabled"], serde_json::json!(true));
        assert_eq!(json["osc9"], serde_json::json!(true));
    }

    // ─── NotifyKind wording ───────────────────────────────────────────────

    #[test]
    fn every_kind_has_distinct_wording_and_produces_sequences() {
        let config = NotifyConfig {
            enabled: true,
            bell: true,
            osc9: true,
            title: true,
        };
        let kinds = [
            NotifyKind::Completed,
            NotifyKind::Failed,
            NotifyKind::ApprovalNeeded,
            NotifyKind::InputNeeded,
        ];

        let mut payloads = Vec::new();
        for kind in kinds {
            let event = NotifyEvent::for_kind(kind);
            assert_eq!(event.kind, kind);
            assert!(event.title.starts_with(APP_NAME), "{:?}", event);
            assert!(event.title.contains(kind.label()), "{}", event.title);
            assert!(event.body.ends_with('.'), "{}", event.body);

            let sequences = sequences_for(&config, &event, true);
            assert_eq!(sequences.len(), 3, "{:?}", sequences);
            assert_eq!(sequences[0], set_title_sequence(&event.title));
            assert_eq!(sequences[1], osc9_sequence(&event.title, &event.body));
            assert_eq!(sequences[2], bel_sequence());
            payloads.push(sequences[1].clone());
        }

        let unique: std::collections::HashSet<&String> = payloads.iter().collect();
        assert_eq!(unique.len(), kinds.len(), "{payloads:?}");

        assert_eq!(
            osc9_sequence(&NotifyKind::Failed.default_title(), NotifyKind::Failed.default_body()),
            "\u{1b}]9;future: run failed: The agent hit an error; open the session for details.\u{7}"
        );
        assert_eq!(
            osc9_sequence(
                &NotifyKind::InputNeeded.default_title(),
                NotifyKind::InputNeeded.default_body()
            ),
            "\u{1b}]9;future: input needed: The agent is waiting on your answer.\u{7}"
        );
    }

    #[test]
    fn kind_labels_are_stable_and_non_empty() {
        assert_eq!(NotifyKind::Completed.label(), "run completed");
        assert_eq!(NotifyKind::Failed.label(), "run failed");
        assert_eq!(NotifyKind::ApprovalNeeded.label(), "approval needed");
        assert_eq!(NotifyKind::InputNeeded.label(), "input needed");
        for kind in [
            NotifyKind::Completed,
            NotifyKind::Failed,
            NotifyKind::ApprovalNeeded,
            NotifyKind::InputNeeded,
        ] {
            assert_eq!(
                kind.default_title(),
                format!("{APP_NAME}: {}", kind.label())
            );
            assert!(!kind.default_body().is_empty());
        }
    }

    // ─── project_name / terminal_title ────────────────────────────────────

    #[test]
    fn project_name_takes_the_last_component_on_both_separators() {
        assert_eq!(project_name("/home/me/proj"), "proj");
        assert_eq!(project_name("/home/me/proj/"), "proj");
        assert_eq!(project_name("relative/dir"), "dir");
        assert_eq!(project_name("dir"), "dir");
        assert_eq!(project_name("C:\\Users\\me\\repo"), "repo");
        assert_eq!(project_name("C:\\"), "C:");
        assert_eq!(project_name("\\\\server\\share"), "share");
        assert_eq!(project_name("\\\\server\\share\\"), "share");
    }

    #[test]
    fn project_name_handles_roots_and_empty_input() {
        assert_eq!(project_name("/"), "/");
        assert_eq!(project_name("\\"), "\\");
        assert_eq!(project_name(""), "");
        assert_eq!(project_name("   "), "");
        assert_eq!(project_name("/\u{1b}[31m"), "/");
        assert_eq!(project_name("///"), "///");
    }

    #[test]
    fn project_name_sanitizes_control_characters_in_the_directory() {
        assert_eq!(project_name("/home/me/\u{7}evil"), "evil");
        assert_eq!(project_name("/home/me/ev\u{1b}]0;x\u{7}il"), "evil");
    }

    #[test]
    fn terminal_title_joins_project_model_and_session() {
        assert_eq!(
            terminal_title("/home/me/proj", "gpt-x", None, false),
            "proj | gpt-x"
        );
        assert_eq!(
            terminal_title("/home/me/proj", "gpt-x", Some("nightly task"), false),
            "proj | gpt-x | nightly task"
        );
        assert_eq!(
            terminal_title("/home/me/proj", "gpt-x", Some(""), false),
            "proj | gpt-x"
        );
        assert_eq!(
            terminal_title("/home/me/proj", "gpt-x", Some("  "), false),
            "proj | gpt-x"
        );
    }

    #[test]
    fn terminal_title_omits_missing_parts_and_falls_back_to_the_app_name() {
        assert_eq!(terminal_title("/home/me/proj", "", None, false), "proj");
        assert_eq!(terminal_title("", "gpt-x", None, false), "gpt-x");
        assert_eq!(terminal_title("", "gpt-x", Some("s"), false), "gpt-x | s");
        assert_eq!(terminal_title("", "", None, false), APP_NAME);
        assert_eq!(terminal_title("   ", "  ", Some("  "), false), APP_NAME);
        assert_eq!(terminal_title("/", "", None, false), "/");
    }

    #[test]
    fn terminal_title_marks_streaming_with_a_prefix() {
        let idle = terminal_title("/home/me/proj", "gpt-x", Some("s"), false);
        let streaming = terminal_title("/home/me/proj", "gpt-x", Some("s"), true);
        assert_eq!(idle, "proj | gpt-x | s");
        assert_eq!(streaming, format!("{STREAMING_PREFIX}proj | gpt-x | s"));
        assert!(streaming.starts_with(STREAMING_PREFIX));
        assert!(!idle.starts_with(STREAMING_PREFIX));
        // Same inputs ⇒ same title (stable across renders).
        assert_eq!(
            streaming,
            terminal_title("/home/me/proj", "gpt-x", Some("s"), true)
        );
    }

    #[test]
    fn terminal_title_truncates_to_the_column_budget_keeping_the_prefix() {
        let long_cwd = format!("/home/me/{}", "p".repeat(200));
        let title = terminal_title(&long_cwd, "gpt-x", Some("session"), false);
        assert!(title.ends_with('…'), "{title}");
        assert!(visible_width(&title) <= MAX_TERMINAL_TITLE_WIDTH, "{title}");

        let streaming = terminal_title(&long_cwd, "gpt-x", Some("session"), true);
        assert!(streaming.starts_with(STREAMING_PREFIX), "{streaming}");
        assert!(streaming.ends_with('…'), "{streaming}");
        assert!(
            visible_width(&streaming) <= MAX_TERMINAL_TITLE_WIDTH,
            "{streaming}"
        );
    }

    #[test]
    fn terminal_title_counts_wide_characters_by_columns() {
        let wide = format!("/home/me/{}", "模".repeat(80));
        let title = terminal_title(&wide, "模型", Some("会话"), false);
        assert!(visible_width(&title) <= MAX_TERMINAL_TITLE_WIDTH, "{title}");
        assert!(title.ends_with('…'), "{title}");
    }

    #[test]
    fn terminal_title_sanitizes_untrusted_parts() {
        let title = terminal_title(
            "/home/me/proj\u{7}",
            "mod\u{1b}]0;x\u{7}el",
            Some("se\u{1b}[31mss"),
            false,
        );
        assert_eq!(title, "proj | model | sess");
        assert!(!title.contains('\u{1b}'));
        assert!(!title.contains('\u{7}'));
    }

    #[test]
    fn terminal_title_keeps_a_short_title_untouched_by_truncation() {
        let title = terminal_title("/tmp/demo", "m", None, false);
        assert_eq!(title, "demo | m");
        assert!(!title.contains('…'));
        assert!(visible_width(&title) <= MAX_TERMINAL_TITLE_WIDTH);
    }
}
