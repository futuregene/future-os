use super::{connect_agent, RpcResponseExt};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionPresence {
    pub database_id: String,
    pub restore_generation: String,
    pub sessions: Vec<PresenceItem>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PresenceItem {
    pub session_id: String,
    pub status: String,
}

pub(crate) async fn session_presence(ids: Vec<String>) -> Result<SessionPresence, crate::AppError> {
    let mut client = connect_agent().await?;
    session_presence_with_client(&mut client, ids).await
}

pub(crate) async fn session_presence_with_client(
    client: &mut super::client::AgentClient,
    ids: Vec<String>,
) -> Result<SessionPresence, crate::AppError> {
    let response = client
        .execute_command(crate::agent_proto::RpcCommand {
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

pub(crate) async fn verify_thread_source(
    thread_id: &str,
    session_id: &str,
) -> Result<(), crate::AppError> {
    let mut client = connect_agent().await?;
    verify_thread_source_with_client(&mut client, thread_id, session_id)
        .await
        .map(|_| ())
}

pub(crate) async fn verify_thread_source_with_client(
    client: &mut super::client::AgentClient,
    thread_id: &str,
    session_id: &str,
) -> Result<SessionPresence, crate::AppError> {
    let presence = session_presence_with_client(client, vec![session_id.into()]).await?;
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
pub(crate) async fn verify_session_database(
    client: &mut super::client::AgentClient,
    session: &str,
) -> Result<(), crate::AppError> {
    let Some(thread) = crate::store::find_thread_by_agent_session(session)? else {
        return Ok(());
    };
    let Some((expected, _)) = crate::store::thread_source(&thread.id)? else {
        return Ok(());
    };
    let presence = session_presence_with_client(client, Vec::new()).await?;
    if expected != presence.database_id {
        return Err(crate::AppError::AgentUnavailable("The conversation's Agent database is unavailable; current database belongs to a different source.".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "gui")]
    use super::*;
    #[cfg(feature = "gui")]
    use crate::agent_bridge::test_support::Reply;
    use crate::agent_bridge::test_support::{mock_agent, seed_thread, seed_workspace, TestHome};
    #[cfg(feature = "gui")]
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
        for command in ["compact", "set_session_name", "get_state"] {
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

    #[cfg(feature = "gui")]
    #[tokio::test]
    async fn foreign_session_metadata_does_not_overwrite_a_conversation_title() {
        let home = TestHome::new("metadata-source-fence");
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
        mock.push_data(
            "get_session_presence",
            json!({"databaseId":"foreign-database","restoreGeneration":"generation",
            "sessions":[{"sessionId":"session","status":"present"}]}),
        );
        let mut state = crate::agent_bridge::test_support::get_state_payload("session", false);
        state["sessionName"] = "foreign title".into();
        mock.push_typed_data("get_state", state);
        assert!(crate::commands::get_thread_agent_state(thread.id.clone())
            .await
            .is_err());
        assert_eq!(
            crate::store::get_thread(&thread.id).unwrap().unwrap().title,
            thread.title
        );
        assert!(mock.requests_of("get_state").is_empty());
    }

    #[cfg(feature = "gui")]
    #[tokio::test]
    async fn history_does_not_follow_a_reconnected_cache_to_another_database() {
        let home = TestHome::new("history-instance-fence");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "source");
        let thread = seed_thread(&workspace.id, Some("session"));
        let (entered, observed) = tokio::sync::oneshot::channel();
        let (answer, data) = tokio::sync::oneshot::channel();
        mock.push("get_session_presence", Reply::Deferred { entered, data });
        let thread_id = thread.id.clone();
        let read = tokio::spawn(async move {
            crate::commands::get_session_entries_page(thread_id, None, 10, None).await
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), observed)
            .await
            .unwrap()
            .unwrap();
        for _ in 0..3 {
            mock.push_data("get_agent_readiness",json!({
                "version":crate::build_info::VERSION,"agentInstanceId":"replacement-agent",
                "protocolVersion":future_rpc::connection::AGENT_PROTOCOL_VERSION,
                "dataRootId":"0".repeat(64),"storageSchemaVersion":future_rpc::connection::AGENT_STORAGE_SCHEMA_VERSION,
            }));
        }
        // Another caller detects the restart, then installs its new connection.
        assert!(connect_agent().await.is_err());
        let replacement = connect_agent().await.unwrap();
        assert_eq!(
            replacement.readiness().agent_instance_id,
            "replacement-agent"
        );
        mock.push("get_session_entries",Reply::BoundData {
            instance:"replacement-agent",
            data:json!({"entries":[{"id":"foreign","kind":"user","role":"user","createdAtMs":1,
                "blocks":[{"kind":"text","text":"foreign history"}]}],"hasMore":false,"nextOffset":0}),
        });
        answer
            .send(
                json!({"databaseId":"original-database","restoreGeneration":"original-generation",
            "sessions":[{"sessionId":"session","status":"present"}]}),
            )
            .unwrap();
        assert!(read.await.unwrap().is_err());
        assert_eq!(
            crate::store::thread_source(&thread.id).unwrap(),
            Some(("original-database".into(), "original-generation".into()))
        );
        assert!(crate::store::get_thread(&thread.id).unwrap().is_some());
    }
}
