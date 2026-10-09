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
fn emitter(app: tauri::AppHandle) -> runtime::Emitter {
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
pub async fn remote_peer_pair(
    app: tauri::AppHandle,
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
pub async fn remote_peer_connect(
    app: tauri::AppHandle,
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
