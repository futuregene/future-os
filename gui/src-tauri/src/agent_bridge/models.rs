//! Model catalogue lookup: asks the agent for its available models.

use serde::{Deserialize, Serialize};

use super::client::{base_command, connect_agent, map_rpc_error, RpcResponseExt};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelOption {
    id: String,
    label: String,
    provider: String,
    #[serde(default)]
    supports_images: bool,
    #[serde(default)]
    thinking_level: Option<String>,
    #[serde(default)]
    context_window: Option<i32>,
    #[serde(default)]
    is_default: bool,
    /// Curated positioning blurb from the Future platform catalog (e.g.
    /// "经济实用版，日常编程和对话任务够用且实惠"). Absent for built-in / user models.
    #[serde(default)]
    description: Option<String>,
    /// English counterpart of `description` (e.g. "Budget-friendly edition, solid
    /// for daily coding and chat"); the GUI shows it when the UI language is not
    /// Chinese. Absent for built-in / user models.
    #[serde(default)]
    description_en: Option<String>,
    /// Future-platform recommendation flag (drives the onboarding model picker).
    #[serde(default)]
    recommended: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentModelsResponse {
    models: Vec<AgentModelOption>,
}

pub async fn list_agent_models() -> Result<Vec<AgentModelOption>, crate::AppError> {
    let mut client = connect_agent().await?;
    let response = client
        .execute_command(base_command("list_models", String::new()))
        .await
        .map_err(|status| map_rpc_error("Unable to load Future Agent models", status))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the model list request.")?;

    let parsed = serde_json::from_str::<AgentModelsResponse>(&response.data)
        .map_err(|error| format!("Future Agent returned invalid model data: {error}"))?;
    Ok(parsed.models)
}
