//! Tauri commands for the client role ("this desktop, connected out to other
//! desktops"). Delegates to `crate::remote_peer`.
//!
//! Kept separate from `commands::remote` (the host role) on purpose: the two
//! answer different questions — "who is connected to me" versus "who am I
//! connected to" — and a single namespace made it easy for the UI to read one
//! status where it meant the other.

use crate::remote_peer::{runtime, PeerEvent, PeerSummary};
use serde_json::Value;
use tauri::Emitter as _;

/// Every paired remote host, connected or not. Never carries credentials.
#[tauri::command]
pub async fn remote_peer_list() -> Result<Vec<PeerSummary>, crate::AppError> {
    runtime::list().await
}

/// The frontend listens for live pushes from a host under this event name.
pub(crate) const PEER_EVENT: &str = "remote-peer-event";

/// The emitter handed to the runtime: every decrypted push is forwarded to the
/// webview as-is. Serialising happens in the runtime's `PeerEvent`, so the UI
/// receives `{desktopId, kind, payload}` and nothing here reinterprets it.
///
/// Generic over the runtime so the command can be driven from a mock app in
/// tests; a `Wry`-only signature would make every command that takes an
/// `AppHandle` untestable (the reason this layer had no coverage at all).
fn emitter<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> runtime::Emitter {
    std::sync::Arc::new(move |event: PeerEvent| {
        if let Err(error) = app.emit(PEER_EVENT, event) {
            // A closed window is not a connection fault; the next status poll
            // still sees the host as connected.
            eprintln!("remote_peer: could not deliver a peer event: {error}");
        }
    })
}

/// Pair from a pasted `futureos://remote/pair` link and connect once.
#[tauri::command]
pub async fn remote_peer_pair<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    invitation: String,
) -> Result<PeerSummary, crate::AppError> {
    match runtime::pair_with_emitter(&invitation, Some(emitter(app))).await {
        Ok(peer) => Ok(peer),
        Err(error) => {
            // The UI shows a support code; the real cause goes to stderr where
            // a support session can read it.
            eprintln!("remote_peer: pairing failed: {error}");
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn remote_peer_connect<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    desktop_id: String,
) -> Result<PeerSummary, crate::AppError> {
    runtime::connect_with_emitter(&desktop_id, Some(emitter(app))).await
}

#[tauri::command]
pub async fn remote_peer_disconnect(desktop_id: String) -> Result<(), crate::AppError> {
    runtime::disconnect(&desktop_id).await;
    Ok(())
}

/// Forget a host. The returned string is a *warning*: the local pairing is gone,
/// but the platform-side revoke could not be delivered and should be retried.
#[tauri::command]
pub async fn remote_peer_unpair(desktop_id: String) -> Result<Option<String>, crate::AppError> {
    runtime::unpair(&desktop_id).await
}

/// Set the user's local name and/or icon for a host. `None` leaves a field
/// alone; an empty string clears it.
#[tauri::command]
pub async fn remote_peer_set_label(
    desktop_id: String,
    name: Option<String>,
    icon: Option<String>,
) -> Result<(), crate::AppError> {
    runtime::set_label(&desktop_id, name.as_deref(), icon.as_deref())
}

/// The host's session snapshot, stamped with `desktopId`.
#[tauri::command]
pub async fn remote_peer_sessions(desktop_id: String) -> Result<Value, crate::AppError> {
    runtime::sessions(&desktop_id).await
}

#[tauri::command]
pub async fn remote_peer_workspaces(desktop_id: String) -> Result<Value, crate::AppError> {
    runtime::workspaces(&desktop_id).await
}

/// Run one command against one host.
///
/// `lane` is the routing fallback (`list` for catalogue reads, the session id
/// otherwise). The Frontend does not build subjects — that stays here, where
/// the host's routing contract lives.
#[tauri::command]
pub async fn remote_peer_request(
    desktop_id: String,
    command: Value,
    lane: Option<String>,
) -> Result<Value, crate::AppError> {
    runtime::request(&desktop_id, command, lane.as_deref().unwrap_or("list")).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::test_support::HomeGuard;
    use crate::remote_peer::testing::{fixture, teardown, Fixture};

    /// Every command shares one setup: a clean runtime, a real host bridge, and
    /// a claimed pairing.
    async fn start(label: &str) -> (HomeGuard, Fixture) {
        runtime::reset_for_test().await;
        fixture(label).await
    }

    /// The command layer's own contract: each wrapper is reachable through the
    /// real IPC deserialization path and rejects a body missing its arguments.
    ///
    /// The async wrapper's argument-deserialization error arm is attributed to
    /// the `#[tauri::command]` attribute line, so without this the line reads as
    /// uncovered no matter how well the body is tested.
    #[test]
    fn command_wrappers_reject_malformed_bodies() {
        // The async wrapper's argument-deserialization error arm is attributed to
        // the `#[tauri::command]` attribute line, so without this the line reads
        // as uncovered no matter how well the body is tested.
        //
        // Where a command takes more than one argument, the body satisfies every
        // argument except the *last*: failing on the first one would put the
        // error region on the function signature instead.
        crate::commands::ipc_harness::assert_all_reject_bodies(
            tauri::generate_handler![
                remote_peer_pair,
                remote_peer_connect,
                remote_peer_disconnect,
                remote_peer_unpair,
                remote_peer_set_label,
                remote_peer_sessions,
                remote_peer_workspaces,
                remote_peer_request,
            ],
            &[
                ("remote_peer_pair", serde_json::json!({})),
                ("remote_peer_connect", serde_json::json!({})),
                ("remote_peer_disconnect", serde_json::json!({})),
                ("remote_peer_unpair", serde_json::json!({})),
                (
                    "remote_peer_set_label",
                    serde_json::json!({ "desktopId": "d", "name": "n", "icon": 1 }),
                ),
                ("remote_peer_sessions", serde_json::json!({})),
                ("remote_peer_workspaces", serde_json::json!({})),
                (
                    "remote_peer_request",
                    serde_json::json!({ "desktopId": "d", "command": {}, "lane": 1 }),
                ),
            ],
        );
    }

    #[tokio::test]
    async fn list_reports_every_paired_host() {
        let (_home, fx) = start("peer-cmd-list").await;
        let peers = remote_peer_list().await.expect("list");
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].desktop_id, fx.paired.creds.desktop_id);
        teardown().await;
    }

    /// Pairing through the command is the path the UI takes. It is also the one
    /// place a failure is logged and then returned unchanged, so the test checks
    /// both halves: the error reaches the caller, and a valid link connects.
    #[tokio::test]
    async fn pairing_through_the_command_connects_and_reports_failures() {
        let (_home, fx) = start("peer-cmd-pair").await;
        let app = mock_app();

        let error = remote_peer_pair(app.clone(), "not a url".into())
            .await
            .expect_err("a bad link cannot pair");
        assert!(
            error.to_string().contains("pairing_link_malformed"),
            "{error}"
        );

        // A fresh invitation the mock platform will accept, then the real call.
        let claim_url = format!("{}/client/v1/remote/pair/claim", fx.platform.url());
        let invitation =
            crate::remote_peer::testing::invitation_for(&fx.host_invitation, &claim_url);
        fx.platform.push(
            "/client/v1/remote/pair/claim",
            200,
            serde_json::json!({
                "pair_id": fx.pair_id,
                "user_jwt": crate::remote::test_support::jwt(
                    crate::remote::test_support::now_secs() + 3_600
                ),
                "refresh_token": "refresh-token-1",
                "nats_url": fx.nats.url(),
                "nats_ws_url": fx.nats.url().replace("nats://", "ws://"),
            }),
        );
        let peer = remote_peer_pair(app, invitation)
            .await
            .expect("a valid link pairs");
        assert!(peer.connected, "{peer:?}");

        teardown().await;
    }

    #[tokio::test]
    async fn connect_disconnect_and_unpair_round_trip() {
        let (_home, fx) = start("peer-cmd-lifecycle").await;
        let app = mock_app();
        let desktop_id = fx.paired.creds.desktop_id.clone();

        let peer = remote_peer_connect(app, desktop_id.clone())
            .await
            .expect("connect");
        assert!(peer.connected, "{peer:?}");

        remote_peer_disconnect(desktop_id.clone())
            .await
            .expect("disconnect");
        assert!(!remote_peer_list().await.expect("list")[0].connected);

        // A revoke the platform refuses is reported as a warning, not an error:
        // the local pairing is already gone.
        fx.platform
            .push("/client/v1/remote/pair/revoke", 500, serde_json::json!({}));
        let warning = remote_peer_unpair(desktop_id).await.expect("unpair");
        assert!(warning.is_some());
        assert!(remote_peer_list().await.expect("list").is_empty());

        teardown().await;
    }

    #[tokio::test]
    async fn labels_and_catalogues_flow_through_the_commands() {
        let (_home, fx) = start("peer-cmd-catalog").await;
        let desktop_id = fx.paired.creds.desktop_id.clone();
        crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".into(),
            title: Some("Through the command".into()),
            workspace_id: None,
            workspace_path: Some("/tmp/peer-cmd".into()),
            workspace_name: Some("peer-cmd".into()),
            agent_session_id: Some("sess_cmd".into()),
        })
        .expect("thread");

        remote_peer_set_label(
            desktop_id.clone(),
            Some("Studio".into()),
            Some("rocket".into()),
        )
        .await
        .expect("label");
        let listed = remote_peer_list().await.expect("list");
        assert_eq!(listed[0].name.as_deref(), Some("Studio"));
        assert_eq!(listed[0].icon.as_deref(), Some("rocket"));

        let sessions = remote_peer_sessions(desktop_id.clone())
            .await
            .expect("sessions");
        assert_eq!(sessions["desktopId"], serde_json::json!(desktop_id));
        assert!(sessions["sessions"]
            .as_array()
            .expect("rows")
            .iter()
            .any(|row| row["sessionId"] == "sess_cmd"));

        let workspaces = remote_peer_workspaces(desktop_id.clone())
            .await
            .expect("workspaces");
        assert!(workspaces["workspaces"].is_array());

        // `remote_peer_request` is the general escape hatch; the lane defaults
        // to a catalogue read when the caller does not name one.
        let requested = remote_peer_request(
            desktop_id.clone(),
            serde_json::json!({ "type": "list_workspaces" }),
            None,
        )
        .await
        .expect("request");
        assert!(requested["workspaces"].is_array());

        let routed = remote_peer_request(
            desktop_id,
            serde_json::json!({ "type": "list_sessions" }),
            Some("list".into()),
        )
        .await
        .expect("request with a lane");
        assert!(routed["sessions"].is_array());

        teardown().await;
    }

    /// A command against an unpaired host fails with a code the UI can act on
    /// rather than a generic message.
    #[tokio::test]
    async fn commands_reject_an_unpaired_host() {
        let (_home, _fx) = start("peer-cmd-unpaired").await;
        let error = remote_peer_sessions("desktop_nobody".into())
            .await
            .expect_err("an unpaired host has no catalogue");
        assert!(error.to_string().contains("peer_not_paired"), "{error}");
        teardown().await;
    }

    /// A mock app whose handle the emitter can be built from. The window is
    /// never shown; only the emit path is exercised.
    fn mock_app() -> tauri::AppHandle<tauri::test::MockRuntime> {
        tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("build mock app")
            .handle()
            .clone()
    }
}
