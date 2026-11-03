//! Persist a Run's review from its recorded boundaries. This is shared by
//! normal completion, on-demand recovery and explicit retry.
use super::{Limits, MaterializedDiff, ShadowRepo};
use crate::store::{self, InsertReviewFileChangeInput, UpsertRunChangesetInput};
use crate::Error as AppError;

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

/// Common projection for normal completion and lazy recovery. Reads immutable
/// boundaries only: no current-workspace access and no Run status mutation.
pub(crate) fn materialize_recorded(
    thread_id: &str,
    run_id: &str,
    mut sensitive: Vec<String>,
) -> Result<(), AppError> {
    let Some(before) = store::get_review_snapshot(run_id, "before")? else {
        return Ok(());
    };
    let after = store::get_review_snapshot(run_id, "after")?;
    let boundaries = match (before.status.as_str(), &before.commit_id, &after) {
        ("failed", _, _) => None,
        (_, Some(before_commit), Some(after)) if after.status != "failed" => after
            .commit_id
            .as_deref()
            .map(|commit| (before_commit.as_str(), commit)),
        _ => None,
    };
    let has_boundaries = boundaries.is_some();
    let mut diff = if let Some((before_commit, after_commit)) = boundaries {
        let repo = ShadowRepo::open_bare(&before.workspace_id)?;
        super::materialize(&repo, before_commit, after_commit, &Limits::default())?
    } else {
        MaterializedDiff::default()
    };
    // A retry must retain metadata-only sensitive paths which were deliberately
    // excluded from Git. They cannot be reconstructed from the two commits.
    if let Some(previous) = store::get_run_changeset(run_id)? {
        sensitive.extend(
            store::list_review_file_changes(&previous.id)?
                .into_iter()
                .filter(|file| file.omission_reason.as_deref() == Some("sensitive"))
                .filter_map(|file| file.path),
        );
    }
    sensitive.sort_unstable();
    sensitive.dedup();
    append_sensitive_rows(&mut diff, &sensitive);
    let completeness = if !has_boundaries {
        "incomplete"
    } else if before.status == "partial" || after.as_ref().is_some_and(|a| a.status == "partial") {
        "partial"
    } else {
        "complete"
    };
    let error_message = if !has_boundaries {
        Some(
            after
                .as_ref()
                .and_then(|a| a.error_message.clone())
                .or_else(|| before.error_message.clone())
                .unwrap_or_else(|| "The Run has no complete pair of recorded snapshots.".into()),
        )
    } else {
        None
    };
    if crate::request_scope::is_stopping() {
        return Err("Review cancelled because the host is stopping.".into());
    }
    let omitted_files = after
        .as_ref()
        .map_or(before.omitted_count, |a| a.omitted_count);
    store::upsert_run_changeset(UpsertRunChangesetInput {
        run_id: run_id.to_string(),
        thread_id: thread_id.to_string(),
        workspace_id: Some(before.workspace_id.clone()),
        title: store::LAST_RUN_CHANGESET_TITLE.to_string(),
        summary: None,
        before_snapshot_id: Some(before.id),
        after_snapshot_id: after.map(|a| a.id),
        files_changed: diff.files_changed,
        additions: diff.additions,
        deletions: diff.deletions,
        binary_files: diff.binary_files,
        omitted_files,
        completeness: completeness.to_string(),
        confidence: if has_boundaries {
            "normal"
        } else {
            "incomplete"
        }
        .into(),
        error_message,
        files: diff.files,
    })?;
    store::mark_run_overlapped(&before.workspace_id, run_id)?;
    Ok(())
}
