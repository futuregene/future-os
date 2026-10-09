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
//! step is closing embedded terminals, which is guarded where it is called.

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
pub(crate) async fn delete_conversation(
    thread_id: &str,
    delete_files: bool,
) -> Result<store::ThreadRecord, crate::AppError> {
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
pub(crate) async fn stop_active_session_before_delete(
    session_id: &str,
) -> Result<(), crate::AppError> {
    if !store::active_run_sessions()?
        .iter()
        .any(|active| active == session_id)
    {
        return Ok(());
    }
    agent_bridge::abort_session(session_id).await?;
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
pub(crate) fn thread_session_id(thread: &store::ThreadRecord) -> &str {
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
    #[cfg(feature = "gui")]
    for target in targets {
        crate::commands::close_thread_terminals(&target.id);
    }
    #[cfg(not(feature = "gui"))]
    let _ = targets;
}
