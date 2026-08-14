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
    spawn_builtin_skills(app)
}

/// Spawn the builtin skill bootstrap on a background thread. Extracted so the
/// thread body can run against a mock handle (the command wrapper itself needs
/// a real Wry handle).
fn spawn_builtin_skills<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    std::thread::spawn(move || skills_bootstrap::run_builtin_skills(&app));
}

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
        script_mock_agent(MockScript::default());
    }
}
