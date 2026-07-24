//! Skill management Tauri commands: the installed list comes from the agent;
//! the catalogue and install/uninstall are handled locally (see
//! [`crate::skills`]).  After install/uninstall, the agent's skills cache is
//! invalidated via `refresh_skills` (fire-and-forget, best-effort).

use crate::{agent_bridge, skills};

#[tauri::command]
pub async fn list_installed_skills() -> Result<Vec<agent_bridge::InstalledSkill>, crate::AppError> {
    agent_bridge::list_installed_skills().await
}

#[tauri::command]
pub async fn list_available_skills() -> Result<Vec<skills::SkillInfo>, crate::AppError> {
    skills::list_available_skills().await
}

#[tauri::command]
pub async fn install_skill(id: String, version: String) -> Result<(), crate::AppError> {
    skills::install_skill(id, version).await?;
    // Notify the agent so the next prompt sees the new skill immediately.
    tokio::spawn(agent_bridge::refresh_skills());
    Ok(())
}

#[tauri::command]
pub async fn uninstall_skill(id: String) -> Result<bool, crate::AppError> {
    let removed = skills::uninstall_skill(&id)?;
    if removed {
        tokio::spawn(agent_bridge::refresh_skills());
    }
    Ok(removed)
}
