//! Clipboard copy backends for the TUI (`/copy`, `ctrl+y`, pager `y`).
//!
//! Three delivery paths, preferred in this order:
//!
//! 1. **Native** — pipe the text into a desktop clipboard program (`pbcopy`,
//!    `clip.exe`, `wl-copy`/`xclip`/`xsel`). This is the only path that
//!    *confirms* delivery, so it is always attempted first, also inside tmux:
//!    an OSC 52 write can race with (and clear) a fresh native selection, and
//!    the reverse order is what users notice as "copy did nothing".
//! 2. **tmux passthrough** — when native is unavailable/failed and we are in
//!    tmux, the caller writes an OSC 52 request wrapped in
//!    `ESC Ptmux; … ESC \` so it survives tmux and reaches the attached
//!    terminal (a persistent tmux session may gain clients after we started).
//! 3. **OSC 52** — same request without the tmux wrapper, for a plain
//!    terminal. Supported by kitty, WezTerm, iTerm2, Ghostty, xterm, …
//!
//! Terminal writes have no acknowledgement: they are *requests*. That is why
//! `copy()` distinguishes `Copied` from `Requested`; a `Requested` outcome must
//! be shown as unconfirmed, and the sequence to write is fetched with
//! [`Clipboard::osc52_sequence`] (already tmux-wrapped when the last `copy()`
//! ran inside tmux). Outside tmux and without a terminal to write to there is no
//! path left at all, so the outcome is `Failed` rather than a silent no-op.
//!
//! Everything that touches the outside world is injectable:
//! [`Clipboard::with_runner`] replaces the process spawner (tests never spawn a
//! real clipboard program), [`Clipboard::with_parts`] additionally fixes the
//! candidate list so tests do not depend on the host OS.
//!
//! Deliberate limits: empty/whitespace-only text never reaches any clipboard
//! (`Failed("empty")`), and a payload above [`OSC52_MAX_RAW_BYTES`] fails
//! before anything is attempted — a single cap keeps the outcome of a copy
//! deterministic regardless of which backend happens to exist on the host.

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use base64::Engine as _;

/// Largest raw payload (bytes) accepted for a copy. Sized from the transport
/// rather than from another program: the payload travels base64-encoded inside
/// a single OSC 52 escape sequence, so it has to stay well under the escape
/// sequence limits terminals and multiplexers enforce. 100 KB fits a full
/// transcript selection while keeping the encoded sequence near 133 KB, which
/// is small enough that a terminal is never asked to swallow a megabyte in one
/// write.
pub const OSC52_MAX_RAW_BYTES: usize = 100_000;

/// Result of [`Clipboard::copy`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyOutcome {
    /// A native clipboard program exited successfully — delivery is confirmed.
    Copied,
    /// Native was unavailable or failed; the caller must write
    /// `Clipboard::osc52_sequence(text)` to the terminal. Delivery is an
    /// unacknowledged request and should be reported as unconfirmed.
    Requested,
    /// Nothing was copied. The payload is a short, user-facing reason.
    Failed(String),
}

/// Which mechanism a copy would use for a given session shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyTarget {
    /// A native clipboard program.
    Native,
    /// An OSC 52 escape sequence written to the terminal.
    ///
    /// [`Clipboard::detect_target`] never returns this (it cannot know whether
    /// a native program exists); [`Clipboard::resolve_target`] and
    /// [`Clipboard::copy`] do, when a terminal is writable but no native
    /// backend is available.
    Osc52,
    /// An OSC 52 sequence wrapped for tmux (`ESC Ptmux; … ESC \`).
    Tmux,
    /// No usable path: no native program and no terminal to write to.
    None,
}

/// Spawns a clipboard program, feeds it `text` on stdin and reports whether it
/// exited successfully. Injected so tests never spawn anything.
pub type ClipboardRunner = Box<dyn Fn(&str, &[String], &str) -> Result<(), String> + Send + Sync>;

/// A clipboard writer bound to one environment.
///
/// `new()` probes the environment once (session programs available for this OS
/// and the tmux/ssh facts); every `copy()` refreshes the tmux fact, because
/// callers pass it explicitly and `osc52_sequence()` must agree with the
/// outcome the caller just received.
pub struct Clipboard {
    runner: ClipboardRunner,
    candidates: Vec<(String, Vec<String>)>,
    tmux: AtomicBool,
    ssh: bool,
}

impl Clipboard {
    /// Probe this process's environment: which native programs apply to this
    /// OS in this session (Wayland/X11), whether we are inside tmux, and
    /// whether we are on the far end of an SSH connection.
    pub fn new() -> Self {
        let (tmux, wayland, x11) = session_environment();
        let ssh = is_ssh_from_env(
            env_value("SSH_CONNECTION").as_deref(),
            env_value("SSH_TTY").as_deref(),
        );
        Self {
            runner: Box::new(native_runner),
            candidates: native_candidates_for(std::env::consts::OS, wayland, x11),
            tmux: AtomicBool::new(tmux),
            ssh,
        }
    }

    /// Like [`Clipboard::new`] but with an injected process spawner.
    pub fn with_runner(runner: ClipboardRunner) -> Self {
        Self {
            runner,
            ..Self::new()
        }
    }

    /// Fully injectable constructor: no environment probing at all. Used by
    /// tests (they pin the candidate list instead of depending on the host OS)
    /// and by callers that already know the session shape.
    pub fn with_parts(
        runner: ClipboardRunner,
        candidates: Vec<(String, Vec<String>)>,
        in_tmux: bool,
    ) -> Self {
        Self {
            runner,
            candidates,
            tmux: AtomicBool::new(in_tmux),
            ssh: false,
        }
    }

    /// Native programs this clipboard would try, in order.
    pub fn candidates(&self) -> &[(String, Vec<String>)] {
        &self.candidates
    }

    /// Whether this process was started behind SSH (captured by `new()`).
    pub fn is_ssh(&self) -> bool {
        self.ssh
    }

    /// The strategy for the given session shape, without native probing:
    /// `Tmux > Native > Osc52 > None` collapsed to what two bits can express —
    /// tmux wins, an interactive terminal means the desktop clipboard is the
    /// expected path, and a non-interactive stdout has neither a terminal to
    /// write to nor evidence of a native backend, so it is `None`.
    ///
    /// Use [`Clipboard::resolve_target`] when the candidate list is known.
    pub fn detect_target(in_tmux: bool, is_tty: bool) -> CopyTarget {
        if in_tmux {
            CopyTarget::Tmux
        } else if is_tty {
            CopyTarget::Native
        } else {
            CopyTarget::None
        }
    }

    /// The strategy for the given session shape *and* this clipboard's native
    /// candidates — the full `Tmux > Native > Osc52 > None` ladder: tmux
    /// passthrough when in tmux, otherwise a native program when one exists,
    /// otherwise an OSC 52 request when we can write to a terminal, otherwise
    /// nothing.
    pub fn resolve_target(&self, in_tmux: bool, is_tty: bool) -> CopyTarget {
        if in_tmux {
            CopyTarget::Tmux
        } else if !self.candidates.is_empty() {
            CopyTarget::Native
        } else if is_tty {
            CopyTarget::Osc52
        } else {
            CopyTarget::None
        }
    }

    /// Copy `text` to the clipboard.
    ///
    /// `in_tmux` / `is_tty` describe the caller's terminal. On `Requested` the
    /// caller writes [`Clipboard::osc52_sequence`] to the terminal itself.
    pub fn copy(&self, text: &str, in_tmux: bool, is_tty: bool) -> CopyOutcome {
        // Remember the session shape first: `osc52_sequence()` is called after
        // this returns and must produce the sequence matching this outcome.
        self.tmux.store(in_tmux, Ordering::Relaxed);

        // Neither an empty selection nor an oversized one may touch a
        // clipboard: a no-op here is a bug report, not a silent paste of a
        // stale buffer.
        if text.trim().is_empty() {
            return CopyOutcome::Failed("empty".to_string());
        }
        if encode_osc52(text).is_none() {
            return CopyOutcome::Failed(format!(
                "payload too large ({} bytes; max {OSC52_MAX_RAW_BYTES})",
                text.len()
            ));
        }

        if !self.candidates.is_empty() {
            return match self.write_native(text) {
                Ok(()) => CopyOutcome::Copied,
                Err(error) => self.terminal_only(&error, in_tmux, is_tty),
            };
        }
        self.terminal_only(
            "no native clipboard program for this session",
            in_tmux,
            is_tty,
        )
    }

    /// The fallback when no native write happened: ask the terminal, or fail.
    fn terminal_only(&self, native_error: &str, in_tmux: bool, is_tty: bool) -> CopyOutcome {
        if in_tmux || is_tty {
            CopyOutcome::Requested
        } else {
            CopyOutcome::Failed(format!(
                "no clipboard backend: {native_error}; output is not a terminal"
            ))
        }
    }

    /// The OSC 52 sequence to write for a `Requested` result, tmux-wrapped
    /// when the most recent `copy()` ran inside tmux. `None` when `text` is
    /// above [`OSC52_MAX_RAW_BYTES`].
    ///
    /// A caller may also write this after a `Copied` outcome when it wants
    /// clients attached to a tmux session to receive the text too — the
    /// sequence is a request, so it never invalidates the native confirmation.
    pub fn osc52_sequence(&self, text: &str) -> Option<String> {
        let sequence = encode_osc52(text)?;
        if self.tmux.load(Ordering::Relaxed) {
            Some(tmux_passthrough(&sequence))
        } else {
            Some(sequence)
        }
    }

    /// First candidate that exits successfully wins; otherwise report the last
    /// failure so the user sees why nothing was copied.
    fn write_native(&self, text: &str) -> Result<(), String> {
        let mut last_error = None;
        for (program, args) in &self.candidates {
            match (self.runner)(program, args, text) {
                Ok(()) => return Ok(()),
                Err(error) => last_error = Some(format!("{program}: {error}")),
            }
        }
        Err(last_error.unwrap_or_else(|| "no native clipboard program".to_string()))
    }
}

impl Default for Clipboard {
    fn default() -> Self {
        Self::new()
    }
}

// ─── OSC 52 ────────────────────────────────────────────────────────────────

/// Base64-encode `text` into a ready-to-write OSC 52 sequence
/// (`ESC ]52;c;<base64> BEL`), or `None` above [`OSC52_MAX_RAW_BYTES`].
///
/// The sequence is *not* tmux-wrapped; use [`tmux_passthrough`] (or
/// [`Clipboard::osc52_sequence`]) when writing inside tmux.
pub fn encode_osc52(text: &str) -> Option<String> {
    if text.len() > OSC52_MAX_RAW_BYTES {
        return None;
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    Some(format!("\x1b]52;c;{encoded}\x07"))
}

/// Wrap a sequence so tmux forwards it to the attached terminal: the tmux DCS
/// passthrough forbids bare escapes inside, so every `ESC` is doubled.
pub fn tmux_passthrough(sequence: &str) -> String {
    format!("\x1bPtmux;{}\x1b\\", sequence.replace('\x1b', "\x1b\x1b"))
}

// ─── Native program selection ──────────────────────────────────────────────

/// The native clipboard programs to try for an OS, best first.
///
/// Linux has no single answer: `wl-copy` on Wayland, `xclip`/`xsel` on X11
/// (also reachable through XWayland, hence appended when `x11`). A session with
/// neither `WAYLAND_DISPLAY` nor `DISPLAY` has no clipboard to write to, so the
/// list is empty and the caller falls back to the terminal.
pub fn native_candidates_for(os: &str, wayland: bool, x11: bool) -> Vec<(String, Vec<String>)> {
    match os.trim().to_ascii_lowercase().as_str() {
        "macos" | "mac" | "darwin" | "ios" => vec![("pbcopy".to_string(), Vec::new())],
        "windows" | "win32" | "windows_nt" => vec![("clip.exe".to_string(), Vec::new())],
        "linux" | "android" => {
            let mut candidates = Vec::new();
            if wayland {
                candidates.push(("wl-copy".to_string(), Vec::new()));
            }
            if x11 {
                candidates.push((
                    "xclip".to_string(),
                    vec!["-selection".to_string(), "clipboard".to_string()],
                ));
                candidates.push((
                    "xsel".to_string(),
                    vec!["--clipboard".to_string(), "--input".to_string()],
                ));
            }
            candidates
        }
        _ => Vec::new(),
    }
}

/// The preferred native clipboard command for an OS, ignoring session type —
/// `pbcopy` (macOS), `clip.exe` (Windows), `wl-copy` (Linux, first candidate).
pub fn native_command_for(os: &str) -> Option<(String, Vec<String>)> {
    native_candidates_for(os, true, true).into_iter().next()
}

// ─── Environment probes ────────────────────────────────────────────────────

/// `TMUX`/`TMUX_PANE` are set (to a non-blank value) inside a tmux pane.
pub fn is_tmux_from_env(tmux: Option<&str>) -> bool {
    tmux.map(|value| !value.trim().is_empty()).unwrap_or(false)
}

/// `SSH_CONNECTION`/`SSH_TTY` are set (to a non-blank value) behind SSH.
pub fn is_ssh_from_env(ssh_conn: Option<&str>, ssh_tty: Option<&str>) -> bool {
    is_tmux_from_env(ssh_conn) || is_tmux_from_env(ssh_tty)
}

fn env_value(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|value| !value.is_empty())
}

/// Whether either tmux variable marks this process as inside a tmux pane.
fn tmux_from_vars(tmux: Option<&str>, pane: Option<&str>) -> bool {
    is_tmux_from_env(tmux) || is_tmux_from_env(pane)
}

/// `(in_tmux, wayland, x11)` for this process.
fn session_environment() -> (bool, bool, bool) {
    let tmux = tmux_from_vars(
        env_value("TMUX").as_deref(),
        env_value("TMUX_PANE").as_deref(),
    );
    (
        tmux,
        env_value("WAYLAND_DISPLAY").is_some(),
        env_value("DISPLAY").is_some(),
    )
}

// ─── Default runner ────────────────────────────────────────────────────────

/// Spawn `program` with the pipes a clipboard program expects: the payload on
/// stdin, stderr captured so a failure surfaces as a message instead of
/// corrupting the alternate screen the TUI owns.
fn spawn_native(program: &str, args: &[String]) -> Result<Child, String> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to spawn: {error}"))
}

/// Pipe `text` into `child` on stdin (the convention for `pbcopy`, `clip.exe`,
/// `wl-copy`, `xclip -i`, `xsel -i`) and report its exit status.
fn run_native(mut child: Child, text: &str) -> Result<(), String> {
    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("failed to open stdin".to_string());
    };
    if let Err(error) = stdin.write_all(text.as_bytes()) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("failed to write: {error}"));
    }
    drop(stdin);

    let output = child
        .wait_with_output()
        .map_err(|error| format!("failed to wait: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        Err(format!("exited with status {}", output.status))
    } else {
        Err(format!("failed: {stderr}"))
    }
}

/// Default [`ClipboardRunner`]: spawn `program` and pipe `text` into it.
fn native_runner(program: &str, args: &[String], text: &str) -> Result<(), String> {
    run_native(spawn_native(program, args)?, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    type Log = Arc<Mutex<Vec<(String, Vec<String>, String)>>>;

    /// Runner that records every call and fails for the named programs.
    fn recording_runner(failing: &[&str]) -> (ClipboardRunner, Log) {
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&log);
        let failing: Vec<String> = failing.iter().map(|name| name.to_string()).collect();
        let runner: ClipboardRunner =
            Box::new(move |program: &str, args: &[String], text: &str| {
                sink.lock()
                    .unwrap()
                    .push((program.to_string(), args.to_vec(), text.to_string()));
                if failing.iter().any(|name| name == program) {
                    Err(format!("{program} is not reachable"))
                } else {
                    Ok(())
                }
            });
        (runner, log)
    }

    /// Runner body for [`forbidden_runner`]: reaching a clipboard from a branch
    /// that must not touch one is a bug, so it aborts loudly.
    fn no_clipboard_allowed(program: &str, _args: &[String], _text: &str) -> Result<(), String> {
        panic!("clipboard runner must not be called for {program}")
    }

    /// Runner that must never be invoked — proves a branch touches no clipboard.
    fn forbidden_runner() -> ClipboardRunner {
        Box::new(no_clipboard_allowed)
    }

    /// The failure reason of a copy outcome; an outcome that unexpectedly
    /// succeeded is described instead, so the caller's `contains` assertion
    /// still fails and says why (no arm here is unreachable).
    fn failure_reason(outcome: CopyOutcome) -> String {
        match outcome {
            CopyOutcome::Failed(reason) => reason,
            other => format!("not a failure: {other:?}"),
        }
    }

    fn recorded(log: &Log) -> Vec<(String, Vec<String>, String)> {
        log.lock().unwrap().clone()
    }

    fn native(candidates: Vec<(String, Vec<String>)>) -> Vec<(String, Vec<String>)> {
        candidates
    }

    fn pbcopy() -> Vec<(String, Vec<String>)> {
        native(vec![("pbcopy".to_string(), Vec::new())])
    }

    fn clipboard_with(
        failing: &[&str],
        candidates: Vec<(String, Vec<String>)>,
        in_tmux: bool,
    ) -> (Clipboard, Log) {
        let (runner, log) = recording_runner(failing);
        (Clipboard::with_parts(runner, candidates, in_tmux), log)
    }

    fn payload_of(sequence: &str) -> String {
        sequence
            .trim_start_matches("\u{1b}]52;c;")
            .trim_end_matches('\u{7}')
            .to_string()
    }

    // ─── Test helpers ──────────────────────────────────────────────────────

    #[test]
    #[should_panic(expected = "clipboard runner must not be called for pbcopy")]
    fn forbidden_runner_aborts_when_a_branch_reaches_it() {
        let _ = no_clipboard_allowed("pbcopy", &[], "x");
    }

    #[test]
    fn failure_reason_reports_the_reason_and_describes_other_outcomes() {
        assert_eq!(
            failure_reason(CopyOutcome::Failed("boom".to_string())),
            "boom"
        );
        assert_eq!(failure_reason(CopyOutcome::Copied), "not a failure: Copied");
        assert_eq!(
            failure_reason(CopyOutcome::Requested),
            "not a failure: Requested"
        );
    }

    // ─── OSC 52 encoding ───────────────────────────────────────────────────

    #[test]
    fn encode_osc52_wraps_the_base64_payload() {
        assert_eq!(encode_osc52("hello").unwrap(), "\u{1b}]52;c;aGVsbG8=\u{7}");
    }

    #[test]
    fn encode_osc52_round_trips_multibyte_text() {
        let text = "# naïve 🚀\n\n```rust\nfn main() {}\n```\n";
        let sequence = encode_osc52(text).expect("sequence");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(payload_of(&sequence))
            .expect("valid base64");
        assert_eq!(decoded, text.as_bytes());
    }

    #[test]
    fn encode_osc52_empty_text_has_an_empty_payload() {
        // `copy()` rejects empty text before encoding; the encoder itself stays
        // total so callers can reason about it without a special case.
        assert_eq!(encode_osc52(""), Some("\u{1b}]52;c;\u{7}".to_string()));
    }

    #[test]
    fn encode_osc52_accepts_a_payload_exactly_at_the_limit() {
        let text = "x".repeat(OSC52_MAX_RAW_BYTES);
        assert!(encode_osc52(&text).is_some());
    }

    #[test]
    fn encode_osc52_rejects_a_payload_one_byte_over_the_limit() {
        let text = "x".repeat(OSC52_MAX_RAW_BYTES + 1);
        assert_eq!(encode_osc52(&text), None);
    }

    #[test]
    fn encode_osc52_counts_bytes_not_characters() {
        // 4-byte emoji: 25_000 of them are exactly at the byte limit.
        let text = "🚀".repeat(25_000);
        assert_eq!(text.len(), OSC52_MAX_RAW_BYTES);
        assert!(encode_osc52(&text).is_some());
        assert_eq!(encode_osc52(&"🚀".repeat(25_001)), None);
    }

    // ─── tmux passthrough ──────────────────────────────────────────────────

    #[test]
    fn tmux_passthrough_wraps_and_escapes_the_sequence() {
        assert_eq!(
            tmux_passthrough("\u{1b}]52;c;aGVsbG8=\u{7}"),
            "\u{1b}Ptmux;\u{1b}\u{1b}]52;c;aGVsbG8=\u{7}\u{1b}\\"
        );
    }

    #[test]
    fn tmux_passthrough_doubles_every_escape() {
        assert_eq!(
            tmux_passthrough("\u{1b}a\u{1b}b"),
            "\u{1b}Ptmux;\u{1b}\u{1b}a\u{1b}\u{1b}b\u{1b}\\"
        );
    }

    #[test]
    fn tmux_passthrough_of_plain_text_only_adds_the_wrapper() {
        assert_eq!(tmux_passthrough("plain"), "\u{1b}Ptmux;plain\u{1b}\\");
        assert_eq!(tmux_passthrough(""), "\u{1b}Ptmux;\u{1b}\\");
    }

    // ─── Native program selection ──────────────────────────────────────────

    #[test]
    fn native_command_for_macos_is_pbcopy() {
        for os in ["macos", "macOS", "Darwin", "mac", "ios"] {
            assert_eq!(
                native_command_for(os),
                Some(("pbcopy".to_string(), Vec::new())),
                "os={os}"
            );
        }
    }

    #[test]
    fn native_command_for_windows_is_clip_exe() {
        for os in ["windows", "Windows", "win32", "windows_nt"] {
            assert_eq!(
                native_command_for(os),
                Some(("clip.exe".to_string(), Vec::new())),
                "os={os}"
            );
        }
    }

    #[test]
    fn native_command_for_linux_prefers_wl_copy() {
        assert_eq!(
            native_command_for("linux"),
            Some(("wl-copy".to_string(), Vec::new()))
        );
        assert_eq!(native_command_for(" linux "), native_command_for("linux"));
    }

    #[test]
    fn native_command_for_unknown_os_is_none() {
        for os in ["", "plan9", "freebsd", "wasm32"] {
            assert_eq!(native_command_for(os), None, "os={os}");
        }
    }

    #[test]
    fn native_candidates_order_follows_the_session_type() {
        let names = |os: &str, wayland: bool, x11: bool| {
            native_candidates_for(os, wayland, x11)
                .into_iter()
                .map(|(program, _)| program)
                .collect::<Vec<_>>()
        };

        assert_eq!(names("linux", true, true), vec!["wl-copy", "xclip", "xsel"]);
        assert_eq!(names("linux", true, false), vec!["wl-copy"]);
        assert_eq!(names("linux", false, true), vec!["xclip", "xsel"]);
        // No display server at all: nothing to write to, so nothing to try.
        assert_eq!(names("linux", false, false), Vec::<String>::new());
        // macOS/Windows ignore the display-server flags.
        assert_eq!(names("macos", false, false), vec!["pbcopy"]);
        assert_eq!(names("windows", false, false), vec!["clip.exe"]);
        assert_eq!(names("plan9", true, true), Vec::<String>::new());
    }

    #[test]
    fn native_candidates_carry_their_arguments() {
        let candidates = native_candidates_for("linux", false, true);
        assert_eq!(
            candidates[0],
            (
                "xclip".to_string(),
                vec!["-selection".to_string(), "clipboard".to_string()]
            )
        );
        assert_eq!(
            candidates[1],
            (
                "xsel".to_string(),
                vec!["--clipboard".to_string(), "--input".to_string()]
            )
        );
    }

    // ─── Target selection ──────────────────────────────────────────────────

    #[test]
    fn detect_target_covers_every_combination() {
        assert_eq!(Clipboard::detect_target(true, true), CopyTarget::Tmux);
        assert_eq!(Clipboard::detect_target(true, false), CopyTarget::Tmux);
        assert_eq!(Clipboard::detect_target(false, true), CopyTarget::Native);
        assert_eq!(Clipboard::detect_target(false, false), CopyTarget::None);
    }

    #[test]
    fn resolve_target_ladder_is_tmux_then_native_then_osc52_then_none() {
        let (with_native, _) = clipboard_with(&[], pbcopy(), false);
        let (without_native, _) = clipboard_with(&[], Vec::new(), false);

        assert_eq!(with_native.resolve_target(true, true), CopyTarget::Tmux);
        assert_eq!(with_native.resolve_target(true, false), CopyTarget::Tmux);
        assert_eq!(with_native.resolve_target(false, true), CopyTarget::Native);
        assert_eq!(with_native.resolve_target(false, false), CopyTarget::Native);

        assert_eq!(without_native.resolve_target(true, true), CopyTarget::Tmux);
        assert_eq!(
            without_native.resolve_target(false, true),
            CopyTarget::Osc52
        );
        assert_eq!(
            without_native.resolve_target(false, false),
            CopyTarget::None
        );
    }

    // ─── copy() ────────────────────────────────────────────────────────────

    #[test]
    fn copy_empty_text_fails_without_touching_the_runner() {
        let clipboard = Clipboard::with_parts(forbidden_runner(), pbcopy(), false);
        assert_eq!(
            clipboard.copy("", false, true),
            CopyOutcome::Failed("empty".to_string())
        );
    }

    #[test]
    fn copy_whitespace_only_text_fails_without_touching_the_runner() {
        let clipboard = Clipboard::with_parts(forbidden_runner(), pbcopy(), false);
        for text in [" ", "\n", "\t \r\n", "\u{2003}"] {
            assert_eq!(
                clipboard.copy(text, false, true),
                CopyOutcome::Failed("empty".to_string()),
                "text={text:?}"
            );
        }
    }

    #[test]
    fn copy_oversize_text_fails_without_touching_the_runner() {
        let clipboard = Clipboard::with_parts(forbidden_runner(), pbcopy(), false);
        let text = "x".repeat(OSC52_MAX_RAW_BYTES + 1);
        assert_eq!(
            clipboard.copy(&text, false, true),
            CopyOutcome::Failed(format!(
                "payload too large ({} bytes; max {OSC52_MAX_RAW_BYTES})",
                OSC52_MAX_RAW_BYTES + 1
            ))
        );
    }

    #[test]
    fn copy_at_the_size_limit_still_reaches_the_runner() {
        let (clipboard, log) = clipboard_with(&[], pbcopy(), false);
        let text = "x".repeat(OSC52_MAX_RAW_BYTES);
        assert_eq!(clipboard.copy(&text, false, true), CopyOutcome::Copied);
        assert_eq!(recorded(&log).len(), 1);
        assert_eq!(recorded(&log)[0].2.len(), OSC52_MAX_RAW_BYTES);
    }

    #[test]
    fn copy_uses_native_and_confirms() {
        let (clipboard, log) = clipboard_with(&[], pbcopy(), false);
        assert_eq!(clipboard.copy("hello", false, true), CopyOutcome::Copied);
        assert_eq!(
            recorded(&log),
            vec![("pbcopy".to_string(), Vec::new(), "hello".to_string())]
        );
    }

    #[test]
    fn copy_passes_text_verbatim() {
        let (clipboard, log) = clipboard_with(&[], pbcopy(), false);
        let text = "line one\nline\ttwo\n\u{1b}[31mred\u{1b}[0m 🚀\n";
        assert_eq!(clipboard.copy(text, false, true), CopyOutcome::Copied);
        assert_eq!(recorded(&log)[0].2, text);
    }

    #[test]
    fn copy_falls_back_to_the_next_native_candidate() {
        let candidates = native_candidates_for("linux", true, true);
        let (clipboard, log) = clipboard_with(&["wl-copy"], candidates, false);
        assert_eq!(clipboard.copy("hello", false, true), CopyOutcome::Copied);
        let calls = recorded(&log);
        assert_eq!(
            calls
                .iter()
                .map(|(program, _, _)| program.as_str())
                .collect::<Vec<_>>(),
            vec!["wl-copy", "xclip"]
        );
        assert_eq!(calls[1].1, vec!["-selection", "clipboard"]);
    }

    #[test]
    fn copy_does_not_try_later_candidates_after_a_success() {
        let candidates = native_candidates_for("linux", true, true);
        let (clipboard, log) = clipboard_with(&[], candidates, false);
        assert_eq!(clipboard.copy("hello", false, true), CopyOutcome::Copied);
        assert_eq!(recorded(&log).len(), 1);
    }

    #[test]
    fn copy_requests_osc52_when_there_is_no_native_backend() {
        let (clipboard, log) = clipboard_with(&[], Vec::new(), false);
        assert_eq!(clipboard.copy("hello", false, true), CopyOutcome::Requested);
        assert!(recorded(&log).is_empty());
        assert_eq!(
            clipboard.osc52_sequence("hello").unwrap(),
            "\u{1b}]52;c;aGVsbG8=\u{7}"
        );
    }

    #[test]
    fn copy_requests_osc52_after_a_native_failure() {
        let (clipboard, log) = clipboard_with(&["pbcopy"], pbcopy(), false);
        assert_eq!(clipboard.copy("hello", false, true), CopyOutcome::Requested);
        assert_eq!(recorded(&log).len(), 1);
    }

    #[test]
    fn copy_fails_when_native_fails_and_there_is_no_terminal() {
        let (clipboard, _) = clipboard_with(&["pbcopy"], pbcopy(), false);
        let reason = failure_reason(clipboard.copy("hello", false, false));
        assert!(reason.contains("no clipboard backend"), "{reason}");
        assert!(reason.contains("pbcopy is not reachable"), "{reason}");
        assert!(reason.contains("not a terminal"), "{reason}");
    }

    #[test]
    fn copy_fails_without_native_backend_and_without_a_terminal() {
        let (clipboard, log) = clipboard_with(&[], Vec::new(), false);
        let reason = failure_reason(clipboard.copy("hello", false, false));
        assert!(reason.contains("no native clipboard program"), "{reason}");
        assert!(reason.contains("not a terminal"), "{reason}");
        assert!(recorded(&log).is_empty());
    }

    #[test]
    fn copy_in_tmux_still_prefers_native() {
        let (clipboard, log) = clipboard_with(&[], pbcopy(), true);
        assert_eq!(clipboard.copy("hello", true, true), CopyOutcome::Copied);
        assert_eq!(recorded(&log).len(), 1);
    }

    #[test]
    fn copy_in_tmux_requests_a_wrapped_passthrough() {
        let (clipboard, log) = clipboard_with(&[], Vec::new(), true);
        assert_eq!(clipboard.copy("hello", true, true), CopyOutcome::Requested);
        assert!(recorded(&log).is_empty());
        assert_eq!(
            clipboard.osc52_sequence("hello").unwrap(),
            "\u{1b}Ptmux;\u{1b}\u{1b}]52;c;aGVsbG8=\u{7}\u{1b}\\"
        );
    }

    #[test]
    fn copy_in_tmux_falls_back_to_passthrough_when_native_fails() {
        let (clipboard, log) = clipboard_with(&["pbcopy"], pbcopy(), true);
        assert_eq!(clipboard.copy("hello", true, false), CopyOutcome::Requested);
        assert_eq!(recorded(&log).len(), 1);
        assert!(clipboard
            .osc52_sequence("hello")
            .unwrap()
            .starts_with("\u{1b}Ptmux;"));
    }

    #[test]
    fn osc52_sequence_is_unwrapped_without_tmux() {
        let clipboard = Clipboard::with_parts(forbidden_runner(), Vec::new(), false);
        assert_eq!(
            clipboard.osc52_sequence("hi").unwrap(),
            "\u{1b}]52;c;aGk=\u{7}"
        );
    }

    #[test]
    fn osc52_sequence_tracks_the_most_recent_copy_session() {
        // `resolve_target`/`osc52_sequence` must agree with the outcome the
        // caller just received, whichever session shape `copy` was told about.
        let clipboard = Clipboard::with_parts(forbidden_runner(), Vec::new(), false);
        assert_eq!(clipboard.copy("hi", false, true), CopyOutcome::Requested);
        assert!(!clipboard
            .osc52_sequence("hi")
            .unwrap()
            .starts_with("\u{1b}Ptmux;"));
        assert_eq!(clipboard.copy("hi", true, true), CopyOutcome::Requested);
        assert!(clipboard
            .osc52_sequence("hi")
            .unwrap()
            .starts_with("\u{1b}Ptmux;"));
        assert_eq!(clipboard.copy("hi", false, true), CopyOutcome::Requested);
        assert!(!clipboard
            .osc52_sequence("hi")
            .unwrap()
            .starts_with("\u{1b}Ptmux;"));
    }

    #[test]
    fn osc52_sequence_is_none_above_the_limit() {
        let clipboard = Clipboard::with_parts(forbidden_runner(), Vec::new(), false);
        let text = "x".repeat(OSC52_MAX_RAW_BYTES + 1);
        assert_eq!(clipboard.osc52_sequence(&text), None);
    }

    #[test]
    fn candidate_list_is_exposed_for_the_ui() {
        let clipboard = Clipboard::with_parts(forbidden_runner(), pbcopy(), false);
        assert_eq!(clipboard.candidates(), pbcopy().as_slice());
        assert!(!clipboard.is_ssh());
    }

    // ─── Environment probes ────────────────────────────────────────────────

    #[test]
    fn is_tmux_from_env_cases() {
        assert!(is_tmux_from_env(Some(
            "/private/tmp/tmux-501/default,1234,0"
        )));
        assert!(is_tmux_from_env(Some("%0")));
        assert!(!is_tmux_from_env(Some("")));
        assert!(!is_tmux_from_env(Some("   ")));
        assert!(!is_tmux_from_env(None));
    }

    #[test]
    fn tmux_from_vars_accepts_either_variable() {
        assert!(tmux_from_vars(Some("%0"), None));
        assert!(tmux_from_vars(
            None,
            Some("/private/tmp/tmux-501/default,1,0")
        ));
        // A blank `TMUX` is not a hit, but it does not mask a set `TMUX_PANE`.
        assert!(tmux_from_vars(Some(""), Some("%1")));
        assert!(tmux_from_vars(Some("  "), Some("%1")));
        assert!(!tmux_from_vars(Some("  "), Some("\t")));
        assert!(!tmux_from_vars(None, None));
    }

    #[test]
    fn is_ssh_from_env_cases() {
        assert!(is_ssh_from_env(Some("10.0.0.1 2222 10.0.0.2 22"), None));
        assert!(is_ssh_from_env(None, Some("/dev/ttys003")));
        assert!(is_ssh_from_env(Some("conn"), Some("/dev/ttys003")));
        assert!(!is_ssh_from_env(Some(""), Some("  ")));
        assert!(!is_ssh_from_env(None, None));
    }

    #[test]
    fn new_probes_the_current_environment() {
        let clipboard = Clipboard::new();
        let (_, wayland, x11) = session_environment();
        assert_eq!(
            clipboard.candidates(),
            native_candidates_for(std::env::consts::OS, wayland, x11).as_slice()
        );
        assert_eq!(
            clipboard.is_ssh(),
            is_ssh_from_env(
                std::env::var("SSH_CONNECTION")
                    .ok()
                    .as_deref()
                    .filter(|v| !v.is_empty()),
                std::env::var("SSH_TTY")
                    .ok()
                    .as_deref()
                    .filter(|v| !v.is_empty()),
            )
        );
        // `new()` must not panic on an empty/plain environment either.
        let defaults = Clipboard::default();
        assert_eq!(defaults.candidates(), clipboard.candidates());
    }

    // ─── Default runner ────────────────────────────────────────────────────

    #[test]
    fn native_runner_reports_a_spawn_failure() {
        // A nonexistent program can never read the clipboard, so this only
        // exercises the error path of the real runner.
        let error = native_runner("future-tui-no-such-clipboard-program", &[], "hi")
            .expect_err("spawn must fail");
        assert!(error.contains("failed to spawn"), "{error}");
    }

    /// The happy path with a real (harmless) program: `cat` drains stdin and
    /// exits 0, which is exactly how `pbcopy`/`wl-copy` report success.
    #[test]
    #[cfg(unix)]
    fn native_runner_round_trips_through_a_real_program() {
        native_runner("cat", &[], "hello\nworld").expect("cat must accept the payload");
    }

    /// A real non-zero exit with an empty stderr must be reported as a status
    /// failure, never as a silent success.
    #[test]
    #[cfg(unix)]
    fn native_runner_reports_a_nonzero_exit_without_stderr() {
        let error = native_runner("false", &[], "hi").expect_err("false must fail");
        assert!(error.starts_with("exited with status"), "{error}");
        assert!(!error.starts_with("failed: "), "{error}");
    }

    /// A real non-zero exit that wrote to stderr must surface that text.
    #[test]
    #[cfg(unix)]
    fn native_runner_reports_stderr_from_a_failing_program() {
        let args = vec!["-c".to_string(), "printf boom >&2; exit 1".to_string()];
        let error = native_runner("sh", &args, "hi").expect_err("the command must fail");
        assert_eq!(error, "failed: boom");
    }

    /// A child without a piped stdin is reported, not panicked on.
    #[test]
    #[cfg(unix)]
    fn run_native_reports_a_missing_stdin_pipe() {
        let child = Command::new("cat")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn cat");
        let error = run_native(child, "hi").expect_err("there is no stdin pipe");
        assert_eq!(error, "failed to open stdin");
    }

    /// `true` exits without reading stdin, so a payload larger than the pipe
    /// buffer cannot be delivered: the write fails instead of hanging.
    #[test]
    #[cfg(unix)]
    fn run_native_reports_a_broken_stdin_write() {
        let child = Command::new("true")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn true");
        let huge = "x".repeat(1_000_000);
        let error = run_native(child, &huge).expect_err("the pipe must break");
        assert!(error.starts_with("failed to write"), "{error}");
    }
}
