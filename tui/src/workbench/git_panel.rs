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

use std::path::Path;
use std::time::Duration;

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

/// `git log --graph`, parsed into rows.
///
/// With no filter this is every branch, remote branch and tag as one timeline
/// — the graph is the panel's point, and a graph of one branch is a straight
/// line with no branch in it. A filter is one line of work and is read on its
/// own.
fn commits(cwd: &Path, filter: Option<&str>) -> Result<Vec<GitRow>, String> {
    // `%x1e` / `%x1f` are git's own escapes for RS and US: a subject is free
    // text, so the field separator has to be a byte a subject cannot contain.
    let pretty = "--pretty=format:%x1e%h%x1f%ad%x1f%an%x1f%D%x1f%s";
    let limit = format!("--max-count={COMMIT_LIMIT}");
    let mut args = vec![
        "log",
        "--graph",
        "--decorate=short",
        "--date=short",
        limit.as_str(),
        pretty,
    ];
    match filter {
        Some(reference) => args.push(reference),
        // Every branch, tag and remote-tracking branch — the three groups the
        // refs tab lists, so the log and that tab describe the same repository.
        // `--all` would also pull in `refs/stash` (a stash is a commit tree), and
        // a stash entry sitting in the log as if it were a branch is noise the
        // refs tab does not have. `--date-order` keeps a parent above its child,
        // which is what makes the lanes readable when several lines of work are
        // interleaved. Naming `HEAD` as well would be the obvious way to cover a
        // detached checkout, but on an unborn branch it is a fatal error — and
        // that error takes the other branches' commits down with it.
        None => args.extend_from_slice(&["--branches", "--remotes", "--tags", "--date-order"]),
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

/// Split `git log --graph` into rows: a line carrying [`RECORD`] is a commit
/// (graph prefix, then the five fields); a line without one is the graph's own
/// continuation (`|\`, `|/`) and stays a non-selectable row so the drawing keeps
/// its shape.
pub fn parse_log(text: &str) -> Vec<GitRow> {
    use GitKind::{Decoration, Meta, Plain, Sha};
    text.lines()
        .map(|line| match line.find(RECORD) {
            Some(at) => {
                let prefix = line[..at].to_string();
                let fields: Vec<&str> = line[at + RECORD.len_utf8()..].split(FIELD).collect();
                let sha = fields.first().copied().unwrap_or("").to_string();
                let date = fields.get(1).copied().unwrap_or("");
                let author = fields.get(2).copied().unwrap_or("");
                let decorations = fields.get(3).copied().unwrap_or("");
                let subject = fields.get(4).copied().unwrap_or("");
                // The subject leads — it is what a reader scans — and the rest
                // are whole pieces a narrow box drops one at a time. Each one
                // travels with what it is, so the painter colours a field by
                // meaning and not by where it happened to land.
                let mut pieces = Vec::new();
                let mut kinds = Vec::new();
                for (piece, kind) in [
                    (subject, Plain),
                    (decorations, Decoration),
                    (sha.as_str(), Sha),
                    (date, Meta),
                    (author, Meta),
                ] {
                    if !piece.is_empty() {
                        pieces.push(piece.to_string());
                        kinds.push(kind);
                    }
                }
                GitRow {
                    key: Some(sha),
                    prefix,
                    pieces,
                    kinds,
                }
            }
            None => GitRow {
                key: None,
                prefix: line.trim_end().to_string(),
                pieces: Vec::new(),
                kinds: Vec::new(),
            },
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

    /// The parse is the contract the painter and the driver share: fields stay
    /// whole (a subject with a `|` or a tab is still one subject), the graph
    /// prefix lands in the row's gutter, and a graph-only line stays a row so
    /// the drawing keeps its shape.
    #[test]
    fn log_lines_split_on_the_record_separator() {
        let rows = parse_log(
            "* \u{1e}abc1234\u{1f}2026-10-27\u{1f}Ada\u{1f}HEAD -> main, tag: v1\u{1f}fix | the parser\n\
             |\\\n\
             * \u{1e}def5678\u{1f}2026-10-26\u{1f}Bob\u{1f}\u{1f}a\ttab and a unit separator\n",
        );
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].prefix, "* ");
        assert_eq!(rows[0].key.as_deref(), Some("abc1234"));
        assert_eq!(rows[0].pieces[0], "fix | the parser");
        assert_eq!(rows[0].pieces[1], "HEAD -> main, tag: v1");
        assert_eq!(rows[1].key, None, "a graph-only line is not selectable");
        assert_eq!(rows[1].prefix, "|\\");
        assert_eq!(rows[2].pieces[0], "a\ttab and a unit separator");
        // The empty decoration was dropped rather than painted as ` · `.
        assert!(!rows[2].pieces.iter().any(|piece| piece.is_empty()));
        assert_eq!(rows[2].pieces.len(), 4, "subject, sha, date and author");
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
            .filter_map(|row| row.pieces.first())
            .any(|subject| subject == "merge feature"));

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

    /// The read the panel performs, against a real repository: all four
    /// sections list something, and each row's detail loads.
    #[test]
    fn a_real_repository_fills_every_section() {
        let dir = repo();
        let name = repo_name(dir.path(), Language::En).unwrap();
        assert!(!name.is_empty());

        let log = rows(dir.path(), GitSection::Commits, None).unwrap();
        let commits: Vec<&GitRow> = log.iter().filter(|row| row.key.is_some()).collect();
        assert_eq!(commits.len(), 4, "first, second, feature and the merge");
        assert_eq!(commits[0].pieces[0], "merge feature");
        // The graph is drawn: the merge commit's line starts with `* `.
        assert!(commits[0].prefix.contains('*'), "{:?}", commits[0].prefix);
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
            .filter_map(|row| row.pieces.first())
            .any(|subject| subject == "feature commit"));
        assert!(!feature
            .iter()
            .filter_map(|row| row.pieces.first())
            .any(|subject| subject == "second commit"));
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
            .filter_map(|row| row.pieces.first())
            .any(|subject| subject == "feature commit"));
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
