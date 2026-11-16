//! Archive-time Shell output cleanup. Archiving stays successful when the
//! Agent cannot complete a cleanup; only confirmed rewrites are marked purged.

use crate::store;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CleanupReport {
    pub purged_sessions: usize,
    pub failed_sessions: usize,
}

impl CleanupReport {
    fn record(&mut self, outcome: Result<(), crate::Error>, thread_id: &str) {
        match outcome {
            Ok(()) => self.purged_sessions += 1,
            Err(error) => {
                self.failed_sessions += 1;
                tracing::warn!(event = "archive_run_log_cleanup_failed", thread_id, %error,
                    "conversation archived, but its Shell output cleanup did not complete");
            }
        }
    }
}

fn cleanup_enabled() -> bool {
    store::get_app_settings()
        .map(|settings| settings.archive_run_log_cleanup)
        .unwrap_or(false)
}

async fn purge_thread(thread: &store::ThreadRecord) -> Result<(), crate::Error> {
    let runs = store::list_runs(&thread.id)?;
    if runs.is_empty() || runs.iter().all(|run| run.logs_purged_at.is_some()) {
        return Ok(());
    }
    let Some(session_id) = thread
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return Ok(());
    };
    crate::agent_bridge::purge_archived_shell_outputs(session_id.to_string()).await?;
    store::mark_run_logs_purged(&thread.id, store::now_millis())?;
    Ok(())
}

/// Clean one already archived fork tree, best effort per member. A failure in
/// one session does not undo the archive or prevent the others from cleaning.
pub async fn thread_tree_if_enabled(thread_id: &str) -> CleanupReport {
    let mut report = CleanupReport::default();
    if !cleanup_enabled() {
        return report;
    }
    let members = match store::thread_archive_closure(thread_id) {
        Ok(members) => members,
        Err(error) => {
            report.record(Err(error), thread_id);
            return report;
        }
    };
    for member in &members {
        report.record(purge_thread(member).await, &member.id);
    }
    report
}

/// Clean every conversation hidden by a workspace archive. The workspace's
/// source directory and independent artifacts are never touched.
pub async fn workspace_if_enabled(workspace_id: &str) -> CleanupReport {
    let mut report = CleanupReport::default();
    if !cleanup_enabled() {
        return report;
    }
    let threads = match store::list_threads() {
        Ok(threads) => threads,
        Err(error) => {
            report.record(Err(error), workspace_id);
            return report;
        }
    };
    for thread in threads
        .iter()
        .filter(|thread| thread.workspace_id == workspace_id)
    {
        report.record(purge_thread(thread).await, &thread.id);
    }
    report
}
