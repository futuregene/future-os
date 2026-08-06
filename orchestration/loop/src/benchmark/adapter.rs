//! Benchmark adapter (G-18) — LoopX `benchmark_core/adapter.py` protocol,
//! with the adapter that reuses our gRPC direct-drive channel.
//!
//! The adapter contract is transport-neutral: preflight → launch → observe →
//! ingest → classify → ledger. `GrpcLoopxAdapter` implements it over
//! `AgentClient` — one bounded prompt per round, readback via `run_turn`
//! (the same observable execution channel the loop control plane uses; no
//! parallel execution path, per §5.4).
//!
//! `ScriptedAdapter` is a deterministic stub used by the contract tests so
//! the round accounting, classification and ledger logic are fully tested
//! without an LLM or a running agent.

use std::path::Path;

use anyhow::{bail, Result};

use super::ledger::{
    build_benchmark_run_ledger_entry, BenchmarkLedger, BenchmarkLedgerEntry, BenchmarkRun,
    RoundRewardTrace,
};
use crate::agent_client::AgentClient;

/// Adapter-neutral request for a benchmark case run (LoopX BenchmarkRequest).
#[derive(Debug, Clone)]
pub struct BenchmarkRequest {
    pub benchmark_id: String,
    pub case_id: String,
    pub route: String,
    pub arm_id: String,
    pub max_rounds: Option<u32>,
    /// The case task/prompt the agent receives each round.
    pub task: String,
    /// Optional verifier expectation — the round evidence must contain this
    /// substring to count as passed (LoopX verifier_reward semantics).
    pub expected_evidence: Option<String>,
    pub metadata: serde_json::Value,
}

impl BenchmarkRequest {
    pub fn new(benchmark_id: &str, case_id: &str, route: &str, task: &str) -> Self {
        Self {
            benchmark_id: benchmark_id.to_string(),
            case_id: case_id.to_string(),
            route: route.to_string(),
            arm_id: String::new(),
            max_rounds: None,
            task: task.to_string(),
            expected_evidence: None,
            metadata: serde_json::Value::Null,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PreflightResult {
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct RunHandle {
    pub run_id: String,
    pub external_id: String,
}

#[derive(Debug, Clone)]
pub struct LaunchResult {
    pub process_started: bool,
    pub handle: Option<RunHandle>,
    pub detail: String,
}

/// What the adapter observed after one round (readback).
#[derive(Debug, Clone)]
pub struct Observation {
    pub terminal_state: String,
    pub error: Option<String>,
    pub tools: Vec<String>,
    /// Truncated assistant text (the evidence candidate).
    pub evidence: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost: f64,
}

#[derive(Debug, Clone)]
pub struct IngestResult {
    /// The normalized raw run the ledger consumes.
    pub benchmark_run: BenchmarkRun,
    pub observation: Observation,
    /// Adapter-level pass determination (round reward = 1.0 when true).
    pub passed: bool,
}

#[derive(Debug, Clone)]
pub struct AdapterClassification {
    /// `passed` / `failed` / `runner_error`.
    pub decision: String,
    pub passed: bool,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct LedgerUpdate {
    pub written: bool,
    pub entry: Option<BenchmarkLedgerEntry>,
}

/// The minimal control-plane adapter interface (LoopX BenchmarkAdapter).
pub trait BenchmarkAdapter {
    fn id(&self) -> &str;
    fn preflight(&mut self, request: &BenchmarkRequest) -> Result<PreflightResult>;
    fn launch(&mut self, request: &BenchmarkRequest) -> Result<LaunchResult>;
    fn observe(&mut self, handle: &RunHandle) -> Result<Observation>;
    fn ingest(
        &mut self,
        request: &BenchmarkRequest,
        handle: &RunHandle,
        observation: &Observation,
    ) -> Result<IngestResult>;
    fn classify(
        &mut self,
        request: &BenchmarkRequest,
        ingest: &IngestResult,
    ) -> Result<AdapterClassification>;
    /// Persist a ledger entry under `ledger_dir` (when Some). Returns the
    /// update; writing is content-idempotent.
    fn ledger(
        &mut self,
        request: &BenchmarkRequest,
        ingest: &IngestResult,
        ledger_dir: Option<&Path>,
        recorded_at: u64,
    ) -> Result<LedgerUpdate>;
}

/// gRPC adapter: preflight → launch → observe over `AgentClient`.
///
/// The trait is synchronous so the round loop is deterministic and testable;
/// the adapter blocks on the ambient tokio runtime (main runs under
/// `#[tokio::main]`).
pub struct GrpcLoopxAdapter {
    pub id: String,
    client: AgentClient,
    session_id: String,
    model: Option<String>,
    thinking_level: Option<String>,
    round: u32,
}

impl GrpcLoopxAdapter {
    /// Connect to FutureAgent and create an isolated session for the case.
    pub async fn connect(addr: &str, cwd: &str) -> Result<Self> {
        let mut client = AgentClient::connect(addr).await?;
        let session_id = client.new_session(cwd).await?;
        Ok(Self {
            id: "grpc-loopx".to_string(),
            client,
            session_id,
            model: None,
            thinking_level: None,
            round: 0,
        })
    }

    pub fn with_model(mut self, model: &str) -> Self {
        self.model = Some(model.to_string());
        self
    }

    pub fn with_thinking_level(mut self, level: &str) -> Self {
        self.thinking_level = Some(level.to_string());
        self
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Handle::current().block_on(fut)
    }
}

impl BenchmarkAdapter for GrpcLoopxAdapter {
    fn id(&self) -> &str {
        &self.id
    }

    fn preflight(&mut self, request: &BenchmarkRequest) -> Result<PreflightResult> {
        if request.task.trim().is_empty() {
            return Ok(PreflightResult {
                ok: false,
                detail: "case task is empty".to_string(),
            });
        }
        // The session answers → the observable channel is live.
        let totals = Self::block_on(self.client.session_totals(&self.session_id))?;
        let detail = format!(
            "session {} live (tokens_in={} tokens_out={})",
            self.session_id, totals.tokens_in, totals.tokens_out
        );
        Ok(PreflightResult { ok: true, detail })
    }

    fn launch(&mut self, request: &BenchmarkRequest) -> Result<LaunchResult> {
        if let Some(model) = &self.model {
            Self::block_on(self.client.set_model(&self.session_id, model))?;
        }
        if let Some(level) = &self.thinking_level {
            Self::block_on(self.client.set_thinking_level(&self.session_id, level))?;
        }
        self.round += 1;
        let instruction = format!(
            "Benchmark case {} (round {}).\nTask: {}\n\nWork on the task; report what you did and observed.",
            request.case_id, self.round, request.task
        );
        let request_id = format!(
            "bench-{}-{}-r{}",
            request.benchmark_id, request.case_id, self.round
        );
        let run_id = Self::block_on(self.client.prompt(
            &self.session_id,
            &instruction,
            &request_id,
        ))?;
        Ok(LaunchResult {
            process_started: true,
            handle: Some(RunHandle {
                run_id,
                external_id: request_id,
            }),
            detail: format!("round {} launched", self.round),
        })
    }

    fn observe(&mut self, handle: &RunHandle) -> Result<Observation> {
        let before = Self::block_on(self.client.session_totals(&self.session_id))?;
        let summary = Self::block_on(self.client.run_turn(&self.session_id, &handle.run_id))?;
        let after = Self::block_on(self.client.session_totals(&self.session_id))?;
        Ok(Observation {
            terminal_state: summary.terminal_state,
            error: summary.error.clone(),
            tools: summary.tools,
            evidence: crate::decision::truncate(&summary.text, 2_000),
            tokens_in: after.tokens_in.saturating_sub(before.tokens_in),
            tokens_out: after.tokens_out.saturating_sub(before.tokens_out),
            cost: (after.cost - before.cost).max(0.0),
        })
    }

    fn ingest(
        &mut self,
        request: &BenchmarkRequest,
        _handle: &RunHandle,
        observation: &Observation,
    ) -> Result<IngestResult> {
        let passed = observation.terminal_state == "completed"
            && request
                .expected_evidence
                .as_ref()
                .map(|expect| observation.evidence.contains(expect.as_str()))
                .unwrap_or(true);
        let trace = RoundRewardTrace::from_records(
            vec![super::ledger::RoundRewardRecord {
                agent_round: self.round,
                passed,
                reward: if passed { 1.0 } else { 0.0 },
            }],
            request.max_rounds.unwrap_or(5),
        );
        let benchmark_run = BenchmarkRun {
            benchmark_id: request.benchmark_id.clone(),
            case_ids: vec![request.case_id.clone()],
            arm_id: request.arm_id.clone(),
            route: request.route.clone(),
            mode: "product".to_string(),
            agent_model: self.model.clone().unwrap_or_else(|| "unknown".to_string()),
            job_name: format!("{}-{}", request.benchmark_id, request.case_id),
            round_reward_trace: trace,
            terminal_status: if observation.terminal_state == "error" {
                "runner_error".to_string()
            } else {
                "completed".to_string()
            },
            notes: observation
                .error
                .clone()
                .map(|e| format!("agent error: {e}"))
                .unwrap_or_default(),
        };
        Ok(IngestResult {
            benchmark_run,
            observation: observation.clone(),
            passed,
        })
    }

    fn classify(
        &mut self,
        _request: &BenchmarkRequest,
        ingest: &IngestResult,
    ) -> Result<AdapterClassification> {
        let run = &ingest.benchmark_run;
        if run.terminal_status == "runner_error" {
            return Ok(AdapterClassification {
                decision: "runner_error".to_string(),
                passed: false,
                reason: "agent reported a terminal error".to_string(),
            });
        }
        if ingest.passed {
            Ok(AdapterClassification {
                decision: "passed".to_string(),
                passed: true,
                reason: "round completed with evidence".to_string(),
            })
        } else {
            Ok(AdapterClassification {
                decision: "failed".to_string(),
                passed: false,
                reason: "round did not satisfy the pass condition".to_string(),
            })
        }
    }

    fn ledger(
        &mut self,
        _request: &BenchmarkRequest,
        ingest: &IngestResult,
        ledger_dir: Option<&Path>,
        recorded_at: u64,
    ) -> Result<LedgerUpdate> {
        let Some(dir) = ledger_dir else {
            return Ok(LedgerUpdate {
                written: false,
                entry: None,
            });
        };
        let mut ledger = BenchmarkLedger::open(dir)?;
        let entry = build_benchmark_run_ledger_entry(&ingest.benchmark_run, recorded_at);
        let written = ledger.append(entry.clone())?;
        Ok(LedgerUpdate {
            written,
            entry: Some(entry),
        })
    }
}

/// Deterministic stub adapter for contract tests: scripted per-round
/// terminal states, so round accounting / classification / ledger logic are
/// exercised without a live agent.
pub struct ScriptedAdapter {
    pub id: String,
    /// terminal states to serve per round (last value repeats).
    pub script: Vec<String>,
    pub round: u32,
    pub evidence: String,
}

impl ScriptedAdapter {
    pub fn new(script: Vec<String>) -> Self {
        Self {
            id: "scripted".to_string(),
            script,
            round: 0,
            evidence: "scripted evidence payload".to_string(),
        }
    }
}

impl BenchmarkAdapter for ScriptedAdapter {
    fn id(&self) -> &str {
        &self.id
    }

    fn preflight(&mut self, request: &BenchmarkRequest) -> Result<PreflightResult> {
        if request.task.trim().is_empty() {
            bail!("scripted preflight: empty task");
        }
        Ok(PreflightResult {
            ok: true,
            detail: "scripted preflight ok".to_string(),
        })
    }

    fn launch(&mut self, request: &BenchmarkRequest) -> Result<LaunchResult> {
        self.round += 1;
        Ok(LaunchResult {
            process_started: true,
            handle: Some(RunHandle {
                run_id: format!("scripted-{}-{}", request.case_id, self.round),
                external_id: String::new(),
            }),
            detail: format!("round {} launched", self.round),
        })
    }

    fn observe(&mut self, handle: &RunHandle) -> Result<Observation> {
        let idx = (self.round as usize).saturating_sub(1);
        let terminal_state = self.script.get(idx).cloned().unwrap_or_else(|| {
            self.script
                .last()
                .cloned()
                .unwrap_or_else(|| "completed".to_string())
        });
        let _ = handle;
        Ok(Observation {
            terminal_state: terminal_state.clone(),
            error: if terminal_state == "error" {
                Some("scripted error".to_string())
            } else {
                None
            },
            tools: if terminal_state == "completed" {
                vec!["bash".to_string()]
            } else {
                vec![]
            },
            evidence: self.evidence.clone(),
            tokens_in: 100,
            tokens_out: 50,
            cost: 0.01,
        })
    }

    fn ingest(
        &mut self,
        request: &BenchmarkRequest,
        _handle: &RunHandle,
        observation: &Observation,
    ) -> Result<IngestResult> {
        let passed = observation.terminal_state == "completed"
            && request
                .expected_evidence
                .as_ref()
                .map(|expect| observation.evidence.contains(expect.as_str()))
                .unwrap_or(true);
        let trace = RoundRewardTrace::from_records(
            vec![super::ledger::RoundRewardRecord {
                agent_round: self.round,
                passed,
                reward: if passed { 1.0 } else { 0.0 },
            }],
            request.max_rounds.unwrap_or(5),
        );
        let benchmark_run = BenchmarkRun {
            benchmark_id: request.benchmark_id.clone(),
            case_ids: vec![request.case_id.clone()],
            arm_id: request.arm_id.clone(),
            route: request.route.clone(),
            mode: "product".to_string(),
            agent_model: "stub".to_string(),
            job_name: format!("{}-{}", request.benchmark_id, request.case_id),
            round_reward_trace: trace,
            terminal_status: if observation.terminal_state == "error" {
                "runner_error".to_string()
            } else {
                "completed".to_string()
            },
            notes: String::new(),
        };
        Ok(IngestResult {
            benchmark_run,
            observation: observation.clone(),
            passed,
        })
    }

    fn classify(
        &mut self,
        _request: &BenchmarkRequest,
        ingest: &IngestResult,
    ) -> Result<AdapterClassification> {
        if ingest.benchmark_run.terminal_status == "runner_error" {
            return Ok(AdapterClassification {
                decision: "runner_error".to_string(),
                passed: false,
                reason: "runner error".to_string(),
            });
        }
        Ok(AdapterClassification {
            decision: if ingest.passed { "passed" } else { "failed" }.to_string(),
            passed: ingest.passed,
            reason: if ingest.passed {
                "scripted pass".to_string()
            } else {
                "scripted fail".to_string()
            },
        })
    }

    fn ledger(
        &mut self,
        _request: &BenchmarkRequest,
        ingest: &IngestResult,
        ledger_dir: Option<&Path>,
        recorded_at: u64,
    ) -> Result<LedgerUpdate> {
        let Some(dir) = ledger_dir else {
            return Ok(LedgerUpdate {
                written: false,
                entry: None,
            });
        };
        let mut ledger = BenchmarkLedger::open(dir)?;
        let entry = build_benchmark_run_ledger_entry(&ingest.benchmark_run, recorded_at);
        let written = ledger.append(entry.clone())?;
        Ok(LedgerUpdate {
            written,
            entry: Some(entry),
        })
    }
}

/// Shared helper: run one round against an adapter and classify it.
pub fn run_round(
    adapter: &mut dyn BenchmarkAdapter,
    request: &BenchmarkRequest,
    ledger_dir: Option<&Path>,
    recorded_at: u64,
) -> Result<(AdapterClassification, LedgerUpdate)> {
    let launch = adapter.launch(request)?;
    let handle = launch
        .handle
        .ok_or_else(|| anyhow::anyhow!("launch returned no handle: {}", launch.detail))?;
    let observation = adapter.observe(&handle)?;
    let ingest = adapter.ingest(request, &handle, &observation)?;
    let classification = adapter.classify(request, &ingest)?;
    let update = adapter.ledger(request, &ingest, ledger_dir, recorded_at)?;
    Ok((classification, update))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripted_round_classification_and_ledger() {
        let dir =
            std::env::temp_dir().join(format!("loopx-bench-adapter-{}", crate::state::now_epoch()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut adapter = ScriptedAdapter::new(vec!["completed".to_string()]);
        let request = BenchmarkRequest::new(
            "skillsbench@1.1",
            "case-1",
            "loopx-product-mode",
            "implement fizzbuzz",
        );
        let pre = adapter.preflight(&request).unwrap();
        assert!(pre.ok);
        let (classification, update) =
            run_round(&mut adapter, &request, Some(&dir), 1_700_000_000).unwrap();
        assert_eq!(classification.decision, "passed");
        assert!(classification.passed);
        let entry = update.entry.unwrap();
        assert_eq!(entry.benchmark_id, "skillsbench@1.1");
        assert_eq!(entry.case_ids, vec!["case-1"]);
        assert_eq!(entry.score, 1.0);
        assert_eq!(entry.failure_class, "success");
        // ledger persisted
        let ledger = BenchmarkLedger::open(&dir).unwrap();
        assert_eq!(ledger.entries().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scripted_runner_error_classifies_fail_closed() {
        let mut adapter = ScriptedAdapter::new(vec!["error".to_string()]);
        let request = BenchmarkRequest::new("b", "c", "loopx-product-mode", "task");
        let (classification, _) = run_round(&mut adapter, &request, None, 1).unwrap();
        assert_eq!(classification.decision, "runner_error");
        assert!(!classification.passed);
    }

    #[test]
    fn expected_evidence_gates_pass() {
        let mut adapter = ScriptedAdapter::new(vec!["completed".to_string()]);
        let mut request = BenchmarkRequest::new("b", "c", "loopx-product-mode", "task");
        request.expected_evidence = Some("missing-marker".to_string());
        let (classification, _) = run_round(&mut adapter, &request, None, 1).unwrap();
        assert!(!classification.passed, "evidence gate must fail closed");
        assert_eq!(classification.decision, "failed");
    }
}
