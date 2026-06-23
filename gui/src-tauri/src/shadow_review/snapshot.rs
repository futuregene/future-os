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

    repo.update_ref(
        &ShadowRepo::snapshot_ref(thread_id, run_id, phase),
        &commit_id,
    )?;
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
    let untracked = repo.git_bytes(
        &["ls-files", "--others", "--exclude-standard", "-z"],
        Some(index),
    )?;

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
fn stage(repo: &ShadowRepo, index: &Path, paths: &[String]) -> Result<(), AppError> {
    if paths.is_empty() {
        return Ok(());
    }
    let stdin = paths.join("\0").into_bytes();
    let output = repo.run(
        &[
            "add",
            "--all",
            "--pathspec-from-file=-",
            "--pathspec-file-nul",
        ],
        Some(index),
        Some(&stdin),
    )?;
    if !output.status.success() {
        return Err(format!(
            "shadow git add failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
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
