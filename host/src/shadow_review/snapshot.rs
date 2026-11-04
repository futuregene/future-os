//! before/after snapshot capture (§5.4): reuse the persisted index, stage only
//! the changed candidate set, write a tree, reuse-or-create a commit, pin a ref,
//! and persist snapshot metadata.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::store::{self, CreateReviewSnapshotInput, ReviewSnapshotRecord};
use crate::Error;

use super::policy::{self, Disposition, Limits};
use super::repository::ShadowRepo;

/// A captured snapshot plus the sensitive credential paths that changed this
/// round (omitted from the tree, surfaced as metadata-only rows — §13).
#[derive(Debug)]
pub struct CaptureOutcome {
    pub snapshot: ReviewSnapshotRecord,
    pub sensitive: Vec<String>,
}

/// Capture one phase (`"before"` or `"after"`) of a Run. The caller must already
/// hold the Workspace shadow lock (§12.1).
pub fn capture(
    repo: &ShadowRepo,
    thread_id: &str,
    run_id: &str,
    phase: &str,
    limits: &Limits,
) -> Result<CaptureOutcome, Error> {
    let tag = format!("{run_id}.{phase}");
    let temp_index = repo.prepare_temp_index(&tag)?;

    // info/exclude: real repo excludes (boundary 1) + non-git defaults (§5.5).
    let real_exclude = repo.real_repo_info_exclude();
    let exclude = policy::build_info_exclude(repo.is_git_workspace, real_exclude.as_deref(), &[]);
    repo.write_info_exclude(&exclude)?;

    let candidates = candidate_paths(repo, &temp_index)?;
    let (candidates, mut ignored) = filter_ignored(repo, &temp_index, &candidates)?;

    // Classify candidates, honouring the per-round limits (§5.5).
    let mut staged: Vec<String> = Vec::new();
    let mut sensitive: Vec<String> = Vec::new();
    let mut total_bytes: u64 = 0;
    let mut omitted: usize = 0;
    let mut over_limit = false;

    for path in &candidates {
        if crate::request_scope::is_stopping() {
            return Err("Snapshot capture cancelled because the host is stopping.".into());
        }
        if staged.len() >= limits.max_candidate_files {
            over_limit = true;
            omitted += 1;
            continue;
        }
        let abs = repo.workspace_path.join(path);
        // A missing file is a deletion — always stage it (size 0) so the removal
        // is captured.
        let size = std::fs::metadata(&abs).map(|m| m.len()).unwrap_or(0);
        match policy::classify(path, size, limits) {
            Disposition::Include => {
                if total_bytes.saturating_add(size) > limits.max_total_bytes {
                    over_limit = true;
                    omitted += 1;
                    continue;
                }
                total_bytes += size;
                staged.push(path.clone());
            }
            Disposition::Sensitive => {
                omitted += 1;
                sensitive.push(path.clone());
            }
            Disposition::Oversized => {
                omitted += 1;
            }
        }
    }

    let staged_outcome = stage(repo, &temp_index, &staged)?;
    // A path can be reported twice (once when candidates are filtered, once by
    // the staging retry); the snapshot lists it once.
    for entry in staged_outcome.ignored {
        if !ignored.iter().any(|seen| seen.path == entry.path) {
            ignored.push(entry);
        }
    }
    if staged_outcome.paths.len() != staged.len() {
        let actually_staged: HashSet<&str> =
            staged_outcome.paths.iter().map(String::as_str).collect();
        total_bytes = staged
            .iter()
            .filter(|path| actually_staged.contains(path.as_str()))
            .map(|path| {
                std::fs::metadata(repo.workspace_path.join(path))
                    .map(|metadata| metadata.len())
                    .unwrap_or(0)
            })
            .sum();
    }
    let tree_id = repo.git(&["write-tree"], Some(&temp_index))?;

    // Reuse the before-commit when the after-tree is identical (zero-change
    // Run) — the common reuse case (§12.2).
    let commit_id = reuse_commit(run_id, phase, &tree_id)?
        .map(Ok)
        .unwrap_or_else(|| repo.commit_tree(&tree_id, &format!("run {run_id} {phase}")))?;

    let ref_name = ShadowRepo::snapshot_ref(thread_id, run_id, phase);
    repo.update_ref(&ref_name, &commit_id)?;
    repo.commit_temp_index(&temp_index)?;

    let status = if over_limit || omitted > 0 {
        "partial"
    } else {
        "complete"
    };

    // The ignore-rule exclusions are named, not only counted: a snapshot has to
    // be able to answer "what am I missing, and because of which rule" (§5.4).
    let ignored_paths = if ignored.is_empty() {
        None
    } else {
        Some(
            serde_json::to_string(&ignored)
                .map_err(|error| format!("shadow snapshot metadata failed: {error}"))?,
        )
    };

    let snapshot = store::create_review_snapshot(CreateReviewSnapshotInput {
        workspace_id: repo.workspace_id.clone(),
        thread_id: thread_id.to_string(),
        run_id: run_id.to_string(),
        phase: phase.to_string(),
        commit_id: Some(commit_id),
        tree_id: Some(tree_id),
        status: status.to_string(),
        file_count: staged_outcome.paths.len() as i64,
        total_bytes: total_bytes as i64,
        ignored_count: ignored.len() as i64,
        ignored_paths,
        omitted_count: omitted as i64,
        error_message: None,
    })?;
    Ok(CaptureOutcome {
        snapshot,
        sensitive,
    })
}

/// Record a `failed` snapshot row so the changeset can be marked `unavailable`
/// instead of silently reading as "no changes" (§6.3).
pub fn record_failure(
    repo: &ShadowRepo,
    thread_id: &str,
    run_id: &str,
    phase: &str,
    error: &str,
) -> Result<ReviewSnapshotRecord, Error> {
    store::create_review_snapshot(CreateReviewSnapshotInput {
        workspace_id: repo.workspace_id.clone(),
        thread_id: thread_id.to_string(),
        run_id: run_id.to_string(),
        phase: phase.to_string(),
        status: "failed".to_string(),
        error_message: Some(error.to_string()),
        ..Default::default()
    })
}

/// The changed candidate set: modified/deleted tracked files plus untracked
/// files, deduped (§5.2). Uses the temp index's stat cache so unchanged files
/// are never opened.
fn candidate_paths(repo: &ShadowRepo, index: &Path) -> Result<Vec<String>, Error> {
    let mut set: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let tracked = repo.git_bytes(&["diff-files", "--name-only", "-z"], Some(index))?;
    let untracked_args: &[&str] = &["ls-files", "--others", "--exclude-standard", "-z"];
    let untracked = repo.git_bytes(untracked_args, Some(index))?;

    for bytes in [tracked, untracked] {
        for raw in bytes.split(|b| *b == 0) {
            if raw.is_empty() {
                continue;
            }
            let path = String::from_utf8_lossy(raw).into_owned();
            if seen.insert(path.clone()) {
                set.push(path);
            }
        }
    }
    Ok(set)
}

/// One candidate the Workspace's ignore rules kept out of the snapshot, with
/// the rule that did it (`source:line:pattern`). Persisted as snapshot
/// metadata so a snapshot can answer "what did I leave out, and why" (§5.4).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(super) struct IgnoredPath {
    pub path: String,
    /// Anonymous when git's `add` refusal named the path and no rule was
    /// readable at the time (the rules changed mid-capture).
    pub rule: Option<String>,
}

/// Split the candidate set into what can be snapshotted and what the
/// Workspace's ignore rules exclude.
///
/// Ask git two different questions, because `git add` and `git check-ignore`
/// disagree about exactly one shape, and that shape broke every before-snapshot
/// of a Workspace that excluded a directory it had already snapshotted
/// (measured 2026-11-03: 40 lost before-snapshots a day, `bugfind/`):
///
/// 1. The candidates themselves, with git's normal index awareness — a path
///    already in the snapshot index counts as tracked, so a rule added later
///    does not silently stop its changes being captured (as with real Git).
/// 2. Their ancestor directories, from the ignore rules alone. `git add -A`
///    refuses a path below an ignored *directory* even when that path is in the
///    index, and names the directory (`bugfind`) — while `check-ignore` on the
///    file reports nothing, because the file's own index entry masks the rule.
///    The directory is the only question git answers the way `add` will.
fn filter_ignored(
    repo: &ShadowRepo,
    index: &Path,
    paths: &[String],
) -> Result<(Vec<String>, Vec<IgnoredPath>), Error> {
    if paths.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let own_rules = check_ignore(repo, index, paths, false)?;
    let dir_rules = check_ignore(repo, index, &ancestor_dirs(paths), true)?;

    let mut visible: Vec<String> = Vec::new();
    let mut ignored: Vec<IgnoredPath> = Vec::new();
    for path in paths {
        let rule = own_rules.get(path).cloned().or_else(|| {
            ancestor_dirs(std::slice::from_ref(path))
                .into_iter()
                .find_map(|dir| dir_rules.get(&dir).cloned())
        });
        match rule {
            Some(rule) => ignored.push(IgnoredPath {
                path: path.clone(),
                rule: Some(rule),
            }),
            None => visible.push(path.clone()),
        }
    }
    Ok((visible, ignored))
}

/// `git check-ignore -v -z --stdin` as `path -> source:line:pattern`.
///
/// `rules_only` adds `--no-index`, which asks what the ignore rules say about a
/// path that this index already carries.
fn check_ignore(
    repo: &ShadowRepo,
    index: &Path,
    paths: &[String],
    rules_only: bool,
) -> Result<HashMap<String, String>, Error> {
    let mut args: Vec<&str> = vec!["check-ignore", "-v", "-z", "--stdin"];
    if rules_only {
        args.insert(1, "--no-index");
    }
    let stdin = paths.join("\0").into_bytes();
    let output = repo.run(&args, Some(index), Some(&stdin))?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(format!(
            "shadow git check-ignore failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    // `-v -z` prints source, line, pattern, pathname — four NUL-terminated
    // fields per match, and nothing at all for a path no rule matches.
    let mut fields: Vec<String> = output
        .stdout
        .split(|byte| *byte == 0)
        .map(|raw| String::from_utf8_lossy(raw).into_owned())
        .collect();
    // The trailing NUL of the last match leaves one empty field (or a single
    // empty field when nothing matched — a case already handled above by git
    // exiting 1 with empty output).
    if fields.last().is_some_and(String::is_empty) {
        fields.pop();
    }
    if !fields.len().is_multiple_of(4) {
        // Reading a garbled answer as "nothing is ignored" would snapshot files
        // the Workspace excludes, so it is reported instead.
        return Err(format!(
            "shadow git check-ignore returned {} unparseable fields",
            fields.len()
        )
        .into());
    }
    let (matches, _rest) = fields.as_chunks::<4>();
    Ok(matches
        .iter()
        .map(|match_fields| {
            let rule = format!(
                "{}:{}:{}",
                short_rule_source(repo, &match_fields[0]),
                match_fields[1],
                match_fields[2]
            );
            (match_fields[3].clone(), rule)
        })
        .collect())
}

/// The rule's location as a person can act on it: `info/exclude` for the rules
/// this capture itself derived from the real repo, a Workspace-relative
/// `.gitignore` otherwise.
fn short_rule_source(repo: &ShadowRepo, source: &str) -> String {
    let path = Path::new(source);
    if repo.owns_path(path) {
        return "info/exclude".to_string();
    }
    path.strip_prefix(&repo.workspace_path)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Every proper ancestor directory of the given paths, deduped, so one
/// `check-ignore` asks about all of them.
fn ancestor_dirs(paths: &[String]) -> Vec<String> {
    let mut dirs: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for path in paths {
        let mut current = path.as_str();
        while let Some((parent, _)) = current.rsplit_once('/') {
            // A directory already walked brought its own parents with it.
            if !seen.insert(parent.to_string()) {
                break;
            }
            dirs.push(parent.to_string());
            current = parent;
        }
    }
    dirs
}

#[derive(Debug)]
struct StageOutcome {
    paths: Vec<String>,
    ignored: Vec<IgnoredPath>,
}

/// Why one `git add` of a candidate set failed.
#[derive(Debug)]
enum StageFailure {
    /// git refused the named candidates as ignored. That is an exclusion the
    /// caller can absorb by staging the rest, not a snapshot failure.
    Ignored { paths: Vec<String>, message: String },
    /// Anything else: the tree cannot be captured.
    Other(Error),
}

impl StageFailure {
    fn into_error(self) -> Error {
        match self {
            StageFailure::Ignored { message, .. } => message.into(),
            StageFailure::Other(error) => error,
        }
    }
}

/// Stage only the given candidate paths (`--all` so deletions are recorded).
/// If an untracked path becomes ignored after candidate discovery, drop it and
/// retry once so one expected exclusion cannot fail the entire snapshot.
fn stage(repo: &ShadowRepo, index: &Path, paths: &[String]) -> Result<StageOutcome, Error> {
    let (paths, mut ignored) = filter_ignored(repo, index, paths)?;
    if paths.is_empty() {
        return Ok(StageOutcome { paths, ignored });
    }
    match try_stage(repo, index, &paths) {
        Ok(()) => Ok(StageOutcome { paths, ignored }),
        Err(failure) => {
            // Two shapes reach here and both are exclusions: the ignore rules
            // changed between discovery and staging (the re-check below finds
            // them), or git refuses a path our own check-ignore pass did not
            // report — measured 2026-11-03, a Workspace whose `.git/info/exclude`
            // hid `bugfind/` failed every before-snapshot this way. git's own
            // list of refused paths is authoritative, so it is what gets dropped.
            let (visible, rechecked) = filter_ignored(repo, index, &paths)?;
            let mut dropped: Vec<IgnoredPath> = rechecked
                .into_iter()
                .filter(|entry| !visible.contains(&entry.path))
                .collect();
            if let StageFailure::Ignored { paths: named, .. } = &failure {
                for path in named {
                    if !dropped.iter().any(|entry| &entry.path == path) {
                        dropped.push(IgnoredPath {
                            path: path.clone(),
                            rule: None,
                        });
                    }
                }
            }
            if dropped.is_empty() {
                return Err(failure.into_error());
            }
            let retry_paths: Vec<String> = paths
                .iter()
                .filter(|path| !dropped.iter().any(|entry| &entry.path == *path))
                .cloned()
                .collect();
            ignored.extend(dropped);
            if retry_paths.is_empty() {
                return Ok(StageOutcome {
                    paths: retry_paths,
                    ignored,
                });
            }
            // A second refusal is no longer a rule change: report it rather than
            // looping over a set that keeps shrinking without converging.
            try_stage(repo, index, &retry_paths).map_err(StageFailure::into_error)?;
            Ok(StageOutcome {
                paths: retry_paths,
                ignored,
            })
        }
    }
}

fn try_stage(repo: &ShadowRepo, index: &Path, paths: &[String]) -> Result<(), StageFailure> {
    let stdin = paths.join("\0").into_bytes();
    let add_args: &[&str] = &[
        "add",
        "--all",
        "--pathspec-from-file=-",
        "--pathspec-file-nul",
    ];
    // A spawn failure is not about the paths: it is the same error in both arms.
    let output = repo
        .run(add_args, Some(index), Some(&stdin))
        .map_err(StageFailure::Other)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // A pathspec that matches nothing (deleted file no longer tracked)
        // shouldn't fail the whole snapshot — skip it with a warning.
        if stderr.contains("pathspec") && stderr.contains("did not match") {
            tracing::warn!(
                target: "future_host::shadow_review",
                paths = paths.len(),
                "skipped paths that no longer exist"
            );
            return Ok(());
        }
        let message = format!("shadow git add failed: {}", stderr.trim());
        let named = ignored_paths_in(&stderr, &repo.workspace_path, paths);
        if !named.is_empty() {
            return Err(StageFailure::Ignored {
                paths: named,
                message,
            });
        }
        return Err(StageFailure::Other(message.into()));
    }
    Ok(())
}

/// The candidate paths git itself named as ignored in a `git add` refusal.
///
/// git prints the deepest ignored *directory* it refused (`bugfind`, measured
/// 2026-11-03) while the candidate we sent is a file below it (`bugfind/h1`),
/// so a candidate counts as named when git's line is the path itself or one of
/// its ancestors. git quotes a path that needs it and never prints one per hint
/// line, so the message is matched against the candidates we sent instead of
/// being re-parsed on its own. An empty result means the refusal was about
/// something else and must stay a failure.
fn ignored_paths_in(stderr: &str, workspace: &Path, paths: &[String]) -> Vec<String> {
    const REFUSALS: [&str; 2] = [
        "are ignored by one of your .gitignore files",
        "is ignored by one of your .gitignore files",
    ];
    if !REFUSALS.iter().any(|marker| stderr.contains(marker)) {
        return Vec::new();
    }
    let named: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("hint:"))
        .filter(|line| !REFUSALS.iter().any(|marker| line.contains(marker)))
        .map(|line| line.trim_end_matches('/'))
        .collect();
    paths
        .iter()
        .filter(|path| {
            // Candidates are workspace-relative; a caller that passed an
            // absolute path gets the same match through the stripped form.
            let relative = Path::new(path.as_str())
                .strip_prefix(workspace)
                .ok()
                .map(|rest| rest.to_string_lossy().into_owned());
            let matches = [Some(path.as_str()), relative.as_deref()]
                .into_iter()
                .flatten()
                .any(|candidate| {
                    let candidate = candidate.trim_start_matches("./");
                    named.iter().any(|line| {
                        *line == candidate
                            || candidate
                                .strip_prefix(line)
                                .is_some_and(|rest| rest.starts_with('/'))
                    })
                });
            matches
        })
        .cloned()
        .collect()
}

/// When capturing `after`, reuse the `before` commit if the trees match.
fn reuse_commit(run_id: &str, phase: &str, tree_id: &str) -> Result<Option<String>, Error> {
    if phase != "after" {
        return Ok(None);
    }
    let Some(before) = store::get_review_snapshot(run_id, "before")? else {
        return Ok(None);
    };
    if before.tree_id.as_deref() == Some(tree_id) {
        Ok(before.commit_id)
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::auth_store::test_support::HomeGuard;
    use crate::store::{CreateRunInput, CreateThreadInput, CreateWorkspaceInput};

    struct Setup {
        _home: HomeGuard,
        workspace_id: String,
        thread_id: String,
        run_id: String,
        repo: ShadowRepo,
    }

    fn setup(name: &str) -> Setup {
        let home = HomeGuard::new(name);
        store::initialize_app_store().unwrap();
        let dir = std::env::temp_dir().join(format!("futureos-snap-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let ws = store::create_workspace(CreateWorkspaceInput {
            name: Some(name.into()),
            path: dir.display().to_string(),
            description: None,
            create_directory: Some(false),
        })
        .unwrap();
        let thread = store::create_thread(CreateThreadInput {
            mode: "workspace".into(),
            title: Some(name.into()),
            workspace_id: Some(ws.id.clone()),
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .unwrap();
        let run = store::create_run(CreateRunInput {
            id: None,
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();

        let repo = ShadowRepo::open(&ws.id, Path::new(&dir), false).unwrap();
        Setup {
            _home: home,
            workspace_id: ws.id,
            thread_id: thread.id,
            run_id: run.id,
            repo,
        }
    }

    #[test]
    fn record_failure_writes_failed_row() {
        let s = setup("record-failure");
        let snap = record_failure(&s.repo, &s.thread_id, &s.run_id, "before", "boom").unwrap();
        assert_eq!(snap.status, "failed");
        assert_eq!(snap.error_message.as_deref(), Some("boom"));
    }

    #[test]
    fn capture_handles_oversized_too_many_and_too_many_bytes() {
        let s = setup("capture-limits");
        std::fs::write(s.repo.workspace_path.join("a.txt"), b"hello").unwrap();

        // Oversized: max_file_bytes = 0 makes the non-empty file oversized.
        let outcome = capture(
            &s.repo,
            &s.thread_id,
            &s.run_id,
            "before",
            &Limits {
                max_file_bytes: 0,
                max_candidate_files: 1000,
                max_total_bytes: 1_000_000,
                ..Limits::default()
            },
        )
        .unwrap();
        assert_eq!(outcome.snapshot.status, "partial");
        assert_eq!(outcome.snapshot.omitted_count, 1);

        // Too many candidates: max_candidate_files = 0.
        let outcome = capture(
            &s.repo,
            &s.thread_id,
            &s.run_id,
            "before",
            &Limits {
                max_candidate_files: 0,
                ..Limits::default()
            },
        )
        .unwrap();
        assert_eq!(outcome.snapshot.status, "partial");

        // Too many bytes: max_total_bytes = 0 with a file → over limit.
        let outcome = capture(
            &s.repo,
            &s.thread_id,
            &s.run_id,
            "before",
            &Limits {
                max_total_bytes: 0,
                ..Limits::default()
            },
        )
        .unwrap();
        assert_eq!(outcome.snapshot.status, "partial");
    }

    #[test]
    fn stage_handles_empty_and_missing_pathspec() {
        let s = setup("stage");
        let index = s.repo.prepare_temp_index("stage-test").unwrap();
        // Empty paths → early return.
        stage(&s.repo, &index, &[]).unwrap();
        // Non-existent path → pathspec "did not match" → Ok.
        stage(
            &s.repo,
            &index,
            &["definitely-not-a-real-file.txt".to_string()],
        )
        .unwrap();
    }

    #[test]
    fn stage_skips_ignored_untracked_paths_without_losing_other_files() {
        let s = setup("stage-ignored");
        std::fs::write(s.repo.workspace_path.join(".gitignore"), "*.log\n").unwrap();
        std::fs::write(s.repo.workspace_path.join("keep.txt"), "keep\n").unwrap();
        std::fs::write(s.repo.workspace_path.join("ignored.log"), "ignored\n").unwrap();
        let index = s.repo.prepare_temp_index("stage-ignored").unwrap();

        let outcome = stage(
            &s.repo,
            &index,
            &["keep.txt".to_string(), "ignored.log".to_string()],
        )
        .unwrap();

        assert_eq!(outcome.paths, vec!["keep.txt"]);
        assert_eq!(outcome.ignored.len(), 1);
        assert_eq!(outcome.ignored[0].path, "ignored.log");
        assert_eq!(
            outcome.ignored[0].rule.as_deref(),
            Some(".gitignore:1:*.log"),
            "the snapshot names the rule that excluded the path"
        );
        assert_eq!(s.repo.git(&["ls-files"], Some(&index)).unwrap(), "keep.txt");
    }

    #[test]
    fn stage_keeps_tracked_paths_that_later_match_ignore_rules() {
        let s = setup("stage-tracked-ignored");
        let path = s.repo.workspace_path.join("tracked.log");
        std::fs::write(&path, "before\n").unwrap();
        let index = s.repo.prepare_temp_index("stage-tracked-ignored").unwrap();
        stage(&s.repo, &index, &["tracked.log".to_string()]).unwrap();

        std::fs::write(s.repo.workspace_path.join(".gitignore"), "*.log\n").unwrap();
        std::fs::write(path, "after\n").unwrap();
        let outcome = stage(&s.repo, &index, &["tracked.log".to_string()]).unwrap();

        assert_eq!(outcome.paths, vec!["tracked.log"]);
        assert!(outcome.ignored.is_empty());
    }

    /// A candidate `git add` refuses is re-checked against the ignore rules
    /// before the whole snapshot is given up on. Here the file is *not*
    /// ignored — a mode-000 file is unreadable, not excluded — so the original
    /// error must surface: omitting a changed file silently would be worse than
    /// failing the capture.
    #[cfg(unix)]
    #[test]
    fn stage_reports_the_git_error_when_a_refused_path_is_not_ignored() {
        use std::os::unix::fs::PermissionsExt;
        let s = setup("stage-refused");
        let path = s.repo.workspace_path.join("locked.txt");
        std::fs::write(&path, "unreadable\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let index = s.repo.prepare_temp_index("stage-refused").unwrap();

        let error = stage(&s.repo, &index, &["locked.txt".to_string()])
            .expect_err("a path git cannot index must be an error");

        assert!(
            error.to_string().contains("shadow git add failed"),
            "{error}"
        );
        assert!(
            s.repo
                .git(&["ls-files"], Some(&index))
                .unwrap_or_default()
                .is_empty(),
            "nothing was staged for the refused path"
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }

    #[test]
    fn stage_surfaces_unexpected_git_error() {
        let s = setup("stage-err");
        // Make the index path a directory so git cannot write the index — a
        // non-pathspec failure that must be surfaced as an error.
        let index = s.repo.prepare_temp_index("stage-err").unwrap();
        std::fs::create_dir_all(&index).unwrap();
        let failure = try_stage(&s.repo, &index, &["some-file.txt".to_string()]).unwrap_err();
        let StageFailure::Other(error) = failure else {
            panic!("an unusable index is not an ignored-path refusal: {failure:?}");
        };
        assert!(error.to_string().contains("shadow git add failed"));
    }

    /// git names the ignored directory it refused (`bugfind`), while the
    /// candidate we offered is a file below it (`bugfind/h1`). Reading the
    /// refusal as "named candidates" is what lets `stage` drop the file and
    /// capture the rest; matching exact strings would leave the snapshot
    /// failing on every run that touches that Workspace (measured 2026-11-03:
    /// 40 before-snapshots a day).
    #[test]
    fn ignored_paths_in_reads_a_directory_refusal_as_the_files_below_it() {
        let stderr = "The following paths are ignored by one of your .gitignore files:\n\
                      bugfind\n\
                      hint: Use -f if you really want to add them.\n\
                      hint: Disable this message with \"git config set advice.addIgnoredFile false\"";
        let workspace = Path::new("/work/ws");
        assert_eq!(
            ignored_paths_in(
                stderr,
                workspace,
                &["bugfind/h1".to_string(), "keep.txt".to_string()]
            ),
            vec!["bugfind/h1".to_string()]
        );
        // The same refusal reaches a caller that passed absolute paths.
        assert_eq!(
            ignored_paths_in(stderr, workspace, &["/work/ws/bugfind/h2".to_string()]),
            vec!["/work/ws/bugfind/h2".to_string()]
        );
    }

    #[test]
    fn ignored_paths_in_stays_empty_for_any_other_failure() {
        let workspace = Path::new("/work/ws");
        assert!(
            ignored_paths_in("fatal: not a git repository", workspace, &["a".to_string()])
                .is_empty()
        );
        // Naming a path we did not offer is not an exclusion we can absorb.
        let stderr = "The following paths are ignored by one of your .gitignore files:\n\
                      other\n\
                      hint: Use -f if you really want to add them.";
        assert!(ignored_paths_in(stderr, workspace, &["bugfind/h1".to_string()]).is_empty());
    }

    #[test]
    fn reuse_commit_only_reuses_matching_after_tree() {
        let s = setup("reuse");
        // phase "before" never reuses.
        assert_eq!(reuse_commit(&s.run_id, "before", "tree1").unwrap(), None);
        // "after" with no before snapshot → None.
        assert_eq!(reuse_commit(&s.run_id, "after", "tree1").unwrap(), None);
        // Insert a before snapshot with a known tree id.
        store::create_review_snapshot(CreateReviewSnapshotInput {
            workspace_id: s.workspace_id.clone(),
            thread_id: s.thread_id.clone(),
            run_id: s.run_id.clone(),
            phase: "before".into(),
            commit_id: Some("commit-abc".into()),
            tree_id: Some("tree1".into()),
            status: "complete".into(),
            ..Default::default()
        })
        .unwrap();
        // Matching tree → reuse the before commit.
        assert_eq!(
            reuse_commit(&s.run_id, "after", "tree1")
                .unwrap()
                .as_deref(),
            Some("commit-abc")
        );
        // Mismatching tree → None.
        assert_eq!(reuse_commit(&s.run_id, "after", "tree2").unwrap(), None);
    }

    #[test]
    fn filter_ignored_surfaces_a_git_failure() {
        let s = setup("filter-ignored-err");
        let index = s.repo.prepare_temp_index("filter-ignored-err").unwrap();
        // A directory where the index file belongs makes git fail with a code
        // other than 0 (success) or 1 (nothing ignored) — a real failure that
        // must be surfaced instead of being read as "nothing is ignored".
        std::fs::create_dir_all(&index).unwrap();
        let error = filter_ignored(&s.repo, &index, &["some-file.txt".to_string()])
            .expect_err("a failing check-ignore must not look like an empty ignore set")
            .to_string();
        assert!(error.contains("shadow git check-ignore failed"), "{error}");
    }

    /// The *other* failure of that call: `git` cannot be spawned at all. The test
    /// above reaches the non-zero-status arm; this one reaches the `?` on
    /// `repo.run(..)`, which is the only path that reports a spawn failure rather
    /// than an exit code. Reading it as "nothing is ignored" would snapshot files
    /// the Workspace excludes, so the error must surface.
    ///
    /// The fixture is the campaign's sanctioned one — `cli_tool::EnvGuard` restores
    /// `PATH` on drop (including on unwind). No explicit `TEST_HOME_LOCK` here:
    /// `HomeGuard` (inside `setup`) **holds that lock for the whole test**, so
    /// taking it first self-deadlocks on a non-reentrant mutex — measured, the
    /// first version of this test hung for the full 500 s.
    #[test]
    fn filter_ignored_reports_a_git_that_cannot_be_spawned() {
        if crate::test_support::reexec_test() {
            return;
        }
        let s = setup("filter-ignored-spawn");
        std::fs::write(s.repo.workspace_path.join("keep.txt"), "keep\n").unwrap();
        let index = s.repo.prepare_temp_index("filter-ignored-spawn").unwrap();
        // A `PATH` with no `git` in it: the spawn itself fails.
        let empty = tempfile::tempdir().unwrap();
        let _path = crate::test_support::EnvGuard::set("PATH", empty.path().as_os_str());

        let error = filter_ignored(&s.repo, &index, &["keep.txt".to_string()])
            .expect_err("a git that cannot be spawned must be reported")
            .to_string();
        assert!(error.contains("failed to spawn"), "{error}");
    }

    /// The one shape that makes a candidate ignored *between* candidate
    /// discovery and `stage`'s retry: the failing `git add` itself rewrites the
    /// ignore rules. A required clean filter writes `.gitignore` and then exits
    /// non-zero, so the failure and the rule change land inside the same
    /// deterministic `git add` — the production case is another process editing
    /// the Workspace's `.gitignore` mid-capture, which no test can schedule.
    ///
    /// The script and its arming marker live outside the Workspace so they are
    /// not candidates themselves; only `.gitattributes` (which routes `z.bin`
    /// through the filter) is inside it.
    ///
    /// Unix-only, like the three callers: the fixture is a `#!/bin/sh` script
    /// made executable through `PermissionsExt::from_mode`, and neither the
    /// interpreter nor the mode bits exist on Windows — there the same premise
    /// would need a `.cmd` filter, which git runs under a different shell. The
    /// gate has to sit on the helper as well as on the callers: without it this
    /// module's test code fails to compile on Windows, taking every crate that
    /// depends on `future-host`'s lib tests with it.
    #[cfg(unix)]
    fn arm_a_staging_that_rewrites_the_ignore_rules(
        repo: &ShadowRepo,
        rules: &str,
    ) -> tempfile::TempDir {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let script = root.path().join("refuse-clean.sh");
        let marker = root.path().join("armed");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nif [ -e '{marker}' ]; then\n  rm -f '{marker}'\n  printf '{rules}' > .gitignore\n  exit 1\nfi\ncat\n",
                marker = marker.display(),
                rules = rules,
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(&marker, b"").unwrap();
        repo.git(
            &[
                "config",
                "filter.refuse.clean",
                &script.display().to_string(),
            ],
            None,
        )
        .unwrap();
        repo.git(&["config", "filter.refuse.required", "true"], None)
            .unwrap();
        std::fs::write(
            repo.workspace_path.join(".gitattributes"),
            "z.bin filter=refuse\n",
        )
        .unwrap();
        root
    }

    /// A refused `git add` is retried once with whatever the ignore rules now
    /// exclude. Everything can be excluded: the retry stages nothing, and that
    /// is reported as "all ignored" rather than as a snapshot failure.
    #[cfg(unix)]
    #[test]
    fn stage_drops_every_path_when_the_retry_finds_them_all_ignored() {
        let s = setup("stage-retry-all");
        std::fs::write(s.repo.workspace_path.join("zz.late.log"), "0123456789").unwrap();
        std::fs::write(s.repo.workspace_path.join("z.bin"), "b").unwrap();
        let _fixture = arm_a_staging_that_rewrites_the_ignore_rules(&s.repo, "*.log\nz.bin\n");
        let index = s.repo.prepare_temp_index("stage-retry-all").unwrap();

        let outcome = stage(
            &s.repo,
            &index,
            &["z.bin".to_string(), "zz.late.log".to_string()],
        )
        .expect("a candidate that became ignored must not fail the whole staging");

        assert!(outcome.paths.is_empty(), "{:?}", outcome.paths);
        assert_eq!(
            outcome.ignored.len(),
            2,
            "both paths are reported as excluded: {:?}",
            outcome.ignored
        );
        assert_eq!(
            s.repo.git(&["ls-files"], Some(&index)).unwrap_or_default(),
            "",
            "the retry had nothing left to stage"
        );
    }

    /// The retry keeps the paths that are still candidates: one exclusion must
    /// not cost the snapshot the files beside it.
    #[cfg(unix)]
    #[test]
    fn stage_retries_once_and_keeps_the_paths_that_are_still_candidates() {
        let s = setup("stage-retry-keep");
        std::fs::write(s.repo.workspace_path.join("zz.late.log"), "0123456789").unwrap();
        std::fs::write(s.repo.workspace_path.join("z.bin"), "b").unwrap();
        let _fixture = arm_a_staging_that_rewrites_the_ignore_rules(&s.repo, "*.log\n");
        let index = s.repo.prepare_temp_index("stage-retry-keep").unwrap();

        let outcome = stage(
            &s.repo,
            &index,
            &["z.bin".to_string(), "zz.late.log".to_string()],
        )
        .unwrap();

        assert_eq!(outcome.paths, vec!["z.bin"]);
        assert_eq!(outcome.ignored.len(), 1);
        assert_eq!(s.repo.git(&["ls-files"], Some(&index)).unwrap(), "z.bin");
    }

    /// When staging drops a candidate, the persisted byte total is the size of
    /// what was actually staged — not of what was offered. A total that keeps
    /// counting an excluded file would report a snapshot bigger than its tree.
    #[cfg(unix)]
    #[test]
    fn capture_recounts_the_bytes_it_actually_staged() {
        let s = setup("capture-recount");
        std::fs::write(s.repo.workspace_path.join("zz.late.log"), "0123456789").unwrap();
        std::fs::write(s.repo.workspace_path.join("z.bin"), "b").unwrap();
        let _fixture = arm_a_staging_that_rewrites_the_ignore_rules(&s.repo, "*.log\n");

        let outcome = capture(
            &s.repo,
            &s.thread_id,
            &s.run_id,
            "before",
            &Limits::default(),
        )
        .unwrap();

        let tree = outcome
            .snapshot
            .tree_id
            .clone()
            .expect("a tree was written");
        let listing = s.repo.git(&["ls-tree", "-l", &tree], None).unwrap();
        let mut staged_bytes = 0u64;
        let mut names = Vec::new();
        for line in listing.lines() {
            let (meta, name) = line.split_once('\t').expect("ls-tree -l names each entry");
            let size: u64 = meta
                .split_whitespace()
                .nth(3)
                .and_then(|size| size.parse().ok())
                .unwrap_or_else(|| panic!("no size in {line:?}"));
            staged_bytes += size;
            names.push(name.to_string());
        }
        assert!(names.contains(&"z.bin".to_string()), "{names:?}");
        assert!(
            !names.contains(&"zz.late.log".to_string()),
            "a path the ignore rules excluded must not reach the tree: {names:?}"
        );
        assert_eq!(
            outcome.snapshot.file_count,
            names.len() as i64,
            "the snapshot counts the files it wrote ({names:?})"
        );
        assert_eq!(
            outcome.snapshot.total_bytes, staged_bytes as i64,
            "the total is the staged tree's size, not the candidate list's ({names:?})"
        );
        assert_eq!(
            outcome.snapshot.status, "complete",
            "an expected exclusion does not make the snapshot partial"
        );
    }

    /// A thread-scoped subscriber that records the events it receives, so a
    /// warning can be asserted instead of taken on faith.
    ///
    /// It carries a measurement note as well: `tracing`'s field expressions
    /// (`paths = paths.len()` in `try_stage`) are evaluated only when a
    /// registered subscriber is interested in the callsite, so a lib test with
    /// no subscriber never runs that line at all. The test below drives a span
    /// through every hook so the fixture itself has no unexercised method.
    /// `(message, paths)` pairs, in the order the subscriber saw them.
    type Captured = Vec<(String, Option<u64>)>;

    #[derive(Clone, Default)]
    struct CapturedEvents(std::sync::Arc<std::sync::Mutex<Captured>>);

    impl CapturedEvents {
        fn push(&self, message: String, paths: Option<u64>) {
            self.0.lock().unwrap().push((message, paths));
        }

        fn take(&self) -> Vec<(String, Option<u64>)> {
            std::mem::take(&mut *self.0.lock().unwrap())
        }
    }

    struct Recorded<'a>(&'a mut String, &'a mut Option<u64>);

    impl tracing::field::Visit for Recorded<'_> {
        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            if field.name() == "paths" {
                *self.1 = Some(value);
            }
        }

        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                *self.0 = format!("{value:?}");
            }
        }
    }

    impl tracing::Subscriber for CapturedEvents {
        fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _attributes: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            tracing::span::Id::from_u64(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1)
        }

        fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}

        fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}

        fn event(&self, event: &tracing::Event<'_>) {
            let mut message = String::new();
            let mut paths = None;
            event.record(&mut Recorded(&mut message, &mut paths));
            self.push(message, paths);
        }

        fn enter(&self, _span: &tracing::span::Id) {}

        fn exit(&self, _span: &tracing::span::Id) {}
    }

    /// A pathspec that matches nothing is skipped with a warning that names how
    /// many candidates were dropped. The warning is the only trace a skipped
    /// path leaves, so it is asserted rather than assumed: an operator reading
    /// the log must be able to tell a no-op capture from a partial one.
    #[test]
    fn stage_warns_with_the_candidate_count_when_a_pathspec_matches_nothing() {
        let s = setup("stage-warn");
        let index = s.repo.prepare_temp_index("stage-warn").unwrap();
        let events = CapturedEvents::default();
        let guard = tracing::subscriber::set_default(events.clone());
        // A span around the staging: this is what a subscriber is actually
        // asked to record, and it exercises the four span hooks this fixture
        // implements beside the event one.
        let staging = tracing::info_span!("staging", refused = tracing::field::Empty);
        staging.record("refused", 3);
        let attempt = tracing::info_span!("attempt");
        attempt.follows_from(&staging);
        let outcome = staging.in_scope(|| {
            stage(
                &s.repo,
                &index,
                &[
                    "gone-a.txt".to_string(),
                    "gone-b.txt".to_string(),
                    "gone-c.txt".to_string(),
                ],
            )
        });
        drop(guard);

        let outcome = outcome.expect("a pathspec that matches nothing is not an error");
        assert_eq!(
            outcome.paths.len(),
            3,
            "the candidates are kept, not dropped"
        );

        let recorded = events.take();
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        assert_eq!(
            recorded[0].0, "skipped paths that no longer exist",
            "the warning keeps its message"
        );
        assert_eq!(
            recorded[0].1,
            Some(3),
            "the warning names every path the add refused"
        );
    }
    // ── ignored directories the snapshot index already carries ──────────────

    /// A Workspace that is a real git repository, so the real repo's
    /// `info/exclude` becomes part of the snapshot's ignore rules (§5.5).
    fn setup_git(name: &str) -> Setup {
        let mut s = setup(name);
        let workspace = s.repo.workspace_path.clone();
        run_git(&workspace, &["init", "-q", "."]);
        run_git(
            &workspace,
            &["config", "user.email", "test@example.invalid"],
        );
        run_git(&workspace, &["config", "user.name", "Test"]);
        std::fs::write(workspace.join("tracked.txt"), "original\n").unwrap();
        run_git(&workspace, &["add", "tracked.txt"]);
        run_git(&workspace, &["commit", "-qm", "init"]);
        s.repo = ShadowRepo::open(&s.workspace_id, &workspace, true).unwrap();
        s
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// The rule a person can act on: the workspace's own `info/exclude`, which
    /// is what hid `bugfind/` in the Workspace that lost 40 snapshots a day.
    fn write_real_info_exclude(workspace: &Path, body: &str) {
        std::fs::write(workspace.join(".git").join("info").join("exclude"), body).unwrap();
    }

    fn new_run(s: &Setup) -> String {
        store::create_run(CreateRunInput {
            id: None,
            thread_id: s.thread_id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap()
        .id
    }

    /// Snapshot a Workspace after it started excluding a directory the shadow
    /// index already carries a file from.
    ///
    /// Measured 2026-11-03 in `future-os`: `bugfind/` was excluded *after*
    /// earlier snapshots had staged files below it, so the shadow index still
    /// tracked 94 of them. Every later run's before-snapshot died on `shadow git
    /// add failed: The following paths are ignored by one of your .gitignore
    /// files: bugfind` — 40 lost snapshots a day. Both halves of git disagree
    /// about that path: `check-ignore` says nothing because the file is in the
    /// index, while `add -A` refuses it because its *directory* is ignored.
    #[test]
    fn capture_keeps_the_snapshot_when_an_ignored_directory_is_already_indexed() {
        let s = setup_git("capture-ignored-dir");

        // An earlier snapshot ran while `bugfind/` was still visible.
        std::fs::create_dir_all(s.repo.workspace_path.join("bugfind/h1")).unwrap();
        std::fs::write(s.repo.workspace_path.join("bugfind/h1/note.md"), "old\n").unwrap();
        capture(
            &s.repo,
            &s.thread_id,
            &s.run_id,
            "before",
            &Limits::default(),
        )
        .unwrap();
        let index = s.repo.prepare_temp_index("snapshot-index-probe").unwrap();
        let mut indexed: Vec<String> = s
            .repo
            .git(&["ls-files"], Some(&index))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect();
        indexed.sort();
        assert_eq!(
            indexed,
            vec!["bugfind/h1/note.md".to_string(), "tracked.txt".to_string()],
            "the shadow index carries a file below the directory about to be excluded"
        );

        // The Workspace starts excluding the directory, as this repo did, and a
        // run then edits a file below it *and* a tracked file.
        write_real_info_exclude(&s.repo.workspace_path, "/bugfind/\n");
        std::fs::write(s.repo.workspace_path.join("bugfind/h1/note.md"), "new\n").unwrap();
        std::fs::write(s.repo.workspace_path.join("tracked.txt"), "changed\n").unwrap();

        let run = new_run(&s);
        let outcome = capture(&s.repo, &s.thread_id, &run, "before", &Limits::default())
            .expect("one excluded directory must not cost the whole snapshot");

        // Not merely "no error": the tracked change is in the snapshot.
        let commit = outcome.snapshot.commit_id.clone().expect("a commit");
        assert_eq!(
            s.repo
                .git(&["show", &format!("{commit}:tracked.txt")], None)
                .unwrap(),
            "changed",
            "the tracked change reached the snapshot tree"
        );
        // The excluded path keeps the content it had when it was last
        // snapshotted and is never refreshed — which the metadata says.
        assert_eq!(
            s.repo
                .git(&["show", &format!("{commit}:bugfind/h1/note.md")], None)
                .unwrap(),
            "old",
            "a path the ignore rules exclude is not re-staged"
        );
        assert_eq!(outcome.snapshot.status, "complete");

        // What is missing, and because of which rule — read back from the row,
        // because `ignored_count` alone cannot say which path was left out or
        // which rule did it.
        let stored = store::get_review_snapshot(&run, "before")
            .unwrap()
            .expect("the before snapshot row");
        assert_eq!(stored.ignored_count, 1);
        let skipped: serde_json::Value = serde_json::from_str(
            stored
                .ignored_paths
                .as_deref()
                .expect("the snapshot names what it left out"),
        )
        .unwrap();
        let listed = skipped.as_array().expect("a list of excluded paths");
        assert_eq!(listed.len(), 1, "{skipped}");
        assert_eq!(listed[0]["path"], "bugfind/h1/note.md", "{skipped}");
        let rule = listed[0]["rule"].as_str().unwrap_or_default();
        assert!(
            rule.starts_with("info/exclude:") && rule.ends_with(":/bugfind/"),
            "the metadata names the rule, not just the count: {skipped}"
        );
        // A path the ignore rules left alone is not reported as excluded, so
        // the list cannot be "every candidate" wearing a different name.
        assert!(
            listed.iter().all(|entry| entry["path"] != "tracked.txt"),
            "a snapshotted path must not be listed as excluded: {skipped}"
        );
    }

    /// A path git genuinely cannot add still fails the capture, with git's own
    /// message — dropping excluded paths must not turn a real failure into a
    /// short snapshot. The unindexable file sits *beside* the excluded one, so
    /// the failure survives the ignore pass that removed its neighbour.
    #[cfg(unix)]
    #[test]
    fn capture_still_fails_with_the_original_error_when_a_candidate_cannot_be_added() {
        use std::os::unix::fs::PermissionsExt;
        let s = setup_git("capture-bad-beside-ignored");
        std::fs::create_dir_all(s.repo.workspace_path.join("bugfind/h1")).unwrap();
        std::fs::write(s.repo.workspace_path.join("bugfind/h1/note.md"), "old\n").unwrap();
        capture(
            &s.repo,
            &s.thread_id,
            &s.run_id,
            "before",
            &Limits::default(),
        )
        .unwrap();

        write_real_info_exclude(&s.repo.workspace_path, "/bugfind/\n");
        std::fs::write(s.repo.workspace_path.join("bugfind/h1/note.md"), "new\n").unwrap();
        let locked = s.repo.workspace_path.join("locked.txt");
        std::fs::write(&locked, "unreadable\n").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();

        let run = new_run(&s);
        let error = capture(&s.repo, &s.thread_id, &run, "before", &Limits::default())
            .expect_err("a file git cannot index must not be absorbed as an exclusion")
            .to_string();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert!(error.contains("shadow git add failed"), "{error}");
        assert!(
            error.contains("locked.txt"),
            "the original error is kept: {error}"
        );
    }

    /// The two questions `filter_ignored` asks, on a path that is in the index
    /// *and* ignored by a directory rule: only the second one sees it. Reading
    /// the first alone is what let `bugfind/` reach `git add` and fail every
    /// capture.
    #[test]
    fn filter_ignored_sees_a_rule_that_the_index_masks() {
        let s = setup("filter-ignored-dir");
        let workspace = s.repo.workspace_path.clone();
        std::fs::create_dir_all(workspace.join("sub")).unwrap();
        std::fs::write(workspace.join("sub/deep.txt"), "deep\n").unwrap();
        std::fs::write(workspace.join(".gitignore"), "").unwrap();
        let index = s.repo.prepare_temp_index("filter-ignored-dir").unwrap();
        stage(&s.repo, &index, &["sub/deep.txt".to_string()]).unwrap();

        // The directory becomes excluded after the file was staged.
        std::fs::write(workspace.join(".gitignore"), "/sub/\n").unwrap();
        let (visible, ignored) = filter_ignored(
            &s.repo,
            &index,
            &["sub/deep.txt".to_string(), "other.txt".to_string()],
        )
        .unwrap();

        assert_eq!(visible, vec!["other.txt"]);
        assert_eq!(ignored.len(), 1);
        assert_eq!(ignored[0].path, "sub/deep.txt");
        assert_eq!(ignored[0].rule.as_deref(), Some(".gitignore:1:/sub/"));
    }
}
