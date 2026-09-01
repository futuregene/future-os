//! FutureGene device-code login Tauri commands (see desktop/ER.md §6.9).

use crate::agent_providers::{self, ProvidersView};
use crate::agent_supervisor;
use crate::future_login::{self, FutureBalance, FutureLoginPoll, FutureLoginStart, FutureProfile};

#[tauri::command]
pub async fn start_future_login() -> Result<FutureLoginStart, crate::AppError> {
    future_login::start().await
}

#[tauri::command]
pub async fn poll_future_login<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    device_code: String,
) -> Result<FutureLoginPoll, crate::AppError> {
    let result = future_login::poll(&device_code).await?;
    // Make sure the agent is running once credentials land. On a fresh install
    // the sidecar came up model-less (agent/src/main.rs no longer exits when
    // nothing is configured) and stays up, so this is usually a cheap no-op probe.
    // But it also self-heals the case where the initial spawn failed — e.g. a
    // Windows portable build where Mark-of-the-Web blocked the child on first
    // launch: `ensure_agent_running` only runs once at startup (no watchdog), so
    // without this the agent would never come up until the app was restarted.
    // Safe to call unconditionally: if an agent is already reachable it attaches
    // instead of spawning a duplicate.
    if result.status == "authorized" {
        // Bring the app window to the front so the user sees the result.
        use tauri::Manager;
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
        let handle = app.clone();
        std::thread::spawn(move || agent_supervisor::ensure_agent_running(&handle));
        // Credential persistence + live-session refresh completed inside the
        // Agent before `future_login::poll` reported authorization.
    }
    Ok(result)
}

#[tauri::command]
pub async fn logout_future_provider() -> Result<ProvidersView, crate::AppError> {
    crate::agent_bridge::config::future_logout().await?;
    agent_providers::list_agent_providers().await
}

#[tauri::command]
pub async fn get_future_profile() -> Result<FutureProfile, crate::AppError> {
    future_login::fetch_profile().await
}

#[tauri::command]
pub async fn get_future_balance() -> Result<FutureBalance, crate::AppError> {
    crate::scheduler::refresh_future_balance_now().await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]
    use super::*;
    use crate::auth_store::test_support::HomeGuard;
    use crate::commands::agent_mock::{mock_agent_lock, script_mock_agent, MockScript};
    use std::collections::HashMap;

    /// A one-shot mock HTTP server: each `(status, content-type, body)` tuple
    /// answers one request.
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

    fn point_auth(url: &str) {
        crate::auth_store::set_future_base_url(&format!("{url}/api")).unwrap();
    }

    fn point_auth_with_key(url: &str) {
        crate::auth_store::set_future_login("sekret", &format!("{url}/api")).unwrap();
    }

    #[tokio::test]
    async fn start_future_login_returns_the_device_code() {
        let _home = HomeGuard::new("cmd-login-start");
        let url = mock_http_server(vec![(
            200,
            "application/json",
            b"{\"device_code\":\"dc-1\",\"user_code\":\"UC-1\",\"verification_uri_complete\":\"https://future-os.cn/oauth/device?user_code=UC-1\",\"expires_in\":1800,\"interval\":5}".to_vec(),
        )]);
        point_auth(&url);
        let start = start_future_login().await.expect("start");
        assert_eq!(start.user_code, "UC-1");
        assert_eq!(start.device_code, "dc-1");
    }

    #[tokio::test]
    async fn logout_future_provider_delegates_to_the_agent() {
        let _lock = mock_agent_lock();
        let _home = HomeGuard::new("cmd-login-logout");
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([("set_auth".to_string(), "{}".to_string())]),
            ..Default::default()
        });
        let view = logout_future_provider().await.expect("logout");
        assert!(!view.builtin.is_empty());
        script_mock_agent(MockScript::default());
    }

    #[tokio::test]
    async fn logout_future_provider_keeps_credentials_when_the_agent_is_down() {
        let _lock = mock_agent_lock();
        let _home = HomeGuard::new("cmd-login-logout-fb");
        crate::auth_store::set_future_login("sekret", "https://future-os.cn/api").unwrap();
        let error = crate::commands::agent_mock::with_broken_endpoint(logout_future_provider)
            .await
            .expect_err("logout must fail without the Agent");
        assert!(error.to_string().contains("not saved"));
        assert_eq!(crate::future_login::future_api_key().unwrap(), "sekret");
    }

    #[tokio::test]
    async fn get_future_profile_fetches_the_account() {
        let _home = HomeGuard::new("cmd-login-profile");
        let url = mock_http_server(vec![(
            200,
            "application/json",
            b"{\"email\":\"a@b.c\",\"user_id\":\"u1\"}".to_vec(),
        )]);
        point_auth_with_key(&url);
        let profile = get_future_profile().await.expect("profile");
        assert_eq!(profile.email, "a@b.c");
    }

    #[tokio::test]
    async fn get_future_balance_fetches_credits() {
        let _home = HomeGuard::new("cmd-login-balance");
        let url = mock_http_server(vec![(
            200,
            "application/json",
            b"{\"balance_credits\":10000000000}".to_vec(),
        )]);
        point_auth_with_key(&url);
        let balance = get_future_balance().await.expect("balance");
        assert_eq!(balance.credits, 1.0);
    }

    fn mock_app_with_main_window() -> tauri::App<tauri::test::MockRuntime> {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("main webview");
        app
    }

    #[test]
    fn poll_future_login_wrapper_rejects_malformed_bodies() {
        crate::commands::ipc_harness::assert_all_reject_bad_body(
            tauri::generate_handler![poll_future_login],
            &["poll_future_login"],
        );
    }

    #[tokio::test]
    async fn poll_future_login_reports_pending_without_authorization() {
        let _home = HomeGuard::new("cmd-login-poll-pending");
        let app = mock_app_with_main_window();
        // A non-2xx `authorization_pending` body exercises the poll call and
        // the `status != authorized` (no window/spawn) branch.
        let url = mock_http_server(vec![(
            400,
            "application/json",
            b"{\"error\":\"authorization_pending\"}".to_vec(),
        )]);
        point_auth(&url);

        let result = poll_future_login(app.handle().clone(), "dc-1".into())
            .await
            .expect("poll");
        assert_eq!(result.status, "pending");
    }

    #[tokio::test]
    async fn poll_future_login_refreshes_the_window_on_authorization() {
        let _lock = mock_agent_lock();
        let _home = HomeGuard::new("cmd-login-poll-auth");
        crate::commands::agent_mock::ensure_mock_agent();
        script_mock_agent(MockScript {
            data: HashMap::from([("set_auth".to_string(), "{}".to_string())]),
            ..Default::default()
        });

        let app = mock_app_with_main_window();
        let url = mock_http_server(vec![(
            200,
            "application/json",
            b"{\"api_key\":\"sk-test\",\"token_type\":\"api_key\"}".to_vec(),
        )]);
        point_auth(&url);

        let result = poll_future_login(app.handle().clone(), "dc-1".into())
            .await
            .expect("poll");
        assert_eq!(result.status, "authorized");

        // The detached ensure_agent_running thread probes the (reachable) mock
        // agent and returns — give it a beat to run so its line is attributed.
        std::thread::sleep(std::time::Duration::from_millis(100));
        script_mock_agent(MockScript::default());
    }
}
