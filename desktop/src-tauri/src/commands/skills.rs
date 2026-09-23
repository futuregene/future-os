//! Thin Tauri surface for the Agent's host-local SkillManager.

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
pub async fn list_available_skills() -> Result<Vec<agent_bridge::AvailableSkill>, crate::AppError> {
    agent_bridge::list_available_skills().await
}

/// Recommend at most one UNINSTALLED skill for the user's first-turn text via
/// the agent's Jev recommender. The caller decides when to invoke (new session,
/// first message, length cap, login/balance) and supplies the candidate set
/// (catalog − installed); this only forwards to the agent. Returns `None` on
/// refusal, timeout, error, or when the feature is unavailable server-side.
#[tauri::command]
pub async fn suggest_skill(
    query: String,
    candidates: Vec<agent_bridge::SkillCandidate>,
) -> Result<Option<agent_bridge::SkillCandidate>, crate::AppError> {
    agent_bridge::suggest_skill(&query, candidates).await
}

/// The platform skill-guide config (coach prompt + manual link) for the
/// skill-onboarding banner. Unauthenticated platform call.
#[tauri::command]
pub async fn get_skill_guide() -> Result<skills::SkillGuide, crate::AppError> {
    skills::get_skill_guide().await
}

#[tauri::command]
pub async fn install_skill(id: String, version: String) -> Result<(), crate::AppError> {
    agent_bridge::install_skill(id, version).await?;
    crate::agent_events::publish_invalidation("skills_changed");
    Ok(())
}

#[tauri::command]
pub async fn uninstall_skill(id: String) -> Result<bool, crate::AppError> {
    let removed = agent_bridge::uninstall_skill(id).await?;
    if removed {
        crate::agent_events::publish_invalidation("skills_changed");
    }
    Ok(removed)
}

#[tauri::command]
pub async fn sync_skills() -> Result<serde_json::Value, crate::AppError> {
    let result = agent_bridge::sync_skills(true).await?;
    crate::agent_events::publish_invalidation("skills_changed");
    Ok(result)
}

/// Force-run the built-in skill bootstrap (installs platform built-in skills
/// via the bundled `future` CLI). Idempotent — the CLI skips already-installed
/// skills. Used by the post-login onboarding flow; runs on a background thread
/// since it blocks on the CLI child process.
/// Spawn the builtin skill bootstrap on a background thread. Extracted so the
/// thread body can run against a mock handle. The command wrapper is generic
/// over `Runtime` for the same reason.
fn spawn_builtin_skills<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    std::thread::spawn(move || skills_bootstrap::run_builtin_skills(&app));
}

#[tauri::command]
#[rustfmt::skip]
pub async fn bootstrap_builtin_skills<R: tauri::Runtime>(app: tauri::AppHandle<R>) { spawn_builtin_skills(app) }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent_mock::{mock_agent_lock, script_mock_agent, MockScript};
    use std::collections::HashMap;

    #[test]
    fn command_wrappers_reject_malformed_bodies() {
        crate::commands::ipc_harness::assert_all_reject_bad_body(
            tauri::generate_handler![install_skill, uninstall_skill],
            &["install_skill", "uninstall_skill"],
        );
        crate::commands::ipc_harness::assert_all_reject_bodies(
            tauri::generate_handler![install_skill],
            &[("install_skill", serde_json::json!({ "id": "x" }))],
        );
    }

    #[tokio::test]
    async fn skills_commands_forward_to_agent() {
        let _lock = mock_agent_lock();
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([
                ("list_installed_skills".into(), "[]".into()),
                ("list_available_skills".into(), "[]".into()),
                ("install_skill".into(), "{}".into()),
                ("uninstall_skill".into(), "{\"removed\":true}".into()),
                (
                    "sync_skills".into(),
                    "{\"installed\":[],\"upgraded\":[],\"skipped\":[],\"failed\":[]}".into(),
                ),
            ]),
            ..Default::default()
        });
        assert!(list_installed_skills().await.unwrap().is_empty());
        assert!(list_available_skills().await.unwrap().is_empty());
        install_skill("acme".into(), "1.0.0".into()).await.unwrap();
        assert!(uninstall_skill("acme".into()).await.unwrap());
        assert!(sync_skills().await.is_ok());
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn skill_guide_still_uses_the_platform_endpoint() {
        let _home = crate::auth_store::test_support::HomeGuard::new("cmd-skills-guide");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let count = stream.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..count]).contains("/client/v1/guide"));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
                .unwrap();
        });
        crate::auth_store::set_future_base_url(&format!("{url}/api")).unwrap();
        assert!(get_skill_guide().await.unwrap().links.help.is_empty());
    }
}
