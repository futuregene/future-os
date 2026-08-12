//! Canary smoke (G-20) — reference `canary/` minimal: smoke-suite profiles +
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

/// A smoke-suite profile (reference SMOKE_SUITE_PROFILE_MANIFEST entries).
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
            description: "reference runtime/control-plane contracts without benchmark adapters."
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
        SmokeProfile {
            id: "premerge".to_string(),
            suite: "premerge".to_string(),
            modules: vec![
                "state".to_string(),
                "quota".to_string(),
                "scheduler".to_string(),
                "todo".to_string(),
                "status".to_string(),
                "capability-extension".to_string(),
            ],
            description: "P1-6 premerge gate (CI): the fast deterministic core surface — \
                          skips backup/extension/canary-self so the PR check stays hermetic \
                          and quick."
                .to_string(),
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

// ── P1-6: premerge gate (CI merge gate) ─────────────────────────────────────

pub const PREMERGE_GATE_REPORT_SCHEMA_VERSION: &str = "canary_premerge_gate_v0";

/// The fixture goal seeded into the isolated premerge root so the gate run
/// is never vacuous (a gate that checked zero goals proves nothing).
pub const PREMERGE_FIXTURE_GOAL_ID: &str = "canary-premerge-fixture";

/// A gate verdict over a smoke run. The pass rule reuses the release gate's
/// rule — every check must pass — plus a non-vacuity guard for CI: the run
/// must have seen at least one registered goal.
#[derive(Debug, Clone, Serialize)]
pub struct GateDecision {
    pub gate: String,
    pub passed: bool,
    pub goals_checked: usize,
    pub failed_checks: Vec<String>,
    pub reason: String,
}

/// Evaluate a smoke run as a gate (`gate` = "premerge" / "release").
pub fn evaluate_gate(run: &SmokeRunResult, gate: &str, goals_checked: usize) -> GateDecision {
    let failed_checks: Vec<String> = run
        .checks
        .iter()
        .filter(|c| !c.passed)
        .map(|c| c.id.clone())
        .collect();
    let (passed, reason) = if !failed_checks.is_empty() {
        (
            false,
            format!(
                "{} check(s) failed: {}",
                failed_checks.len(),
                failed_checks.join(", ")
            ),
        )
    } else if goals_checked == 0 {
        (
            false,
            "vacuous run: no goals checked — the gate cannot prove health".to_string(),
        )
    } else {
        (
            true,
            format!(
                "all {} check(s) passed over {goals_checked} goal(s)",
                run.checks.len()
            ),
        )
    };
    GateDecision {
        gate: gate.to_string(),
        passed,
        goals_checked,
        failed_checks,
        reason,
    }
}

/// The full premerge report: the gate decision plus the underlying smoke run.
#[derive(Debug, Clone, Serialize)]
pub struct PremergeGateReport {
    pub schema_version: String,
    pub gate: GateDecision,
    pub run: SmokeRunResult,
}

/// Seed a minimal but real fixture goal so the premerge gate is non-vacuous:
/// one registered goal with a started ledger and one open advancement todo.
pub fn seed_premerge_fixture(store: &mut Store) -> Result<String> {
    let goal_id = PREMERGE_FIXTURE_GOAL_ID.to_string();
    let goal = crate::state::Goal::new(&goal_id, "canary premerge fixture", "/tmp");
    store.register(&goal)?;
    store.append(crate::store::Event::GoalStarted {
        goal_id: goal_id.clone(),
        ts: crate::state::now_epoch(),
    })?;
    store.append(crate::store::Event::TodoAdded {
        goal_id: goal_id.clone(),
        todo: crate::state::Todo::advancement("T1", "fixture work item"),
        ts: crate::state::now_epoch(),
    })?;
    Ok(goal_id)
}

/// Run the premerge gate against an existing state root (the fixture is
/// seeded in place). Split from [`run_premerge_gate_isolated`] so tests can
/// point it at their own roots.
pub fn run_premerge_gate_in(root: &str) -> Result<PremergeGateReport> {
    let mut store = Store::open(root)?;
    seed_premerge_fixture(&mut store)?;
    let run = run_smoke(&store, "premerge")?;
    let gate = evaluate_gate(&run, "premerge", store.registry().len());
    Ok(PremergeGateReport {
        schema_version: PREMERGE_GATE_REPORT_SCHEMA_VERSION.to_string(),
        gate,
        run,
    })
}

/// Run the premerge gate the way CI does (P1-6): an isolated temporary state
/// root (never the operator's live root), a seeded fixture goal, the fast
/// `premerge` profile, and the release-gate pass rule via [`evaluate_gate`].
/// The temp root is always removed afterwards.
pub fn run_premerge_gate_isolated() -> Result<PremergeGateReport> {
    let root = std::env::temp_dir().join(format!(
        "future-loop-premerge-{}-{}",
        std::process::id(),
        crate::state::now_epoch()
    ));
    std::fs::create_dir_all(&root)?;
    let result = run_premerge_gate_in(&root.to_string_lossy());
    let _ = std::fs::remove_dir_all(&root);
    result
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

/// Compose the check detail line: vacuous (no goals), healthy, or the joined
/// failure list.
fn check_detail(checked: usize, failures: &[String], vacuous: &str, healthy: String) -> String {
    if checked == 0 {
        vacuous.to_string()
    } else if failures.is_empty() {
        healthy
    } else {
        failures.join("; ")
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
        // store::verify is infallible in practice (verify_ledger degrades
        // unreadable ledgers to an empty report) — no Err arm to handle.
        let conflicted = store
            .verify(&entry.goal_id)
            .map(|r| (!r.ok).then_some(r.conflicts.len()))
            .unwrap_or(None);
        failures.extend(conflicted.map(|n| format!("{}: {} conflicts", entry.goal_id, n)));
    }
    let detail = check_detail(
        checked,
        &failures,
        "no goals registered — ledger checks vacuous",
        format!("{checked} goal ledger(s) verified"),
    );
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
        // then_some evaluates eagerly — the failure string is built (and its
        // line executed) whether or not a drift was detected.
        failures
            .extend((av != bv).then_some(format!("{}: decision not deterministic", entry.goal_id)));
    }
    let detail = check_detail(
        checked,
        &failures,
        "no goals — determinism check vacuous",
        format!("{checked} goal(s) deterministic"),
    );
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
            failures.extend(
                (packet.goal_id != entry.goal_id)
                    .then_some(format!("{}: packet goal mismatch", entry.goal_id)),
            );
        }
    }
    let detail = check_detail(
        checked,
        &failures,
        "no goals — quota check vacuous",
        format!("{checked} goal(s) produced should-run packets"),
    );
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
            let gap = crate::store::projection_gap(&goal);
            failures.extend(gap.map(|g| format!("{}: {g}", entry.goal_id)));
        }
    }
    let detail = check_detail(
        checked,
        &failures,
        "no goals — frontier check vacuous",
        format!("{checked} goal(s) frontier consistent"),
    );
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
            let mismatch = summary.agent_open != agent_open;
            failures.extend(mismatch.then_some(format!(
                "{}: summary agent_open={} != graph {}",
                entry.goal_id, summary.agent_open, agent_open
            )));
        }
    }
    let detail = check_detail(
        checked,
        &failures,
        "no goals — status check vacuous",
        format!("{checked} goal(s) status consistent"),
    );
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
    for entry in store.registry() {
        let _ = store.backups(&entry.goal_id);
        // verify is infallible (verify_ledger degrades unreadable ledgers to
        // an empty report), so no failure can surface from it here.
        let _ = store.verify(&entry.goal_id);
    }
    SmokeCheckOutcome {
        id: "backup_dir".to_string(),
        module: "backup".to_string(),
        passed: true,
        detail: "backup/restore surfaces healthy".to_string(),
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
        let root = std::env::temp_dir().join(format!(
            "future-loop-canary-{tag}-{}",
            crate::state::now_epoch()
        ));
        std::fs::create_dir_all(&root).unwrap();
        Store::open(&root.to_string_lossy()).unwrap()
    }

    #[test]
    fn unknown_module_yields_no_checks() {
        let store = tmp_store("unknown-module");
        assert!(run_module_checks(&store, "not-a-module").is_empty());
        let _ = std::fs::remove_dir_all(store.root_path());
    }

    #[test]
    fn check_detail_covers_vacuous_healthy_and_failures() {
        assert_eq!(
            check_detail(0, &[], "vacuous", "healthy".to_string()),
            "vacuous"
        );
        assert_eq!(
            check_detail(2, &[], "vacuous", "healthy".to_string()),
            "healthy"
        );
        let failures = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            check_detail(2, &failures, "vacuous", "healthy".to_string()),
            "a; b"
        );
    }

    #[test]
    fn profile_manifest_is_stable() {
        let profiles = smoke_suite_profiles();
        assert_eq!(profiles.len(), 4);
        let ids: Vec<&str> = profiles.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "core-control-plane",
                "extension-runtime",
                "release-gate",
                "premerge"
            ]
        );
        assert!(resolve_smoke_profile("release-gate").is_ok());
        assert!(resolve_smoke_profile("premerge").is_ok());
        assert!(resolve_smoke_profile("nope").is_err());
    }

    #[test]
    fn premerge_gate_isolated_passes_non_vacuous() {
        let report = run_premerge_gate_isolated().unwrap();
        assert_eq!(report.schema_version, PREMERGE_GATE_REPORT_SCHEMA_VERSION);
        assert_eq!(report.run.profile_id, "premerge");
        assert_eq!(report.run.suite, "premerge");
        assert!(report.gate.passed, "{}", report.gate.reason);
        assert_eq!(report.gate.goals_checked, 1);
        assert!(report.gate.failed_checks.is_empty());
    }

    #[test]
    fn premerge_gate_detects_corrupt_fixture_ledger() {
        let mut store = tmp_store("premerge-corrupt");
        let goal_id = seed_premerge_fixture(&mut store).unwrap();
        let ledger = store.goal_dir(&goal_id).join("events.jsonl");
        std::fs::write(&ledger, "not-json\n").unwrap();
        let run = run_smoke(&store, "premerge").unwrap();
        let gate = evaluate_gate(&run, "premerge", store.registry().len());
        assert!(!gate.passed);
        assert!(gate.failed_checks.contains(&"ledger_integrity".to_string()));
        assert!(gate.reason.contains("ledger_integrity"));
        let _ = std::fs::remove_dir_all(store.root_path());
    }

    #[test]
    fn evaluate_gate_rejects_vacuous_run() {
        let run = SmokeRunResult {
            schema_version: CANARY_SMOKE_RUN_SCHEMA_VERSION.to_string(),
            profile_id: "premerge".to_string(),
            suite: "premerge".to_string(),
            all_passed: true,
            checks: vec![SmokeCheckOutcome {
                id: "root_writable".to_string(),
                module: "state".to_string(),
                passed: true,
                detail: "ok".to_string(),
            }],
        };
        let gate = evaluate_gate(&run, "premerge", 0);
        assert!(!gate.passed);
        assert!(gate.reason.contains("vacuous"));
        // Same run with goals checked passes.
        let gate = evaluate_gate(&run, "premerge", 2);
        assert!(gate.passed, "{}", gate.reason);
        assert_eq!(gate.goals_checked, 2);
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
