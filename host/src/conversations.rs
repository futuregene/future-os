//! Conversation lifetime, shared by the GUI commands and the tasks host.
//!
//! "Delete a conversation" has to mean one thing wherever it is asked for: the
//! user pressing delete in the sidebar, a workspace delete taking its
//! conversations with it, and a task that does not keep a conversation per run.
//! The sequence — stop whatever is running in it, close the shells opened from
//! it, delete the rows and the Agent session behind them, then drain the delete
//! outbox — lives here so those callers cannot drift apart.
//!
//! Compiled in both builds: the tasks host runs headless too. The only GUI-only
//! step is closing embedded terminals, supplied by the entry presentation sink.

use crate::agent_bridge;
use crate::store;

/// Delete one conversation and everything that belongs to it.
///
/// The recursive target set is resolved *before* anything is deleted: a
/// conversation delete takes its descendants with it, so each of their shells
/// must close and each of their active runs must stop first. Shells opened from
/// a conversation are children of the app and must not outlive it — closing
/// them here means a deleted conversation can never leave an orphaned terminal
/// pointable at a removed directory. Errors when the thread is already gone.
pub async fn delete_conversation(
    thread_id: &str,
    delete_files: bool,
) -> Result<store::ThreadRecord, crate::Error> {
    let targets = store::thread_delete_closure(thread_id)?;
    close_terminals(&targets);
    for target in &targets {
        stop_active_session_before_delete(thread_session_id(target)).await?;
    }
    let thread = store::delete_thread_tree(thread_id, delete_files)?;
    for target in &targets {
        let session_id = thread_session_id(target);
        if store::is_agent_session_tombstoned(session_id)? {
            agent_bridge::drop_observer(session_id);
        }
    }
    agent_bridge::reconcile_delete_outbox().await;
    Ok(thread)
}

/// Stop and confirm any active Agent execution before its thread row or files
/// are removed. Inactive sessions avoid an unnecessary Agent round trip.
pub async fn stop_active_session_before_delete(session_id: &str) -> Result<(), crate::Error> {
    if !store::active_run_sessions()?
        .iter()
        .any(|active| active == session_id)
    {
        return Ok(());
    }
    if agent_bridge::abort_session(session_id).await?
        == agent_bridge::AbortSessionOutcome::SessionMissing
    {
        // Definitive absence already confirms idle. A second RPC could fail
        // after this answer and incorrectly prevent the local deletion.
        return Ok(());
    }
    if !agent_bridge::wait_for_agent_idle(session_id).await {
        return Err(
            "Future Agent did not confirm that the session stopped; deletion was cancelled."
                .to_string()
                .into(),
        );
    }
    Ok(())
}

/// The Agent session a GUI thread is bound to: its `agent_session_id`, else its
/// own id for an unbound thread (same resolution the store uses as a session
/// key when it tombstones a delete).
pub fn thread_session_id(thread: &store::ThreadRecord) -> &str {
    thread
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or(&thread.id)
}

/// Close every terminal tab a conversation owns. Called from the deletion
/// paths so terminal lifetime is bounded by conversation lifetime, independent
/// of whether a client still holds an open panel. The embedded terminal panel
/// is GUI-only (the server build has no shells to close).
fn close_terminals(targets: &[store::ThreadRecord]) {
    for target in targets {
        crate::events::publish(crate::events::Event::CloseThreadTerminals(&target.id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_bridge::test_support::{
        get_state_payload, mock_agent, seed_run, seed_thread, seed_workspace, Reply, TestHome,
    };

    #[tokio::test]
    async fn missing_sessions_delete_local_threads_and_runs_without_a_second_probe() {
        let home = TestHome::new("delete-confirmed-missing");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "workspace");
        for binding in [None, Some("missing-session")] {
            let thread = seed_thread(&workspace.id, binding);
            if let Some(session) = binding {
                store::record_thread_source(
                    &thread.id,
                    session,
                    "test-agent-database",
                    "test-generation",
                )
                .unwrap();
            }
            let run = seed_run(&thread.id);
            let session = thread_session_id(&thread);
            mock.push_state_for_session(
                session,
                Reply::RejectCode("session_not_found", "session not found".into()),
            );
            // If deletion polls again after confirmed absence, it will fail.
            mock.push_state_for_session(
                session,
                Reply::Status(tonic::Code::Unavailable, "agent restarted"),
            );

            delete_conversation(&thread.id, false).await.unwrap();

            assert!(store::get_thread(&thread.id).unwrap().is_none());
            assert!(store::get_run(&run.id).unwrap().is_none());
            assert_eq!(
                mock.requests_of("get_state")
                    .iter()
                    .filter(|request| request.session_id == session)
                    .count(),
                1
            );
        }
        assert!(mock.requests_of("abort").is_empty());
    }

    #[tokio::test]
    async fn a_session_disappearing_during_abort_still_allows_local_deletion() {
        let home = TestHome::new("delete-missing-during-abort");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "workspace");
        let thread = seed_thread(&workspace.id, Some("disappearing-session"));
        let run = seed_run(&thread.id);
        mock.push_state_for_session(
            "disappearing-session",
            Reply::TypedData(get_state_payload("disappearing-session", true)),
        );
        mock.push("abort", Reply::Reject("No such session".into()));
        mock.push_state_for_session(
            "disappearing-session",
            Reply::Status(tonic::Code::Unavailable, "agent restarted"),
        );

        delete_conversation(&thread.id, false).await.unwrap();

        assert!(store::get_thread(&thread.id).unwrap().is_none());
        assert!(store::get_run(&run.id).unwrap().is_none());
        assert_eq!(mock.requests_of("get_state").len(), 1);
        assert_eq!(mock.requests_of("abort").len(), 1);
    }

    #[tokio::test]
    async fn storage_failure_or_an_unconfirmed_abort_preserves_the_conversation() {
        let home = TestHome::new("delete-uncertain-session");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "workspace");
        let thread = seed_thread(&workspace.id, Some("unconfirmed-session"));
        let run = seed_run(&thread.id);
        mock.push_state_for_session(
            "unconfirmed-session",
            Reply::RejectCode(
                "session_storage_unavailable",
                "session not found in recovery archive".into(),
            ),
        );
        assert!(delete_conversation(&thread.id, false).await.is_err());
        assert!(mock.requests_of("abort").is_empty());

        mock.push_state_for_session(
            "unconfirmed-session",
            Reply::TypedData(get_state_payload("unconfirmed-session", true)),
        );
        mock.push_data("abort", serde_json::json!({}));
        mock.push_state_for_session(
            "unconfirmed-session",
            Reply::Status(tonic::Code::Unavailable, "agent unavailable"),
        );
        assert!(delete_conversation(&thread.id, false).await.is_err());
        assert!(store::get_thread(&thread.id).unwrap().is_some());
        assert_eq!(store::get_run(&run.id).unwrap().unwrap().status, "running");
    }
}
