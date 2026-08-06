//! G-18 benchmark contract tests — the minimal closed loop, deterministic:
//! loop protocol classification, ledger compaction + idempotent persistence,
//! adapter round accounting, and the qualification single-scenario runner.
//! The gRPC adapter itself is a thin transport over the already-tested
//! AgentClient; everything policy-shaped is exercised with the scripted
//! adapter.

use std::path::PathBuf;

use future_loop::benchmark::adapter::{
    run_round, BenchmarkAdapter, BenchmarkRequest, ScriptedAdapter,
};
use future_loop::benchmark::ledger::{
    build_benchmark_run_ledger_entry, classify_failure, derive_benchmark_run_id, BenchmarkLedger,
    BenchmarkRun, RoundRewardRecord, RoundRewardTrace,
};
use future_loop::benchmark::loop_protocol::{
    build_benchmark_loop_contract, build_product_mode_main_table_comparison_contract,
    CODEX_ACP_BLIND_LOOP_BASELINE_ROUTE, LOOPX_PACKET_ONLY_OBSERVATION_ROUTE,
    LOOPX_PRODUCT_MODE_ROUTE, MAX5_BLIND_LOOP_NO_FEEDBACK_PROTOCOL_ID,
    PACKET_ONLY_OBSERVATION_PROTOCOL_ID, PRODUCT_MODE_MAX5_NO_FEEDBACK_PROTOCOL_ID,
};
use future_loop::benchmark::qualification::{run_qualification_case, QualificationCase};

fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "future-loop-p4-bench-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ── loop protocol ─────────────────────────────────────────────────────────

#[test]
fn loop_protocol_classifies_routes() {
    let product = build_benchmark_loop_contract(LOOPX_PRODUCT_MODE_ROUTE, None, None);
    assert_eq!(
        product.protocol_id,
        PRODUCT_MODE_MAX5_NO_FEEDBACK_PROTOCOL_ID
    );
    assert!(product.product_mode);
    assert!(!product.blind_loop);
    assert!(product.strict_treatment_claim_allowed);
    assert_eq!(product.max_rounds_budget, 5);

    let blind = build_benchmark_loop_contract(CODEX_ACP_BLIND_LOOP_BASELINE_ROUTE, None, None);
    assert_eq!(blind.protocol_id, MAX5_BLIND_LOOP_NO_FEEDBACK_PROTOCOL_ID);
    assert!(blind.blind_loop);
    assert!(!blind.strict_treatment_claim_allowed);

    let packet_only =
        build_benchmark_loop_contract(LOOPX_PACKET_ONLY_OBSERVATION_ROUTE, None, None);
    assert_eq!(packet_only.protocol_id, PACKET_ONLY_OBSERVATION_PROTOCOL_ID);
    assert_eq!(packet_only.claim_blocker, "packet_only_no_max5_controller");
    assert!(!packet_only.strict_treatment_claim_allowed);
}

#[test]
fn product_mode_comparison_contract_is_arms_and_gate() {
    let c = build_product_mode_main_table_comparison_contract(
        "skillsbench@1.1",
        None,
        "raw-codex-autonomous-max5",
        LOOPX_PRODUCT_MODE_ROUTE,
    );
    assert_eq!(c.comparison_id, "skillsbench_product_mode_main_table_v0");
    assert_eq!(c.baseline_arm["route"], "raw-codex-autonomous-max5");
    assert_eq!(c.treatment_arm["arm_id"], "future_loop_product_mode");
    assert!(c.policy_gate["same_benchmark_and_case_required"]
        .as_bool()
        .unwrap());
    assert_eq!(c.policy_gate["headline_metrics"][0], "best_score");
}

// ── ledger ────────────────────────────────────────────────────────────────

fn sample_run() -> BenchmarkRun {
    BenchmarkRun {
        benchmark_id: "skillsbench@1.1".to_string(),
        case_ids: vec!["case-42".to_string()],
        arm_id: "future_loop_product_mode".to_string(),
        route: LOOPX_PRODUCT_MODE_ROUTE.to_string(),
        mode: "product".to_string(),
        agent_model: "future/deepseek-v4-flash".to_string(),
        job_name: "job-1".to_string(),
        terminal_status: "completed".to_string(),
        round_reward_trace: RoundRewardTrace::from_records(
            vec![
                RoundRewardRecord {
                    agent_round: 1,
                    passed: false,
                    reward: 0.0,
                },
                RoundRewardRecord {
                    agent_round: 2,
                    passed: true,
                    reward: 1.0,
                },
            ],
            5,
        ),
        notes: String::new(),
    }
}

#[test]
fn ledger_entry_compacts_headline_surface() {
    let entry = build_benchmark_run_ledger_entry(&sample_run(), 1_700_000_000);
    assert_eq!(entry.schema_version, "benchmark_run_ledger_v0");
    assert_eq!(entry.run_id, derive_benchmark_run_id(&sample_run()));
    assert_eq!(entry.first_success_round, Some(2));
    assert_eq!(entry.best_round_reward, 1.0);
    assert_eq!(entry.score, 1.0);
    assert!(entry.passed);
    assert_eq!(entry.failure_class, "success");
    assert_eq!(entry.agent_model, "future/deepseek-v4-flash");
}

#[test]
fn ledger_persists_and_dedupes_by_run_id() {
    let dir = tmp_dir("persist");
    let mut ledger = BenchmarkLedger::open(&dir).unwrap();
    let entry = build_benchmark_run_ledger_entry(&sample_run(), 1);
    assert!(ledger.append(entry.clone()).unwrap());
    assert!(!ledger.append(entry).unwrap(), "duplicate identity deduped");
    let reopened = BenchmarkLedger::open(&dir).unwrap();
    assert_eq!(reopened.entries().len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ledger_classifies_failure_modes() {
    let mut run = sample_run();
    run.terminal_status = "runner_error".to_string();
    assert_eq!(
        classify_failure(&run),
        ("runner_error".to_string(), "runner".to_string())
    );
    let mut run = sample_run();
    run.round_reward_trace = RoundRewardTrace::from_records(
        vec![RoundRewardRecord {
            agent_round: 1,
            passed: false,
            reward: 0.0,
        }],
        1,
    );
    assert_eq!(classify_failure(&run).0, "budget_exhausted");
    let mut run = sample_run();
    run.round_reward_trace = RoundRewardTrace::from_records(
        vec![RoundRewardRecord {
            agent_round: 1,
            passed: false,
            reward: 0.0,
        }],
        5,
    );
    assert_eq!(classify_failure(&run).0, "case_failure");
}

// ── adapter (scripted, deterministic) ─────────────────────────────────────

#[test]
fn scripted_adapter_round_classifies_and_ledgers() {
    let dir = tmp_dir("adapter");
    let mut adapter = ScriptedAdapter::new(vec!["completed".to_string()]);
    let request = BenchmarkRequest::new(
        "skillsbench@1.1",
        "case-1",
        LOOPX_PRODUCT_MODE_ROUTE,
        "fizzbuzz",
    );
    let pre = adapter.preflight(&request).unwrap();
    assert!(pre.ok);
    let (classification, update) =
        run_round(&mut adapter, &request, Some(&dir), 1_700_000_000).unwrap();
    assert_eq!(classification.decision, "passed");
    let entry = update.entry.unwrap();
    assert_eq!(entry.case_ids, vec!["case-1"]);
    assert_eq!(entry.score, 1.0);
    let ledger = BenchmarkLedger::open(&dir).unwrap();
    assert_eq!(ledger.entries().len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scripted_adapter_runner_error_fails_closed() {
    let mut adapter = ScriptedAdapter::new(vec!["error".to_string()]);
    let request = BenchmarkRequest::new("b", "c", LOOPX_PRODUCT_MODE_ROUTE, "task");
    let (classification, _) = run_round(&mut adapter, &request, None, 1).unwrap();
    assert_eq!(classification.decision, "runner_error");
    assert!(!classification.passed);
}

#[test]
fn expected_evidence_gate_fails_closed() {
    let mut adapter = ScriptedAdapter::new(vec!["completed".to_string()]);
    let mut request = BenchmarkRequest::new("b", "c", LOOPX_PRODUCT_MODE_ROUTE, "task");
    request.expected_evidence = Some("absent-marker".to_string());
    let (classification, _) = run_round(&mut adapter, &request, None, 1).unwrap();
    assert!(!classification.passed);
}

// ── qualification single scenario ─────────────────────────────────────────

#[test]
fn qualification_stops_on_first_pass() {
    let mut adapter = ScriptedAdapter::new(vec!["completed".to_string(), "error".to_string()]);
    let case = QualificationCase::new("skillsbench@1.1", "case-1", "fizzbuzz", 5);
    let result = run_qualification_case(&mut adapter, &case, None).unwrap();
    assert!(result.passed);
    assert_eq!(result.rounds_used, 1);
    assert_eq!(result.headline.first_success_round, Some(1));
    assert_eq!(result.headline.best_score, 1.0);
    assert_eq!(result.failure_class, "success");
    assert_eq!(adapter.round, 1, "loop stops after first pass");
}

#[test]
fn qualification_exhausts_budget_without_pass() {
    let mut adapter = ScriptedAdapter::new(vec!["cancelled".to_string()]);
    let case = QualificationCase::new("b", "c", "hard task", 3);
    let result = run_qualification_case(&mut adapter, &case, None).unwrap();
    assert!(!result.passed);
    assert_eq!(result.rounds_used, 3);
    assert_eq!(result.failure_class, "budget_exhausted");
}

#[test]
fn qualification_runner_error_aborts() {
    let mut adapter = ScriptedAdapter::new(vec!["error".to_string()]);
    let case = QualificationCase::new("b", "c", "task", 5);
    let result = run_qualification_case(&mut adapter, &case, None).unwrap();
    assert_eq!(result.rounds_used, 1);
    assert_eq!(result.failure_class, "runner_error");
    assert_eq!(result.failure_scope, "runner");
}

#[test]
fn qualification_writes_idempotent_ledger_entries() {
    let dir = tmp_dir("qual");
    let mut adapter = ScriptedAdapter::new(vec!["completed".to_string()]);
    let case = QualificationCase::new("b", "c", "task", 5);
    let result = run_qualification_case(&mut adapter, &case, Some(&dir)).unwrap();
    assert!(result.passed);
    let ledger = BenchmarkLedger::open(&dir).unwrap();
    assert_eq!(ledger.entries().len(), 1);
    // re-running the identical case appends nothing
    let mut adapter2 = ScriptedAdapter::new(vec!["completed".to_string()]);
    run_qualification_case(&mut adapter2, &case, Some(&dir)).unwrap();
    let ledger = BenchmarkLedger::open(&dir).unwrap();
    assert_eq!(ledger.entries().len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}
