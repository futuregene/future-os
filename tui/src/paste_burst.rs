//! Paste-burst detection: recognising a paste on a terminal that does not wrap
//! one in bracketed-paste markers.
//!
//! A terminal that implements bracketed paste (mode 2004) tells the TUI where a
//! paste starts and ends, so `stdin_buffer` hands the app one `Paste` event and
//! a newline inside it can never be mistaken for a submit. Some terminals —
//! the Windows console host is the one that matters — ignore mode 2004, and a
//! paste then arrives as an ordinary burst of keystrokes: the newlines inside it
//! submit the message line by line, and single-key shortcuts (`?` opens help,
//! `tab` opens autocomplete, `y` copies) fire on characters the user never
//! typed.
//!
//! This module is the state machine that recognises such a burst and the gate
//! that decides whether it runs at all. Everything in it is pure: the caller
//! hands it one classified event and the time that event arrived, and gets back
//! a decision. Nothing here reads the clock, spawns a thread or touches a
//! terminal, so the timing rules are tested by feeding instants, never by
//! sleeping.
//!
//! # The rules
//!
//! * **A run of characters closer together than the idle window is a paste.**
//!   [`PASTE_BURST_MIN_CHARS`] characters have to arrive inside the window
//!   before the run counts as one; the first [`PASTE_BURST_MIN_CHARS`]−1 are
//!   held but still delivered as *typing* if the run ends there, so a two-key
//!   reflex (`y` then `Enter`) behaves exactly as it always did.
//! * **The first character of a possible run is held** for at most the idle
//!   window, so a paste never paints its first character and then replaces it
//!   with a paste (the flicker the reference implementation's hold exists to
//!   avoid). A run that turns out to be typing is flushed, in order, as the
//!   same per-character events it would have produced.
//! * **`Enter` inside a run is text, not a submit**; the same for `tab`, which
//!   otherwise opens the autocomplete popup from the middle of a paste. Outside
//!   a run both keys are passed straight through, so only a *confirmed* burst
//!   can ever swallow a key press.
//! * **Anything that is not a plain character ends the run**: escape sequences
//!   and control keys flush what is held and then take their normal path, so a
//!   burst can never reorder or delay a real key.
//!
//! # The gate
//!
//! [`PasteBurstGate::for_host`] arms the machine only where bracketed paste is
//! known to be missing — see [`idle_for`] and [`gate_from_env`]. Everywhere
//! else a paste already arrives wrapped, and leaving the machine off is what
//! keeps ordinary fast typing byte-identical to the behaviour before it existed.
//! `FUTURE_TUI_PASTE_BURST=on|off` overrides the platform default for a terminal
//! whose behaviour the probe cannot know.

use std::time::{Duration, Instant};

/// Characters that have to arrive inside the idle window before a run counts as
/// a paste rather than typing.
///
/// Three, matching the reference implementation: two characters can be a fast
/// reflex (`y` `Enter`, `q` `Enter`) and must never be turned into a paste,
/// while a paste of a single word is at least three. Pinned by tests on both
/// sides of the boundary.
pub const PASTE_BURST_MIN_CHARS: usize = 3;

/// Idle gap that ends a run on a terminal whose input arrives promptly (the
/// X11/Wayland/macOS path). Measured on this host: every character of one paste
/// arrives in a single read from the pty, i.e. microseconds apart, while a
/// human typing at 20 characters per second is 50 ms apart — one and a half
/// orders of magnitude of headroom on either side.
pub const PASTE_BURST_IDLE_MS: u64 = 8;

/// Idle gap on Windows, where the console delivers input in coarser lumps than a
/// pty does: the reference implementation uses 60 ms there, and this host cannot
/// measure the real figure (no Windows console to paste into), so the value is
/// taken from the reference rather than invented. A tighter window would make
/// the machine miss the very pastes it exists for; the safety net for a false
/// positive is [`PASTE_BURST_MIN_CHARS`], not the window.
pub const PASTE_BURST_IDLE_MS_WINDOWS: u64 = 60;

/// Environment override for the gate: `on`/`off` (also `1`/`0`, `true`/`false`,
/// `yes`/`no`, case-insensitive). Anything else — including unset — keeps the
/// platform default.
pub const PASTE_BURST_ENV: &str = "FUTURE_TUI_PASTE_BURST";

/// The idle window for a platform name (`std::env::consts::OS`).
pub fn idle_for(platform: &str) -> Duration {
    if platform.eq_ignore_ascii_case("windows") {
        Duration::from_millis(PASTE_BURST_IDLE_MS_WINDOWS)
    } else {
        Duration::from_millis(PASTE_BURST_IDLE_MS)
    }
}

/// Whether the machine runs for `platform`, from an [`PASTE_BURST_ENV`] value.
///
/// The default is "no" everywhere except Windows. A terminal that wraps pastes
/// itself needs none of this, and *arming* the machine is what can steal an
/// `Enter` from a fast typist, so the risky default is the off one: the platform
/// where bracketed paste is known to be missing is the one that gets it. A
/// user on some other terminal that ignores mode 2004 turns it on explicitly.
pub fn gate_from_env(value: Option<&str>, platform: &str) -> bool {
    match value.map(|raw| raw.trim().to_ascii_lowercase()) {
        Some(flag) if matches!(flag.as_str(), "1" | "on" | "true" | "yes") => true,
        Some(flag) if matches!(flag.as_str(), "0" | "off" | "false" | "no") => false,
        _ => platform.eq_ignore_ascii_case("windows"),
    }
}

/// The gate for this process: the platform default, overridden by
/// [`PASTE_BURST_ENV`].
pub fn gate_for_host() -> bool {
    gate_from_env(
        std::env::var(PASTE_BURST_ENV).ok().as_deref(),
        std::env::consts::OS,
    )
}

/// How one input event sits in a burst.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// Exactly one printable character: the only kind that can start or extend
    /// a run.
    Char(char),
    /// `Enter` (`\r` or `\n` as the terminal sends it).
    Newline,
    /// `Tab`.
    Tab,
    /// Anything else — an escape sequence, a control key, a pasted block the
    /// terminal did wrap. Runs end here and the event takes its normal path.
    Other,
}

/// Classify one `stdin_buffer` event for the burst machine.
pub fn classify(sequence: &str) -> EventKind {
    match sequence {
        "\r" | "\n" => return EventKind::Newline,
        "\t" => return EventKind::Tab,
        _ => {}
    }
    let mut chars = sequence.chars();
    match (chars.next(), chars.next()) {
        // `is_control` covers C0, DEL and C1 (including ESC), so an escape
        // sequence — or a lone ESC — is never a plain character.
        (Some(ch), None) if !ch.is_control() => EventKind::Char(ch),
        _ => EventKind::Other,
    }
}

/// What a finished run of held characters turns out to have been.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BurstText {
    /// Typing: deliver each character as its own input event, exactly as the
    /// terminal delivers normal keys.
    Typed(String),
    /// A paste: deliver the run as one bracketed paste, so the app folds,
    /// attaches or inserts it through the paste path.
    Paste(String),
}

/// What the caller should do with the event it just classified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BurstStep {
    /// Nothing yet: the character is held (a run may be forming).
    Hold,
    /// Deliver this finished run first; the new character is now held.
    Emit(BurstText),
}

/// The burst machine.
///
/// Invariant: `held` is empty exactly when `last_char_at` is `None`, and
/// `is_burst` is set once `held` has reached [`PASTE_BURST_MIN_CHARS`] — so a
/// finished run is `Paste` when it got that far and `Typed` otherwise.
#[derive(Debug)]
pub struct PasteBurst {
    idle: Duration,
    held: String,
    last_char_at: Option<Instant>,
    is_burst: bool,
}

impl PasteBurst {
    pub fn new(idle: Duration) -> Self {
        Self {
            idle,
            held: String::new(),
            last_char_at: None,
            is_burst: false,
        }
    }

    /// The idle window this machine runs with.
    pub fn idle(&self) -> Duration {
        self.idle
    }

    /// Is a run being held back right now?
    pub fn is_active(&self) -> bool {
        self.last_char_at.is_some()
    }

    /// Has the held run already been confirmed as a paste?
    pub fn is_burst(&self) -> bool {
        self.is_burst
    }

    /// When the held run must be released (the reader loop wakes for this), or
    /// `None` when nothing is held.
    pub fn deadline(&self) -> Option<Instant> {
        self.last_char_at.map(|at| at + self.idle)
    }

    /// Feed one plain character.
    pub fn on_char(&mut self, ch: char, now: Instant) -> BurstStep {
        if let Some(at) = self.last_char_at {
            if now.duration_since(at) >= self.idle {
                // The gap ended the earlier run: it is emitted first, and this
                // character starts a new one.
                let released = self.release();
                self.hold(ch, now);
                return BurstStep::Emit(released);
            }
        }
        self.hold(ch, now);
        BurstStep::Hold
    }

    /// Feed `Enter`. `true` means it belongs to the running burst and became a
    /// newline in the pasted text (the caller must not submit); `false` means it
    /// is an ordinary key press.
    pub fn on_newline(&mut self, now: Instant) -> bool {
        self.absorb('\n', now)
    }

    /// Feed `Tab`. `true` means it became a tab character inside the pasted
    /// text (the caller must not open the autocomplete popup).
    pub fn on_tab(&mut self, now: Instant) -> bool {
        self.absorb('\t', now)
    }

    /// Take the run once its deadline has passed, if it has.
    pub fn take_due(&mut self, now: Instant) -> Option<BurstText> {
        match self.last_char_at {
            Some(at) if now.duration_since(at) >= self.idle => Some(self.release()),
            _ => None,
        }
    }

    /// Take whatever is held, whatever the time says — a non-character event
    /// arrived (or a modal is draining input) and the run cannot continue.
    pub fn flush(&mut self) -> Option<BurstText> {
        self.last_char_at.map(|_| self.release())
    }

    /// Append a key that is text only when a burst is already running.
    fn absorb(&mut self, ch: char, now: Instant) -> bool {
        if !self.is_burst {
            return false;
        }
        self.hold(ch, now);
        true
    }

    /// Add a character to the held run.
    fn hold(&mut self, ch: char, now: Instant) {
        self.held.push(ch);
        self.last_char_at = Some(now);
        if !self.is_burst && self.held.chars().count() >= PASTE_BURST_MIN_CHARS {
            self.is_burst = true;
        }
    }

    /// End the run: reset the state and hand back what it was.
    fn release(&mut self) -> BurstText {
        let text = std::mem::take(&mut self.held);
        let paste = self.is_burst;
        self.last_char_at = None;
        self.is_burst = false;
        if paste {
            BurstText::Paste(text)
        } else {
            BurstText::Typed(text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed instant plus an offset in milliseconds — the tests never read the
    /// clock while deciding anything, and one base instant keeps the offsets
    /// comparable.
    fn at(ms: u64) -> Instant {
        static BASE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        *BASE.get_or_init(Instant::now) + Duration::from_millis(ms)
    }

    fn burst() -> PasteBurst {
        PasteBurst::new(Duration::from_millis(PASTE_BURST_IDLE_MS))
    }

    /// Feed `text` character by character `gap` apart, starting at `start` ms,
    /// and return everything released along the way.
    fn feed(machine: &mut PasteBurst, text: &str, gap: u64, start: u64) -> Vec<BurstText> {
        let mut released = Vec::new();
        for (index, ch) in text.chars().enumerate() {
            let now = at(start + gap * index as u64);
            if let BurstStep::Emit(text) = machine.on_char(ch, now) {
                released.push(text);
            }
        }
        released
    }

    // ─── classification ────────────────────────────────────────────────

    #[test]
    fn classify_separates_characters_from_keys_and_sequences() {
        assert_eq!(classify("a"), EventKind::Char('a'));
        assert_eq!(classify("é"), EventKind::Char('é'));
        assert_eq!(classify("🚀"), EventKind::Char('🚀'));
        assert_eq!(classify("\r"), EventKind::Newline);
        assert_eq!(classify("\n"), EventKind::Newline);
        assert_eq!(classify("\t"), EventKind::Tab);
        // Everything else is not a plain character: control keys, escape
        // sequences, and any multi-character block.
        for other in [
            "\x1b",
            "\x03",
            "\x7f",
            "\x1b[A",
            "\x1ba",
            "\x1b[200~x\x1b[201~",
            "ab",
            "",
        ] {
            assert_eq!(classify(other), EventKind::Other, "{other:?}");
        }
    }

    // ─── the gate ──────────────────────────────────────────────────────

    #[test]
    fn the_gate_arms_only_where_bracketed_paste_is_missing() {
        // Platform default: off everywhere the terminal wraps pastes itself.
        for platform in ["macos", "linux", "android", "freebsd"] {
            assert!(!gate_from_env(None, platform), "{platform}");
        }
        assert!(gate_from_env(None, "windows"));
        assert!(gate_from_env(None, "Windows"));
        // An unparseable value keeps the default rather than guessing.
        assert!(!gate_from_env(Some("maybe"), "linux"));
        assert!(gate_from_env(Some("maybe"), "windows"));
    }

    #[test]
    fn the_gate_honours_an_explicit_override_on_every_platform() {
        for on in ["1", "on", "ON", " true ", "yes", "Yes"] {
            assert!(gate_from_env(Some(on), "linux"), "{on:?}");
            assert!(gate_from_env(Some(on), "macos"), "{on:?}");
        }
        for off in ["0", "off", "OFF", " false ", "no", "No"] {
            assert!(!gate_from_env(Some(off), "windows"), "{off:?}");
            assert!(!gate_from_env(Some(off), "linux"), "{off:?}");
        }
    }

    #[test]
    fn the_idle_window_is_looser_on_windows() {
        assert_eq!(idle_for("windows"), Duration::from_millis(60));
        assert_eq!(idle_for("Windows"), idle_for("windows"));
        assert_eq!(
            idle_for("macos"),
            Duration::from_millis(PASTE_BURST_IDLE_MS)
        );
        assert_eq!(idle_for("linux"), idle_for("macos"));
        // The Windows window has to be the looser one: the console's own input
        // granularity is why the platform needs this machine at all.
        assert!(idle_for("windows") > idle_for("linux"));
    }

    // ─── a burst is recognised ─────────────────────────────────────────

    #[test]
    fn characters_inside_the_window_are_held_and_released_as_one_paste() {
        let mut machine = burst();
        // Nothing reaches the app while the run is open…
        assert!(feed(&mut machine, "hello", 1, 0).is_empty());
        assert!(machine.is_active());
        assert!(machine.is_burst());
        assert_eq!(machine.deadline(), Some(at(4) + machine.idle()));
        // …and the whole run arrives at once when the window closes.
        assert_eq!(
            machine.take_due(at(4) + machine.idle()),
            Some(BurstText::Paste("hello".to_string()))
        );
        assert!(!machine.is_active());
        assert!(!machine.is_burst());
        // Nothing is left to release twice.
        assert_eq!(machine.take_due(at(100)), None);
        assert_eq!(machine.flush(), None);
    }

    #[test]
    fn a_newline_inside_a_burst_becomes_text_and_never_submits() {
        let mut machine = burst();
        feed(&mut machine, "one", 1, 0);
        // Enter arrives with the burst still open: it is text…
        assert!(machine.on_newline(at(4)));
        // …the run stays open, and the next line joins the same paste.
        assert!(feed(&mut machine, "two", 1, 5).is_empty());
        // A second Enter, still inside the run, is text as well.
        assert!(machine.on_newline(at(9)));
        assert_eq!(
            machine.flush(),
            Some(BurstText::Paste("one\ntwo\n".to_string()))
        );
    }

    #[test]
    fn a_tab_inside_a_burst_is_text_and_does_not_open_the_autocomplete() {
        let mut machine = burst();
        feed(&mut machine, "ab", 1, 0);
        // …but on its own (no run open) it is a key press, so the autocomplete
        // still opens from a tab the user really pressed.
        assert!(!machine.on_tab(at(2)));
        assert!(!machine.is_burst());
        assert_eq!(machine.flush(), Some(BurstText::Typed("ab".to_string())));
        // Three characters in, the tab joins the pasted text.
        let mut machine = burst();
        feed(&mut machine, "abc", 1, 0);
        assert!(machine.on_tab(at(3)));
        assert!(feed(&mut machine, "😀", 1, 4).is_empty());
        assert_eq!(
            machine.flush(),
            Some(BurstText::Paste("abc\t😀".to_string()))
        );
    }

    #[test]
    fn the_paste_text_is_delivered_verbatim_including_ui_shortcuts() {
        // `?` and `y` are the keys that do something on their own; inside a
        // burst they are just characters on their way to the input box.
        let mut machine = burst();
        feed(&mut machine, "?y/", 1, 0);
        assert!(machine.on_newline(at(3)));
        feed(&mut machine, "q/help", 1, 4);
        assert_eq!(
            machine.flush(),
            Some(BurstText::Paste("?y/\nq/help".to_string()))
        );
    }

    // ─── typing is not ─────────────────────────────────────────────────

    #[test]
    fn a_gap_larger_than_the_window_keeps_typing_intact() {
        let mut machine = burst();
        // Human-speed typing: every character is released as typing, in order,
        // and the last one is released by the deadline rather than held.
        let mut released = feed(&mut machine, "hello", 50, 0);
        assert_eq!(
            released,
            vec![
                BurstText::Typed("h".to_string()),
                BurstText::Typed("e".to_string()),
                BurstText::Typed("l".to_string()),
                BurstText::Typed("l".to_string()),
            ]
        );
        released.extend(machine.take_due(at(200 + PASTE_BURST_IDLE_MS)));
        assert_eq!(released.len(), 5);
        assert!(
            released
                .iter()
                .all(|text| matches!(text, BurstText::Typed(_))),
            "a slow typist never produces a paste"
        );
        assert!(!machine.is_active());

        // Key repeat (holding a key down) is slower than the window too.
        let mut machine = burst();
        let repeated = feed(&mut machine, "aaaaaaaa", 30, 0);
        assert_eq!(repeated.len(), 7);
        assert!(repeated
            .iter()
            .all(|text| matches!(text, BurstText::Typed(_))));
    }

    #[test]
    fn two_fast_characters_are_still_typing_not_a_paste() {
        // The boundary: PASTE_BURST_MIN_CHARS − 1 characters held inside the
        // window are released as typing, so `?` still opens help and `Enter`
        // still submits.
        let mut machine = burst();
        assert!(feed(&mut machine, "?y", 1, 0).is_empty());
        assert!(machine.is_active());
        assert!(!machine.is_burst(), "two characters are not a paste");
        // Enter outside a burst is a key press, never text.
        assert!(!machine.on_newline(at(2)));
        assert_eq!(machine.flush(), Some(BurstText::Typed("?y".to_string())));
    }

    #[test]
    fn the_character_that_completes_the_minimum_is_what_confirms_a_burst() {
        let mut machine = burst();
        assert_eq!(machine.on_char('a', at(0)), BurstStep::Hold);
        assert!(!machine.is_burst());
        assert_eq!(machine.on_char('b', at(1)), BurstStep::Hold);
        assert!(!machine.is_burst(), "two characters are not a paste yet");
        assert_eq!(machine.on_char('c', at(2)), BurstStep::Hold);
        assert!(machine.is_burst(), "the third character confirms it");
        // A confirmation is what lets a tab sit inside the run.
        assert!(machine.on_tab(at(3)));
        assert_eq!(machine.flush(), Some(BurstText::Paste("abc\t".to_string())));
    }

    #[test]
    fn a_break_before_the_minimum_releases_typing_and_holds_the_new_character() {
        let mut machine = burst();
        assert!(feed(&mut machine, "ab", 1, 0).is_empty());
        // A third character much later ends the two-character run as typing and
        // starts a new one.
        assert_eq!(
            machine.on_char('c', at(100)),
            BurstStep::Emit(BurstText::Typed("ab".to_string()))
        );
        assert!(machine.is_active());
        assert!(!machine.is_burst(), "the new run starts from scratch");
        assert_eq!(machine.flush(), Some(BurstText::Typed("c".to_string())));
    }

    #[test]
    fn a_break_after_the_minimum_releases_a_paste() {
        let mut machine = burst();
        feed(&mut machine, "paste this", 1, 0);
        assert_eq!(
            machine.on_char('x', at(500)),
            BurstStep::Emit(BurstText::Paste("paste this".to_string()))
        );
        assert_eq!(machine.flush(), Some(BurstText::Typed("x".to_string())));
    }

    // ─── the hold is bounded ───────────────────────────────────────────

    #[test]
    fn a_held_first_character_is_released_by_the_deadline() {
        let mut machine = burst();
        assert_eq!(machine.on_char('h', at(0)), BurstStep::Hold);
        assert!(!machine.is_burst());
        assert_eq!(machine.deadline(), Some(at(0) + machine.idle()));
        // Before the deadline nothing is due…
        assert_eq!(
            machine.take_due(at(0) + machine.idle() - Duration::from_millis(1)),
            None
        );
        // …at it, the character arrives as typing: no character is lost.
        assert_eq!(
            machine.take_due(at(0) + machine.idle()),
            Some(BurstText::Typed("h".to_string()))
        );
        assert!(!machine.is_active());
    }

    #[test]
    fn the_deadline_moves_with_every_character_of_a_running_burst() {
        let mut machine = burst();
        feed(&mut machine, "abc", 1, 0);
        assert_eq!(machine.deadline(), Some(at(2) + machine.idle()));
        // A gap that is still inside the window extends the run instead of
        // closing it, so a long paste is one paste and not one per chunk.
        assert_eq!(
            machine.on_char('d', at(2 + PASTE_BURST_IDLE_MS - 1)),
            BurstStep::Hold
        );
        assert_eq!(
            machine.deadline(),
            Some(at(2 + PASTE_BURST_IDLE_MS - 1) + machine.idle())
        );
        assert!(machine.is_burst());
    }

    #[test]
    fn a_long_burst_may_cross_many_chunks() {
        // A pasted block arrives in 4 KiB reads; as long as the reads follow
        // each other inside the window it stays one paste.
        let mut machine = burst();
        let mut now = 0;
        for _ in 0..8 {
            for ch in "x".repeat(4096).chars() {
                assert_eq!(machine.on_char(ch, at(now)), BurstStep::Hold);
                now += 1;
            }
        }
        assert!(machine.is_burst());
        let expected = BurstText::Paste("x".repeat(8 * 4096));
        assert_eq!(machine.flush().expect("the run is released"), expected);
    }
}
