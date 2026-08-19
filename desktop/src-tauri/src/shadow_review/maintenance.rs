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
        repos
            .entry(workspace_id.clone())
            .or_insert_with(|| ShadowRepo::open_bare(&workspace_id).ok())
            .as_mut()
            .inspect(|repo| {
                if !repo.commit_exists(&commit_id) {
                    let _ = store::mark_snapshot_failed(&snapshot_id, "snapshot commit is missing");
                }
            });
    }
    Ok(())
}

/// Recover Runs left without a materialized changeset by a crash (§6.6, B-6):
///   - interrupted (no `after`): settle cancelled, capture the current state as
///     the after, and mark the result `recovered`;
///   - finished-but-unmaterialized (`after` present): reuse the captured after
///     verbatim and mark it `normal` — the diff is fully attributable.
fn recover_interrupted_runs() {
    if let Err(error) = try_recover_interrupted_runs() {
        eprintln!("FutureOS shadow review recovery failed: {error}");
    }
}

fn try_recover_interrupted_runs() -> Result<(), AppError> {
    for (run_id, thread_id, workspace_id) in store::list_unmaterialized_runs()? {
        match recover_one(&run_id, &thread_id, &workspace_id) {
            Ok(()) => {}
            Err(error) => {
                eprintln!("FutureOS shadow review recovery of run {run_id} failed: {error}");
            }
        }
    }
    Ok(())
}

fn recover_one(run_id: &str, thread_id: &str, workspace_id: &str) -> Result<(), AppError> {
    let Some(before) = store::get_review_snapshot(run_id, "before")? else {
        return Ok(());
    };

    // Two recovery shapes share the materialize + upsert tail below, differing
    // only in how the `after` snapshot is obtained and how confident we are that
    // the delta belongs to this Run.
    let (after, confidence) = match store::get_review_snapshot(run_id, "after")? {
        // B-6: the Run finished and captured its after, but the deferred
        // materialize never ran before exit. Reuse the after as-is.
        Some(after) => (after, "normal"),
        // §6.6: the Run was interrupted before its after snapshot. Settle it
        // cancelled, then capture the current workspace state as the after.
        None => {
            // CAS instead of read-then-write: only settle a run that is still
            // non-terminal, atomically, so a run that finished in the startup
            // window isn't rewritten to cancelled.
            let _ = store::update_run_status_if_active(store::UpdateRunStatusInput {
                run_id: run_id.to_string(),
                status: "cancelled".to_string(),
                error_message: Some("Interrupted by application restart.".to_string()),
                error_type: Some("interrupted".to_string()),
            });

            // FK invariant: `list_unmaterialized_runs` only returns snapshots
            // whose workspace_id references an existing workspace row, so the
            // workspace lookup can never be `None` here.
            let workspace = store::get_workspace(workspace_id)?
                .expect("snapshot workspace is FK-guaranteed to exist");
            let path = PathBuf::from(&workspace.path);
            if !path.is_dir() {
                return Ok(());
            }
            let is_git = crate::git_review::is_git_workspace(&path);
            let repo = ShadowRepo::open(workspace_id, &path, is_git)?;
            let after = with_workspace_lock(workspace_id, || {
                capture(&repo, thread_id, run_id, "after", &Limits::default())
            })?
            .snapshot;
            (after, "recovered")
        }
    };

    let (Some(before_commit), Some(after_commit)) =
        (before.commit_id.as_deref(), after.commit_id.as_deref())
    else {
        return Ok(());
    };
    // Diffing between fixed commits needs only the object DB — a bare handle.
    let repo = ShadowRepo::open_bare(workspace_id)?;
    let limits = Limits::default();
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
        title: crate::store::LAST_RUN_CHANGESET_TITLE.to_string(),
        summary: None,
        before_snapshot_id: Some(before.id),
        after_snapshot_id: Some(after.id),
        files_changed: diff.files_changed,
        additions: diff.additions,
        deletions: diff.deletions,
        binary_files: diff.binary_files,
        omitted_files: after.omitted_count,
        completeness: completeness.to_string(),
        confidence: confidence.to_string(),
        error_message: None,
        files: diff.files,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::auth_store::test_support::HomeGuard;
    use crate::store::{
        CreateReviewSnapshotInput, CreateRunInput, CreateThreadInput, CreateWorkspaceInput,
        UpsertRunChangesetInput,
    };

    struct Setup {
        _home: HomeGuard,
        workspace_id: String,
        thread_id: String,
        run_id: String,
        dir: PathBuf,
        repo: ShadowRepo,
    }

    fn setup(name: &str) -> Setup {
        let home = HomeGuard::new(name);
        store::initialize_app_store().unwrap();
        let dir =
            std::env::temp_dir().join(format!("futureos-maint-{name}-{}", std::process::id()));
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
            dir,
            repo,
        }
    }

    #[test]
    fn wrappers_log_errors_when_store_uninitialized() {
        let _home = HomeGuard::new("maintenance-uninit");
        // No initialize_app_store: the empty DB has no tables, so each
        // best-effort entry point fails and logs instead of panicking.
        enforce_retention("thread-1");
        run_startup_maintenance();
    }

    #[test]
    fn retention_returns_early_when_nothing_to_prune() {
        let s = setup("retention-empty");
        try_enforce_retention(&s.thread_id).unwrap();
    }

    #[test]
    fn retention_prunes_and_deletes_refs() {
        let s = setup("retention-prune");
        for _ in 0..11 {
            let run = store::create_run(CreateRunInput {
                id: None,
                thread_id: s.thread_id.clone(),
                trigger_message_id: None,
                model_provider: None,
                model_id: None,
            })
            .unwrap();
            store::upsert_run_changeset(UpsertRunChangesetInput {
                run_id: run.id.clone(),
                thread_id: s.thread_id.clone(),
                workspace_id: Some(s.workspace_id.clone()),
                title: "t".into(),
                completeness: "complete".into(),
                confidence: "normal".into(),
                ..Default::default()
            })
            .unwrap();
        }
        try_enforce_retention(&s.thread_id).unwrap();
    }

    #[test]
    fn consistency_marks_missing_commits() {
        let s = setup("consistency");
        // A snapshot whose commit never existed in the shadow repo is failed.
        store::create_review_snapshot(CreateReviewSnapshotInput {
            workspace_id: s.workspace_id.clone(),
            thread_id: s.thread_id.clone(),
            run_id: s.run_id.clone(),
            phase: "before".into(),
            commit_id: Some("0000000000000000000000000000000000000000".into()),
            tree_id: Some("tree".into()),
            status: "complete".into(),
            ..Default::default()
        })
        .unwrap();
        // A real commit (via capture) stays untouched.
        std::fs::write(s.dir.join("a.txt"), "v1\n").unwrap();
        super::super::snapshot::capture(
            &s.repo,
            &s.thread_id,
            &s.run_id,
            "after",
            &Limits::default(),
        )
        .unwrap();

        try_verify_consistency().unwrap();

        assert_eq!(
            store::get_review_snapshot(&s.run_id, "before")
                .unwrap()
                .unwrap()
                .status,
            "failed"
        );
        assert_eq!(
            store::get_review_snapshot(&s.run_id, "after")
                .unwrap()
                .unwrap()
                .status,
            "complete"
        );
    }

    #[test]
    fn recover_one_returns_early_without_before() {
        let s = setup("recover-no-before");
        recover_one(&s.run_id, &s.thread_id, &s.workspace_id).unwrap();
        assert!(store::get_run_changeset(&s.run_id).unwrap().is_none());
    }

    #[test]
    fn recover_one_returns_early_without_commit_ids() {
        let s = setup("recover-no-commit");
        for phase in ["before", "after"] {
            store::create_review_snapshot(CreateReviewSnapshotInput {
                workspace_id: s.workspace_id.clone(),
                thread_id: s.thread_id.clone(),
                run_id: s.run_id.clone(),
                phase: phase.into(),
                status: "complete".into(),
                ..Default::default()
            })
            .unwrap();
        }
        recover_one(&s.run_id, &s.thread_id, &s.workspace_id).unwrap();
        assert!(store::get_run_changeset(&s.run_id).unwrap().is_none());
    }

    #[test]
    fn recover_one_finished_reuses_after_snapshot() {
        let s = setup("recover-finished");
        std::fs::write(s.dir.join("a.txt"), "v1\n").unwrap();
        super::super::snapshot::capture(
            &s.repo,
            &s.thread_id,
            &s.run_id,
            "before",
            &Limits::default(),
        )
        .unwrap();
        std::fs::write(s.dir.join("a.txt"), "v2\n").unwrap();
        super::super::snapshot::capture(
            &s.repo,
            &s.thread_id,
            &s.run_id,
            "after",
            &Limits::default(),
        )
        .unwrap();

        recover_one(&s.run_id, &s.thread_id, &s.workspace_id).unwrap();
        let cs = store::get_run_changeset(&s.run_id).unwrap().unwrap();
        assert_eq!(cs.confidence, "normal");
        assert_eq!(cs.files_changed, 1);
    }

    #[test]
    fn recover_one_interrupted_captures_after() {
        let s = setup("recover-interrupted");
        std::fs::write(s.dir.join("a.txt"), "v1\n").unwrap();
        super::super::snapshot::capture(
            &s.repo,
            &s.thread_id,
            &s.run_id,
            "before",
            &Limits::default(),
        )
        .unwrap();
        // Edit but do not capture the after snapshot — the interruption shape.
        std::fs::write(s.dir.join("a.txt"), "v2\n").unwrap();

        recover_one(&s.run_id, &s.thread_id, &s.workspace_id).unwrap();
        let cs = store::get_run_changeset(&s.run_id).unwrap().unwrap();
        assert_eq!(cs.confidence, "recovered");
        assert_eq!(
            store::get_run(&s.run_id).unwrap().unwrap().status,
            "cancelled"
        );
    }

    #[test]
    fn try_recover_runs_over_a_successful_recovery() {
        let s = setup("recover-loop");
        std::fs::write(s.dir.join("a.txt"), "v1\n").unwrap();
        super::super::snapshot::capture(
            &s.repo,
            &s.thread_id,
            &s.run_id,
            "before",
            &Limits::default(),
        )
        .unwrap();
        // list_unmaterialized_runs returns this run; recover_one succeeds and
        // the loop's Ok path is exercised.
        try_recover_interrupted_runs().unwrap();
        assert!(store::get_run_changeset(&s.run_id).unwrap().is_some());
    }

    #[test]
    fn try_recover_logs_and_continues_past_a_failed_recovery() {
        let s = setup("recover-err");
        // A "before" snapshot (no "after") makes list_unmaterialized_runs return
        // this run; the interrupted branch then re-opens the workspace to capture
        // the after snapshot.
        std::fs::write(s.dir.join("a.txt"), "v1\n").unwrap();
        super::super::snapshot::capture(
            &s.repo,
            &s.thread_id,
            &s.run_id,
            "before",
            &Limits::default(),
        )
        .unwrap();
        // Collide the shadow review root for this workspace with a file so the
        // interrupted-branch `ShadowRepo::open` fails → the logging error arm.
        let review_root = crate::store::app_data_path().unwrap().app_dir;
        let ws_root = PathBuf::from(review_root)
            .join("review")
            .join(&s.workspace_id);
        let _ = std::fs::remove_dir_all(&ws_root);
        std::fs::write(&ws_root, b"").unwrap();

        // The failure is logged and swallowed; recovery continues and returns Ok.
        try_recover_interrupted_runs().unwrap();
        // The run was settled cancelled before the reopen failed.
        assert_eq!(
            store::get_run(&s.run_id).unwrap().unwrap().status,
            "cancelled"
        );
    }

    #[test]
    fn recover_one_skips_workspace_path_that_is_no_longer_a_dir() {
        let s = setup("recover-no-dir");
        std::fs::write(s.dir.join("a.txt"), "v1\n").unwrap();
        super::super::snapshot::capture(
            &s.repo,
            &s.thread_id,
            &s.run_id,
            "before",
            &Limits::default(),
        )
        .unwrap();
        // Replace the workspace directory with a regular file: the interrupted
        // branch must bail (not capture) rather than error.
        let _ = std::fs::remove_dir_all(&s.dir);
        std::fs::write(&s.dir, b"not-a-dir").unwrap();

        recover_one(&s.run_id, &s.thread_id, &s.workspace_id).unwrap();
        assert!(store::get_run_changeset(&s.run_id).unwrap().is_none());
    }

    #[test]
    fn recover_one_marks_partial_when_a_snapshot_is_partial() {
        let s = setup("recover-partial");
        for (phase, status) in [("before", "partial"), ("after", "complete")] {
            store::create_review_snapshot(CreateReviewSnapshotInput {
                workspace_id: s.workspace_id.clone(),
                thread_id: s.thread_id.clone(),
                run_id: s.run_id.clone(),
                phase: phase.into(),
                commit_id: Some(format!("commit-{phase}")),
                tree_id: Some(format!("tree-{phase}")),
                status: status.into(),
                ..Default::default()
            })
            .unwrap();
        }
        recover_one(&s.run_id, &s.thread_id, &s.workspace_id).unwrap();
        let cs = store::get_run_changeset(&s.run_id).unwrap().unwrap();
        assert_eq!(cs.completeness, "partial");
        assert_eq!(cs.confidence, "normal");
    }
}
