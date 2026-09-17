use super::*;

/// Append one signature field as `<byte-len>:<bytes>`. Because every record
/// emits a fixed number of fields in a fixed order, length-prefixing makes the
/// whole catalog signature unambiguous without any record/field separator — so a
/// title that happens to contain a separator character can't collide two
/// different catalogs into the same signature (which would silently skip a sync).
/// Build the full presence snapshot (directory + per-session streaming) together
/// with a signature that changes iff the snapshot's UI-visible content changes.
/// The signature is recomputed straight from the store each call, so it can never
/// drift from reality: a missed dirty-mark only delays propagation (the 20s
/// heartbeat recomputes and self-heals), it never desyncs.
pub(super) fn build_presence_snapshot(
    pair_id: &str,
    bridge_instance_id: &str,
) -> (serde_json::Value, String) {
    let mut payload = light_presence_payload(pair_id, bridge_instance_id);
    payload["catalogEpoch"] = json!(host().catalog_epoch());
    let mut signature = String::new();
    if let Some((sessions, sig)) = build_sessions_snapshot(pair_id) {
        payload["sessions"] = sessions["sessions"].clone();
        payload["sessionsVersion"] = sessions["version"].clone();
        signature.push_str(&sig);
    }
    if let Some((workspaces, sig)) = build_workspaces_snapshot() {
        payload["workspaces"] = workspaces["workspaces"].clone();
        payload["workspacesVersion"] = workspaces["version"].clone();
        signature.push_str(&sig);
    }
    (payload, signature)
}

/// Full directory snapshot for the handshake and on-demand `get_presence`
/// (always complete, so a freshly connected client gets a usable baseline).
pub(super) fn build_presence_payload(pair_id: &str, bridge_instance_id: &str) -> serde_json::Value {
    build_presence_snapshot(pair_id, bridge_instance_id).0
}

/// Sessions-only snapshot for `p.{pair}.state.sessions`.
///
/// Returns `None` when any backing store read fails. A transient SQLite error
/// must not surface as an *empty* session list — publishing `{"sessions":[]}`
/// makes the phone's "selected session vanished" heuristic fire and close a
/// conversation the user is reading (audit 05 L8). On failure the publisher
/// skips this tick and the phone keeps the previous snapshot.
pub(super) fn build_sessions_snapshot(pair_id: &str) -> Option<(serde_json::Value, String)> {
    host().sessions(pair_id)
}
pub(super) fn build_workspaces_snapshot() -> Option<(serde_json::Value, String)> {
    host().workspaces()
}

/// Liveness-only heartbeat (no directory). Sent every ~20s while the catalog is
/// unchanged so an idle link carries almost no traffic.
pub(super) fn light_presence_payload(pair_id: &str, bridge_instance_id: &str) -> serde_json::Value {
    json!({
        "online": true,
        "agentAvailable": host().agent_available(),
        "pairId": pair_id,
        "bridgeInstanceId": bridge_instance_id,
        "lastHeartbeatTs": unix_timestamp(),
    })
}

pub(super) fn unix_timestamp() -> u64 {
    // `unwrap_or_default`: a pre-epoch clock is not a reachable failure mode
    // worth an arm — treat it as 0.
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(super) fn unix_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
