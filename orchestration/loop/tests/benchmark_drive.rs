//! Coverage drive for `benchmark/`: qualification flow control (bails,
//! budget exhaustion, preflight failures), adapter stage errors, the gRPC
//! adapter against the mock agent, and the run-ledger edges.

mod common;

use common::mock_agent::{completed_events, ev, spawn_mock, MockState};
use future_loop::benchmark::adapter::{
    AdapterClassification, BenchmarkAdapter, GrpcLoopxAdapter, IngestResult, LaunchResult,
    Observation, PreflightResult, RunHandle, ScriptedAdapter,
};
use future_loop::benchmark::ledger::{
    build_benchmark_run_ledger_entry, classify_failure, derive_benchmark_run_id, BenchmarkLedger,
    BenchmarkRun, RoundRewardTrace,
};
use future_loop::benchmark::qualification::{run_qualification_case, QualificationCase};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn case(bench: &str, case: &str, rounds: u32) -> QualificationCase {
    QualificationCase::new(bench, case, "do the task", rounds)
}

// ── qualification flow control ─────────────────────────────────────────────

#[test]
fn qualification_flow_control() {
    // max_rounds == 0 → bail.
    let mut adapter = ScriptedAdapter::new(vec!["completed".to_string()]);
    assert!(run_qualification_case(&mut adapter, &case("b", "c", 0), None).is_err());

    // Preflight returns Err (transport-style failure) → propagates.
    struct PreflightErr;
    impl BenchmarkAdapter for PreflightErr {
        fn id(&self) -> &str {
            "pre-err"
        }
        fn preflight(&mut self, _: &future_loop::benchmark::adapter::BenchmarkRequest) -> anyhow::Result<PreflightResult> {
            anyhow::bail!("preflight exploded")
        }
        fn launch(&mut self, _: &future_loop::benchmark::adapter::BenchmarkRequest) -> anyhow::Result<LaunchResult> {
            unreachable!()
        }
        fn observe(&mut self, _: &RunHandle) -> anyhow::Result<Observation> {
            unreachable!()
        }
        fn ingest(&mut self, _: &future_loop::benchmark::adapter::BenchmarkRequest, _: &RunHandle, _: &Observation) -> anyhow::Result<IngestResult> {
            unreachable!()
        }
        fn classify(&mut self, _: &future_loop::benchmark::adapter::BenchmarkRequest, _: &IngestResult) -> anyhow::Result<AdapterClassification> {
            unreachable!()
        }
        fn ledger(
            &mut self,
            _: &future_loop::benchmark::adapter::BenchmarkRequest,
            _: &IngestResult,
            _: Option<&std::path::Path>,
            _: u64,
        ) -> anyhow::Result<future_loop::benchmark::adapter::LedgerUpdate> {
            unreachable!()
        }
    }
    let mut a = PreflightErr;
    assert!(run_qualification_case(&mut a, &case("b", "c", 2), None).is_err());

    // All rounds fail (non-error terminal → loop runs to budget) → exhausted.
    let mut failing = ScriptedAdapter::new(vec!["incomplete".to_string()]);
    let result = run_qualification_case(&mut failing, &case("b", "c", 2), None).unwrap();
    assert!(!result.passed);
    assert_eq!(result.failure_class, "budget_exhausted");
    assert_eq!(result.rounds_used, 2);
    assert_eq!(result.headline.first_success_round, None);
}

#[test]
fn scripted_adapter_edges() {
    // Empty task → preflight bail.
    let mut a = ScriptedAdapter::new(vec![]);
    let mut c = case("b", "c", 1);
    c.task = "   ".to_string();
    assert!(run_qualification_case(&mut a, &c, None).is_err());
    // Empty script → observe falls back to "completed".
    let mut a = ScriptedAdapter::new(vec![]);
    let result = run_qualification_case(&mut a, &case("b", "c", 1), None).unwrap();
    assert!(result.passed);
    // Script shorter than rounds → the last entry repeats. A non-error
    // failure keeps the loop going; round 2 passes and stops it.
    let mut a = ScriptedAdapter::new(vec!["incomplete".to_string(), "completed".to_string()]);
    let result = run_qualification_case(&mut a, &case("b", "c", 3), None).unwrap();
    assert!(result.passed, "second round passes and stops the loop");
    assert_eq!(result.rounds_used, 2);
    assert_eq!(result.headline.first_success_round, Some(2));
    assert_eq!(result.headline.best_score, 1.0);
    assert_eq!(result.headline.final_score, 1.0);
    assert_eq!(result.headline.declared_done_score, 1.0);
    assert_eq!(result.failure_class, "success");
    assert_eq!(result.failure_scope, "case");
}

// ── ledger ─────────────────────────────────────────────────────────────────

fn run_with(bench: &str, status: &str, passed: bool, final_round: u32, budget: u32) -> BenchmarkRun {
    BenchmarkRun {
        benchmark_id: bench.to_string(),
        case_ids: vec!["c1".to_string()],
        arm_id: "arm".to_string(),
        route: "r".to_string(),
        mode: "product".to_string(),
        agent_model: "m".to_string(),
        job_name: "j".to_string(),
        round_reward_trace: RoundRewardTrace::from_records(
            vec![future_loop::benchmark::ledger::RoundRewardRecord {
                agent_round: final_round,
                passed,
                reward: if passed { 1.0 } else { 0.0 },
            }],
            budget,
        ),
        terminal_status: status.to_string(),
        notes: String::new(),
    }
}

#[test]
fn classify_failure_matrix() {
    assert_eq!(classify_failure(&run_with("b", "runner_error", false, 1, 5)).0, "runner_error");
    assert_eq!(classify_failure(&run_with("b", "aborted", false, 1, 5)).0, "aborted");
    assert_eq!(classify_failure(&run_with("b", "completed", true, 1, 5)).0, "success");
    assert_eq!(
        classify_failure(&run_with("b", "completed", false, 5, 5)).0,
        "budget_exhausted"
    );
    assert_eq!(classify_failure(&run_with("b", "completed", false, 1, 5)).0, "case_failure");
    assert_eq!(classify_failure(&run_with("b", "mystery", false, 1, 5)).0, "unknown");
    // Empty trace → runner_error regardless.
    let mut run = run_with("b", "completed", false, 1, 5);
    run.round_reward_trace = RoundRewardTrace::default();
    assert_eq!(classify_failure(&run).0, "runner_error");
}

#[test]
fn ledger_entry_builder_and_store() {
    // Long notes are compacted with an ellipsis; declared-done builder sets fields.
    let mut run = run_with("bench-x", "completed", true, 1, 5);
    run.notes = "x".repeat(500);
    let entry = build_benchmark_run_ledger_entry(&run, 1_700_000_000);
    assert!(entry.notes.len() < 500, "compacted: {}", entry.notes.len());
    assert!(entry.notes.ends_with('…'));
    assert_eq!(entry.benchmark_id, "bench-x");
    assert!(entry.passed);
    // derive id stable + distinct on content change.
    assert_eq!(derive_benchmark_run_id(&run), derive_benchmark_run_id(&run));
    let other = run_with("bench-y", "completed", true, 1, 5);
    assert_ne!(derive_benchmark_run_id(&run), derive_benchmark_run_id(&other));
    // Trace with declared done.
    let trace = RoundRewardTrace::from_records(vec![], 5).with_declared_done(2, 1.0);
    assert!(trace.agent_declared_done);
    assert_eq!(trace.declared_done_round, Some(2));
    assert_eq!(trace.declared_done_score, Some(1.0));

    // Store: open/append (idempotent dup)/query/aggregate/corrupt line.
    let dir = tempfile::tempdir().unwrap();
    let mut ledger = BenchmarkLedger::open(dir.path()).unwrap();
    assert!(ledger.append(entry.clone()).unwrap());
    assert!(!ledger.append(entry.clone()).unwrap(), "duplicate run_id is a no-op");
    assert_eq!(ledger.entries().len(), 1);
    assert!(!ledger.path().as_os_str().is_empty());
    assert_eq!(ledger.query(Some("bench-x"), Some("c1"), None).len(), 1);
    assert_eq!(ledger.query(Some("other"), None, None).len(), 0);
    assert_eq!(ledger.query(None, Some("c1"), None).len(), 1);
    assert_eq!(ledger.query(None, Some("nope"), None).len(), 0);
    let agg = ledger.aggregate(Some("bench-x"));
    assert_eq!(agg["run_count"], serde_json::json!(1));
    let agg_all = ledger.aggregate(None);
    assert_eq!(agg_all["run_count"], serde_json::json!(1));
    // Corrupt line → open fails with line context.
    std::fs::write(ledger.path(), "{corrupt\n").unwrap();
    let err = BenchmarkLedger::open(dir.path()).unwrap_err();
    assert!(format!("{err:#}").contains("corrupt"), "{err:#}");
}

// ── gRPC adapter against the mock agent ────────────────────────────────────

#[test]
fn grpc_adapter_full_surface() {
    rt().block_on(async {
        let (addr, _shared) = spawn_mock(MockState {
            events: completed_events("mock-run-1"),
            ..Default::default()
        })
        .await;
        let mut adapter = GrpcLoopxAdapter::connect(&addr, "/tmp")
            .await
            .unwrap()
            .with_model("future/k3")
            .with_thinking_level("low");
        assert_eq!(adapter.id(), "grpc-loopx");
        assert!(!adapter.session_id().is_empty());
        // Full qualification through the mock (covers launch/observe/ingest/
        // classify/ledger arms end to end).
        let dir = tempfile::tempdir().unwrap();
        let mut c = case("gb", "cb", 2);
        c.expected_evidence = Some("artifact".to_string());
        let result = run_qualification_case(&mut adapter, &c, Some(dir.path())).unwrap();
        assert!(result.passed, "{result:?}");
        // Ledger received one idempotent entry.
        let mut ledger = BenchmarkLedger::open(dir.path()).unwrap();
        assert_eq!(ledger.entries().len(), 1);
        let _ = &mut ledger;
    });
}

#[test]
fn grpc_adapter_error_paths() {
    rt().block_on(async {
        // connect failure.
        assert!(GrpcLoopxAdapter::connect("127.0.0.1:1", "/tmp").await.is_err());
        // preflight: empty task → ok:false.
        let (addr, _) = spawn_mock(MockState::default()).await;
        let mut adapter = GrpcLoopxAdapter::connect(&addr, "/tmp").await.unwrap();
        let mut c = case("gb", "cb", 1);
        c.task = String::new();
        let result = run_qualification_case(&mut adapter, &c, None).unwrap();
        assert!(!result.passed);
        assert_eq!(result.failure_class, "runner_error");
        assert_eq!(result.rounds_used, 0);
        // launch: prompt fails → round error propagates.
        let (addr, _) = spawn_mock(MockState::fail("prompt")).await;
        let mut adapter = GrpcLoopxAdapter::connect(&addr, "/tmp").await.unwrap();
        assert!(run_qualification_case(&mut adapter, &case("gb", "cb", 1), None).is_err());
        // observe: stream attach failure.
        let mut st = MockState::default();
        st.stream_error = true;
        let (addr, _) = spawn_mock(st).await;
        let mut adapter = GrpcLoopxAdapter::connect(&addr, "/tmp").await.unwrap();
        assert!(run_qualification_case(&mut adapter, &case("gb", "cb", 1), None).is_err());
        // terminal error round → runner_error classification.
        let (addr, _) = spawn_mock(MockState {
            events: vec![ev("mock-run-1", 0, "agent_end", "{\"state\":\"error\",\"error\":\"dead\"}")],
            ..Default::default()
        })
        .await;
        let mut adapter = GrpcLoopxAdapter::connect(&addr, "/tmp").await.unwrap();
        let result = run_qualification_case(&mut adapter, &case("gb", "cb", 1), None).unwrap();
        assert!(!result.passed);
        assert_eq!(result.failure_class, "runner_error");
    });
}

#[test]
fn grpc_adapter_classify_failed_and_scripted_id() {
    // ScriptedAdapter::id() accessor.
    let a = ScriptedAdapter::new(vec![]);
    assert_eq!(a.id(), "scripted");
    rt().block_on(async {
        // A completed turn whose evidence LACKS the expected marker → the
        // round does not pass, but is not a runner error → "failed".
        let (addr, _) = spawn_mock(MockState {
            events: completed_events("mock-run-1"),
            ..Default::default()
        })
        .await;
        let mut adapter = GrpcLoopxAdapter::connect(&addr, "/tmp").await.unwrap();
        let mut c = case("gb", "cb", 1);
        c.expected_evidence = Some("STRING_NOT_IN_OUTPUT".to_string());
        let result = run_qualification_case(&mut adapter, &c, None).unwrap();
        assert!(!result.passed);
        assert_eq!(result.failure_class, "budget_exhausted");
    });
}
