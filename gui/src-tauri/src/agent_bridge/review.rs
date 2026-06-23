//! Shadow review wiring into the Run lifecycle (§6.1): capture the before
//! snapshot just before the prompt reaches the Agent; once the prompt future
//! returns, capture the after snapshot synchronously (under the prompt guard)
//! and then materialize the changeset off the IPC path (C1). All failures are
//! swallowed (logged) — review must never block a Run (§6.2).

use std::path::PathBuf;

use crate::shadow_review::{
    self, Limits, MaterializedDiff, ShadowRepo, VolumeRedline, VolumeVerdict,
};
use crate::store::{self, InsertReviewFileChangeInput, UpsertRunChangesetInput};
use crate::AppError;

/// Append metadata-only rows for changed sensitive files (§13): path + status,
/// no blob or diff.
fn append_sensitive_rows(diff: &mut MaterializedDiff, sensitive: &[String]) {
    for path in sensitive {
        diff.files.push(InsertReviewFileChangeInput {
            path: Some(path.clone()),
            change_type: "M".to_string(),
            omission_reason: Some("sensitive".to_string()),
            ..Default::default()
        });
        diff.files_changed += 1;
    }
}

struct ReviewContext {
    workspace_id: String,
    workspace_path: PathBuf,
    is_git: bool,
}

/// Resolve the Workspace context for a Thread, or `None` when shadow review
/// does not apply (chat threads keep Artifacts — §14.6).
fn resolve(thread_id: &str) -> Result<Option<ReviewContext>, AppError> {
    let Some(thread) = store::get_thread(thread_id)? else {
        return Ok(None);
    };
    if thread.mode != "workspace" {
        return Ok(None);
    }
    let Some(workspace) = store::get_workspace(&thread.workspace_id)? else {
        return Ok(None);
    };
    let workspace_path = PathBuf::from(&workspace.path);
    if !workspace_path.is_dir() {
        return Ok(None);
    }
    let is_git = crate::git_review::is_git_workspace(&workspace_path);
    Ok(Some(ReviewContext {
        workspace_id: workspace.id,
        workspace_path,
        is_git,
    }))
}

/// Capture the before snapshot. Non-git Workspaces over the volume red line are
/// skipped so they don't block every prompt (§6.7); the missing before row makes
/// the Run resolve to the "directory too large" state via capabilities.
pub fn capture_before(thread_id: &str, run_id: &str) {
    if let Err(error) = try_capture_before(thread_id, run_id) {
        eprintln!("FutureOS shadow review before-snapshot failed: {error}");
    }
}

fn try_capture_before(thread_id: &str, run_id: &str) -> Result<(), AppError> {
    let Some(ctx) = resolve(thread_id)? else {
        return Ok(());
    };
    if !ctx.is_git
        && shadow_review::evaluate_volume(&ctx.workspace_path, &VolumeRedline::default())
            == VolumeVerdict::TooLarge
    {
        return Ok(());
    }

    let repo = ShadowRepo::open(&ctx.workspace_id, &ctx.workspace_path, ctx.is_git)?;
    let result = shadow_review::with_workspace_lock(&ctx.workspace_id, || {
        shadow_review::capture(&repo, thread_id, run_id, "before", &Limits::default())?;
        Ok(())
    });
    if let Err(error) = result {
        // Record a failed before row so the changeset resolves to `unavailable`
        // rather than reading as "no changes" (§6.3).
        let _ =
            shadow_review::record_failure(&repo, thread_id, run_id, "before", &error.to_string());
        return Err(error);
    }
    Ok(())
}

/// Convenience for synchronous callers (tests): capture the after snapshot then
/// materialize the changeset in one call. The lifecycle splits these — capture
/// runs under the prompt guard, materialize runs off the IPC path (§6.1).
#[cfg(test)]
pub fn finalize_after(thread_id: &str, run_id: &str) {
    let sensitive = capture_after(thread_id, run_id);
    materialize_changeset(thread_id, run_id, sensitive);
}

/// Capture the after snapshot (§6.1). MUST run synchronously while the prompt
/// guard is held, so the next Run's before-snapshot can't interleave with it.
/// Returns the changed sensitive paths to carry into the (deferred) materialize.
pub fn capture_after(thread_id: &str, run_id: &str) -> Vec<String> {
    match try_capture_after(thread_id, run_id) {
        Ok(sensitive) => sensitive,
        Err(error) => {
            eprintln!("FutureOS shadow review after-snapshot failed: {error}");
            Vec::new()
        }
    }
}

fn try_capture_after(thread_id: &str, run_id: &str) -> Result<Vec<String>, AppError> {
    let Some(ctx) = resolve(thread_id)? else {
        return Ok(Vec::new());
    };
    // No before snapshot → Run was gated (too large) or not applicable.
    if store::get_review_snapshot(run_id, "before")?.is_none() {
        return Ok(Vec::new());
    }

    let repo = ShadowRepo::open(&ctx.workspace_id, &ctx.workspace_path, ctx.is_git)?;
    let limits = Limits::default();
    let outcome = shadow_review::with_workspace_lock(&ctx.workspace_id, || {
        shadow_review::capture(&repo, thread_id, run_id, "after", &limits)
    });
    match outcome {
        Ok(outcome) => Ok(outcome.sensitive),
        Err(error) => {
            let _ = shadow_review::record_failure(
                &repo,
                thread_id,
                run_id,
                "after",
                &error.to_string(),
            );
            Ok(Vec::new())
        }
    }
}

/// Materialize the changeset from the before/after snapshots, mark overlap, and
/// enforce retention (§7.1, §12.3, §12.5). This is a read-only diff between fixed
/// commits, so it is safe to run off the IPC path.
pub fn materialize_changeset(thread_id: &str, run_id: &str, sensitive: Vec<String>) {
    if let Err(error) = try_materialize_changeset(thread_id, run_id, sensitive) {
        eprintln!("FutureOS shadow review materialize failed: {error}");
    }
}

fn try_materialize_changeset(
    thread_id: &str,
    run_id: &str,
    sensitive: Vec<String>,
) -> Result<(), AppError> {
    let Some(ctx) = resolve(thread_id)? else {
        return Ok(());
    };
    let Some(before) = store::get_review_snapshot(run_id, "before")? else {
        return Ok(());
    };
    let after = store::get_review_snapshot(run_id, "after")?;

    // Diffing between commits needs only the object DB — a bare handle.
    let repo = ShadowRepo::open_bare(&ctx.workspace_id)?;
    let limits = Limits::default();

    // Materialize the diff only when both ends produced a real commit.
    let both_ok = before.status != "failed"
        && after
            .as_ref()
            .map(|a| a.status != "failed")
            .unwrap_or(false);
    let materialized = match (both_ok, before.commit_id.as_deref(), after.as_ref()) {
        (true, Some(before_commit), Some(after_snap)) => match after_snap.commit_id.as_deref() {
            Some(after_commit) => {
                shadow_review::materialize(&repo, before_commit, after_commit, &limits).ok()
            }
            None => None,
        },
        _ => None,
    };

    let completeness = match (
        before.status.as_str(),
        after.as_ref().map(|a| a.status.as_str()),
    ) {
        (b, Some(a)) if b == "partial" || a == "partial" => "partial",
        _ => "complete",
    };
    let omitted_files = after
        .as_ref()
        .map(|a| a.omitted_count)
        .unwrap_or(before.omitted_count);

    let mut diff = materialized.unwrap_or_default();
    // §13: surface changed sensitive files as metadata-only rows (no content).
    append_sensitive_rows(&mut diff, &sensitive);

    store::upsert_run_changeset(UpsertRunChangesetInput {
        run_id: run_id.to_string(),
        thread_id: thread_id.to_string(),
        workspace_id: Some(ctx.workspace_id.clone()),
        title: "上一轮变更".to_string(),
        summary: None,
        before_snapshot_id: Some(before.id),
        after_snapshot_id: after.map(|a| a.id),
        files_changed: diff.files_changed,
        additions: diff.additions,
        deletions: diff.deletions,
        binary_files: diff.binary_files,
        omitted_files,
        completeness: completeness.to_string(),
        confidence: "normal".to_string(),
        error_message: None,
        files: diff.files,
    })?;

    // §12.5: flag this Run and any concurrently-overlapping peers.
    let _ = store::mark_run_overlapped(&ctx.workspace_id, run_id);
    // §12.3: prune old changesets/refs for this Thread and let git gc when due.
    shadow_review::enforce_retention(thread_id);
    Ok(())
}

/// Re-materialize a Run's changeset from its existing before/after commits
/// (§10.4). Only valid when both commits still exist; never fabricates.
pub fn retry(run_id: &str) -> Result<(), AppError> {
    let run = store::get_run(run_id)?.ok_or_else(|| "Run could not be loaded.".to_string())?;
    let thread_id = run.thread_id;
    let ctx = resolve(&thread_id)?
        .ok_or_else(|| "Shadow review does not apply to this Run.".to_string())?;

    let before = store::get_review_snapshot(run_id, "before")?
        .ok_or_else(|| "Missing before snapshot; cannot retry.".to_string())?;
    let after = store::get_review_snapshot(run_id, "after")?
        .ok_or_else(|| "Missing after snapshot; cannot retry.".to_string())?;
    let (Some(before_commit), Some(after_commit)) =
        (before.commit_id.as_deref(), after.commit_id.as_deref())
    else {
        return Err("Snapshots have no commits; cannot retry."
            .to_string()
            .into());
    };

    let repo = ShadowRepo::open(&ctx.workspace_id, &ctx.workspace_path, ctx.is_git)?;
    let limits = Limits::default();
    let diff = shadow_review::materialize(&repo, before_commit, after_commit, &limits)?;
    let completeness = if before.status == "partial" || after.status == "partial" {
        "partial"
    } else {
        "complete"
    };
    let omitted_files = after.omitted_count;

    store::upsert_run_changeset(UpsertRunChangesetInput {
        run_id: run_id.to_string(),
        thread_id,
        workspace_id: Some(ctx.workspace_id),
        title: "上一轮变更".to_string(),
        summary: None,
        before_snapshot_id: Some(before.id),
        after_snapshot_id: Some(after.id),
        files_changed: diff.files_changed,
        additions: diff.additions,
        deletions: diff.deletions,
        binary_files: diff.binary_files,
        omitted_files,
        completeness: completeness.to_string(),
        confidence: "normal".to_string(),
        error_message: None,
        files: diff.files,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::store::{self, CreateRunInput, CreateThreadInput, CreateWorkspaceInput};

    // Reproduces the GUI lifecycle (before snapshot → edit → after snapshot +
    // materialize) on a real non-git Workspace, verifying the shadow pipeline
    // neither hangs nor errors and produces a correct changeset.
    #[test]
    fn shadow_review_lifecycle_smoke() {
        let base = std::env::temp_dir().join(format!("futureos_shadow_{}", std::process::id()));
        let home = base.join("home");
        let workspace_dir = base.join("ws");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&workspace_dir).unwrap();
        std::env::set_var("HOME", &home);

        store::initialize_app_store().unwrap();

        let workspace = store::create_workspace(CreateWorkspaceInput {
            name: Some("smoke".into()),
            path: workspace_dir.display().to_string(),
            description: None,
            create_directory: Some(false),
        })
        .unwrap();
        let thread = store::create_thread(CreateThreadInput {
            mode: "workspace".into(),
            title: Some("smoke".into()),
            workspace_id: Some(workspace.id.clone()),
            workspace_path: None,
            workspace_name: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        let run = store::create_run(CreateRunInput {
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();

        // Pre-existing file (present before the Run).
        fs::write(workspace_dir.join("keep.txt"), "untouched\n").unwrap();

        super::capture_before(&thread.id, &run.id);

        // Simulate the agent's edits during the Run: a text edit, a text add,
        // and a binary add (PNG magic bytes + a NUL).
        fs::write(workspace_dir.join("hello.txt"), "你好\nworld\n").unwrap();
        fs::write(workspace_dir.join("keep.txt"), "untouched\nchanged\n").unwrap();
        fs::write(
            workspace_dir.join("pic.png"),
            [0x89u8, 0x50, 0x4e, 0x47, 0x00, 0x01],
        )
        .unwrap();
        fs::write(workspace_dir.join(".env"), "SECRET=abc\n").unwrap();

        super::finalize_after(&thread.id, &run.id);

        let changeset = store::get_last_run_changeset(&thread.id)
            .unwrap()
            .expect("a changeset should exist after the Run");
        assert_eq!(changeset.source_kind, "run_snapshot");
        assert!(
            changeset.files_changed >= 3,
            "expected >= 3 changed files, got {}",
            changeset.files_changed
        );
        assert_eq!(changeset.binary_files, 1, "expected 1 binary file");

        let files = store::list_review_file_changes(&changeset.id).unwrap();
        let paths: Vec<_> = files.iter().filter_map(|f| f.path.clone()).collect();
        assert!(paths.iter().any(|p| p == "hello.txt"), "paths: {paths:?}");

        // A3: binary metadata is populated (size + MIME), no text diff.
        let png = files
            .iter()
            .find(|f| f.path.as_deref() == Some("pic.png"))
            .expect("pic.png should be in the changeset");
        assert!(png.binary, "pic.png should be binary");
        assert_eq!(png.mime.as_deref(), Some("image/png"));
        assert_eq!(
            png.after_size,
            Some(6),
            "after size should be the blob size"
        );
        assert!(png.before_size.is_none(), "added file has no before size");
        assert!(png.diff.is_none(), "binary file has no text diff");

        // B4: the changed .env is a sensitive metadata-only row (no diff), and
        // is counted as omitted (A4).
        let env = files
            .iter()
            .find(|f| f.path.as_deref() == Some(".env"))
            .expect(".env should appear as a sensitive row");
        assert_eq!(env.omission_reason.as_deref(), Some("sensitive"));
        assert!(env.diff.is_none(), "sensitive file stores no diff");
        assert!(
            changeset.omitted_files >= 1,
            "omitted_files should count .env"
        );

        // B3: retention keeps the newest 10 changesets per Thread. Seed 11 more
        // (12 total) and prune.
        for _ in 0..11 {
            let extra = store::create_run(CreateRunInput {
                thread_id: thread.id.clone(),
                trigger_message_id: None,
                model_provider: None,
                model_id: None,
            })
            .unwrap();
            store::upsert_run_changeset(store::UpsertRunChangesetInput {
                run_id: extra.id,
                thread_id: thread.id.clone(),
                workspace_id: Some(workspace.id.clone()),
                title: "上一轮变更".to_string(),
                completeness: "complete".to_string(),
                confidence: "normal".to_string(),
                ..Default::default()
            })
            .unwrap();
        }
        let pruned = store::prune_thread_changesets(&thread.id, 10).unwrap();
        assert_eq!(pruned.len(), 2, "12 changesets pruned to 10 → 2 removed");
        assert!(
            store::get_run_changeset(&run.id).unwrap().is_none(),
            "the oldest Run's changeset should be pruned"
        );

        let _ = fs::remove_dir_all(&base);
    }
}
