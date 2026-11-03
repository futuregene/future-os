//! The "last-run changes" (last-run delta) read model (§10.3): assemble the
//! Thread's latest run_snapshot changeset, its file rows, the owning Run, and the
//! derived `snapshotStatus` (§8.5) into the payload the frontend renders. Pure
//! store + git reads — no agent involvement — so it lives in the review subsystem
//! rather than the thin command layer.

use serde::Serialize;

use crate::store;
use crate::Error;

/// The "last-run changes" payload for a Thread (§10.3).
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
) -> Result<LastRunReviewData, Error> {
    let files = store::list_review_file_changes(&changeset.id)?;
    let run = changeset
        .run_id
        .as_deref()
        .and_then(|run_id| store::get_run(run_id).ok().flatten());

    let before = changeset
        .run_id
        .as_deref()
        .and_then(|run_id| store::get_review_snapshot(run_id, "before").ok().flatten());
    let after = changeset
        .run_id
        .as_deref()
        .and_then(|run_id| store::get_review_snapshot(run_id, "after").ok().flatten());

    let snapshot_status = derive_snapshot_status(&changeset, &before, &after);

    Ok(LastRunReviewData {
        snapshot_status,
        confidence: changeset.confidence.clone(),
        overlapped: changeset.overlapped,
        files,
        run,
        changeset,
    })
}

/// Derive status entirely from the saved review, even when its workspace has
/// moved. Missing boundaries remain incomplete for Git and plain directories.
fn derive_snapshot_status(
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
        return "incomplete".to_string();
    }
    if changeset.completeness == "incomplete" {
        return "incomplete".to_string();
    }
    if changeset.completeness == "partial" {
        return "partial".to_string();
    }
    "complete".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::test_support::HomeGuard;
    use crate::store::{
        CreateReviewSnapshotInput, CreateRunInput, CreateThreadInput, CreateWorkspaceInput,
        InsertReviewFileChangeInput, UpsertRunChangesetInput,
    };

    fn changeset(completeness: &str) -> store::ReviewChangesetRecord {
        store::ReviewChangesetRecord {
            id: "cs".into(),
            thread_id: "thread".into(),
            run_id: Some("run".into()),
            tool_call_id: None,
            title: "t".into(),
            summary: None,
            status: "ready".into(),
            files_changed: 0,
            additions: 0,
            deletions: 0,
            source_kind: "run_snapshot".into(),
            workspace_id: Some("ws".into()),
            before_snapshot_id: None,
            after_snapshot_id: None,
            binary_files: 0,
            omitted_files: 0,
            completeness: completeness.into(),
            confidence: "high".into(),
            overlapped: false,
            error_message: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn snapshot(status: &str) -> store::ReviewSnapshotRecord {
        store::ReviewSnapshotRecord {
            id: "snap".into(),
            workspace_id: "ws".into(),
            thread_id: "thread".into(),
            run_id: "run".into(),
            phase: "before".into(),
            commit_id: None,
            tree_id: None,
            status: status.into(),
            file_count: 0,
            total_bytes: 0,
            ignored_count: 0,
            omitted_count: 0,
            error_message: None,
            created_at: 0,
        }
    }

    #[test]
    fn status_describes_recorded_boundaries_without_a_workspace_probe() {
        for (before, after, completeness, expected) in [
            (None, Some("complete"), "complete", "unavailable"),
            (Some("failed"), Some("complete"), "complete", "unavailable"),
            (Some("complete"), None, "complete", "incomplete"),
            (Some("complete"), Some("failed"), "complete", "incomplete"),
            (
                Some("complete"),
                Some("complete"),
                "incomplete",
                "incomplete",
            ),
            (Some("complete"), Some("complete"), "partial", "partial"),
            (Some("complete"), Some("complete"), "complete", "complete"),
        ] {
            assert_eq!(
                derive_snapshot_status(
                    &changeset(completeness),
                    &before.map(snapshot),
                    &after.map(snapshot)
                ),
                expected
            );
        }
    }

    #[test]
    fn build_last_run_review_assembles_payload() {
        let _home = HomeGuard::new("shadow-last-run");
        store::initialize_app_store().unwrap();

        let base = std::env::temp_dir().join(format!("futureos-lastrun-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        let ws = store::create_workspace(CreateWorkspaceInput {
            name: Some("lr".into()),
            path: base.display().to_string(),
            description: None,
            create_directory: Some(false),
        })
        .unwrap();
        let thread = store::create_thread(CreateThreadInput {
            mode: "workspace".into(),
            title: Some("lr".into()),
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

        for phase in ["before", "after"] {
            store::create_review_snapshot(CreateReviewSnapshotInput {
                workspace_id: ws.id.clone(),
                thread_id: thread.id.clone(),
                run_id: run.id.clone(),
                phase: phase.into(),
                status: "complete".into(),
                ..Default::default()
            })
            .unwrap();
        }

        let changeset = store::upsert_run_changeset(UpsertRunChangesetInput {
            run_id: run.id.clone(),
            thread_id: thread.id.clone(),
            workspace_id: Some(ws.id.clone()),
            title: "t".into(),
            completeness: "complete".into(),
            confidence: "high".into(),
            files_changed: 1,
            additions: 1,
            deletions: 0,
            files: vec![InsertReviewFileChangeInput {
                path: Some("hello.txt".into()),
                change_type: "M".into(),
                additions: 1,
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();

        let data = build_last_run_review(changeset.expect("changeset is written")).unwrap();
        assert_eq!(data.snapshot_status, "complete");
        assert_eq!(data.confidence, "high");
        assert!(!data.overlapped);
        assert!(data.run.is_some());
        assert_eq!(data.files.len(), 1);
    }
}
