//! FutureGene device-code login Tauri commands (see desktop/ER.md §6.9).

use crate::agent_providers::{self, ProvidersView};
use crate::agent_supervisor;
use crate::future_login::{
    self, FutureAuthState, FutureBalance, FutureLoginPoll, FutureLoginStart, FutureProfile,
};

#[tauri::command]
pub async fn start_future_login<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<FutureLoginStart, crate::AppError> {
    if !agent_supervisor::ensure_agent_ready_for_login(app).await {
        return Err(crate::AppError::Message(
            "Future Agent could not be started; authorization has not begun. Please retry."
                .to_string(),
        ));
    }
    future_login::start().await
}

#[tauri::command]
pub async fn poll_future_login<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    device_code: String,
) -> Result<FutureLoginPoll, crate::AppError> {
    // Do this again for every poll: the user may spend minutes in the browser
    // after `start_future_login`, and the Agent can exit during that gap. Do
    // not ask the server for a potentially one-time credential until its sole
    // durable writer is ready. A startup failure remains a transient poll so
    // the UI keeps the same device code and retries.
    if !agent_supervisor::ensure_agent_ready_for_login(app.clone()).await {
        return Ok(FutureLoginPoll {
            status: "retry".to_string(),
            message: None,
            retry_after_seconds: None,
        });
    }
    let result = future_login::poll(&device_code).await?;
    if result.status == "authorized" {
        // Bring the app window to the front so the user sees the result.
        use tauri::Manager;
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
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
pub async fn get_future_auth_state() -> FutureAuthState {
    future_login::check_auth_state().await
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
        let _lock = mock_agent_lock();
        let _home = HomeGuard::new("cmd-login-start");
        crate::commands::agent_mock::ensure_mock_agent();
        let url = mock_http_server(vec![(
            200,
            "application/json",
            b"{\"device_code\":\"dc-1\",\"user_code\":\"UC-1\",\"verification_uri_complete\":\"https://future-os.cn/oauth/device?user_code=UC-1\",\"expires_in\":1800,\"interval\":5}".to_vec(),
        )]);
        point_auth(&url);
        let app = mock_app_with_main_window();
        let start = start_future_login(app.handle().clone())
            .await
            .expect("start");
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

    /// `get_future_auth_state` is a three-way classification, not a boolean: a
    /// home with no credential is `signed_out`, a stored key whose platform
    /// cannot be reached is `unavailable` (not `invalid` — the key itself was
    /// never judged), and a key that fetches a profile is `authenticated`.
    #[tokio::test]
    async fn auth_state_distinguishes_signed_out_unavailable_and_authenticated() {
        let _home = HomeGuard::new("cmd-login-state");

        let before = get_future_auth_state().await;
        assert!(
            matches!(before.status, future_login::FutureAuthStatus::SignedOut),
            "a fresh home has no credential: {before:?}"
        );
        assert!(before.profile.is_none());

        // A stored key, but a platform that refuses the connection: the key was
        // never judged, so this is `unavailable` rather than `invalid`.
        point_auth_with_key("http://127.0.0.1:9");
        let unreachable = get_future_auth_state().await;
        assert!(
            matches!(
                unreachable.status,
                future_login::FutureAuthStatus::Unavailable
            ),
            "an unreachable platform is not an invalid credential: {unreachable:?}"
        );
        assert!(unreachable.profile.is_none());

        let url = mock_http_server(vec![(
            200,
            "application/json",
            b"{\"email\":\"a@b.c\",\"user_id\":\"u1\"}".to_vec(),
        )]);
        point_auth_with_key(&url);
        let authenticated = get_future_auth_state().await;
        assert!(
            matches!(
                authenticated.status,
                future_login::FutureAuthStatus::Authenticated
            ),
            "a key that fetches a profile is authenticated: {authenticated:?}"
        );
        assert_eq!(
            authenticated.profile.map(|profile| profile.email),
            Some("a@b.c".to_string())
        );
    }

    /// Both login commands gate on the agent being ready, and both must say so
    /// rather than half-start a flow: `start` refuses and `poll` answers
    /// `retry` *with the same device code*, so the UI can keep waiting instead
    /// of restarting the flow.
    #[tokio::test]
    async fn login_commands_refuse_to_run_without_a_ready_agent() {
        let _lock = mock_agent_lock();
        let _home = HomeGuard::new("cmd-login-not-ready");
        let app = mock_app_with_main_window();
        let handle = app.handle().clone();

        let started = crate::commands::agent_mock::with_broken_endpoint(|| {
            start_future_login(handle.clone())
        })
        .await;
        let error = started.expect_err("login must not begin without the agent");
        assert!(
            error.to_string().contains("authorization has not begun"),
            "the refusal must tell the user to retry: {error}"
        );

        let polled = crate::commands::agent_mock::with_broken_endpoint(|| {
            poll_future_login(handle, "dc-1".into())
        })
        .await
        .expect("a not-yet-ready agent is a transient poll, not an error");
        assert_eq!(polled.status, "retry");
        assert!(polled.message.is_none());
        assert!(polled.retry_after_seconds.is_none());
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
        let _lock = mock_agent_lock();
        let _home = HomeGuard::new("cmd-login-poll-pending");
        crate::commands::agent_mock::ensure_mock_agent();
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
        script_mock_agent(MockScript::default());
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

        script_mock_agent(MockScript::default());
    }
}
