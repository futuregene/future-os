//! Status cache projection (G-4 multi-projection) — a second projection
//! alongside the markdown state projection, mirroring LoopX
//! `control_plane/runtime/status_projection_cache.py`: a serialized snapshot
//! of the status read model keyed to the ledger-head digest so staleness is
//! detectable. It is a CACHE — the ledger + replay stay the source of truth;
//! a stale cache only means "rebuild before trusting".

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::state::{now_epoch, Goal, TodoSummary};

pub const STATUS_CACHE_SCHEMA_VERSION: &str = "status_cache_projection_v0";
const STATUS_CACHE_FILE: &str = "status-cache.json";

/// The status read-model cache (projection, never a second source of truth).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusCache {
    pub schema_version: String,
    pub goal_id: String,
    /// FNV-1a digest of the raw events.jsonl bytes — staleness key.
    pub ledger_digest: String,
    pub generated_at: u64,
    pub next_action: Option<String>,
    pub todo_count: u32,
    pub open_agent_todos: u32,
    pub open_user_gates: u32,
    pub open_monitors: u32,
    pub terminal: bool,
    pub summary: TodoSummary,
}

/// Ledger-head digest: FNV-1a over the raw events.jsonl bytes (16 hex).
pub fn ledger_digest(goal_dir: &Path) -> String {
    let events = goal_dir.join("events.jsonl");
    let bytes = std::fs::read(&events).unwrap_or_default();
    crate::store::content_digest(&bytes)[..16].to_string()
}

/// Build the cache snapshot from replayed state.
pub fn build_status_cache(goal: &Goal, digest: &str, now: u64) -> StatusCache {
    StatusCache {
        schema_version: STATUS_CACHE_SCHEMA_VERSION.to_string(),
        goal_id: goal.goal_id.clone(),
        ledger_digest: digest.to_string(),
        generated_at: now,
        next_action: goal.next_action.clone(),
        todo_count: goal.todos.len() as u32,
        open_agent_todos: goal.open_of(crate::state::TaskClass::Advancement).count() as u32,
        open_user_gates: goal.open_gates().count() as u32,
        open_monitors: goal.open_monitors().count() as u32,
        terminal: goal.is_terminal(),
        summary: goal.todo_summary(),
    }
}

/// Write the cache atomically (tmp + rename, like the scheduler state).
pub fn write_status_cache(goal_dir: &Path, cache: &StatusCache) -> Result<()> {
    let path = goal_dir.join(STATUS_CACHE_FILE);
    let tmp = path.with_extension("json.tmp");
    let payload = serde_json::to_string_pretty(cache)?;
    std::fs::write(&tmp, format!("{payload}\n")).context("write status cache tmp")?;
    std::fs::rename(&tmp, &path).context("rename status cache")?;
    Ok(())
}

/// Read the cache snapshot (None when absent or unparsable).
pub fn read_status_cache(goal_dir: &Path) -> Option<StatusCache> {
    let text = std::fs::read_to_string(goal_dir.join(STATUS_CACHE_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Whether the cache is stale relative to the current ledger head.
pub fn status_cache_stale(cache: &StatusCache, digest: &str) -> bool {
    cache.schema_version != STATUS_CACHE_SCHEMA_VERSION || cache.ledger_digest != digest
}

/// Rebuild the cache for a goal (read model refresh) and return it.
pub fn refresh_status_cache(goal: &Goal, goal_dir: &Path) -> Result<StatusCache> {
    let digest = ledger_digest(goal_dir);
    let cache = build_status_cache(goal, &digest, now_epoch());
    write_status_cache(goal_dir, &cache)?;
    Ok(cache)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Todo;

    #[test]
    fn cache_roundtrip_and_staleness() {
        let dir = tempfile::tempdir().unwrap();
        let mut goal = Goal::new("g", "objective", "/tmp");
        goal.add(Todo::advancement("t1", "work"));
        let digest = ledger_digest(dir.path());
        let cache = build_status_cache(&goal, &digest, 1_700_000_000);
        assert_eq!(cache.todo_count, 1);
        assert_eq!(cache.open_agent_todos, 1);
        assert!(!cache.terminal);

        write_status_cache(dir.path(), &cache).unwrap();
        let read = read_status_cache(dir.path()).unwrap();
        assert_eq!(read.goal_id, "g");
        assert!(!status_cache_stale(&read, &digest));

        // Different ledger head → stale.
        assert!(status_cache_stale(&read, "deadbeef"));
    }

    #[test]
    fn digest_changes_when_ledger_grows() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("events.jsonl"), "line1\n").unwrap();
        let d1 = ledger_digest(dir.path());
        std::fs::write(dir.path().join("events.jsonl"), "line1\nline2\n").unwrap();
        let d2 = ledger_digest(dir.path());
        assert_ne!(d1, d2);
    }
}
