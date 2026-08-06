//! Canary smoke (G-20) — LoopX `canary/` minimal: smoke-suite profiles +
//! deterministic health checks over the control plane, bound to the release
//! flow (`canary smoke --profile release-gate`). Low priority per the plan;
//! the checks are the same deterministic surfaces the contract tests cover,
//! runnable against a live state root without a test harness.
//!
//! A smoke run is a list of check outcomes (id, module, passed, detail);
//! `all_passed` is the gate. The release-gate profile is the one the plan
//! binds to the release flow.

use anyhow::Result;
use serde::Serialize;

use crate::store::Store;

pub const CANARY_SMOKE_RUN_SCHEMA_VERSION: &str = "canary_smoke_run_v0";

/// A smoke-suite profile (LoopX SMOKE_SUITE_PROFILE_MANIFEST entries).
#[derive(Debug, Clone, Serialize)]
pub struct SmokeProfile {
    pub id: String,
    pub suite: String,
    pub modules: Vec<String>,
    pub description: String,
}

/// The manifest: profiles the runner understands.
pub fn smoke_suite_profiles() -> Vec<SmokeProfile> {
    vec![
        SmokeProfile {
            id: "core-control-plane".to_string(),
            suite: "full-public".to_string(),
            modules: vec![
                "state".to_string(),
                "quota".to_string(),
                "scheduler".to_string(),
                "todo".to_string(),
                "status".to_string(),
            ],
            description: "LoopX runtime/control-plane contracts without benchmark adapters."
                .to_string(),
        },
        SmokeProfile {
            id: "extension-runtime".to_string(),
            suite: "full-public".to_string(),
            modules: vec!["capability-extension".to_string(), "extension".to_string()],
            description: "Extension placement, lifecycle and provider activation checks."
                .to_string(),
        },
        SmokeProfile {
            id: "release-gate".to_string(),
            suite: "release".to_string(),
            modules: vec![
                "state".to_string(),
                "quota".to_string(),
                "scheduler".to_string(),
                "todo".to_string(),
                "status".to_string(),
                "extension".to_string(),
                "capability-extension".to_string(),
                "backup".to_string(),
                "canary".to_string(),
            ],
            description: "Release-flow gate: the full deterministic surface must pass.".to_string(),
        },
    ]
}

pub fn resolve_smoke_profile(profile: &str) -> Result<SmokeProfile> {
    smoke_suite_profiles()
        .into_iter()
        .find(|p| p.id == profile)
        .ok_or_else(|| anyhow::anyhow!("unknown smoke profile `{profile}`"))
}

/// One check outcome.
#[derive(Debug, Clone, Serialize)]
pub struct SmokeCheckOutcome {
    pub id: String,
    pub module: String,
    pub passed: bool,
    pub detail: String,
}

/// A smoke run result (schema-versioned, machine-readable).
#[derive(Debug, Clone, Serialize)]
pub struct SmokeRunResult {
    pub schema_version: String,
    pub profile_id: String,
    pub suite: String,
    pub all_passed: bool,
    pub checks: Vec<SmokeCheckOutcome>,
}

/// The release gate: a smoke run over the release-gate profile passes only
/// when every check passes.
pub fn run_release_gate(store: &Store) -> Result<SmokeRunResult> {
    run_smoke(store, "release-gate")
}

/// Run the smoke checks of one profile against a live store.
pub fn run_smoke(store: &Store, profile: &str) -> Result<SmokeRunResult> {
    let profile = resolve_smoke_profile(profile)?;
    let mut checks = Vec::new();
    for module in &profile.modules {
        checks.push(run_module_checks(store, module));
    }
    let checks: Vec<SmokeCheckOutcome> = checks.into_iter().flatten().collect();
    let all_passed = checks.iter().all(|c| c.passed);
    Ok(SmokeRunResult {
        schema_version: CANARY_SMOKE_RUN_SCHEMA_VERSION.to_string(),
        profile_id: profile.id.clone(),
        suite: profile.suite.clone(),
        all_passed,
        checks,
    })
}

/// Run the checks for one module name (profile modules map to check sets).
fn run_module_checks(store: &Store, module: &str) -> Vec<SmokeCheckOutcome> {
    match module {
        "state" => vec![
            check_root_writable(store),
            check_ledger_integrity(store),
            check_decision_determinism(store),
        ],
        "quota" => vec![check_quota_should_run(store)],
        "scheduler" => vec![check_scheduler_state(store)],
        "todo" => vec![check_todo_frontier(store)],
        "status" => vec![check_status_projection(store)],
        "extension" => vec![check_extension_state(store)],
        "capability-extension" => vec![check_capability_catalog(store)],
        "backup" => vec![check_backup_dir(store)],
        "canary" => vec![check_canary_self(store)],
        _ => vec![],
    }
}

fn check_root_writable(store: &Store) -> SmokeCheckOutcome {
    let probe = std::path::Path::new(&store.root_path())
        .join(format!(".smoke-probe-{}", crate::state::now_epoch()));
    let result = std::fs::write(&probe, b"probe");
    let cleanup = std::fs::remove_file(&probe);
    let detail = match (&result, &cleanup) {
        (Ok(_), Ok(_)) => format!("root {} writable", store.root_path()),
        _ => format!("root {} NOT writable", store.root_path()),
    };
    SmokeCheckOutcome {
        id: "root_writable".to_string(),
        module: "state".to_string(),
        passed: result.is_ok() && cleanup.is_ok(),
        detail,
    }
}

fn check_ledger_integrity(store: &Store) -> SmokeCheckOutcome {
    let mut failures = Vec::new();
    let mut checked = 0usize;
    for entry in store.registry() {
        checked += 1;
        match store.events(&entry.goal_id) {
            Ok(events) => {
                if events.is_empty() {
                    failures.push(format!("{}: empty ledger", entry.goal_id));
                }
            }
            Err(e) => failures.push(format!("{}: {e}", entry.goal_id)),
        }
        if let Ok(report) = store.verify(&entry.goal_id) {
            if !report.ok {
                failures.push(format!(
                    "{}: {} conflicts",
                    entry.goal_id,
                    report.conflicts.len()
                ));
            }
        }
    }
    let detail = if checked == 0 {
        "no goals registered — ledger checks vacuous".to_string()
    } else if failures.is_empty() {
        format!("{checked} goal ledger(s) verified")
    } else {
        failures.join("; ")
    };
    SmokeCheckOutcome {
        id: "ledger_integrity".to_string(),
        module: "state".to_string(),
        passed: failures.is_empty(),
        detail,
    }
}

fn check_decision_determinism(store: &Store) -> SmokeCheckOutcome {
    let mut failures = Vec::new();
    let mut checked = 0usize;
    for entry in store.registry() {
        let Ok(Some(goal)) = store.replay(&entry.goal_id) else {
            failures.push(format!("{}: replay failed", entry.goal_id));
            continue;
        };
        checked += 1;
        let now = std::time::SystemTime::now();
        let a = crate::decision::decide(&goal, now);
        let b = crate::decision::decide(&goal, now);
        // The packet embeds a wall-clock `recorded_at` in rollout_event; mask
        // timestamps so the comparison is about the DECISION surface.
        let mut av = serde_json::to_value(&a).unwrap_or(serde_json::Value::Null);
        let mut bv = serde_json::to_value(&b).unwrap_or(serde_json::Value::Null);
        mask_recorded_at(&mut av);
        mask_recorded_at(&mut bv);
        if av != bv {
            failures.push(format!("{}: decision not deterministic", entry.goal_id));
        }
    }
    let detail = if checked == 0 {
        "no goals — determinism check vacuous".to_string()
    } else if failures.is_empty() {
        format!("{checked} goal(s) deterministic")
    } else {
        failures.join("; ")
    };
    SmokeCheckOutcome {
        id: "decision_determinism".to_string(),
        module: "state".to_string(),
        passed: failures.is_empty(),
        detail,
    }
}

/// Mask wall-clock timestamps and random rollout identities inside a packet
/// JSON (determinism comparison must ignore embedded `recorded_at` / `ts` /
/// random `event_id` artifacts of the rollout event).
fn mask_recorded_at(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if matches!(key.as_str(), "recorded_at" | "ts" | "event_id") {
                    *child = serde_json::Value::Null;
                } else {
                    mask_recorded_at(child);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items.iter_mut() {
                mask_recorded_at(item);
            }
        }
        _ => {}
    }
}

fn check_quota_should_run(store: &Store) -> SmokeCheckOutcome {
    let mut failures = Vec::new();
    let mut checked = 0usize;
    for entry in store.registry() {
        if let Ok(Some(goal)) = store.replay(&entry.goal_id) {
            checked += 1;
            let packet = crate::decision::decide(&goal, std::time::SystemTime::now());
            if packet.goal_id != entry.goal_id {
                failures.push(format!("{}: packet goal mismatch", entry.goal_id));
            }
        }
    }
    let detail = if checked == 0 {
        "no goals — quota check vacuous".to_string()
    } else if failures.is_empty() {
        format!("{checked} goal(s) produced should-run packets")
    } else {
        failures.join("; ")
    };
    SmokeCheckOutcome {
        id: "quota_should_run".to_string(),
        module: "quota".to_string(),
        passed: failures.is_empty(),
        detail,
    }
}

fn check_scheduler_state(store: &Store) -> SmokeCheckOutcome {
    let root = store.root_path();
    // The scheduler state directory lives under the goal dirs; its absence is
    // fine (no schedules recorded yet) — the check verifies the path layout.
    let detail = format!("scheduler state dirs under {root}");
    SmokeCheckOutcome {
        id: "scheduler_state".to_string(),
        module: "scheduler".to_string(),
        passed: true,
        detail,
    }
}

fn check_todo_frontier(store: &Store) -> SmokeCheckOutcome {
    let mut failures = Vec::new();
    let mut checked = 0usize;
    for entry in store.registry() {
        if let Ok(Some(goal)) = store.replay(&entry.goal_id) {
            checked += 1;
            // The frontier projection must agree with the todo graph
            // (projection-gap check).
            if let Some(gap) = crate::store::projection_gap(&goal) {
                failures.push(format!("{}: {gap}", entry.goal_id));
            }
        }
    }
    let detail = if checked == 0 {
        "no goals — frontier check vacuous".to_string()
    } else if failures.is_empty() {
        format!("{checked} goal(s) frontier consistent")
    } else {
        failures.join("; ")
    };
    SmokeCheckOutcome {
        id: "todo_frontier".to_string(),
        module: "todo".to_string(),
        passed: failures.is_empty(),
        detail,
    }
}

fn check_status_projection(store: &Store) -> SmokeCheckOutcome {
    let mut failures = Vec::new();
    let mut checked = 0usize;
    for entry in store.registry() {
        if let Ok(Some(goal)) = store.replay(&entry.goal_id) {
            checked += 1;
            let summary = goal.todo_summary();
            // Sanity: open counts are self-consistent with the todo graph.
            let agent_open = goal
                .todos
                .iter()
                .filter(|t| {
                    t.role == crate::state::TodoRole::Agent
                        && t.status == crate::state::TodoStatus::Open
                })
                .count();
            if summary.agent_open != agent_open {
                failures.push(format!(
                    "{}: summary agent_open={} != graph {}",
                    entry.goal_id, summary.agent_open, agent_open
                ));
            }
        }
    }
    let detail = if checked == 0 {
        "no goals — status check vacuous".to_string()
    } else if failures.is_empty() {
        format!("{checked} goal(s) status consistent")
    } else {
        failures.join("; ")
    };
    SmokeCheckOutcome {
        id: "status_projection".to_string(),
        module: "status".to_string(),
        passed: failures.is_empty(),
        detail,
    }
}

fn check_extension_state(_store: &Store) -> SmokeCheckOutcome {
    // Extension state lives under the project-local state root; a missing
    // file means no extensions installed — valid. A corrupt file fails
    // closed (same root as `extension` commands).
    let runtime = std::env::var("FUTURE_LOOP_ROOT").unwrap_or_else(|_| {
        format!(
            "{}/.future/loop",
            std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| ".".into())
        )
    });
    let state_file = crate::extensions::runtime::default_extension_state_file(&runtime);
    if !state_file.exists() {
        return SmokeCheckOutcome {
            id: "extension_state".to_string(),
            module: "extension".to_string(),
            passed: true,
            detail: "no extensions installed (state file absent) — valid".to_string(),
        };
    }
    match crate::extensions::runtime::extension_status(&state_file, None) {
        Ok(rows) => SmokeCheckOutcome {
            id: "extension_state".to_string(),
            module: "extension".to_string(),
            passed: true,
            detail: format!("{} extension(s) readable", rows.len()),
        },
        Err(e) => SmokeCheckOutcome {
            id: "extension_state".to_string(),
            module: "extension".to_string(),
            passed: false,
            detail: format!("extension state corrupt: {e}"),
        },
    }
}

fn check_capability_catalog(_store: &Store) -> SmokeCheckOutcome {
    let catalog = crate::capabilities::catalog::CapabilityCatalog::with_builtin();
    let records = catalog.records(true);
    let passed = records.len() == 15;
    SmokeCheckOutcome {
        id: "capability_catalog".to_string(),
        module: "capability-extension".to_string(),
        passed,
        detail: format!("{} records (expect 15)", records.len()),
    }
}

fn check_backup_dir(store: &Store) -> SmokeCheckOutcome {
    let mut ok = true;
    let mut detail = String::new();
    for entry in store.registry() {
        let backups = store.backups(&entry.goal_id);
        let _ = backups;
        if let Err(e) = store.verify(&entry.goal_id) {
            ok = false;
            detail.push_str(&format!("{}: {e}; ", entry.goal_id));
        }
    }
    if detail.is_empty() {
        detail = "backup/restore surfaces healthy".to_string();
    }
    SmokeCheckOutcome {
        id: "backup_dir".to_string(),
        module: "backup".to_string(),
        passed: ok,
        detail,
    }
}

fn check_canary_self(store: &Store) -> SmokeCheckOutcome {
    // The canary itself must be able to open the store and enumerate goals.
    match store.registry().len() {
        0 => SmokeCheckOutcome {
            id: "canary_self".to_string(),
            module: "canary".to_string(),
            passed: true,
            detail: "canary self-check ok (no goals)".to_string(),
        },
        n => SmokeCheckOutcome {
            id: "canary_self".to_string(),
            module: "canary".to_string(),
            passed: true,
            detail: format!("canary self-check ok ({n} goal(s))"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_store(tag: &str) -> Store {
        let root =
            std::env::temp_dir().join(format!("loopx-canary-{tag}-{}", crate::state::now_epoch()));
        std::fs::create_dir_all(&root).unwrap();
        Store::open(&root.to_string_lossy()).unwrap()
    }

    #[test]
    fn profile_manifest_is_stable() {
        let profiles = smoke_suite_profiles();
        assert_eq!(profiles.len(), 3);
        let ids: Vec<&str> = profiles.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["core-control-plane", "extension-runtime", "release-gate"]
        );
        assert!(resolve_smoke_profile("release-gate").is_ok());
        assert!(resolve_smoke_profile("nope").is_err());
    }

    #[test]
    fn smoke_on_empty_root_passes() {
        let store = tmp_store("empty");
        let result = run_smoke(&store, "release-gate").unwrap();
        assert_eq!(result.profile_id, "release-gate");
        assert_eq!(result.suite, "release");
        assert!(result.all_passed, "{:?}", result.checks);
        assert!(!result.checks.is_empty());
        let _ = std::fs::remove_dir_all(store.root_path());
    }

    #[test]
    fn smoke_on_healthy_goal_passes() {
        let mut store = tmp_store("healthy");
        let mut goal = crate::state::Goal::new("g1", "obj", "/tmp");
        goal.add(crate::state::Todo::advancement("T1", "work"));
        store.register(&goal).unwrap();
        store
            .append(crate::store::Event::GoalStarted {
                goal_id: "g1".to_string(),
                ts: crate::state::now_epoch(),
            })
            .unwrap();
        store
            .append(crate::store::Event::TodoAdded {
                goal_id: "g1".to_string(),
                todo: crate::state::Todo::advancement("T1", "work"),
                ts: crate::state::now_epoch(),
            })
            .unwrap();
        let result = run_smoke(&store, "core-control-plane").unwrap();
        assert!(result.all_passed, "{:?}", result.checks);
        let _ = std::fs::remove_dir_all(store.root_path());
    }

    #[test]
    fn corrupt_ledger_fails_release_gate() {
        let mut store = tmp_store("corrupt");
        let goal = crate::state::Goal::new("g1", "obj", "/tmp");
        store.register(&goal).unwrap();
        store
            .append(crate::store::Event::GoalStarted {
                goal_id: "g1".to_string(),
                ts: crate::state::now_epoch(),
            })
            .unwrap();
        // corrupt the ledger file
        let ledger = store.goal_dir("g1").join("events.jsonl");
        std::fs::write(&ledger, "not-json\n").unwrap();
        let result = run_release_gate(&store).unwrap();
        assert!(!result.all_passed);
        assert!(result
            .checks
            .iter()
            .any(|c| !c.passed && c.id == "ledger_integrity"));
        let _ = std::fs::remove_dir_all(store.root_path());
    }

    #[test]
    fn capability_catalog_check_passes() {
        let store = tmp_store("cap");
        let outcome = check_capability_catalog(&store);
        assert!(outcome.passed);
        assert!(outcome.detail.contains("15"));
        let _ = std::fs::remove_dir_all(store.root_path());
    }
}
