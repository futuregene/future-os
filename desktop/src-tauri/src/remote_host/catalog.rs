//! Desktop-owned catalog source. Reads and revisions share one serialization
//! point across handshake, RPC and push; publication time never creates versions.
use serde_json::{json, Value};
use std::sync::{LazyLock, Mutex};

struct VersionedCatalog {
    epoch: String,
    sessions: (u64, String),
    workspaces: (u64, String),
}
static CATALOG: LazyLock<Mutex<VersionedCatalog>> = LazyLock::new(|| {
    Mutex::new(VersionedCatalog {
        epoch: nkeys::KeyPair::new_user().public_key(),
        sessions: (0, String::new()),
        workspaces: (0, String::new()),
    })
});
fn version(payload: &mut Value, epoch: &str, previous: &mut (u64, String), key: &str) {
    let signature = payload[key].to_string();
    if previous.0 == 0 || previous.1 != signature {
        previous.0 += 1;
        previous.1 = signature;
    }
    payload["version"] = json!({ "epoch": epoch, "revision": previous.0 });
}
pub(crate) fn epoch() -> String {
    CATALOG.lock().unwrap().epoch.clone()
}

/// The revisions last observed for each catalog domain, with the epoch they
/// belong to. Deliberately O(1): it reads the revision cached by the last
/// [`sessions`]/[`workspaces`] call instead of re-reading the store, so the
/// presence heartbeat can advertise this recovery signal on every tick for
/// free. A client that sees a revision newer than the one it last applied
/// knows a pushed snapshot was lost and pulls the catalogue itself — which is
/// what lets the desktop stop re-sending unchanged snapshots on a timer.
///
/// `0` means "no snapshot computed in this process yet"; the client's version
/// gate starts below that, so an unread domain never triggers a pull.
pub(crate) fn revisions() -> (String, u64, u64) {
    let state = CATALOG.lock().unwrap();
    (state.epoch.clone(), state.sessions.0, state.workspaces.0)
}
pub(crate) fn sessions(pair_id: &str) -> Option<(Value, String)> {
    let mut state = CATALOG.lock().unwrap();
    let mut payload = read_sessions(pair_id)?;
    let epoch = state.epoch.clone();
    version(&mut payload, &epoch, &mut state.sessions, "sessions");
    let signature = state.sessions.1.clone();
    Some((payload, signature))
}
pub(crate) fn workspaces() -> Option<(Value, String)> {
    let mut state = CATALOG.lock().unwrap();
    let mut payload = read_workspaces()?;
    let epoch = state.epoch.clone();
    version(&mut payload, &epoch, &mut state.workspaces, "workspaces");
    let signature = state.workspaces.1.clone();
    Some((payload, signature))
}
fn read_sessions(pair_id: &str) -> Option<Value> {
    let active_sessions = crate::store::active_run_sessions().ok()?;
    let threads = crate::store::list_threads().ok()?;

    let thread_ids: Vec<String> = threads.iter().map(|t| t.id.clone()).collect();
    let run_infos = crate::store::latest_run_infos(&thread_ids).ok()?;
    let run_status_by_thread: std::collections::HashMap<&str, &str> = run_infos
        .iter()
        .map(|info| (info.thread_id.as_str(), info.status.as_str()))
        .collect();

    let mut sessions: Vec<serde_json::Value> = Vec::new();
    for t in &threads {
        let Some(sid) = t
            .agent_session_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let streaming = active_sessions.iter().any(|active| active == sid);
        let status = run_status_by_thread.get(t.id.as_str()).copied();
        sessions.push(json!({
            "sessionId": sid,
            "threadId": t.id,
            "title": t.title,
            "mode": t.mode,
            "workspaceId": t.workspace_id,
            "parentSessionId": t.parent_session_id,
            "pinned": t.pinned,
            "streaming": streaming,
            "status": status,
            // Last-activity timestamp, in the *same* coalesce the store's own
            // ordering uses (`pinned DESC, COALESCE(last_message_at, updated_at,
            // created_at) DESC`). A client that merges several desktops ranks
            // rows across machines by this field and must not disagree with how
            // this desktop ranks its own list. Optional by contract: clients
            // built before this field fall back to the snapshot order, and a
            // row whose thread has no timestamps at all omits it.
            "lastMessageAt": t.last_message_at.unwrap_or(t.updated_at),
        }));
    }

    let payload = json!({
        "pairId": pair_id,
        "sessions": sessions,
    });
    Some(payload)
}

/// Workspaces-only snapshot for `p.{pair}.state.workspaces`.
fn read_workspaces() -> Option<Value> {
    let workspaces = crate::store::list_workspaces().ok()?;
    let mut workspace_values: Vec<serde_json::Value> = Vec::new();
    for w in &workspaces {
        if w.kind != "user" {
            continue;
        }
        if let Ok(value) = serde_json::to_value(w) {
            workspace_values.push(value);
        }
    }
    let payload = json!({ "workspaces": workspace_values });
    Some(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{create_thread, record_thread_message_activity, CreateThreadInput};

    fn new_thread(session: &str, title: &str) -> crate::store::ThreadRecord {
        create_thread(CreateThreadInput {
            mode: "chat".into(),
            title: Some(title.into()),
            workspace_id: None,
            workspace_path: Some("/tmp/catalog-snapshot".into()),
            workspace_name: Some("catalog".into()),
            agent_session_id: Some(session.into()),
        })
        .expect("create thread under the test HOME")
    }

    /// The cross-device ordering key. A client merging several desktops ranks
    /// rows across machines by `lastMessageAt`, so it has to be present, be the
    /// same coalesce the store orders by (`last_message_at`, else `updated_at`),
    /// and move when a message lands. Clients older than this field ignore it.
    #[test]
    fn sessions_snapshot_carries_last_message_at() {
        let _home = crate::remote::test_support::HomeGuard::new("catalog-last-message");
        crate::remote::test_support::init_store();

        let older = new_thread("sess-older", "Older");
        let newer = new_thread("sess-newer", "Newer");
        record_thread_message_activity(&newer.id, 2_000).expect("record activity");

        let (payload, _signature) = sessions("pair_test").expect("snapshot");
        let rows = payload["sessions"].as_array().expect("sessions array");
        let by_id = |session: &str| {
            rows.iter()
                .find(|row| row["sessionId"] == json!(session))
                .expect("session in snapshot")
        };
        assert_eq!(by_id("sess-newer")["lastMessageAt"], json!(2_000));
        // No message yet: the same `updated_at` fallback the store's ORDER BY uses.
        assert_eq!(
            by_id("sess-older")["lastMessageAt"],
            json!(older.updated_at)
        );
    }

    /// A thread without an agent session is not a remote session at all, and a
    /// deleted one never reaches the snapshot — `lastMessageAt` must not become
    /// a way to sneak either of them back in.
    #[test]
    fn sessions_snapshot_still_omits_unbound_threads() {
        let _home = crate::remote::test_support::HomeGuard::new("catalog-unbound");
        crate::remote::test_support::init_store();

        create_thread(CreateThreadInput {
            mode: "chat".into(),
            title: Some("No agent session yet".into()),
            workspace_id: None,
            workspace_path: Some("/tmp/catalog-snapshot".into()),
            workspace_name: Some("catalog".into()),
            agent_session_id: None,
        })
        .expect("create unbound thread");

        let (payload, _signature) = sessions("pair_test").expect("snapshot");
        assert!(payload["sessions"]
            .as_array()
            .expect("sessions array")
            .is_empty());
    }

    #[test]
    fn versions_follow_exact_snapshot_content_not_read_or_publish_time() {
        let mut previous = (0, String::new());
        let mut first = json!({"workspaces":[{"id":"w", "name":"same", "path":"/a"}]});
        version(&mut first, "source", &mut previous, "workspaces");
        let original = first["version"].clone();
        version(&mut first, "source", &mut previous, "workspaces");
        assert_eq!(first["version"], original);
        first["workspaces"][0]["path"] = json!("/b");
        version(&mut first, "source", &mut previous, "workspaces");
        assert_eq!(first["version"]["revision"], 2);
        assert_eq!(first["version"]["epoch"], "source");
    }
}
