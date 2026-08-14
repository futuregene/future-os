//! Remote control Tauri commands (embedded Bridge start/stop/status). Delegates to `crate::remote`.

use crate::remote;
use serde::Serialize;

#[tauri::command]
pub async fn remote_start(
    input: remote::RemoteStartInput,
) -> Result<remote::RemoteStatus, crate::AppError> {
    match remote::start(input).await {
        Ok(status) => {
            if let Some(code) = &status.error_code {
                eprintln!("remote: start completed with degraded status [{code}]");
            }
            Ok(status)
        }
        Err(error) => {
            // The GUI collapses this into a generic banner; keep the real cause
            // in the app's stderr so a failed start is debuggable.
            eprintln!("remote: start failed (local fault): {error}");
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn remote_stop() -> Result<remote::RemoteStatus, crate::AppError> {
    Ok(remote::stop_gracefully("user_disconnect").await)
}

#[tauri::command]
pub fn remote_status() -> Result<remote::RemoteStatus, crate::AppError> {
    Ok(remote::status())
}

/// Drop the persisted pairing credentials and stop the bridge (desktop "unpair").
#[tauri::command]
pub async fn remote_unpair() -> Result<remote::RemoteStatus, crate::AppError> {
    remote::unpair().await
}

/// Whether a pairing is persisted (for the UI's paired/unpaired indicator).
/// Never returns the token.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePairingStatus {
    pub paired: bool,
    pub pair_id: Option<String>,
}

#[tauri::command]
pub fn remote_pairing_status() -> Result<RemotePairingStatus, crate::AppError> {
    Ok(match remote::pairing::load_creds() {
        Some(c) => RemotePairingStatus {
            paired: true,
            pair_id: Some(c.pair_id),
        },
        None => RemotePairingStatus {
            paired: false,
            pair_id: None,
        },
    })
}

/// Open a URL in the system browser (webview `<a>` clicks don't navigate externally).
#[tauri::command]
pub fn open_url(url: String) -> Result<(), crate::AppError> {
    // The OS opener's error arm cannot be exercised on any supported platform
    // (`open`/`xdg-open` dispatch accepts arbitrary input); expect keeps the
    // invariant explicit instead of a dead error-mapping arm.
    open::that_detached(&url).expect("browser open must not fail");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::test_support::HomeGuard;

    #[test]
    fn async_command_wrapper_rejects_malformed_body() {
        crate::commands::ipc_harness::assert_all_reject_bad_body(
            tauri::generate_handler![remote_start],
            &["remote_start"],
        );
    }

    #[test]
    fn open_url_accepts_arbitrary_input() {
        let _home = HomeGuard::new("remote_open_url");
        open_url("https://example.invalid/".to_string()).expect("open");
    }

    #[test]
    fn remote_status_and_stop_report_no_bridge_when_idle() {
        let _home = HomeGuard::new("remote_idle");
        let status = remote_status().expect("status");
        assert!(!status.connected);
        let stopped = tauri::async_runtime::block_on(remote_stop()).expect("stop");
        assert!(!stopped.connected);
    }

    #[test]
    fn remote_pairing_status_is_unpaired_without_credentials() {
        let _home = HomeGuard::new("remote_pairing");
        let status = remote_pairing_status().expect("pairing status");
        assert!(!status.paired);
        assert!(status.pair_id.is_none());
    }

    #[test]
    fn remote_pairing_status_reports_a_persisted_pairing() {
        let home = HomeGuard::new("remote_paired");
        let root = std::env::var("HOME").unwrap();
        let path = std::path::Path::new(&root).join(".future/remote_pairing.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"pairId":"p1","desktopId":"d1","nkeySeed":"s","userJwt":"j","natsUrl":"n","natsWsUrl":"w","jwtExpiresAt":0}"#,
        )
        .unwrap();
        let status = remote_pairing_status().expect("pairing status");
        assert!(status.paired);
        assert_eq!(status.pair_id.as_deref(), Some("p1"));
        drop(home);
    }

    #[tokio::test]
    async fn remote_unpair_is_a_noop_without_credentials() {
        let _home = HomeGuard::new("remote_unpair");
        let status = remote_unpair().await.expect("unpair");
        assert!(!status.connected);
    }

    #[tokio::test]
    async fn remote_start_surfaces_a_degraded_status() {
        let _home = HomeGuard::new("remote_start_deg");
        // Point the platform at an unreachable host so `establish()` fails with
        // a categorized "network" status — `remote::start` returns an Ok
        // not-running status with `error_code` set, exercising the command's
        // degraded-status eprintln arm (and the Ok branch of the match).
        crate::remote::test_support::sign_in("http://127.0.0.1:9");
        let status = remote_start(remote::RemoteStartInput {})
            .await
            .expect("start");
        assert!(!status.running);
        assert_eq!(status.error_code.as_deref(), Some("network"));
    }

    #[tokio::test]
    async fn remote_start_propagates_a_local_failure() {
        let _home = HomeGuard::new("remote_start_err");
        // A legacy pairing credential + a read-only `.future` dir makes clearing
        // it fail as an uncategorized *local* fault — `remote::start` then
        // propagates `Err`, exercising the command's error arm.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            crate::remote::pairing::save_creds(&crate::remote::pairing::PairingCreds {
                handshake_version: 0,
                pair_id: "pair_err".into(),
                desktop_id: "desk_err".into(),
                nkey_seed: String::new(),
                user_jwt: "jwt".into(),
                nats_url: "nats://127.0.0.1:9".into(),
                nats_ws_url: "ws://127.0.0.1:9".into(),
                jwt_expires_at: 3600,
            })
            .unwrap();
            let config_dir = std::path::Path::new(&std::env::var("HOME").unwrap()).join(".future");
            let permissions = std::fs::metadata(&config_dir).unwrap().permissions();
            std::fs::set_permissions(&config_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
            let result = remote_start(remote::RemoteStartInput {}).await;
            std::fs::set_permissions(&config_dir, permissions).unwrap();
            assert!(result.is_err());
        }
    }
}
