//! Phase 2 maintenance: retention + GC (§12.3), restart recovery (§6.6), and
//! the startup consistency check (§8.4). All entry points are best-effort —
//! failures are logged, never propagated.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::store::{self, UpsertRunChangesetInput};
use crate::AppError;

use super::diff::materialize;
use super::policy::Limits;
use super::repository::{with_workspace_lock, ShadowRepo};
use super::snapshot::capture;

/// Run changesets kept per Thread (§12.3).
const RETENTION_KEEP: usize = 10;

/// Prune a Thread's old run changesets, delete their shadow refs, and let git
/// gc when warranted. Called after each Run finalizes.
pub fn enforce_retention(thread_id: &str) {
    if let Err(error) = try_enforce_retention(thread_id) {
        eprintln!("FutureOS shadow review retention failed: {error}");
    }
}

fn try_enforce_retention(thread_id: &str) -> Result<(), AppError> {
    let pruned = store::prune_thread_changesets(thread_id, RETENTION_KEEP)?;
    if pruned.is_empty() {
        return Ok(());
    }

    let mut workspaces: HashSet<String> = HashSet::new();
    for (workspace_id, run_id) in &pruned {
        workspaces.insert(workspace_id.clone());
        if let Ok(repo) = ShadowRepo::open_bare(workspace_id) {
            let _ = repo.delete_ref(&ShadowRepo::snapshot_ref(thread_id, run_id, "before"));
            let _ = repo.delete_ref(&ShadowRepo::snapshot_ref(thread_id, run_id, "after"));
        }
    }
    for workspace_id in workspaces {
        if let Ok(repo) = ShadowRepo::open_bare(&workspace_id) {
            repo.gc_auto();
        }
    }
    Ok(())
}

/// On startup, run the consistency check then recover interrupted Runs. Safe to
/// call from a background thread.
pub fn run_startup_maintenance() {
    verify_consistency();
    recover_interrupted_runs();
}

/// Mark snapshots whose pinned commit has gone missing as `failed`, so their
/// changeset resolves to `unavailable` rather than reading a broken commit (§8.4).
fn verify_consistency() {
    if let Err(error) = try_verify_consistency() {
        eprintln!("FutureOS shadow review consistency check failed: {error}");
    }
}

fn try_verify_consistency() -> Result<(), AppError> {
    let mut repos: HashMap<String, Option<ShadowRepo>> = HashMap::new();
    for (snapshot_id, workspace_id, commit_id) in store::list_snapshots_with_commits()? {
        let repo = repos
            .entry(workspace_id.clone())
            .or_insert_with(|| ShadowRepo::open_bare(&workspace_id).ok());
        if let Some(repo) = repo {
            if !repo.commit_exists(&commit_id) {
                let _ = store::mark_snapshot_failed(&snapshot_id, "snapshot commit is missing");
            }
        }
    }
    Ok(())
}

/// Recover Runs interrupted by a crash (before snapshot present, no after/no
/// changeset): mark them cancelled and produce a `recovered` changeset whose
/// diff is the before → current-state delta (§6.6).
fn recover_interrupted_runs() {
    if let Err(error) = try_recover_interrupted_runs() {
        eprintln!("FutureOS shadow review recovery failed: {error}");
    }
}

fn try_recover_interrupted_runs() -> Result<(), AppError> {
    for (run_id, thread_id, workspace_id) in store::list_interrupted_runs()? {
        if let Err(error) = recover_one(&run_id, &thread_id, &workspace_id) {
            eprintln!("FutureOS shadow review recovery of run {run_id} failed: {error}");
        }
    }
    Ok(())
}

fn recover_one(run_id: &str, thread_id: &str, workspace_id: &str) -> Result<(), AppError> {
    // The Run was interrupted — settle it as cancelled if still open.
    if let Ok(Some(run)) = store::get_run(run_id) {
        if !matches!(run.status.as_str(), "completed" | "failed" | "cancelled") {
            let _ = store::update_run_status(store::UpdateRunStatusInput {
                run_id: run_id.to_string(),
                status: "cancelled".to_string(),
                error_message: Some("Interrupted by application restart.".to_string()),
                error_type: Some("interrupted".to_string()),
            });
        }
    }

    let Some(before) = store::get_review_snapshot(run_id, "before")? else {
        return Ok(());
    };
    let Some(workspace) = store::get_workspace(workspace_id)? else {
        return Ok(());
    };
    let path = PathBuf::from(&workspace.path);
    if !path.is_dir() {
        return Ok(());
    }
    let is_git = crate::git_review::is_git_workspace(&path);
    let repo = ShadowRepo::open(workspace_id, &path, is_git)?;
    let limits = Limits::default();

    let after = with_workspace_lock(workspace_id, || {
        capture(&repo, thread_id, run_id, "after", &limits)
    })?
    .snapshot;

    let (Some(before_commit), Some(after_commit)) =
        (before.commit_id.as_deref(), after.commit_id.as_deref())
    else {
        return Ok(());
    };
    let diff = materialize(&repo, before_commit, after_commit, &limits).unwrap_or_default();
    let completeness = if before.status == "partial" || after.status == "partial" {
        "partial"
    } else {
        "complete"
    };

    store::upsert_run_changeset(UpsertRunChangesetInput {
        run_id: run_id.to_string(),
        thread_id: thread_id.to_string(),
        workspace_id: Some(workspace_id.to_string()),
        title: "上一轮变更".to_string(),
        summary: None,
        before_snapshot_id: Some(before.id),
        after_snapshot_id: Some(after.id),
        files_changed: diff.files_changed,
        additions: diff.additions,
        deletions: diff.deletions,
        binary_files: diff.binary_files,
        omitted_files: after.omitted_count,
        completeness: completeness.to_string(),
        // §6.6: app was closed during the Run, so the delta can't be fully
        // attributed to it.
        confidence: "recovered".to_string(),
        error_message: None,
        files: diff.files,
    })?;
    Ok(())
}
