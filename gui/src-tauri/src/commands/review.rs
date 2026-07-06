//! Review changeset and git-diff Tauri commands.

use std::path::Path;

use serde::Serialize;

use crate::shadow_review::{self, VolumeRedline, VolumeVerdict};
use crate::{agent_bridge, git_review, store};

/// Workspace review capabilities for the frontend (§10.1). `changePreview`
/// flips to `unsupported_too_large` for oversized non-git Workspaces (§6.7).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceReviewCapabilities {
    is_git_workspace: bool,
    views: Vec<String>,
    default_view: String,
    change_preview: String,
}

/// The "previous turn changes" payload for a Thread (§10.3).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastRunReviewData {
    changeset: store::ReviewChangesetRecord,
    files: Vec<store::ReviewFileChangeRecord>,
    run: Option<store::RunRecord>,
    snapshot_status: String,
    confidence: String,
    overlapped: bool,
}

#[tauri::command]
pub fn get_workspace_review_capabilities(
    workspace_id: String,
) -> Result<WorkspaceReviewCapabilities, crate::AppError> {
    let workspace = store::get_workspace(&workspace_id)?
        .ok_or_else(|| "Workspace could not be loaded.".to_string())?;
    let path = Path::new(&workspace.path);
    let is_git = git_review::is_git_workspace(path);

    let change_preview = if !is_git
        && shadow_review::evaluate_volume(path, &VolumeRedline::default())
            == VolumeVerdict::TooLarge
    {
        "unsupported_too_large"
    } else {
        "ready"
    };

    let (views, default_view) = if is_git {
        (
            vec!["git_changes".to_string(), "last_run".to_string()],
            "git_changes".to_string(),
        )
    } else {
        (vec!["last_run".to_string()], "last_run".to_string())
    };

    Ok(WorkspaceReviewCapabilities {
        is_git_workspace: is_git,
        views,
        default_view,
        change_preview: change_preview.to_string(),
    })
}

#[tauri::command]
pub fn get_last_run_review(
    thread_id: String,
) -> Result<Option<LastRunReviewData>, crate::AppError> {
    let Some(changeset) = store::get_last_run_changeset(&thread_id)? else {
        return Ok(None);
    };
    Ok(Some(build_last_run_review(changeset)?))
}

#[tauri::command]
pub fn retry_run_review(run_id: String) -> Result<Option<LastRunReviewData>, crate::AppError> {
    agent_bridge::retry_run_review(&run_id)?;
    let Some(changeset) = store::get_run_changeset(&run_id)? else {
        return Ok(None);
    };
    Ok(Some(build_last_run_review(changeset)?))
}

fn build_last_run_review(
    changeset: store::ReviewChangesetRecord,
) -> Result<LastRunReviewData, crate::AppError> {
    let files = store::list_review_file_changes(&changeset.id)?;
    let run = changeset
        .run_id
        .as_deref()
        .and_then(|run_id| store::get_run(run_id).ok().flatten());

    let is_git = changeset
        .workspace_id
        .as_deref()
        .and_then(|id| store::get_workspace(id).ok().flatten())
        .map(|workspace| git_review::is_git_workspace(Path::new(&workspace.path)))
        .unwrap_or(false);

    let before = changeset
        .run_id
        .as_deref()
        .and_then(|run_id| store::get_review_snapshot(run_id, "before").ok().flatten());
    let after = changeset
        .run_id
        .as_deref()
        .and_then(|run_id| store::get_review_snapshot(run_id, "after").ok().flatten());

    let snapshot_status = derive_snapshot_status(is_git, &changeset, &before, &after);

    Ok(LastRunReviewData {
        snapshot_status,
        confidence: changeset.confidence.clone(),
        overlapped: changeset.overlapped,
        files,
        run,
        changeset,
    })
}

/// Derive `snapshotStatus` (§8.5). Non-git Workspaces collapse `partial` /
/// `incomplete` to `unavailable` (§6.7).
fn derive_snapshot_status(
    is_git: bool,
    changeset: &store::ReviewChangesetRecord,
    before: &Option<store::ReviewSnapshotRecord>,
    after: &Option<store::ReviewSnapshotRecord>,
) -> String {
    let before_ok = before
        .as_ref()
        .map(|s| s.status != "failed")
        .unwrap_or(false);
    if !before_ok {
        return "unavailable".to_string();
    }
    let after_ok = after
        .as_ref()
        .map(|s| s.status != "failed")
        .unwrap_or(false);
    if !after_ok {
        return if is_git { "incomplete" } else { "unavailable" }.to_string();
    }
    if changeset.completeness == "partial" {
        return if is_git { "partial" } else { "unavailable" }.to_string();
    }
    "complete".to_string()
}

#[tauri::command]
pub fn get_git_review(
    workspace_id: String,
    base: Option<String>,
    custom_base: Option<String>,
) -> Result<git_review::GitReview, crate::AppError> {
    git_review::get_git_review(workspace_id, base, custom_base)
}
