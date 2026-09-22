//! `/worktree` — run (or continue) a conversation inside a git worktree.
//!
//! This repository's own workflow is "one worktree per unit of work"
//! (`CLAUDE.md`), and the TUI had no way to get there: the user had to leave
//! the TUI, run `git worktree add` in a shell and come back to `/cwd <path>`.
//! `/worktree` closes that loop **in the TUI layer only** — it *adds* a
//! worktree and moves the session's working directory through the existing
//! `set_cwd` RPC, so no agent change is involved.
//!
//! Design notes:
//!
//! * **Read-and-add only.** The invocations this module can build are
//!   `worktree list --porcelain`, `status --porcelain`,
//!   `rev-parse --abbrev-ref HEAD` and `worktree add`. [`ensure_read_only`]
//!   refuses `remove`/`prune`/`reset`/`clean`/`checkout`/`gc`/`reflog`
//!   structurally, in one place: this repository has concurrent sessions in
//!   sibling worktrees, and a UI that can delete one is a footgun. The guard
//!   looks at the *subcommand* position only, so a branch named `clean` is
//!   still creatable.
//! * **The main worktree is the root.** New worktrees go to
//!   `<main>/<WORKTREES_DIR>/<name>`, mirroring this repository's own
//!   `.worktrees/<name>` layout. `git worktree list` prints the main worktree
//!   first, so a session that already sits inside `.worktrees/x` creates
//!   siblings of `x` rather than nesting under it.
//! * **The name is the validation boundary.** [`validate_worktree_name`]
//!   refuses anything outside `[A-Za-z0-9._/-]`, any whitespace or control
//!   character, `..`, a segment starting with `.` or `-` (git would read the
//!   latter as an option) and a `.lock` segment — *before* a process is
//!   spawned. The tests prove a rejected name reaches no process.
//! * **Every process boundary is injectable** ([`GitCli::with_runner`]), so the
//!   unit tests never touch the real git; the integration test does, in a
//!   throwaway repository under the temp dir.
//! * **A hung or chatty child cannot hang the TUI.** Invocations go through
//!   `crate::skills_cli::spawn_with_timeout`, the shared spawn helper that
//!   drains both pipes while the child runs and kills *and reaps* a child that
//!   outlives its budget ([`GIT_QUERY_TIMEOUT`] for reads, [`GIT_ADD_TIMEOUT`]
//!   for `worktree add`, which may check out a whole tree). The app runs every
//!   call on the blocking pool.
//! * **No rendering here.** This module decides *what* to run and parses what
//!   came back; the picker is `crate::components::worktree_view`.
//!
//! Write-set deviation (recorded in `.future/tui-parity/handoffs/worktree.md`):
//! `skills_cli::spawn_with_timeout` is `pub(crate)` — a visibility change only —
//! so this module reuses the tested drain/timeout machinery instead of growing
//! a second copy of it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::skills_cli::spawn_with_timeout;

/// Directory, relative to the repository's main worktree, that new worktrees
/// are created in (this repository's own `.worktrees/<name>` convention).
pub const WORKTREES_DIR: &str = ".worktrees";

/// The usage line `/worktree` prints for anything it cannot parse.
pub const WORKTREE_USAGE: &str = "Usage: /worktree [new <branch>]";

/// The `git` program: no path, so `PATH` decides which git runs.
pub const GIT_PROGRAM: &str = "git";

/// Budget for one read-only git query (`worktree list`, `status`,
/// `rev-parse`). All three only read the index and the refs.
pub const GIT_QUERY_TIMEOUT: Duration = Duration::from_secs(10);

/// Budget for `git worktree add`. It writes a whole checkout, and on a
/// network-mounted repository that is slow rather than broken.
pub const GIT_ADD_TIMEOUT: Duration = Duration::from_secs(60);

/// How many worktrees one probe asks `git status` for.
///
/// Listing is one call for the whole repository; the dirty flag costs one call
/// per worktree, so a machine with dozens of them pays `n × status`. Past this
/// many the remaining entries keep `dirty: None` and the panel says `unknown` —
/// truthful, and bounded, where probing everything would make `/worktree` slow
/// exactly on the machines that have the most worktrees.
pub const MAX_DIRTY_PROBES: usize = 16;

/// Subcommands `/worktree` must never run, whatever a future caller asks for.
const DESTRUCTIVE: [&str; 7] = [
    "remove", "prune", "reset", "clean", "checkout", "gc", "reflog",
];

/// The state word a row shows when git was never asked about it.
const UNKNOWN_STATE: &str = "unknown";
/// The state word for a worktree with no uncommitted changes.
const CLEAN_STATE: &str = "clean";
/// The state word for a worktree with uncommitted changes.
const DIRTY_STATE: &str = "dirty";
/// What a row shows in place of the branch name for a detached HEAD.
const DETACHED: &str = "detached";

// ─── The process boundary ──────────────────────────────────────────────────

/// One finished `git` invocation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitOutput {
    /// Exit code, or `-1` when the child was killed by a signal.
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// The injectable spawner: `(args, timeout) → output`.
///
/// The program is bound into the closure ([`GitCli::new`] uses `git`,
/// [`GitCli::with_runner`] takes whatever the caller wants) so a test can
/// answer without a process and assert on the exact argv.
pub type GitRunner = Box<dyn Fn(&[String], Duration) -> Result<GitOutput, String> + Send + Sync>;

/// The real spawn: one `<program> <args…>` through the shared
/// drain/limit/timeout helper (see the module docs).
fn spawn_git(program: &Path, args: &[String], timeout: Duration) -> Result<GitOutput, String> {
    let (code, stdout, stderr) = spawn_with_timeout(program, args, timeout)?;
    Ok(GitOutput {
        code,
        stdout,
        stderr,
    })
}

/// `git` plumbing for `/worktree`.
pub struct GitCli {
    /// The program the runner spawns (kept for diagnostics and tests).
    program: PathBuf,
    runner: GitRunner,
    query_timeout: Duration,
    add_timeout: Duration,
}

impl Default for GitCli {
    fn default() -> Self {
        Self::new()
    }
}

impl GitCli {
    /// The real thing: `git` from `PATH`, with the production timeouts.
    pub fn new() -> Self {
        Self::with_program(PathBuf::from(GIT_PROGRAM))
    }

    /// The real thing against an explicit program: the seam a host without git
    /// drives (the spawn failure is reported, not hidden), and what a
    /// `future`-style sibling lookup would use.
    pub fn with_program(program: PathBuf) -> Self {
        let spawn_program = program.clone();
        Self::with_runner(
            Box::new(move |args: &[String], timeout: Duration| {
                spawn_git(&spawn_program, args, timeout)
            }),
            program,
        )
    }

    /// A CLI whose process boundary is `runner`. The two timeouts start at
    /// their production values.
    pub fn with_runner(runner: GitRunner, program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            runner,
            query_timeout: GIT_QUERY_TIMEOUT,
            add_timeout: GIT_ADD_TIMEOUT,
        }
    }

    /// Narrow both budgets (a test may assert the budgets a call carries).
    pub fn with_timeouts(mut self, query: Duration, add: Duration) -> Self {
        self.query_timeout = query;
        self.add_timeout = add;
        self
    }

    /// The query budget this CLI hands its runner.
    pub fn query_timeout(&self) -> Duration {
        self.query_timeout
    }

    /// The `worktree add` budget this CLI hands its runner.
    pub fn add_timeout(&self) -> Duration {
        self.add_timeout
    }

    /// The program this CLI reports in diagnostics.
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// Run `args` — refused when the *subcommand* is one of [`DESTRUCTIVE`].
    fn run(&self, args: &[String], timeout: Duration) -> Result<GitOutput, String> {
        ensure_read_only(args)?;
        (self.runner)(args, timeout)
    }

    /// `git worktree list --porcelain`, parsed into records.
    pub fn list(&self, dir: &Path) -> Result<Vec<WorktreeInfo>, String> {
        let args = list_args(dir);
        let out = self.run(&args, self.query_timeout)?;
        if out.code != 0 {
            return Err(git_failure(&args, &out));
        }
        Ok(parse_worktree_list(&out.stdout))
    }

    /// The short branch name at `dir` — what a new worktree is cut from.
    pub fn branch(&self, dir: &Path) -> Result<String, String> {
        let args = branch_args(dir);
        let out = self.run(&args, self.query_timeout)?;
        if out.code != 0 {
            return Err(git_failure(&args, &out));
        }
        Ok(out.stdout.trim().to_string())
    }

    /// `true` when `path` has uncommitted changes.
    fn is_dirty(&self, path: &str) -> Result<bool, String> {
        let args = status_args(Path::new(path));
        let out = self.run(&args, self.query_timeout)?;
        if out.code != 0 {
            return Err(git_failure(&args, &out));
        }
        Ok(!out.stdout.trim().is_empty())
    }

    /// Fill in `dirty` for the first [`MAX_DIRTY_PROBES`] entries.
    ///
    /// A `git status` that fails (a worktree directory that is gone, a
    /// permission problem) leaves that entry `None` — "unknown" — instead of
    /// failing the whole listing: the path and the branch are still worth
    /// showing.
    fn mark_dirty(&self, list: &mut [WorktreeInfo]) {
        for info in list.iter_mut().take(MAX_DIRTY_PROBES) {
            info.dirty = self.is_dirty(&info.path).ok();
        }
    }

    /// List + dirty flags, the shape the picker shows.
    pub fn probe(&self, dir: &Path) -> Result<Vec<WorktreeInfo>, String> {
        let mut list = self.list(dir)?;
        self.mark_dirty(&mut list);
        Ok(list)
    }

    /// `git worktree add <path> -b <branch>`.
    ///
    /// When the branch already exists (its worktree was removed, or it was cut
    /// by hand) git refuses `-b`; the branch is then *attached* instead of the
    /// whole command failing, which is what a user asking for that name means.
    pub fn add(&self, root: &Path, plan: &WorktreePlan) -> Result<(), String> {
        let args = add_args(root, plan);
        let out = self.run(&args, self.add_timeout)?;
        if out.code == 0 {
            return Ok(());
        }
        if !out.stderr.contains("already exists") {
            return Err(git_failure(&args, &out));
        }
        let attach = attach_args(root, plan);
        let out = self.run(&attach, self.add_timeout)?;
        if out.code == 0 {
            return Ok(());
        }
        Err(git_failure(&attach, &out))
    }
}

// ─── The command whitelist ─────────────────────────────────────────────────
//
// One pure function per invocation, so the exact argv is a unit test's subject
// rather than a detail buried in a call site.

/// `git -C <dir> worktree list --porcelain`.
pub fn list_args(dir: &Path) -> Vec<String> {
    vec![
        "-C".to_string(),
        dir.display().to_string(),
        "worktree".to_string(),
        "list".to_string(),
        "--porcelain".to_string(),
    ]
}

/// `git -C <dir> status --porcelain` (empty output ⇔ clean).
pub fn status_args(dir: &Path) -> Vec<String> {
    vec![
        "-C".to_string(),
        dir.display().to_string(),
        "status".to_string(),
        "--porcelain".to_string(),
    ]
}

/// `git -C <dir> rev-parse --abbrev-ref HEAD`.
pub fn branch_args(dir: &Path) -> Vec<String> {
    vec![
        "-C".to_string(),
        dir.display().to_string(),
        "rev-parse".to_string(),
        "--abbrev-ref".to_string(),
        "HEAD".to_string(),
    ]
}

/// `git -C <root> worktree add <path> -b <branch>`.
pub fn add_args(root: &Path, plan: &WorktreePlan) -> Vec<String> {
    vec![
        "-C".to_string(),
        root.display().to_string(),
        "worktree".to_string(),
        "add".to_string(),
        plan.path.display().to_string(),
        "-b".to_string(),
        plan.branch.clone(),
    ]
}

/// `git -C <root> worktree add <path> <branch>` — attach an existing branch.
pub fn attach_args(root: &Path, plan: &WorktreePlan) -> Vec<String> {
    vec![
        "-C".to_string(),
        root.display().to_string(),
        "worktree".to_string(),
        "add".to_string(),
        plan.path.display().to_string(),
        plan.branch.clone(),
    ]
}

/// The subcommand an invocation carries: the word after `-C <dir>`, or the word
/// after `worktree` when that is the first one.
fn subcommand(args: &[String]) -> Option<&str> {
    match args.get(2)?.as_str() {
        "worktree" => args.get(3).map(String::as_str),
        other => Some(other),
    }
}

/// Refuse an invocation whose subcommand is one of [`DESTRUCTIVE`].
///
/// Only the subcommand *position* is inspected: `worktree add <path> -b clean`
/// is a legitimate request (a branch named `clean`) and must not be mistaken
/// for `git clean`.
pub fn ensure_read_only(args: &[String]) -> Result<(), String> {
    let Some(sub) = subcommand(args) else {
        return Ok(());
    };
    if DESTRUCTIVE.contains(&sub) {
        return Err(format!(
            "refusing to run the destructive git subcommand '{sub}'"
        ));
    }
    Ok(())
}

/// The args without the leading `-C <dir>` every invocation carries — what a
/// failure message names.
fn git_command(args: &[String]) -> String {
    args.iter().skip(2).cloned().collect::<Vec<_>>().join(" ")
}

/// The first non-blank line of `text`, trimmed.
fn first_non_empty_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

/// The message for a non-zero exit: git's own first stderr line, so the panel
/// shows the real reason (`not a git repository`, `already exists`, …).
fn git_failure(args: &[String], out: &GitOutput) -> String {
    let detail = match first_non_empty_line(&out.stderr) {
        Some(line) => line,
        None => format!("exit code {}", out.code),
    };
    format!("git {}: {detail}", git_command(args))
}

// ─── What git reported ─────────────────────────────────────────────────────

/// One entry of `git worktree list`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorktreeInfo {
    /// The path git reported (absolute in practice).
    pub path: String,
    /// Short branch name, or `None` for a detached HEAD.
    pub branch: Option<String>,
    /// The `HEAD <sha>` line, when git printed one.
    pub head: Option<String>,
    /// The `detached` line was present.
    pub detached: bool,
    /// `git status --porcelain` was empty (`false`), had entries (`true`), or
    /// was never asked / failed (`None`).
    pub dirty: Option<bool>,
}

impl WorktreeInfo {
    /// The directory's own name — the picker's label, and what makes two
    /// worktrees of the same repository distinguishable.
    pub fn name(&self) -> String {
        match Path::new(&self.path).file_name() {
            Some(name) => name.to_string_lossy().into_owned(),
            // "/" and "" have no final component; showing the path is better
            // than showing nothing.
            None => self.path.clone(),
        }
    }

    /// `clean` / `dirty` / `unknown`.
    pub fn state_text(&self) -> &'static str {
        match self.dirty {
            Some(true) => DIRTY_STATE,
            Some(false) => CLEAN_STATE,
            None => UNKNOWN_STATE,
        }
    }

    /// `branch · state`, the picker row's description.
    pub fn describe(&self) -> String {
        let branch = match &self.branch {
            Some(branch) => branch.clone(),
            None => DETACHED.to_string(),
        };
        format!("{branch} · {}", self.state_text())
    }

    /// Does `cwd` live inside this worktree?
    ///
    /// Component-wise (`Path::starts_with`), so `/a/bc` is not inside `/a/b`.
    pub fn is_current(&self, cwd: &str) -> bool {
        !cwd.is_empty() && Path::new(cwd).starts_with(self.path.as_str())
    }
}

/// The worktree the session's cwd is in: the **innermost** match.
///
/// A nested worktree (`.worktrees/demo` inside the repository) also sits inside
/// its parent's directory, so "contains the cwd" is true of both; the longest
/// path is the one the user is actually standing in.
pub fn current_worktree(list: &[WorktreeInfo], cwd: &str) -> Option<usize> {
    list.iter()
        .enumerate()
        .filter(|(_, info)| info.is_current(cwd))
        .max_by_key(|(_, info)| info.path.len())
        .map(|(index, _)| index)
}

/// Parse `git worktree list --porcelain`.
///
/// One record per `worktree <path>` line; the keys this module does not show
/// (`bare`, `locked`, `prunable`, the blank separators) are skipped. Malformed
/// input yields whatever was parsed — never a panic.
pub fn parse_worktree_list(stdout: &str) -> Vec<WorktreeInfo> {
    let mut list: Vec<WorktreeInfo> = Vec::new();
    for line in stdout.lines() {
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "worktree" => list.push(WorktreeInfo {
                path: value.to_string(),
                ..WorktreeInfo::default()
            }),
            "HEAD" => {
                if let Some(last) = list.last_mut() {
                    last.head = Some(value.to_string());
                }
            }
            "branch" => {
                if let Some(last) = list.last_mut() {
                    let short = value.strip_prefix("refs/heads/").unwrap_or(value);
                    last.branch = Some(short.to_string());
                }
            }
            "detached" => {
                if let Some(last) = list.last_mut() {
                    last.detached = true;
                }
            }
            _ => {}
        }
    }
    list
}

/// The repository's main worktree — where new worktrees are created.
pub fn repo_root(list: &[WorktreeInfo]) -> Result<PathBuf, String> {
    match list.first() {
        Some(main) => Ok(PathBuf::from(&main.path)),
        None => Err("git reported no worktrees for this repository".to_string()),
    }
}

// ─── Names and plans ───────────────────────────────────────────────────────

/// Is `c` allowed in a worktree name? Deliberately the narrow set a branch and
/// a directory share, so one name is valid for both without further mapping.
fn is_allowed_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/')
}

/// Validate a name the user typed, returning the final path segment.
///
/// Everything here is rejected *before* any process exists: a bad name costs a
/// message, not a failed spawn. The rules are git's own (`check-ref-format`)
/// plus the charset this feature documents.
pub fn validate_worktree_name(input: &str) -> Result<&str, String> {
    let name = input.trim();
    if name.is_empty() {
        return Err(WORKTREE_USAGE.to_string());
    }
    if let Some(bad) = name.chars().find(|c| !is_allowed_char(*c)) {
        return Err(format!(
            "'{bad}' cannot appear in a worktree name — use letters, digits, '.', '_', '/' and '-'"
        ));
    }
    if name.contains("..") {
        return Err("A worktree name must not contain '..'".to_string());
    }
    if name.starts_with('/') || name.ends_with('/') || name.contains("//") {
        return Err("A worktree name must not start or end with '/'".to_string());
    }
    for segment in name.split('/') {
        // git reads a leading '-' as an option, and a leading '.' or a
        // trailing '.lock' as an invalid ref component.
        if segment.starts_with('-') || segment.starts_with('.') {
            return Err(
                "A worktree name must not have a segment starting with '-' or '.'".to_string(),
            );
        }
        if segment.ends_with(".lock") {
            return Err("A worktree name must not have a segment ending with '.lock'".to_string());
        }
    }
    match name.rsplit_once('/') {
        Some((_, last)) => Ok(last),
        None => Ok(name),
    }
}

/// A validated plan: the branch to create and the directory to create it in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreePlan {
    /// The branch name, exactly as the user typed it (`feat/tui-worktree`).
    pub branch: String,
    /// The directory under [`WORKTREES_DIR`] — the branch's last segment, so
    /// `feat/tui-parity` lands in `.worktrees/tui-parity`, exactly like this
    /// repository's own worktrees.
    pub name: String,
    pub path: PathBuf,
}

/// Turn a typed name into a plan under `root` (pure: no filesystem, no git).
pub fn plan_worktree(root: &Path, input: &str) -> Result<WorktreePlan, String> {
    let name = validate_worktree_name(input)?;
    Ok(WorktreePlan {
        branch: input.trim().to_string(),
        name: name.to_string(),
        path: root.join(WORKTREES_DIR).join(name),
    })
}

/// Refuse a plan whose directory already exists.
pub fn ensure_path_free(plan: &WorktreePlan) -> Result<(), String> {
    if plan.path.exists() {
        return Err(format!(
            "{} already exists — pick another name",
            plan.path.display()
        ));
    }
    Ok(())
}

// ─── The two flows the app drives ──────────────────────────────────────────

/// The directories `/worktree` may inspect, in order: the session's cwd, then
/// the directory the TUI was started in.
fn candidate_dirs(session_cwd: &str, fallback: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if !session_cwd.trim().is_empty() {
        dirs.push(PathBuf::from(session_cwd));
    }
    if let Some(fallback) = fallback {
        if dirs.first().is_none_or(|first| first != fallback) {
            dirs.push(fallback.to_path_buf());
        }
    }
    dirs
}

/// List the worktrees of the repository the session is in.
///
/// The session's cwd is asked first; a cwd that is not inside a repository (a
/// session that never moved, `~`, a scratch directory) falls back to the
/// directory the TUI was started in, which is where the user actually is. The
/// last candidate's failure is what the caller reports.
pub fn probe_repo(
    cli: &GitCli,
    session_cwd: &str,
    fallback: Option<&Path>,
) -> Result<Vec<WorktreeInfo>, String> {
    let mut last = "no directory to inspect".to_string();
    for dir in candidate_dirs(session_cwd, fallback) {
        match cli.probe(&dir) {
            Ok(list) if !list.is_empty() => return Ok(list),
            Ok(_) => last = format!("{}: git reported no worktrees", dir.display()),
            Err(err) => last = format!("{}: {err}", dir.display()),
        }
    }
    Err(last)
}

/// What a successful create did — one transcript line plus the cwd switch that
/// follows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeCreated {
    pub plan: WorktreePlan,
    /// The branch the new worktree was cut from, when git could say.
    pub base: Option<String>,
}

impl WorktreeCreated {
    /// The transcript line for a created worktree.
    pub fn summary(&self) -> String {
        let from = match &self.base {
            Some(base) => format!(" from {base}"),
            None => String::new(),
        };
        format!(
            "Created worktree {name} on branch {branch}{from}.",
            name = self.plan.name,
            branch = self.plan.branch
        )
    }
}

/// Create the worktree `input` names, in the repository the session is in.
///
/// Runs on the blocking pool (it spawns git) and never removes or touches
/// anything that already exists — [`ensure_path_free`] refuses a taken
/// directory.
pub fn create_worktree(
    cli: &GitCli,
    session_cwd: &str,
    fallback: Option<&Path>,
    input: &str,
) -> Result<WorktreeCreated, String> {
    // Validate before anything else so a bad name costs no process at all.
    let _ = validate_worktree_name(input)?;
    let list = probe_repo(cli, session_cwd, fallback)?;
    let root = repo_root(&list)?;
    let plan = plan_worktree(&root, input)?;
    ensure_path_free(&plan)?;
    let base = cli.branch(&root).ok();
    cli.add(&root, &plan)?;
    Ok(WorktreeCreated { plan, base })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// The arguments of every call an injected runner saw, in order.
    type Calls = Arc<Mutex<Vec<Vec<String>>>>;

    /// A runner that records its calls and answers with `answers` front to
    /// back. Running out of answers is an error rather than a repeat: a test
    /// that expected fewer git calls than it got must fail, not spin.
    fn scripted_runner(answers: Vec<Result<GitOutput, String>>) -> (GitRunner, Calls) {
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&calls);
        let script = Arc::new(Mutex::new(answers));
        let runner: GitRunner = Box::new(move |args: &[String], _timeout: Duration| {
            sink.lock().unwrap().push(args.to_vec());
            let mut script = script.lock().unwrap();
            if script.is_empty() {
                return Err("scripted runner ran out of answers".to_string());
            }
            script.remove(0)
        });
        (runner, calls)
    }

    /// A CLI over an injected runner, with the program name tests assert on.
    fn fake_cli(answers: Vec<Result<GitOutput, String>>) -> (GitCli, Calls) {
        let (runner, calls) = scripted_runner(answers);
        (
            GitCli::with_runner(runner, PathBuf::from("/fake/git")),
            calls,
        )
    }

    /// A successful invocation with `stdout`.
    fn ok(stdout: &str) -> Result<GitOutput, String> {
        Ok(GitOutput {
            code: 0,
            stdout: stdout.to_string(),
            stderr: String::new(),
        })
    }

    #[test]
    fn default_is_the_real_git_with_the_production_budgets() {
        // `/worktree` builds its CLI with `GitCli::default()`. If that ever
        // resolved to a stub or to narrowed budgets, the panel would keep
        // "working" while never actually reaching git.
        let cli = GitCli::default();
        assert_eq!(cli.program(), Path::new(GIT_PROGRAM));
        assert_eq!(cli.query_timeout(), GIT_QUERY_TIMEOUT);
        assert_eq!(cli.add_timeout(), GIT_ADD_TIMEOUT);
    }

    #[test]
    fn a_script_that_runs_out_of_answers_fails_instead_of_guessing() {
        // The injected boundary must not invent a plausible success once its
        // script is exhausted: a test that forgot an answer has to see the
        // failure rather than an empty-but-valid result.
        let (cli, calls) = fake_cli(Vec::new());

        let err = cli
            .list(Path::new("/repo"))
            .expect_err("an empty script cannot answer `worktree list`");

        assert!(err.contains("ran out of answers"), "got {err}");
        assert_eq!(calls.lock().unwrap().len(), 1, "the runner was asked once");
    }

    /// A failed invocation with `stderr` (exit code 1).
    fn failed(stderr: &str) -> Result<GitOutput, String> {
        Ok(GitOutput {
            code: 1,
            stdout: String::new(),
            stderr: stderr.to_string(),
        })
    }

    /// The calls recorded so far.
    fn recorded(calls: &Calls) -> Vec<Vec<String>> {
        calls.lock().unwrap().clone()
    }

    /// The porcelain document `git worktree list --porcelain` prints for a
    /// repository with a main worktree and one branch worktree.
    const PORCELAIN: &str = "worktree /repo\nHEAD 1111111111111111111111111111111111111111\nbranch refs/heads/main\n\nworktree /repo/.worktrees/demo\nHEAD 2222222222222222222222222222222222222222\nbranch refs/heads/feat/demo\n\n";

    /// A plan for the fake paths the injected-runner tests use.
    fn demo_plan() -> WorktreePlan {
        WorktreePlan {
            branch: "feat/demo".to_string(),
            name: "demo".to_string(),
            path: PathBuf::from("/repo/.worktrees/demo"),
        }
    }

    // ─── Parsing ───────────────────────────────────────────────────────

    #[test]
    fn parses_the_porcelain_document() {
        let list = parse_worktree_list(PORCELAIN);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].path, "/repo");
        assert_eq!(list[0].branch.as_deref(), Some("main"));
        assert_eq!(
            list[0].head.as_deref(),
            Some("1111111111111111111111111111111111111111")
        );
        assert!(!list[0].detached);
        assert_eq!(list[1].path, "/repo/.worktrees/demo");
        assert_eq!(list[1].branch.as_deref(), Some("feat/demo"));
        assert_eq!(list[0].dirty, None, "porcelain says nothing about changes");
    }

    #[test]
    fn parses_detached_heads_and_skips_the_keys_the_panel_does_not_show() {
        let list = parse_worktree_list(
            "worktree /repo\nHEAD 33\nbare\n\nworktree /repo/wt\nHEAD 44\ndetached\nlocked\nprunable gitdir file points to non-existent location\n",
        );
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].branch, None);
        assert!(!list[0].detached);
        assert!(list[1].detached);
        assert_eq!(list[1].branch, None);
        assert_eq!(list[1].head.as_deref(), Some("44"));
    }

    #[test]
    fn parsing_tolerates_empty_and_garbage_input() {
        assert!(parse_worktree_list("").is_empty());
        // A `HEAD`/`branch`/`detached` line before any `worktree` line belongs
        // to no record: it is dropped rather than panicking on an empty list.
        assert!(parse_worktree_list("HEAD 1\nbranch refs/heads/x\ndetached\n").is_empty());
    }

    #[test]
    fn a_branch_outside_refs_heads_is_shown_verbatim() {
        let list = parse_worktree_list("worktree /repo\nbranch refs/remotes/origin/main\n");
        assert_eq!(list[0].branch.as_deref(), Some("refs/remotes/origin/main"));
    }

    // ─── Row text ──────────────────────────────────────────────────────

    #[test]
    fn rows_describe_branch_state_and_name() {
        let info = WorktreeInfo {
            path: "/repo/.worktrees/demo".to_string(),
            branch: Some("feat/demo".to_string()),
            dirty: Some(false),
            ..WorktreeInfo::default()
        };
        assert_eq!(info.name(), "demo");
        assert_eq!(info.describe(), "feat/demo · clean");

        let dirty = WorktreeInfo {
            dirty: Some(true),
            ..info.clone()
        };
        assert_eq!(dirty.state_text(), DIRTY_STATE);
        assert_eq!(dirty.describe(), "feat/demo · dirty");

        let detached = WorktreeInfo {
            path: "/repo".to_string(),
            ..WorktreeInfo::default()
        };
        assert_eq!(detached.name(), "repo");
        assert_eq!(detached.describe(), "detached · unknown");
        assert_eq!(detached.state_text(), UNKNOWN_STATE);
    }

    #[test]
    fn a_path_without_a_final_component_is_shown_as_is() {
        let root = WorktreeInfo {
            path: "/".to_string(),
            ..WorktreeInfo::default()
        };
        assert_eq!(root.name(), "/");
    }

    #[test]
    fn the_current_worktree_is_the_one_containing_the_session_cwd() {
        let info = WorktreeInfo {
            path: "/repo/.worktrees/demo".to_string(),
            ..WorktreeInfo::default()
        };
        assert!(info.is_current("/repo/.worktrees/demo"));
        assert!(info.is_current("/repo/.worktrees/demo/tui/src"));
        assert!(!info.is_current("/repo/.worktrees/demo-2"));
        assert!(!info.is_current("/repo"));
        assert!(!info.is_current(""));
    }

    #[test]
    fn a_nested_worktree_wins_over_the_repository_that_contains_it() {
        // Both entries contain the cwd; only the inner one is where the
        // session actually is.
        let list = parse_worktree_list(PORCELAIN);
        assert_eq!(current_worktree(&list, "/repo/.worktrees/demo"), Some(1));
        assert_eq!(
            current_worktree(&list, "/repo/.worktrees/demo/src"),
            Some(1)
        );
        assert_eq!(current_worktree(&list, "/repo"), Some(0));
        assert_eq!(current_worktree(&list, "/elsewhere"), None);
    }

    // ─── Names ─────────────────────────────────────────────────────────

    #[test]
    fn accepts_the_names_this_repository_uses() {
        for (input, segment) in [
            ("demo", "demo"),
            ("feat/tui-worktree", "tui-worktree"),
            ("fix.tui_2", "fix.tui_2"),
            ("  feat/spaced  ", "spaced"),
        ] {
            assert_eq!(
                validate_worktree_name(input),
                Ok(segment),
                "expected '{input}' to be accepted"
            );
        }
    }

    #[test]
    fn rejects_every_name_that_would_break_git_or_the_shell() {
        let charset = |bad: char| {
            format!(
                "'{bad}' cannot appear in a worktree name — use letters, digits, '.', '_', '/' and '-'"
            )
        };
        let leading = "A worktree name must not have a segment starting with '-' or '.'";
        let cases: Vec<(&str, String)> = vec![
            ("", WORKTREE_USAGE.to_string()),
            ("   ", WORKTREE_USAGE.to_string()),
            ("my worktree", charset(' ')),
            ("demo\u{7}", charset('\u{7}')),
            ("démo", charset('é')),
            (
                "../etc",
                "A worktree name must not contain '..'".to_string(),
            ),
            ("a..b", "A worktree name must not contain '..'".to_string()),
            (
                "/abs",
                "A worktree name must not start or end with '/'".to_string(),
            ),
            (
                "trail/",
                "A worktree name must not start or end with '/'".to_string(),
            ),
            (
                "a//b",
                "A worktree name must not start or end with '/'".to_string(),
            ),
            ("-demo", leading.to_string()),
            ("feat/-demo", leading.to_string()),
            (".hidden", leading.to_string()),
            ("feat/.hidden", leading.to_string()),
            (
                "demo.lock",
                "A worktree name must not have a segment ending with '.lock'".to_string(),
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(
                validate_worktree_name(input),
                Err(expected),
                "expected '{input}' to be rejected"
            );
        }
    }

    #[test]
    fn a_plan_puts_the_branchs_last_segment_under_the_worktrees_directory() {
        let plan = plan_worktree(Path::new("/repo"), "feat/tui-worktree").unwrap();
        assert_eq!(plan.branch, "feat/tui-worktree");
        assert_eq!(plan.name, "tui-worktree");
        assert_eq!(plan.path, Path::new("/repo/.worktrees/tui-worktree"));
        assert!(plan_worktree(Path::new("/repo"), "bad name").is_err());
        assert_eq!(
            plan_worktree(Path::new(""), "demo").unwrap().path,
            Path::new(".worktrees/demo")
        );
    }

    #[test]
    fn refuses_a_plan_whose_directory_already_exists() {
        let dir =
            std::env::temp_dir().join(format!("future-worktree-taken-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let plan = WorktreePlan {
            branch: "demo".to_string(),
            name: "demo".to_string(),
            path: dir.clone(),
        };
        let err = ensure_path_free(&plan).unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        std::fs::remove_dir_all(&dir).unwrap();

        // The same plan with the directory gone is accepted.
        assert_eq!(ensure_path_free(&plan), Ok(()));
    }

    // ─── The command whitelist ─────────────────────────────────────────

    #[test]
    fn every_invocation_is_built_exactly_once_here() {
        let plan = demo_plan();
        assert_eq!(
            list_args(Path::new("/repo")),
            ["-C", "/repo", "worktree", "list", "--porcelain"]
        );
        assert_eq!(
            status_args(Path::new("/repo/.worktrees/demo")),
            ["-C", "/repo/.worktrees/demo", "status", "--porcelain"]
        );
        assert_eq!(
            branch_args(Path::new("/repo")),
            ["-C", "/repo", "rev-parse", "--abbrev-ref", "HEAD"]
        );
        assert_eq!(
            add_args(Path::new("/repo"), &plan),
            [
                "-C",
                "/repo",
                "worktree",
                "add",
                "/repo/.worktrees/demo",
                "-b",
                "feat/demo"
            ]
        );
        assert_eq!(
            attach_args(Path::new("/repo"), &plan),
            [
                "-C",
                "/repo",
                "worktree",
                "add",
                "/repo/.worktrees/demo",
                "feat/demo"
            ]
        );
        // A failure message names the invocation without its `-C <dir>`.
        assert_eq!(
            git_command(&list_args(Path::new("/repo"))),
            "worktree list --porcelain"
        );
    }

    #[test]
    fn the_whitelist_allows_every_invocation_this_module_builds() {
        let plan = demo_plan();
        for args in [
            list_args(Path::new("/repo")),
            status_args(Path::new("/repo")),
            branch_args(Path::new("/repo")),
            add_args(Path::new("/repo"), &plan),
            attach_args(Path::new("/repo"), &plan),
        ] {
            assert_eq!(ensure_read_only(&args), Ok(()), "{args:?}");
        }
    }

    #[test]
    fn the_whitelist_refuses_every_destructive_subcommand() {
        for sub in DESTRUCTIVE {
            let args = ["-C", "/repo", sub]
                .iter()
                .map(|arg| arg.to_string())
                .collect::<Vec<_>>();
            assert_eq!(
                ensure_read_only(&args),
                Err(format!(
                    "refusing to run the destructive git subcommand '{sub}'"
                ))
            );
        }
        // …including the nested form, where `worktree` comes first.
        let nested = ["-C", "/repo", "worktree", "prune"]
            .iter()
            .map(|arg| arg.to_string())
            .collect::<Vec<_>>();
        assert!(ensure_read_only(&nested).is_err());
    }

    #[test]
    fn the_whitelist_does_not_mistake_a_branch_name_for_a_subcommand() {
        // `-b clean` is a branch *named* clean: the guard reads the subcommand
        // position only, so this must be allowed.
        let plan = WorktreePlan {
            branch: "clean".to_string(),
            name: "clean".to_string(),
            path: PathBuf::from("/repo/.worktrees/clean"),
        };
        assert_eq!(
            ensure_read_only(&add_args(Path::new("/repo"), &plan)),
            Ok(())
        );
        // A pair of args with no subcommand at all is nothing to refuse.
        let short = ["-C", "/repo"]
            .iter()
            .map(|arg| arg.to_string())
            .collect::<Vec<_>>();
        assert_eq!(ensure_read_only(&short), Ok(()));
        assert_eq!(ensure_read_only(&[]), Ok(()));
        // `worktree` with nothing after it has no subcommand either.
        let bare = ["-C", "/repo", "worktree"]
            .iter()
            .map(|arg| arg.to_string())
            .collect::<Vec<_>>();
        assert_eq!(ensure_read_only(&bare), Ok(()));
    }

    // ─── Queries over an injected runner ───────────────────────────────

    #[test]
    fn probe_lists_worktrees_and_marks_them_clean_and_dirty() {
        let (cli, calls) = fake_cli(vec![
            ok(PORCELAIN),
            ok(""),                // the main worktree is clean
            ok(" M src/app.rs\n"), // the demo worktree is not
        ]);
        let list = cli.probe(Path::new("/repo")).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].dirty, Some(false));
        assert_eq!(list[1].dirty, Some(true));
        assert_eq!(
            recorded(&calls),
            vec![
                list_args(Path::new("/repo")),
                status_args(Path::new("/repo")),
                status_args(Path::new("/repo/.worktrees/demo")),
            ]
        );
    }

    #[test]
    fn a_failed_status_leaves_that_entry_unknown_and_the_listing_usable() {
        let (cli, calls) = fake_cli(vec![
            ok(PORCELAIN),
            failed("fatal: cannot change to '/repo': No such file or directory"),
            ok(""),
        ]);
        let list = cli.probe(Path::new("/repo")).unwrap();
        assert_eq!(list[0].dirty, None);
        assert_eq!(list[0].state_text(), UNKNOWN_STATE);
        assert_eq!(list[1].dirty, Some(false));
        assert_eq!(recorded(&calls).len(), 3);
    }

    #[test]
    fn listing_stops_probing_status_past_the_cap() {
        let many: String = (0..MAX_DIRTY_PROBES + 1)
            .map(|index| format!("worktree /repo/w{index}\nHEAD 5\nbranch refs/heads/b{index}\n\n"))
            .collect();
        let mut answers = vec![ok(&many)];
        answers.extend((0..MAX_DIRTY_PROBES).map(|_| ok("")));
        let (cli, calls) = fake_cli(answers);
        let list = cli.probe(Path::new("/repo")).unwrap();
        assert_eq!(list.len(), MAX_DIRTY_PROBES + 1);
        assert_eq!(list[MAX_DIRTY_PROBES].dirty, None);
        assert_eq!(
            recorded(&calls).len(),
            MAX_DIRTY_PROBES + 1,
            "one listing plus exactly MAX_DIRTY_PROBES status calls"
        );
    }

    #[test]
    fn a_failing_listing_is_reported_with_gits_own_words() {
        let (cli, _calls) = fake_cli(vec![failed(
            "fatal: not a git repository (or any of the parent directories): .git\n",
        )]);
        assert_eq!(
            cli.list(Path::new("/tmp")).unwrap_err(),
            "git worktree list --porcelain: fatal: not a git repository (or any of the parent directories): .git"
        );
    }

    #[test]
    fn a_failure_without_stderr_falls_back_to_the_exit_code() {
        let out = GitOutput {
            code: 3,
            stdout: String::new(),
            stderr: "\n   \n".to_string(),
        };
        assert_eq!(
            git_failure(&list_args(Path::new("/repo")), &out),
            "git worktree list --porcelain: exit code 3"
        );
    }

    #[test]
    fn a_failing_query_returns_the_git_error_not_a_partial_answer() {
        // `branch` (the cut-from line) can fail on a repository with no commit
        // yet; the failure is a `Result`, never an empty branch name.
        let (cli, _calls) = fake_cli(vec![failed("fatal: ambiguous argument 'HEAD'")]);
        assert!(cli
            .branch(Path::new("/repo"))
            .unwrap_err()
            .contains("ambiguous argument"));
    }

    #[test]
    fn a_runner_error_travels_through_unchanged() {
        let (cli, calls) = fake_cli(vec![Err("failed to spawn git: No such file".to_string())]);
        assert_eq!(
            cli.list(Path::new("/repo")),
            Err("failed to spawn git: No such file".to_string())
        );
        assert_eq!(recorded(&calls).len(), 1);
        // …and a timeout keeps its marker, so the panel can say "retry".
        let (cli, _calls) = fake_cli(vec![Err("future skills: timed out after 60s".to_string())]);
        let err = create_worktree(&cli, "/repo", None, "demo").unwrap_err();
        assert!(err.contains("timed out"), "{err}");
    }

    #[test]
    fn the_cli_hands_its_two_budgets_to_the_runner() {
        let (runner, calls) = scripted_runner(vec![ok("feat/tui-parity\n"), ok("")]);
        let seen: Arc<Mutex<Vec<Duration>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let timed: GitRunner = Box::new(move |args: &[String], timeout: Duration| {
            sink.lock().unwrap().push(timeout);
            runner(args, timeout)
        });
        let cli = GitCli::with_runner(timed, PathBuf::from("/fake/git"))
            .with_timeouts(Duration::from_secs(1), Duration::from_secs(2));
        assert_eq!(cli.query_timeout(), Duration::from_secs(1));
        assert_eq!(cli.add_timeout(), Duration::from_secs(2));
        assert_eq!(cli.program(), Path::new("/fake/git"));
        cli.branch(Path::new("/repo")).unwrap();
        assert_eq!(
            *seen.lock().unwrap(),
            vec![Duration::from_secs(1)],
            "a read uses the query budget"
        );
        assert_eq!(recorded(&calls).len(), 1);
    }

    #[test]
    fn the_branch_of_a_checkout_is_what_a_new_worktree_is_cut_from() {
        let (cli, calls) = fake_cli(vec![ok("feat/tui-parity\n")]);
        assert_eq!(cli.branch(Path::new("/repo")).unwrap(), "feat/tui-parity");
        assert_eq!(recorded(&calls), vec![branch_args(Path::new("/repo"))]);
    }

    #[test]
    fn add_creates_a_branch_and_reports_a_real_failure() {
        let plan = demo_plan();
        let (cli, calls) = fake_cli(vec![ok("Preparing worktree\n")]);
        assert_eq!(cli.add(Path::new("/repo"), &plan), Ok(()));
        assert_eq!(
            recorded(&calls),
            vec![add_args(Path::new("/repo"), &plan)],
            "a successful add runs exactly one command"
        );

        let (cli, calls) = fake_cli(vec![failed(
            "fatal: '/repo/.worktrees/demo' is not a valid directory",
        )]);
        let err = cli.add(Path::new("/repo"), &plan).unwrap_err();
        assert!(err.contains("not a valid directory"), "{err}");
        assert_eq!(
            recorded(&calls).len(),
            1,
            "a failure that is not 'branch exists' is not retried as an attach"
        );
    }

    #[test]
    fn an_existing_branch_is_attached_instead_of_failing() {
        let plan = demo_plan();
        let (cli, calls) = fake_cli(vec![
            failed("fatal: a branch named 'feat/demo' already exists"),
            ok("Preparing worktree\n"),
        ]);
        assert_eq!(cli.add(Path::new("/repo"), &plan), Ok(()));
        assert_eq!(
            recorded(&calls),
            vec![
                add_args(Path::new("/repo"), &plan),
                attach_args(Path::new("/repo"), &plan),
            ]
        );

        // The attach can fail too (the branch is checked out elsewhere).
        let (cli, calls) = fake_cli(vec![
            failed("fatal: a branch named 'feat/demo' already exists"),
            failed("fatal: 'feat/demo' is already checked out at '/repo/.worktrees/other'"),
        ]);
        let err = cli.add(Path::new("/repo"), &plan).unwrap_err();
        assert!(err.contains("already checked out"), "{err}");
        assert_eq!(recorded(&calls).len(), 2);
    }

    // ─── The two flows the app drives ──────────────────────────────────

    #[test]
    fn the_session_cwd_is_preferred_over_the_start_directory() {
        let (cli, calls) = fake_cli(vec![ok(PORCELAIN), ok(""), ok("")]);
        let list = probe_repo(&cli, "/repo", Some(Path::new("/elsewhere"))).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(
            recorded(&calls),
            vec![
                list_args(Path::new("/repo")),
                status_args(Path::new("/repo")),
                status_args(Path::new("/repo/.worktrees/demo")),
            ],
            "a cwd inside a repository is the only directory asked"
        );
    }

    #[test]
    fn a_session_cwd_outside_any_repository_falls_back_to_the_start_directory() {
        let (cli, calls) = fake_cli(vec![
            failed("fatal: not a git repository (or any of the parent directories): .git"),
            ok(PORCELAIN),
            ok(""),
            ok(""),
        ]);
        let list = probe_repo(&cli, "/mock", Some(Path::new("/repo"))).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(recorded(&calls)[1], list_args(Path::new("/repo")));
    }

    #[test]
    fn with_nowhere_to_look_the_failure_says_so() {
        let (cli, calls) = fake_cli(vec![]);
        assert_eq!(
            probe_repo(&cli, "   ", None),
            Err("no directory to inspect".to_string())
        );
        assert!(recorded(&calls).is_empty(), "no directory, no process");
    }

    #[test]
    fn the_start_directory_is_not_asked_twice() {
        // The same directory in both slots: one probe, not two.
        let (cli, calls) = fake_cli(vec![ok(PORCELAIN), ok(""), ok("")]);
        probe_repo(&cli, "/repo", Some(Path::new("/repo"))).unwrap();
        assert_eq!(
            recorded(&calls).len(),
            3,
            "one listing plus two status calls"
        );
    }

    #[test]
    fn an_empty_listing_is_reported_as_such() {
        let (cli, _calls) = fake_cli(vec![ok(""), ok("")]);
        assert_eq!(
            probe_repo(&cli, "/repo", Some(Path::new("/elsewhere"))).unwrap_err(),
            "/elsewhere: git reported no worktrees"
        );
        assert_eq!(
            repo_root(&[]),
            Err("git reported no worktrees for this repository".to_string())
        );
        assert_eq!(
            repo_root(&parse_worktree_list(PORCELAIN)).unwrap(),
            Path::new("/repo")
        );
    }

    #[test]
    fn creating_a_worktree_lists_then_branches_then_adds() {
        let (cli, calls) = fake_cli(vec![
            ok(PORCELAIN),
            ok(""),
            ok(""),
            ok("feat/tui-parity\n"),    // the branch the new one is cut from
            ok("Preparing worktree\n"), // the add itself
        ]);
        let created = create_worktree(&cli, "/repo", None, "feat/demo").unwrap();
        assert_eq!(created.plan.name, "demo");
        assert_eq!(
            created.plan.path,
            Path::new("/repo/.worktrees/demo"),
            "the main worktree of the listing is the root"
        );
        assert_eq!(created.base.as_deref(), Some("feat/tui-parity"));
        assert_eq!(
            created.summary(),
            "Created worktree demo on branch feat/demo from feat/tui-parity."
        );
        let calls = recorded(&calls);
        assert_eq!(calls[3], branch_args(Path::new("/repo")));
        assert_eq!(calls[4], add_args(Path::new("/repo"), &created.plan));
    }

    #[test]
    fn a_relative_name_creates_nothing() {
        let (cli, calls) = fake_cli(vec![]);
        assert!(create_worktree(&cli, "/repo", None, "..").is_err());
        assert!(recorded(&calls).is_empty(), "a bad name spawns nothing");
    }

    #[test]
    fn a_create_whose_base_branch_cannot_be_read_still_adds_the_worktree() {
        // `rev-parse` failing is not fatal: it only feeds the "from <branch>"
        // part of the summary, and the worktree is still created.
        let (cli, _calls) = fake_cli(vec![
            ok(PORCELAIN),
            ok(""),
            ok(""),
            failed("fatal: ambiguous argument 'HEAD'"),
            ok("Preparing worktree\n"),
        ]);
        let created = create_worktree(&cli, "/repo", None, "demo").unwrap();
        assert_eq!(created.base, None);
        assert_eq!(created.summary(), "Created worktree demo on branch demo.");
    }

    #[test]
    fn a_create_into_a_taken_directory_never_spawns_the_add() {
        let root =
            std::env::temp_dir().join(format!("future-worktree-root-{}", uuid::Uuid::new_v4()));
        let taken = root.join(WORKTREES_DIR).join("demo");
        std::fs::create_dir_all(&taken).unwrap();
        let list = format!(
            "worktree {}\nHEAD 1\nbranch refs/heads/main\n",
            root.display()
        );
        let (cli, calls) = fake_cli(vec![ok(&list), ok("")]);
        let err = create_worktree(&cli, &root.display().to_string(), None, "demo").unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        assert!(
            !recorded(&calls)
                .iter()
                .any(|args| args.contains(&"add".to_string())),
            "the add never ran"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    // ─── The real git ──────────────────────────────────────────────────

    /// Run one `git` command directly. Test *setup* only: the module's own
    /// whitelist stays read-and-add, so the fixture's `init`/`commit`/`branch`
    /// calls live here instead of in a builder.
    fn git_in(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git runs in this test");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "git {args:?} failed: {stderr}");
    }

    /// A throwaway repository with one commit on `main`, removed on drop.
    struct TempRepo {
        root: PathBuf,
    }

    impl TempRepo {
        fn new() -> Self {
            let dir =
                std::env::temp_dir().join(format!("future-worktree-{}", uuid::Uuid::new_v4()));
            // `git worktree list` prints resolved paths, and on macOS the temp
            // dir is reached through the `/var` → `/private/var` symlink: the
            // fixture canonicalises once so nothing downstream compares a
            // resolved path against an unresolved one.
            std::fs::create_dir_all(&dir).unwrap();
            let root = std::fs::canonicalize(&dir).unwrap();
            git_in(&root, &["init", "-q", "-b", "main"]);
            git_in(&root, &["config", "user.email", "tui@example.com"]);
            git_in(&root, &["config", "user.name", "TUI Test"]);
            std::fs::write(root.join("README.md"), "hello\n").unwrap();
            git_in(&root, &["add", "README.md"]);
            git_in(
                &root,
                &["-c", "commit.gpgsign=false", "commit", "-q", "-m", "init"],
            );
            Self { root }
        }

        fn path(&self) -> String {
            self.root.display().to_string()
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn real_git_lists_creates_and_reports_a_worktree() {
        let repo = TempRepo::new();
        let cli = GitCli::new();
        let cwd = repo.path();

        // A fresh repository: one worktree, on the branch just created, clean.
        let list = cli.probe(&repo.root).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].path, cwd);
        assert_eq!(list[0].branch.as_deref(), Some("main"));
        assert_eq!(list[0].dirty, Some(false));
        assert_eq!(cli.branch(&repo.root).unwrap(), "main");

        // The whole flow the app runs for `/worktree new feat/demo`.
        let created = create_worktree(&cli, &cwd, None, "feat/demo").unwrap();
        assert_eq!(created.plan.path, repo.root.join(".worktrees").join("demo"));
        assert_eq!(created.plan.branch, "feat/demo");
        assert_eq!(created.base.as_deref(), Some("main"));
        assert!(created.plan.path.is_dir(), "git created the directory");
        assert_eq!(
            cli.branch(&created.plan.path).unwrap(),
            "feat/demo",
            "the new worktree checks out the branch it was asked for"
        );

        // …and git lists it, so the picker shows it.
        let list = cli.probe(&repo.root).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].path, created.plan.path.display().to_string());
        assert_eq!(list[1].branch.as_deref(), Some("feat/demo"));
        assert_eq!(list[1].dirty, Some(false));
        assert!(list[1].is_current(&created.plan.path.display().to_string()));
        assert!(!list[1].is_current(&cwd));

        // An untracked file in the new worktree is what `dirty` means…
        std::fs::write(created.plan.path.join("scratch.txt"), "x\n").unwrap();
        let list = cli.probe(&repo.root).unwrap();
        assert_eq!(list[1].dirty, Some(true));
        // …and the main worktree reports the new `.worktrees/` directory as
        // untracked, because this fixture does not ignore it (this repository
        // does; a real checkout of it would stay clean).
        assert_eq!(list[0].dirty, Some(true));
        assert_eq!(list[0].state_text(), DIRTY_STATE);

        // Asking for the same name again refuses before git runs, and the
        // first worktree is still there and still on its branch.
        let err = create_worktree(&cli, &cwd, None, "feat/demo").unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        assert!(created.plan.path.is_dir());
        assert_eq!(cli.branch(&created.plan.path).unwrap(), "feat/demo");
    }

    #[test]
    fn real_git_attaches_a_branch_that_already_exists() {
        let repo = TempRepo::new();
        let cli = GitCli::new();
        // A branch with no worktree of its own (cut by hand, as the fixture).
        git_in(&repo.root, &["branch", "feat/taken"]);

        let created = create_worktree(&cli, &repo.path(), None, "feat/taken").unwrap();
        assert_eq!(created.plan.path, repo.root.join(".worktrees/taken"));
        assert_eq!(cli.branch(&created.plan.path).unwrap(), "feat/taken");
        let list = cli.probe(&repo.root).unwrap();
        assert_eq!(list.len(), 2, "the branch was attached, not recreated");
    }

    #[test]
    fn a_host_without_git_reports_the_spawn_failure() {
        let cli = GitCli::with_program(PathBuf::from("/nonexistent/git"));
        let err = cli.list(Path::new("/repo")).unwrap_err();
        assert!(err.contains("failed to spawn"), "{err}");
        assert!(err.contains("/nonexistent/git"), "{err}");
    }
}
