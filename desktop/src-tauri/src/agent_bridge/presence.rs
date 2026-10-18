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
    let presence = session_presence(vec![session_id.into()]).await?;
    if presence.sessions[0].status != "present" {
        return Err("Conversation source is unavailable; restore the matching Agent database before continuing.".into());
    }
    crate::store::record_thread_source(
        thread_id,
        session_id,
        &presence.database_id,
        &presence.restore_generation,
    )
}
