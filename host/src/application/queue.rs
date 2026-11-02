//! The conversation queue, as the desktop window sees it.
//!
//! The queue is the Agent's (see `agent_bridge::queue`), so this is a thin,
//! shared entry point: the same one the Remote host answers a phone from, so a
//! prompt held for this machine and one held for a paired computer are described
//! the same way.

use crate::{agent_bridge, store};
use serde_json::Value;

/// The session's run in flight and the rows waiting behind it.
///
/// A thread without an agent session yet (a draft conversation) has nothing in
/// flight, which is an empty queue rather than an error.
pub async fn session_queue(thread_id: &str) -> Result<Value, crate::Error> {
    let Some(thread) = store::get_thread(thread_id)? else {
        return Ok(agent_bridge::empty_queue());
    };
    let session_id = thread
        .agent_session_id
        .clone()
        .unwrap_or_else(|| thread.id.clone());
    agent_bridge::session_queue_snapshot(&session_id).await
}

/// Take one held message back out of the queue before it leaves.
///
/// The row's conversation is resolved from the returned run rather than trusted
/// from the caller: a queue is per session, and cancelling by a run id against
/// the wrong session would either fail or, worse, cancel somebody else's prompt.
pub async fn cancel_queued_run(run_id: &str) -> Result<(), crate::Error> {
    let run = store::get_run(run_id)?
        .ok_or_else(|| "That queued message could not be found.".to_string())?;
    let thread = store::get_thread(&run.thread_id)?
        .ok_or_else(|| "That queued message's conversation could not be found.".to_string())?;
    let session_id = thread
        .agent_session_id
        .clone()
        .unwrap_or_else(|| thread.id.clone());
    agent_bridge::cancel_queued_run(&session_id, run_id).await
}
