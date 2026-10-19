//! Desktop forwards snapshots/control notifications to its owned Server instance.
use super::*;
pub(super) fn build_presence_payload(pair: &str, bridge: &str) -> serde_json::Value {
    SUPERVISOR.build_presence_payload(pair, bridge)
}
#[cfg(test)]
pub(super) fn build_sessions_snapshot(pair: &str) -> Option<(serde_json::Value, String)> {
    SUPERVISOR.build_sessions_snapshot(pair)
}
#[cfg(test)]
pub(super) fn build_workspaces_snapshot() -> Option<(serde_json::Value, String)> {
    SUPERVISOR.build_workspaces_snapshot()
}
pub(super) fn light_presence_payload(pair: &str, bridge: &str) -> serde_json::Value {
    SUPERVISOR.light_presence_payload(pair, bridge)
}
#[cfg(test)]
pub(super) fn build_presence_snapshot(pair: &str, bridge: &str) -> (serde_json::Value, String) {
    SUPERVISOR.build_presence_snapshot(pair, bridge)
}
