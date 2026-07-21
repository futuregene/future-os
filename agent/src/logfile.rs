//! Log-file mirroring: keep the log file byte-for-byte consistent with what
//! the operator sees on the terminal.
//!
//! Two output paths exist in the agent:
//!
//! 1. **tracing events** (`tracing::info!` etc.) — each event is formatted
//!    once per `fmt` layer, so the console layer renders ANSI colors while the
//!    file layer (via [`LogMirror`] as its `MakeWriter`) renders plain text.
//!    Same event, same content, both sinks.
//!
//! 2. **raw streaming prints** (`eprint!` for `[thinking]` / `[output]`
//!    deltas in the agent run loop) — these bypass tracing entirely. The
//!    [`eprint_log!`] / [`eprintln_log!`] macros print to stderr verbatim AND
//!    append the same bytes (ANSI-stripped) to the mirrored file.
//!
//! Both paths serialize on a single `Arc<Mutex<...>>`, so a tracing event
//! and a raw print can never tear each other mid-line, and the file ends up
//! being exactly "console output minus ANSI colors".
//!
//! Retention: to keep the file from growing without bound, only the newest
//! `max_lines` lines are kept. Newlines are counted incrementally as bytes
//! are written (no extra I/O); when the count exceeds `max_lines` plus a
//! slack margin, the file is atomically rewritten (temp file + rename) with
//! just its tail. A check also runs at open so a file that grew huge in a
//! previous run is trimmed on startup. `max_lines = 0` disables trimming.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

/// Default retention when `--log-file` is on and `--log-max-lines` is unset.
pub const DEFAULT_MAX_LINES: usize = 1000;

/// Trimming triggers at `max_lines + TRIM_SLACK` and rewrites down to
/// `max_lines`, so steady-state logging trims once per SLACK lines instead
/// of on every line past the limit.
const TRIM_SLACK: usize = 200;

/// Cloneable handle to the shared log file. Implements `MakeWriter` so a
/// tracing `fmt` layer can write through the same mutex as raw prints.
#[derive(Clone)]
pub struct LogMirror {
    inner: Arc<Mutex<LogFile>>,
}

struct LogFile {
    file: File,
    path: PathBuf,
    /// Lines currently in the file, tracked incrementally by counting
    /// newlines in everything written (seeded from the existing file at
    /// open). Only maintained when `max_lines > 0`.
    lines: usize,
    /// Retention limit; 0 = unlimited.
    max_lines: usize,
}

static MIRROR: OnceLock<Option<LogMirror>> = OnceLock::new();

/// Open `path` for appending (creating parent directories) and install it as
/// the global mirror. Returns a clone for use as the tracing file layer's
/// writer. Calling this more than once keeps the first installation.
///
/// If `max_lines > 0`, an over-long existing file is trimmed to its last
/// `max_lines` lines right away, and the same retention is enforced as new
/// content is written.
pub fn init(path: &Path, max_lines: usize) -> io::Result<LogMirror> {
    let mirror = LogMirror {
        inner: Arc::new(Mutex::new(LogFile::open(path, max_lines)?)),
    };
    // Ignore the AlreadySet case: the first installation wins.
    let _ = MIRROR.set(Some(mirror.clone()));
    Ok(mirror)
}

/// The globally installed mirror, if `init` was called.
pub fn get() -> Option<&'static LogMirror> {
    MIRROR.get().and_then(|m| m.as_ref())
}

impl LogMirror {
    /// Append raw bytes to the mirrored file with ANSI escape sequences
    /// stripped. Best-effort: errors (lock poison, I/O) are swallowed so a
    /// broken log file never kills the agent.
    pub fn write_stripped(&self, text: &str) {
        let stripped = strip_ansi(text);
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let _ = guard.write(stripped.as_bytes());
    }
}

impl LogFile {
    fn open(path: &Path, max_lines: usize) -> io::Result<LogFile> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        // Seed the line counter from existing content so retention accounts
        // for what previous runs left behind.
        let lines = if max_lines > 0 {
            count_lines(&std::fs::read(path).unwrap_or_default())
        } else {
            0
        };
        let mut log = LogFile {
            file,
            path: path.to_path_buf(),
            lines,
            max_lines,
        };
        if max_lines > 0 && lines > max_lines {
            log.trim()?;
        }
        Ok(log)
    }

    fn write(&mut self, buf: &[u8]) -> io::Result<()> {
        self.file.write_all(buf)?;
        if self.max_lines > 0 {
            self.lines += count_lines(buf);
            if self.lines > self.max_lines + TRIM_SLACK {
                self.trim()?;
            }
        }
        Ok(())
    }

    /// Rewrite the file keeping only its last `max_lines` lines. The rewrite
    /// goes through a temp file + rename so a crash mid-trim cannot leave a
    /// half-written log.
    fn trim(&mut self) -> io::Result<()> {
        let data = std::fs::read(&self.path)?;
        let total = count_lines(&data);
        if total <= self.max_lines {
            self.lines = total;
            return Ok(());
        }
        // Byte offset just past the (total - max_lines)-th newline: the start
        // of the tail we keep.
        let mut skip = total - self.max_lines;
        let mut offset = 0;
        for (i, &b) in data.iter().enumerate() {
            if b == b'\n' {
                skip -= 1;
                if skip == 0 {
                    offset = i + 1;
                    break;
                }
            }
        }
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, &data[offset..])?;
        std::fs::rename(&tmp, &self.path)?;
        // The rename swapped the inode — reopen so future appends land in the
        // new file instead of the unlinked old one.
        self.file = OpenOptions::new().append(true).open(&self.path)?;
        self.lines = self.max_lines;
        Ok(())
    }
}

fn count_lines(buf: &[u8]) -> usize {
    buf.iter().filter(|&&b| b == b'\n').count()
}

/// `MakeWriter` implementation for the tracing file layer. The returned
/// writer holds the mutex guard for its whole lifetime, so one formatted
/// event is written contiguously even if raw prints race it on another
/// thread.
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogMirror {
    type Writer = MirrorWriter<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        MirrorWriter(self.inner.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

pub struct MirrorWriter<'a>(MutexGuard<'a, LogFile>);

impl Write for MirrorWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.file.flush()
    }
}

/// Strip ANSI CSI sequences (e.g. `\x1b[35m`) from `text`. Fast path returns
/// the input untouched when no ESC byte is present (true for LLM deltas).
pub fn strip_ansi(text: &str) -> String {
    if !text.contains('\x1b') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next(); // consume '['
            for c2 in chars.by_ref() {
                // Skip parameter/intermediate bytes until the final byte (@-~).
                if ('@'..='~').contains(&c2) {
                    break;
                }
            }
        } else if c != '\x1b' {
            out.push(c);
        }
    }
    out
}

/// `eprint!` equivalent that also mirrors into the log file (ANSI-stripped).
pub fn eprint_mirror(args: std::fmt::Arguments) {
    eprint!("{args}");
    if let Some(mirror) = get() {
        mirror.write_stripped(&args.to_string());
    }
}

/// `eprintln!` equivalent that also mirrors into the log file (ANSI-stripped).
pub fn eprintln_mirror(args: std::fmt::Arguments) {
    eprintln!("{args}");
    if let Some(mirror) = get() {
        mirror.write_stripped(&format!("{args}\n"));
    }
}

/// Print to stderr AND the log file (if enabled via `--log-file`).
/// Drop-in replacement for `eprint!`.
#[macro_export]
macro_rules! eprint_log {
    ($($arg:tt)*) => {
        $crate::logfile::eprint_mirror(format_args!($($arg)*))
    };
}

/// Print a line to stderr AND the log file (if enabled via `--log-file`).
/// Drop-in replacement for `eprintln!`.
#[macro_export]
macro_rules! eprintln_log {
    () => {
        $crate::logfile::eprintln_mirror(format_args!(""))
    };
    ($($arg:tt)*) => {
        $crate::logfile::eprintln_mirror(format_args!($($arg)*))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_ansi_removes_sgr_codes() {
        assert_eq!(strip_ansi("\x1b[35m[thinking]\x1b[0m hi"), "[thinking] hi");
        assert_eq!(strip_ansi("plain text"), "plain text");
        assert_eq!(strip_ansi("\x1b[1;31mbold red\x1b[0m"), "bold red");
        assert_eq!(strip_ansi("no esc at all"), "no esc at all");
        // Lone ESC without '[' is dropped, following char kept.
        assert_eq!(strip_ansi("a\x1bXb"), "aXb");
    }

    #[test]
    fn count_lines_counts_newlines() {
        assert_eq!(count_lines(b""), 0);
        assert_eq!(count_lines(b"one\ntwo\nthree\n"), 3);
        assert_eq!(count_lines(b"partial line"), 0);
    }

    fn lines_in(path: &Path) -> Vec<String> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn trims_existing_file_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.log");
        let content: String = (0..10).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&path, content).unwrap();

        let log = LogFile::open(&path, 5).unwrap();
        assert_eq!(log.lines, 5);
        assert_eq!(
            lines_in(&path),
            vec!["line 5", "line 6", "line 7", "line 8", "line 9"]
        );
    }

    #[test]
    fn keeps_short_file_untouched_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.log");
        std::fs::write(&path, "a\nb\n").unwrap();

        let log = LogFile::open(&path, 1000).unwrap();
        assert_eq!(log.lines, 2);
        assert_eq!(lines_in(&path), vec!["a", "b"]);
    }

    #[test]
    fn trims_incrementally_as_lines_are_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.log");
        let max = 100;
        let mut log = LogFile::open(&path, max).unwrap();

        // Trim fires once, when the (max + SLACK + 1)-th line is written, and
        // shrinks the file back to `max` lines; writes after that append on top.
        let total_writes = max + TRIM_SLACK + 50;
        for i in 0..total_writes {
            log.write(format!("event {i}\n").as_bytes()).unwrap();
        }
        let trimmed_at = max + TRIM_SLACK + 1; // lines in file when trim fired
        let lines = lines_in(&path);
        assert_eq!(lines.len(), max + (total_writes - trimmed_at));
        assert_eq!(
            lines.first().unwrap(),
            &format!("event {}", trimmed_at - max)
        );
        assert_eq!(
            lines.last().unwrap(),
            &format!("event {}", total_writes - 1)
        );
    }

    #[test]
    fn max_lines_zero_disables_trimming() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.log");
        let mut log = LogFile::open(&path, 0).unwrap();
        for i in 0..5000 {
            log.write(format!("line {i}\n").as_bytes()).unwrap();
        }
        drop(log);
        assert_eq!(lines_in(&path).len(), 5000);
    }
}
