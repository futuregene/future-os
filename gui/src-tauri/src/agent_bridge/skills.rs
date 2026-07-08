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

    let parsed = serde_json::from_str::<CommandsResponse>(&response.data)
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
