use super::{connect_agent, RpcResponseExt};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPresence {
    pub database_id: String,
    pub restore_generation: String,
    pub sessions: Vec<PresenceItem>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceItem {
    pub session_id: String,
    pub status: String,
}

pub async fn session_presence(ids: Vec<String>) -> Result<SessionPresence, crate::Error> {
    let mut client = connect_agent().await?;
    session_presence_with_client(&mut client, ids).await
}

/// How long a fence waits before treating `missing` as a fact.
///
/// The agent answers presence on a lane that does not queue behind its ordered
/// writer (it used to, which is how one long writer job stalled every read for
/// 18–38 s). What that costs is ordering: a session whose placeholder commit is
/// still queued reads `missing` for a moment. A session that is genuinely gone
/// stays gone, so one bounded second look — never a loop — separates the race
/// from the verdict. `deleted` and `import_failed` are decisions and are never
/// retried.
pub const PRESENCE_RECHECK_DELAY: std::time::Duration = std::time::Duration::from_millis(150);

/// Presence for a caller for whom `missing` means "refuse this command".
///
/// Asks twice: immediately, then once after [`PRESENCE_RECHECK_DELAY`] when the
/// first answer was `missing` and a commit could still be in flight. Every other
/// status is returned as-is, so a deleted or unimported conversation is refused
/// without the delay.
pub async fn require_present_with_client(
    client: &mut super::client::AgentClient,
    session_id: &str,
) -> Result<SessionPresence, crate::Error> {
    let first = session_presence_with_client(client, vec![session_id.to_string()]).await?;
    if first.sessions[0].status != "missing" {
        return Ok(first);
    }
    tokio::time::sleep(PRESENCE_RECHECK_DELAY).await;
    session_presence_with_client(client, vec![session_id.to_string()]).await
}

pub async fn session_presence_with_client(
    client: &mut super::client::AgentClient,
    ids: Vec<String>,
) -> Result<SessionPresence, crate::Error> {
    let response = client
        .execute_command(future_rpc::proto::RpcCommand {
            r#type: "get_session_presence".into(),
            session_ids: ids.clone(),
            ..Default::default()
        })
        .await
        .map_err(|error| format!("Session catalogue query failed: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Session catalogue query failed")?;
    let result: SessionPresence =
        serde_json::from_value(future_rpc::decode::response_data(&response))?;
    let expected: std::collections::HashSet<_> = ids.iter().map(String::as_str).collect();
    let actual: std::collections::HashSet<_> = result
        .sessions
        .iter()
        .map(|item| item.session_id.as_str())
        .collect();
    if result.database_id.is_empty()
        || result.restore_generation.is_empty()
        || expected != actual
        || result.sessions.len() != ids.len()
        || result.sessions.iter().any(|item| {
            !matches!(
                item.status.as_str(),
                "present" | "deleted" | "missing" | "import_failed"
            )
        })
    {
        return Err("Agent returned an incomplete session catalogue response.".into());
    }
    Ok(result)
}

pub async fn verify_thread_source(thread_id: &str, session_id: &str) -> Result<(), crate::Error> {
    let mut client = connect_agent().await?;
    verify_thread_source_with_client(&mut client, thread_id, session_id)
        .await
        .map(|_| ())
}

pub async fn verify_thread_source_with_client(
    client: &mut super::client::AgentClient,
    thread_id: &str,
    session_id: &str,
) -> Result<SessionPresence, crate::Error> {
    let presence = require_present_with_client(client, session_id).await?;
    if presence.sessions[0].status != "present" {
        return Err("Conversation source is unavailable; restore the matching Agent database before continuing.".into());
    }
    crate::store::record_thread_source(
        thread_id,
        session_id,
        &presence.database_id,
        &presence.restore_generation,
    )?;
    Ok(presence)
}

/// Fence known source bindings before mutations. Unbound/orphaned runs keep
/// their existing termination path; a missing matching database is unavailable.
pub async fn verify_session_database(
    client: &mut super::client::AgentClient,
    session: &str,
) -> Result<(), crate::Error> {
    let Some(thread) = crate::store::find_thread_by_agent_session(session)? else {
        return Ok(());
    };
    let Some((expected, _)) = crate::store::thread_source(&thread.id)? else {
        return Ok(());
    };
    let presence = session_presence_with_client(client, Vec::new()).await?;
    if expected != presence.database_id {
        return Err(crate::Error::AgentUnavailable("The conversation's Agent database is unavailable; current database belongs to a different source.".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_bridge::test_support::{mock_agent, seed_thread, seed_workspace, TestHome};
    use serde_json::json;

    #[tokio::test]
    async fn foreign_database_cannot_compact_or_rename_the_original_conversation() {
        let home = TestHome::new("mutation-source-fence");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "source");
        let thread = seed_thread(&workspace.id, Some("session"));
        crate::store::record_thread_source(
            &thread.id,
            "session",
            "original-database",
            "generation",
        )
        .unwrap();
        assert!(crate::agent_bridge::compact_agent_session("session".into())
            .await
            .is_err());
        assert!(
            crate::agent_bridge::rename_session("session".into(), "foreign title".into())
                .await
                .is_err()
        );
        assert!(!crate::agent_bridge::wait_for_agent_idle("session").await);
        assert!(crate::agent_bridge::get_session_entries("session".into())
            .await
            .is_err());
        assert!(
            crate::agent_bridge::get_session_entries_before("session".into(), i64::MAX, 10)
                .await
                .is_err()
        );
        assert!(crate::agent_bridge::get_session_messages("session".into())
            .await
            .is_err());
        for command in [
            "compact",
            "set_session_name",
            "get_state",
            "get_session_entries",
            "get_messages",
        ] {
            assert!(mock.requests_of(command).is_empty());
        }
        assert_eq!(
            crate::store::get_thread(&thread.id).unwrap().unwrap().title,
            thread.title
        );
    }

    #[tokio::test]
    async fn stopping_a_foreign_source_settles_only_the_local_run() {
        let home = TestHome::new("abort-source-fence");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "source");
        let thread = seed_thread(&workspace.id, Some("session"));
        crate::store::record_thread_source(
            &thread.id,
            "session",
            "original-database",
            "generation",
        )
        .unwrap();
        let run = crate::agent_bridge::test_support::seed_run(&thread.id);
        let stopped = crate::agent_bridge::abort_run(thread.id.clone(), run.id)
            .await
            .unwrap();
        assert_eq!(stopped.status, "cancelled");
        assert!(mock.requests_of("abort").is_empty());
        assert!(mock.requests_of("get_state").is_empty());
    }

    /// The fence keeps its teeth: an id the Agent database has NO row for is
    /// still refused, and refuses before any source binding is recorded.
    ///
    /// This is the negative half of the fresh-conversation fix. Widening
    /// presence to count the creation placeholder (`revision=-1`) must not turn
    /// into "any id is fine" — a conversation from another database would then
    /// be silently bound and its (empty) history would replace the real one.
    #[tokio::test]
    async fn a_session_with_no_row_is_still_refused_as_unavailable() {
        let home = TestHome::new("presence-missing");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "missing-source");
        let thread = seed_thread(&workspace.id, Some("session"));
        mock.push_data(
            "get_session_presence",
            json!({"databaseId":"some-database","restoreGeneration":"generation",
            "sessions":[{"sessionId":"session","status":"missing"}]}),
        );
        // The fence confirms a `missing` answer before refusing: presence no
        // longer queues behind the agent's ordered writer, so a session whose
        // placeholder commit is still queued reads `missing` for a moment. A
        // session that really has no row answers `missing` again.
        mock.push_data(
            "get_session_presence",
            json!({"databaseId":"some-database","restoreGeneration":"generation",
            "sessions":[{"sessionId":"session","status":"missing"}]}),
        );

        let error = verify_thread_source(&thread.id, "session")
            .await
            .expect_err("a session with no row must not pass the source fence");
        assert!(
            error
                .to_string()
                .contains("Conversation source is unavailable"),
            "unexpected error: {error}"
        );
        assert_eq!(
            crate::store::thread_source(&thread.id).unwrap(),
            None,
            "a refused session must not be bound to this database"
        );
    }
}
