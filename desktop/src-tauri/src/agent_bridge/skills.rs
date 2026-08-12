//! Installed-skill listing via the agent. The agent is the source of truth for
//! which skills are active (it discovers them across scopes and resolves
//! collisions), so the "installed" tab reads its `get_commands` rather than
//! scanning the filesystem directly. Versions are enriched locally since
//! `get_commands` only carries name + description.

use serde::{Deserialize, Serialize};

use super::client::{base_command, connect_agent, RpcResponseExt};

#[derive(Debug, Serialize)]
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
    #[derive(Deserialize)]
    struct CommandsResponse {
        #[serde(default)]
        commands: Vec<CommandEntry>,
    }

    #[derive(Deserialize)]
    struct CommandEntry {
        #[serde(default)]
        name: String,
        #[serde(default)]
        description: String,
        #[serde(default, alias = "nameZh")]
        name_zh: Option<String>,
        #[serde(default, alias = "descriptionZh")]
        description_zh: Option<String>,
        #[serde(default)]
        source: String,
    }

    let mut client = connect_agent().await?;
    let response = client
        .execute_command(base_command("get_commands", String::new()))
        .await
        .map_err(|error| format!("Unable to load installed skills: {error}"))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the skills request.")?;

    let parsed =
        serde_json::from_value::<CommandsResponse>(future_rpc::decode::response_data(&response))
            .map_err(|error| format!("Future Agent returned invalid skills data: {error}"))?;

    let versions = crate::skills::installed_versions();
    let skills = parsed
        .commands
        .into_iter()
        .filter(|command| command.source == "skill")
        .map(|command| {
            let version = versions.get(&command.name).cloned().flatten();
            InstalledSkill {
                id: command.name.clone(),
                name: command.name,
                description: command.description,
                name_zh: command.name_zh,
                description_zh: command.description_zh,
                version,
            }
        })
        .collect();
    Ok(skills)
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

#[cfg(test)]
mod tests {
    use super::super::test_support::{mock_agent, Reply, TestHome};
    use super::*;

    fn commands_payload() -> serde_json::Value {
        serde_json::json!({
            "commands": [
                {"name": "my-skill", "description": "does things", "source": "skill",
                 "nameZh": "我的技能", "descriptionZh": "做事"},
                {"name": "plain-skill", "description": "no zh", "source": "skill"},
                {"name": "not-a-skill", "description": "command", "source": "command"}
            ]
        })
    }

    #[tokio::test]
    async fn list_installed_skills_filters_and_enriches_versions() {
        let home = TestHome::new("skills-list");
        let mock = mock_agent();
        // Plant a versioned skill in the global scope ($HOME/.agents/skills).
        let skill_dir = home.path().join(".agents/skills/my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my-skill\nversion: 1.2.3\n---\n# my-skill\n",
        )
        .unwrap();

        mock.push_data("get_commands", commands_payload());
        let skills = list_installed_skills().await.expect("skills");
        assert_eq!(skills.len(), 2, "non-skill commands filtered out");
        let mine = skills.iter().find(|s| s.id == "my-skill").expect("my-skill");
        assert_eq!(mine.name_zh.as_deref(), Some("我的技能"));
        assert_eq!(mine.description_zh.as_deref(), Some("做事"));
        assert_eq!(mine.version.as_deref(), Some("1.2.3"), "version enriched");
        let plain = skills
            .iter()
            .find(|s| s.id == "plain-skill")
            .expect("plain-skill");
        assert_eq!(plain.version, None, "no local install → no version");
        assert_eq!(plain.name_zh, None);
    }

    #[tokio::test]
    async fn list_installed_skills_error_paths() {
        let _home = TestHome::new("skills-errors");
        let mock = mock_agent();

        mock.push(
            Some("get_commands"),
            Reply::Status(tonic::Code::Internal, "boom"),
        );
        let error = list_installed_skills().await.expect_err("transport");
        assert!(
            error.to_string().contains("Unable to load installed skills"),
            "{error}"
        );

        mock.push(Some("get_commands"), Reply::Reject(String::new()));
        let error = list_installed_skills().await.expect_err("rejected");
        assert_eq!(
            error.to_string(),
            "Future Agent rejected the skills request."
        );

        mock.push_data("get_commands", serde_json::json!({"commands": "nope"}));
        let error = list_installed_skills().await.expect_err("invalid");
        assert!(
            error.to_string().contains("invalid skills data"),
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
            Some("refresh_skills"),
            Reply::Status(tonic::Code::Unavailable, "down"),
        );
        refresh_skills().await;
    }
}
