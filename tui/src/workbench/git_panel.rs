//! `/git` — a repository browser: log, refs, working tree, stashes, worktrees,
//! branches.
//!
//! The shape is tig's: one panel whose tab decides what the list holds, a
//! detail pane beside it, and the graph drawn in the log because a log without
//! its graph is a list of commits with no idea which line of work they belong
//! to. What it deliberately does not have is a write verb: nothing here stages,
//! commits, checks out, resets or drops a stash. The same rule `/diff` and
//! `/worktree` follow, for the same reason — a panel that can rewrite the
//! reader's tree is a footgun one keypress away, and every one of those verbs
//! already belongs to the agent or to a shell.
//!
//! Two tabs do reach past the current checkout without writing to it: 工作区
//! lists the repository's worktrees and `enter` moves the session to one (the
//! same `set_cwd` `/worktree` uses), and 分支 lists the local branches with
//! `enter` narrowing the log to one. Both only change what the panel reads —
//! no branch is switched, nothing is staged, no tree is touched.
//!
//! Three properties the reads keep:
//!
//! * **Bounded.** The log is one page ([`COMMIT_LIMIT`]), a patch is capped the
//!   way `/diff` caps one, and every query has a deadline — a `git` blocked on
//!   another session's `rebase` lock must not freeze the frame that asked.
//! * **Argv, never a shell.** References, shas and paths travel as argv, so a
//!   file called `$(rm -rf)` is a file.
//! * **One pass per load.** A section is read once into rows; the detail pane is
//!   a second, separate read for the selected row only, so moving the cursor
//!   costs one `git show` rather than re-reading the log.
//!
//! Field and record separators (US/RS) carry the structured fields: a commit
//! subject is free text and may contain anything a line can contain, so splitting
//! on a tab or a `|` would corrupt it.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

use super::ansi;
use super::i18n::{t, Key, Language};
use super::state::{GitKind, GitRow, GitSection, GitView};

/// How many commits one log read asks for. A page, not the history: the reader
/// scrolls a screen at a time, and `--graph` on a repository with 10⁵ commits is
/// exactly the read that should not happen on a keypress.
pub const COMMIT_LIMIT: usize = 200;

/// Budget for one read-only query (`log`, `for-each-ref`, `stash list`).
pub const QUERY_TIMEOUT: Duration = Duration::from_secs(3);

/// Budget for loading one row's detail. A patch is bounded *after* it arrives,
/// so the read itself gets room for a large commit on a slow disk.
pub const SHOW_TIMEOUT: Duration = Duration::from_secs(10);

const FIELD: char = '\u{1f}';
const RECORD: char = '\u{1e}';

/// A failed `git` that printed nothing at all. The read carries it as an empty
/// message — git's own words are content and travel verbatim — and [`spoken`]
/// turns it into the reader's copy at the one point an error is stored.
const SILENT: &str = "";

/// `git -C <cwd> <args…>` under a deadline. Non-zero exit is an error carrying
/// git's own message, which is what the panel shows instead of an empty list:
/// "not a repository" and "no commits yet" are different answers and a reader
/// can tell them apart.
fn git(cwd: &Path, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut full = vec!["-C".to_string(), cwd.display().to_string()];
    full.extend(args.iter().map(|arg| (*arg).to_string()));
    match crate::subprocess::spawn_with_timeout(Path::new("git"), &full, timeout) {
        Ok((0, stdout, _)) => Ok(stdout),
        Ok((_, _, stderr)) => {
            let message = stderr.trim();
            Err(if message.is_empty() {
                SILENT.to_string()
            } else {
                message.to_string()
            })
        }
        Err(error) => Err(error),
    }
}

/// The text a failed read is shown with: git's own message, which is content
/// and stays exactly as git wrote it, or the panel's words in `lang` when git
/// said nothing at all.
fn spoken(lang: Language, error: String) -> String {
    if error.is_empty() {
        t(lang, Key::GitNoMessage).to_string()
    } else {
        error
    }
}

/// The repository's own name — the last component of its toplevel — for the
/// box title. `Err` when `cwd` is not inside a repository (or `git` is missing).
pub fn repo_name(cwd: &Path, lang: Language) -> Result<String, String> {
    let toplevel = git(cwd, &["rev-parse", "--show-toplevel"], QUERY_TIMEOUT)?;
    let toplevel = toplevel.trim();
    if toplevel.is_empty() {
        return Err(t(lang, Key::GitNoRepo).to_string());
    }
    Ok(Path::new(toplevel)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| toplevel.to_string()))
}

/// The rows of `section`, one [`GitRow`] per painted line.
///
/// `filter` narrows the log to a revision (a branch, a tag, a sha) and is
/// ignored by every other section.
pub fn rows(cwd: &Path, section: GitSection, filter: Option<&str>) -> Result<Vec<GitRow>, String> {
    match section {
        GitSection::Commits => commits(cwd, filter),
        GitSection::Refs => refs(cwd),
        GitSection::Status => status(cwd),
        GitSection::Stash => stashes(cwd),
        GitSection::Worktrees => worktrees(cwd),
        GitSection::Branches => branches(cwd),
    }
}

/// `git log` in topological order, drawn into rows.
///
/// With no filter this is every branch, remote branch and tag as one timeline
/// — the graph is the panel's point, and a graph of one branch is a straight
/// line with no branch in it. A filter is one line of work and is read on its
/// own.
///
/// The graph is not git's own. `git log --graph` prints a commit's row and then
/// the *drawing* of the connections below it as further lines (`|\`, `|/`), so
/// the pane shows rows with no commit on them and a diagonal wherever two lanes
/// swap columns — on this repository the first 60 commits print as 102 lines,
/// 42 of them bare drawing, and the diagonals make two lines of work read as
/// one. Here every row is a commit, connections are verticals and elbows only,
/// and a lane is a fixed column for as long as it exists.
fn commits(cwd: &Path, filter: Option<&str>) -> Result<Vec<GitRow>, String> {
    // `%x1e` / `%x1f` are git's own escapes for RS and US: a subject is free
    // text, so the field separator has to be a byte a subject cannot contain.
    // `%H`/`%P` (full ids) rather than `%h`/`%p` are what the lanes are matched
    // on: an abbreviation length can change mid-log, and a lane keyed on one
    // would stop matching its own parent. `%cd`/`%cn` are the *committer's* date
    // and name — the commit's own stamp, which is what the columns beside the
    // graph are for; the author is the detail pane's.
    let pretty = "--pretty=format:%x1e%h%x1f%H%x1f%P%x1f%cd%x1f%cn%x1f%D%x1f%s";
    let limit = format!("--max-count={COMMIT_LIMIT}");
    // Topological order, in both reads: a parent never comes before a child,
    // and a line of work is read as one stretch rather than interleaved with
    // every other branch's dated commits. That is what lets a lane stay in one
    // column from the tip of a branch to where it joins — the property the
    // columns are read by. Date order is still newest-first at the tips, which
    // is where the reader starts.
    let mut args = vec![
        "log",
        "--decorate=short",
        // Minute precision, no zone: the reader is looking for "when, and by
        // whom", and a `+0800` in every one of 200 rows is six cells that only
        // ever repeat what the local clock already says.
        "--date=format:%Y-%m-%d %H:%M",
        "--topo-order",
        limit.as_str(),
        pretty,
    ];
    match filter {
        Some(reference) => args.push(reference),
        // Every branch, tag and remote-tracking branch — the three groups the
        // refs tab lists, so the log and that tab describe the same repository.
        // `--all` would also pull in `refs/stash` (a stash is a commit tree), and
        // a stash entry sitting in the log as if it were a branch is noise the
        // refs tab does not have. Naming `HEAD` as well would be the obvious way
        // to cover a detached checkout, but on an unborn branch it is a fatal
        // error — and that error takes the other branches' commits down with it.
        None => args.extend_from_slice(&["--branches", "--remotes", "--tags"]),
    }
    // `--` ends revision parsing: a branch name that also names a path (a
    // `release` *file* beside a `release` branch) would otherwise be resolved
    // as the path and log nothing.
    args.push("--");
    match git(cwd, &args, QUERY_TIMEOUT) {
        Ok(text) => Ok(parse_log(&text)),
        // A repository with no commit is not an error the reader can act on; it
        // is an empty log, and every other panel in this surface says "nothing
        // here" rather than showing git's fatal message.
        Err(error) if error.contains("does not have any commits") => Ok(Vec::new()), // i18n: matches git's own stderr, never drawn
        Err(error) => Err(error),
    }
}

/// One commit as the log read it: what the lanes are matched on (`id`,
/// `parents`, full ids) and what the row shows.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    /// Short id, the row's key and the id the reader copies.
    short: String,
    id: String,
    parents: Vec<String>,
    /// The committer's date and name, as git formatted them.
    when: String,
    who: String,
    decorations: String,
    subject: String,
}

/// Split a log read into entries: one line per commit, [`RECORD`] leading its
/// fields and [`FIELD`] between them.
fn parse_entries(text: &str) -> Vec<Entry> {
    text.lines()
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

/// A parent that lies past the page the log asked for. Its lane is kept — a
/// line running off the bottom says the history continues, which is true — but
/// it can never be claimed by a commit, so the line is never handed to another
/// branch.
const GHOST: usize = usize::MAX;

/// One line of work being drawn: the commit it is waiting for and what opened
/// it.
#[derive(Clone, Copy)]
struct Lane {
    /// The entry this line waits for, or [`GHOST`] past the page.
    target: usize,
    /// The commit and the index of the parent in its list that opened this line,
    /// when a merge's *extra* parent did. `None` for a line that starts at the
    /// top of the page or carries a commit's first parent: those are the line
    /// itself and are never dropped.
    opened_by: Option<(usize, usize)>,
    /// Whether a commit has been drawn in this line yet.
    hosted: bool,
}

/// How many times the layout may be laid out again after dropping a lane that
/// carried nothing. Dropping one link can only free a column, so the second pass
/// is normally the last; the bound is what keeps a pathological history from
/// looping rather than a budget anything real gets near.
const PASSES: usize = 4;

/// The graph gutter for each entry, one string per row and every string the
/// same width so the subjects line up in one column.
///
/// The model is tig's: a *lane* is a column waiting for a commit, a commit
/// claims the leftmost lane waiting for it, its first parent inherits that
/// lane, and each further parent opens a column to the right. Three decisions
/// make the drawing readable rather than merely correct:
///
/// * **A lane keeps its column.** A new parent takes a free column to the
///   right of the commit when there is one and only appends when there is not,
///   so opening a branch never shifts the lanes beside it — a column that
///   moved would also change colour, since a lane's colour is its column.
/// * **No diagonals and no orphan rows.** A connection is a vertical, a
///   horizontal, or an elbow between them (`┐ ┘ ┤ ┬ ┴`), and each one is drawn
///   on the row of the commit that makes it. Every row carries a commit. The
///   horizontal breaks around a lane that keeps going (`●─│─┐`) rather than
///   striking through it: a crossing mark would claim the two lines touch,
///   where the connection merely passes.
/// * **A lane that carries nothing is not drawn.** When a merge's extra parent
///   is a commit another lane already reaches — `origin/main` merged into every
///   feature branch, or a branch merged twice — the lane opened for it runs down
///   the page with no commit of its own and closes at the commit that other lane
///   lands on. That link is dropped and the page laid out again: the covered
///   line keeps its commits, they are simply drawn where they already were.
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

/// One pass over the log: the rows, and the parent links whose lane turned out
/// to carry no commit of its own.
fn draw(
    entries: &[Entry],
    position: &HashMap<&str, usize>,
    covered: &HashSet<(usize, usize)>,
) -> (Vec<Vec<char>>, Vec<(usize, usize)>) {
    // Column -> the line it holds, `None` while the column is free.
    let mut lanes: Vec<Option<Lane>> = Vec::new();
    let mut rows: Vec<Vec<char>> = Vec::with_capacity(entries.len());
    let mut empty: Vec<(usize, usize)> = Vec::new();
    for (at, entry) in entries.iter().enumerate() {
        // Every lane waiting for this commit: the first is where it is drawn,
        // the rest are lines of work joining it here.
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
        // This commit is the line's content.
        if let Some(lane) = lanes[mine].as_mut() {
            lane.hosted = true;
        }
        let parents: Vec<usize> = entry
            .parents
            .iter()
            .map(|parent| position.get(parent.as_str()).copied().unwrap_or(GHOST))
            .collect();
        // The extra parents, each in a column right of this one: the leftmost
        // one that is free, and — this is what keeps the page narrow — a column
        // a lane is giving up on *this* row. A branch that ends where the next
        // one begins hangs off the same cell as the one before it (`●─┤`), which
        // is how successive short branches stay in one column beside the line
        // instead of walking right by two every merge. A column already taken
        // by another lane, or by an earlier parent of this same commit, is
        // passed over, so two live lines never share one. One whose lane was
        // found to carry nothing is not opened at all.
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
        // The row runs to the last lane that exists, how far right this commit
        // reaches too.
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
                '●'
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
/// directions have a line (`up`, `down`, `left`, `right`), and nothing drawn
/// outside ones that do.
///
/// Every column right of the commit has a line to its *left* (the horizontal
/// comes from the commit), so `left` is only ever false on the commit's own
/// column — which is a `●`, not a junction. What is left to decide is whether
/// the horizontal goes on past the column (`right`) and whether the vertical
/// runs above or below it.
fn junction(up: bool, down: bool, left: bool, right: bool) -> char {
    if !left && !right {
        return if up || down { '│' } else { ' ' };
    }
    match (up, down, right) {
        // A lane that keeps going is drawn as it is, with the horizontal broken
        // around it: a crossing mark would claim the two lines touch.
        (true, true, true) => '│',
        (true, true, false) => '┤',
        (true, false, true) => '┴',
        (true, false, false) => '┘',
        (false, true, true) => '┬',
        (false, true, false) => '┐',
        (false, false, _) => '─',
    }
}

/// How wide the date-time column is: `2026-10-30 10:12`, minute precision and no
/// zone.
const WHEN_WIDTH: usize = 16;

/// How wide the committer column grows before a long name is clipped. Wide
/// enough for the names this repository is read by, and short enough that the
/// graph is still on screen in a box that is not.
const WHO_LIMIT: usize = 10;

/// The rows of a log read: each entry with its date-time and committer columns,
/// its graph gutter, and its subject.
///
/// The columns are padded to one width for the whole page — a lane's column is
/// read against the row above it, so the gutter has to start at the same cell in
/// every row. The id is deliberately not among them: the detail pane beside the
/// list already prints the whole hash, so repeating eight cells of it in every
/// row spends the width the subject needs.
pub fn parse_log(text: &str) -> Vec<GitRow> {
    use GitKind::{Decoration, Plain};
    let entries = parse_entries(text);
    let prefixes = graph(&entries);
    let who_width = entries
        .iter()
        .map(|entry| ansi::width(&entry.who))
        .max()
        .unwrap_or(0)
        .clamp(0, WHO_LIMIT);
    entries
        .into_iter()
        .zip(prefixes)
        .map(|(entry, prefix)| {
            // `fit` pads to the width and clips a long name with an ellipsis, so
            // every column ends on the same cell in every row.
            let mut columns = Vec::new();
            if !entry.when.is_empty() {
                columns.push(ansi::fit(&entry.when, WHEN_WIDTH));
            }
            if !entry.who.is_empty() {
                columns.push(ansi::fit(&entry.who, who_width));
            }
            // The refs lead and the subject follows, the way tig reads a log:
            // `HEAD -> dev-huichen, origin/dev-huichen · Merge branch …` says
            // which line of work the row belongs to before what the commit did.
            // A commit with no refs on it starts at the subject, which is then
            // the first thing in the row.
            let mut pieces = Vec::new();
            let mut kinds = Vec::new();
            for (piece, kind) in [
                (entry.decorations.as_str(), Decoration),
                (entry.subject.as_str(), Plain),
            ] {
                if !piece.is_empty() {
                    pieces.push(piece.to_string());
                    kinds.push(kind);
                }
            }
            GitRow {
                key: Some(entry.short),
                columns,
                prefix,
                pieces,
                kinds,
            }
        })
        .collect()
}

/// Branches, remote-tracking branches and tags, most recently committed first.
fn refs(cwd: &Path) -> Result<Vec<GitRow>, String> {
    // `for-each-ref` has no `%xNN` escape (that is a *pretty format* feature),
    // so the separator is the unit separator itself, as a byte of the format
    // string. A ref name cannot contain it, which is what makes the split safe.
    const FORMAT: &str = "--format=%(refname)\u{1f}%(refname:short)\u{1f}%(objectname:short)\u{1f}\
%(committerdate:short)\u{1f}%(upstream:short)\u{1f}%(upstream:track)\u{1f}%(HEAD)\u{1f}%(subject)";
    let text = git(
        cwd,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            FORMAT,
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ],
        QUERY_TIMEOUT,
    )?;
    Ok(parse_refs(&text))
}

/// Parse [`refs`]' output. The row's key is the *full* ref name, so a short name
/// that is ambiguous later (`main` the branch, `main` the tag) can never make
/// the detail pane read a different object than the painted row.
pub fn parse_refs(text: &str) -> Vec<GitRow> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let fields: Vec<&str> = line.split(FIELD).collect();
            let full = fields.first().copied().unwrap_or("").to_string();
            let short = fields.get(1).copied().unwrap_or(&full);
            let sha = fields.get(2).copied().unwrap_or("");
            let date = fields.get(3).copied().unwrap_or("");
            let upstream = fields.get(4).copied().unwrap_or("");
            let track = fields.get(5).copied().unwrap_or("");
            let head = fields.get(6).copied().unwrap_or("");
            let subject = fields.get(7).copied().unwrap_or("").trim();
            // The group is part of the label because `main` and a tag named
            // `main` are two different refs, and `%(refname:short)` prints both
            // the same way. A branch keeps its bare name (that is what the
            // reader types); the other two say what they are.
            let name = if let Some(tag) = full.strip_prefix("refs/tags/") {
                format!("tag: {tag}")
            } else if let Some(remote) = full.strip_prefix("refs/remotes/") {
                remote.to_string()
            } else {
                short.to_string()
            };
            let mut pieces = Vec::new();
            let mut kinds = Vec::new();
            for (piece, kind) in [
                (name.as_str(), GitKind::Ref),
                (track, GitKind::Meta),
                (upstream, GitKind::Meta),
                (sha, GitKind::Sha),
                (date, GitKind::Meta),
                (subject, GitKind::Plain),
            ] {
                if !piece.is_empty() {
                    pieces.push(piece.to_string());
                    kinds.push(kind);
                }
            }
            GitRow {
                key: Some(full),
                // `●` marks the branch this checkout is on, the same badge
                // `/worktree` uses for the worktree the session is in.
                columns: Vec::new(),
                prefix: if head == "*" {
                    "● ".to_string()
                } else {
                    String::new()
                },
                pieces,
                kinds,
            }
        })
        .collect()
}

/// The repository's checkouts, from `git worktree list --porcelain`: the main
/// worktree first, then every other, each with the branch it holds.
///
/// A worktree row is a *place*, not a commit: `enter` moves the session's
/// workspace there (`set_cwd`, the verb `/worktree` already uses), which is the
/// only way this panel reads another checkout — the reads themselves stay
/// rooted in one directory.
fn worktrees(cwd: &Path) -> Result<Vec<GitRow>, String> {
    let text = git(cwd, &["worktree", "list", "--porcelain"], QUERY_TIMEOUT)?;
    Ok(parse_worktrees(&text, cwd))
}

/// Parse `git worktree list --porcelain` into one row per worktree.
///
/// Porcelain prints blank-line-separated blocks of `key value` lines; the keys
/// this panel reads are `worktree`, `branch`, `detached`, `bare` and `locked`.
/// The block's path is both the key and the row's own name — a worktree is
/// identified by where it is, and two branches with the same short name can be
/// checked out in two places.
pub fn parse_worktrees(text: &str, cwd: &Path) -> Vec<GitRow> {
    let current = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let mut rows = Vec::new();
    // A block is the stretch between blank lines; the trailing blank line
    // leaves one empty stretch, which has no `worktree` line and is skipped.
    for block in text.split("\n\n") {
        let mut path = None;
        let mut branch = None;
        let mut detached = false;
        let mut bare = false;
        let mut locked = false;
        for line in block.lines() {
            if let Some(rest) = line.strip_prefix("worktree ") {
                path = Some(rest.trim().to_string());
            } else if let Some(rest) = line.strip_prefix("branch ") {
                let name = rest.trim();
                branch = Some(name.strip_prefix("refs/heads/").unwrap_or(name).to_string());
            } else if line == "detached" {
                detached = true;
            } else if line == "bare" {
                bare = true;
            } else if line == "locked" || line.starts_with("locked ") {
                locked = true;
            }
        }
        let Some(path) = path else {
            continue;
        };
        // What the worktree *holds* leads: the branch is the reason a reader
        // looks at this tab, and the path is the long part beside it.
        let state = match (branch, detached, bare) {
            (Some(branch), _, _) => branch,
            (None, true, _) => "detached".to_string(),
            (None, _, true) => "bare".to_string(),
            (None, _, _) => String::new(),
        };
        let mut pieces = Vec::new();
        let mut kinds = Vec::new();
        if !state.is_empty() {
            pieces.push(state);
            kinds.push(GitKind::Ref);
        }
        pieces.push(path.clone());
        kinds.push(GitKind::Meta);
        if locked {
            pieces.push("locked".to_string());
            kinds.push(GitKind::Meta);
        }
        // Both spellings of the same place: the path git printed and the path
        // the session was moved to may differ by a symlink or a `..`.
        let here = Path::new(&path)
            .canonicalize()
            .map(|resolved| resolved == current)
            .unwrap_or(false)
            || Path::new(&path) == cwd;
        rows.push(GitRow {
            key: Some(path),
            columns: Vec::new(),
            prefix: if here {
                "● ".to_string()
            } else {
                String::new()
            },
            pieces,
            kinds,
        });
    }
    rows
}

/// Local branches, most recently committed first.
///
/// The refs tab lists everything (`refs/heads`, `refs/remotes`, `refs/tags`) in
/// one timeline. This one is the working set: only the branches this checkout
/// can enter, named the way a reader types them, keyed by that short name so
/// the log filter it feeds is the same string.
fn branches(cwd: &Path) -> Result<Vec<GitRow>, String> {
    const FORMAT: &str =
        "--format=%(refname:short)\u{1f}%(objectname:short)\u{1f}%(committerdate:short)\u{1f}%(upstream:short)\u{1f}%(upstream:track)\u{1f}%(HEAD)\u{1f}%(subject)";
    let text = git(
        cwd,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            FORMAT,
            "refs/heads",
        ],
        QUERY_TIMEOUT,
    )?;
    Ok(parse_branches(&text))
}

/// Parse [`branches`]' output into `name · upstream · sha · date · subject`
/// rows keyed by the short branch name, which is what the log filter and
/// `git show` both take.
pub fn parse_branches(text: &str) -> Vec<GitRow> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let fields: Vec<&str> = line.split(FIELD).collect();
            let name = fields.first().copied().unwrap_or("").to_string();
            let sha = fields.get(1).copied().unwrap_or("");
            let date = fields.get(2).copied().unwrap_or("");
            let upstream = fields.get(3).copied().unwrap_or("");
            let track = fields.get(4).copied().unwrap_or("");
            let head = fields.get(5).copied().unwrap_or("");
            let subject = fields.get(6).copied().unwrap_or("").trim();
            let mut pieces = Vec::new();
            let mut kinds = Vec::new();
            for (piece, kind) in [
                (name.as_str(), GitKind::Ref),
                (track, GitKind::Meta),
                (upstream, GitKind::Meta),
                (sha, GitKind::Sha),
                (date, GitKind::Meta),
                (subject, GitKind::Plain),
            ] {
                if !piece.is_empty() {
                    pieces.push(piece.to_string());
                    kinds.push(kind);
                }
            }
            GitRow {
                key: Some(name),
                columns: Vec::new(),
                prefix: if head == "*" {
                    "● ".to_string()
                } else {
                    String::new()
                },
                pieces,
                kinds,
            }
        })
        .collect()
}

/// The working tree, from the same porcelain read `/diff` uses: tracked edits
/// and untracked files, in git's order.
fn status(cwd: &Path) -> Result<Vec<GitRow>, String> {
    let files = super::diff_review::files(cwd)?;
    Ok(files
        .into_iter()
        .map(|file| GitRow {
            key: Some(file.path.clone()),
            columns: Vec::new(),
            prefix: format!("{} ", file.status),
            pieces: vec![file.path],
            kinds: vec![GitKind::Plain],
        })
        .collect())
}

/// `git stash list`, newest first (git's own order).
fn stashes(cwd: &Path) -> Result<Vec<GitRow>, String> {
    let text = git(
        cwd,
        &[
            "stash",
            "list",
            "--date=short",
            "--pretty=format:%gd%x1f%ad%x1f%s",
        ],
        QUERY_TIMEOUT,
    )?;
    Ok(parse_stashes(&text))
}

/// Parse [`stashes`]' output into `#N · message · date` rows keyed by
/// `stash@{N}`, which is what every stash verb takes.
pub fn parse_stashes(text: &str) -> Vec<GitRow> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start_matches('\u{1f}');
        if trimmed.trim().is_empty() {
            continue;
        }
        let index = rows.len();
        let fields: Vec<&str> = trimmed.split(FIELD).collect();
        let reference = fields.first().copied().unwrap_or("").to_string();
        let date = fields.get(1).copied().unwrap_or("");
        let message = fields.get(2).copied().unwrap_or("").trim();
        let mut pieces = Vec::new();
        let mut kinds = Vec::new();
        for (piece, kind) in [(message, GitKind::Plain), (date, GitKind::Meta)] {
            if !piece.is_empty() {
                pieces.push(piece.to_string());
                kinds.push(kind);
            }
        }
        rows.push(GitRow {
            key: Some(reference),
            columns: Vec::new(),
            prefix: format!("#{index} "),
            pieces,
            kinds,
        });
    }
    rows
}

/// What `y` puts on the clipboard for a row: the sha, the path, the stash ref,
/// or a ref's short name — what a reader would type, not the lookup key the
/// panel stores (`refs/remotes/origin/main` copies as `origin/main`).
pub fn copy_text(section: GitSection, key: &str) -> String {
    if section != GitSection::Refs {
        return key.to_string();
    }
    for prefix in ["refs/heads/", "refs/remotes/", "refs/tags/"] {
        if let Some(short) = key.strip_prefix(prefix) {
            return short.to_string();
        }
    }
    key.to_string()
}

/// The detail pane for one row: a commit's stat and patch, a ref's tip, a file's
/// working-tree patch, or a stash's patch. An error arrives in the reader's
/// language, ready to be painted.
pub fn detail(
    cwd: &Path,
    section: GitSection,
    key: &str,
    lang: Language,
) -> Result<Vec<String>, String> {
    let lines = match section {
        GitSection::Commits => {
            let text = git(
                cwd,
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
            Ok(bounded_lines(&text))
        }
        GitSection::Refs => ref_detail(cwd, key, lang),
        GitSection::Status => {
            let review = super::diff_review::load(cwd, key)?;
            Ok(bounded_lines(&review.patch))
        }
        GitSection::Stash => stash_detail(cwd, key),
        GitSection::Branches => ref_detail(cwd, key, lang),
        GitSection::Worktrees => worktree_detail(key, lang),
    };
    lines.map_err(|error| spoken(lang, error))
}

/// A worktree's own record: its status and its tip, read *in* that directory.
///
/// The row's key is the worktree's path, so this is the one detail read that
/// starts somewhere other than the session's workspace — and it is still a
/// read: `status --short --branch` and `log -1` write nothing. A path that has
/// been deleted out from under the list answers with git's own error, which is
/// the truth the reader needs.
fn worktree_detail(path: &str, lang: Language) -> Result<Vec<String>, String> {
    let dir = Path::new(path);
    let status = git(dir, &["status", "--short", "--branch"], SHOW_TIMEOUT)?;
    let mut lines = bounded_lines(&status);
    let tip = git(
        dir,
        &[
            "log",
            "-1",
            "--no-color",
            "--no-ext-diff",
            "--date=short",
            "--stat",
            t(lang, Key::GitTipFormat),
            "--",
        ],
        SHOW_TIMEOUT,
    )?;
    if !lines.is_empty() && !tip.trim().is_empty() {
        lines.push(String::new());
    }
    lines.extend(bounded_lines(&tip));
    Ok(lines)
}

/// A ref's own record: which ref it is, how its upstream stands, and its tip.
fn ref_detail(cwd: &Path, key: &str, lang: Language) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    if let Ok(full) = git(
        cwd,
        &["rev-parse", "--symbolic-full-name", key],
        QUERY_TIMEOUT,
    ) {
        let full = full.trim();
        if !full.is_empty() {
            lines.push(format!("ref  {full}"));
        }
    }
    // `key@{upstream}` is a revision expression, not shell syntax; it only
    // resolves for a local branch that tracks something, and a tag answering
    // with an error is simply a ref with no upstream.
    if let Ok(counts) = git(
        cwd,
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("{key}...@{{upstream}}"),
        ],
        QUERY_TIMEOUT,
    ) {
        let counts = counts.trim();
        if let Some((ahead, behind)) = counts.split_once('\t') {
            lines.push(super::i18n::render(
                lang,
                Key::GitAheadBehind,
                &[&ahead, &behind],
            ));
        }
    }
    let tip = git(
        cwd,
        &[
            "show",
            "-s",
            "--no-color",
            "--date=short",
            t(lang, Key::GitTipFormat),
            key,
            "--",
        ],
        SHOW_TIMEOUT,
    )?;
    if !lines.is_empty() {
        lines.push(String::new());
    }
    lines.extend(tip.lines().map(str::to_string));
    Ok(lines)
}

/// A stash's patch. `stash show -p` is the documented door; a stash entry that
/// is not shaped like one (`git stash store` of an odd object) still shows
/// through `git show`, which is why the fallback exists.
fn stash_detail(cwd: &Path, key: &str) -> Result<Vec<String>, String> {
    let patch = git(
        cwd,
        &[
            "stash",
            "show",
            "--patch",
            "--no-color",
            "--no-ext-diff",
            key,
        ],
        SHOW_TIMEOUT,
    )
    .or_else(|_| {
        git(
            cwd,
            &[
                "show",
                "--patch",
                "--stat",
                "--no-color",
                "--no-ext-diff",
                key,
                "--",
            ],
            SHOW_TIMEOUT,
        )
    })?;
    Ok(bounded_lines(&patch))
}

/// Read one section for `cwd`, put the selection on the nearest selectable row
/// to `want` and load that row's detail — the whole panel in one call, which is
/// what opening it, switching a tab and `r` all need.
pub fn read(
    cwd: &Path,
    section: GitSection,
    filter: Option<String>,
    want: usize,
    lang: Language,
) -> GitView {
    let mut view = GitView {
        section,
        filter,
        ..GitView::default()
    };
    match repo_name(cwd, lang) {
        Ok(repo) => view.repo = repo,
        Err(error) => {
            view.error = Some(spoken(lang, error));
            return view;
        }
    }
    match rows(cwd, section, view.filter.as_deref()) {
        Ok(rows) => view.rows = rows,
        Err(error) => {
            view.error = Some(spoken(lang, error));
            return view;
        }
    }
    focus(cwd, &mut view, want, lang);
    view
}

/// Move `view`'s selection to the nearest selectable row at or after `want` and
/// re-read that row's detail. A cursor move costs one `git show`, not another
/// log read.
pub fn focus(cwd: &Path, view: &mut GitView, want: usize, lang: Language) {
    view.selected = view.nearest(want, true);
    view.detail.clear();
    if let Some(key) = view.rows.get(view.selected).and_then(|row| row.key.clone()) {
        match detail(cwd, view.section, &key, lang) {
            Ok(lines) => view.detail = lines,
            Err(error) => view.detail = vec![spoken(lang, error)],
        }
    }
}

/// A detail body, capped the way `/diff` caps a patch. The cap lives in
/// `diff_review` because one message and one truncation marker are better than
/// two that drift.
fn bounded_lines(patch: &str) -> Vec<String> {
    super::diff_review::bounded_patch(patch)
        .lines()
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            output
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "tui@example.invalid"]);
        git(&["config", "user.name", "TUI test"]);
        std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "first commit"]);
        // A second line of work, so the graph has a merge in it.
        git(&["checkout", "-q", "-b", "feature"]);
        std::fs::write(dir.path().join("b.txt"), "two\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "feature commit"]);
        git(&["checkout", "-q", "main"]);
        std::fs::write(dir.path().join("a.txt"), "changed\n").unwrap();
        git(&["commit", "-qam", "second commit"]);
        git(&["merge", "-q", "--no-ff", "-m", "merge feature", "feature"]);
        git(&["tag", "v1"]);
        std::fs::write(dir.path().join("a.txt"), "dirty\n").unwrap();
        std::fs::write(dir.path().join("untracked file.txt"), "new\n").unwrap();
        git(&["stash", "push", "-q", "-m", "a stashed change"]);
        // The stash takes the edit with it; write it again so the working tree
        // has an edited *and* an untracked file to list.
        std::fs::write(dir.path().join("a.txt"), "dirty\n").unwrap();
        dir
    }

    /// One line of the read per commit, and the fields stay whole: a subject
    /// with a `|` or a tab is still one subject. The date-time and committer
    /// travel beside the graph, in columns of one width; the id does not appear
    /// at all — the detail pane names the whole commit.
    #[test]
    fn log_lines_split_on_the_record_separator() {
        let rows = parse_log(
            "\u{1e}abc1234\u{1f}aaaaaaa\u{1f}\u{1f}2026-10-27 09:15\u{1f}Ada\u{1f}HEAD -> main, tag: v1\u{1f}fix | the parser\n\
             \u{1e}def5678\u{1f}bbbbbbb\u{1f}aaaaaaa\u{1f}2026-10-26 18:40\u{1f}Bob\u{1f}\u{1f}a\ttab and a unit separator\n",
        );
        assert_eq!(rows.len(), 2, "one row per commit, none for the drawing");
        assert_eq!(rows[0].prefix, "● ");
        assert_eq!(rows[0].key.as_deref(), Some("abc1234"));
        assert_eq!(
            rows[0].pieces[0], "HEAD -> main, tag: v1",
            "the refs say which line of work the row is on, before what it did"
        );
        assert_eq!(rows[0].pieces[1], "fix | the parser");
        assert!(
            !rows[0].pieces.iter().any(|piece| piece.contains("abc1234")),
            "the id is the detail pane's: {:?}",
            rows[0].pieces
        );
        assert_eq!(rows[1].pieces[0], "a\ttab and a unit separator");
        // The empty decoration was dropped rather than painted as ` · `.
        assert!(!rows[1].pieces.iter().any(|piece| piece.is_empty()));
        assert_eq!(rows[1].pieces.len(), 1, "subject alone");
        // Two cells: the date-time (fixed 16) and the widest committer. Padded
        // to one width for the page, so the graph starts at one cell every row.
        assert_eq!(rows[0].columns, vec!["2026-10-27 09:15", "Ada"]);
        assert_eq!(rows[1].columns, vec!["2026-10-26 18:40", "Bob"]);
        assert_eq!(
            rows[0]
                .columns
                .iter()
                .map(|cell| cell.chars().count())
                .sum::<usize>(),
            rows[1]
                .columns
                .iter()
                .map(|cell| cell.chars().count())
                .sum::<usize>()
        );
    }

    /// A long committer name is clipped to the column instead of pushing the
    /// graph off the cell its lane lives in.
    #[test]
    fn a_long_committer_name_keeps_the_column() {
        let rows = parse_log(&log(&[("a", "b"), ("b", "")]));
        // `log()` writes `Ada`; a real one goes through the same `fit`.
        let long = parse_log(
            "\u{1e}c\u{1f}cccc\u{1f}\u{1f}2026-10-27 09:15\u{1f}A committer with a very long name\u{1f}\u{1f}s\n",
        );
        assert_eq!(ansi::width(&long[0].columns[1]), WHO_LIMIT);
        assert!(long[0].columns[1].contains('…'), "{:?}", long[0].columns);
        assert_eq!(rows[0].columns, vec!["2026-10-27 09:15", "Ada"]);
    }

    /// A log of `(short, parents)` pairs, in the read's own text form.
    fn log(commits: &[(&str, &str)]) -> String {
        commits
            .iter()
            .map(|(short, parents)| {
                format!(
                    "\u{1e}{short}\u{1f}{short}\u{1f}{parents}\u{1f}2026-10-27 09:15\u{1f}Ada\u{1f}\u{1f}{short}\n"
                )
            })
            .collect()
    }

    /// The gutter of a log read, one string per commit.
    fn gutters(commits: &[(&str, &str)]) -> Vec<String> {
        parse_log(&log(commits))
            .into_iter()
            .map(|row| row.prefix)
            .collect()
    }

    /// A straight history is one column; a merge opens a column to its right
    /// and the fork that closes the branch turns back into the commit's own
    /// column — with verticals, horizontals and elbows, never a diagonal.
    #[test]
    fn a_merge_opens_a_column_and_a_fork_closes_it() {
        assert_eq!(
            gutters(&[("c", "b"), ("b", "a"), ("a", "")]),
            vec!["● ", "● ", "● "]
        );
        assert_eq!(
            gutters(&[("m", "x y"), ("x", "b"), ("y", "b"), ("b", "")]),
            vec!["●─┐ ", "● │ ", "│ ● ", "●─┘ "]
        );
    }

    /// A second parent takes a free column right of the commit and passes over
    /// the lanes in between, so the lane it walks past keeps its column — and
    /// gets a junction, not a gap.
    #[test]
    fn a_connection_crosses_a_lane_without_moving_it() {
        assert_eq!(
            gutters(&[("m", "x y"), ("x", "p q"), ("y", ""), ("p", ""), ("q", ""),]),
            vec!["●─┐   ", "●─│─┐ ", "│ ● │ ", "●   │ ", "    ● "]
        );
        // Two lanes joining one commit turn into it side by side.
        assert_eq!(
            gutters(&[
                ("m", "a b c"),
                ("a", "d"),
                ("b", "d"),
                ("c", "d"),
                ("d", ""),
            ]),
            vec!["●─┬─┐ ", "● │ │ ", "│ ● │ ", "│ │ ● ", "●─┴─┘ "]
        );
    }

    /// A branch that starts where the one before it ends hangs off the same
    /// cell: the merge closes a lane with `┤` and the next branch opens in that
    /// column, so a run of short branches stays two columns wide instead of
    /// walking right by two at every merge (tig's shape).
    #[test]
    fn a_branch_that_starts_where_another_ends_keeps_the_column() {
        assert_eq!(
            gutters(&[
                ("m2", "b2 d2"),
                ("b2", "m1"),
                ("d2", "m1"),
                ("m1", "b1 d1"),
                ("b1", "m0"),
                ("d1", "m0"),
                ("m0", ""),
            ]),
            vec!["●─┐ ", "● │ ", "│ ● ", "●─┤ ", "● │ ", "│ ● ", "●─┘ "]
        );
    }

    /// A merge's extra parent opens a lane of its own — unless that parent is a
    /// commit another lane already reaches. Then the lane would run down the
    /// page with no commit of its own and close where the other lane lands, so
    /// the link is dropped and the page is laid out again: the covered line is
    /// drawn once, where it already was.
    #[test]
    fn a_branch_another_line_already_draws_gets_no_lane() {
        // `a` merges `c`, which its own first parent already goes through:
        // `b`'s parent is `c` as well, so `c` is drawn on the line `a` and `b`
        // share.
        assert_eq!(
            gutters(&[("a", "b c"), ("b", "c"), ("c", "")]),
            vec!["● ", "● ", "● "],
            "the covered parent is not a lane of its own"
        );
        // The same shape with a commit that only that lane carries keeps the
        // lane: it is real work, not a duplicate of a line already drawn.
        assert_eq!(
            gutters(&[("a", "b c"), ("b", "d"), ("c", "d"), ("d", "")]),
            vec!["●─┐ ", "● │ ", "│ ● ", "●─┘ "]
        );
    }

    /// A lane that is handed a commit keeps its column, so a branch is one
    /// colour from its tip to where it joins, and every row is as wide as the
    /// whole log so the subjects share one column.
    #[test]
    fn every_row_is_a_commit_and_every_gutter_is_as_wide() {
        let commits = [("m", "x y"), ("x", "a"), ("y", "a"), ("a", "b"), ("b", "")];
        let rows = parse_log(&log(&commits));
        assert_eq!(rows.len(), commits.len());
        assert!(rows.iter().all(|row| row.key.is_some()), "{rows:?}");
        let width = rows[0].prefix.chars().count();
        assert!(rows.iter().all(|row| row.prefix.chars().count() == width));
        for row in &rows {
            assert!(
                !row.prefix.contains(['/', '\\', '*', '|', '_']),
                "git's own vocabulary is gone: {:?}",
                row.prefix
            );
        }
    }

    #[test]
    fn refs_keep_their_group_and_full_name() {
        let rows = parse_refs(
            "refs/heads/main\u{1f}main\u{1f}abc1234\u{1f}2026-10-27\u{1f}\u{1f}\u{1f}*\u{1f}second commit\n\
             refs/remotes/origin/main\u{1f}origin/main\u{1f}abc1234\u{1f}2026-10-27\u{1f}\u{1f}\u{1f}\u{1f}second commit\n\
             refs/tags/main\u{1f}main\u{1f}def5678\u{1f}2026-10-26\u{1f}\u{1f}\u{1f}\u{1f}a tag named main\n",
        );
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].prefix, "● ", "the checked-out branch is marked");
        assert_eq!(rows[0].pieces[0], "main");
        assert_eq!(rows[0].key.as_deref(), Some("refs/heads/main"));
        assert_eq!(rows[1].pieces[0], "origin/main");
        // A tag that shares a branch's short name must stay distinguishable.
        assert_eq!(rows[2].pieces[0], "tag: main");
        assert_eq!(rows[2].key.as_deref(), Some("refs/tags/main"));
    }

    #[test]
    fn read_lands_on_a_selectable_row_and_loads_its_detail() {
        let dir = repo();
        let view = read(dir.path(), GitSection::Commits, None, 0, Language::En);
        assert!(view.error.is_none(), "{:?}", view.error);
        assert!(!view.repo.is_empty());
        assert!(view.rows[view.selected].key.is_some());
        assert!(!view.detail.is_empty());

        // The selection follows the section, and a filter narrows the log.
        let refs = read(dir.path(), GitSection::Refs, None, 0, Language::En);
        assert_eq!(refs.section, GitSection::Refs);
        assert!(refs.detail.join("\n").contains("commit "));
        let filtered = read(
            dir.path(),
            GitSection::Commits,
            Some("v1".into()),
            0,
            Language::En,
        );
        assert_eq!(filtered.filter.as_deref(), Some("v1"));
        assert!(filtered
            .rows
            .iter()
            .flat_map(|row| row.pieces.iter())
            .any(|piece| piece == "merge feature"));

        // A requested row that is not selectable (the graph's continuation)
        // lands on the next one that is.
        let mut view = read(dir.path(), GitSection::Commits, None, 0, Language::En);
        if let Some(index) = view.rows.iter().position(|row| row.key.is_none()) {
            focus(dir.path(), &mut view, index, Language::En);
            assert!(view.rows[view.selected].key.is_some());
        }
    }

    #[test]
    fn copy_text_gives_the_name_a_reader_would_type() {
        assert_eq!(copy_text(GitSection::Refs, "refs/heads/feature"), "feature");
        assert_eq!(
            copy_text(GitSection::Refs, "refs/remotes/origin/feature"),
            "origin/feature"
        );
        assert_eq!(copy_text(GitSection::Refs, "refs/tags/v1"), "v1");
        assert_eq!(copy_text(GitSection::Commits, "abc1234"), "abc1234");
        assert_eq!(
            copy_text(GitSection::Status, "src/a file.rs"),
            "src/a file.rs"
        );
    }

    #[test]
    fn stashes_are_keyed_by_their_reference() {
        let rows = parse_stashes(
            "stash@{0}\u{1f}2026-10-27\u{1f}a stashed change\n\
             stash@{1}\u{1f}2026-10-26\u{1f}older\n",
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].prefix, "#0 ");
        assert_eq!(rows[0].key.as_deref(), Some("stash@{0}"));
        assert_eq!(rows[0].pieces, vec!["a stashed change", "2026-10-27"]);
        // The number is the row's position, not its arrival: a skipped blank
        // line must not renumber the rest.
        assert_eq!(rows[1].prefix, "#1 ");
    }

    #[test]
    #[ignore]
    fn dump_real_log() {
        let dir = std::path::Path::new("/Users/geilige/future-os");
        let rows = rows(dir, GitSection::Commits, None).unwrap();
        let width = rows
            .iter()
            .map(|r| r.prefix.chars().count())
            .max()
            .unwrap_or(0);
        println!("gutter {width}");
        for r in rows.iter().take(80) {
            println!("{}|{}", r.prefix, r.pieces.join(" · "));
        }
    }

    /// The read the panel performs, against a real repository: all four
    /// sections list something, and each row's detail loads.
    #[test]
    fn a_real_repository_fills_every_section() {
        let dir = repo();
        let name = repo_name(dir.path(), Language::En).unwrap();
        assert!(!name.is_empty());

        let log = rows(dir.path(), GitSection::Commits, None).unwrap();
        assert_eq!(
            log.len(),
            4,
            "one row per commit: first, second, feature, merge"
        );
        assert!(log.iter().all(|row| row.key.is_some()), "{log:?}");
        let commits: Vec<&GitRow> = log.iter().filter(|row| row.key.is_some()).collect();
        assert_eq!(
            commits[0].pieces[0], "HEAD -> main, tag: v1",
            "the tagged merge leads with its refs"
        );
        assert_eq!(commits[0].pieces[1], "merge feature");
        // The columns carry the committer's own stamp, minute precision and no
        // zone, in one width for every row.
        let columns = commits[0].columns.join("  ");
        assert!(columns.starts_with("20"), "{columns:?}");
        assert!(
            columns.contains("TUI test"),
            "the committer, not the author: {columns:?}"
        );
        assert!(
            !columns.contains('+') && !columns.contains('Z'),
            "no zone: {columns:?}"
        );
        // The graph is drawn: the merge's row opens a lane beside its own, and
        // the gutter is one width for every row so the subjects line up.
        assert!(
            commits[0].prefix.starts_with("●─"),
            "{:?}",
            commits[0].prefix
        );
        let width = commits[0].prefix.chars().count();
        assert!(commits
            .iter()
            .all(|row| row.prefix.chars().count() == width));
        let patch = detail(
            dir.path(),
            GitSection::Commits,
            commits[0].key.as_deref().unwrap(),
            Language::En,
        )
        .unwrap()
        .join("\n");
        assert!(patch.contains("merge feature"), "{patch}");
        assert!(
            patch.contains("b.txt"),
            "the merge's stat is there: {patch}"
        );

        let refs = rows(dir.path(), GitSection::Refs, None).unwrap();
        let names: Vec<&str> = refs.iter().map(|row| row.pieces[0].as_str()).collect();
        assert!(names.contains(&"main"), "{names:?}");
        assert!(names.contains(&"feature"), "{names:?}");
        assert!(names.contains(&"tag: v1"), "{names:?}");
        let head = refs
            .iter()
            .find(|row| row.prefix == "● ")
            .expect("the checked-out branch is marked");
        let tip = detail(
            dir.path(),
            GitSection::Refs,
            head.key.as_deref().unwrap(),
            Language::En,
        )
        .unwrap();
        assert!(tip.join("\n").contains("refs/heads/main"), "{tip:?}");

        let status_rows = rows(dir.path(), GitSection::Status, None).unwrap();
        let dirty = status_rows
            .iter()
            .find(|row| row.pieces[0] == "a.txt")
            .expect("the edited file is listed");
        let patch = detail(
            dir.path(),
            GitSection::Status,
            dirty.key.as_deref().unwrap(),
            Language::En,
        )
        .unwrap()
        .join("\n");
        assert!(patch.contains("+dirty"), "{patch}");

        let stashes = rows(dir.path(), GitSection::Stash, None).unwrap();
        assert_eq!(stashes.len(), 1);
        assert!(
            stashes[0].pieces[0].contains("a stashed change"),
            "{:?}",
            stashes[0].pieces
        );
        let patch = detail(dir.path(), GitSection::Stash, "stash@{0}", Language::En)
            .unwrap()
            .join("\n");
        assert!(patch.contains("+dirty"), "{patch}");

        // A ref filter narrows the log to that line of work.
        let feature = rows(dir.path(), GitSection::Commits, Some("feature")).unwrap();
        assert!(feature
            .iter()
            .flat_map(|row| row.pieces.iter())
            .any(|piece| piece == "feature commit"));
        assert!(!feature
            .iter()
            .flat_map(|row| row.pieces.iter())
            .any(|piece| piece == "second commit"));
    }

    /// Worktree blocks: the branch leads, the path is the key, and the checkout
    /// the session is in is marked — including a detached one and a locked one.
    #[test]
    fn worktree_blocks_keep_their_branch_and_mark_this_checkout() {
        let cwd = Path::new("/w/future-os/.worktrees/git-panel-layout");
        let rows = parse_worktrees(
            "worktree /w/future-os\nHEAD abc1234\nbranch refs/heads/dev-huichen\n\n\
             worktree /w/future-os/.worktrees/git-panel-layout\nHEAD def5678\nbranch refs/heads/feat/git-panel-layout\n\n\
             worktree /w/detached\nHEAD 1234567\ndetached\n\n\
             worktree /w/bare\nbare\n\n\
             worktree /w/locked\nHEAD 89abcde\ndetached\nlocked another session\n",
            cwd,
        );
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].pieces[0], "dev-huichen", "the branch leads");
        assert_eq!(rows[0].key.as_deref(), Some("/w/future-os"));
        assert_eq!(rows[0].prefix, "", "the main worktree is elsewhere");
        assert_eq!(rows[1].prefix, "● ", "this checkout is marked");
        assert_eq!(rows[1].pieces[1], cwd.display().to_string());
        assert_eq!(rows[2].pieces[0], "detached");
        assert_eq!(rows[2].pieces.len(), 2, "a detached worktree has no branch");
        assert_eq!(rows[3].pieces[0], "bare");
        assert_eq!(rows[4].pieces.last().map(String::as_str), Some("locked"));
    }

    /// Branch rows are keyed by the short name — the same string the log filter
    /// and `git show` take — and mark the branch this checkout is on.
    #[test]
    fn branch_rows_keep_the_short_name_the_log_filter_takes() {
        let rows = parse_branches(
            "dev-huichen\u{1f}abc1234\u{1f}2026-10-27\u{1f}origin/dev-huichen\u{1f}[ahead 2]\u{1f}*\u{1f}second commit\n\
             feature\u{1f}def5678\u{1f}2026-10-26\u{1f}\u{1f}\u{1f}\u{1f}a feature\n",
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].key.as_deref(), Some("dev-huichen"));
        assert_eq!(rows[0].prefix, "● ");
        assert_eq!(
            rows[0].pieces,
            vec![
                "dev-huichen",
                "[ahead 2]",
                "origin/dev-huichen",
                "abc1234",
                "2026-10-27",
                "second commit"
            ]
        );
        assert_eq!(rows[1].prefix, "");
        assert_eq!(rows[1].key.as_deref(), Some("feature"));
        // The empty upstream fields were dropped rather than painted as gaps.
        assert!(!rows[1].pieces.iter().any(|piece| piece.is_empty()));
    }

    /// The two new sections against a real repository: a worktree added beside
    /// this one is listed and its detail reads *in* that checkout, and the
    /// branches are the local ones with the current one marked.
    #[test]
    fn a_real_repository_lists_worktrees_and_branches() {
        let dir = repo();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            output
        };
        git(&["worktree", "add", "-q", "second", "feature"]);

        let worktrees = rows(dir.path(), GitSection::Worktrees, None).unwrap();
        assert_eq!(worktrees.len(), 2);
        let here = worktrees
            .iter()
            .find(|row| row.prefix == "● ")
            .expect("the checkout this test runs in is marked");
        assert_eq!(here.pieces[0], "main");
        let other = worktrees
            .iter()
            .find(|row| row.pieces[0] == "feature")
            .expect("the added worktree is listed with its branch");
        let text = detail(
            dir.path(),
            GitSection::Worktrees,
            other.key.as_deref().unwrap(),
            Language::En,
        )
        .unwrap()
        .join("\n");
        assert!(text.contains("feature"), "its own status is read: {text}");

        let branches = rows(dir.path(), GitSection::Branches, None).unwrap();
        let names: Vec<&str> = branches.iter().map(|row| row.pieces[0].as_str()).collect();
        assert!(names.contains(&"main"), "{names:?}");
        assert!(names.contains(&"feature"), "{names:?}");
        assert_eq!(
            branches
                .iter()
                .find(|row| row.prefix == "● ")
                .expect("the checked-out branch is marked")
                .pieces[0],
            "main"
        );
        // A branch is a revision the log can be filtered to.
        let log = rows(dir.path(), GitSection::Commits, Some("feature")).unwrap();
        assert!(log
            .iter()
            .flat_map(|row| row.pieces.iter())
            .any(|piece| piece == "feature commit"));
    }

    #[test]
    fn outside_a_repository_the_read_says_so() {
        let dir = tempfile::tempdir().unwrap();
        assert!(repo_name(dir.path(), Language::En).is_err());
        assert!(rows(dir.path(), GitSection::Commits, None).is_err());
    }

    /// A repository with no commit yet is an empty log, not an error: git's
    /// "your current branch does not have any commits" is a state, and the panel
    /// paints "no commits" for it.
    #[test]
    fn an_empty_repository_is_an_empty_log() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success());
        };
        git(&["init", "-q"]);
        assert!(rows(dir.path(), GitSection::Commits, None)
            .unwrap()
            .is_empty());
        assert!(rows(dir.path(), GitSection::Stash, None)
            .unwrap()
            .is_empty());
    }
}
