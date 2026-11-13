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

/// Start a held message now, because the reader asked for it.
pub async fn release_queued_run(thread_id: &str, run_id: &str) -> Result<(), crate::Error> {
    let (session_id, run_id) = queued_run_session(thread_id, run_id)?;
    agent_bridge::release_queued_run(&session_id, &run_id).await
}

/// Take one held message back out of the queue before it leaves.
pub async fn cancel_queued_run(thread_id: &str, run_id: &str) -> Result<(), crate::Error> {
    let (session_id, run_id) = queued_run_session(thread_id, run_id)?;
    agent_bridge::cancel_queued_run(&session_id, &run_id).await
}

/// Correct one waiting message's words, without moving it in the queue.
///
/// The queue is the Agent's and it is ordered, so an edit is not a removal and a
/// re-send: a re-send would enter at the back, after messages the reader wrote
/// later. The words change where the row already waits; its files, images and
/// frozen run settings stay as they were accepted.
pub async fn edit_queued_run(
    thread_id: &str,
    run_id: &str,
    message: &str,
) -> Result<(), crate::Error> {
    let (session_id, run_id) = queued_run_session(thread_id, run_id)?;
    agent_bridge::edit_queued_run(&session_id, &run_id, message).await
}

/// The conversation a queued run belongs to, and the run id to address.
///
/// The conversation comes from the caller — it is rendering that conversation's
/// queue, so it knows which one the row is on — and the local run row is only a
/// cross-check. Resolving the conversation *from the row* used to be the whole
/// rule, and it made a waiting message impossible to take back whenever the row
/// had no local run behind it: a prompt the Agent accepted from the goal plane,
/// from `future run --session`, or from any other client writing to it directly
/// is in the same queue (the queue is the Agent's, not this app's), but nothing
/// ever created a `runs` row for it here. Both buttons then failed at this
/// lookup, the window swallowed the error, and the row stayed put with nothing
/// said about why.
///
/// A row that *is* known here still decides one thing: it must belong to the same
/// conversation the caller named. Cancelling by a run id against the wrong
/// session would either fail or, worse, cancel somebody else's prompt.
fn queued_run_session(thread_id: &str, run_id: &str) -> Result<(String, String), crate::Error> {
    let run = store::get_run(run_id)?;
    // The caller's conversation decides; a call that names none (an older
    // frontend) falls back to the row's own, which is the only other address
    // there is.
    let thread_id = match (thread_id.trim(), &run) {
        ("", Some(run)) => run.thread_id.clone(),
        ("", None) => return Err("That queued message could not be found.".to_string().into()),
        (given, _) => given.to_string(),
    };
    let thread = store::get_thread(&thread_id)?
        .ok_or_else(|| "That queued message's conversation could not be found.".to_string())?;
    if let Some(run) = run {
        if run.thread_id != thread.id {
            return Err(format!("Queued message {run_id} belongs to another conversation.").into());
        }
    }
    let session_id = thread
        .agent_session_id
        .clone()
        .unwrap_or_else(|| thread.id.clone());
    Ok((session_id, run_id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::test_support::HomeGuard;
    use crate::store;

    fn seeded(label: &str) -> (HomeGuard, store::ThreadRecord) {
        let home = HomeGuard::new(label);
        crate::store::initialize_app_store().expect("init store");
        let ws = crate::store::create_workspace(store::CreateWorkspaceInput {
            name: Some("WS".into()),
            path: std::env::temp_dir()
                .join(format!("futureos-queue-ws-{}", std::process::id()))
                .display()
                .to_string(),
            description: None,
            create_directory: Some(true),
        })
        .expect("create workspace");
        let thread = crate::store::create_thread(store::CreateThreadInput {
            mode: "workspace".into(),
            title: Some("Queue".into()),
            workspace_id: Some(ws.id.clone()),
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some("agent-session".into()),
        })
        .expect("create thread");
        (home, thread)
    }

    /// The row a goal note or a CLI prompt leaves behind has no local run; the
    /// conversation the panel names is still enough to address it.
    #[test]
    fn a_queued_run_without_a_local_row_resolves_through_its_conversation() {
        let (_home, thread) = seeded("queue-no-local-run");
        let (session_id, run_id) = queued_run_session(&thread.id, "run_from_elsewhere")
            .expect("the named conversation can address the row");
        assert_eq!(session_id, "agent-session");
        assert_eq!(run_id, "run_from_elsewhere");
    }

    /// The local row, when there is one, still says which conversation the run
    /// belongs to — and a pair that disagrees is refused rather than acted on.
    #[test]
    fn a_run_recorded_in_another_conversation_is_refused() {
        let (_home, thread) = seeded("queue-foreign-run");
        let other = store::create_thread(store::CreateThreadInput {
            mode: "workspace".into(),
            title: Some("Other".into()),
            workspace_id: Some(thread.workspace_id.clone()),
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some("other-session".into()),
        })
        .expect("create second thread");
        store::create_run(store::CreateRunInput {
            id: Some("run_foreign".into()),
            thread_id: other.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .expect("create run");

        let error = queued_run_session(&thread.id, "run_foreign")
            .expect_err("a run from another conversation must not be cancelled through this one")
            .to_string();
        assert!(error.contains("another conversation"), "{error}");
    }

    /// Without the panel's conversation and without a row there is nothing to
    /// address: say so instead of guessing a session.
    #[test]
    fn an_unknown_row_with_no_conversation_is_an_error() {
        let (_home, _thread) = seeded("queue-unaddressable");
        let error = queued_run_session("", "run_unknown")
            .expect_err("an unaddressable row must not be guessed at")
            .to_string();
        assert!(error.contains("could not be found"), "{error}");
    }
}
