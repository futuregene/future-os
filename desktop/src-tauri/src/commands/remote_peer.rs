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

/// Bring every paired host up at once and answer with the list as it stands.
///
/// Two callers, both of them "everything should be live": the app's launch
/// pass, and the user's own "reconnect all". It never fails as a whole 鈥?each
/// host's own failure is recorded against it and read off the returned list 鈥?
/// because one machine being asleep must not leave the other two unconnected.
#[tauri::command]
pub async fn remote_peer_connect_all<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<Vec<PeerSummary>, crate::AppError> {
    runtime::connect_all(Some(emitter(app))).await
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

/// Tell the backend which conversation the user is looking at, so the live lane
/// can be narrowed to it (`selective_events_v1`).
///
/// The view that opens a conversation is the only thing that knows this, and it
/// is the difference between reading one conversation's events and reading every
/// conversation on the host. Always `Ok`: it records an intent, and the wide
/// lane is the fallback whenever the host cannot narrow — so there is nothing
/// for a caller to handle and nothing it could do about a failure.
#[tauri::command]
pub async fn remote_peer_set_visible_session(
    desktop_id: String,
    session_id: Option<String>,
) -> Result<(), crate::AppError> {
    runtime::set_visible_session(&desktop_id, session_id.as_deref()).await;
    Ok(())
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

/// Upload a local file to a host, and return the id that references it there.
///
/// The id is the whole point: the host keeps the bytes in a staging area, and a
/// prompt attaches them by id. So this does not write into the other machine's
/// filesystem — nothing is placed anywhere until the user actually sends it.
///
/// The name is taken from the path unless the caller has a better one, because
/// the host uses it for the attachment's display name and sends it back in the
/// conversation.
#[tauri::command]
pub async fn remote_peer_upload_file(
    desktop_id: String,
    name: Option<String>,
    path: String,
) -> Result<UploadedAttachment, crate::AppError> {
    let bytes = std::fs::read(&path)
        .map_err(|error| crate::AppError::Message(format!("remote_upload_read_failed: {error}")))?;
    let file_name = name
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::path::Path::new(&path)
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
        })
        .ok_or_else(|| crate::AppError::Message("remote_upload_needs_a_name".into()))?;
    let uploaded = crate::remote_peer::transfer::upload(&desktop_id, &file_name, &bytes).await?;
    Ok(UploadedAttachment {
        content_hash: uploaded.content_hash,
        name: uploaded.name,
        upload_id: uploaded.upload_id,
    })
}

/// A file this machine has handed to a host, as the UI needs it.
///
/// `upload_id` is what a prompt attaches; the name and hash come back so the
/// composer can show what was sent and the same bytes are never sent twice by
/// accident.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadedAttachment {
    pub content_hash: String,
    pub name: String,
    pub upload_id: String,
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
    crate::remote_peer::transfer::write_to_path(&destination, &fetched.bytes)?;
    Ok(fetched.name)
}

/// Pull a file from a host into this machine's preview cache, and answer with the
/// local path the overlay can read.
///
/// Two things this does that a save-as cannot. It asks for the host's *preview*
/// variant, which is what a host resizes an image down to and validates before
/// sending — the same variant the phone previews with. And when the host refuses
/// that variant it retries with the original bytes: the host's preview vocabulary
/// is the *phone's* (a handful of text extensions plus images) while this app's
/// overlay reads whatever the operating system can hand it, so a refusal there is
/// not "this file cannot be previewed here". The overlay decides what is
/// previewable; this only fetches.
///
/// The name the host reports is what the caller shows and what the cache keeps,
/// because the overlay picks its renderer off the extension.
#[tauri::command]
pub async fn remote_peer_preview_file(
    desktop_id: String,
    session_id: String,
    path: String,
    name: Option<String>,
) -> Result<Value, crate::AppError> {
    let fetched = match crate::remote_peer::transfer::download(
        &desktop_id,
        &session_id,
        &path,
        name.as_deref(),
        "preview",
    )
    .await
    {
        Ok(fetched) => fetched,
        Err(_) => {
            crate::remote_peer::transfer::download(
                &desktop_id,
                &session_id,
                &path,
                name.as_deref(),
                "original",
            )
            .await?
        }
    };
    let destination = crate::remote_peer::transfer::preview_cache_path(
        &desktop_id,
        &session_id,
        &path,
        &fetched.name,
    )?;
    crate::remote_peer::transfer::write_atomically(&destination, &fetched.bytes)?;
    crate::remote_peer::transfer::prune_preview_cache();
    Ok(serde_json::json!({
        "name": fetched.name,
        "path": destination.display().to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::test_support::HomeGuard;
    use crate::remote_peer::testing::{fixture, teardown, Fixture};

    /// Every command shares one setup: a clean runtime, a real host bridge, and
    /// a claimed pairing.
    async fn start(label: &str) -> (HomeGuard, Fixture) {
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
                remote_peer_set_visible_session,
                remote_peer_sessions,
                remote_peer_workspaces,
                remote_peer_list_files,
                remote_peer_download_file,
                remote_peer_preview_file,
                remote_peer_upload_file,
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
                (
                    // `session_id` is the last argument and an `Option<String>`.
                    // A *missing* key would `visit_none()` and succeed, so only a
                    // present wrong-typed value makes the final
                    // `CommandArg::from_command(..)?` fail — which is what puts the
                    // error arm on the `#[tauri::command]` attribute line.
                    "remote_peer_set_visible_session",
                    serde_json::json!({ "desktopId": "d", "sessionId": 1 }),
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
                    "remote_peer_preview_file",
                    serde_json::json!({
                        "desktopId": "d",
                        "sessionId": "s",
                        "path": "p",
                        "name": 1,
                    }),
                ),
                (
                    "remote_peer_upload_file",
                    serde_json::json!({ "desktopId": "d", "name": "n", "path": 1 }),
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
    async fn connect_all_through_the_command_reaches_the_paired_host() {
        let (_home, fx) = start("peer-cmd-connect-all").await;
        let app = mock_app();
        let desktop_id = fx.paired.creds.desktop_id.clone();

        // The app's own liveness pass goes through this wrapper, so the wrapper
        // is what has to carry the emitter: without one the host would be
        // connected and its pushes would go nowhere.
        let peers = remote_peer_connect_all(app)
            .await
            .expect("connect every pairing");
        let peer = peers
            .into_iter()
            .find(|peer| peer.desktop_id == desktop_id)
            .expect("the paired host is in the list");
        assert!(peer.connected, "{peer:?}");

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

    /// The command is the only thing that knows which conversation the view is
    /// on, so it is what moves the host's live lane onto it — and closing the
    /// conversation gives the whole host back.
    ///
    /// Asserted on the broker's own subscription table, not merely on "no error
    /// arrived": the command's whole effect is *which subject* the runtime
    /// subscribes to, and an intent that was dropped on the floor would look the
    /// same as one that took effect. It also pins the "always `Ok`" contract — a
    /// host that is not connected, or cannot narrow, is only ever a fallback to
    /// the wide lane, so there is nothing for the caller to handle.
    #[tokio::test]
    async fn the_visible_session_command_moves_the_hosts_live_lane() {
        let (_home, fx) = start("peer-cmd-visible-session").await;
        let desktop_id = fx.paired.creds.desktop_id.clone();
        let noop: runtime::Emitter = std::sync::Arc::new(|_| {});
        runtime::connect_with_emitter(&desktop_id, Some(noop))
            .await
            .expect("connect");

        // Connected, every conversation on the host is the fallback lane.
        let wide = format!("p.{}.evt.>", fx.pair_id);
        fx.nats
            .wait_for_sub(&wide, std::time::Duration::from_secs(10))
            .await;

        // Opening a conversation through the command narrows it to that one...
        remote_peer_set_visible_session(desktop_id.clone(), Some("sess_open".into()))
            .await
            .expect("the set command never fails");
        let narrow = format!("p.{}.evt.sess_open", fx.pair_id);
        fx.nats
            .wait_for_sub(&narrow, std::time::Duration::from_secs(10))
            .await;

        // ...and leaving it (`None`) takes the whole host back.
        remote_peer_set_visible_session(desktop_id, None)
            .await
            .expect("the clear command never fails");
        fx.nats
            .wait_for_sub(&wide, std::time::Duration::from_secs(10))
            .await;

        // A host with no connection at all is still only an intent: recorded,
        // answered `Ok`, and left to the wide lane to fall back on.
        remote_peer_set_visible_session("desktop_nobody".into(), Some("sess_elsewhere".into()))
            .await
            .expect("an intent for an unconnected host is not a failure");

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

    /// A preview is pulled into this machine's cache, under the host's own name.
    ///
    /// Two things are asserted together because they are one contract: the local
    /// path is readable (that is what the overlay is handed), and it *ends with the
    /// host's name* — the overlay picks its renderer from the extension, so a
    /// cache entry that lost it would open every image as an unreadable blob.
    #[tokio::test]
    async fn a_preview_lands_in_this_machines_cache_under_the_hosts_name() {
        let (_home, fx) = start("peer-cmd-preview").await;
        let desktop_id = fx.paired.creds.desktop_id.clone();
        let noop: runtime::Emitter = std::sync::Arc::new(|_| {});
        runtime::connect_with_emitter(&desktop_id, Some(noop))
            .await
            .expect("connect");

        let dir = std::env::temp_dir().join("futureos-peer-cmd-preview");
        std::fs::create_dir_all(&dir).expect("create dir");
        let source = dir.join("notes.md");
        let contents = b"# Notes\n\nbody";
        std::fs::write(&source, contents).expect("write the source file");

        let preview = remote_peer_preview_file(
            desktop_id,
            "sess_unused".into(),
            source.to_string_lossy().into_owned(),
            Some("notes.md".into()),
        )
        .await
        .expect("preview");

        assert_eq!(preview["name"], serde_json::json!("notes.md"));
        let local = preview["path"].as_str().expect("a local path");
        assert!(
            local.contains("remote-previews") && local.ends_with("notes.md"),
            "the preview is cached under the host's name: {local}"
        );
        assert_eq!(
            std::fs::read(local).expect("read the cached preview"),
            contents
        );

        teardown().await;
    }

    /// A file the host will not preview is still previewed here.
    ///
    /// The host's preview vocabulary is the *phone's* — images and a short list of
    /// text extensions — while this app's overlay reads whatever the operating
    /// system can hand it. A refusal there therefore means "the phone could not show
    /// this", not "this cannot be previewed", so the original bytes are fetched and
    /// the same overlay reads them. Without this the two clients would disagree
    /// about which files can be opened, which is exactly what "the same file types
    /// as the local panel" rules out.
    #[tokio::test]
    async fn a_file_the_host_will_not_preview_is_fetched_as_its_original() {
        let (_home, fx) = start("peer-cmd-preview-fallback").await;
        let desktop_id = fx.paired.creds.desktop_id.clone();
        let noop: runtime::Emitter = std::sync::Arc::new(|_| {});
        runtime::connect_with_emitter(&desktop_id, Some(noop))
            .await
            .expect("connect");

        let dir = std::env::temp_dir().join("futureos-peer-cmd-preview-fallback");
        std::fs::create_dir_all(&dir).expect("create dir");
        // Not an image, not Markdown/JSON, not one of the phone's text extensions:
        // the host's preview variant refuses this one.
        let source = dir.join("archive.zip");
        let contents = b"PK-not-really-a-zip";
        std::fs::write(&source, contents).expect("write the source file");

        let preview = remote_peer_preview_file(
            desktop_id,
            "sess_unused".into(),
            source.to_string_lossy().into_owned(),
            Some("archive.zip".into()),
        )
        .await
        .expect("preview");

        let local = preview["path"].as_str().expect("a local path");
        assert_eq!(
            std::fs::read(local).expect("read the cached preview"),
            contents
        );

        teardown().await;
    }

    /// The upload command, end to end: a real file, a real host, and the id a
    /// prompt would attach.
    #[tokio::test]
    async fn an_upload_reaches_the_host_and_reports_the_ids_name_and_hash() {
        let (_home, fx) = start("peer-cmd-upload").await;
        let desktop_id = fx.paired.creds.desktop_id.clone();
        let noop: runtime::Emitter = std::sync::Arc::new(|_| {});
        runtime::connect_with_emitter(&desktop_id, Some(noop))
            .await
            .expect("connect");

        let dir = std::env::temp_dir().join("futureos-peer-cmd-upload");
        std::fs::create_dir_all(&dir).expect("create dir");
        let source = dir.join("report.txt");
        let contents = b"the report the model should read";
        std::fs::write(&source, contents).expect("write the file");

        let uploaded =
            remote_peer_upload_file(desktop_id, None, source.to_string_lossy().into_owned())
                .await
                .expect("upload");

        assert_eq!(uploaded.name, "report.txt");
        assert!(uploaded.upload_id.starts_with("upload_"));
        let expected = {
            use sha2::{Digest, Sha256};
            format!("{:x}", Sha256::digest(contents))
        };
        assert_eq!(uploaded.content_hash, expected);

        teardown().await;
    }

    /// The caller's own name wins, because the host shows it in the conversation.
    #[tokio::test]
    async fn an_upload_uses_the_name_the_caller_gave() {
        let (_home, fx) = start("peer-cmd-upload-name").await;
        let desktop_id = fx.paired.creds.desktop_id.clone();
        let noop: runtime::Emitter = std::sync::Arc::new(|_| {});
        runtime::connect_with_emitter(&desktop_id, Some(noop))
            .await
            .expect("connect");

        let dir = std::env::temp_dir().join("futureos-peer-cmd-upload-name");
        std::fs::create_dir_all(&dir).expect("create dir");
        let source = dir.join("DSC_0042.JPG");
        std::fs::write(&source, b"pretend jpeg").expect("write");

        let uploaded = remote_peer_upload_file(
            desktop_id,
            Some("Holiday photo.jpg".into()),
            source.to_string_lossy().into_owned(),
        )
        .await
        .expect("upload");

        // The host's display name for it is the one it was given, not the path's.
        assert_eq!(uploaded.name, "Holiday photo.jpg");

        teardown().await;
    }

    /// A path that cannot be read fails before anything reaches the host.
    #[tokio::test]
    async fn an_upload_of_an_unreadable_file_fails_locally() {
        let (_home, fx) = start("peer-cmd-upload-missing").await;
        let desktop_id = fx.paired.creds.desktop_id.clone();
        let noop: runtime::Emitter = std::sync::Arc::new(|_| {});
        runtime::connect_with_emitter(&desktop_id, Some(noop))
            .await
            .expect("connect");

        let error = remote_peer_upload_file(
            desktop_id,
            None,
            std::env::temp_dir()
                .join("futureos-peer-cmd-upload-absent")
                .join("nothing-here.txt")
                .to_string_lossy()
                .into_owned(),
        )
        .await
        .expect_err("a file that is not there cannot be uploaded");

        assert!(
            error.to_string().contains("remote_upload_read_failed"),
            "got: {error}"
        );

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
