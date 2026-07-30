//! Skill management Tauri commands: the installed list comes from the agent;
//! the catalogue and install/uninstall are handled locally (see
//! [`crate::skills`]).  After install/uninstall, the agent's skills cache is
//! invalidated via `refresh_skills` — awaited (best-effort, bounded by the
//! agent connect timeout) so the notification is guaranteed to be sent
//! before this command returns and no follow-up prompt can race the stale
//! cache.

use crate::{agent_bridge, skills, skills_bootstrap};

/// Manually tell the agent to drop its skills cache and re-discover.
/// Use when entering the Skills page or on app startup so the
/// displayed list always reflects the current filesystem state.
#[tauri::command]
pub async fn refresh_skills() -> Result<(), crate::AppError> {
    agent_bridge::refresh_skills().await;
    Ok(())
}

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
    // Awaited (never fails) so the refresh is in flight before we return.
    agent_bridge::refresh_skills().await;
    Ok(())
}

#[tauri::command]
pub async fn uninstall_skill(id: String) -> Result<bool, crate::AppError> {
    let removed = skills::uninstall_skill(&id)?;
    if removed {
        agent_bridge::refresh_skills().await;
    }
    Ok(removed)
}

/// Force-run the built-in skill bootstrap (installs platform built-in skills
/// via the bundled `future` CLI). Idempotent — the CLI skips already-installed
/// skills. Used by the post-login onboarding flow; runs on a background thread
/// since it blocks on the CLI child process.
#[tauri::command]
pub async fn bootstrap_builtin_skills(app: tauri::AppHandle) {
    std::thread::spawn(move || skills_bootstrap::run_builtin_skills(&app));
}
