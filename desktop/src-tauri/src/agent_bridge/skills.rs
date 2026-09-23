//! Host-local skill management through the Agent's single SkillManager.

use serde::{Deserialize, Serialize};

use super::client::{base_command, connect_agent, RpcResponseExt};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledSkill {
    /// Equal to the install directory name and the catalogue id (a skill's
    /// SKILL.md `name` matches its id).
    pub id: String,
    pub name: String,
    pub description: String,
    pub name_zh: Option<String>,
    pub description_zh: Option<String>,
    pub version: Option<String>,
}

pub async fn list_installed_skills() -> Result<Vec<InstalledSkill>, crate::AppError> {
    serde_json::from_value(
        skill_command(base_command("list_installed_skills", String::new())).await?,
    )
    .map_err(|error| format!("Invalid installed skills response: {error}").into())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableSkill {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub name_zh: String,
    #[serde(default)]
    pub description_zh: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub category_zh: String,
    pub latest_version: Option<String>,
    pub builtin: bool,
    #[serde(default)]
    pub upgrade_available: bool,
}

async fn skill_command(
    command: crate::agent_proto::RpcCommand,
) -> Result<serde_json::Value, crate::AppError> {
    let mut client = connect_agent().await?;
    let response = client
        .execute_command(command)
        .await
        .map_err(|error| format!("Skill request failed: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the skill request.")?;
    Ok(future_rpc::decode::response_data(&response))
}

pub async fn list_available_skills() -> Result<Vec<AvailableSkill>, crate::AppError> {
    serde_json::from_value(
        skill_command(base_command("list_available_skills", String::new())).await?,
    )
    .map_err(|error| format!("Invalid available skills response: {error}").into())
}

pub async fn install_skill(id: String, version: String) -> Result<(), crate::AppError> {
    let mut command = base_command("install_skill", String::new());
    command.skill_id = id;
    command.skill_version = version;
    skill_command(command).await?;
    Ok(())
}

pub async fn uninstall_skill(id: String) -> Result<bool, crate::AppError> {
    let mut command = base_command("uninstall_skill", String::new());
    command.skill_id = id;
    let response = skill_command(command).await?;
    Ok(response
        .get("removed")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false))
}

pub async fn sync_skills(enabled: bool) -> Result<serde_json::Value, crate::AppError> {
    let mut command = base_command("sync_skills", String::new());
    command.enabled = enabled;
    skill_command(command).await
}

/// Tell the agent to drop its 60 s skills cache and re-scan so freshly
/// installed / uninstalled skills are visible on the next prompt without
/// waiting for the TTL to expire.  Best-effort — never fail the caller.
pub async fn refresh_skills() {
    if let Ok(mut client) = connect_agent().await {
        let _ = client
            .execute_command(base_command("refresh_skills", String::new()))
            .await;
    }
}

/// One skill the recommendation engine may offer (catalog − installed, computed
/// by the caller) and, on success, the single recommendation returned.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillCandidate {
    pub name: String,
    pub description: String,
}

/// Ask the agent's Jev recommender for at most one skill matching `query`.
/// Returns `None` on refusal, timeout, error, or when the feature is
/// unavailable server-side — recommendation is best-effort, so every failure
/// collapses to "no recommendation" and the caller submits normally.
pub async fn suggest_skill(
    query: &str,
    candidates: Vec<SkillCandidate>,
) -> Result<Option<SkillCandidate>, crate::AppError> {
    #[derive(Deserialize)]
    struct SuggestResponse {
        #[serde(default)]
        skill: Option<SkillCandidate>,
    }

    let mut command = base_command("suggest_skill", String::new());
    command.suggest_query = query.to_string();
    command.suggest_candidates = candidates
        .into_iter()
        .map(|c| crate::agent_proto::SkillCandidate {
            name: c.name,
            description: c.description,
        })
        .collect();

    let mut client = connect_agent().await?;
    let response = client
        .execute_command(command)
        .await
        .map_err(|error| format!("Unable to request a skill suggestion: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the skill suggestion.")?;

    let parsed =
        serde_json::from_value::<SuggestResponse>(future_rpc::decode::response_data(&response))
            .map_err(|error| format!("Future Agent returned an invalid suggestion: {error}"))?;
    Ok(parsed.skill)
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{mock_agent, Reply, TestHome};
    use super::*;

    #[tokio::test]
    async fn list_installed_skills_uses_agent_snapshot() {
        let _home = TestHome::new("skills-list");
        let mock = mock_agent();
        mock.push_data(
            "list_installed_skills",
            serde_json::json!([{
                "id":"my-skill", "name":"my-skill", "description":"does things",
                "nameZh":"我的技能", "descriptionZh":"做事", "version":"1.2.3"
            }]),
        );
        let skills = list_installed_skills().await.expect("skills");
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].version.as_deref(), Some("1.2.3"));
        assert_eq!(skills[0].name_zh.as_deref(), Some("我的技能"));
        assert_eq!(mock.requests_of("list_installed_skills").len(), 1);
    }

    #[tokio::test]
    async fn list_installed_skills_error_paths() {
        let _home = TestHome::new("skills-errors");
        let mock = mock_agent();

        mock.push(
            "list_installed_skills",
            Reply::Status(tonic::Code::Internal, "boom"),
        );
        let error = list_installed_skills().await.expect_err("transport");
        assert!(
            error.to_string().contains("Skill request failed"),
            "{error}"
        );

        mock.push("list_installed_skills", Reply::Reject(String::new()));
        let error = list_installed_skills().await.expect_err("rejected");
        assert_eq!(
            error.to_string(),
            "Future Agent rejected the skill request."
        );

        mock.push_data(
            "list_installed_skills",
            serde_json::json!({"commands": "nope"}),
        );
        let error = list_installed_skills().await.expect_err("invalid");
        assert!(
            error
                .to_string()
                .contains("Invalid installed skills response"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn refresh_skills_is_best_effort() {
        let mock = mock_agent();
        // Agent reachable: the command goes out.
        refresh_skills().await;
        assert_eq!(mock.requests_of("refresh_skills").len(), 1);

        // Agent unreachable (unparseable endpoint): still returns ().
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").ok();
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        refresh_skills().await;
        if let Some(prev) = prev {
            std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
        }

        // Command fails at transport level: still returns ().
        mock.push(
            "refresh_skills",
            Reply::Status(tonic::Code::Unavailable, "down"),
        );
        refresh_skills().await;
    }
}
