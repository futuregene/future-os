//! Qualification scenario (G-18) — the single-scenario closed loop: preflight
//! → rounds (launch/observe/classify) → round-reward trace → ledger append →
//! headline metrics. reference `benchmarks/qualification/` + `benchmark_core/
//! rounds.py` minimal core.
//!
//! The loop is deliberately small and deterministic: stop on first pass
//! (reference `stop_on_reward_one`), stop on the round budget, count every round
//! in the trace, and let the ledger classify the outcome.

use std::path::Path;

use anyhow::{bail, Result};

use super::adapter::{run_round, BenchmarkAdapter, BenchmarkRequest};
use super::ledger::{RoundRewardRecord, RoundRewardTrace};
use crate::state::now_epoch;

/// One qualification case (a single benchmark scenario).
#[derive(Debug, Clone)]
pub struct QualificationCase {
    pub benchmark_id: String,
    pub case_id: String,
    pub route: String,
    pub arm_id: String,
    pub task: String,
    pub max_rounds: u32,
    /// Optional verifier expectation; the round evidence must contain it.
    pub expected_evidence: Option<String>,
}

impl QualificationCase {
    pub fn new(benchmark_id: &str, case_id: &str, task: &str, max_rounds: u32) -> Self {
        Self {
            benchmark_id: benchmark_id.to_string(),
            case_id: case_id.to_string(),
            route: "future-loop-product-mode".to_string(),
            arm_id: "future_loop_product_mode".to_string(),
            task: task.to_string(),
            max_rounds,
            expected_evidence: None,
        }
    }

    pub fn request(&self) -> BenchmarkRequest {
        BenchmarkRequest {
            benchmark_id: self.benchmark_id.clone(),
            case_id: self.case_id.clone(),
            route: self.route.clone(),
            arm_id: self.arm_id.clone(),
            max_rounds: Some(self.max_rounds),
            task: self.task.clone(),
            expected_evidence: self.expected_evidence.clone(),
            metadata: serde_json::Value::Null,
        }
    }
}

/// Headline metrics (reference product-mode policy gate headline_metrics).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HeadlineMetrics {
    pub best_score: f64,
    pub final_score: f64,
    pub first_success_round: Option<u32>,
    pub declared_done_score: f64,
}

/// The result of one qualification run.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QualificationResult {
    pub schema_version: String,
    pub benchmark_id: String,
    pub case_id: String,
    pub route: String,
    pub arm_id: String,
    pub passed: bool,
    pub rounds_used: u32,
    pub max_rounds: u32,
    pub round_reward_trace: RoundRewardTrace,
    pub headline: HeadlineMetrics,
    pub failure_class: String,
    pub failure_scope: String,
}

pub const QUALIFICATION_RESULT_SCHEMA_VERSION: &str = "qualification_result_v0";

/// Run one qualification case to completion (or budget).
///
/// The loop: preflight → up to `max_rounds` rounds; each round launches,
/// observes, ingests and classifies through the adapter; the first passed
/// round stops the loop (reward 1.0). When `ledger_dir` is Some the per-round
/// ledger entries are persisted (idempotent).
pub fn run_qualification_case(
    adapter: &mut dyn BenchmarkAdapter,
    case: &QualificationCase,
    ledger_dir: Option<&Path>,
) -> Result<QualificationResult> {
    if case.max_rounds == 0 {
        bail!("qualification case requires max_rounds > 0");
    }
    let request = case.request();
    let preflight = adapter.preflight(&request)?;
    if !preflight.ok {
        return Ok(QualificationResult {
            schema_version: QUALIFICATION_RESULT_SCHEMA_VERSION.to_string(),
            benchmark_id: case.benchmark_id.clone(),
            case_id: case.case_id.clone(),
            route: case.route.clone(),
            arm_id: case.arm_id.clone(),
            passed: false,
            rounds_used: 0,
            max_rounds: case.max_rounds,
            round_reward_trace: RoundRewardTrace::default(),
            headline: HeadlineMetrics {
                best_score: 0.0,
                final_score: 0.0,
                first_success_round: None,
                declared_done_score: 0.0,
            },
            failure_class: "runner_error".to_string(),
            failure_scope: "runner".to_string(),
        });
    }

    let mut records: Vec<RoundRewardRecord> = Vec::new();
    let mut passed_round: Option<u32> = None;
    let mut terminal_status = "launched".to_string();
    for round in 1..=case.max_rounds {
        let recorded_at = now_epoch();
        let (classification, _update) = run_round(adapter, &request, ledger_dir, recorded_at)?;
        let passed = classification.passed;
        records.push(RoundRewardRecord {
            agent_round: round,
            passed,
            reward: if passed { 1.0 } else { 0.0 },
        });
        if classification.decision == "runner_error" {
            terminal_status = "runner_error".to_string();
            break;
        }
        if passed {
            passed_round = Some(round);
            terminal_status = "completed".to_string();
            break;
        }
        terminal_status = "completed".to_string();
    }

    let trace = RoundRewardTrace::from_records(records, case.max_rounds);
    let passed = passed_round.is_some();
    let headline = HeadlineMetrics {
        best_score: trace.best_round_reward,
        final_score: trace.final_round_reward,
        first_success_round: trace.first_success_round,
        declared_done_score: if passed { 1.0 } else { 0.0 },
    };
    // Failure class from the last per-round entry semantics (reuse the ledger
    // classifier over a synthetic run).
    let synthetic = super::ledger::BenchmarkRun {
        benchmark_id: case.benchmark_id.clone(),
        case_ids: vec![case.case_id.clone()],
        arm_id: case.arm_id.clone(),
        route: case.route.clone(),
        mode: "product".to_string(),
        agent_model: "adapter".to_string(),
        job_name: format!("{}-{}", case.benchmark_id, case.case_id),
        round_reward_trace: trace.clone(),
        terminal_status,
        notes: String::new(),
    };
    let (failure_class, failure_scope) = super::ledger::classify_failure(&synthetic);
    // A stopped-on-pass loop that exhausted every round with no pass is a
    // budget exhaustion only when the loop actually ran to the budget.
    // A non-pass run either broke early on runner_error (handled above) or
    // ran every round — records == max_rounds — so the remaining outcome is
    // always budget exhaustion.
    let failure_class = if passed {
        "success".to_string()
    } else if failure_class == "runner_error" {
        failure_class
    } else {
        debug_assert!(trace.records.len() as u32 >= case.max_rounds);
        "budget_exhausted".to_string()
    };

    Ok(QualificationResult {
        schema_version: QUALIFICATION_RESULT_SCHEMA_VERSION.to_string(),
        benchmark_id: case.benchmark_id.clone(),
        case_id: case.case_id.clone(),
        route: case.route.clone(),
        arm_id: case.arm_id.clone(),
        passed,
        rounds_used: trace.records.len() as u32,
        max_rounds: case.max_rounds,
        round_reward_trace: trace,
        headline,
        failure_class,
        failure_scope,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark::adapter::ScriptedAdapter;

    #[test]
    fn passes_on_first_success_round_and_stops() {
        let mut adapter = ScriptedAdapter::new(vec![
            "completed".to_string(),
            "error".to_string(), // must never run — loop stops at round 1
        ]);
        let case = QualificationCase::new("skillsbench@1.1", "case-1", "fizzbuzz", 5);
        let result = run_qualification_case(&mut adapter, &case, None).unwrap();
        assert!(result.passed);
        assert_eq!(result.rounds_used, 1);
        assert_eq!(result.headline.first_success_round, Some(1));
        assert_eq!(result.headline.best_score, 1.0);
        assert_eq!(result.failure_class, "success");
        assert_eq!(adapter.round, 1, "loop must stop after the first pass");
    }

    #[test]
    fn exhausts_budget_when_never_passing() {
        let mut adapter = ScriptedAdapter::new(vec!["cancelled".to_string()]);
        let case = QualificationCase::new("b", "c", "hard task", 3);
        let result = run_qualification_case(&mut adapter, &case, None).unwrap();
        assert!(!result.passed);
        assert_eq!(result.rounds_used, 3);
        assert_eq!(result.headline.best_score, 0.0);
        assert_eq!(result.failure_class, "budget_exhausted");
    }

    #[test]
    fn runner_error_aborts_the_loop() {
        let mut adapter = ScriptedAdapter::new(vec!["error".to_string()]);
        let case = QualificationCase::new("b", "c", "task", 5);
        let result = run_qualification_case(&mut adapter, &case, None).unwrap();
        assert!(!result.passed);
        assert_eq!(result.rounds_used, 1);
        assert_eq!(result.failure_class, "runner_error");
        assert_eq!(result.failure_scope, "runner");
    }

    #[test]
    fn preflight_failure_is_runner_error_with_zero_rounds() {
        struct BrokenAdapter;
        impl BenchmarkAdapter for BrokenAdapter {
            fn id(&self) -> &str {
                "broken"
            }
            fn preflight(
                &mut self,
                _request: &BenchmarkRequest,
            ) -> Result<super::super::adapter::PreflightResult> {
                Ok(super::super::adapter::PreflightResult {
                    ok: false,
                    detail: "agent unreachable".to_string(),
                })
            }
            fn launch(
                &mut self,
                _request: &BenchmarkRequest,
            ) -> Result<super::super::adapter::LaunchResult> {
                bail!("unreachable")
            }
            fn observe(
                &mut self,
                _handle: &super::super::adapter::RunHandle,
            ) -> Result<super::super::adapter::Observation> {
                bail!("unreachable")
            }
            fn ingest(
                &mut self,
                _request: &BenchmarkRequest,
                _handle: &super::super::adapter::RunHandle,
                _observation: &super::super::adapter::Observation,
            ) -> Result<super::super::adapter::IngestResult> {
                bail!("unreachable")
            }
            fn classify(
                &mut self,
                _request: &BenchmarkRequest,
                _ingest: &super::super::adapter::IngestResult,
            ) -> Result<super::super::adapter::AdapterClassification> {
                bail!("unreachable")
            }
            fn ledger(
                &mut self,
                _request: &BenchmarkRequest,
                _ingest: &super::super::adapter::IngestResult,
                _ledger_dir: Option<&Path>,
                _recorded_at: u64,
            ) -> Result<super::super::adapter::LedgerUpdate> {
                bail!("unreachable")
            }
        }
        let mut adapter = BrokenAdapter;
        let case = QualificationCase::new("b", "c", "task", 5);
        let result = run_qualification_case(&mut adapter, &case, None).unwrap();
        assert!(!result.passed);
        assert_eq!(result.rounds_used, 0);
        assert_eq!(result.failure_class, "runner_error");
        // Exercise the unreachable-by-design stub arms so coverage sees them
        // (they must bail, never panic-free return).
        let request = case.request();
        let handle = super::super::adapter::RunHandle {
            run_id: "r".to_string(),
            external_id: "e".to_string(),
        };
        let observation = super::super::adapter::Observation {
            terminal_state: "completed".to_string(),
            error: None,
            tools: vec![],
            evidence: String::new(),
            tokens_in: 0,
            tokens_out: 0,
            cost: 0.0,
        };
        assert_eq!(adapter.id(), "broken");
        assert!(adapter.launch(&request).is_err());
        assert!(adapter.observe(&handle).is_err());
        assert!(adapter.ingest(&request, &handle, &observation).is_err());
        let ingest = super::super::adapter::IngestResult {
            benchmark_run: Default::default(),
            observation,
            passed: false,
        };
        assert!(adapter.classify(&request, &ingest).is_err());
        assert!(adapter.ledger(&request, &ingest, None, 0).is_err());
    }

    #[test]
    fn ledger_dir_records_entries_and_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("future-loop-bench-qual-{}", now_epoch()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut adapter = ScriptedAdapter::new(vec!["completed".to_string()]);
        let case = QualificationCase::new("b", "c", "task", 5);
        let result = run_qualification_case(&mut adapter, &case, Some(&dir)).unwrap();
        assert!(result.passed);
        let ledger = super::super::ledger::BenchmarkLedger::open(&dir).unwrap();
        assert_eq!(ledger.entries().len(), 1);
        assert_eq!(ledger.entries()[0].case_ids, vec!["c"]);
        // Re-running the same case appends nothing (idempotent identity).
        let mut adapter2 = ScriptedAdapter::new(vec!["completed".to_string()]);
        run_qualification_case(&mut adapter2, &case, Some(&dir)).unwrap();
        let ledger = super::super::ledger::BenchmarkLedger::open(&dir).unwrap();
        assert_eq!(ledger.entries().len(), 1, "same identity must dedupe");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
