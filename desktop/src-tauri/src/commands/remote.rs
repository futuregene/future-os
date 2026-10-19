//! Remote control Tauri commands (embedded Bridge start/stop/status). Delegates to `crate::remote`.

use crate::remote;
use serde::Serialize;

#[tauri::command]
pub async fn remote_start(
    input: remote::RemoteStartInput,
) -> Result<remote::RemoteStatus, crate::AppError> {
    // The switch starts what is paired and nothing else: turning it on is not
    // a request to pair a device, and a machine with no devices stays on and
    // empty until the user adds one. The input is part of the command's shape
    // (the frontend always sends it) and carries nothing the switch needs.
    let _ = input;
    match remote::enable().await {
        Ok(status) => {
            if let Some(reason) = &status.reason {
                eprintln!("remote: start completed in {:?} [{reason:?}]", status.phase);
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

/// Unpair one device and leave the others paired.
#[tauri::command]
pub async fn remote_unpair_device(
    pair_id: String,
) -> Result<remote::RemoteStatus, crate::AppError> {
    remote::unpair_slot(&pair_id).await
}

/// Disconnect one device but keep it paired.
///
/// Stops serving that device (and tells it it is offline) without touching its
/// credential, so "断开" and "删除" stay distinct actions in the client list: the
/// device can be reconnected by turning the remote desktop off and on again.
#[tauri::command]
pub async fn remote_disconnect_device(
    pair_id: String,
) -> Result<remote::RemoteStatus, crate::AppError> {
    remote::disconnect_device(&pair_id).await
}

/// Add a device beside the ones already paired and return the invitation to
/// display. The bridge for the new device is started immediately so the code
/// works the moment it is shown.
#[tauri::command]
pub async fn remote_add_device() -> Result<remote::RemoteStatus, crate::AppError> {
    remote::add_device().await
}

/// Make `pair_id` the device that drives this machine.
///
/// The desktop-side twin of the client's `takeover` command, for when the user
/// is sitting at the machine and would rather click than reach for the phone.
#[tauri::command]
pub async fn remote_takeover(pair_id: String) -> Result<remote::RemoteStatus, crate::AppError> {
    remote::takeover(&pair_id).await
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
    // The controlling device, or the first one when nothing has claimed
    // control: the same slot the single-device fields in `RemoteStatus`
    // project, so the two never disagree.
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
    open_url_with(&url, |u| {
        open::that_detached(u).map_err(|e| format!("Failed to open URL: {e}").into())
    })
}

/// Validation + opener with the OS layer injectable so the scheme guard is
/// testable without launching a browser. The scheme is restricted to
/// http/https/mailto — matching [`crate::commands::files::open_external_url`] —
/// so a crafted URL can't launch a local handler (`file:`, custom app schemes).
fn open_url_with(
    url: &str,
    opener: impl Fn(&str) -> Result<(), crate::AppError>,
) -> Result<(), crate::AppError> {
    let trimmed = url.trim();
    let normalized = trimmed.to_ascii_lowercase();
    if !(normalized.starts_with("http://")
        || normalized.starts_with("https://")
        || normalized.starts_with("mailto:"))
    {
        return Err("Only http(s) or mailto URLs can be opened."
            .to_string()
            .into());
    }
    opener(trimmed)
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
    fn open_url_with_rejects_non_http_schemes() {
        assert!(open_url_with("file:///etc/passwd", |_| unreachable!()).is_err());
        assert!(open_url_with("ftp://x", |_| unreachable!()).is_err());
        assert!(open_url_with("javascript:alert(1)", |_| unreachable!()).is_err());
        assert!(open_url_with("   ", |_| unreachable!()).is_err());
    }

    #[test]
    fn open_url_with_allows_http_and_mailto() {
        assert!(open_url_with("https://example.invalid/", |_| Ok(())).is_ok());
        assert!(open_url_with("mailto:x@example.com", |_| Ok(())).is_ok());
        assert!(open_url_with("https://x", |_| Err("os failed".to_string().into())).is_err());
    }

    #[test]
    fn open_url_rejects_non_http_schemes_before_reaching_the_os() {
        // The command wrapper validates the scheme before handing to the OS
        // opener, so an unsafe scheme fails without launching a browser.
        assert!(open_url("file:///etc/passwd".into()).is_err());
        assert!(open_url("ftp://x".into()).is_err());
        assert!(open_url("javascript:alert(1)".into()).is_err());
        assert!(open_url("   ".into()).is_err());
    }

    #[test]
    fn remote_status_and_stop_report_no_bridge_when_idle() {
        let _home = HomeGuard::new("remote_idle");
        let status = remote_status().expect("status");
        assert!(!matches!(status.phase, remote::RemotePhase::Ready));
        let stopped = tauri::async_runtime::block_on(remote_stop()).expect("stop");
        assert!(!matches!(stopped.phase, remote::RemotePhase::Ready));
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
        assert!(!matches!(status.phase, remote::RemotePhase::Ready));
    }

    /// Unpairing a device this machine does not have is not a fault: the wrapper
    /// answers with the stopped projection and leaves the book empty. Covers the
    /// `remote_unpair_device` command body, whose only production caller is the
    /// Remote page's per-row button.
    #[tokio::test]
    async fn remote_unpair_device_is_a_noop_for_a_device_it_does_not_have() {
        let _home = HomeGuard::new("remote_unpair_device");
        let status = remote_unpair_device("pair_absent".into())
            .await
            .expect("a device this machine never paired is not a fault");
        assert!(!matches!(status.phase, remote::RemotePhase::Ready));
        assert!(
            crate::remote::pairing::active_slot().is_none(),
            "nothing was in control before the call, so nothing may be named after it"
        );
        assert!(
            crate::remote::pairing::load_slots().is_empty(),
            "a call for an absent device must not mint one"
        );
    }

    /// Adding a device needs the account: the platform issues the code, so a
    /// signed-out desktop stops at the credential check and never reaches the
    /// network. Covers the `remote_add_device` command body.
    #[tokio::test]
    async fn remote_add_device_stops_at_the_account_check() {
        let _home = HomeGuard::new("remote_add_device");
        let error = remote_add_device()
            .await
            .expect_err("a signed-out desktop cannot mint an invitation");
        assert!(
            matches!(error, crate::AppError::NotSignedIn),
            "the missing account must surface as NotSignedIn, got {error:?}"
        );
        assert!(
            crate::remote::pairing::load_slots().is_empty(),
            "a refused invitation must not leave a device behind"
        );
    }

    #[tokio::test]
    async fn remote_start_surfaces_a_degraded_status() {
        let _home = HomeGuard::new("remote_start_deg");
        // A paired machine whose platform is unreachable: refreshing the
        // device's bridge credential fails with a categorized "network" status,
        // so `remote::enable` returns an Ok not-running status with the reason
        // set — the command's degraded-status eprintln arm and its Ok branch.
        crate::remote::test_support::sign_in("http://127.0.0.1:9");
        crate::remote::pairing::save_creds(&crate::remote::test_support::pairing_creds("pair_deg"))
            .expect("seed a paired device");
        let status = remote_start(remote::RemoteStartInput {})
            .await
            .expect("start");
        assert!(!matches!(status.phase, remote::RemotePhase::Ready));
        assert_eq!(status.reason, Some(remote::RemoteFailureReason::Network));
    }

    /// A pre-v2 pairing credential cannot be upgraded in place: `establish()`
    /// clears it (its PA003 branch) and the machine comes up enabled with no
    /// device. Minting the replacement is the add step's job, and the platform
    /// needs the account for that — so on a desktop with no stored key the
    /// invitation is refused as an *account* failure the user can act on, not as
    /// the generic local fault it used to be reported as.
    #[tokio::test]
    async fn remote_start_replaces_a_legacy_credential_and_reports_the_missing_account() {
        let _home = HomeGuard::new("remote_start_legacy");
        crate::remote::pairing::save_creds(&crate::remote::pairing::PairingCreds {
            handshake_version: 0,
            secure: None,
            pair_id: "pair_err".into(),
            desktop_id: "desk_err".into(),
            nkey_seed: String::new(),
            user_jwt: "jwt".into(),
            nats_url: "nats://127.0.0.1:9".into(),
            nats_ws_url: "ws://127.0.0.1:9".into(),
            jwt_expires_at: 3600,
            control_url: None,
        })
        .expect("write the legacy credential");
        let creds_path = std::path::Path::new(&std::env::var("HOME").unwrap())
            .join(".future")
            .join("remote_pairings.json");
        assert!(
            creds_path.is_file(),
            "the legacy credential must be on disk before the start"
        );

        // The switch starts what is paired and nothing else, so an unusable
        // credential leaves an enabled, empty machine — not a failed start. The
        // assertion that carries weight is the file: it must be gone, which is
        // only true if the credential was really cleared rather than retried.
        let status = remote_start(remote::RemoteStartInput {})
            .await
            .expect("start");
        assert_eq!(status.phase, remote::RemotePhase::Enabled);
        assert!(status.reason.is_none(), "{status:?}");
        assert!(
            status.devices.is_empty(),
            "the unusable credential must not be listed as a device"
        );
        assert!(
            !creds_path.exists(),
            "the unusable legacy credential must have been cleared, not left behind"
        );

        // The replacement invitation is a device the user adds, and minting one
        // needs the account — the account fault is reported where it happens.
        let error = remote_add_device()
            .await
            .expect_err("a missing account cannot mint the replacement");
        assert!(
            matches!(error, crate::AppError::NotSignedIn),
            "the missing account must surface as NotSignedIn, got {error:?}"
        );
    }

    /// Whatever this desktop cannot categorize is still a fault of its own: the
    /// command surfaces it as an error instead of dressing it up as a remote
    /// status. A pairing-code response the desktop cannot read is exactly that
    /// shape — the platform answered, so there is no network or account verdict
    /// to report, and only the raw cause is honest. Minting is what asks for a
    /// code, so the fault belongs to the add command.
    #[tokio::test]
    async fn remote_start_propagates_an_uncategorized_local_failure() {
        let _home = HomeGuard::new("remote_start_err");
        let platform = crate::remote::test_support::MockPlatform::start().await;
        platform.push("/client/v1/remote/pair/code", 200, serde_json::json!({}));
        crate::remote::test_support::sign_in(platform.url());

        let message = remote_add_device()
            .await
            .expect_err("an unreadable pairing response is a local fault")
            .to_string();
        assert!(
            message.contains("Failed to parse create pairing code response"),
            "the raw fault must be the one reported, got: {message}"
        );
    }
}
