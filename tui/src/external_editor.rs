//! External editor support for composing multi-line input (`/editor`).
//!
//! Design: the editor is resolved from the environment, run against a temporary
//! draft file, and the terminal lifecycle is left to the caller (see
//! [`App::open_external_editor`], which suspends and resumes the terminal around
//! the child):
//!
//! 1. `$VISUAL`, falling back to `$EDITOR`, is split into argv
//!    (`resolve_editor_command` + `split_command`). Values are *not* run
//!    through a shell, so quoting is ours to implement — the parser below is
//!    platform-aware instead of pulling in `shlex`/`winsplit` (POSIX escape
//!    rules vs. Windows backslash-run rules; see `Dialect`).
//! 2. `EditorDraft::new` writes the seed text into a fresh, uniquely named
//!    file inside a caller-supplied directory, created with `create_new`
//!    (atomic, never truncating an existing file) and, on Unix, mode `0600` —
//!    the buffer is a draft of user input, not world-readable content. The
//!    file handle is closed before returning so Windows can hand the path to
//!    another process.
//! 3. The caller suspends the terminal, spawns `build_command(&argv, path)`,
//!    restores the terminal, then reads the buffer back with `contents()` and
//!    finishes with `cleanup()`. `EditorDraft` also removes the file on drop,
//!    so an aborted spawn cannot leak a draft.
//!
//! Nothing in this module spawns a process or touches the terminal, and no
//! function reads the environment implicitly: the integrator passes
//! `std::env::var("VISUAL")`/`"EDITOR"` in, and the tests inject both the
//! command line and the draft directory, so no test can ever launch a real
//! editor.

use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Failure modes of resolving / materialising an editor command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorError {
    /// `$VISUAL` and `$EDITOR` are both absent.
    MissingEditor,
    /// The selected variable is set but holds nothing usable (empty/blank).
    EmptyCommand,
    /// The command line could not be tokenised (unbalanced quote, dangling
    /// escape).
    ParseFailed(String),
    /// Reading/writing/removing the draft file failed.
    Io(String),
}

impl std::fmt::Display for EditorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingEditor => write!(f, "neither $VISUAL nor $EDITOR is set"),
            Self::EmptyCommand => write!(f, "editor command is empty"),
            Self::ParseFailed(reason) => {
                write!(f, "failed to parse editor command: {reason}")
            }
            Self::Io(reason) => write!(f, "editor draft i/o error: {reason}"),
        }
    }
}

impl std::error::Error for EditorError {}

/// Quoting dialect of a command line. Both are implemented (and tested) on
/// every platform so the Windows behaviour cannot silently rot on macOS CI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dialect {
    /// Shell rules: `\` escapes the next character (outside single quotes),
    /// inside `"` it escapes `"` `\` `` ` `` `$` and newline only.
    Posix,
    /// Windows `CommandLineToArgvW`-ish rules: `\` is an ordinary character
    /// outside quotes; inside `"` a run of N backslashes followed by `"`
    /// yields N/2 backslashes and (for odd N) one literal `"`.
    ///
    /// On a non-Windows host the lib never builds this variant (only the tests
    /// below do — same precedent as `windows_program_candidates`).
    #[cfg_attr(not(windows), allow(dead_code))]
    Windows,
}

/// The dialect `split_command` uses on this host, chosen by `cfg` rather than
/// by a `cfg!(windows)` runtime branch so only the platform's own dialect is
/// compiled in (no unreachable arm on the other platform).
#[cfg(windows)]
const HOST_DIALECT: Dialect = Dialect::Windows;
#[cfg(not(windows))]
const HOST_DIALECT: Dialect = Dialect::Posix;

/// The dialect `split_command` uses on this host.
fn host_dialect() -> Dialect {
    HOST_DIALECT
}

/// Split an editor command line into argv.
///
/// Platform-agnostic by construction (no shell, no `shlex`): supports single
/// quotes (fully literal), double quotes, backslash-escaped spaces, and empty
/// quoted arguments (`""` is one empty argument, not nothing). Blank input is
/// `EmptyCommand`; an unterminated quote or a dangling escape is
/// `ParseFailed`.
pub fn split_command(raw: &str) -> Result<Vec<String>, EditorError> {
    split_command_with(raw, host_dialect())
}

fn split_command_with(raw: &str, dialect: Dialect) -> Result<Vec<String>, EditorError> {
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    // Distinguishes `""` (one empty argument) from "no argument yet".
    let mut started = false;
    let mut chars = raw.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            c if c.is_whitespace() => {
                if started {
                    parts.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            '\'' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => current.push(c),
                        None => {
                            return Err(EditorError::ParseFailed(
                                "unterminated single quote".to_string(),
                            ))
                        }
                    }
                }
            }
            '"' => {
                started = true;
                read_double_quoted(&mut chars, &mut current, dialect)?;
            }
            '\\' if dialect == Dialect::Posix => match chars.next() {
                // An escaped newline is a line continuation: it contributes
                // nothing and must not start an argument of its own. Any other
                // escaped character (including a space) is literal content.
                Some('\n') => {}
                Some(c) => {
                    started = true;
                    current.push(c);
                }
                None => {
                    return Err(EditorError::ParseFailed(
                        "trailing escape character".to_string(),
                    ))
                }
            },
            c => {
                started = true;
                current.push(c);
            }
        }
    }

    if started {
        parts.push(current);
    }
    if parts.is_empty() {
        return Err(EditorError::EmptyCommand);
    }
    Ok(parts)
}

/// Consume a double-quoted section (the opening `"` is already consumed) until
/// the closing quote. Returns `Err` if the string ends inside the quotes.
fn read_double_quoted(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    current: &mut String,
    dialect: Dialect,
) -> Result<(), EditorError> {
    loop {
        match chars.next() {
            Some('"') => return Ok(()),
            Some('\\') => match dialect {
                Dialect::Posix => match chars.next() {
                    // Only these keep shell-escape meaning inside "".
                    Some(c @ ('"' | '\\' | '$' | '`')) => current.push(c),
                    Some('\n') => {}
                    Some(c) => {
                        current.push('\\');
                        current.push(c);
                    }
                    None => {
                        return Err(EditorError::ParseFailed(
                            "unterminated escape character".to_string(),
                        ))
                    }
                },
                Dialect::Windows => {
                    // Count the whole backslash run: 2n + 1 means the quote is
                    // literal content, 2n means the run collapses and the
                    // quote closes the section.
                    let mut backslashes = 1usize;
                    while chars.peek() == Some(&'\\') {
                        chars.next();
                        backslashes += 1;
                    }
                    if chars.peek() == Some(&'"') {
                        for _ in 0..backslashes / 2 {
                            current.push('\\');
                        }
                        if backslashes % 2 == 1 {
                            // Odd run: the quote is escaped content.
                            current.push('"');
                            chars.next();
                        } else {
                            // Even run: the quote terminates the section. It
                            // must be consumed here — leaving it for the outer
                            // loop would start a second (unterminated) section.
                            chars.next();
                            return Ok(());
                        }
                    } else {
                        for _ in 0..backslashes {
                            current.push('\\');
                        }
                    }
                }
            },
            Some(c) => current.push(c),
            None => {
                return Err(EditorError::ParseFailed(
                    "unterminated double quote".to_string(),
                ))
            }
        }
    }
}

/// Resolve the editor argv from the `$VISUAL` / `$EDITOR` values.
///
/// `$VISUAL` wins whenever it is present, even when it is blank (a blank
/// `$VISUAL` does not fall through to `$EDITOR`; the variable was set
/// deliberately and an empty value is the user's own instruction, so it yields
/// `EmptyCommand` rather than silently running something else).
pub fn resolve_editor_command(
    visual: Option<&str>,
    editor: Option<&str>,
) -> Result<Vec<String>, EditorError> {
    let raw = match (visual, editor) {
        (Some(visual), _) => visual,
        (None, Some(editor)) => editor,
        (None, None) => return Err(EditorError::MissingEditor),
    };
    split_command(raw)
}

/// An on-disk draft buffer for the external editor.
///
/// Holds only the path (no open handle), so the caller can launch another
/// process against it — including on Windows.
#[derive(Debug)]
pub struct EditorDraft {
    path: PathBuf,
}

/// Distinct file names tried before giving up on creating a draft.
const DRAFT_ATTEMPTS: u32 = 8;

/// Monotonic per-process counter; combined with the pid and a timestamp it
/// makes the draft name unique without depending on randomness or `tempfile`.
static DRAFT_COUNTER: AtomicU64 = AtomicU64::new(0);

fn draft_file_name(attempt: u32) -> String {
    let counter = DRAFT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    format!(
        "future-tui-editor-{}-{nanos}-{counter}-{attempt}.md",
        std::process::id()
    )
}

/// Create a file that must not already exist, with owner-only permissions.
fn create_private(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        // A draft of user input: owner read/write only (umask can only clear
        // further bits, never add any).
        options.mode(0o600);
    }
    // On Windows there is no mode(); the file inherits the ACL of the
    // user-owned directory we were given, which is the platform equivalent of
    // "private to this user".
    options.open(path)
}

impl EditorDraft {
    /// Write `initial` to a new draft file inside `dir` (created if missing).
    ///
    /// The name carries the pid, a timestamp and a counter, and the file is
    /// created with `create_new`, so an existing file is never truncated; a
    /// name collision is retried with a fresh name.
    pub fn new(dir: &Path, initial: &str) -> Result<Self, EditorError> {
        fs::create_dir_all(dir)
            .map_err(|err| EditorError::Io(format!("{}: {err}", dir.display())))?;
        let path = create_draft_file(dir, initial, draft_file_name)?;
        Ok(Self { path })
    }

    /// Path of the draft file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read the buffer back after the editor exited.
    pub fn contents(&self) -> Result<String, EditorError> {
        fs::read_to_string(&self.path)
            .map_err(|err| EditorError::Io(format!("{}: {err}", self.path.display())))
    }

    /// Delete the draft file. Idempotent: an already-absent file is success.
    pub fn cleanup(self) -> Result<(), EditorError> {
        remove_draft(&self.path)
    }
}

impl Drop for EditorDraft {
    fn drop(&mut self) {
        // Best effort: `cleanup()` reports the error, dropping cannot. A
        // second removal after an explicit `cleanup()` is a no-op.
        let _ = remove_draft(&self.path);
    }
}

/// Remove `path`, treating "already gone" as success (idempotent cleanup).
fn remove_draft(path: &Path) -> Result<(), EditorError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(EditorError::Io(format!("{}: {err}", path.display()))),
    }
}

/// Create the draft file via `name_for(attempt)`, retrying on collisions.
///
/// `name_for` is injected so the collision/retry arms are reachable in tests
/// without racing real file names.
fn create_draft_file(
    dir: &Path,
    initial: &str,
    mut name_for: impl FnMut(u32) -> String,
) -> Result<PathBuf, EditorError> {
    let mut collision: Option<String> = None;
    for attempt in 0..DRAFT_ATTEMPTS {
        let path = dir.join(name_for(attempt));
        match create_private(&path) {
            Ok(mut file) => {
                let written = file.write_all(initial.as_bytes());
                // Close the handle before anything else touches the path — on
                // Windows an open handle blocks deletion and the editor launch.
                drop(file);
                return finish_draft(path, written);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                collision = Some(err.to_string());
            }
            Err(err) => return Err(EditorError::Io(format!("{}: {err}", path.display()))),
        }
    }
    Err(EditorError::Io(format!(
        "could not create a unique draft file in {} after {DRAFT_ATTEMPTS} attempts{}",
        dir.display(),
        collision.map(|err| format!(": {err}")).unwrap_or_default()
    )))
}

/// Map the seed-write result to a ready draft, removing a partial file so a
/// failed write never leaves a half-written buffer behind.
fn finish_draft(path: PathBuf, written: std::io::Result<()>) -> Result<PathBuf, EditorError> {
    match written {
        Ok(()) => Ok(path),
        Err(err) => {
            let _ = fs::remove_file(&path);
            Err(EditorError::Io(format!("{}: {err}", path.display())))
        }
    }
}

/// Build the command that opens `path` in the resolved editor.
///
/// `parts` is the argv from `resolve_editor_command` (its first element is the
/// program); `path` is appended last. Empty `parts` yields a command with an
/// empty program rather than panicking — the caller validates the command
/// before spawning.
pub fn build_command(parts: &[String], path: &Path) -> Command {
    let program = parts.first().map(String::as_str).unwrap_or("");
    #[cfg(windows)]
    let mut command = Command::new(resolve_windows_program(program));
    #[cfg(not(windows))]
    let mut command = Command::new(program);
    if parts.len() > 1 {
        command.args(&parts[1..]);
    }
    command.arg(path);
    command
}

/// Candidate executable paths for `program` on Windows, honoring `PATH` and
/// `PATHEXT` so shims like `code.cmd` resolve (`Command::new("code")` alone
/// misses them).
///
/// Pure for testability; `resolve_windows_program` feeds it the real
/// environment. A path separator in `program` or any `/`/`\` disables the
/// search (already an explicit path).
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_program_candidates(program: &str, path_var: &str, pathext_var: &str) -> Vec<PathBuf> {
    if program.is_empty() || program.contains('/') || program.contains('\\') {
        return Vec::new();
    }
    let mut extensions: Vec<String> = pathext_var
        .split(';')
        .map(str::trim)
        .filter(|ext| !ext.is_empty())
        .map(|ext| {
            if ext.starts_with('.') {
                ext.to_string()
            } else {
                format!(".{ext}")
            }
        })
        .collect();
    if extensions.is_empty() {
        extensions = [".COM", ".EXE", ".BAT", ".CMD"]
            .iter()
            .map(|ext| (*ext).to_string())
            .collect();
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    for dir in path_var
        .split(';')
        .map(str::trim)
        .filter(|dir| !dir.is_empty())
    {
        for ext in &extensions {
            let candidate = PathBuf::from(dir).join(format!("{program}{ext}"));
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
    }
    candidates
}

/// First existing `PATH`+`PATHEXT` candidate for `program`, falling back to
/// the bare name (which `Command` resolves itself).
#[cfg(windows)]
fn resolve_windows_program(program: &str) -> PathBuf {
    let path_var = std::env::var("PATH").unwrap_or_default();
    let pathext_var = std::env::var("PATHEXT").unwrap_or_default();
    windows_program_candidates(program, &path_var, &pathext_var)
        .into_iter()
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| PathBuf::from(program))
}

/// Drop exactly one trailing line break (`\n`, `\r\n` or a lone `\r`) from
/// editor output; blank lines inside the text are preserved.
pub fn strip_trailing_newline(text: &str) -> String {
    if let Some(stripped) = text.strip_suffix("\r\n") {
        return stripped.to_string();
    }
    if let Some(stripped) = text.strip_suffix('\n').or_else(|| text.strip_suffix('\r')) {
        return stripped.to_string();
    }
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    /// Root bypasses directory permissions, so the permission test is skipped.
    #[cfg(unix)]
    fn is_root() -> bool {
        // SAFETY: geteuid has no preconditions and no side effects.
        unsafe { libc::geteuid() == 0 }
    }

    /// The message of an `EditorError::Io`, or a description of the variant
    /// that arrived instead. Returning a string (rather than panicking on the
    /// unexpected arm) lets the caller's `contains` assertion report the
    /// surprise without leaving a statement no test can reach.
    fn io_message(err: EditorError) -> String {
        match err {
            EditorError::Io(message) => message,
            other => format!("not an io error: {other:?}"),
        }
    }

    #[test]
    fn io_message_reports_the_io_message_and_describes_other_variants() {
        assert_eq!(io_message(EditorError::Io("boom".to_string())), "boom");
        assert_eq!(
            io_message(EditorError::EmptyCommand),
            "not an io error: EmptyCommand"
        );
        assert!(io_message(EditorError::ParseFailed("odd".to_string()))
            .starts_with("not an io error: ParseFailed"));
    }

    // ---------------------------------------------------------------- argv ---

    #[test]
    #[cfg(windows)]
    fn host_dialect_matches_the_platform() {
        assert_eq!(host_dialect(), Dialect::Windows);
    }

    #[test]
    #[cfg(not(windows))]
    fn host_dialect_matches_the_platform() {
        assert_eq!(host_dialect(), Dialect::Posix);
    }

    #[test]
    fn split_command_handles_simple_and_spaced_arguments() {
        assert_eq!(split_command("vim").unwrap(), vec!["vim"]);
        assert_eq!(
            split_command("  zed   --wait \t -f  ").unwrap(),
            vec!["zed", "--wait", "-f"]
        );
        assert_eq!(
            split_command("nvim -c 'set ft=markdown'").unwrap(),
            vec!["nvim", "-c", "set ft=markdown"]
        );
    }

    #[test]
    fn split_command_keeps_empty_quoted_arguments() {
        assert_eq!(split_command("\"\"").unwrap(), vec![""]);
        assert_eq!(split_command("''").unwrap(), vec![""]);
        assert_eq!(split_command("vim '' x").unwrap(), vec!["vim", "", "x"]);
    }

    #[test]
    fn split_command_joins_adjacent_quoted_and_bare_parts() {
        assert_eq!(split_command("a\"b c\"d").unwrap(), vec!["ab cd"]);
        assert_eq!(
            split_command("/usr/bin/\"my editor\" --wait").unwrap(),
            vec!["/usr/bin/my editor", "--wait"]
        );
    }

    #[test]
    fn split_command_posix_escaping() {
        let posix = |raw: &str| split_command_with(raw, Dialect::Posix);
        assert_eq!(posix(r"my\ editor").unwrap(), vec!["my editor"]);
        assert_eq!(posix(r"vim \'q\'").unwrap(), vec!["vim", "'q'"]);
        assert_eq!(posix(r"a\\b").unwrap(), vec![r"a\b"]);
        // A line continuation contributes nothing at all — not even an empty
        // argument.
        assert_eq!(posix("vim \\\n  -w").unwrap(), vec!["vim", "-w"]);
        // Inside "" it contributes nothing either, and does not end the pair.
        assert_eq!(posix("\"a\\\nb\"").unwrap(), vec!["ab"]);
        // Inside "" only `"` `\` `$` and backtick are escapes.
        assert_eq!(posix(r#""a\"b""#).unwrap(), vec![r#"a"b"#]);
        assert_eq!(posix(r#""a\\b""#).unwrap(), vec![r"a\b"]);
        assert_eq!(posix(r#""a\\""#).unwrap(), vec![r"a\"]);
        assert_eq!(posix(r#""\$HOME""#).unwrap(), vec!["$HOME"]);
        assert_eq!(posix(r#""a\db""#).unwrap(), vec![r"a\db"]);
        // Single quotes are fully literal.
        assert_eq!(posix(r"'C:\tools\vim'").unwrap(), vec![r"C:\tools\vim"]);
        assert_eq!(posix(r#"'it"s'"#).unwrap(), vec![r#"it"s"#]);
        // ... and a quote of the other kind inside them is just a character.
        assert_eq!(posix(r#""it's""#).unwrap(), vec!["it's"]);
        assert_eq!(posix(r#"'a "b'"#).unwrap(), vec![r#"a "b"#]);
    }

    #[test]
    fn split_command_windows_quoting() {
        let win = |raw: &str| split_command_with(raw, Dialect::Windows);
        // Backslash is an ordinary character outside quotes ...
        assert_eq!(win(r"C:\tools\ed.exe").unwrap(), vec![r"C:\tools\ed.exe"]);
        assert_eq!(
            win(r"C:\Program Files\Code\code.exe").unwrap(),
            vec![r"C:\Program", r"Files\Code\code.exe"]
        );
        assert_eq!(win(r"C:\").unwrap(), vec![r"C:\"]);
        // ... and survives inside quotes, including a trailing backslash run.
        assert_eq!(
            win(r#""C:\Program Files\Code\code.exe""#).unwrap(),
            vec![r"C:\Program Files\Code\code.exe"]
        );
        assert_eq!(win(r#""C:\dir\\""#).unwrap(), vec![r"C:\dir\"]);
        assert_eq!(win(r#""a\"b""#).unwrap(), vec![r#"a"b"#]);
        assert_eq!(
            win(r#""\\server\share\ed.exe""#).unwrap(),
            vec![r"\\server\share\ed.exe"]
        );
        // Spaces still split without quotes (Windows-like); single quotes do
        // quote, as a convenience for users who write them.
        assert_eq!(win("'a b' c").unwrap(), vec!["a b", "c"]);
    }

    #[test]
    fn split_command_rejects_empty_input() {
        for raw in ["", "   ", "\t\n "] {
            assert_eq!(
                split_command(raw),
                Err(EditorError::EmptyCommand),
                "raw={raw:?}"
            );
        }
    }

    #[test]
    fn split_command_reports_parse_failures() {
        // Unterminated quotes (both kinds) and a dangling escape.
        for raw in [r"'abc", r#""abc"#, r#""a\""#, r#"'a "b"#] {
            // Hoisted: format arguments are only evaluated when the assert fails.
            let result = split_command(raw);
            assert!(
                matches!(&result, Err(EditorError::ParseFailed(_))),
                "raw={raw:?} -> {result:?}"
            );
        }
        assert!(matches!(
            split_command_with("vim \\", Dialect::Posix),
            Err(EditorError::ParseFailed(_))
        ));
        // A dangling escape inside "" is a parse failure too.
        assert!(matches!(
            split_command_with("\"vim \\", Dialect::Posix),
            Err(EditorError::ParseFailed(_))
        ));
        // An unterminated "" on Windows is a parse failure as well.
        assert!(matches!(
            split_command_with(r#""C:\unclosed"#, Dialect::Windows),
            Err(EditorError::ParseFailed(_))
        ));
    }

    // ------------------------------------------------------------ resolve ---

    #[test]
    fn resolve_editor_prefers_visual() {
        assert_eq!(
            resolve_editor_command(Some("vis --wait"), Some("ed")).unwrap(),
            vec!["vis", "--wait"]
        );
    }

    #[test]
    fn resolve_editor_falls_back_to_editor() {
        assert_eq!(
            resolve_editor_command(None, Some("'ed with space' -w")).unwrap(),
            vec!["ed with space", "-w"]
        );
    }

    #[test]
    fn resolve_editor_reports_missing_and_blank() {
        assert_eq!(
            resolve_editor_command(None, None),
            Err(EditorError::MissingEditor)
        );
        assert_eq!(
            resolve_editor_command(Some("   "), Some("ed")),
            Err(EditorError::EmptyCommand)
        );
        assert_eq!(
            resolve_editor_command(Some(""), Some("ed")),
            Err(EditorError::EmptyCommand)
        );
        assert_eq!(
            resolve_editor_command(None, Some("")),
            Err(EditorError::EmptyCommand)
        );
    }

    #[test]
    fn resolve_editor_propagates_parse_failure() {
        assert!(matches!(
            resolve_editor_command(Some("'unclosed"), None),
            Err(EditorError::ParseFailed(_))
        ));
    }

    // -------------------------------------------------------------- draft ---

    #[test]
    fn draft_round_trips_initial_content() {
        let dir = draft_dir();
        let draft = EditorDraft::new(dir.path(), "hello\nworld\n").unwrap();
        assert_eq!(draft.contents().unwrap(), "hello\nworld\n");
        assert!(draft.path().is_file());
        assert!(draft.path().starts_with(dir.path()));
        assert_eq!(
            draft.path().extension().and_then(|e| e.to_str()),
            Some("md")
        );
        // The name is unique per draft.
        let other = EditorDraft::new(dir.path(), "second").unwrap();
        assert_ne!(draft.path(), other.path());
        assert_eq!(other.contents().unwrap(), "second");
    }

    #[test]
    fn draft_creates_missing_directory_and_cleans_up() {
        let dir = draft_dir();
        let nested = dir.path().join("deep").join("nested");
        let draft = EditorDraft::new(&nested, "").unwrap();
        let path = draft.path().to_path_buf();
        assert!(path.is_file());
        assert_eq!(draft.contents().unwrap(), "");
        draft.cleanup().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn draft_contents_reflect_editor_writes() {
        let dir = draft_dir();
        let draft = EditorDraft::new(dir.path(), "seed").unwrap();
        fs::write(draft.path(), "edited\n").unwrap();
        assert_eq!(draft.contents().unwrap(), "edited\n");
        fs::remove_file(draft.path()).unwrap();
        assert!(matches!(draft.contents(), Err(EditorError::Io(_))));
    }

    #[test]
    fn draft_reports_directory_creation_failure() {
        let dir = draft_dir();
        let file = dir.path().join("not-a-dir");
        fs::write(&file, "x").unwrap();
        assert!(matches!(
            EditorDraft::new(&file, "seed"),
            Err(EditorError::Io(_))
        ));
    }

    #[test]
    fn cleanup_is_idempotent_and_drop_does_not_leak() {
        let dir = draft_dir();
        let draft = EditorDraft::new(dir.path(), "x").unwrap();
        let path = draft.path().to_path_buf();
        // Removing twice through the same code path: second call is a no-op.
        remove_draft(&path).unwrap();
        remove_draft(&path).unwrap();
        // cleanup() after the file is already gone still succeeds.
        draft.cleanup().unwrap();

        let dropped = EditorDraft::new(dir.path(), "y").unwrap();
        let dropped_path = dropped.path().to_path_buf();
        drop(dropped);
        assert!(!dropped_path.exists(), "drop must not leak a draft file");
    }

    #[test]
    fn draft_files_are_owner_only() {
        let dir = draft_dir();
        let draft = EditorDraft::new(dir.path(), "secret").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(draft.path()).unwrap().permissions().mode();
            assert_eq!(mode & 0o600, 0o600, "owner must read/write: {mode:o}");
            assert_eq!(mode & 0o077, 0, "no group/other access: {mode:o}");
        }
        #[cfg(not(unix))]
        {
            assert!(draft.path().is_file());
        }
    }

    #[test]
    fn create_private_never_truncates_an_existing_file() {
        let dir = draft_dir();
        let path = dir.path().join("taken.md");
        drop(create_private(&path).unwrap());
        let err = create_private(&path).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&path).unwrap(), "");
    }

    #[test]
    fn finish_draft_discards_partial_buffers() {
        let dir = draft_dir();
        let good = dir.path().join("good.md");
        fs::write(&good, "seed").unwrap();
        assert_eq!(finish_draft(good.clone(), Ok(())).unwrap(), good);
        assert!(good.exists());

        let partial = dir.path().join("partial.md");
        fs::write(&partial, "half").unwrap();
        let err = finish_draft(
            partial.clone(),
            Err(std::io::Error::new(
                std::io::ErrorKind::StorageFull,
                "disk full",
            )),
        )
        .unwrap_err();
        let message = io_message(err);
        assert!(message.contains("disk full"), "{message}");
        assert!(message.contains("partial.md"), "{message}");
        assert!(!partial.exists(), "a partial draft must not survive");
    }

    #[test]
    fn create_draft_file_retries_name_collisions() {
        let dir = draft_dir();
        let existing = dir.path().join("fixed.md");
        fs::write(&existing, "keep").unwrap();
        let mut attempts = 0u32;
        let path = create_draft_file(dir.path(), "fresh", |attempt| {
            attempts += 1;
            if attempt == 0 {
                "fixed.md".to_string()
            } else {
                "free.md".to_string()
            }
        })
        .unwrap();
        assert_eq!(path, dir.path().join("free.md"));
        assert_eq!(attempts, 2);
        assert_eq!(fs::read_to_string(&path).unwrap(), "fresh");
        // The pre-existing file is untouched.
        assert_eq!(fs::read_to_string(&existing).unwrap(), "keep");
    }

    #[test]
    fn create_draft_file_gives_up_after_repeated_collisions() {
        let dir = draft_dir();
        fs::write(dir.path().join("always.md"), "x").unwrap();
        let err = create_draft_file(dir.path(), "seed", |_| "always.md".to_string()).unwrap_err();
        let message = io_message(err);
        assert!(message.contains("unique draft file"), "{message}");
        assert!(message.contains(&DRAFT_ATTEMPTS.to_string()), "{message}");
    }

    /// The **non-collision** failure arm, on every platform. A name that cannot
    /// be created at all (its parent directory does not exist) is not a name
    /// collision: retrying the same broken name would loop, so the error must
    /// come straight back and name the path it could not create. The POSIX test
    /// below reaches the same arm through permissions; this one needs no
    /// platform-specific setup.
    #[test]
    fn create_draft_file_reports_a_name_that_cannot_be_created() {
        let dir = draft_dir();
        let mut attempts = 0u32;
        let err = create_draft_file(dir.path(), "seed", |_| {
            attempts += 1;
            "missing-subdirectory/draft.md".to_string()
        })
        .unwrap_err();
        assert_eq!(attempts, 1, "a non-collision error must not be retried");
        let message = io_message(err);
        assert!(
            message.contains("draft.md"),
            "the failing path must be named: {message}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn create_draft_file_reports_unwritable_directory() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = draft_dir();
        let locked = dir.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).unwrap();
        let result = create_draft_file(&locked, "seed", draft_file_name);
        // Restore permissions so the TempDir can clean itself up.
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
        // An unprivileged user gets `Io(` from the locked directory; root
        // ignores the mode bits (`is_root`), so a single assertion covers both
        // hosts instead of an early return no unprivileged run could reach.
        let io_error = matches!(result, Err(EditorError::Io(_)));
        assert_eq!(io_error, !is_root(), "{result:?}");
    }

    #[test]
    fn remove_draft_reports_a_failure_other_than_a_missing_file() {
        let dir = draft_dir();
        let occupied = dir.path().join("occupied");
        fs::create_dir(&occupied).unwrap();
        // Unlinking a directory is never `NotFound`, so it must surface as Io.
        let message = io_message(remove_draft(&occupied).expect_err("directory removal fails"));
        assert!(message.contains("occupied"), "{message}");
        assert!(occupied.is_dir(), "the directory must survive the failure");
        // Idempotent arm: an already-absent path is success, not an error.
        assert!(remove_draft(&dir.path().join("never-created")).is_ok());
    }

    #[test]
    fn draft_file_names_are_unique_and_marked() {
        let first = draft_file_name(0);
        let second = draft_file_name(0);
        assert_ne!(first, second);
        assert!(first.starts_with("future-tui-editor-"), "{first}");
        assert!(first.contains(&std::process::id().to_string()), "{first}");
        assert!(first.ends_with("-0.md"), "{first}");
        assert!(draft_file_name(1).ends_with("-1.md"));
    }

    // ------------------------------------------------------------ command ---

    #[test]
    fn build_command_appends_the_path_after_the_arguments() {
        let path = Path::new("draft dir").join("buffer.md");
        let parts = vec!["vim".to_string(), "-c".to_string(), "set paste".to_string()];
        let command = build_command(&parts, &path);
        assert_eq!(command.get_program(), std::ffi::OsStr::new("vim"));
        let args: Vec<&std::ffi::OsStr> = command.get_args().collect();
        assert_eq!(
            args,
            vec![
                std::ffi::OsStr::new("-c"),
                std::ffi::OsStr::new("set paste"),
                path.as_os_str(),
            ]
        );
    }

    #[test]
    fn build_command_handles_a_lone_program_and_empty_parts() {
        let path = Path::new("buffer.md");
        let single = vec!["ed".to_string()];
        let command = build_command(&single, path);
        assert_eq!(command.get_program(), std::ffi::OsStr::new("ed"));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![path.as_os_str()]
        );

        // Empty argv must not panic; it produces an unusable program name that
        // the caller validates before spawning.
        let empty: Vec<String> = Vec::new();
        let command = build_command(&empty, path);
        assert_eq!(command.get_program(), std::ffi::OsStr::new(""));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![path.as_os_str()]
        );
    }

    // ----------------------------------------------------- windows lookup ---

    #[test]
    fn windows_candidates_follow_path_and_pathext_order() {
        let candidates = windows_program_candidates(
            "code",
            r"C:\bin;C:\Users\me\AppData\Local\Programs\VS Code\bin",
            ".COM;.EXE;.BAT;.CMD",
        );
        assert_eq!(candidates[0], PathBuf::from(r"C:\bin").join("code.COM"));
        assert!(candidates.contains(
            &PathBuf::from(r"C:\Users\me\AppData\Local\Programs\VS Code\bin").join("code.CMD")
        ));
        assert_eq!(candidates.len(), 8);
        // Lowercase extensions are honored verbatim.
        assert_eq!(
            windows_program_candidates("code", r"C:\bin", ";.exe;.cmd;"),
            vec![
                PathBuf::from(r"C:\bin").join("code.exe"),
                PathBuf::from(r"C:\bin").join("code.cmd"),
            ]
        );
    }

    #[test]
    fn windows_candidates_default_pathext_and_explicit_paths() {
        assert_eq!(
            windows_program_candidates("ed", r"C:\bin", ""),
            vec![
                PathBuf::from(r"C:\bin").join("ed.COM"),
                PathBuf::from(r"C:\bin").join("ed.EXE"),
                PathBuf::from(r"C:\bin").join("ed.BAT"),
                PathBuf::from(r"C:\bin").join("ed.CMD"),
            ]
        );
        // Extensions without a leading dot still match real files.
        assert_eq!(
            windows_program_candidates("ed", r"C:\bin", "EXE"),
            vec![PathBuf::from(r"C:\bin").join("ed.EXE")]
        );
        // Explicit paths and empty programs are left alone.
        for program in [r"C:\tools\vim.exe", "tools/vim", "", "/usr/bin/vim"] {
            assert!(
                windows_program_candidates(program, r"C:\bin", ".EXE").is_empty(),
                "program={program:?}"
            );
        }
        // Empty PATH entries are skipped.
        assert_eq!(
            windows_program_candidates("ed", r";C:\bin;;", ".EXE"),
            vec![PathBuf::from(r"C:\bin").join("ed.EXE")]
        );
    }

    // -------------------------------------------------------------- misc ----

    #[test]
    fn strip_trailing_newline_removes_at_most_one_break() {
        assert_eq!(strip_trailing_newline("a\n"), "a");
        assert_eq!(strip_trailing_newline("a\r\n"), "a");
        assert_eq!(strip_trailing_newline("a\r"), "a");
        assert_eq!(strip_trailing_newline("a\n\n"), "a\n");
        assert_eq!(strip_trailing_newline("a\n\n\n"), "a\n\n");
        assert_eq!(strip_trailing_newline("\n"), "");
        assert_eq!(strip_trailing_newline("\n\n"), "\n");
        assert_eq!(strip_trailing_newline("a"), "a");
        assert_eq!(strip_trailing_newline(""), "");
        assert_eq!(strip_trailing_newline("a\nb"), "a\nb");
        assert_eq!(strip_trailing_newline("a\r\n\r\n"), "a\r\n");
        // Only the end is touched: interior blank lines survive.
        assert_eq!(strip_trailing_newline("\n\nkeep\n\n"), "\n\nkeep\n");
    }

    #[test]
    fn errors_are_displayable() {
        let errors = [
            EditorError::MissingEditor,
            EditorError::EmptyCommand,
            EditorError::ParseFailed("why".to_string()),
            EditorError::Io("boom".to_string()),
        ];
        for error in &errors {
            let text = error.to_string();
            assert!(!text.is_empty());
            assert_eq!(&format!("{error}"), &text);
        }
        assert!(errors[2].to_string().contains("why"));
        // Usable as a boxed error (integrators log it as one).
        let boxed: Box<dyn std::error::Error> = Box::new(EditorError::MissingEditor);
        assert!(boxed.to_string().contains("VISUAL"));
    }
}
