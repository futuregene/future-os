use super::*;

/// Append one signature field as `<byte-len>:<bytes>`. Because every record
/// emits a fixed number of fields in a fixed order, length-prefixing makes the
/// whole catalog signature unambiguous without any record/field separator — so a
/// title that happens to contain a separator character can't collide two
/// different catalogs into the same signature (which would silently skip a sync).
/// Build the full presence snapshot (directory + per-session streaming) together
/// with a signature that changes iff the snapshot's UI-visible content changes.
/// The signature is recomputed straight from the store each call, so it can never
/// drift from reality: a missed dirty-mark only delays propagation (the catalog
/// tick recomputes it and advertises the new revision), it never desyncs.
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

/// Liveness-only payload: no directory content. Carries no catalog revision
/// either; a caller that needs the recovery signal uses
/// [`presence_heartbeat_payload`].
pub(super) fn light_presence_payload(pair_id: &str, bridge_instance_id: &str) -> serde_json::Value {
    json!({
        "online": true,
        "agentAvailable": host().agent_available(),
        "pairId": pair_id,
        "bridgeInstanceId": bridge_instance_id,
        "lastHeartbeatTs": unix_timestamp(),
    })
}

/// Heartbeat payload: liveness plus the catalog revision.
///
/// The revision is what replaces the old "re-send every unchanged snapshot
/// every 20s" self-heal. A client compares it against the revision it last
/// applied; when the advertised one is newer, a pushed snapshot was lost (core
/// NATS is at-most-once) and the client pulls the catalogue itself. That makes
/// recovery faster than the timer it replaces (one tick, not 20s) while an
/// idle, unchanged directory costs nothing but this packet.
///
/// A revision that stays put on an unchanged snapshot is only safe because
/// `sessions()`/`workspaces()` recompute it from the store on every catalog
/// tick: the signal is derived from ground truth, not from an accumulating
/// dirty flag that a missed mark could strand.
pub(super) fn presence_heartbeat_payload(
    pair_id: &str,
    bridge_instance_id: &str,
) -> serde_json::Value {
    let mut payload = light_presence_payload(pair_id, bridge_instance_id);
    let (epoch, sessions, workspaces) = host().catalog_revisions();
    payload["catalogVersion"] = json!({
        "epoch": epoch,
        "sessions": sessions,
        "workspaces": workspaces,
    });
    payload
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
