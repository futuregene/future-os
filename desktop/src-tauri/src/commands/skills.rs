//! Tauri skill operations and GUI bootstrap adapter.
use crate::{agent_bridge, skills, skills_bootstrap};
#[tauri::command]
pub async fn refresh_skills() -> Result<(), crate::AppError> {
    future_host::application::skills::refresh_skills().await
}

#[tauri::command]
pub async fn list_installed_skills() -> Result<Vec<agent_bridge::InstalledSkill>, crate::AppError> {
    future_host::application::skills::list_installed_skills().await
}

#[tauri::command]
pub async fn list_available_skills() -> Result<Vec<agent_bridge::AvailableSkill>, crate::AppError> {
    future_host::application::skills::list_available_skills().await
}

#[tauri::command]
pub async fn suggest_skill(
    query: String,
    candidates: Vec<agent_bridge::SkillCandidate>,
) -> Result<Option<agent_bridge::SkillCandidate>, crate::AppError> {
    future_host::application::skills::suggest_skill(query, candidates).await
}

#[tauri::command]
pub async fn skill_reco_today() -> Result<crate::store::SkillRecoToday, crate::AppError> {
    future_host::application::skills::skill_reco_today().await
}

#[tauri::command]
pub async fn record_skill_reco(
    skill_id: String,
    message_hash: String,
) -> Result<(), crate::AppError> {
    future_host::application::skills::record_skill_reco(skill_id, message_hash).await
}

#[tauri::command]
pub async fn get_skill_guide() -> Result<skills::SkillGuide, crate::AppError> {
    future_host::application::skills::get_skill_guide().await
}

#[tauri::command]
pub async fn install_skill(id: String, version: String) -> Result<(), crate::AppError> {
    future_host::application::skills::install_skill(id, version).await
}

#[tauri::command]
pub async fn uninstall_skill(id: String) -> Result<bool, crate::AppError> {
    future_host::application::skills::uninstall_skill(id).await
}

#[tauri::command]
pub async fn sync_skills() -> Result<serde_json::Value, crate::AppError> {
    future_host::application::skills::sync_skills().await
}

/// Force-run the built-in skill bootstrap (installs platform built-in skills
/// via the bundled `future` CLI). Idempotent — the CLI skips already-installed
/// skills. Used by the post-login onboarding flow; runs on a background thread
/// since it blocks on the CLI child process.
/// Spawn the builtin skill bootstrap on a background thread. Extracted so the
/// thread body can run against a mock handle. The command wrapper is generic
/// over `Runtime` for the same reason.
fn spawn_builtin_skills<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    crate::runtime::spawn_native(move || skills_bootstrap::run_builtin_skills(&app));
}

#[tauri::command]
#[rustfmt::skip]
pub async fn bootstrap_builtin_skills<R: tauri::Runtime>(app: tauri::AppHandle<R>) { spawn_builtin_skills(app) }

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]
    use super::*;
    use crate::commands::agent_mock::{mock_agent_lock, script_mock_agent, MockScript};
    use std::collections::HashMap;

    /// The bootstrap command and its background spawner, against a mock handle.
    /// In a test home no bundled CLI resolves, so the body's documented
    /// "skipping built-in skill setup" path runs — which is exactly the arm that
    /// must not panic on a machine without a CLI. The command wrapper is called
    /// directly (it is generic over `Runtime`, so a mock handle instantiates it).
    ///
    /// Its `#[tauri::command]` attribute line (96) is **not** covered and is
    /// waived with the family in §4.1: the only argument is an `AppHandle<R>`,
    /// and `AppHandle::from_command` is infallible, so no body can reach that
    /// error region — measured here, not assumed: the IPC harness's
    /// `assert_all_reject_bad_body` fails its own assertion for this command
    /// ("should reject a malformed body") because the malformed body is accepted.
    #[tokio::test]
    async fn the_bootstrap_spawns_the_cli_runner_and_tolerates_a_missing_cli() {
        let _home = crate::auth_store::test_support::HomeGuard::new("cmd-skills-bootstrap");
        let _path = future_host::test_support::EnvGuard::set("PATH", std::ffi::OsStr::new(""));
        assert!(
            crate::cli_tool::resolve_for("skills.init").is_err(),
            "the fixture must have no runnable CLI"
        );
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_shell::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app with shell support");
        bootstrap_builtin_skills(app.handle().clone()).await;
        spawn_builtin_skills(app.handle().clone());
        // Keep the empty PATH until both native jobs have actually finished.
        crate::runtime::cancel_test_tasks();
    }

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
        // Two-argument commands: the empty body fails on the *first* argument,
        // whose error arm is attributed to the signature line. Supplying the
        // first argument makes the last one fail, which is the arm attributed
        // to the `#[tauri::command]` attribute line.
        crate::commands::ipc_harness::assert_all_reject_bodies(
            tauri::generate_handler![suggest_skill, record_skill_reco],
            &[
                (
                    "suggest_skill",
                    serde_json::json!({ "query": "write a test" }),
                ),
                ("record_skill_reco", serde_json::json!({ "skillId": "s" })),
            ],
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
