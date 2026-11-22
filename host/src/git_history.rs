//! Read-only Git history for the paired phone: the log as a lane graph, and one
//! commit's patch.
//!
//! The algorithm is the TUI's `/git` panel (`tui/src/workbench/git_panel.rs`),
//! ported rather than re-invented: the phone shows the same picture the desktop's
//! own terminal shows, so a reader who learned to read one already knows the
//! other. The gutter is computed here, on the host, for the same reason it is
//! computed there — it is a property of the history, not of the screen — and the
//! phone only colours the glyphs it is handed (see `mobile/src/features/chat/
//! gitGraph.ts`).
//!
//! Three rules this module keeps, all of them the TUI's:
//!
//! * **Read-only.** Nothing here stages, commits, checks out or resets. The
//!   verbs are `rev-parse`, `log` and `show`.
//! * **Bounded.** One page of commits ([`COMMIT_LIMIT`]), a patch capped the way
//!   the desktop's diff preview caps one, and every read under a deadline — a
//!   `git` blocked on another session's `rebase` lock must not hold the reply.
//! * **Argv, never a shell.** The sha travels as an argv element after `--`.
//!
//! The graph is *not* `git log --graph`: that prints a commit's row and then the
//! drawing of its connections as further lines, so rows carry no commit and a
//! diagonal appears wherever two lanes swap columns. Here every row is a commit,
//! connections are verticals and elbows only, and a lane keeps one column for as
//! long as it exists.

use crate::proc::NoWindow;
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How many commits one page of the history asks for. A page, not the history: a
/// phone scrolls a screen at a time and a repository with 10⁵ commits is exactly
/// the read that should not happen when a sheet opens.
const COMMIT_LIMIT: usize = 200;

/// How many lanes the picture draws before it stops widening.
///
/// A gutter is only as wide as the row that carries it — a row with one lane is
/// two cells and its subject starts immediately — so the width a page costs is
/// the width of its busiest row, and a wider window is affordable: ten lanes is
/// twenty cells beside the subject in the one row that needs them, not in every
/// row. A line past the tenth is drawn *in* the tenth column, sharing it with the
/// other lines that run past the window. It keeps its commits — each one still
/// gets its own row and a mark — but it no longer has a column of its own: those
/// commits are drawn with `*` instead of `o`, so the tenth column's own line and
/// the folded ones do not read as one line.
pub const LANE_WINDOW: usize = 10;

/// Budget for one read-only query (`rev-parse`, `log`).
const QUERY_TIMEOUT: Duration = Duration::from_secs(5);

/// Budget for one commit's detail. A patch is bounded *after* it arrives, so the
/// read gets room for a large commit on a slow disk.
const SHOW_TIMEOUT: Duration = Duration::from_secs(10);

/// Ceiling for a single read's stdout. A page is a few hundred kilobytes (a page
/// is read at `COMMIT_LIMIT` rows whatever the depth, so this bound does not
/// move with the reader's scroll); a patch can be far larger, and it is bounded
/// here rather than in memory after the fact.
const OUTPUT_LIMIT: usize = 8 * 1024 * 1024;

/// How long a killed child gets to actually die before the read gives up on
/// reaping it (the same bound `cli_tool` keeps: a child wedged in an
/// uninterruptible syscall must not hold the reply).
const REAP_GRACE: Duration = Duration::from_millis(500);

/// A patch's rows, and its bytes: the desktop's diff preview caps at the same
/// two numbers, so the phone shows the same amount of a large commit it would
/// show of a large file.
const PATCH_MAX_ROWS: usize = 2_000;
const PATCH_MAX_BYTES: usize = 256 * 1024;

const FIELD: char = '\u{1f}';
const RECORD: char = '\u{1e}';

/// How many times the layout may be laid out again after dropping a lane that
/// carried nothing.
const PASSES: usize = 4;

/// One commit as the log read it: what the lanes are matched on (`id`, `parents`)
/// and what a row shows.
struct Entry {
    short: String,
    id: String,
    parents: Vec<String>,
    when: String,
    who: String,
    decorations: String,
    subject: String,
}

/// One row of the history list, as the phone paints it.
///
/// `graph` is the gutter for this row: two characters per lane, `' '` where
/// nothing is drawn, and every row's string is the same length so the subjects
/// line up in one column. It is a string rather than a per-cell list because the
/// phone colours it by column, which is `index / 2`, and a string keeps that
/// cheap.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphRow {
    /// The abbreviated id: the row's key, and what a tap asks the detail for.
    pub short: String,
    pub when: String,
    /// The commit's *author* (`%an`), not its committer (`%cn`): a squash merge
    /// on GitHub rewrites the committer to "GitHub", and the row would then name
    /// the tool instead of the person who wrote the change.
    pub who: String,
    /// Ref names on this commit (`HEAD -> main, origin/main, tag: v1`), empty
    /// when none.
    pub decorations: String,
    pub subject: String,
    pub graph: String,
}

/// The history list's reply.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphListing {
    /// Whether the session's workspace is inside a Git repository at all. The
    /// phone draws no entry for a plain directory, so this is answered by the
    /// same read the list uses instead of costing a round trip of its own.
    pub is_repo: bool,
    /// The repository's directory name, for the sheet's title. Empty when the
    /// workspace is not a repository.
    pub repo_name: String,
    /// The repository root the history was read from, in the file system's own
    /// spelling — a client renders this string verbatim, so it carries neither a
    /// Windows extended-length prefix nor Git's own separator style (see
    /// `reported_root`). The phone shows it, because a workspace can hold
    /// several checkouts.
    pub path: String,
    pub commits: Vec<GraphRow>,
    /// Whether more commits exist past this page. The same fact as
    /// `!cursor.is_empty()`, kept because a client that cannot page (an older
    /// phone, an older desktop) still has to know where the list stops.
    pub truncated: bool,
    /// What to ask for the next page with: hand it back verbatim as the next
    /// read's cursor. Empty on the last page — the read that answers it is the
    /// one that showed the root commit.
    ///
    /// Opaque to every client on purpose. It is the log's *frontier*, not a
    /// number: one entry per open lane (the commit that lane is waiting for,
    /// empty when the column is free) plus the refs the pages above have not
    /// shown. That is what lets the next page continue from where this one
    /// stopped instead of from the top of the history, and it is also what lets
    /// the drawing carry on in the same columns.
    pub cursor: String,
}

/// Where a page resumes: the open lanes, and the refs still unshown.
///
/// Both halves are needed. The lanes alone lose a branch tip that has not been
/// reached yet — a tip is not an ancestor of anything above it — and the result
/// is a page missing exactly the commits of a branch that had not started
/// showing. `tips` alone cannot continue the drawing, because it says nothing
/// about which column a line was in.
#[derive(Default)]
struct PageCursor {
    /// One entry per lane column: the sha that column waits for, `""` when the
    /// column is free.
    lanes: Vec<String>,
    /// Ref tips the pages above have not shown yet.
    tips: Vec<String>,
}

/// The cursor's on-the-wire spelling.
///
/// Plain text rather than JSON because it travels in a string field and only
/// this module reads it: shas are hex, so `,` and `|` cannot occur inside one.
/// A version tag leads it, so a future change to the shape can be told apart
/// from this one instead of being misread as lanes.
const CURSOR_VERSION: &str = "1";

impl PageCursor {
    fn decode(text: &str) -> Self {
        let mut parts = text.splitn(3, '|');
        if parts.next() != Some(CURSOR_VERSION) {
            return Self::default();
        }
        let lanes = parts
            .next()
            .map(|raw| raw.split(',').map(str::to_string).collect())
            .unwrap_or_default();
        let tips = parts
            .next()
            .map(|raw| {
                raw.split(',')
                    .filter(|sha| !sha.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        Self { lanes, tips }
    }

    fn encode(&self) -> String {
        format!(
            "{CURSOR_VERSION}|{}|{}",
            self.lanes.join(","),
            self.tips.join(",")
        )
    }

    /// Whether this is the first page: nothing read yet.
    fn is_start(&self) -> bool {
        self.lanes.iter().all(String::is_empty) && self.tips.is_empty()
    }

    /// The revs git should walk from: every lane's target and every unshown tip.
    fn revs(&self) -> Vec<&str> {
        self.lanes
            .iter()
            .chain(self.tips.iter())
            .map(String::as_str)
            .filter(|sha| !sha.is_empty())
            .collect()
    }
}

/// One commit's detail: its stat and patch, already bounded.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitDetail {
    pub short: String,
    /// The patch *with* the stat, as `git show --stat --patch` prints it.
    pub patch: String,
    /// Whether [`PATCH_MAX_ROWS`] / [`PATCH_MAX_BYTES`] cut the patch.
    pub truncated: bool,
    /// Every branch, remote-tracking branch and tag that points at this commit,
    /// in the shape [`Refs::of`] describes.
    pub refs: Refs,
}

/// The refs that point at one commit, split by the question a reader asks.
///
/// A list row can only say "`HEAD -> main, origin/main, tag: v1`" on one line,
/// and git's short decoration leaves local branches out once a commit has
/// enough refs. The detail page has room for all of them, so it asks
/// `for-each-ref` instead of reusing the log's own string.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Refs {
    /// Local branches, without `refs/heads/`.
    pub branches: Vec<String>,
    /// Remote-tracking branches, without `refs/remotes/` (`origin/main`).
    pub remotes: Vec<String>,
    /// Tags, without `refs/tags/`.
    pub tags: Vec<String>,
    /// `HEAD` when it is this commit. A detached checkout of an old commit is
    /// otherwise invisible: no branch points there, yet it is what is checked out.
    pub head: bool,
}

impl Refs {
    /// Read every ref pointing at `sha`.
    ///
    /// `for-each-ref --points-at` is the whole answer rather than a decoration:
    /// it walks every ref, so a commit with five branches still lists five,
    /// where the log's `%D` would shorten the line.
    fn of(cwd: &Path, sha: &str) -> Self {
        let mut refs = Refs::default();
        // A ref read that fails is not worth failing the patch over: the patch
        // is what the page is for, and an empty ref list says "none known".
        let Ok(output) = git(
            cwd,
            &["for-each-ref", "--points-at", sha, "--format=%(refname)"],
            QUERY_TIMEOUT,
        ) else {
            return refs;
        };
        if !output.success {
            return refs;
        }
        for line in output
            .stdout
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
        {
            if let Some(name) = line.strip_prefix("refs/heads/") {
                refs.branches.push(name.to_string());
            } else if let Some(name) = line.strip_prefix("refs/remotes/") {
                refs.remotes.push(name.to_string());
            } else if let Some(name) = line.strip_prefix("refs/tags/") {
                refs.tags.push(name.to_string());
            }
        }
        refs.branches.sort();
        refs.remotes.sort();
        refs.tags.sort();
        // `HEAD` is a symref, so `--points-at` does not list it. Ask the
        // checkout directly: it is the one ref a reader is most likely to care
        // about, and the only one that can point at a commit no branch names.
        refs.head = git(
            cwd,
            &["rev-parse", "--verify", "--quiet", "HEAD"],
            QUERY_TIMEOUT,
        )
        .map(|head| head.success && head.stdout.trim() == sha)
        .unwrap_or(false);
        refs
    }
}

/// `git` run under a deadline, in `cwd`, writing nothing to the repository.
struct Output {
    stdout: String,
    stderr: String,
    success: bool,
}

fn git(cwd: &Path, args: &[&str], timeout: Duration) -> Result<Output, String> {
    let mut command = Command::new("git");
    command
        .no_window()
        .arg("-C")
        .arg(cwd)
        // Non-ASCII paths come back literal, matching every other Git read in
        // this host (`git_review::git_output`). Harmless for log/show.
        .args(["-c", "core.quotePath=false"])
        .args(args)
        // A prompt (`GIT_TERMINAL_PROMPT`) or a pager would wait on a tty that
        // does not exist; the deadline would then report a timeout instead of
        // the answer.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PAGER", "cat")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| describe(&error))?;
    let mut stdout = child.stdout.take().ok_or("git produced no stdout handle")?;
    let mut stderr = child.stderr.take().ok_or("git produced no stderr handle")?;
    // Drained on their own threads: a pipe whose reader is the caller blocks the
    // child once the buffer fills, and a large log does fill it.
    let out_reader = std::thread::spawn(move || read_bounded(&mut stdout));
    let err_reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).into_owned()
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    break None;
                }
                if crate::request_scope::is_stopping() {
                    break None;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => {
                reap_quietly(&mut child);
                return Err(describe(&error));
            }
        }
    };
    let Some(status) = status else {
        reap_quietly(&mut child);
        return Err("git did not answer in time".to_string());
    };
    let (stdout, overflowed) = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    if overflowed {
        return Err("The Git output is too large to display.".to_string());
    }
    Ok(Output {
        stdout,
        stderr,
        success: status.success(),
    })
}

/// Kill a child and reap it without blocking the caller on its exit: a process
/// wedged in an uninterruptible syscall must not hold the reply (see
/// [`REAP_GRACE`]).
fn reap_quietly(child: &mut std::process::Child) {
    let _ = child.kill();
    let grace = Instant::now() + REAP_GRACE;
    while Instant::now() < grace {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return,
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

/// Read a pipe to the end, keeping at most [`OUTPUT_LIMIT`] bytes. The second
/// value says the cap was hit: the reader keeps draining (so the child is never
/// blocked) but the caller will refuse the result.
fn read_bounded(pipe: &mut impl std::io::Read) -> (String, bool) {
    let mut kept = Vec::new();
    let mut overflowed = false;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        match pipe.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                if kept.len() + read > OUTPUT_LIMIT {
                    overflowed = true;
                    let room = OUTPUT_LIMIT.saturating_sub(kept.len());
                    kept.extend_from_slice(&buffer[..room]);
                    continue;
                }
                kept.extend_from_slice(&buffer[..read]);
            }
        }
    }
    (String::from_utf8_lossy(&kept).into_owned(), overflowed)
}

fn describe(error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        "Git is not installed on this computer.".to_string()
    } else {
        format!("Git could not be started: {error}")
    }
}

/// The message a failed read is shown with: git's own words, which are content
/// and travel verbatim.
fn failure(output: &Output) -> String {
    let message = output.stderr.trim();
    if message.is_empty() {
        "Git failed without a message.".to_string()
    } else {
        message.to_string()
    }
}

/// The repository root in the spelling this host reports it in.
///
/// `git rev-parse --show-toplevel` prints the path in Git's own spelling: forward
/// slashes on Windows, where the file system (and every other path this host
/// hands a client) uses backslashes. A client renders the string verbatim, as it
/// does the session file listing's paths, so it is normalised once here, at the
/// boundary, rather than by each reader: the directory's canonical form
/// (`fs::canonicalize`, which also resolves a symlinked spelling such as macOS's
/// `/var`), with the Windows extended-length prefix (`\\?\`) removed — that
/// prefix is a long-path implementation detail, not part of a path a reader
/// should see. `crate::store::strip_verbatim_prefix` is the same removal the
/// stored workspace paths and `remote_host::session_files` use.
///
/// Only the reply is normalised: `git` is still run in Git's own spelling, so a
/// Windows path that needs the extended-length form (past `MAX_PATH`) keeps it.
/// A directory that no longer resolves (removed between the two reads) keeps
/// Git's own spelling rather than failing the listing.
fn reported_root(toplevel: &Path) -> std::path::PathBuf {
    crate::store::strip_verbatim_prefix(crate::git_review::canonical_or_raw(toplevel))
}

/// The repository whose history this is: its toplevel and its own directory
/// name. `None` when `cwd` is not inside a working tree (or `git` is missing, or
/// the directory is gone) — every one of those answers "no history to show"
/// rather than an error the reader cannot act on.
fn repository(cwd: &Path) -> Option<(std::path::PathBuf, String)> {
    let output = git(
        cwd,
        &["rev-parse", "--is-inside-work-tree", "--show-toplevel"],
        QUERY_TIMEOUT,
    )
    .ok()?;
    if !output.success {
        return None;
    }
    let mut lines = output.stdout.lines();
    let inside = lines.next().unwrap_or("").trim() == "true";
    let toplevel = lines.next().unwrap_or("").trim();
    if !inside || toplevel.is_empty() {
        return None;
    }
    let toplevel = std::path::PathBuf::from(toplevel);
    let name = toplevel
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| toplevel.display().to_string());
    Some((toplevel, name))
}

/// The session workspace's history: the log as a lane graph.
///
/// `cursor` is what the caller's last page answered with — empty for the first
/// page. A page is a window into the log, and the phone reads it a page at a
/// time rather than opening a sheet on 10⁵ commits. A workspace that is not a
/// repository answers [`GraphListing::is_repo`] `false` with nothing in it, which
/// is what the phone draws nothing for.
pub fn list(workspace: &str, cursor: &str) -> Result<GraphListing, crate::Error> {
    page(workspace, cursor, COMMIT_LIMIT)
}

/// One page of the history: `take` commits from where `cursor` stopped.
///
/// Split from [`list`] so a test can page a handful of commits instead of
/// building two hundred to see a second page.
///
/// The page is read *from the cursor*, never from the top of the log: the cursor
/// names the commits the page above stopped in front of, so the cost of a read
/// is one page whatever the reader's depth. Reading from the top instead (with
/// `--skip`, or by asking for `skip + take` commits and dropping the first
/// `skip`) makes every page cost what the whole history above it cost, which is
/// the read that turns a long scroll into a slow one.
///
/// The same cursor is what keeps the *picture* continuous: its lanes say which
/// column is waiting for which commit, so the page opens the lines the page above
/// left running, in the columns it left them in.
fn page(workspace: &str, cursor: &str, take: usize) -> Result<GraphListing, crate::Error> {
    let root = Path::new(workspace);
    let empty = |path: String| GraphListing {
        is_repo: false,
        repo_name: String::new(),
        path,
        commits: Vec::new(),
        truncated: false,
        cursor: String::new(),
    };
    let Some((toplevel, repo_name)) = repository(root) else {
        return Ok(empty(workspace.to_string()));
    };
    let cursor = PageCursor::decode(cursor);
    // Every branch, tag and remote-tracking branch: the log describes the whole
    // repository, not just the checkout. `--all` would also pull in `refs/stash`
    // (a stash is a commit tree) as if it were a line of work, and naming `HEAD`
    // would be fatal on an unborn branch — taking every other branch's commits
    // down with it.
    //
    // A continuation names its own revs instead: the refs are already described
    // by the cursor's unshown tips, and passing them again would restart the walk
    // at every tip — answering with the first page over and over.
    //
    // `%an` (author), not `%cn` (committer). They differ whenever someone else
    // lands a change — a squash merge on GitHub rewrites the committer to
    // "GitHub", a rebase or a cherry-pick to whoever ran it — and the author is
    // the person the row is about. Reading the committer there made a page of
    // real work look like it was all done by GitHub.
    //
    // `--date-order`, not `--topo-order`: both never show a parent before its
    // children, but only the date walk can be cut and continued. A topological
    // order is computed for the whole graph, so asking for it again from below a
    // cut can order two live branches differently from the way the walk above
    // had them queued — the page after a seam would then hold the right commits
    // in a different order than one uninterrupted read. The date walk's order is
    // a property of the remaining commits alone.
    let pretty = "--pretty=format:%x1e%h%x1f%H%x1f%P%x1f%cd%x1f%an%x1f%D%x1f%s";
    // One past the page: whether another page follows is answered by the read
    // itself rather than by a count that has to be trusted.
    let limit = format!("--max-count={}", take + 1);
    let mut args: Vec<String> = vec![
        "log".to_string(),
        "--decorate=short".to_string(),
        // Minute precision, no zone: a `+0800` in every one of 200 rows is
        // six cells that repeat what the local clock already says.
        "--date=format:%Y-%m-%d %H:%M".to_string(),
        "--date-order".to_string(),
        limit,
        pretty.to_string(),
    ];
    if cursor.is_start() {
        args.extend(["--branches", "--remotes", "--tags"].map(str::to_string));
    } else {
        args.extend(cursor.revs().into_iter().map(str::to_string));
    }
    // `--` ends revision parsing: a branch named like a path would otherwise be
    // resolved as the path and log nothing.
    args.push("--".to_string());
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = git(&toplevel, &borrowed, QUERY_TIMEOUT)?;
    // A repository with no commit is not an error the reader can act on; it is
    // an empty log.
    let text = if output.success {
        output.stdout
    } else if output.stderr.contains("does not have any commits") {
        String::new()
    } else {
        return Err(failure(&output).into());
    };
    let mut entries = parse_entries(&text);
    let truncated = entries.len() > take;
    entries.truncate(take);
    let (gutters, lanes) = graph(&entries, &cursor.lanes);
    let shown: Vec<&str> = entries.iter().map(|entry| entry.id.as_str()).collect();
    let next = if truncated {
        // The lanes the page ended with, plus the refs still unshown. A tip the
        // pages above have not reached is not an ancestor of anything above the
        // seam, so leaving it out would drop a whole branch from the page after.
        let tips = if cursor.is_start() {
            ref_tips(&toplevel)
        } else {
            cursor.tips.clone()
        };
        PageCursor {
            lanes,
            tips: tips
                .into_iter()
                .filter(|tip| !shown.contains(&tip.as_str()))
                .collect(),
        }
        .encode()
    } else {
        String::new()
    };
    let commits = entries
        .into_iter()
        .zip(gutters)
        .map(|(entry, graph)| GraphRow {
            short: entry.short,
            when: entry.when,
            who: entry.who,
            decorations: entry.decorations,
            subject: entry.subject,
            graph,
        })
        .collect();
    Ok(GraphListing {
        is_repo: true,
        repo_name,
        // The root in the spelling a client may render and send back, while the
        // log above was read in Git's own (`reported_root`).
        path: reported_root(&toplevel).display().to_string(),
        commits,
        truncated,
        cursor: next,
    })
}

/// The shas the log walks from: every branch, tag and remote-tracking branch.
///
/// The same set the first page's `--branches --remotes --tags` names, read as
/// shas so a continuation can carry the ones it has not shown yet.
fn ref_tips(toplevel: &Path) -> Vec<String> {
    let output = match git(
        toplevel,
        &[
            "for-each-ref",
            "--format=%(objectname)",
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ],
        QUERY_TIMEOUT,
    ) {
        Ok(output) if output.success => output,
        // A cursor without tips still pages; it can only lose a branch tip that
        // had not been reached. Better a page than a failure to read history.
        _ => return Vec::new(),
    };
    let mut seen: Vec<String> = Vec::new();
    for sha in output
        .stdout
        .lines()
        .map(str::trim)
        .filter(|sha| !sha.is_empty())
    {
        if !seen.iter().any(|already| already == sha) {
            seen.push(sha.to_string());
        }
    }
    seen
}

/// One commit's patch: `git show --stat --patch`, bounded.
///
/// `key` is a sha and nothing else: it travels after `--`, so a value that names
/// a path is a path the revision parse never sees.
pub fn detail(workspace: &str, key: &str) -> Result<CommitDetail, crate::Error> {
    let key = key.trim();
    if key.is_empty() || key.len() > 128 || !key.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("The commit is not a commit id.".to_string().into());
    }
    let root = Path::new(workspace);
    let Some((toplevel, _)) = repository(root) else {
        return Err("This workspace is not a Git repository.".to_string().into());
    };
    let output = git(
        &toplevel,
        &[
            "show",
            "--no-color",
            "--no-ext-diff",
            "--date=short",
            "--stat",
            "--patch",
            key,
            "--",
        ],
        SHOW_TIMEOUT,
    )?;
    if !output.success {
        return Err(failure(&output).into());
    }
    let (patch, truncated) = bounded_patch(&output.stdout);
    // The full sha, not the abbreviation the caller sent: `--points-at` matches
    // on the ref's own 40-character id.
    let full = git(&toplevel, &["rev-parse", "--verify", key], QUERY_TIMEOUT)
        .ok()
        .filter(|out| out.success)
        .map(|out| out.stdout.trim().to_string())
        .unwrap_or_else(|| key.to_string());
    Ok(CommitDetail {
        short: key.to_string(),
        patch,
        truncated,
        refs: Refs::of(&toplevel, &full),
    })
}

/// Cut a patch to what a phone can paint, ending on a line rather than mid-byte.
fn bounded_patch(patch: &str) -> (String, bool) {
    let mut out = String::new();
    let mut truncated = false;
    for (index, line) in patch.lines().enumerate() {
        if index >= PATCH_MAX_ROWS || out.len() + line.len() > PATCH_MAX_BYTES {
            out.push_str("… diff truncated; open the desktop's Git panel for the rest …\n");
            truncated = true;
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    (out, truncated)
}

/// Split a log read into entries: one line per commit, [`RECORD`] leading its
/// fields and [`FIELD`] between them. A subject is free text, so the separators
/// have to be bytes a line cannot contain.
fn parse_entries(text: &str) -> Vec<Entry> {
    text.split('\n')
        .filter_map(|line| {
            let at = line.find(RECORD)?;
            let fields: Vec<&str> = line[at + RECORD.len_utf8()..].split(FIELD).collect();
            Some(Entry {
                short: fields.first().copied().unwrap_or("").to_string(),
                id: fields.get(1).copied().unwrap_or("").to_string(),
                parents: fields
                    .get(2)
                    .map(|parents| parents.split_whitespace().map(str::to_string).collect())
                    .unwrap_or_default(),
                when: fields.get(3).copied().unwrap_or("").to_string(),
                who: fields.get(4).copied().unwrap_or("").to_string(),
                decorations: fields.get(5).copied().unwrap_or("").to_string(),
                subject: fields.get(6).copied().unwrap_or("").to_string(),
            })
        })
        .collect()
}

/// One line of work being drawn: the commit it is waiting for and what opened it.
#[derive(Clone)]
struct Lane {
    /// The commit this line waits for — a sha, not a position, so a line that runs
    /// off the bottom of the page is an ordinary open lane and the next page's
    /// cursor can pick it up.
    target: String,
    /// The commit and the index of the parent in its list that opened this line,
    /// when a merge's *extra* parent did. `None` for a line that starts at the
    /// top of the page or carries a commit's first parent.
    opened_by: Option<(usize, usize)>,
    /// Whether a commit has been drawn in this line yet.
    hosted: bool,
}

/// The graph gutter for each entry, one string per row and every string the same
/// width so the subjects line up in one column — and the lanes the page ends
/// with, which the next page resumes from.
///
/// `seed` is that same lane row from the page above: one entry per column, the
/// sha it waits for or `""` where the column is free. A first page seeds nothing.
///
/// The model is tig's: a *lane* is a column waiting for a commit, a commit claims
/// the leftmost lane waiting for it, its first parent inherits that lane, and
/// each further parent opens a column to the right. Read
/// `tui/src/workbench/git_panel.rs` for why each rule below is what it is; this
/// is the same algorithm, kept identical so both surfaces draw one picture.
fn graph(entries: &[Entry], seed: &[String]) -> (Vec<String>, Vec<String>) {
    let seeded: Vec<Option<Lane>> = seed
        .iter()
        .map(|sha| {
            (!sha.is_empty()).then(|| Lane {
                target: sha.clone(),
                opened_by: None,
                // A seeded lane has already drawn its commit — the page above —
                // so a second line joining it here is not a lane that carried
                // nothing.
                hosted: true,
            })
        })
        .collect();
    let mut covered: HashSet<(usize, usize)> = HashSet::new();
    let mut rows = Vec::new();
    let mut lanes: Vec<Option<Lane>> = seeded.clone();
    for pass in 1..=PASSES {
        let (drawn, discovered, ended) = draw(entries, &covered, seeded.clone());
        rows = drawn;
        lanes = ended;
        let mut fresh: Vec<(usize, usize)> = Vec::new();
        for edge in discovered {
            if !covered.contains(&edge) && !fresh.contains(&edge) {
                fresh.push(edge);
            }
        }
        if fresh.is_empty() || pass == PASSES {
            break;
        }
        covered.extend(fresh);
    }
    // Each gutter keeps the width of its own row: a row with one lane is two
    // cells, so its subject starts right after that lane instead of after the
    // page's busiest row. Nothing pads a row to the widest one, which is what
    // makes a ten-lane window affordable — the page pays for its busiest row,
    // not for its busiest row in every row.
    let gutters = rows
        .into_iter()
        .map(|cells| cells.into_iter().collect())
        .collect();
    let mut ended: Vec<String> = lanes
        .into_iter()
        .map(|lane| lane.map(|lane| lane.target).unwrap_or_default())
        .collect();
    while ended.last().is_some_and(String::is_empty) {
        ended.pop();
    }
    (gutters, ended)
}

/// One pass over the layout: the drawn rows, the parent links whose lane turned
/// out to carry no commit of its own, and the lanes still open at the bottom.
type Pass = (Vec<Vec<char>>, Vec<(usize, usize)>, Vec<Option<Lane>>);

/// One pass over the log: the rows, the parent links whose lane turned out to
/// carry no commit of its own, and the lanes still open at the bottom.
fn draw(entries: &[Entry], covered: &HashSet<(usize, usize)>, seeded: Vec<Option<Lane>>) -> Pass {
    let mut lanes: Vec<Option<Lane>> = seeded;
    let mut rows: Vec<Vec<char>> = Vec::with_capacity(entries.len());
    let mut empty: Vec<(usize, usize)> = Vec::new();
    for (at, entry) in entries.iter().enumerate() {
        // Every lane waiting for this commit: the first is where it is drawn, the
        // rest are lines of work joining it here.
        let waiting: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter(|(_, lane)| lane.as_ref().is_some_and(|lane| lane.target == entry.id))
            .map(|(column, _)| column)
            .collect();
        let (mine, closed) = match waiting.split_first() {
            Some((&first, rest)) => (first, rest.to_vec()),
            None => {
                // A tip of its own: the leftmost free column, else the end.
                let free = lanes.iter().position(Option::is_none);
                let column = match free {
                    Some(column) => column,
                    None => {
                        lanes.push(None);
                        lanes.len() - 1
                    }
                };
                lanes[column] = Some(Lane {
                    target: entry.id.clone(),
                    opened_by: None,
                    hosted: false,
                });
                (column, Vec::new())
            }
        };
        // A line closing here that never drew a commit was opened for a parent
        // another line also reaches: its history is already on the page.
        for &column in &closed {
            if let Some(lane) = &lanes[column] {
                if !lane.hosted {
                    if let Some(edge) = lane.opened_by {
                        empty.push(edge);
                    }
                }
            }
        }
        if let Some(lane) = lanes[mine].as_mut() {
            lane.hosted = true;
        }
        // The extra parents, each in a column right of this one: the leftmost one
        // that is free, and — this is what keeps the page narrow — a column a
        // lane is giving up on *this* row.
        let mut opened: Vec<(usize, String, usize)> = Vec::new();
        for (index, parent) in entry.parents.iter().enumerate().skip(1) {
            if covered.contains(&(at, index)) {
                continue;
            }
            let free = (mine + 1..lanes.len()).find(|column| {
                (lanes[*column].is_none() || closed.contains(column))
                    && !opened.iter().any(|(taken, _, _)| taken == column)
            });
            let column = match free {
                Some(column) => column,
                None => {
                    lanes.push(None);
                    lanes.len() - 1
                }
            };
            opened.push((column, parent.clone(), index));
        }
        let rightmost = opened
            .iter()
            .map(|(column, _, _)| *column)
            .chain(closed.iter().copied())
            .fold(mine, usize::max);
        let width = lanes
            .iter()
            .rposition(Option::is_some)
            .unwrap_or(0)
            .max(rightmost);
        // Every lane past the window is drawn in the window's last column: the
        // picture stops widening there, and the lines beyond it stay on the page
        // as one bundle instead of disappearing. A commit on one of them still
        // gets its dot, in that same column, so its row says where the work
        // landed even though the line it sits on is shared.
        let visible = width.min(LANE_WINDOW - 1);
        let mut up = vec![false; visible + 1];
        let mut down = vec![false; visible + 1];
        let mut left = vec![false; visible + 1];
        let mut right = vec![false; visible + 1];
        // Where the commit itself landed, and whether it joins lines. Several
        // lanes can fold into one column, but only one of them is this row's.
        let mut marks = vec![false; visible + 1];
        let mut joins = vec![false; visible + 1];
        // Whether this commit's own line is one of the folded ones. It is drawn
        // in the bundle column like the others, but with its own glyph: inside
        // the window a dot says "this line", here it would have to say "one of
        // several lines that are not drawn", and the two would then read alike.
        let folded = mine > visible;
        for column in 0..=width {
            let fold = column.min(visible);
            let column_up = lanes.get(column).is_some_and(|lane| lane.is_some()) && column != mine;
            // A column a line gives up on and another takes over in the same row
            // continues below: the vertical passes through it.
            let column_down = if opened.iter().any(|(taken, _, _)| *taken == column) {
                true
            } else if closed.contains(&column) {
                false
            } else {
                column_up
            };
            // The bundle carries the directions of every lane inside it, so a
            // thread that runs through it is still drawn.
            up[fold] |= column_up;
            down[fold] |= column_down;
            right[fold] |= column >= mine && column < rightmost;
            left[fold] |= column > mine && column <= rightmost;
            if column == mine {
                marks[fold] = true;
                joins[fold] = entry.parents.len() > 1;
            }
        }
        // The bundle closes at the window's edge: a connector in its last cell
        // would run the horizontal out of the picture and into the fields.
        right[visible] = false;
        let mut cells = vec![' '; 2 * (visible + 1)];
        for column in 0..=visible {
            cells[2 * column] = if marks[column] {
                // `M` is a commit that joins lines, `o` one that only continues
                // its own — tig's two glyphs — and `*` one whose line has no
                // column of its own because the picture stopped widening. A
                // folded commit that joins lines is `*` too: the elbow a merge
                // needs cannot be drawn past the window's edge, so the ring would
                // promise a shape the row cannot show.
                if folded && column == visible {
                    '*'
                } else if joins[column] {
                    'M'
                } else {
                    'o'
                }
            } else {
                let thread = junction(up[column], down[column], left[column], right[column]);
                // The bundle's thread is not this column's own line: a `│` here
                // reads exactly like a lane, and nothing on the row says the work
                // continues past the window rather than in this column. `┊` keeps
                // the column, its width and its colour and says which one it is.
                // A connector keeps its shape — a `┬` is a shape, not a thread.
                if column == visible && width > visible && thread == '│' {
                    '┊'
                } else {
                    thread
                }
            };
            if right[column] {
                cells[2 * column + 1] = '─';
            }
        }
        rows.push(cells);
        for &column in &closed {
            lanes[column] = None;
        }
        lanes[mine] = entry.parents.first().map(|target| Lane {
            target: target.clone(),
            opened_by: None,
            hosted: false,
        });
        for (column, target, index) in opened {
            if column >= lanes.len() {
                lanes.resize(column + 1, None);
            }
            lanes[column] = Some(Lane {
                target,
                opened_by: Some((at, index)),
                hosted: false,
            });
        }
        while lanes.last().is_some_and(Option::is_none) {
            lanes.pop();
        }
    }
    (rows, empty, lanes)
}

/// The glyph where a lane meets the row's horizontal: which of the four
/// directions have a line, and nothing drawn outside ones that do. A lane that
/// keeps going is drawn as it is, with the horizontal broken around it — a
/// crossing mark would claim the two lines touch.
fn junction(up: bool, down: bool, left: bool, right: bool) -> char {
    if !left && !right {
        return if up || down { '│' } else { ' ' };
    }
    match (up, down, right) {
        (true, true, true) => '│',
        (true, true, false) => '┤',
        (true, false, true) => '┴',
        (true, false, false) => '┘',
        (false, true, true) => '┬',
        (false, true, false) => '┐',
        (false, false, _) => '─',
    }
}

#[cfg(test)]
mod tests;
