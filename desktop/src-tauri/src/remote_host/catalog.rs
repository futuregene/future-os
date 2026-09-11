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
