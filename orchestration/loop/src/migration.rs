//! Schema migration bridge (G-6) — event-store version stamps + a minimal
//! migration-step registry, mirroring LoopX
//! `control_plane/runtime/event_store_migration_bridge.py` + `state_migration`.
//!
//! The migration bridge is FAIL-CLOSED: the Markdown parser stays the
//! canonical read source until event-read-path prerequisites are done,
//! dual-read parity + rollback + idempotency + public-boundary checks are
//! clean, and a bounded canary passes. `promotion_allowed` is always `false`
//! here — promotion is an explicit reviewed write-path change, never auto.
//!
//! The minimal step registry covers the two event-surface changes P2 makes:
//! "add event id envelope" and "add event types" (new variants deserialize
//! via serde defaults; the step documents + backfills ids on legacy lines).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::store::{content_digest, EVENT_STORE_SCHEMA_VERSION, LEGACY_EVENT_STORE_SCHEMA_VERSION};

pub const EVENT_STORE_MIGRATION_BRIDGE_SCHEMA_VERSION: &str = "event_store_migration_bridge_v0";
pub const MARKDOWN_ACTIVE_STATE_SOURCE: &str = "markdown_active_state";
pub const EVENT_PROJECTION_SOURCE: &str = "event_projection";

/// The kind of schema change a migration step represents (LoopX state_migration
/// classifies steps; we cover the two minimal kinds P2 introduces).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MigrationKind {
    #[serde(rename = "add_event_type")]
    AddEventType,
    #[serde(rename = "add_field")]
    AddField,
    #[serde(rename = "envelope")]
    Envelope,
}

/// One registered migration step (from → to, with a line transform).
#[derive(Debug, Clone)]
pub struct MigrationStep {
    pub from: &'static str,
    pub to: &'static str,
    pub kind: MigrationKind,
    pub description: &'static str,
    pub transform: fn(&mut serde_json::Value) -> Result<()>,
}

/// Ensure a legacy line carries a content-derived `event_id` (the G-3
/// envelope migration; idempotent — leaves an existing id untouched).
fn ensure_event_id(value: &mut serde_json::Value) -> Result<()> {
    let has_id = value
        .as_object()
        .and_then(|o| o.get("event_id"))
        .and_then(|v| v.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    if has_id {
        return Ok(());
    }
    let id = crate::store::derive_event_id_from_value(value);
    if let Some(obj) = value.as_object_mut() {
        obj.insert("event_id".to_string(), serde_json::Value::String(id));
    }
    Ok(())
}

/// The registered migration steps (minimal set for P2).
pub fn migration_steps() -> Vec<MigrationStep> {
    vec![
        MigrationStep {
            from: LEGACY_EVENT_STORE_SCHEMA_VERSION,
            to: EVENT_STORE_SCHEMA_VERSION,
            kind: MigrationKind::Envelope,
            description: "add event_id envelope + content-derived ids; new event types (quota_spent / evidence_attached / lease) read via serde defaults",
            transform: ensure_event_id,
        },
        MigrationStep {
            from: LEGACY_EVENT_STORE_SCHEMA_VERSION,
            to: EVENT_STORE_SCHEMA_VERSION,
            kind: MigrationKind::AddEventType,
            description: "register quota_spent / evidence_attached / todo_renewed / todo_released / todo_expired variants (no structural change — serde defaults)",
            transform: |_| Ok(()),
        },
        MigrationStep {
            from: LEGACY_EVENT_STORE_SCHEMA_VERSION,
            to: EVENT_STORE_SCHEMA_VERSION,
            kind: MigrationKind::AddField,
            description: "add Goal.quota_spent_slots projection field (serde default 0 on rebuild)",
            transform: |_| Ok(()),
        },
        MigrationStep {
            from: LEGACY_EVENT_STORE_SCHEMA_VERSION,
            to: EVENT_STORE_SCHEMA_VERSION,
            kind: MigrationKind::AddEventType,
            description: "register supervisor_proposed / supervisor_receipt_recorded variants (G-16; projection-only, no structural change)",
            transform: |_| Ok(()),
        },
    ]
}

/// The steps applicable for a `from → to` upgrade, in registration order.
pub fn applicable_steps(from: &str, to: &str) -> Vec<MigrationStep> {
    migration_steps()
        .into_iter()
        .filter(|s| s.from == from && s.to == to)
        .collect()
}

/// Apply the registered migrations to ONE event line in place (read path).
/// No-op when `from == to` or no step matches.
pub fn migrate_event_line(value: &mut serde_json::Value, from: &str, to: &str) -> Result<()> {
    if from == to {
        return Ok(());
    }
    for step in applicable_steps(from, to) {
        (step.transform)(value)?;
    }
    Ok(())
}

// ── Write-path migration (explicit, reviewable) ────────────────────────────

/// Result of a write-path migration run.
#[derive(Debug, Clone, Serialize)]
pub struct MigrationReport {
    pub goal_id: String,
    pub from: String,
    pub to: String,
    pub migrated_lines: usize,
    pub backup_path: String,
    pub rollback_plan: String,
    pub non_destructive: bool,
}

/// Rewrite the ledger applying all applicable migrations (write path), with
/// a pre-migration backup so the migration is reversible (LoopX rollback
/// plan: keep the pre-migration file, one-command restore). Non-destructive:
/// tmp + rename; the backup is byte-identical to the pre-migration ledger.
pub fn apply_migrations(goal_dir: &Path, goal_id: &str) -> Result<MigrationReport> {
    let events_path = goal_dir.join("events.jsonl");
    if !events_path.exists() {
        anyhow::bail!("goal {goal_id} has no event ledger to migrate");
    }
    let stamp_path = goal_dir.join("schema.json");
    let from = read_schema_version(&stamp_path)
        .unwrap_or_else(|| LEGACY_EVENT_STORE_SCHEMA_VERSION.to_string());
    if from == EVENT_STORE_SCHEMA_VERSION {
        anyhow::bail!("goal {goal_id} is already on schema {from}");
    }
    let text = std::fs::read_to_string(&events_path)?;
    let mut migrated = 0usize;
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        if line.trim().is_empty() {
            out.push('\n');
            continue;
        }
        let mut value: serde_json::Value =
            serde_json::from_str(line).context("parse event line")?;
        migrate_event_line(&mut value, &from, EVENT_STORE_SCHEMA_VERSION)?;
        out.push_str(&serde_json::to_string(&value)?);
        out.push('\n');
        migrated += 1;
    }

    // Backup (rollback plan): byte-identical pre-migration copy.
    let backup_path = goal_dir.join(format!(
        "events.pre-migration-{}-{}.jsonl",
        crate::state::now_epoch(),
        &content_digest(text.as_bytes())[..8]
    ));
    std::fs::write(&backup_path, &text).context("write migration backup")?;

    let tmp = events_path.with_extension("jsonl.tmp");
    std::fs::write(&tmp, &out).context("write migrated ledger tmp")?;
    std::fs::rename(&tmp, &events_path).context("rename migrated ledger")?;
    write_schema_version(&stamp_path, EVENT_STORE_SCHEMA_VERSION)?;

    Ok(MigrationReport {
        goal_id: goal_id.to_string(),
        from: from.clone(),
        to: EVENT_STORE_SCHEMA_VERSION.to_string(),
        migrated_lines: migrated,
        backup_path: backup_path.to_string_lossy().into_owned(),
        rollback_plan: format!(
            "restore `{}` over events.jsonl and delete schema.json to roll back to {from}",
            backup_path.to_string_lossy()
        ),
        non_destructive: true,
    })
}

fn read_schema_version(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get("event_store_schema_version")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn write_schema_version(path: &Path, version: &str) -> Result<()> {
    let payload = serde_json::json!({
        "event_store_schema_version": version,
        "migrated_at": crate::state::now_epoch(),
    });
    std::fs::write(path, serde_json::to_string_pretty(&payload)? + "\n")
        .context("write schema stamp")?;
    Ok(())
}

// ── Fail-closed bridge status ──────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MigrationChecks {
    pub event_read_path_ready: bool,
    pub active_state_projection_ready: bool,
    pub dual_read_parity_clean: bool,
    pub rollback_plan_recorded: bool,
    pub bounded_canary_passed: bool,
    pub idempotency_conflicts_clean: bool,
    pub public_boundary_clean: bool,
    pub event_projection_head_matches_store: bool,
}

impl MigrationChecks {
    /// Look up a check flag by its manifest key (`None` for unknown keys).
    fn get(&self, key: &str) -> Option<bool> {
        Some(match key {
            "event_read_path_ready" => self.event_read_path_ready,
            "active_state_projection_ready" => self.active_state_projection_ready,
            "dual_read_parity_clean" => self.dual_read_parity_clean,
            "event_projection_head_matches_store" => self.event_projection_head_matches_store,
            "rollback_plan_recorded" => self.rollback_plan_recorded,
            "idempotency_conflicts_clean" => self.idempotency_conflicts_clean,
            "public_boundary_clean" => self.public_boundary_clean,
            "bounded_canary_passed" => self.bounded_canary_passed,
            _ => return None,
        })
    }
}

/// Fail-closed migration bridge (LoopX `build_event_store_migration_bridge`):
/// stage is derived from the checks; `promotion_allowed` is always false.
#[derive(Debug, Clone, Serialize)]
pub struct MigrationBridge {
    pub schema_version: String,
    pub goal_id: String,
    pub source_of_truth: String,
    pub candidate_source: String,
    pub stage: String,
    pub promotion_allowed: bool,
    pub promotion_candidate: bool,
    pub next_action: String,
    pub checks: MigrationChecks,
    pub missing_for_shadow: Vec<String>,
    pub missing_for_canary: Vec<String>,
    pub missing_for_promotion: Vec<String>,
    pub rollback: RollbackPlan,
}

#[derive(Debug, Clone, Serialize)]
pub struct RollbackPlan {
    pub required: bool,
    pub recorded: bool,
    pub fallback_source: String,
    pub triggers: Vec<String>,
    pub action: String,
}

/// Build the bridge payload from check flags (LoopX semantics).
pub fn build_migration_bridge(
    goal_id: &str,
    checks: MigrationChecks,
    backups_exist: bool,
) -> MigrationBridge {
    let required_for_shadow = ["event_read_path_ready", "active_state_projection_ready"];
    let required_for_canary = [
        "event_read_path_ready",
        "active_state_projection_ready",
        "dual_read_parity_clean",
        "event_projection_head_matches_store",
        "rollback_plan_recorded",
        "idempotency_conflicts_clean",
        "public_boundary_clean",
    ];
    let required_for_promotion = [
        "event_read_path_ready",
        "active_state_projection_ready",
        "dual_read_parity_clean",
        "event_projection_head_matches_store",
        "rollback_plan_recorded",
        "idempotency_conflicts_clean",
        "public_boundary_clean",
        "bounded_canary_passed",
    ];
    let is_set = |key: &str| checks.get(key).unwrap_or(false);
    let missing = |required: &[&str]| -> Vec<String> {
        required
            .iter()
            .filter(|k| !is_set(k))
            .map(|k| k.to_string())
            .collect()
    };
    let missing_for_shadow = missing(&required_for_shadow);
    let missing_for_canary = missing(&required_for_canary);
    let missing_for_promotion = missing(&required_for_promotion);

    let (stage, next_action) = if !missing_for_shadow.is_empty() {
        (
            "wait_for_event_read_path",
            "finish event read-path prerequisites before dual-read migration work",
        )
    } else if !missing_for_canary.is_empty() {
        (
            "dual_read_shadow",
            "compare Markdown read model and event projection until parity, rollback, idempotency, and public-boundary checks are clean",
        )
    } else if !missing_for_promotion.is_empty() {
        (
            "bounded_canary",
            "run the bounded canary on a small goal set before promotion",
        )
    } else {
        (
            "promotion_candidate",
            "promote event projection only through an explicit reviewed write-path change",
        )
    };

    MigrationBridge {
        schema_version: EVENT_STORE_MIGRATION_BRIDGE_SCHEMA_VERSION.to_string(),
        goal_id: goal_id.to_string(),
        source_of_truth: MARKDOWN_ACTIVE_STATE_SOURCE.to_string(),
        candidate_source: EVENT_PROJECTION_SOURCE.to_string(),
        stage: stage.to_string(),
        promotion_allowed: false,
        promotion_candidate: missing_for_promotion.is_empty(),
        next_action: next_action.to_string(),
        checks: checks.clone(),
        missing_for_shadow,
        missing_for_canary,
        missing_for_promotion,
        rollback: RollbackPlan {
            required: true,
            recorded: backups_exist,
            fallback_source: MARKDOWN_ACTIVE_STATE_SOURCE.to_string(),
            triggers: vec![
                "parity delta".to_string(),
                "projection head mismatch".to_string(),
                "event append conflict".to_string(),
                "public boundary warning".to_string(),
                "canary regression".to_string(),
            ],
            action: "disable event projection preference and keep Markdown parser as canonical read fallback".to_string(),
        },
    }
}

/// Derive the bridge checks from the actual goal state (G-3 verify + G-4
/// privacy + G-3 markdown backfill parity). Fail-closed on any error.
pub fn migration_bridge_status(
    store: &crate::store::Store,
    goal_id: &str,
    goal_dir: &Path,
) -> MigrationBridge {
    let mut checks = MigrationChecks::default();

    let ledger_ok = store.verify(goal_id).map(|r| r.ok).unwrap_or(false);
    checks.event_read_path_ready = ledger_ok;
    checks.idempotency_conflicts_clean = ledger_ok;

    let goal = store.replay(goal_id).ok().flatten();
    let active_state_ready = goal
        .as_ref()
        .map(|g| g.next_action.is_some())
        .unwrap_or(false);
    checks.active_state_projection_ready = active_state_ready;

    // Dual-read parity: Markdown workbench todo ids == replayed todo ids.
    if let (Some(goal), Some(state_file)) = (
        goal.as_ref(),
        active_state_file(goal_dir, goal.as_ref().map(|g| g.cwd.as_str())),
    ) {
        if let Ok(text) = std::fs::read_to_string(&state_file) {
            let markdown_ids: std::collections::BTreeSet<String> =
                crate::backfill::parse_markdown_todos(&text)
                    .iter()
                    .filter_map(|r| r.todo_id.clone())
                    .collect();
            let replay_ids: std::collections::BTreeSet<String> =
                goal.todos.iter().map(|t| t.id.clone()).collect();
            checks.dual_read_parity_clean = !markdown_ids.is_empty() && markdown_ids == replay_ids;
        }
    }

    checks.rollback_plan_recorded = !store.backups(goal_id).is_empty();
    checks.bounded_canary_passed = false; // never auto-promoted
    checks.public_boundary_clean = goal
        .as_ref()
        .map(crate::projection::privacy::privacy_boundary_clean)
        .unwrap_or(false);
    checks.event_projection_head_matches_store = store
        .raw_ledger_lines(goal_id)
        .map(|lines| !lines.is_empty())
        .unwrap_or(false);

    build_migration_bridge(goal_id, checks, !store.backups(goal_id).is_empty())
}

/// Locate the ACTIVE_GOAL_STATE.md for a goal (project-local state layout:
/// `<cwd>/.future/loop/goals/<id>/ACTIVE_GOAL_STATE.md`).
fn active_state_file(goal_dir: &Path, _cwd: Option<&str>) -> Option<PathBuf> {
    let local = goal_dir.join("ACTIVE_GOAL_STATE.md");
    local.exists().then_some(local)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_get_covers_every_key_and_unknown() {
        let checks = MigrationChecks::default();
        for key in [
            "event_read_path_ready",
            "active_state_projection_ready",
            "dual_read_parity_clean",
            "event_projection_head_matches_store",
            "rollback_plan_recorded",
            "idempotency_conflicts_clean",
            "public_boundary_clean",
            "bounded_canary_passed",
        ] {
            assert_eq!(checks.get(key), Some(false), "{key}");
        }
        assert_eq!(checks.get("bogus"), None);
    }

    #[test]
    fn steps_cover_the_p2_surface_change() {
        let steps = applicable_steps(
            LEGACY_EVENT_STORE_SCHEMA_VERSION,
            EVENT_STORE_SCHEMA_VERSION,
        );
        assert!(!steps.is_empty());
        assert!(steps.iter().any(|s| s.kind == MigrationKind::Envelope));
        assert!(steps.iter().any(|s| s.kind == MigrationKind::AddEventType));
        assert!(steps.iter().any(|s| s.kind == MigrationKind::AddField));
    }

    #[test]
    fn migrate_adds_content_derived_id_to_legacy_line() {
        let mut value = serde_json::json!({
            "kind": "todo_added",
            "goal_id": "g1",
            "todo": {"id": "t1"},
            "ts": 1,
        });
        migrate_event_line(
            &mut value,
            LEGACY_EVENT_STORE_SCHEMA_VERSION,
            EVENT_STORE_SCHEMA_VERSION,
        )
        .unwrap();
        let id = value.get("event_id").and_then(|v| v.as_str()).unwrap();
        assert!(id.starts_with("evt-"));
        assert_eq!(id.len(), 4 + 16);
        // Idempotent: re-running leaves the id untouched.
        let before = value.clone();
        migrate_event_line(
            &mut value,
            LEGACY_EVENT_STORE_SCHEMA_VERSION,
            EVENT_STORE_SCHEMA_VERSION,
        )
        .unwrap();
        assert_eq!(value, before);
    }

    #[test]
    fn bridge_is_fail_closed_without_prerequisites() {
        let checks = MigrationChecks::default();
        let bridge = build_migration_bridge("g1", checks, false);
        assert_eq!(bridge.stage, "wait_for_event_read_path");
        assert!(!bridge.promotion_allowed);
        assert!(!bridge.promotion_candidate);
        assert_eq!(bridge.missing_for_shadow.len(), 2);
    }

    #[test]
    fn bridge_never_promotes_even_when_all_checks_pass() {
        let checks = MigrationChecks {
            event_read_path_ready: true,
            active_state_projection_ready: true,
            dual_read_parity_clean: true,
            rollback_plan_recorded: true,
            bounded_canary_passed: true,
            idempotency_conflicts_clean: true,
            public_boundary_clean: true,
            event_projection_head_matches_store: true,
        };
        let bridge = build_migration_bridge("g1", checks, true);
        assert_eq!(bridge.stage, "promotion_candidate");
        assert!(bridge.promotion_candidate);
        assert!(!bridge.promotion_allowed, "promotion stays fail-closed");
    }

    #[test]
    fn write_path_migration_is_non_destructive_and_reversible() {
        let dir = tempfile::tempdir().unwrap();
        let events = dir.path().join("events.jsonl");
        std::fs::write(
            &events,
            "{\"kind\":\"goal_started\",\"goal_id\":\"g1\",\"ts\":1}\n",
        )
        .unwrap();
        let report = apply_migrations(dir.path(), "g1").unwrap();
        assert_eq!(report.from, LEGACY_EVENT_STORE_SCHEMA_VERSION);
        assert_eq!(report.to, EVENT_STORE_SCHEMA_VERSION);
        assert_eq!(report.migrated_lines, 1);
        assert!(report.non_destructive);
        // Backup exists (rollback plan) and is byte-identical to the original.
        let backup = std::fs::read_to_string(&report.backup_path).unwrap();
        assert_eq!(
            backup,
            "{\"kind\":\"goal_started\",\"goal_id\":\"g1\",\"ts\":1}\n"
        );
        // Ledger now carries an id.
        let migrated = std::fs::read_to_string(&events).unwrap();
        assert!(migrated.contains("\"event_id\":\"evt-"));
        // Stamp bumped.
        assert_eq!(
            read_schema_version(&dir.path().join("schema.json")).unwrap(),
            EVENT_STORE_SCHEMA_VERSION
        );
        // Re-running refuses (already current).
        assert!(apply_migrations(dir.path(), "g1").is_err());
    }
}
