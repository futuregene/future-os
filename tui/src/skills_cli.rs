//! Skill catalogue + install plumbing for the TUI, via the unified `future` binary.
//!
//! The TUI cannot reuse the CLI's skill code in-process: `future-cli` depends on
//! `future-tui` (it embeds this crate as `future tui`), so a reverse dependency
//! would be circular. Skill install/uninstall/update therefore shell out to
//! `future skills <verb>` exactly like the desktop app does for built-in skills
//! (`desktop/src-tauri/src/skills_bootstrap.rs` runs the bundled `future` CLI).
//!
//! Binary resolution mirrors `crate::agent_supervisor::launch_candidates`:
//! when the process *is* the unified binary (`future tui`) use it directly,
//! else look for a sibling `future` next to the current executable, else fall
//! back to `PATH`.
//!
//! Implemented by the `skills-cli` worker; see
//! `.future/tui-parity/texts/skills-cli.md` for the required API.
//!
//! # What this module guarantees
//!
//! * **One program, three verbs.** [`SkillsCli::list`] runs
//!   `future skills list --json` and parses the document the CLI emits
//!   (`{"skills":[{"id", "name", "latestVersion", "installedVersion",
//!   "description", "descriptionZh"}], "count"}`); [`SkillsCli::run`] runs
//!   `install`/`uninstall`/`update` and reports the exit code plus both streams.
//! * **The id is the security boundary.** [`validate_skill_id`] applies the CLI's
//!   own rule (no empty value, no path separator, no `..`, no whitespace or
//!   control character, no Windows device name, ≤128 bytes) *before* anything is
//!   spawned, and a rejected operation never reaches the injected runner — the
//!   tests prove that by asserting the runner recorded no call at all.
//! * **A hung child cannot hang the TUI.** The real runner waits at most
//!   [`SKILL_OP_TIMEOUT`] — [`SKILL_LIST_TIMEOUT`] for the read-only browse
//!   call — then kills *and reaps* the child and fails with
//!   [`TIMEOUT_MARKER`], which [`SkillsCli::run`] turns into
//!   [`SkillOpOutcome::timed_out`]. `stdin` is `/dev/null`: the child shares the
//!   TUI's terminal, so inheriting raw-mode stdin would feed it the user's keys.
//! * **A chatty child cannot deadlock on a full pipe.** Both pipes are drained
//!   for as long as the child runs (a reader thread per stream), because a
//!   child whose output exceeds the OS pipe buffer (~64 KiB) blocks in `write`
//!   until someone reads. Capture is capped at [`MAX_CAPTURED_BYTES`]: past it
//!   the child is killed and the call fails explicitly rather than growing the
//!   TUI's memory or handing back a silently truncated document.
//! * **Every process boundary is injectable.** [`SkillsCli::with_runner`] takes
//!   the spawner and the program path, so no test of `list`/`run` ever launches
//!   the real `future` binary.

use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::Value;

/// Upper bound on one *mutating* `future skills …` invocation (install,
/// uninstall, update). Those really do work — resolve, download, copy — so
/// reaching it means something is wrong and killing the child is the only way
/// back to a usable TUI.
pub const SKILL_OP_TIMEOUT: Duration = Duration::from_secs(120);

/// Upper bound on a *browsing* `future skills list --json` call.
///
/// Browsing only reads a catalogue, so a child that has not answered in this
/// much time is wedged rather than busy; the panel can ask again (`r`), which
/// is why the failure is reported as retryable. Kept well under
/// [`SKILL_OP_TIMEOUT`] so a broken catalogue cannot make `/skills` look hung
/// for two minutes.
pub const SKILL_LIST_TIMEOUT: Duration = Duration::from_secs(15);

/// The wait budget for one invocation of `args`: browsing gets
/// [`SKILL_LIST_TIMEOUT`], everything else [`SKILL_OP_TIMEOUT`].
pub fn timeout_for_args(args: &[String]) -> Duration {
    if is_browse_args(args) {
        SKILL_LIST_TIMEOUT
    } else {
        SKILL_OP_TIMEOUT
    }
}

/// Is `args` one of the read-only catalogue calls (`skills list …`)?
///
/// Matched on the subcommand rather than on exact equality with
/// [`list_args`], so a future flag on the browse call cannot silently fall back
/// to the write budget.
fn is_browse_args(args: &[String]) -> bool {
    args.len() >= 2 && args[0] == "skills" && args[1] == "list"
}

/// Prefix of the runner error raised when [`SKILL_OP_TIMEOUT`] expired and the
/// child was killed.
///
/// The injected runner can only hand back a `String`, so the timeout travels as
/// this marker and [`SkillsCli::run`] maps it onto
/// [`SkillOpOutcome::timed_out`].
pub const TIMEOUT_MARKER: &str = "future skills: timed out";

/// How often the blocking waiter re-checks a running child.
const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Hard ceiling on the output one `future skills …` invocation may hand back.
///
/// The waiter drains both pipes while the child runs, so a child that talks a
/// lot no longer wedges on a full pipe — but draining without a ceiling would
/// only trade that deadlock for unbounded memory in the TUI. Crossing the
/// ceiling is therefore an explicit error (the child is killed), never a silent
/// truncation that a later `parse_catalogue` would mistake for a short-but-valid
/// document. 16 MiB is ~200× the real `future skills list --json` document
/// (~81 KiB with a full local catalogue) and far above anything a skill
/// operation writes, so a healthy child can never reach it.
const MAX_CAPTURED_BYTES: usize = 16 * 1024 * 1024;

/// Read size for one draining thread: big enough that a large payload costs few
/// syscalls, small enough that a burst of output is never overshot by much
/// before the waiter can check [`MAX_CAPTURED_BYTES`].
const PIPE_READ_CHUNK: usize = 8 * 1024;

/// Name of the unified binary (the CLI embeds this TUI as `future tui`).
const FUTURE_BINARY: &str = "future";

/// Longest captured-output excerpt embedded in an error or summary line.
const EXCERPT_MAX_CHARS: usize = 200;

#[cfg(windows)]
const EXE_SUFFIX: &str = ".exe";
#[cfg(not(windows))]
const EXE_SUFFIX: &str = "";

/// One catalogue row: what the platform offers plus what is installed locally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillCatalogueEntry {
    pub id: String,
    pub name: Option<String>,
    pub summary: Option<String>,
    pub summary_zh: Option<String>,
    pub latest_version: Option<String>,
    pub installed_version: Option<String>,
}

/// The parsed `future skills list --json` document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillCatalogue {
    pub entries: Vec<SkillCatalogueEntry>,
}

/// One mutating skill operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillOp {
    Install {
        id: String,
        version: Option<String>,
    },
    Uninstall {
        id: String,
    },
    /// `future skills update` — upgrade every installed skill.
    UpdateAll,
}

/// What was attempted and what came back.
///
/// `exit_code` is `None` when no process ran to completion (spawn failure,
/// invalid input, timeout); `timed_out` distinguishes a killed child from any
/// other runner error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillOpOutcome {
    pub op: SkillOp,
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

/// Injected process executor: `(program, args) -> (exit_code, stdout, stderr)`.
///
/// `Err` means "no exit code at all" (spawn failure, or a timeout when the
/// message starts with [`TIMEOUT_MARKER`]).
pub type SkillRunner =
    Box<dyn Fn(&Path, &[String]) -> Result<(i32, String, String), String> + Send + Sync>;

/// Skill catalogue + install plumbing bound to one `future` binary.
pub struct SkillsCli {
    runner: SkillRunner,
    program: PathBuf,
}

impl SkillsCli {
    /// Real binary and real process spawner.
    pub fn new() -> Self {
        Self {
            runner: Box::new(spawn_runner),
            program: resolve_program(),
        }
    }

    /// Test seam: an injected spawner and an explicit program path.
    pub fn with_runner(runner: SkillRunner, program: PathBuf) -> Self {
        Self { runner, program }
    }

    /// The binary every call goes through.
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// `future skills list --json`.
    ///
    /// A non-zero exit code or an unparseable document is an `Err` — the
    /// message always carries whatever the child wrote to stderr, so a failure
    /// is never swallowed.
    pub fn list(&self) -> Result<SkillCatalogue, String> {
        let args = list_args();
        let outcome = match (self.runner)(&self.program, &args) {
            Ok(outcome) => outcome,
            Err(err) => return Err(format!("failed to run `future skills list --json`: {err}")),
        };
        let (code, stdout, stderr) = outcome;
        if code != 0 {
            return Err(format!(
                "`future skills list --json` exited with status {code}{}",
                stderr_suffix(&stderr)
            ));
        }
        parse_catalogue(&stdout).map_err(|err| format!("{err}{}", stderr_suffix(&stderr)))
    }

    /// Run one mutating operation.
    ///
    /// An id (or version) that fails [`validate_skill_id`] is rejected here and
    /// the runner is never called.
    pub fn run(&self, op: &SkillOp) -> SkillOpOutcome {
        match validate_op(op) {
            Ok(()) => self.run_validated(op),
            Err(reason) => rejected(op, reason),
        }
    }

    /// The `run` body for an operation that passed validation.
    fn run_validated(&self, op: &SkillOp) -> SkillOpOutcome {
        let args = op_args(op);
        match (self.runner)(&self.program, &args) {
            Ok((exit_code, stdout, stderr)) => SkillOpOutcome {
                op: op.clone(),
                ok: exit_code == 0,
                exit_code: Some(exit_code),
                stdout,
                stderr,
                timed_out: false,
            },
            Err(err) => failed(op, err),
        }
    }
}

impl Default for SkillsCli {
    fn default() -> Self {
        Self::new()
    }
}

/// The outcome of an operation rejected before any process was spawned.
fn rejected(op: &SkillOp, reason: String) -> SkillOpOutcome {
    SkillOpOutcome {
        op: op.clone(),
        ok: false,
        exit_code: None,
        stdout: String::new(),
        stderr: reason,
        timed_out: false,
    }
}

/// The outcome of a runner that produced no exit code: a timeout when the
/// message carries [`TIMEOUT_MARKER`], otherwise a spawn/wait failure.
fn failed(op: &SkillOp, err: String) -> SkillOpOutcome {
    SkillOpOutcome {
        op: op.clone(),
        ok: false,
        exit_code: None,
        stdout: String::new(),
        timed_out: err.starts_with(TIMEOUT_MARKER),
        stderr: err,
    }
}

/// Arguments of `future skills list --json`.
pub fn list_args() -> Vec<String> {
    vec![
        "skills".to_string(),
        "list".to_string(),
        "--json".to_string(),
    ]
}

/// Arguments of one mutating operation.
pub fn op_args(op: &SkillOp) -> Vec<String> {
    match op {
        SkillOp::Install { id, version } => {
            let mut args = vec!["skills".to_string(), "install".to_string(), id.clone()];
            if let Some(version) = version {
                args.extend(["--version".to_string(), version.clone()]);
            }
            args
        }
        SkillOp::Uninstall { id } => {
            vec!["skills".to_string(), "uninstall".to_string(), id.clone()]
        }
        SkillOp::UpdateAll => vec!["skills".to_string(), "update".to_string()],
    }
}

/// Check that an operation may be spawned at all.
fn validate_op(op: &SkillOp) -> Result<(), String> {
    match op {
        SkillOp::Install { id, version } => {
            reject_component("id", id)?;
            if let Some(version) = version {
                reject_component("version", version)?;
            }
            Ok(())
        }
        SkillOp::Uninstall { id } => reject_component("id", id),
        SkillOp::UpdateAll => Ok(()),
    }
}

/// [`validate_skill_id`] with the offending field named in the message.
fn reject_component(label: &str, value: &str) -> Result<(), String> {
    validate_skill_id(value).map_err(|reason| format!("invalid skill {label}: {reason}"))
}

/// The CLI's own rule (`cli/src/commands/skills.rs::validate_skill_component`):
/// a skill id becomes one path component under the skills directory, so it must
/// be a plain slug — non-empty, ≤128 bytes, no path separator, no `..`, no
/// whitespace or control character, no Windows device name.
pub fn validate_skill_id(id: &str) -> Result<(), String> {
    match id_rejection(id) {
        Some(reason) => Err(reason),
        None => Ok(()),
    }
}

/// Why `id` is not a usable skill component, or `None` when it is.
fn id_rejection(id: &str) -> Option<String> {
    if id.is_empty() {
        return Some("skill id is empty".to_string());
    }
    if id.len() > 128 {
        return Some(format!("skill id is {} bytes (max 128)", id.len()));
    }
    if id.contains("..") || id.ends_with('.') {
        return Some(format!("{id:?} must not contain `..` or end with a dot"));
    }
    let stem = id.split('.').next().unwrap_or_default();
    if is_reserved_stem(stem) {
        return Some(format!("{id:?} is a reserved device name"));
    }
    if !id.bytes().all(is_skill_component_byte) {
        return Some(format!(
            "{id:?} may only contain ASCII letters, digits, `.`, `_` and `-`"
        ));
    }
    None
}

/// Windows device names (`CON`, `NULL`, `COM1`, …) that cannot be directories.
fn is_reserved_stem(stem: &str) -> bool {
    let stem = stem.to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let device = stem.starts_with("COM") || stem.starts_with("LPT");
    device && stem.len() == 4 && matches!(stem.as_bytes()[3], b'1'..=b'9')
}

/// Bytes a skill component may consist of.
fn is_skill_component_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
}

/// Parse the `future skills list --json` document.
///
/// Tolerant about framing — a progress line printed before or after the document
/// is ignored by slicing from the first `{` to the last `}` — and about field
/// naming (`latestVersion`/`latest_version`, `description`/`summary`,
/// `descriptionZh`/`summaryZh`), because the wire document is owned by the CLI.
/// Anything that is not a JSON object with a `skills` array of objects is an
/// `Err`.
pub fn parse_catalogue(stdout: &str) -> Result<SkillCatalogue, String> {
    let object = extract_json_object(stdout)?;
    let value: Value = serde_json::from_str(object)
        .map_err(|err| format!("invalid skills catalogue JSON: {err}"))?;
    let entries = match value.get("skills") {
        Some(Value::Array(items)) => items.iter().filter_map(parse_entry).collect(),
        Some(Value::Null) | None => Vec::new(),
        Some(other) => {
            return Err(format!(
                "skills catalogue field `skills` is not an array: {other}"
            ))
        }
    };
    Ok(SkillCatalogue { entries })
}

/// The outermost `{…}` of `stdout`, or an `Err` describing what came instead.
fn extract_json_object(stdout: &str) -> Result<&str, String> {
    match (stdout.find('{'), stdout.rfind('}')) {
        (Some(start), Some(end)) if end > start => Ok(&stdout[start..=end]),
        _ => Err(format!(
            "skills catalogue: no JSON object in output: {}",
            output_excerpt(stdout)
        )),
    }
}

/// One catalogue row, or `None` when the row has no usable id.
fn parse_entry(value: &Value) -> Option<SkillCatalogueEntry> {
    let id = value.get("id").and_then(Value::as_str)?;
    if id.is_empty() {
        return None;
    }
    Some(SkillCatalogueEntry {
        id: id.to_string(),
        name: string_field(value, &["name"]),
        summary: string_field(value, &["summary", "description"]),
        summary_zh: string_field(value, &["summaryZh", "descriptionZh"]),
        latest_version: string_field(value, &["latestVersion", "latest_version"]),
        installed_version: string_field(value, &["installedVersion", "installed_version"]),
    })
}

/// First of `keys` present as a non-empty string.
///
/// An absent key, an explicit `null` and a `""` all mean "nothing to show", so
/// they collapse to `None` instead of surfacing an empty label to the UI.
fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| value.get(*key).and_then(Value::as_str))
        .find(|text| !text.is_empty())
        .map(str::to_string)
}

/// Single-line summary of an operation outcome, for a system/chat message.
pub fn summarize_outcome(outcome: &SkillOpOutcome) -> String {
    let label = op_label(&outcome.op);
    if outcome.timed_out {
        return format!("{label} timed out");
    }
    if outcome.ok {
        return format!("{label} succeeded");
    }
    let stderr = output_excerpt(&outcome.stderr);
    let detail = if stderr.is_empty() {
        output_excerpt(&outcome.stdout)
    } else {
        stderr
    };
    match outcome.exit_code {
        Some(code) if detail.is_empty() => format!("{label} failed (exit code {code})"),
        Some(code) => format!("{label} failed (exit code {code}): {detail}"),
        None if detail.is_empty() => format!("{label} failed"),
        None => format!("{label} failed: {detail}"),
    }
}

/// Human-readable name of an operation (`install future-web@1.2`).
fn op_label(op: &SkillOp) -> String {
    match op {
        SkillOp::Install { id, version } => match version {
            Some(version) => format!("install {id}@{version}"),
            None => format!("install {id}"),
        },
        SkillOp::Uninstall { id } => format!("uninstall {id}"),
        SkillOp::UpdateAll => "update all skills".to_string(),
    }
}

/// Catalogue entries whose installed version is strictly older than the latest.
///
/// Versions are compared with semver: a leading `v` is accepted and ignored,
/// shortened catalogue versions (`1.2`) are padded (`1.2.0`), `1.9 < 1.10`
/// numerically, a pre-release sorts before its release, and build metadata is
/// ignored (`1.0.0+a` is not an upgrade of `1.0.0+b`). A missing or
/// unparseable version on either side is *not* upgradable — an unknown version
/// must never trigger a download.
pub fn upgradable(catalogue: &SkillCatalogue) -> Vec<&SkillCatalogueEntry> {
    catalogue
        .entries
        .iter()
        .filter(|entry| is_upgradable(entry))
        .collect()
}

/// Whether `entry` has both versions and installed < latest.
fn is_upgradable(entry: &SkillCatalogueEntry) -> bool {
    let (Some(installed), Some(latest)) = (
        entry.installed_version.as_deref(),
        entry.latest_version.as_deref(),
    ) else {
        return false;
    };
    match (parse_version(installed), parse_version(latest)) {
        // `cmp_precedence` ignores build metadata, so `1.0.0+a` does not count
        // as an upgrade of `1.0.0+b` (semver's `Ord` would, as a tie-breaker).
        (Some(installed), Some(latest)) => installed.cmp_precedence(&latest).is_lt(),
        _ => false,
    }
}

/// Semver version, tolerating surrounding whitespace and a leading `v`.
/// Semver version, tolerating surrounding whitespace, a leading `v` and the
/// shortened forms the platform catalogue uses (`1`, `1.2` — strict semver
/// needs all three components).
fn parse_version(raw: &str) -> Option<semver::Version> {
    let trimmed = raw.trim();
    let without_prefix = trimmed.strip_prefix(['v', 'V']).unwrap_or(trimmed);
    match semver::Version::parse(without_prefix) {
        Ok(version) => Some(version),
        Err(_) => semver::Version::parse(&pad_version(without_prefix)).ok(),
    }
}

/// `1` → `1.0.0`, `1.2` → `1.2.0`; a numeric core with three or more
/// components is left alone (as is any pre-release/build suffix).
fn pad_version(version: &str) -> String {
    let (core, suffix) = match version.find(['-', '+']) {
        Some(index) => version.split_at(index),
        None => (version, ""),
    };
    let mut padded = core.to_string();
    for _ in core.split('.').count()..3 {
        padded.push_str(".0");
    }
    padded.push_str(suffix);
    padded
}

/// One-line, length-capped excerpt of captured process output; blank input is
/// `""` so callers can omit an empty detail instead of printing `": "`.
fn output_excerpt(text: &str) -> String {
    let flattened = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flattened.is_empty() {
        return String::new();
    }
    if flattened.chars().count() <= EXCERPT_MAX_CHARS {
        return flattened;
    }
    let head: String = flattened.chars().take(EXCERPT_MAX_CHARS).collect();
    format!("{head}…")
}

/// `; stderr: …` when the child wrote anything, `""` otherwise.
fn stderr_suffix(stderr: &str) -> String {
    let excerpt = output_excerpt(stderr);
    if excerpt.is_empty() {
        String::new()
    } else {
        format!("; stderr: {excerpt}")
    }
}

// ─── Binary resolution ─────────────────────────────────────────────────────

/// Candidate paths for the unified binary, most specific first.
///
/// `current_exe` itself when it *is* `future` (`future tui`, so the sidecar we
/// ship with the app), then a sibling `future` next to it (the `future` next to
/// a `future-tui` install), then one entry per `PATH` directory, and finally the
/// bare name (which `Command` resolves through `PATH` at spawn time).
pub fn future_binary_candidates(current_exe: &Path, path_env: Option<&str>) -> Vec<PathBuf> {
    candidates_with_suffix(current_exe, path_env, EXE_SUFFIX)
}

/// [`future_binary_candidates`] for an explicit executable suffix, so the
/// Windows sibling name (`future.exe`) is testable on every platform.
fn candidates_with_suffix(
    current_exe: &Path,
    path_env: Option<&str>,
    suffix: &str,
) -> Vec<PathBuf> {
    let name = format!("{FUTURE_BINARY}{suffix}");
    let mut candidates: Vec<PathBuf> = Vec::new();
    if executable_name(current_exe) == FUTURE_BINARY {
        candidates.push(current_exe.to_path_buf());
    }
    if let Some(parent) = current_exe.parent() {
        candidates.push(parent.join(&name));
    }
    if let Some(path_env) = path_env {
        for dir in std::env::split_paths(path_env) {
            if dir.as_os_str().is_empty() {
                continue;
            }
            candidates.push(dir.join(&name));
        }
    }
    candidates.push(PathBuf::from(FUTURE_BINARY));
    candidates.dedup();
    candidates
}

/// Lower-cased file stem (`/opt/future.exe` → `future`), or `""` for a path
/// without one (`/`). Mirrors `agent_supervisor::executable_name`.
fn executable_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// The program [`SkillsCli::new`] uses: the first candidate that exists as a
/// file, else the bare name.
fn resolve_program() -> PathBuf {
    let current_exe = std::env::current_exe().unwrap_or_default();
    let path_env = std::env::var_os("PATH");
    let path_env = path_env.as_deref().and_then(std::ffi::OsStr::to_str);
    pick_existing(&future_binary_candidates(&current_exe, path_env))
}

/// First candidate that is an existing file, else the bare `future` name.
fn pick_existing(candidates: &[PathBuf]) -> PathBuf {
    match candidates.iter().find(|candidate| candidate.is_file()) {
        Some(existing) => existing.clone(),
        None => PathBuf::from(FUTURE_BINARY),
    }
}

// ─── Real runner ───────────────────────────────────────────────────────────

/// Real [`SkillRunner`]: spawn `program args…` with a capped wait.
fn spawn_runner(program: &Path, args: &[String]) -> Result<(i32, String, String), String> {
    spawn_with_timeout(program, args, timeout_for_args(args))
}

/// Spawn and wait with an explicit timeout (the test seam for
/// [`SKILL_OP_TIMEOUT`], which no test may actually wait for).
///
/// `pub(crate)` because `/worktree` (`crate::worktree`) spawns `git` through it
/// too: the drain-while-waiting and the capped-output machinery below are the
/// fix for a real deadlock (see [`wait_for`]), and a second copy of it in
/// another module is how that defect comes back.
pub(crate) fn spawn_with_timeout(
    program: &Path,
    args: &[String],
    timeout: Duration,
) -> Result<(i32, String, String), String> {
    let child = Command::new(program)
        .args(args)
        // `/dev/null`, never the TUI's stdin: the child shares our terminal, so
        // an inherited raw-mode stdin would swallow the user's keystrokes.
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to spawn {}: {err}", program.display()))?;
    wait_for(child, timeout)
}

/// Poll `child` until it exits *and* both pipes are closed, or `timeout`
/// expires.
///
/// The blocking poll loop is what makes the timeout enforceable: the child's
/// handle stays ours, so a child that outlives the deadline is killed *and*
/// reaped (an unwaited child would stay a zombie until the TUI exits). Output
/// captured before a kill is discarded — a timeout is a failure, not a partial
/// success.
///
/// Draining is not optional: `wait_with_output` used to run only *after* the
/// child exited, so a child whose output exceeded the OS pipe buffer (~64 KiB)
/// blocked in `write` forever and was killed as a timeout even though it had
/// answered instantly — the `/skills` panel's red "Failed to list installable
/// skills" on any machine whose catalogue is bigger than the buffer. Reading
/// both streams while the child runs (what [`CapturedPipes`] is for) makes the
/// wait independent of how much the child writes.
fn wait_for(mut child: Child, timeout: Duration) -> Result<(i32, String, String), String> {
    let mut pipes = CapturedPipes::start(&mut child);
    let deadline = Instant::now() + timeout;
    let mut code = None;
    loop {
        if let Some(error) = pipes.pump() {
            terminate(&mut child);
            return Err(error);
        }
        if code.is_none() {
            code = child
                .try_wait()
                .expect("a live child can always be polled")
                .map(|status| status_code(&status));
        }
        // Only the *pipes* can still be holding output once the child is gone.
        if let Some(code) = code.filter(|_| pipes.closed()) {
            return Ok(pipes.streams(code));
        }
        if Instant::now() >= deadline {
            terminate(&mut child);
            return Err(deadline_error(code.is_some(), timeout));
        }
        thread::sleep(CHILD_POLL_INTERVAL);
    }
}

/// The failure for a wait that ran out of budget.
///
/// [`TIMEOUT_MARKER`] when the child was still running and had to be killed;
/// otherwise a distinct message, because a child that already answered and left
/// a pipe open (a descendant inherited the write end) is not a timeout, and
/// pretending its output was "cut short" would be inventing a result.
fn deadline_error(child_exited: bool, timeout: Duration) -> String {
    if child_exited {
        format!(
            "future skills: child exited but a descendant kept its output pipe open for {}s",
            timeout.as_secs()
        )
    } else {
        format!("{TIMEOUT_MARKER} after {}s", timeout.as_secs())
    }
}

/// Which of a child's two pipes a captured chunk came from.
#[derive(Clone, Copy)]
enum Stream {
    Stdout,
    Stderr,
}

/// One message from a pipe-draining thread to the waiter.
enum PipeMessage {
    /// Bytes just read from one stream.
    Data(Stream, Vec<u8>),
    /// That stream is closed; no further bytes will follow for it.
    Eof(Stream),
}

/// Read `pipe` into `tx` in [`PIPE_READ_CHUNK`] pieces until it closes.
///
/// A read error (other than an interrupted signal) counts as EOF: the waiter
/// cannot do anything with it beyond "nothing more is coming", and reporting it
/// would have to race the exit code for the caller's attention.
fn drain_pipe<R: Read>(stream: Stream, mut pipe: R, tx: mpsc::Sender<PipeMessage>) {
    let mut buffer = vec![0u8; PIPE_READ_CHUNK];
    loop {
        match pipe.read(&mut buffer) {
            // A signal that interrupted the read (SIGWINCH, …) says nothing
            // about the stream: retry rather than mistake it for EOF.
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Ok(0) | Err(_) => {
                let _ = tx.send(PipeMessage::Eof(stream));
                return;
            }
            Ok(read) => {
                if tx
                    .send(PipeMessage::Data(stream, buffer[..read].to_vec()))
                    .is_err()
                {
                    // The waiter is gone (timeout path): stop reading.
                    return;
                }
            }
        }
    }
}

/// Incremental capture of a running child's stdout and stderr.
///
/// Running [`CapturedPipes::pump`] from the wait loop keeps both pipes empty
/// enough that the child's `write` calls always return, while the running totals
/// stay bounded by [`MAX_CAPTURED_BYTES`].
struct CapturedPipes {
    rx: mpsc::Receiver<PipeMessage>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_eof: bool,
    stderr_eof: bool,
    readers: Vec<JoinHandle<()>>,
}

impl CapturedPipes {
    /// Take `child`'s pipes and start one reader thread per stream.
    ///
    /// A pipe the child does not have counts as already closed, which matches
    /// the empty string `wait_with_output` would have reported.
    fn start(child: &mut Child) -> Self {
        let stdout_eof = child.stdout.is_none();
        let stderr_eof = child.stderr.is_none();
        let (tx, rx) = mpsc::channel();
        // Boxed so both pipes share one tuple type (they are different structs).
        let pipes: [(Stream, Option<Box<dyn Read + Send>>); 2] = [
            (
                Stream::Stdout,
                child.stdout.take().map(|pipe| Box::new(pipe) as _),
            ),
            (
                Stream::Stderr,
                child.stderr.take().map(|pipe| Box::new(pipe) as _),
            ),
        ];
        let readers = pipes
            .into_iter()
            .filter_map(|(stream, pipe)| {
                let pipe = pipe?;
                let tx = tx.clone();
                Some(thread::spawn(move || drain_pipe(stream, pipe, tx)))
            })
            .collect();
        Self {
            rx,
            stdout: Vec::new(),
            stderr: Vec::new(),
            stdout_eof,
            stderr_eof,
            readers,
        }
    }

    /// Append every chunk that has arrived, without blocking.
    ///
    /// `Some(error)` once either total crosses [`MAX_CAPTURED_BYTES`]: the
    /// caller kills the child instead of buffering without bound. The check runs
    /// per chunk, so a burst is overshot by at most one [`PIPE_READ_CHUNK`].
    fn pump(&mut self) -> Option<String> {
        while let Ok(message) = self.rx.try_recv() {
            self.accept(message);
            if self.stdout.len() + self.stderr.len() > MAX_CAPTURED_BYTES {
                return Some(self.overflow_error());
            }
        }
        None
    }

    /// Have both reader threads reported EOF?
    ///
    /// True only once every chunk they read has also reached this side (the
    /// threads send their `Eof` *after* their last `Data`), so a closed pair of
    /// pipes means the capture is complete.
    fn closed(&self) -> bool {
        self.stdout_eof && self.stderr_eof
    }

    /// Exit code and both complete streams of a child that has exited.
    ///
    /// Both pipes are closed, so the reader threads are already returning; the
    /// joins only reap them (a reader that is gone can no longer be waiting on a
    /// pipe nobody will write to).
    fn streams(mut self, code: i32) -> (i32, String, String) {
        while let Some(reader) = self.readers.pop() {
            let _ = reader.join();
        }
        (code, decode(self.stdout), decode(self.stderr))
    }

    /// Add one chunk to the stream it came from.
    fn accept(&mut self, message: PipeMessage) {
        match message {
            PipeMessage::Data(Stream::Stdout, chunk) => self.stdout.extend_from_slice(&chunk),
            PipeMessage::Data(Stream::Stderr, chunk) => self.stderr.extend_from_slice(&chunk),
            PipeMessage::Eof(Stream::Stdout) => self.stdout_eof = true,
            PipeMessage::Eof(Stream::Stderr) => self.stderr_eof = true,
        }
    }

    /// The explicit failure for output past [`MAX_CAPTURED_BYTES`].
    fn overflow_error(&self) -> String {
        format!(
            "future skills: child wrote more than the {MAX_CAPTURED_BYTES}-byte capture limit \
             ({} bytes captured); killed it instead of buffering the rest",
            self.stdout.len() + self.stderr.len()
        )
    }
}

/// Kill a child and reap it.
fn terminate(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Exit code, or `-1` when the child was killed by a signal (Unix) or the OS
/// reports none.
fn status_code(status: &ExitStatus) -> i32 {
    status.code().unwrap_or(-1)
}

/// Lossy UTF-8 for captured output: one broken byte must not discard the rest.
fn decode(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Every call the injected runner saw: `(program, args)`.
    type Calls = Arc<Mutex<Vec<(PathBuf, Vec<String>)>>>;

    /// A runner that records its calls and always returns `result` — the only
    /// seam through which `list`/`run` reach a process, so the security tests
    /// can prove a rejected operation spawned nothing by asserting the log is
    /// empty.
    fn recording_runner(result: Result<(i32, String, String), String>) -> (SkillRunner, Calls) {
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&calls);
        let runner: SkillRunner = Box::new(move |program: &Path, args: &[String]| {
            sink.lock()
                .unwrap()
                .push((program.to_path_buf(), args.to_vec()));
            result.clone()
        });
        (runner, calls)
    }

    /// A CLI with an injected runner and a fixed program path.
    fn fake_cli(result: Result<(i32, String, String), String>) -> (SkillsCli, Calls) {
        let (runner, calls) = recording_runner(result);
        (
            SkillsCli::with_runner(runner, PathBuf::from("/opt/future")),
            calls,
        )
    }

    /// Calls recorded so far.
    fn recorded_calls(calls: &Calls) -> Vec<(PathBuf, Vec<String>)> {
        calls.lock().unwrap().clone()
    }

    /// A runner result that exits successfully with `stdout`.
    fn ok_outcome(stdout: &str) -> Result<(i32, String, String), String> {
        Ok((0, stdout.to_string(), String::new()))
    }

    fn install(id: &str, version: Option<&str>) -> SkillOp {
        SkillOp::Install {
            id: id.to_string(),
            version: version.map(str::to_string),
        }
    }

    fn uninstall(id: &str) -> SkillOp {
        SkillOp::Uninstall { id: id.to_string() }
    }

    /// An outcome shape for the `summarize_outcome` table (the `ok` flag
    /// follows the exit code, as the runner reports it).
    fn outcome_shape(
        op: SkillOp,
        exit_code: Option<i32>,
        stdout: &str,
        stderr: &str,
        timed_out: bool,
    ) -> SkillOpOutcome {
        SkillOpOutcome {
            op,
            ok: exit_code == Some(0),
            exit_code,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            timed_out,
        }
    }

    /// The `future skills list --json` document the CLI emits.
    const CATALOGUE_JSON: &str = concat!(
        r#"{"skills":[{"id":"future-web","name":"Web","latestVersion":"1.2","#,
        r#""installedVersion":"1.1","description":"Web search","descriptionZh":"网页搜索"},"#,
        r#"{"id":"future-paper","name":"Paper","latestVersion":null,"#,
        r#""installedVersion":null,"description":"","descriptionZh":""}],"count":2}"#
    );

    /// A `PATH` value holding `dirs`, with this host's separator.
    fn path_var(dirs: &[&str]) -> String {
        std::env::join_paths(dirs.iter().copied())
            .expect("join paths")
            .to_string_lossy()
            .into_owned()
    }

    // ── binary resolution ──────────────────────────────────────────

    #[test]
    fn candidates_prefer_the_current_exe_when_it_is_the_unified_binary() {
        let dirs = path_var(&["/a", "/b"]);
        let candidates = candidates_with_suffix(Path::new("/opt/tools/future"), Some(&dirs), "");
        assert_eq!(
            candidates,
            vec![
                PathBuf::from("/opt/tools/future"),
                PathBuf::from("/a/future"),
                PathBuf::from("/b/future"),
                PathBuf::from("future"),
            ]
        );
    }

    #[test]
    fn candidates_fall_back_to_the_sibling_then_to_path() {
        let dirs = path_var(&["/a", "/b"]);
        let candidates =
            candidates_with_suffix(Path::new("/opt/tools/future-tui"), Some(&dirs), "");
        assert_eq!(candidates.len(), 4);
        assert_eq!(candidates[0], PathBuf::from("/opt/tools/future"));
        assert_eq!(candidates[3], PathBuf::from("future"));
    }

    #[test]
    fn candidates_handle_a_windows_executable_suffix_and_a_cased_name() {
        let candidates =
            candidates_with_suffix(Path::new("/opt/tools/FUTURE.EXE"), Some("/a"), ".exe");
        assert_eq!(
            candidates,
            vec![
                PathBuf::from("/opt/tools/FUTURE.EXE"),
                PathBuf::from("/opt/tools/future.exe"),
                PathBuf::from("/a/future.exe"),
                PathBuf::from("future"),
            ]
        );
    }

    #[test]
    fn candidates_skip_empty_path_entries_and_absent_path() {
        let dirs = path_var(&["/a", "", "/b"]);
        let candidates = candidates_with_suffix(Path::new("/opt/future-tui"), Some(&dirs), "");
        assert!(candidates.contains(&PathBuf::from("/a/future")));
        assert!(candidates.contains(&PathBuf::from("/b/future")));
        let empty: Vec<&PathBuf> = candidates
            .iter()
            .filter(|candidate| candidate.as_os_str().is_empty())
            .collect();
        assert!(empty.is_empty());

        let expected = vec![PathBuf::from("/opt/future"), PathBuf::from("future")];
        assert_eq!(
            candidates_with_suffix(Path::new("/opt/future-tui"), None, ""),
            expected
        );
        assert_eq!(
            candidates_with_suffix(Path::new("/opt/future-tui"), Some(""), ""),
            expected
        );
        // An empty path has no parent, so only the bare name remains.
        assert_eq!(
            candidates_with_suffix(Path::new(""), None, ""),
            vec![PathBuf::from("future")]
        );
    }

    #[test]
    fn executable_name_strips_directories_and_the_exe_suffix() {
        assert_eq!(executable_name(Path::new("/opt/tools/future")), "future");
        assert_eq!(
            executable_name(Path::new("/opt/tools/FUTURE.EXE")),
            "future"
        );
        assert_eq!(executable_name(Path::new("future")), "future");
        assert_eq!(executable_name(Path::new("/opt/future-tui")), "future-tui");
        // No stem at all: never a panic, just an empty name.
        assert_eq!(executable_name(Path::new("/")), "");
        assert_eq!(executable_name(Path::new("..")), "");
    }

    #[test]
    fn pick_existing_prefers_a_real_file_and_otherwise_the_bare_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let real = dir.path().join("future");
        std::fs::write(&real, "#!/bin/sh\n").expect("write");
        let missing = dir.path().join("nope").join("future");
        assert_eq!(pick_existing(&[missing.clone(), real.clone()]), real);
        assert_eq!(pick_existing(&[missing]), PathBuf::from(FUTURE_BINARY));
        assert_eq!(pick_existing(&[]), PathBuf::from(FUTURE_BINARY));
        // A directory named `future` is not an executable.
        let dir_named_future = dir.path().join("dir-future");
        std::fs::create_dir(&dir_named_future).expect("mkdir");
        assert_eq!(
            pick_existing(&[dir_named_future]),
            PathBuf::from(FUTURE_BINARY)
        );
    }

    #[test]
    fn skills_cli_resolves_a_future_binary_and_honors_an_injected_program() {
        let resolved = SkillsCli::new();
        let name = resolved
            .program()
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        assert!(name.starts_with("future"), "{name}");
        assert_eq!(
            SkillsCli::default().program().file_name(),
            resolved.program().file_name()
        );
        let injected = SkillsCli::with_runner(
            recording_runner(ok_outcome("")).0,
            PathBuf::from("/x/future"),
        );
        assert_eq!(injected.program(), Path::new("/x/future"));
    }

    // ── catalogue parsing ──────────────────────────────────────────

    #[test]
    fn parse_catalogue_reads_the_cli_json_shape() {
        let catalogue = parse_catalogue(CATALOGUE_JSON).expect("parse");
        assert_eq!(catalogue.entries.len(), 2);
        assert_eq!(
            catalogue.entries[0],
            SkillCatalogueEntry {
                id: "future-web".to_string(),
                name: Some("Web".to_string()),
                summary: Some("Web search".to_string()),
                summary_zh: Some("网页搜索".to_string()),
                latest_version: Some("1.2".to_string()),
                installed_version: Some("1.1".to_string()),
            }
        );
        // `null` and `""` both mean "nothing to show".
        assert_eq!(catalogue.entries[1].latest_version, None);
        assert_eq!(catalogue.entries[1].installed_version, None);
        assert_eq!(catalogue.entries[1].summary, None);
        assert_eq!(catalogue.entries[1].summary_zh, None);
        assert_eq!(catalogue.entries[1].name, Some("Paper".to_string()));
    }

    #[test]
    fn parse_catalogue_tolerates_noise_around_the_document() {
        let noisy =
            format!("Fetching skill catalog from https://example...\n{CATALOGUE_JSON}\nDone.\n");
        let catalogue = parse_catalogue(&noisy).expect("parse");
        assert_eq!(catalogue.entries.len(), 2);
        assert_eq!(catalogue.entries[0].id, "future-web");
    }

    #[test]
    fn parse_catalogue_accepts_alias_fields_and_ignores_extras() {
        let json = r#"{"skills":[{"id":"future-x","summary":"s","summaryZh":"译",
            "latest_version":"2.0","installed_version":"1.0","category":"c","price":"p"}],
            "count":1,"generatedAt":"now"}"#;
        let catalogue = parse_catalogue(json).expect("parse");
        assert_eq!(catalogue.entries.len(), 1);
        let entry = &catalogue.entries[0];
        assert_eq!(entry.summary.as_deref(), Some("s"));
        assert_eq!(entry.summary_zh.as_deref(), Some("译"));
        assert_eq!(entry.latest_version.as_deref(), Some("2.0"));
        assert_eq!(entry.installed_version.as_deref(), Some("1.0"));
        assert_eq!(entry.name, None);
    }

    #[test]
    fn parse_catalogue_rejects_output_without_a_json_object() {
        for stdout in [
            "",
            "   ",
            "No skills available.\n",
            "null",
            "}{",
            "failure: }",
        ] {
            let result = parse_catalogue(stdout);
            assert!(result.is_err(), "{stdout:?}");
        }
        let message = parse_catalogue("No skills available.\n").expect_err("no object");
        assert!(message.contains("no JSON object"), "{message}");
        assert!(message.contains("No skills available."), "{message}");
    }

    #[test]
    fn parse_catalogue_rejects_broken_json_and_a_non_array_skills_field() {
        let broken = parse_catalogue("{oops}").expect_err("broken");
        assert!(broken.contains("invalid skills catalogue JSON"), "{broken}");
        let not_an_array = parse_catalogue(r#"{"skills":1}"#).expect_err("not an array");
        assert!(not_an_array.contains("is not an array"), "{not_an_array}");
        assert!(parse_catalogue(r#"{"skills":"x"}"#).is_err());
    }

    #[test]
    fn parse_catalogue_treats_missing_or_null_skills_as_empty() {
        assert!(parse_catalogue("{}").expect("parse").entries.is_empty());
        assert!(parse_catalogue(r#"{"skills":null}"#)
            .expect("parse")
            .entries
            .is_empty());
        assert!(parse_catalogue(r#"{"skills":[],"count":0}"#)
            .expect("parse")
            .entries
            .is_empty());
    }

    #[test]
    fn parse_catalogue_skips_entries_without_a_usable_id() {
        let json = r#"{"skills":[{"name":"no id"},{"id":null},{"id":""},{"id":7},{"id":"keep"}]}"#;
        let catalogue = parse_catalogue(json).expect("parse");
        assert_eq!(catalogue.entries.len(), 1);
        assert_eq!(catalogue.entries[0].id, "keep");
    }

    #[test]
    fn parse_catalogue_caps_the_excerpt_in_its_error() {
        let message = parse_catalogue(&"x".repeat(500)).expect_err("no object");
        assert!(message.contains('…'), "{message}");
        assert!(message.len() < 300);
    }

    // ── list ───────────────────────────────────────────────────────

    #[test]
    fn list_args_is_the_json_subcommand() {
        assert_eq!(list_args(), ["skills", "list", "--json"]);
    }

    /// Browsing is the call that gets the short budget: it only reads a
    /// catalogue, and the panel can retry it — waiting two minutes for it is
    /// what made `/skills` look hung.
    #[test]
    fn browsing_gets_the_short_timeout_and_writing_the_long_one() {
        assert!(SKILL_LIST_TIMEOUT < SKILL_OP_TIMEOUT);
        assert_eq!(timeout_for_args(&list_args()), SKILL_LIST_TIMEOUT);
        // A flag added to the browse call keeps the browse budget…
        let mut with_flag = list_args();
        with_flag.push("--all".to_string());
        assert_eq!(timeout_for_args(&with_flag), SKILL_LIST_TIMEOUT);
        // …while every mutating operation keeps the write budget.
        for op in [
            SkillOp::Install {
                id: "alpha".to_string(),
                version: None,
            },
            SkillOp::Uninstall {
                id: "alpha".to_string(),
            },
            SkillOp::UpdateAll,
        ] {
            assert_eq!(timeout_for_args(&op_args(&op)), SKILL_OP_TIMEOUT);
        }
        // Not a browse call: an empty or truncated argv is not `skills list`.
        assert_eq!(timeout_for_args(&[]), SKILL_OP_TIMEOUT);
        assert_eq!(timeout_for_args(&["skills".to_string()]), SKILL_OP_TIMEOUT);
    }

    #[test]
    fn list_runs_the_json_subcommand_and_parses_the_catalogue() {
        let (cli, calls) = fake_cli(ok_outcome(CATALOGUE_JSON));
        let catalogue = cli.list().expect("list");
        assert_eq!(catalogue.entries.len(), 2);
        assert_eq!(catalogue.entries[0].id, "future-web");
        assert_eq!(
            recorded_calls(&calls),
            vec![(PathBuf::from("/opt/future"), list_args())]
        );
    }

    #[test]
    fn list_reports_a_non_zero_exit_with_the_stderr_excerpt() {
        let (cli, _) = fake_cli(Ok((
            1,
            String::new(),
            "Failed to fetch skills\n  no network\n".to_string(),
        )));
        let message = cli.list().expect_err("non-zero exit");
        assert!(message.contains("exited with status 1"), "{message}");
        assert!(
            message.contains("Failed to fetch skills no network"),
            "{message}"
        );
    }

    #[test]
    fn list_reports_a_failure_without_any_stderr() {
        let (cli, _) = fake_cli(Ok((2, String::new(), String::new())));
        let message = cli.list().expect_err("non-zero exit");
        assert_eq!(message, "`future skills list --json` exited with status 2");
    }

    #[test]
    fn list_reports_a_runner_error() {
        let (cli, _) = fake_cli(Err("failed to spawn /opt/future: no such file".to_string()));
        let message = cli.list().expect_err("spawn failure");
        assert!(
            message.contains("failed to run `future skills list --json`"),
            "{message}"
        );
        assert!(message.contains("no such file"), "{message}");

        // A timeout is a runner error too, and its message says why.
        let (cli, _) = fake_cli(Err(format!("{TIMEOUT_MARKER} after 120s")));
        assert!(cli.list().expect_err("timeout").contains(TIMEOUT_MARKER));
    }

    #[test]
    fn list_reports_a_parse_failure_with_the_stderr_excerpt() {
        let (cli, _) = fake_cli(Ok((
            0,
            "No skills available.".to_string(),
            "warning: stale cache\n".to_string(),
        )));
        let message = cli.list().expect_err("unparseable output");
        assert!(message.contains("no JSON object"), "{message}");
        assert!(message.contains("warning: stale cache"), "{message}");
    }

    // ── run ────────────────────────────────────────────────────────

    #[test]
    fn op_args_builds_the_cli_command_lines() {
        assert_eq!(
            op_args(&install("future-web", None)),
            ["skills", "install", "future-web"]
        );
        assert_eq!(
            op_args(&install("future-web", Some("1.2"))),
            ["skills", "install", "future-web", "--version", "1.2"]
        );
        assert_eq!(
            op_args(&uninstall("future-web")),
            ["skills", "uninstall", "future-web"]
        );
        assert_eq!(op_args(&SkillOp::UpdateAll), ["skills", "update"]);
    }

    #[test]
    fn run_install_maps_a_zero_exit_to_success() {
        let (cli, calls) = fake_cli(Ok((0, "Installed future-web\n".to_string(), String::new())));
        let outcome = cli.run(&install("future-web", Some("1.2")));
        assert_eq!(
            outcome,
            SkillOpOutcome {
                op: install("future-web", Some("1.2")),
                ok: true,
                exit_code: Some(0),
                stdout: "Installed future-web\n".to_string(),
                stderr: String::new(),
                timed_out: false,
            }
        );
        assert_eq!(
            recorded_calls(&calls),
            vec![(
                PathBuf::from("/opt/future"),
                op_args(&install("future-web", Some("1.2")))
            )]
        );
    }

    #[test]
    fn run_update_all_and_an_unversioned_install_use_their_commands() {
        let (cli, calls) = fake_cli(Ok((0, "Updated 1 skill(s)\n".to_string(), String::new())));
        assert!(cli.run(&SkillOp::UpdateAll).ok);
        assert!(cli.run(&install("future-web", None)).ok);
        let recorded = recorded_calls(&calls);
        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[0].1, op_args(&SkillOp::UpdateAll));
        assert_eq!(recorded[1].1, op_args(&install("future-web", None)));
    }

    #[test]
    fn run_reports_a_non_zero_exit_as_a_failure() {
        let (cli, _) = fake_cli(Ok((
            1,
            String::new(),
            "Skill \"ghost\" is not installed.".to_string(),
        )));
        let outcome = cli.run(&uninstall("ghost"));
        assert!(!outcome.ok);
        assert_eq!(outcome.exit_code, Some(1));
        assert_eq!(outcome.stderr, "Skill \"ghost\" is not installed.");
        assert!(!outcome.timed_out);
    }

    #[test]
    fn run_reports_a_timeout() {
        let (cli, _) = fake_cli(Err(format!("{TIMEOUT_MARKER} after 120s")));
        let outcome = cli.run(&install("future-web", None));
        assert!(!outcome.ok);
        assert!(outcome.timed_out);
        assert_eq!(outcome.exit_code, None);
        assert!(outcome.stdout.is_empty());
        assert!(outcome.stderr.contains("timed out"), "{}", outcome.stderr);
    }

    #[test]
    fn run_reports_a_spawn_failure_as_a_non_timeout_failure() {
        let (cli, _) = fake_cli(Err("failed to spawn /opt/future: ENOENT".to_string()));
        let outcome = cli.run(&SkillOp::UpdateAll);
        assert!(!outcome.ok);
        assert!(!outcome.timed_out);
        assert_eq!(outcome.exit_code, None);
        assert_eq!(outcome.stderr, "failed to spawn /opt/future: ENOENT");
    }

    #[test]
    fn run_rejects_unsafe_ids_before_spawning() {
        let (cli, calls) = fake_cli(ok_outcome("this must never be reached"));
        for bad in [
            "",
            ".",
            "..",
            "../sessions",
            "future-x/../../sessions",
            "/tmp/evil",
            "C:\\Users\\evil",
            "..\\sessions",
            "a/b",
            "a\\b",
            "a b",
            "a\tb",
            "a\nb",
            "CON",
            "nul.txt",
            "LPT1",
            "COM9.log",
            "trailing.",
            "café",
            "a'b",
            "a*b",
        ] {
            let outcome = cli.run(&install(bad, None));
            assert!(!outcome.ok, "{bad:?}");
            assert_eq!(outcome.exit_code, None, "{bad:?}");
            assert!(!outcome.timed_out, "{bad:?}");
            assert!(outcome.stdout.is_empty(), "{bad:?}");
            let reason = outcome.stderr;
            assert!(reason.starts_with("invalid skill id: "), "{bad:?} {reason}");
            assert!(!cli.run(&uninstall(bad)).ok, "{bad:?}");
        }
        let too_long = cli.run(&install(&"a".repeat(129), None));
        assert!(too_long.stderr.contains("max 128"), "{}", too_long.stderr);
        // The security boundary: not one of those reached the runner.
        assert!(recorded_calls(&calls).is_empty());
    }

    #[test]
    fn run_rejects_an_unsafe_version_before_spawning() {
        let (cli, calls) = fake_cli(ok_outcome("this must never be reached"));
        let rejected = cli.run(&install("future-web", Some("../1.0")));
        assert!(!rejected.ok);
        assert_eq!(rejected.exit_code, None);
        assert!(
            rejected.stderr.starts_with("invalid skill version: "),
            "{}",
            rejected.stderr
        );
        for bad in ["", "1 0", "v/1", "1.0.", "COM1", ".."] {
            assert!(!cli.run(&install("future-web", Some(bad))).ok, "{bad:?}");
        }
        assert!(recorded_calls(&calls).is_empty());
    }

    #[test]
    fn run_accepts_a_prerelease_version() {
        let (cli, calls) = fake_cli(ok_outcome("Installed\n"));
        assert!(cli.run(&install("future-web", Some("1.0.0-rc.1"))).ok);
        assert_eq!(
            recorded_calls(&calls)[0].1,
            op_args(&install("future-web", Some("1.0.0-rc.1")))
        );
    }

    // ── summaries ──────────────────────────────────────────────────

    #[test]
    fn summarize_outcome_covers_every_shape_on_one_line() {
        let op = install("future-web", Some("1.2"));
        let cases = vec![
            (
                outcome_shape(op.clone(), Some(0), "done\n", "", false),
                "install future-web@1.2 succeeded",
            ),
            (
                outcome_shape(
                    op.clone(),
                    None,
                    "",
                    "future skills: timed out after 120s",
                    true,
                ),
                "install future-web@1.2 timed out",
            ),
            (
                outcome_shape(op.clone(), Some(1), "", "boom\n", false),
                "install future-web@1.2 failed (exit code 1): boom",
            ),
            (
                outcome_shape(op.clone(), Some(3), "", "", false),
                "install future-web@1.2 failed (exit code 3)",
            ),
            (
                outcome_shape(op.clone(), None, "", "failed to spawn", false),
                "install future-web@1.2 failed: failed to spawn",
            ),
            (
                outcome_shape(op.clone(), None, "half way\n", "", false),
                "install future-web@1.2 failed: half way",
            ),
            (
                outcome_shape(op.clone(), None, "", "", false),
                "install future-web@1.2 failed",
            ),
            (
                outcome_shape(install("future-web", None), Some(0), "", "", false),
                "install future-web succeeded",
            ),
            (
                outcome_shape(uninstall("ghost"), Some(1), "", "", false),
                "uninstall ghost failed (exit code 1)",
            ),
            (
                outcome_shape(SkillOp::UpdateAll, Some(0), "", "", false),
                "update all skills succeeded",
            ),
        ];
        for (outcome, expected) in cases {
            let summary = summarize_outcome(&outcome);
            assert_eq!(summary, expected);
            assert!(!summary.contains('\n'), "{summary}");
        }
    }

    #[test]
    fn output_excerpt_flattens_shortens_and_blanks() {
        assert_eq!(output_excerpt(""), "");
        assert_eq!(output_excerpt("  \n\t "), "");
        assert_eq!(output_excerpt("  hello \n world \n"), "hello world");
        let long = output_excerpt(&"x".repeat(EXCERPT_MAX_CHARS + 50));
        assert_eq!(long.chars().count(), EXCERPT_MAX_CHARS + 1);
        assert!(long.ends_with('…'), "{long}");
    }

    #[test]
    fn stderr_suffix_only_mentions_a_non_empty_stderr() {
        assert_eq!(stderr_suffix(""), "");
        assert_eq!(stderr_suffix("  \n"), "");
        assert_eq!(stderr_suffix("boom\n"), "; stderr: boom");
    }

    // ── validation ─────────────────────────────────────────────────

    #[test]
    fn validate_skill_id_accepts_plain_slugs() {
        let longest = "a".repeat(128);
        for good in [
            "future-web",
            "skill_name",
            "a",
            "a.b.c",
            "0",
            "x-1_2.3",
            longest.as_str(),
        ] {
            assert!(validate_skill_id(good).is_ok(), "{good:?}");
        }
    }

    #[test]
    fn validate_skill_id_explains_every_rejection() {
        assert_eq!(validate_skill_id("").unwrap_err(), "skill id is empty");
        let long = validate_skill_id(&"a".repeat(129)).unwrap_err();
        assert!(long.contains("129 bytes (max 128)"), "{long}");
        let traversal = validate_skill_id("../sessions").unwrap_err();
        assert!(traversal.contains("must not contain `..`"), "{traversal}");
        let trailing = validate_skill_id("trailing.").unwrap_err();
        assert!(trailing.contains("end with a dot"), "{trailing}");
        let device = validate_skill_id("nul.txt").unwrap_err();
        assert!(device.contains("reserved device name"), "{device}");
        let charset = validate_skill_id("a b").unwrap_err();
        assert!(
            charset.contains("may only contain ASCII letters"),
            "{charset}"
        );
        for bad in [
            "..",
            ".",
            "/tmp/evil",
            "C:\\evil",
            "a\\b",
            "..\\x",
            "a/b",
            "a\tb",
            "\u{7}",
            "café",
            "a'b",
            "a*b",
        ] {
            let result = validate_skill_id(bad);
            assert!(result.is_err(), "{bad:?}");
        }
    }

    #[test]
    fn reserved_device_stems_are_recognised_case_insensitively() {
        for reserved in ["CON", "con", "Con", "PRN", "AUX", "NUL", "COM1", "lpt9"] {
            assert!(is_reserved_stem(reserved), "{reserved}");
        }
        for allowed in [
            "COM",
            "COM0",
            "COM10",
            "LPT",
            "CONSOLE",
            "COMX",
            "future-web",
        ] {
            assert!(!is_reserved_stem(allowed), "{allowed}");
        }
    }

    #[test]
    fn component_bytes_are_ascii_alphanumeric_or_a_connector() {
        for byte in *b"aZ0._-" {
            assert!(is_skill_component_byte(byte), "{byte}");
        }
        for byte in *b" /\\\t\x07@\x80" {
            assert!(!is_skill_component_byte(byte), "{byte}");
        }
    }

    // ── upgradable ─────────────────────────────────────────────────

    fn entry(id: &str, installed: Option<&str>, latest: Option<&str>) -> SkillCatalogueEntry {
        SkillCatalogueEntry {
            id: id.to_string(),
            name: None,
            summary: None,
            summary_zh: None,
            latest_version: latest.map(str::to_string),
            installed_version: installed.map(str::to_string),
        }
    }

    fn catalogue_of(entries: Vec<SkillCatalogueEntry>) -> SkillCatalogue {
        SkillCatalogue { entries }
    }

    /// Ids of the entries `upgradable` selected, in catalogue order.
    fn upgradable_ids(catalogue: &SkillCatalogue) -> Vec<&str> {
        upgradable(catalogue)
            .iter()
            .map(|entry| entry.id.as_str())
            .collect()
    }

    #[test]
    fn upgradable_flags_only_strictly_older_installed_versions() {
        let catalogue = catalogue_of(vec![
            entry("older", Some("1.0"), Some("1.1")),
            entry("numeric", Some("1.9"), Some("1.10")),
            entry("equal", Some("2.0"), Some("2.0")),
            entry("newer", Some("2.0"), Some("1.9")),
            entry("prefixed", Some("v1.0"), Some("1.1")),
            entry("prerelease", Some("1.0.0-rc.1"), Some("1.0.0")),
            entry("build-metadata", Some("1.0.0+one"), Some("1.0.0+two")),
            entry("shortened", Some("1"), Some("1.0.1")),
        ]);
        assert_eq!(
            upgradable_ids(&catalogue),
            ["older", "numeric", "prefixed", "prerelease", "shortened"]
        );
    }

    #[test]
    fn upgradable_ignores_missing_and_unparseable_versions() {
        let catalogue = catalogue_of(vec![
            entry("no-latest", Some("1.0"), None),
            entry("no-installed", None, Some("1.0")),
            entry("both-missing", None, None),
            entry("unparseable-latest", Some("1.0"), Some("nightly")),
            entry("unparseable-installed", Some("nightly"), Some("1.0")),
            entry("both-unparseable", Some("nightly"), Some("latest")),
            entry("ok", Some("1.0"), Some("1.0.1")),
        ]);
        assert_eq!(upgradable_ids(&catalogue), ["ok"]);
        assert!(upgradable(&catalogue_of(vec![])).is_empty());
    }

    #[test]
    fn parse_version_accepts_a_leading_v_and_rejects_junk() {
        assert_eq!(
            parse_version(" 1.2.3 "),
            Some(semver::Version::new(1, 2, 3))
        );
        assert_eq!(parse_version("v1.2.3"), Some(semver::Version::new(1, 2, 3)));
        assert_eq!(parse_version("V2.0.0"), Some(semver::Version::new(2, 0, 0)));
        assert_eq!(parse_version("1.9"), Some(semver::Version::new(1, 9, 0)));
        assert_eq!(parse_version("2"), Some(semver::Version::new(2, 0, 0)));
        assert_eq!(
            parse_version("1.2-alpha"),
            Some(semver::Version::parse("1.2.0-alpha").expect("semver"))
        );
        assert_eq!(parse_version("nightly"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("v"), None);
    }

    #[test]
    fn pad_version_completes_the_numeric_core_only() {
        assert_eq!(pad_version("1"), "1.0.0");
        assert_eq!(pad_version("1.2"), "1.2.0");
        assert_eq!(pad_version("1.2.3"), "1.2.3");
        assert_eq!(pad_version("1.2.3.4"), "1.2.3.4");
        assert_eq!(pad_version("1.2+build"), "1.2.0+build");
        assert_eq!(pad_version("1.2.3+meta"), "1.2.3+meta");
        assert_eq!(pad_version(""), ".0.0");
    }

    // ── real runner (only ever a hermetic `sh`, never `future`) ─────

    /// `sh -c <script>` argv.
    #[cfg(unix)]
    fn sh_args(script: &str) -> Vec<String> {
        vec!["-c".to_string(), script.to_string()]
    }

    #[cfg(unix)]
    #[test]
    fn spawn_runner_captures_streams_and_the_exit_code() {
        let args = sh_args("printf out; printf err >&2; exit 3");
        let (code, stdout, stderr) = spawn_runner(Path::new("sh"), &args).expect("run sh");
        assert_eq!((code, stdout.as_str(), stderr.as_str()), (3, "out", "err"));
        let (code, stdout, stderr) =
            spawn_runner(Path::new("sh"), &sh_args("exit 0")).expect("run sh");
        assert_eq!((code, stdout.as_str(), stderr.as_str()), (0, "", ""));
    }

    #[cfg(unix)]
    #[test]
    fn spawn_runner_reports_a_spawn_failure() {
        let error = spawn_runner(Path::new("future-tui-no-such-command"), &[])
            .expect_err("spawn must fail");
        assert!(
            error.contains("failed to spawn future-tui-no-such-command"),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn spawn_runner_reports_a_signal_death_as_minus_one() {
        let (code, stdout, stderr) = spawn_with_timeout(
            Path::new("sh"),
            &sh_args("kill -TERM $$"),
            Duration::from_secs(10),
        )
        .expect("run sh");
        assert_eq!((code, stdout.as_str(), stderr.as_str()), (-1, "", ""));
    }

    #[cfg(unix)]
    #[test]
    fn wait_for_kills_a_child_that_outlives_the_timeout() {
        let started = Instant::now();
        let error = spawn_with_timeout(
            Path::new("sh"),
            &sh_args("sleep 30"),
            Duration::from_millis(150),
        )
        .expect_err("the child must be killed");
        assert!(error.starts_with(TIMEOUT_MARKER), "{error}");
        // Killed, not waited out: the call returns long before `sleep 30` ends.
        let elapsed = started.elapsed();
        assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
    }

    // ── the pipe-buffer regression (real children, real pipes) ──────

    /// `sh -c 'yes x | head -c N'`: exactly `N` bytes on stdout, exit 0.
    ///
    /// The pipeline is the deadlock reproducer — `yes` keeps writing until it
    /// has produced `N` bytes, so as soon as the OS pipe buffer (~64 KiB) fills
    /// up it blocks in `write` until somebody reads. Nothing here is `future`, so
    /// these tests can never touch the user's skill catalogue.
    ///
    /// Shell-only, hence `cfg(unix)`: Windows has no `sh`. The code under test is
    /// platform-neutral (`Read` on the same `ChildStdout`/`ChildStderr` handles on
    /// every OS), so Unix coverage is representative and a Windows skip cannot
    /// hide a platform-specific deadlock.
    #[cfg(unix)]
    fn bytes_on_stdout(bytes: usize) -> Vec<String> {
        sh_args(&format!("yes x | head -c {bytes}"))
    }

    #[cfg(unix)]
    #[test]
    fn wait_for_drains_stdout_larger_than_the_pipe_buffer() {
        // 200_000 > 65_536 (the macOS/Linux pipe capacity): while nothing drained
        // the pipe, this blocked in `write` and came back as a timeout although
        // the child had answered immediately — the `/skills` failure on a machine
        // whose catalogue is ~81 KiB.
        let bytes = 200_000;
        let (code, stdout, stderr) = spawn_with_timeout(
            Path::new("sh"),
            &bytes_on_stdout(bytes),
            Duration::from_secs(10),
        )
        .expect("a chatty child must not time out");
        assert_eq!(code, 0);
        assert_eq!(stdout.len(), bytes, "every byte must be captured");
        let intact = stdout.bytes().all(|byte| byte == b'x' || byte == b'\n');
        assert!(intact, "the payload must arrive in order");
        assert_eq!(stderr, "");
    }

    #[cfg(unix)]
    #[test]
    fn wait_for_drains_stderr_larger_than_the_pipe_buffer() {
        let bytes = 200_000;
        let (code, stdout, stderr) = spawn_with_timeout(
            Path::new("sh"),
            &sh_args(&format!("yes x | head -c {bytes} >&2")),
            Duration::from_secs(10),
        )
        .expect("a chatty child must not time out");
        assert_eq!((code, stdout.as_str()), (0, ""));
        assert_eq!(stderr.len(), bytes, "every byte must be captured");
    }

    #[cfg(unix)]
    #[test]
    fn wait_for_drains_output_on_both_sides_of_the_pipe_buffer() {
        // The boundary and one byte past it: reading only after exit happens to
        // work up to the buffer size and deadlocks from the first byte over.
        for bytes in [65_536usize, 65_537] {
            let result = spawn_with_timeout(
                Path::new("sh"),
                &bytes_on_stdout(bytes),
                Duration::from_secs(10),
            );
            let (code, stdout, _) =
                result.unwrap_or_else(|error| panic!("{bytes} bytes timed out: {error}"));
            assert_eq!((code, stdout.len()), (0, bytes), "at {bytes} bytes");
        }
    }

    #[cfg(unix)]
    #[test]
    fn wait_for_still_times_out_after_a_big_write() {
        // Draining must not move the deadline: a child that writes past the
        // buffer and then hangs is still killed, and the kill path does not wait
        // on readers that a surviving pipeline could keep alive.
        let started = Instant::now();
        let error = spawn_with_timeout(
            Path::new("sh"),
            &sh_args("yes x | head -c 200000; sleep 5"),
            Duration::from_millis(300),
        )
        .expect_err("the child must be killed");
        assert!(error.starts_with(TIMEOUT_MARKER), "{error}");
        let elapsed = started.elapsed();
        assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
    }

    #[cfg(unix)]
    #[test]
    fn wait_for_fails_explicitly_when_a_child_floods_the_capture_limit() {
        // 20 MiB > MAX_CAPTURED_BYTES: capture stops at the ceiling and says so,
        // instead of letting the TUI's memory follow the child or handing back a
        // truncated document that would still look parseable.
        let started = Instant::now();
        let error = spawn_with_timeout(
            Path::new("sh"),
            &bytes_on_stdout(MAX_CAPTURED_BYTES + 4 * 1024 * 1024),
            Duration::from_secs(30),
        )
        .expect_err("past the capture limit the call must fail");
        assert!(error.contains("capture limit"), "{error}");
        let not_a_timeout = !error.starts_with(TIMEOUT_MARKER);
        assert!(not_a_timeout, "the ceiling is not a timeout: {error}");
        // Killed at the ceiling, not waited out for the whole 30 s budget.
        let elapsed = started.elapsed();
        assert!(elapsed < Duration::from_secs(15), "{elapsed:?}");
    }

    #[cfg(unix)]
    #[test]
    fn wait_for_reports_a_pipe_a_descendant_kept_open() {
        // The child exits at once, but the shell's background `sleep` inherits the
        // write end (deliberately not redirected), so stdout never reaches EOF.
        // The wait must end at the deadline with its own message: not the timeout
        // marker (the child did exit), and not a hang until the grandchild goes.
        let error = spawn_with_timeout(
            Path::new("sh"),
            &sh_args("sleep 5 & exit 0"),
            Duration::from_millis(300),
        )
        .expect_err("an unclosed pipe must not look like success");
        assert!(error.contains("descendant"), "{error}");
        let not_a_timeout = !error.starts_with(TIMEOUT_MARKER);
        assert!(not_a_timeout, "the child had already exited: {error}");
    }

    /// A `Read` that reports `Interrupted` once before handing over `payload`.
    struct InterruptOnce {
        payload: &'static [u8],
        interrupted: bool,
    }

    impl Read for InterruptOnce {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if !self.interrupted {
                self.interrupted = true;
                return Err(std::io::Error::from(ErrorKind::Interrupted));
            }
            let read = self.payload.len().min(buffer.len());
            buffer[..read].copy_from_slice(&self.payload[..read]);
            self.payload = &self.payload[read..];
            Ok(read)
        }
    }

    #[test]
    fn drain_pipe_retries_a_signal_interrupted_read() {
        // EINTR says nothing about the stream: treating it as EOF would silently
        // truncate the output of a child a signal happened to hit mid-read.
        let (tx, rx) = mpsc::channel();
        drain_pipe(
            Stream::Stdout,
            InterruptOnce {
                payload: b"payload",
                interrupted: false,
            },
            tx,
        );
        let mut captured = Vec::new();
        let mut eof = false;
        for message in rx.iter() {
            match message {
                PipeMessage::Data(_, chunk) => captured.extend_from_slice(&chunk),
                PipeMessage::Eof(_) => eof = true,
            }
        }
        assert_eq!(captured, b"payload");
        assert!(eof, "the reader must close the stream");
    }

    /// A `Read` that hands out `chunks` more bufferfuls of `x`, counting calls,
    /// until it reports EOF.
    struct ChunkCounter {
        chunks: usize,
        reads: usize,
    }

    impl Read for ChunkCounter {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.reads += 1;
            self.chunks = self.chunks.saturating_sub(1);
            let read = if self.chunks == 0 { 0 } else { buffer.len() };
            buffer[..read].fill(b'x');
            Ok(read)
        }
    }

    #[test]
    fn drain_pipe_stops_reading_once_the_waiter_is_gone() {
        // The waiter drops its end when it gives up (timeout, capture ceiling).
        // A reader that kept draining would go on feeding a runaway child and
        // copying into a channel nobody reads: it must stop at the first failed
        // send, not after the 16 chunks this reader still has to offer.
        let (tx, rx) = mpsc::channel();
        drop(rx);
        let mut reader = ChunkCounter {
            chunks: 16,
            reads: 0,
        };
        drain_pipe(Stream::Stdout, &mut reader, tx);
        assert_eq!(reader.reads, 1, "one read, then the failed send ends it");
    }
}
