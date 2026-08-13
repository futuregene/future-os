//! before/after snapshot capture (§5.4): reuse the persisted index, stage only
//! the changed candidate set, write a tree, reuse-or-create a commit, pin a ref,
//! and persist snapshot metadata.

use std::path::Path;

use crate::store::{self, CreateReviewSnapshotInput, ReviewSnapshotRecord};
use crate::AppError;

use super::policy::{self, Disposition, Limits};
use super::repository::ShadowRepo;

/// A captured snapshot plus the sensitive credential paths that changed this
/// round (omitted from the tree, surfaced as metadata-only rows — §13).
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
) -> Result<CaptureOutcome, AppError> {
    let tag = format!("{run_id}.{phase}");
    let temp_index = repo.prepare_temp_index(&tag)?;

    // info/exclude: real repo excludes (boundary 1) + non-git defaults (§5.5).
    let real_exclude = repo.real_repo_info_exclude();
    let exclude = policy::build_info_exclude(repo.is_git_workspace, real_exclude.as_deref(), &[]);
    repo.write_info_exclude(&exclude)?;

    let candidates = candidate_paths(repo, &temp_index)?;

    // Classify candidates, honouring the per-round limits (§5.5).
    let mut staged: Vec<String> = Vec::new();
    let mut sensitive: Vec<String> = Vec::new();
    let mut total_bytes: u64 = 0;
    let mut omitted: usize = 0;
    let mut over_limit = false;

    for path in &candidates {
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

    stage(repo, &temp_index, &staged)?;
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

    let snapshot = store::create_review_snapshot(CreateReviewSnapshotInput {
        workspace_id: repo.workspace_id.clone(),
        thread_id: thread_id.to_string(),
        run_id: run_id.to_string(),
        phase: phase.to_string(),
        commit_id: Some(commit_id),
        tree_id: Some(tree_id),
        status: status.to_string(),
        file_count: staged.len() as i64,
        total_bytes: total_bytes as i64,
        ignored_count: 0,
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
) -> Result<ReviewSnapshotRecord, AppError> {
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
fn candidate_paths(repo: &ShadowRepo, index: &Path) -> Result<Vec<String>, AppError> {
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

/// Stage only the given candidate paths (`--all` so deletions are recorded).
/// Paths that no longer exist and aren't tracked in the index are silently
/// skipped so stale shadow index entries don't break the snapshot.
fn stage(repo: &ShadowRepo, index: &Path, paths: &[String]) -> Result<(), AppError> {
    if paths.is_empty() {
        return Ok(());
    }
    let stdin = paths.join("\0").into_bytes();
    let add_args: &[&str] = &[
        "add",
        "--all",
        "--pathspec-from-file=-",
        "--pathspec-file-nul",
    ];
    let output = repo.run(add_args, Some(index), Some(&stdin))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // A pathspec that matches nothing (deleted file no longer tracked)
        // shouldn't fail the whole snapshot — skip it with a warning.
        if stderr.contains("pathspec") && stderr.contains("did not match") {
            eprintln!(
                "FutureOS shadow review: skipped {} paths that no longer exist",
                paths.len()
            );
            return Ok(());
        }
        return Err(format!("shadow git add failed: {}", stderr.trim()).into());
    }
    Ok(())
}

/// When capturing `after`, reuse the `before` commit if the trees match.
fn reuse_commit(run_id: &str, phase: &str, tree_id: &str) -> Result<Option<String>, AppError> {
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
    fn stage_surfaces_unexpected_git_error() {
        let s = setup("stage-err");
        // Make the index path a directory so git cannot write the index — a
        // non-pathspec failure that must be surfaced as an error.
        let index = s.repo.prepare_temp_index("stage-err").unwrap();
        std::fs::create_dir_all(&index).unwrap();
        let err = stage(&s.repo, &index, &["some-file.txt".to_string()]).unwrap_err();
        assert!(err.to_string().contains("shadow git add failed"));
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
}
