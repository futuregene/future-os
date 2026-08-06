//! Benchmark run ledger (G-18) — reference `benchmark_ledger.py` (3.8k lines),
//! the minimal compaction core: a content-addressed run entry + a durable
//! JSONL store with idempotent append (same identity → same run_id, re-append
//! is a no-op — mirroring the G-3 event-ledger idempotency).
//!
//! The entry keeps the reference headline surface: identity, round-reward trace
//! compaction (first success / best / final / declared-done), score + pass
//! status, failure class/scope, and the agent model under test.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::store::content_digest;

pub const BENCHMARK_RUN_LEDGER_SCHEMA_VERSION: &str = "benchmark_run_ledger_v0";

/// One round of a benchmark run: reward 1.0 when the verifier passed the
/// round, 0.0 otherwise (reference round_reward_trace records).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RoundRewardRecord {
    pub agent_round: u32,
    pub passed: bool,
    pub reward: f64,
}

/// Compact round-reward trace (reference `round_reward_trace`).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RoundRewardTrace {
    pub records: Vec<RoundRewardRecord>,
    pub max_rounds_budget: u32,
    pub final_round: u32,
    pub final_round_reward: f64,
    pub final_round_passed: bool,
    pub best_reward_round: u32,
    pub best_round_reward: f64,
    pub best_round_passed: bool,
    pub declared_done_round: Option<u32>,
    pub declared_done_score: Option<f64>,
    pub agent_declared_done: bool,
    pub first_success_round: Option<u32>,
}

impl RoundRewardTrace {
    /// Build a trace from per-round records, deriving the headline fields
    /// (reference `_round_reward_best_stats` + `first_success_round` scan).
    pub fn from_records(records: Vec<RoundRewardRecord>, max_rounds_budget: u32) -> Self {
        let first_success_round = records.iter().find(|r| r.passed).map(|r| r.agent_round);
        let best = records
            .iter()
            .max_by(|a, b| {
                a.reward
                    .partial_cmp(&b.reward)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .cloned();
        let final_record = records.last().cloned();
        RoundRewardTrace {
            records,
            max_rounds_budget,
            final_round: final_record.as_ref().map(|r| r.agent_round).unwrap_or(0),
            final_round_reward: final_record.as_ref().map(|r| r.reward).unwrap_or(0.0),
            final_round_passed: final_record.as_ref().map(|r| r.passed).unwrap_or(false),
            best_reward_round: best.as_ref().map(|r| r.agent_round).unwrap_or(0),
            best_round_reward: best.as_ref().map(|r| r.reward).unwrap_or(0.0),
            best_round_passed: best.as_ref().map(|r| r.passed).unwrap_or(false),
            declared_done_round: None,
            declared_done_score: None,
            agent_declared_done: false,
            first_success_round,
        }
    }

    /// Mark the agent's declared-done round (reference `declared_done_round` /
    /// `declared_done_score`).
    pub fn with_declared_done(mut self, round: u32, score: f64) -> Self {
        self.declared_done_round = Some(round);
        self.declared_done_score = Some(score);
        self.agent_declared_done = true;
        self
    }
}

/// The normalized raw run fed to the ledger builder (reference `benchmark_run`).
#[derive(Debug, Clone, Default)]
pub struct BenchmarkRun {
    pub benchmark_id: String,
    pub case_ids: Vec<String>,
    pub arm_id: String,
    pub route: String,
    pub mode: String,
    pub agent_model: String,
    pub job_name: String,
    pub round_reward_trace: RoundRewardTrace,
    /// Terminal status observed by the adapter (`launched` / `completed` /
    /// `runner_error` / `aborted`).
    pub terminal_status: String,
    pub notes: String,
}

/// One ledger entry (reference `build_benchmark_run_ledger_entry` minimal).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BenchmarkLedgerEntry {
    pub schema_version: String,
    pub run_id: String,
    pub benchmark_id: String,
    pub case_ids: Vec<String>,
    pub arm_id: String,
    pub route: String,
    pub mode: String,
    pub agent_model: String,
    pub max_rounds_budget: u32,
    pub first_success_round: Option<u32>,
    pub final_round: u32,
    pub final_round_reward: f64,
    pub final_round_passed: bool,
    pub best_reward_round: u32,
    pub best_round_reward: f64,
    pub best_round_passed: bool,
    pub declared_done_round: Option<u32>,
    pub declared_done_score: Option<f64>,
    pub agent_declared_done: bool,
    /// Best-round reward as the headline score (reference headline metrics).
    pub score: f64,
    pub passed: bool,
    pub failure_class: String,
    pub failure_scope: String,
    pub recorded_at: u64,
    pub notes: String,
}

fn compact_text(value: &str, limit: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= limit {
        trimmed.to_string()
    } else {
        trimmed.chars().take(limit).collect::<String>() + "…"
    }
}

/// Content-derived run identity: `bench-<12 hex>` over
/// benchmark_id|case_id|arm_id|route|job_name. `recorded_at` is deliberately
/// excluded so re-ingesting the same run is idempotent (reference sha1 identity).
pub fn derive_benchmark_run_id(run: &BenchmarkRun) -> String {
    let identity = [
        run.benchmark_id.as_str(),
        run.case_ids.first().map(String::as_str).unwrap_or(""),
        run.arm_id.as_str(),
        run.route.as_str(),
        run.job_name.as_str(),
    ]
    .join("|");
    format!("bench-{}", &content_digest(identity.as_bytes())[..12])
}

/// Failure classification (reference `_failure_class` minimal set):
/// `success` / `case_failure` / `budget_exhausted` / `runner_error` /
/// `unknown`.
pub fn classify_failure(run: &BenchmarkRun) -> (String, String) {
    let rounds = &run.round_reward_trace;
    let passed = rounds.best_round_passed || rounds.final_round_passed;
    let completed = run.terminal_status == "completed" || run.terminal_status == "launched";
    if run.terminal_status == "runner_error" {
        return ("runner_error".to_string(), "runner".to_string());
    }
    if run.terminal_status == "aborted" {
        return ("aborted".to_string(), "run".to_string());
    }
    if passed {
        return ("success".to_string(), "case".to_string());
    }
    if rounds.records.is_empty() {
        return ("runner_error".to_string(), "runner".to_string());
    }
    if completed && rounds.final_round >= rounds.max_rounds_budget && rounds.max_rounds_budget > 0 {
        return ("budget_exhausted".to_string(), "case".to_string());
    }
    if completed {
        return ("case_failure".to_string(), "case".to_string());
    }
    ("unknown".to_string(), "run".to_string())
}

/// Build the compact ledger entry for a raw run (LoopX
/// `build_benchmark_run_ledger_entry`, minimal subset).
pub fn build_benchmark_run_ledger_entry(
    run: &BenchmarkRun,
    recorded_at: u64,
) -> BenchmarkLedgerEntry {
    let trace = &run.round_reward_trace;
    let score = trace.best_round_reward;
    let (failure_class, failure_scope) = classify_failure(run);
    BenchmarkLedgerEntry {
        schema_version: BENCHMARK_RUN_LEDGER_SCHEMA_VERSION.to_string(),
        run_id: derive_benchmark_run_id(run),
        benchmark_id: compact_text(&run.benchmark_id, 120),
        case_ids: run.case_ids.clone(),
        arm_id: compact_text(&run.arm_id, 120),
        route: compact_text(&run.route, 160),
        mode: compact_text(&run.mode, 120),
        agent_model: compact_text(&run.agent_model, 120),
        max_rounds_budget: trace.max_rounds_budget,
        first_success_round: trace.first_success_round,
        final_round: trace.final_round,
        final_round_reward: trace.final_round_reward,
        final_round_passed: trace.final_round_passed,
        best_reward_round: trace.best_reward_round,
        best_round_reward: trace.best_round_reward,
        best_round_passed: trace.best_round_passed,
        declared_done_round: trace.declared_done_round,
        declared_done_score: trace.declared_done_score,
        agent_declared_done: trace.agent_declared_done,
        score,
        passed: trace.best_round_passed || trace.final_round_passed,
        failure_class,
        failure_scope,
        recorded_at,
        notes: compact_text(&run.notes, 240),
    }
}

/// The durable benchmark ledger: JSONL under `<dir>/benchmark_ledger.jsonl`.
/// Appends are content-idempotent (duplicate run_id → no-op); save is atomic
/// (tmp + rename, mirroring the scheduler-state write path).
#[derive(Debug, Clone)]
pub struct BenchmarkLedger {
    path: PathBuf,
    entries: Vec<BenchmarkLedgerEntry>,
}

const LEDGER_FILE: &str = "benchmark_ledger.jsonl";

impl BenchmarkLedger {
    pub fn open(dir: &Path) -> Result<Self> {
        let path = dir.join(LEDGER_FILE);
        let mut entries = Vec::new();
        if path.exists() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("read benchmark ledger {}", path.display()))?;
            for (idx, line) in text.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                match serde_json::from_str::<BenchmarkLedgerEntry>(line) {
                    Ok(entry) => entries.push(entry),
                    Err(e) => bail!("benchmark ledger line {} corrupt: {e}", idx + 1),
                }
            }
        }
        Ok(Self { path, entries })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn entries(&self) -> &[BenchmarkLedgerEntry] {
        &self.entries
    }

    /// Append an entry; duplicate run_id is a no-op (idempotent).
    pub fn append(&mut self, entry: BenchmarkLedgerEntry) -> Result<bool> {
        if self.entries.iter().any(|e| e.run_id == entry.run_id) {
            return Ok(false);
        }
        self.entries.push(entry);
        self.save()?;
        Ok(true)
    }

    /// Query by optional filters (all filters must match).
    pub fn query(
        &self,
        benchmark_id: Option<&str>,
        case_id: Option<&str>,
        arm_id: Option<&str>,
    ) -> Vec<&BenchmarkLedgerEntry> {
        self.entries
            .iter()
            .filter(|e| {
                benchmark_id.map(|b| e.benchmark_id == b).unwrap_or(true)
                    && case_id
                        .map(|c| e.case_ids.iter().any(|id| id == c))
                        .unwrap_or(true)
                    && arm_id.map(|a| e.arm_id == a).unwrap_or(true)
            })
            .collect()
    }

    /// Headline aggregate over matching entries (reference current aggregate).
    pub fn aggregate(&self, benchmark_id: Option<&str>) -> serde_json::Value {
        let matched = match benchmark_id {
            Some(b) => self.query(Some(b), None, None),
            None => self.entries.iter().collect::<Vec<_>>(),
        };
        let mut by_class: BTreeMap<String, usize> = BTreeMap::new();
        let mut passed = 0usize;
        let mut total_score = 0.0f64;
        for e in &matched {
            *by_class.entry(e.failure_class.clone()).or_insert(0) += 1;
            if e.passed {
                passed += 1;
            }
            total_score += e.score;
        }
        serde_json::json!({
            "schema_version": BENCHMARK_RUN_LEDGER_SCHEMA_VERSION,
            "run_count": matched.len(),
            "passed": passed,
            "avg_best_score": if matched.is_empty() { 0.0 } else { total_score / matched.len() as f64 },
            "by_failure_class": by_class,
        })
    }

    fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let mut text = String::new();
        for e in &self.entries {
            text.push_str(&serde_json::to_string(e)?);
            text.push('\n');
        }
        let tmp = self.path.with_extension("jsonl.tmp");
        std::fs::write(&tmp, text).with_context(|| format!("write {}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path)
            .with_context(|| format!("rename onto {}", self.path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_run() -> BenchmarkRun {
        BenchmarkRun {
            benchmark_id: "skillsbench@1.1".to_string(),
            case_ids: vec!["case-42".to_string()],
            arm_id: "future_loop_product_mode".to_string(),
            route: "future-loop-product-mode".to_string(),
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
    fn run_id_is_content_stable_and_excludes_time() {
        let a = derive_benchmark_run_id(&sample_run());
        let b = derive_benchmark_run_id(&sample_run());
        assert_eq!(a, b);
        assert!(a.starts_with("bench-"));
        assert_eq!(a.len(), "bench-".len() + 12);
        // recorded_at (a caller-side field) is not part of identity
        let mut mutated = sample_run();
        mutated.case_ids = vec!["case-43".to_string()];
        assert_ne!(derive_benchmark_run_id(&mutated), a);
        // non-identity fields (notes) do not perturb the id
        let mut same = sample_run();
        same.notes = "different".to_string();
        assert_eq!(derive_benchmark_run_id(&same), a);
    }

    #[test]
    fn entry_compacts_round_trace_headline_fields() {
        let entry = build_benchmark_run_ledger_entry(&sample_run(), 1_700_000_000);
        assert_eq!(entry.schema_version, BENCHMARK_RUN_LEDGER_SCHEMA_VERSION);
        assert_eq!(entry.first_success_round, Some(2));
        assert_eq!(entry.best_reward_round, 2);
        assert_eq!(entry.best_round_reward, 1.0);
        assert!(entry.best_round_passed);
        assert_eq!(entry.final_round, 2);
        assert!(entry.final_round_passed);
        assert_eq!(entry.score, 1.0);
        assert!(entry.passed);
        assert_eq!(entry.failure_class, "success");
        assert_eq!(entry.failure_scope, "case");
        assert_eq!(entry.max_rounds_budget, 5);
    }

    #[test]
    fn failure_classification() {
        // budget exhausted without pass
        let mut run = sample_run();
        run.terminal_status = "completed".to_string();
        run.round_reward_trace = RoundRewardTrace::from_records(
            vec![RoundRewardRecord {
                agent_round: 1,
                passed: false,
                reward: 0.0,
            }],
            1,
        );
        assert_eq!(classify_failure(&run).0, "budget_exhausted");
        // runner error
        let mut run = sample_run();
        run.terminal_status = "runner_error".to_string();
        assert_eq!(classify_failure(&run).0, "runner_error");
        assert_eq!(classify_failure(&run).1, "runner");
        // case failure (rounds but no pass, budget not exhausted)
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
        // no rounds at all
        let mut run = sample_run();
        run.round_reward_trace = RoundRewardTrace::default();
        run.terminal_status = "launched".to_string();
        assert_eq!(classify_failure(&run).0, "runner_error");
    }

    #[test]
    fn ledger_append_is_idempotent_and_persists() {
        let dir = std::env::temp_dir().join(format!(
            "future-loop-bench-ledger-{}",
            crate::state::now_epoch()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut ledger = BenchmarkLedger::open(&dir).unwrap();
        let entry = build_benchmark_run_ledger_entry(&sample_run(), 1);
        assert!(ledger.append(entry.clone()).unwrap());
        // duplicate append is a no-op
        assert!(!ledger.append(entry).unwrap());
        assert_eq!(ledger.entries().len(), 1);
        // re-open persists
        let reopened = BenchmarkLedger::open(&dir).unwrap();
        assert_eq!(reopened.entries().len(), 1);
        assert_eq!(
            reopened.entries()[0].run_id,
            derive_benchmark_run_id(&sample_run())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn query_and_aggregate() {
        let mut ledger = BenchmarkLedger::open(&std::env::temp_dir().join(format!(
            "future-loop-bench-agg-{}",
            crate::state::now_epoch()
        )))
        .unwrap();
        ledger
            .append(build_benchmark_run_ledger_entry(&sample_run(), 1))
            .unwrap();
        let mut other = sample_run();
        other.case_ids = vec!["case-7".to_string()];
        other.job_name = "job-2".to_string();
        ledger
            .append(build_benchmark_run_ledger_entry(&other, 2))
            .unwrap();
        assert_eq!(ledger.query(Some("skillsbench@1.1"), None, None).len(), 2);
        assert_eq!(
            ledger
                .query(Some("skillsbench@1.1"), Some("case-42"), None)
                .len(),
            1
        );
        assert_eq!(ledger.query(None, Some("case-7"), None).len(), 1);
        assert_eq!(ledger.query(Some("other"), None, None).len(), 0);
        let agg = ledger.aggregate(Some("skillsbench@1.1"));
        assert_eq!(agg["run_count"], 2);
        assert_eq!(agg["passed"], 2);
        assert_eq!(agg["by_failure_class"]["success"], 2);
    }

    #[test]
    fn default_budget_constant_available() {
        // sanity: the ledger references the loop-protocol default
        assert_eq!(
            super::super::loop_protocol::BLIND_LOOP_DEFAULT_MAX_ROUNDS,
            5
        );
    }
}
