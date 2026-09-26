//! Cross-platform child-command fixtures for the terminal tests.
//!
//! The terminal suite used to hard-code `/bin/sh`, which is why every
//! session/manager/pty test carried `#[cfg(unix)]`: on Windows the whole
//! state machine below the PTY boundary was untested. These helpers let one
//! test body run on both platforms — `cmd.exe` on Windows, `/bin/sh`
//! elsewhere — with scripts that mean the same thing on each, so the same
//! assertions hold and a Windows-only regression cannot hide behind a `cfg`.
//!
//! The commands are deliberately *silent* where a test asserts on exact
//! bytes: the session tests feed output with `Session::on_data` (the buffer
//! arithmetic is theirs to control), and a chatty child would land in the
//! replay. `echo_command` exists for the one test that must prove the reader
//! thread really drains a live PTY.

use std::path::PathBuf;

/// Absolute `cmd.exe` on Windows (`portable-pty` hands the program straight to
/// `CreateProcessW`, which does not search `PATH`).
#[cfg(windows)]
fn windows_cmd() -> PathBuf {
    if let Some(comspec) = std::env::var_os("COMSPEC") {
        return PathBuf::from(comspec);
    }
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    PathBuf::from(root).join("System32").join("cmd.exe")
}

/// The shell used to drive a child on this platform.
pub fn shell_program() -> PathBuf {
    #[cfg(windows)]
    {
        windows_cmd()
    }
    #[cfg(unix)]
    {
        PathBuf::from("/bin/sh")
    }
}

/// Wrap `script` for the platform shell's `-c`/`/C` flag.
pub fn script_command(script: &str) -> (PathBuf, Vec<String>) {
    #[cfg(windows)]
    {
        (shell_program(), vec!["/C".to_string(), script.to_string()])
    }
    #[cfg(unix)]
    {
        (shell_program(), vec!["-c".to_string(), script.to_string()])
    }
}

/// A child that stays alive and writes nothing until it is killed.
///
/// `ping` is used on Windows because `cmd.exe` has no `sleep`, and its output
/// is redirected to `NUL` so the session buffer holds only what the test fed
/// it. The unix equivalent is a plain `sleep`.
pub fn idle_command() -> (PathBuf, Vec<String>) {
    #[cfg(windows)]
    {
        script_command("ping -n 60 127.0.0.1 >NUL")
    }
    #[cfg(unix)]
    {
        script_command("sleep 60")
    }
}

/// A child that prints `text` to stdout and exits: the PTY reader thread must
/// surface these bytes.
///
/// `text` must be free of shell metacharacters (every caller uses a plain
/// marker). The two platforms differ in the line ending they add, which is why
/// callers assert a prefix rather than equality.
pub fn echo_command(text: &str) -> (PathBuf, Vec<String>) {
    #[cfg(windows)]
    {
        script_command(&format!("echo {text}"))
    }
    #[cfg(unix)]
    {
        script_command(&format!("printf '{text}'"))
    }
}

/// A child that exits immediately with `code`. Both shells report the code as
/// the process exit status (`exit N` / `-c 'exit N'`), which is what
/// `PtySession::try_wait` must surface through ConPTY.
pub fn exit_command(code: i32) -> (PathBuf, Vec<String>) {
    script_command(&format!("exit {code}"))
}

/// An interactive shell (no `-c`): the tests that must write to the child's
/// stdin drive it here, and a line the shell executes proves the bytes reached
/// it rather than merely being echoed.
pub fn interactive_command() -> (PathBuf, Vec<String>) {
    (shell_program(), Vec::new())
}

/// The `cwd` these fixtures run in — a directory that always exists.
pub fn working_dir() -> PathBuf {
    std::env::temp_dir()
}

/// Device Status Report query (`ESC [ 6 n`, "where is the cursor?").
///
/// A child spawned on a ConPTY on this host emits this query as it starts and
/// then **waits for the answer before it runs its command line**: a test that
/// launches a child and does not reply observes exactly `ESC[6n` on the master
/// and nothing else until its deadline (measured: `pty.rs`'s
/// `output_is_readable_through_the_master` saw `"\u{1b}[6n"` for 20 s, and
/// `cmd /C exit 7` stayed `STILL_ACTIVE` for 20 s). `terminal/server.rs`'s
/// end-to-end test already answers the same query for the same reason; these
/// fixtures exist so the unit tests do not each have to rediscover it.
pub const DSR_QUERY: &[u8] = b"\x1b[6n";

/// The cursor-position reply (`ESC [ 1 ; 1 R`, row 1 column 1).
pub const DSR_REPLY: &[u8] = b"\x1b[1;1R";

/// True when `bytes` contains the cursor-position query.
pub fn asks_for_the_cursor(bytes: &[u8]) -> bool {
    !bytes.is_empty()
        && bytes
            .windows(DSR_QUERY.len())
            .any(|window| window == DSR_QUERY)
}

/// `bytes` as text with ANSI control sequences removed.
///
/// A terminal's byte stream is not plain text: the child may prepend or append
/// cursor queries, and a resize makes it redraw. Assertions about *content*
/// therefore compare the visible text, while assertions about the byte
/// arithmetic compare bytes. Handles the two shapes a terminal actually emits —
/// CSI (`ESC [ … final`) and OSC (`ESC ] … BEL`/`ESC \`).
pub fn visible_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            // CSI: parameters/intermediates, then one final byte in 0x40..=0x7e.
            Some('[') => {
                for next in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&next) {
                        break;
                    }
                }
            }
            // OSC: terminated by BEL or ST.
            Some(']') => {
                let mut previous = '\0';
                for next in chars.by_ref() {
                    if next == '\u{7}' || (previous == '\u{1b}' && next == '\\') {
                        break;
                    }
                    previous = next;
                }
            }
            // A lone ESC or a two-byte escape: drop the pair.
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `visible_text` is what every content assertion in this suite compares
    /// against, so its own state machine is worth pinning: an escape sequence
    /// it fails to strip makes a byte-exact assertion pass or fail for the
    /// wrong reason.
    ///
    /// The four shapes are the ones a real terminal emits: CSI (a cursor
    /// query), OSC (a window title), a lone `ESC` at the end of the stream,
    /// and a two-byte escape. The last two are the arm that a stream ending
    /// mid-sequence hits — `ESC` with nothing (or exactly one byte) after it
    /// must be dropped, not echoed, and must not panic.
    #[test]
    fn visible_text_strips_escapes_and_keeps_the_text() {
        // CSI, the shape a ConPTY child opens with.
        assert_eq!(visible_text(b"\x1b[6nready"), "ready");
        assert_eq!(visible_text(b"\x1b[1;1R"), "");
        // OSC with a BEL terminator and with the ST terminator.
        assert_eq!(visible_text(b"\x1b]0;title\x07body"), "body");
        assert_eq!(visible_text(b"\x1b]0;title\x1b\\body"), "body");
        // A lone ESC, and a two-byte escape: dropped as a pair, never echoed.
        assert_eq!(visible_text(b"text\x1b"), "text");
        assert_eq!(visible_text(b"text\x1b7more"), "textmore");
        assert_eq!(visible_text(b"\x1b"), "");
        // The boundary cases the byte-exact tests rely on.
        assert_eq!(visible_text(b""), "");
        assert_eq!(visible_text(b"plain text"), "plain text");
        // Invalid UTF-8 is replaced rather than panicking: the buffer holds raw
        // PTY bytes, not text.
        assert_eq!(visible_text(b"a\xffb"), "a\u{fffd}b");
        // CJK survives; only the escapes are removed.
        assert_eq!(visible_text("\u{1b}[1m终端\u{1b}[0m".as_bytes()), "终端");
    }

    /// The DSR query detector is what stops a test from waiting forever on a
    /// child that is blocked asking for the cursor. It must find the query
    /// anywhere in a chunk that arrived in one read, and must not fire on a
    /// stream that merely starts with `CSI`.
    #[test]
    fn asks_for_the_cursor_recognises_only_the_full_query() {
        assert!(asks_for_the_cursor(DSR_QUERY));
        assert!(asks_for_the_cursor(b"\x1b[6nrest of the banner"));
        assert!(asks_for_the_cursor(b"prefix\x1b[6n"));
        assert!(
            !asks_for_the_cursor(b"\x1b[1;1R"),
            "the client's own reply must not be mistaken for the query: a test \
             that answered itself forever would hang the suite"
        );
        // A query split across two reads is not detectable in one buffer, which
        // is the documented limit of this helper rather than a bug: the tests
        // answer as soon as the bytes are complete.
        assert!(!asks_for_the_cursor(b"\x1b[6"));
        assert!(!asks_for_the_cursor(b"plain output"));
        assert!(!asks_for_the_cursor(b""));
    }
}
