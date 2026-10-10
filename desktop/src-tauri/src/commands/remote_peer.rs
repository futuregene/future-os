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
        report_emit_failure(app.emit(PEER_EVENT, event));
    })
}

/// Report a push that could not be delivered.
///
/// Extracted so the "the window is gone" arm is reachable at all: a mock app's
/// `emit` returns `Ok` even with no webview, and a real window cannot be made to
/// fail on demand, so a branch left inline here would be an arm no test could
/// ever execute.
///
/// It is deliberately not a fault: a closed window is not a connection fault,
/// and the next status poll still sees the host as connected.
fn report_emit_failure<E: std::fmt::Display>(result: Result<(), E>) {
    if let Err(error) = result {
        eprintln!("remote_peer: could not deliver a peer event: {error}");
    }
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

/// List one directory inside a session on a host.
///
/// Its own command rather than a `remote_peer_request` call because the answer
/// is a typed listing the UI navigates, and because the session root has to be
/// addressable — an empty path means "the session's own directory".
#[tauri::command]
pub async fn remote_peer_list_files(
    desktop_id: String,
    session_id: String,
    path: Option<String>,
) -> Result<Value, crate::AppError> {
    runtime::request(
        &desktop_id,
        serde_json::json!({
            "type": "list_session_files",
            "sessionId": session_id,
            "filePath": path.unwrap_or_default(),
        }),
        &session_id,
    )
    .await
}

/// Pull a file from a host and write it to a local path.
///
/// The destination is chosen by the caller (a native save dialog on the
/// desktop), so this command never guesses where a file should land. It returns
/// the name the host gave the file, which is what the UI reports: the host may
/// rename a preview variant, and telling the user a name the file does not have
/// would make the saved copy unfindable.
#[tauri::command]
pub async fn remote_peer_download_file(
    desktop_id: String,
    session_id: String,
    path: String,
    name: Option<String>,
    variant: Option<String>,
    destination: String,
) -> Result<String, crate::AppError> {
    let fetched = crate::remote_peer::transfer::download(
        &desktop_id,
        &session_id,
        &path,
        name.as_deref(),
        variant.as_deref().unwrap_or("original"),
    )
    .await?;
    crate::remote_peer::transfer::write_atomically(
        std::path::Path::new(&destination),
        &fetched.bytes,
    )?;
    Ok(fetched.name)
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
                remote_peer_list_files,
                remote_peer_download_file,
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
                    "remote_peer_list_files",
                    serde_json::json!({ "desktopId": "d", "sessionId": "s", "path": 1 }),
                ),
                (
                    "remote_peer_download_file",
                    serde_json::json!({
                        "desktopId": "d",
                        "sessionId": "s",
                        "path": "p",
                        "name": "n",
                        "variant": "original",
                        "destination": 1,
                    }),
                ),
                (
                    "remote_peer_request",
                    serde_json::json!({ "desktopId": "d", "command": {}, "lane": 1 }),
                ),
            ],
        );
    }

    /// A push that cannot be delivered is logged, not propagated: the window may
    /// simply be closed, which says nothing about the connection.
    #[test]
    fn an_undeliverable_push_is_reported_not_raised() {
        report_emit_failure::<&str>(Ok(()));
        report_emit_failure(Err("no window to receive it"));
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

    /// The file list command addresses a session and defaults the path to the
    /// session root, which is the host's own meaning for an empty path.
    #[tokio::test]
    async fn list_files_asks_the_host_for_the_session_root() {
        let (_home, fx) = start("peer-cmd-list-files").await;
        let desktop_id = fx.paired.creds.desktop_id.clone();
        let noop: runtime::Emitter = std::sync::Arc::new(|_| {});
        runtime::connect_with_emitter(&desktop_id, Some(noop))
            .await
            .expect("connect");

        // The host has no such session, so the read fails — what is asserted is
        // that the command forwarded a *well formed* request rather than
        // refusing one of its own arguments.
        let error = remote_peer_list_files(desktop_id, "sess_missing".into(), None)
            .await
            .expect_err("no such session");
        assert!(
            !error.to_string().contains("peer_not_connected"),
            "the command must reach the host, got: {error}"
        );

        teardown().await;
    }

    /// Downloading asks the host for a transfer it never prepared, so it fails —
    /// and the file must not exist afterwards. The command is the layer that
    /// decides *where* bytes land, so "nothing was written on failure" is the
    /// property worth asserting here.
    #[tokio::test]
    async fn a_failed_download_writes_nothing() {
        let (_home, fx) = start("peer-cmd-download-fails").await;
        let desktop_id = fx.paired.creds.desktop_id.clone();
        let noop: runtime::Emitter = std::sync::Arc::new(|_| {});
        runtime::connect_with_emitter(&desktop_id, Some(noop))
            .await
            .expect("connect");

        let destination = std::env::temp_dir()
            .join("futureos-peer-cmd-download")
            .join("never-written.txt");
        std::fs::create_dir_all(destination.parent().expect("parent")).expect("create dir");
        let _ = std::fs::remove_file(&destination);

        let error = remote_peer_download_file(
            desktop_id,
            "sess_missing".into(),
            "/tmp/whatever.txt".into(),
            Some("whatever.txt".into()),
            None,
            destination.to_string_lossy().into_owned(),
        )
        .await
        .expect_err("an unprepared transfer cannot be pulled");

        assert!(!error.to_string().is_empty(), "a reason must be reported");
        assert!(
            !destination.exists(),
            "a download that failed must not leave a file at the destination"
        );

        teardown().await;
    }

    /// The happy path through the command, end to end: the client asks a real
    /// host to prepare a real file, pulls it in chunks, and writes it where the
    /// caller chose.
    ///
    /// An absolute path is what makes this reachable without an agent session:
    /// the host resolves an absolute path directly (`resolve_local_link`), and
    /// its policy check only guards credential files. So this exercises the whole
    /// chain — prepare, the field mapping from the host's declaration, the chunk
    /// pull, the integrity check and the write — rather than a stand-in for it.
    #[tokio::test]
    async fn a_download_reaches_the_chosen_destination() {
        let (_home, fx) = start("peer-cmd-download-writes").await;
        let desktop_id = fx.paired.creds.desktop_id.clone();
        let noop: runtime::Emitter = std::sync::Arc::new(|_| {});
        runtime::connect_with_emitter(&desktop_id, Some(noop))
            .await
            .expect("connect");

        let source_dir = std::env::temp_dir().join("futureos-peer-cmd-download-src");
        std::fs::create_dir_all(&source_dir).expect("create dir");
        let source = source_dir.join("report.txt");
        let contents = b"the report contents";
        std::fs::write(&source, contents).expect("write the source file");

        let destination = std::env::temp_dir()
            .join("futureos-peer-cmd-download-dst")
            .join("saved.txt");
        std::fs::create_dir_all(destination.parent().expect("parent")).expect("create dir");
        let _ = std::fs::remove_file(&destination);

        let name = remote_peer_download_file(
            desktop_id,
            // The session is only consulted to name a file when the caller does
            // not supply a name, and an absolute path skips it entirely.
            "sess_unused".into(),
            source.to_string_lossy().into_owned(),
            Some("report.txt".into()),
            None,
            destination.to_string_lossy().into_owned(),
        )
        .await
        .expect("download");

        // The name the host chose, which is what the UI reports as saved.
        assert_eq!(name, "report.txt");
        assert_eq!(std::fs::read(&destination).expect("read back"), contents);

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
