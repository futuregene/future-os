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
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How many commits one log read asks for. A page, not the history: a phone
/// scrolls a screen at a time and a repository with 10⁵ commits is exactly the
/// read that should not happen when a sheet opens.
const COMMIT_LIMIT: usize = 200;

/// Budget for one read-only query (`rev-parse`, `log`).
const QUERY_TIMEOUT: Duration = Duration::from_secs(5);

/// Budget for one commit's detail. A patch is bounded *after* it arrives, so the
/// read gets room for a large commit on a slow disk.
const SHOW_TIMEOUT: Duration = Duration::from_secs(10);

/// Ceiling for a single read's stdout. `log --max-count=200` is a few hundred
/// kilobytes; a patch can be far larger, and it is bounded here rather than in
/// memory after the fact.
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

/// A parent that lies past the page the log asked for. Its lane is kept — a line
/// running off the bottom says the history continues, which is true — but it can
/// never be claimed by a commit, so the line is never handed to another branch.
const GHOST: usize = usize::MAX;

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
    /// The workspace the history was read in — the phone shows it, because a
    /// workspace can hold several checkouts.
    pub path: String,
    pub commits: Vec<GraphRow>,
    /// The log was cut at [`COMMIT_LIMIT`]; the oldest row is not the root.
    pub truncated: bool,
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
/// A workspace that is not a repository answers [`GraphListing::is_repo`]
/// `false` with nothing in it, which is what the phone draws nothing for.
pub fn list(workspace: &str) -> Result<GraphListing, crate::Error> {
    let root = Path::new(workspace);
    let empty = |path: String| GraphListing {
        is_repo: false,
        repo_name: String::new(),
        path,
        commits: Vec::new(),
        truncated: false,
    };
    let Some((toplevel, repo_name)) = repository(root) else {
        return Ok(empty(workspace.to_string()));
    };
    // Every branch, tag and remote-tracking branch: the log describes the whole
    // repository, not just the checkout. `--all` would also pull in `refs/stash`
    // (a stash is a commit tree) as if it were a line of work, and naming `HEAD`
    // would be fatal on an unborn branch — taking every other branch's commits
    // down with it.
    let pretty = "--pretty=format:%x1e%h%x1f%H%x1f%P%x1f%cd%x1f%cn%x1f%D%x1f%s";
    let limit = format!("--max-count={COMMIT_LIMIT}");
    let output = git(
        &toplevel,
        &[
            "log",
            "--decorate=short",
            // Minute precision, no zone: a `+0800` in every one of 200 rows is
            // six cells that repeat what the local clock already says.
            "--date=format:%Y-%m-%d %H:%M",
            "--topo-order",
            limit.as_str(),
            pretty,
            "--branches",
            "--remotes",
            "--tags",
            // `--` ends revision parsing: a branch named like a path would
            // otherwise be resolved as the path and log nothing.
            "--",
        ],
        QUERY_TIMEOUT,
    )?;
    // A repository with no commit is not an error the reader can act on; it is
    // an empty log.
    let text = if output.success {
        output.stdout
    } else if output.stderr.contains("does not have any commits") {
        String::new()
    } else {
        return Err(failure(&output).into());
    };
    let entries = parse_entries(&text);
    let truncated = entries.len() >= COMMIT_LIMIT;
    let gutters = graph(&entries);
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
        path: toplevel.display().to_string(),
        commits,
        truncated,
    })
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
    Ok(CommitDetail {
        short: key.to_string(),
        patch,
        truncated,
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
#[derive(Clone, Copy)]
struct Lane {
    /// The entry this line waits for, or [`GHOST`] past the page.
    target: usize,
    /// The commit and the index of the parent in its list that opened this line,
    /// when a merge's *extra* parent did. `None` for a line that starts at the
    /// top of the page or carries a commit's first parent.
    opened_by: Option<(usize, usize)>,
    /// Whether a commit has been drawn in this line yet.
    hosted: bool,
}

/// The graph gutter for each entry, one string per row and every string the same
/// width so the subjects line up in one column.
///
/// The model is tig's: a *lane* is a column waiting for a commit, a commit claims
/// the leftmost lane waiting for it, its first parent inherits that lane, and
/// each further parent opens a column to the right. Read
/// `tui/src/workbench/git_panel.rs` for why each rule below is what it is; this
/// is the same algorithm, kept identical so both surfaces draw one picture.
fn graph(entries: &[Entry]) -> Vec<String> {
    let position: HashMap<&str, usize> = entries
        .iter()
        .enumerate()
        .map(|(at, entry)| (entry.id.as_str(), at))
        .collect();
    let mut covered: HashSet<(usize, usize)> = HashSet::new();
    let mut rows = Vec::new();
    for pass in 1..=PASSES {
        let (drawn, discovered) = draw(entries, &position, &covered);
        rows = drawn;
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
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    rows.into_iter()
        .map(|mut cells| {
            cells.resize(width, ' ');
            cells.into_iter().collect()
        })
        .collect()
}

/// One pass over the log: the rows, and the parent links whose lane turned out to
/// carry no commit of its own.
fn draw(
    entries: &[Entry],
    position: &HashMap<&str, usize>,
    covered: &HashSet<(usize, usize)>,
) -> (Vec<Vec<char>>, Vec<(usize, usize)>) {
    let mut lanes: Vec<Option<Lane>> = Vec::new();
    let mut rows: Vec<Vec<char>> = Vec::with_capacity(entries.len());
    let mut empty: Vec<(usize, usize)> = Vec::new();
    for (at, entry) in entries.iter().enumerate() {
        // Every lane waiting for this commit: the first is where it is drawn, the
        // rest are lines of work joining it here.
        let waiting: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter(|(_, lane)| lane.is_some_and(|lane| lane.target == at))
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
                    target: at,
                    opened_by: None,
                    hosted: false,
                });
                (column, Vec::new())
            }
        };
        // A line closing here that never drew a commit was opened for a parent
        // another line also reaches: its history is already on the page.
        for &column in &closed {
            if let Some(lane) = lanes[column] {
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
        let parents: Vec<usize> = entry
            .parents
            .iter()
            .map(|parent| position.get(parent.as_str()).copied().unwrap_or(GHOST))
            .collect();
        // The extra parents, each in a column right of this one: the leftmost one
        // that is free, and — this is what keeps the page narrow — a column a
        // lane is giving up on *this* row.
        let mut opened: Vec<(usize, usize, usize)> = Vec::new();
        for (index, &parent) in parents.iter().enumerate().skip(1) {
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
            opened.push((column, parent, index));
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
        let mut cells = vec![' '; 2 * (width + 1)];
        for column in 0..=width {
            let up = lanes.get(column).is_some_and(|lane| lane.is_some()) && column != mine;
            // A column a line gives up on and another takes over in the same row
            // continues below: the vertical passes through it.
            let down = if opened.iter().any(|(taken, _, _)| *taken == column) {
                true
            } else if closed.contains(&column) {
                false
            } else {
                up
            };
            let right = column >= mine && column < rightmost;
            let left = column > mine && column <= rightmost;
            cells[2 * column] = if column == mine {
                // `M` is a commit that joins lines, `o` one that only continues
                // its own — tig's two glyphs.
                if parents.len() > 1 {
                    'M'
                } else {
                    'o'
                }
            } else {
                junction(up, down, left, right)
            };
            if column >= mine && column < rightmost {
                cells[2 * column + 1] = '─';
            }
        }
        rows.push(cells);
        for &column in &closed {
            lanes[column] = None;
        }
        lanes[mine] = parents.first().map(|&target| Lane {
            target,
            opened_by: None,
            hosted: false,
        });
        for &(column, target, index) in &opened {
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
    (rows, empty)
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
