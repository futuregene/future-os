//! Sessionless skill management. All clients use the host-local manager.

use super::{AppState, RpcCommand, RpcResponse};
use crate::skills::manager::SkillManager;

pub(super) fn handle(state: &AppState, cmd: &RpcCommand) -> String {
    let result = (|| -> anyhow::Result<serde_json::Value> {
        let manager = SkillManager::local()?;
        match cmd.cmd_type.as_str() {
            "list_installed_skills" => Ok(serde_json::to_value(manager.list_installed()?)?),
            "list_available_skills" => Ok(serde_json::to_value(manager.catalogue_with_status()?)?),
            "install_skill" => {
                manager.install(&cmd.skill_id, &cmd.skill_version)?;
                let _ = super::providers::cmd_refresh_skills(state, &cmd.id);
                Ok(serde_json::json!({}))
            }
            "uninstall_skill" => {
                let removed = manager.uninstall(&cmd.skill_id)?;
                let _ = super::providers::cmd_refresh_skills(state, &cmd.id);
                Ok(serde_json::json!({"removed": removed}))
            }
            "sync_skills" => {
                let result = manager.sync(cmd.enabled)?;
                let _ = super::providers::cmd_refresh_skills(state, &cmd.id);
                Ok(serde_json::to_value(result)?)
            }
            _ => unreachable!(),
        }
    })();
    match result {
        Ok(value) => RpcResponse::ok(&cmd.id, &cmd.cmd_type, value),
        Err(error) => RpcResponse::build_fail(&cmd.id, &cmd.cmd_type, &format!("{error:#}")),
    }
}
