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

/// Recommend at most one UNINSTALLED skill for the user's message via the
/// agent's Jev recommender. The caller decides when to invoke (toggle, length
/// bounds, login/balance, daily budget) and supplies the candidate set
/// (catalog − installed); this only forwards to the agent. Returns `None` on
/// refusal, timeout, error, or when the feature is unavailable server-side.
#[tauri::command]
pub async fn suggest_skill(
    query: String,
    candidates: Vec<agent_bridge::SkillCandidate>,
) -> Result<Option<agent_bridge::SkillCandidate>, crate::AppError> {
    agent_bridge::suggest_skill(&query, candidates).await
}

/// Today's recommendation state (count + already-shown skills and messages),
/// for the client's daily budget and duplicate checks.
#[tauri::command]
pub async fn skill_reco_today() -> Result<crate::store::SkillRecoToday, crate::AppError> {
    crate::store::skill_reco_today()
}

/// Record one recommendation that was actually shown to the user. Only shown
/// recommendations consume the daily budget; calls that recommend nothing must
/// not call this.
#[tauri::command]
pub async fn record_skill_reco(
    skill_id: String,
    message_hash: String,
) -> Result<(), crate::AppError> {
    crate::store::record_skill_reco(&skill_id, &message_hash)
}

/// The platform skill-guide config (coach prompt + manual link) for the
/// skill-onboarding banner. Unauthenticated platform call.
#[tauri::command]
pub async fn get_skill_guide() -> Result<skills::SkillGuide, crate::AppError> {
    skills::get_skill_guide().await
}

#[tauri::command]
pub async fn install_skill(id: String, version: String) -> Result<(), crate::AppError> {
    skills::install_and_refresh(id, version).await
}

#[tauri::command]
pub async fn uninstall_skill(id: String) -> Result<bool, crate::AppError> {
    skills::uninstall_and_refresh(id).await
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
    #![allow(clippy::await_holding_lock)]
    use super::*;
    use crate::commands::agent_mock::{mock_agent_lock, script_mock_agent, MockScript};
    use std::collections::HashMap;

    #[test]
    fn spawn_builtin_skills_runs_against_a_mock_handle() {
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_shell::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("build mock app");
        // The spawned thread fails the sidecar spawn and logs — no panic, no
        // drain, and the thread body runs against the mock handle.
        spawn_builtin_skills(app.handle().clone());
    }

    #[tokio::test]
    async fn bootstrap_builtin_skills_spawns_against_a_mock_handle() {
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_shell::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("build mock app");
        // The command wrapper is generic over Runtime so its body (which just
        // delegates to spawn_builtin_skills) can run against a mock handle.
        bootstrap_builtin_skills(app.handle().clone()).await;
    }

    #[test]
    fn async_command_wrappers_reject_malformed_bodies() {
        crate::commands::ipc_harness::assert_all_reject_bad_body(
            tauri::generate_handler![install_skill, uninstall_skill],
            &["install_skill", "uninstall_skill"],
        );
        // `install_skill` takes two arguments, so the empty-body rejection above
        // `install_skill` takes two arguments, so the empty-body rejection above
        // only exercises its *first* argument's error arm (attributed to the
        // signature line). Fail the *last* argument instead to hit the error arm
        // attributed to the `#[tauri::command]` attribute line.
        crate::commands::ipc_harness::assert_all_reject_bodies(
            tauri::generate_handler![install_skill],
            &[("install_skill", serde_json::json!({ "id": "x" }))],
        );
    }

    #[tokio::test]
    async fn list_installed_skills_parses_skill_sourced_commands() {
        let _lock = mock_agent_lock();
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([(
                "get_commands".to_string(),
                "{\"commands\":[{\"name\":\"foo\",\"description\":\"d\",\"source\":\"skill\"},{\"name\":\"bar\",\"description\":\"d\",\"source\":\"builtin\"}]}".to_string(),
            )]),
            ..Default::default()
        });
        let skills = list_installed_skills().await.expect("skills");
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].id, "foo");
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn refresh_skills_is_best_effort() {
        let _lock = mock_agent_lock();
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([("refresh_skills".to_string(), "{}".to_string())]),
            ..Default::default()
        });
        refresh_skills().await.expect("refresh");
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn uninstall_skill_rejects_invalid_ids_and_removes_installed() {
        // The uninstall path records a registry tombstone under the FutureOS
        // home — isolate it from the developer's real agent.db.
        let _home = crate::auth_store::test_support::HomeGuard::new("cmd-skills-uninstall-ghost");
        // Invalid id is rejected before touching the filesystem.
        assert!(uninstall_skill("../evil".into()).await.is_err());
        // A valid id with nothing installed reports "nothing removed".
        assert!(!uninstall_skill("ghost_skill".into())
            .await
            .expect("uninstall"));
    }

    #[tokio::test]
    async fn list_available_skills_lists_the_filesystem_catalog() {
        let _home = crate::auth_store::test_support::HomeGuard::new("cmd-skills-avail");
        // A clean home has no bundled skills yet — the wrapper still returns a
        // (possibly empty) catalog rather than failing.
        let _ = list_available_skills().await;
    }

    #[tokio::test]
    async fn get_skill_guide_fetches_the_platform_guide() {
        let _home = crate::auth_store::test_support::HomeGuard::new("cmd-skills-guide");
        // Point the platform at a mock serving an empty (all-default) guide,
        // so the unauthenticated fetch parses without a real network call.
        let url = mock_http_server(vec![(200, "application/json", b"{}".to_vec())]);
        crate::auth_store::set_future_base_url(&format!("{url}/api")).unwrap();
        let guide = get_skill_guide().await.expect("guide");
        assert!(guide.links.help.is_empty());
        assert!(guide.skills.coach_prompt.zh.is_empty());
    }

    #[tokio::test]
    async fn install_skill_rejects_a_bad_id_before_fs_work() {
        let _home = crate::auth_store::test_support::HomeGuard::new("cmd-skills-install");
        assert!(install_skill("../evil".into(), "1.0".into()).await.is_err());
    }

    /// A one-shot mock HTTP server: each `(status, content-type, body)` tuple
    /// answers one request. `Connection: close` so the client reads the body
    /// and moves on without keep-alive stalls.
    fn mock_http_server(responses: Vec<(u16, &'static str, Vec<u8>)>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for (status, content_type, body) in responses {
                let (mut stream, _) = listener.accept().expect("mock accept");
                let mut sink = [0u8; 8192];
                let _ = stream.read(&mut sink);
                let header = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    fn skill_zip() -> Vec<u8> {
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut cursor);
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("SKILL.md", options).unwrap();
            std::io::Write::write_all(&mut writer, b"# acme\n").unwrap();
            writer.finish().unwrap();
        }
        cursor.into_inner()
    }

    /// The (version, deleted) registry row for `id` in the isolated home's
    /// agent.db, or `None` when the skill has no row.
    fn registry_row(id: &str) -> Option<(Option<String>, bool)> {
        let connection =
            rusqlite::Connection::open(crate::auth_store::agent_dir().unwrap().join("agent.db"))
                .expect("open agent.db");
        connection
            .query_row(
                "SELECT version, deleted FROM skills WHERE name = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get::<_, i64>(1)? != 0)),
            )
            .ok()
    }

    #[tokio::test]
    async fn install_skill_success_refreshes_the_agent() {
        let _lock = mock_agent_lock();
        let _home = crate::auth_store::test_support::HomeGuard::new("cmd-skills-install-ok");
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([("refresh_skills".to_string(), "{}".to_string())]),
            ..Default::default()
        });

        // Point the platform at a mock that serves a valid skill zip, so the
        // download + extract path succeeds and the command reaches its
        // post-install agent refresh.
        let url = mock_http_server(vec![(200, "application/zip", skill_zip())]);
        crate::auth_store::set_future_base_url(&format!("{url}/api")).unwrap();

        install_skill("acme".into(), "1.0".into())
            .await
            .expect("install");
        assert_eq!(
            registry_row("acme"),
            Some((Some("1.0".to_string()), false)),
            "install recorded in the registry"
        );
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn uninstall_skill_removed_true_refreshes_the_agent() {
        let _lock = mock_agent_lock();
        let _home = crate::auth_store::test_support::HomeGuard::new("cmd-skills-uninstall-ok");
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([("refresh_skills".to_string(), "{}".to_string())]),
            ..Default::default()
        });

        // Lay down an installed skill dir manually (no download needed) so the
        // command's `if removed` branch fires and refreshes the agent.
        let dest = crate::auth_store::agent_dir().unwrap().join("skills/acme");
        std::fs::create_dir_all(&dest).unwrap();

        let removed = uninstall_skill("acme".into()).await.expect("uninstall");
        assert!(removed);
        assert_eq!(
            registry_row("acme"),
            Some((None, true)),
            "uninstall left a tombstone"
        );
        script_mock_agent(MockScript::default());
    }
}
