//! The "previous turn changes" (last-run delta) read model (§10.3): assemble the
//! Thread's latest run_snapshot changeset, its file rows, the owning Run, and the
//! derived `snapshotStatus` (§8.5) into the payload the frontend renders. Pure
//! store + git reads — no agent involvement — so it lives in the review subsystem
//! rather than the thin command layer.

use std::path::Path;

use serde::Serialize;

use crate::store;
use crate::{git_review, AppError};

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

/// Assemble the last-run payload from a resolved `run_snapshot` changeset.
pub fn build_last_run_review(
    changeset: store::ReviewChangesetRecord,
) -> Result<LastRunReviewData, AppError> {
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
