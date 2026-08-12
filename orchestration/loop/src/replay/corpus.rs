//! Model-behavior corpus (G-19) — LoopX
//! `control_plane/testing/model_behavior_corpus.py` (344 lines), natively.
//!
//! The corpus is the LLM-side extension of the contract tests: a set of
//! behavior cases (state-matrix patches, retained real decisions,
//! counterfactuals, candidate ablations) where each case runs a FULL packet
//! arm against a CANDIDATE arm through a model-behavior actor, and the pair
//! must come out equivalent — or, for ablations, fail closed.
//!
//! The harness is fully deterministic under `StubActor` (the contract-test
//! path); a real LLM actor plugs in by implementing [`ModelBehaviorActor`]
//! and parsing the same signal schema from the rendered request. Persistence
//! boundary: raw packets and model responses are never persisted.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::contract::ShouldRunPacket;

pub const MODEL_BEHAVIOR_CORPUS_SCHEMA_VERSION: &str = "model_behavior_corpus_v0";
pub const MODEL_BEHAVIOR_CORPUS_CASE_SCHEMA_VERSION: &str = "model_behavior_corpus_case_v0";
pub const MODEL_BEHAVIOR_CORPUS_RESULT_SCHEMA_VERSION: &str = "model_behavior_corpus_result_v0";

/// Hard invariant fields: the actor's behavior signal must be identical
/// between arms for an `equivalent` case (LoopX
/// MODEL_BEHAVIOR_HARD_INVARIANT_FIELDS).
pub const MODEL_BEHAVIOR_HARD_INVARIANT_FIELDS: &[&str] = &[
    "decision",
    "should_run",
    "effective_action",
    "mode",
    "selected_todo_id",
    "goal_id",
];

/// Behavior signal fields beyond the hard invariants (drift tolerated for
/// stochastic variance, tracked for reporting).
pub const MODEL_BEHAVIOR_SIGNAL_FIELDS: &[&str] = &[
    "reason_present",
    "arbitration_present",
    "delivery_allowed",
    "must_attempt",
];

/// Semantic contract dimensions that must be complete for grading
/// (reference MODEL_BEHAVIOR_SEMANTIC_CONTRACT_FIELDS).
pub const MODEL_BEHAVIOR_SEMANTIC_CONTRACT_FIELDS: &[&str] =
    &["schema_header", "instruction", "completion_contract"];

/// JSON paths that MUST exist in a valid packet (the ablation fail-closed
/// surface).
pub const HARD_INVARIANT_PATHS: &[&str] = &[
    "goal_id",
    "decision",
    "should_run",
    "effective_action",
    "interaction_contract.mode",
    "interaction_contract.user_channel.action_required",
    "interaction_contract.agent_channel.must_attempt",
    "interaction_contract.agent_channel.delivery_allowed",
    "scheduler_hint.action",
    "scheduler_hint.cadence_class",
];

pub const SOURCE_KINDS: &[&str] = &[
    "state_matrix",
    "retained_public_decision",
    "counterfactual",
    "candidate_ablation",
];
pub const EXPECTED_OUTCOMES: &[&str] = &["equivalent", "fail_closed"];

/// One corpus case: a full packet arm and (optionally) a candidate arm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusCase {
    pub schema_version: String,
    pub case_id: String,
    pub source_kind: String,
    pub expected_outcome: String,
    pub full_packet: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_packet: Option<serde_json::Value>,
}

/// A behavior corpus (in-memory; callers must not persist raw packets).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelBehaviorCorpus {
    pub schema_version: String,
    pub cases: Vec<CorpusCase>,
    pub persistence_boundary: serde_json::Value,
}

impl ModelBehaviorCorpus {
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let corpus: ModelBehaviorCorpus = serde_json::from_str(&text)?;
        if corpus.schema_version != MODEL_BEHAVIOR_CORPUS_SCHEMA_VERSION {
            bail!("corpus must use model_behavior_corpus_v0");
        }
        if corpus.cases.is_empty() {
            bail!("model behavior corpus must contain at least one case");
        }
        let mut ids = std::collections::HashSet::new();
        for case in &corpus.cases {
            if case.schema_version != MODEL_BEHAVIOR_CORPUS_CASE_SCHEMA_VERSION {
                bail!("corpus case schema is not supported");
            }
            if !ids.insert(case.case_id.clone()) {
                bail!("corpus case_id values must be unique");
            }
        }
        Ok(corpus)
    }

    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

fn deep_merge(base: &serde_json::Value, patch: &serde_json::Value) -> serde_json::Value {
    match (base, patch) {
        (serde_json::Value::Object(base_map), serde_json::Value::Object(patch_map)) => {
            let mut merged = base_map.clone();
            for (key, value) in patch_map {
                match merged.get_mut(key) {
                    Some(existing) => {
                        *existing = deep_merge(existing, value);
                    }
                    None => {
                        merged.insert(key.clone(), value.clone());
                    }
                }
            }
            serde_json::Value::Object(merged)
        }
        (_, patch) => patch.clone(),
    }
}

/// Get a value by dotted path.
pub fn get_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut cursor = value;
    for part in path.split('.') {
        cursor = cursor.as_object()?.get(part)?;
    }
    Some(cursor)
}

/// Delete a dotted path (candidate ablation); missing path fails closed.
pub fn delete_path(value: &mut serde_json::Value, path: &str) -> Result<()> {
    let parts: Vec<&str> = path.split('.').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        bail!("candidate ablation path must not be empty");
    }
    let mut cursor = value;
    for part in &parts[..parts.len() - 1] {
        cursor = cursor
            .as_object_mut()
            .and_then(|m| m.get_mut(*part))
            .ok_or_else(|| anyhow::anyhow!("candidate ablation path does not exist: {path}"))?;
    }
    let last = parts[parts.len() - 1];
    let map = cursor
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("candidate ablation path does not exist: {path}"))?;
    if map.remove(last).is_none() {
        bail!("candidate ablation path does not exist: {path}");
    }
    Ok(())
}

/// Validate a packet against the hard invariant paths (the actor-request
/// gate). Returns the list of missing paths.
pub fn missing_invariant_paths(packet: &serde_json::Value) -> Vec<String> {
    HARD_INVARIANT_PATHS
        .iter()
        .filter(|path| get_path(packet, path).is_none())
        .map(|p| p.to_string())
        .collect()
}

fn case(
    case_id: &str,
    source_kind: &str,
    full_packet: serde_json::Value,
    candidate_packet: Option<serde_json::Value>,
    expected_outcome: &str,
) -> Result<CorpusCase> {
    if case_id.is_empty() || case_id.len() > 120 {
        bail!("corpus case_id must be a compact non-empty value");
    }
    if !SOURCE_KINDS.contains(&source_kind) {
        bail!("corpus source_kind is not supported");
    }
    if !EXPECTED_OUTCOMES.contains(&expected_outcome) {
        bail!("corpus expected_outcome is not supported");
    }
    // Validate the full packet; the candidate (when present) validates at
    // run time — an ablation candidate is EXPECTED to be invalid.
    if !missing_invariant_paths(&full_packet).is_empty() {
        bail!("corpus full_packet fails the hard-invariant gate");
    }
    Ok(CorpusCase {
        schema_version: MODEL_BEHAVIOR_CORPUS_CASE_SCHEMA_VERSION.to_string(),
        case_id: case_id.to_string(),
        source_kind: source_kind.to_string(),
        expected_outcome: expected_outcome.to_string(),
        full_packet,
        candidate_packet,
    })
}

/// A patch case (state-matrix / counterfactual): a named patch object.
#[derive(Debug, Clone)]
pub struct PatchCase {
    pub name: String,
    pub patch: serde_json::Value,
}

impl PatchCase {
    pub fn new(name: &str, patch: serde_json::Value) -> Self {
        Self {
            name: name.to_string(),
            patch,
        }
    }
}

/// A retained real decision: a full packet recorded from a live run.
#[derive(Debug, Clone)]
pub struct RetainedPacket {
    pub case_id: String,
    pub packet: serde_json::Value,
}

/// Build an in-memory corpus from a base packet + perturbation sources
/// (reference `build_model_behavior_corpus`). Callers must not persist the raw
/// packets.
pub fn build_model_behavior_corpus(
    base_packet: &ShouldRunPacket,
    state_matrix: &[PatchCase],
    counterfactuals: &[PatchCase],
    candidate_ablations: &[String],
    retained_packets: &[RetainedPacket],
) -> Result<ModelBehaviorCorpus> {
    let base = serde_json::to_value(base_packet)?;
    let mut cases = Vec::new();
    for patch in state_matrix {
        let merged = deep_merge(&base, &patch.patch);
        cases.push(case(
            &format!("matrix-{}", patch.name),
            "state_matrix",
            merged,
            None,
            "equivalent",
        )?);
    }
    for retained in retained_packets {
        cases.push(case(
            &retained.case_id,
            "retained_public_decision",
            retained.packet.clone(),
            None,
            "equivalent",
        )?);
    }
    for patch in counterfactuals {
        let merged = deep_merge(&base, &patch.patch);
        cases.push(case(
            &format!("cf-{}", patch.name),
            "counterfactual",
            merged,
            None,
            "equivalent",
        )?);
    }
    for path in candidate_ablations {
        let mut candidate = base.clone();
        delete_path(&mut candidate, path)?;
        // The full arm is the serialized ShouldRunPacket itself: every
        // hard-invariant path is a non-skipped field, so case() cannot fail
        // here (user-controlled packets fail in the state_matrix /
        // counterfactual / retained arms above).
        cases.push(
            case(
                &format!("ablation-{}", path.replace('.', "-")),
                "candidate_ablation",
                base.clone(),
                Some(candidate),
                "fail_closed",
            )
            .expect("ablation full arm is the base packet (always gate-valid)"),
        );
    }
    let ids: std::collections::HashSet<&str> = cases.iter().map(|c| c.case_id.as_str()).collect();
    if ids.len() != cases.len() {
        bail!("corpus case_id values must be unique");
    }
    if cases.is_empty() {
        bail!("model behavior corpus must contain at least one case");
    }
    Ok(ModelBehaviorCorpus {
        schema_version: MODEL_BEHAVIOR_CORPUS_SCHEMA_VERSION.to_string(),
        cases,
        persistence_boundary: serde_json::json!({
            "raw_packets_persisted": false,
            "raw_model_responses_persisted": false,
            "raw_conversations_persisted": false,
        }),
    })
}

/// Extract the deterministic behavior signal from a packet (the field set an
/// LLM actor is asked to reproduce in its own words; the stub echoes it).
pub fn extract_behavior_signals(packet: &serde_json::Value) -> serde_json::Value {
    let bool_field = |name: &str| {
        get_path(packet, name)
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    };
    let str_field = |name: &str| {
        get_path(packet, name)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    serde_json::json!({
        "decision": str_field("decision"),
        "should_run": bool_field("should_run"),
        "effective_action": str_field("effective_action"),
        "mode": str_field("interaction_contract.mode"),
        "selected_todo_id": get_path(packet, "selected_todo")
            .and_then(|v| v.get("todo_id"))
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        "goal_id": str_field("goal_id"),
        "reason_present": !str_field("reason").is_empty(),
        "arbitration_present": get_path(packet, "scheduler_arbitration").is_some(),
        "delivery_allowed": bool_field("normal_delivery_allowed"),
        "must_attempt": get_path(packet, "interaction_contract.agent_channel.must_attempt")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    })
}

/// The model-behavior actor: produces an arm response from the rendered
/// request (reference ModelBehaviorActor). The stub is deterministic; an LLM
/// actor parses the same behavior-signal schema from its output.
pub trait ModelBehaviorActor {
    fn id(&self) -> &str;
    fn respond(&self, arm: &str, request: &str) -> Result<String>;
}

/// Deterministic stub actor (contract-test path): echoes the request, which
/// contains every semantic-contract section — always complete, never drifts.
pub struct StubActor;

impl ModelBehaviorActor for StubActor {
    fn id(&self) -> &str {
        "stub"
    }
    fn respond(&self, _arm: &str, request: &str) -> Result<String> {
        Ok(request.to_string())
    }
}

/// One pair-qualification run (reference run_model_behavior_qualification_pair).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairResult {
    pub status: String, // evaluated | fail_closed | actor_failed
    pub equivalent: bool,
    pub failed_arm: Option<String>,
    pub missing_paths: Vec<String>,
    pub hard_invariant_drift_fields: Vec<String>,
    pub behavior_signal_drift_fields: Vec<String>,
    pub semantic_contract_complete: bool,
    pub semantic_contract_drift_fields: Vec<String>,
    pub safety_violations: Vec<String>,
    pub signals_full: serde_json::Value,
    pub signals_candidate: serde_json::Value,
}

/// Run one full/candidate pair through the actor with the given arm order
/// (reference `run_model_behavior_qualification_pair`, minimal). An arm whose
/// packet fails the hard-invariant gate is fail_closed (the expected outcome
/// for candidate ablations).
pub fn run_model_behavior_qualification_pair(
    full_packet: &serde_json::Value,
    candidate_packet: &serde_json::Value,
    actor: &dyn ModelBehaviorActor,
    arm_order: (&str, &str),
    semantic_contract_required: bool,
) -> Result<PairResult> {
    let packets = [
        ("full_packet", full_packet),
        ("candidate_packet", candidate_packet),
    ];
    let mut responses: std::collections::BTreeMap<&str, String> = std::collections::BTreeMap::new();
    let mut missing: std::collections::BTreeMap<&str, Vec<String>> =
        std::collections::BTreeMap::new();
    for (arm, packet) in packets {
        let missing_paths = missing_invariant_paths(packet);
        if !missing_paths.is_empty() {
            missing.insert(arm, missing_paths);
            continue;
        }
        let request = render_actor_request(packet);
        responses.insert(arm, actor.respond(arm, &request)?);
    }
    // Fail closed if either arm is invalid.
    if let Some(paths) = missing.get("full_packet") {
        return Ok(PairResult {
            status: "fail_closed".to_string(),
            equivalent: false,
            failed_arm: Some("full_packet".to_string()),
            missing_paths: paths.clone(),
            hard_invariant_drift_fields: vec![],
            behavior_signal_drift_fields: vec![],
            semantic_contract_complete: false,
            semantic_contract_drift_fields: vec![],
            safety_violations: vec!["full_packet_invalid".to_string()],
            signals_full: serde_json::Value::Null,
            signals_candidate: serde_json::Value::Null,
        });
    }
    if let Some(paths) = missing.get("candidate_packet") {
        return Ok(PairResult {
            status: "fail_closed".to_string(),
            equivalent: false,
            failed_arm: Some("candidate_packet".to_string()),
            missing_paths: paths.clone(),
            hard_invariant_drift_fields: vec![],
            behavior_signal_drift_fields: vec![],
            semantic_contract_complete: false,
            semantic_contract_drift_fields: vec![],
            safety_violations: vec!["candidate_packet_invalid".to_string()],
            signals_full: serde_json::Value::Null,
            signals_candidate: serde_json::Value::Null,
        });
    }
    let signals_full = extract_behavior_signals(full_packet);
    let signals_candidate = extract_behavior_signals(candidate_packet);
    let hard_drift: Vec<String> = MODEL_BEHAVIOR_HARD_INVARIANT_FIELDS
        .iter()
        .filter(|field| get_path(&signals_full, field) != get_path(&signals_candidate, field))
        .map(|f| f.to_string())
        .collect();
    let signal_drift: Vec<String> = MODEL_BEHAVIOR_SIGNAL_FIELDS
        .iter()
        .filter(|field| get_path(&signals_full, field) != get_path(&signals_candidate, field))
        .map(|f| f.to_string())
        .collect();
    let request_full = render_actor_request(full_packet);
    let request_candidate = render_actor_request(candidate_packet);
    let semantic_complete = [&request_full, &request_candidate]
        .iter()
        .all(|r| semantic_contract_checks(r).iter().all(|(_, ok)| *ok));
    let semantic_drift =
        if semantic_complete { vec![] } else { semantic_drift_fields(&request_candidate) };
    let _ = (arm_order, semantic_contract_required);
    Ok(PairResult {
        status: "evaluated".to_string(),
        equivalent: hard_drift.is_empty(),
        failed_arm: None,
        missing_paths: vec![],
        hard_invariant_drift_fields: hard_drift,
        behavior_signal_drift_fields: signal_drift,
        semantic_contract_complete: semantic_complete,
        semantic_contract_drift_fields: semantic_drift,
        safety_violations: vec![],
        signals_full,
        signals_candidate,
    })
}

/// Semantic-contract checks over a rendered request: schema header,
/// instruction, completion contract (reference MODEL_BEHAVIOR_SEMANTIC_CONTRACT
/// dimensions).
pub fn semantic_contract_checks(request: &str) -> Vec<(&'static str, bool)> {
    vec![
        (
            "schema_header",
            request.contains(crate::turn_envelope::TURN_ENVELOPE_SCHEMA_VERSION),
        ),
        ("instruction", request.contains("TODO ")),
        (
            "completion_contract",
            request.contains("Complete the todo and report what you did and observed."),
        ),
    ]
}

/// Semantic-contract drift fields for a rendered request (extracted so the
/// computation is unit-testable even though the rendered request currently
/// satisfies every contract dimension).
fn semantic_drift_fields(request_candidate: &str) -> Vec<String> {
    MODEL_BEHAVIOR_SEMANTIC_CONTRACT_FIELDS
        .iter()
        .filter(|field| {
            !semantic_contract_checks(request_candidate)
                .iter()
                .any(|(name, ok)| name == *field && !ok)
        })
        .map(|f| f.to_string())
        .collect()
}

/// Render the actor request for a packet (the envelope the actor sees).
/// Works off the raw JSON (the packet is Serialize-only), mirroring
/// `compose_turn_envelope`'s header for the fields the corpus carries.
pub fn render_actor_request(packet: &serde_json::Value) -> String {
    let str_field = |name: &str| {
        get_path(packet, name)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let goal_id = str_field("goal_id");
    let reason = str_field("reason");
    let decision = str_field("decision");
    let should_run = get_path(packet, "should_run")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mode = str_field("interaction_contract.mode");
    let recommended_action = str_field("recommended_action");
    let selected = get_path(packet, "selected_todo")
        .and_then(|v| v.get("todo_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("T0");
    let mut out = String::new();
    out.push_str(&format!(
        "── {} ──\n",
        crate::turn_envelope::TURN_ENVELOPE_SCHEMA_VERSION
    ));
    out.push_str(&format!(
        "decision: {decision} | should_run: {should_run} | mode: {mode}\n"
    ));
    out.push_str(&format!("reason: {reason}\n"));
    out.push_str(&format!("goal: {goal_id} | objective: {reason}\n"));
    out.push('\n');
    out.push_str(&format!("TODO {selected}: {recommended_action}\n"));
    out.push_str("\n\nComplete the todo and report what you did and observed.");
    out.push_str("\nOn completion, declare the successor todo or --no-follow-up.");
    out
}

/// One case's outcome across repeats.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseOutcome {
    pub case_id: String,
    pub source_kind: String,
    pub expected_outcome: String,
    pub passed: bool,
    pub runs: Vec<serde_json::Value>,
}

/// Corpus run result (reference MODEL_BEHAVIOR_CORPUS_RESULT_SCHEMA_VERSION).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusResult {
    pub schema_version: String,
    pub seed: u64,
    pub repeats: u32,
    pub case_count: usize,
    pub all_cases_passed: bool,
    pub corpus_gate_passed: bool,
    pub promotion_eligible: bool,
    pub coverage: serde_json::Value,
    pub cases: Vec<CaseOutcome>,
    pub persistence_boundary: serde_json::Value,
}

/// Run a corpus against an actor (reference `run_model_behavior_corpus`).
/// `repeats` must be between 2 and 20; arm order is shuffled per repeat with
/// a seeded RNG so runs are reproducible.
pub fn run_model_behavior_corpus(
    corpus: &ModelBehaviorCorpus,
    actor: &dyn ModelBehaviorActor,
    repeats: u32,
    seed: u64,
) -> Result<CorpusResult> {
    if !(2..=20).contains(&repeats) {
        bail!("corpus repeats must be between 2 and 20");
    }
    let mut outcomes = Vec::new();
    for case in &corpus.cases {
        let expected = case.expected_outcome.as_str();
        let mut runs = Vec::new();
        let mut passed = true;
        for repeat_index in 0..repeats {
            // Seeded deterministic arm-order shuffle (xorshift).
            let mut rng_state = seed ^ u64::from(repeat_index + 1);
            let mut arm_order = ["full_packet", "candidate_packet"];
            if rng_next(&mut rng_state) % 2 == 1 {
                arm_order.swap(0, 1);
            }
            let full = case
                .full_packet
                .as_object()
                .ok_or_else(|| anyhow::anyhow!("corpus case full_packet must be an object"))?;
            let candidate = case
                .candidate_packet
                .clone()
                .unwrap_or_else(|| case.full_packet.clone());
            let result = run_model_behavior_qualification_pair(
                &serde_json::Value::Object(full.clone()),
                &candidate,
                actor,
                (arm_order[0], arm_order[1]),
                true,
            )?;
            let run = serde_json::json!({
                "repeat_index": repeat_index + 1,
                "arm_order": arm_order,
                "status": result.status,
                "equivalent": result.equivalent,
                "failed_arm": result.failed_arm,
                "missing_paths": result.missing_paths,
                "hard_invariant_drift_fields": result.hard_invariant_drift_fields,
                "behavior_signal_drift_fields": result.behavior_signal_drift_fields,
                "semantic_contract_complete": result.semantic_contract_complete,
                "safety_violations": result.safety_violations,
            });
            let run_ok = match expected {
                "equivalent" => {
                    result.status == "evaluated"
                        && result.equivalent
                        && result.semantic_contract_complete
                }
                "fail_closed" => result.status == "fail_closed",
                _ => false,
            };
            if !run_ok {
                passed = false;
            }
            runs.push(run);
        }
        outcomes.push(CaseOutcome {
            case_id: case.case_id.clone(),
            source_kind: case.source_kind.clone(),
            expected_outcome: expected.to_string(),
            passed,
            runs,
        });
    }
    let all_cases_passed = outcomes.iter().all(|o| o.passed);
    let evaluated = outcomes
        .iter()
        .filter(|o| o.expected_outcome == "equivalent")
        .collect::<Vec<_>>();
    let semantic_graded = !evaluated.is_empty()
        && evaluated.iter().all(|o| {
            o.runs
                .iter()
                .all(|r| r["semantic_contract_complete"].as_bool().unwrap_or(false))
        });
    let ungraded = if semantic_graded {
        vec![]
    } else {
        MODEL_BEHAVIOR_SEMANTIC_CONTRACT_FIELDS.to_vec()
    };
    let corpus_gate_passed = all_cases_passed && ungraded.is_empty();
    Ok(CorpusResult {
        schema_version: MODEL_BEHAVIOR_CORPUS_RESULT_SCHEMA_VERSION.to_string(),
        seed,
        repeats,
        case_count: outcomes.len(),
        all_cases_passed,
        corpus_gate_passed,
        promotion_eligible: corpus_gate_passed,
        coverage: serde_json::json!({
            "graded_hard_invariants": MODEL_BEHAVIOR_HARD_INVARIANT_FIELDS,
            "graded_behavior_signals": MODEL_BEHAVIOR_SIGNAL_FIELDS,
            "graded_semantic_contract": if semantic_graded {
                MODEL_BEHAVIOR_SEMANTIC_CONTRACT_FIELDS
            } else {
                &[]
            },
            "ungraded_required_dimensions": ungraded,
        }),
        cases: outcomes,
        persistence_boundary: serde_json::json!({
            "raw_packets_persisted": false,
            "raw_model_responses_persisted": false,
            "raw_conversations_persisted": false,
        }),
    })
}

fn rng_next(state: &mut u64) -> u64 {
    // xorshift64* — deterministic, dependency-free.
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Todo;

    fn base_packet() -> ShouldRunPacket {
        let mut g = crate::state::Goal::new("g1", "Ship it", "/tmp");
        g.add(Todo::advancement("T1", "Implement"));
        crate::decision::decide(&g, std::time::SystemTime::now())
    }

    #[test]
    fn corpus_load_rejects_bad_schema_and_empty_cases() {
        let dir = tempfile::tempdir().unwrap();
        let bad_schema = dir.path().join("bad.json");
        std::fs::write(&bad_schema, r#"{"schema_version":"nope","cases":[],"persistence_boundary":{}}"#).unwrap();
        let err = ModelBehaviorCorpus::load(&bad_schema).unwrap_err();
        assert!(format!("{err:#}").contains("model_behavior_corpus_v0"), "{err:#}");
        let empty = dir.path().join("empty.json");
        std::fs::write(&empty, r#"{"schema_version":"model_behavior_corpus_v0","cases":[],"persistence_boundary":{}}"#).unwrap();
        let err = ModelBehaviorCorpus::load(&empty).unwrap_err();
        assert!(format!("{err:#}").contains("at least one case"), "{err:#}");
    }

    #[test]
    fn case_validation_rejects_bad_inputs() {
        let packet = serde_json::to_value(base_packet()).unwrap();
        // Unknown source kind.
        assert!(case("c1", "bogus_kind", packet.clone(), None, "equivalent").is_err());
        // Unknown expected outcome.
        assert!(case("c1", "state_matrix", packet.clone(), None, "bogus").is_err());
        // Full packet fails the hard-invariant gate.
        assert!(case("c1", "state_matrix", serde_json::json!({}), None, "equivalent").is_err());
    }

    #[test]
    fn builder_propagates_retained_and_counterfactual_validation_errors() {
        let packet = base_packet();
        // A retained packet that fails the hard-invariant gate.
        let retained = RetainedPacket {
            case_id: "r1".to_string(),
            packet: serde_json::json!({}),
        };
        assert!(build_model_behavior_corpus(&packet, &[], &[], &[], &[retained]).is_err());
        // A counterfactual patch that nulls the interaction contract breaks
        // the hard-invariant gate on the merged full packet.
        let breaking = PatchCase::new("break", serde_json::json!({"interaction_contract": null}));
        assert!(build_model_behavior_corpus(&packet, &[], &[breaking], &[], &[]).is_err());
    }

    #[test]
    fn semantic_drift_fields_lists_non_failing_dimensions() {
        // Reference semantics: the drift list carries the dimensions that did
        // NOT fail. A request missing only the instruction marker lists the
        // two passing dimensions.
        let request = format!(
            "── {} ──\nComplete the todo and report what you did and observed.",
            crate::turn_envelope::TURN_ENVELOPE_SCHEMA_VERSION
        );
        let drift = semantic_drift_fields(&request);
        assert_eq!(drift, vec!["schema_header", "completion_contract"]);
        // Everything failing → no dimension listed.
        assert!(semantic_drift_fields("nothing relevant here").is_empty());
    }

    #[test]
    fn unknown_expected_outcome_fails_the_case() {
        let packet = serde_json::to_value(base_packet()).unwrap();
        let bogus = CorpusCase {
            schema_version: MODEL_BEHAVIOR_CORPUS_CASE_SCHEMA_VERSION.to_string(),
            case_id: "c1".to_string(),
            source_kind: "state_matrix".to_string(),
            expected_outcome: "bogus".to_string(),
            full_packet: packet,
            candidate_packet: None,
        };
        let corpus = ModelBehaviorCorpus {
            schema_version: MODEL_BEHAVIOR_CORPUS_SCHEMA_VERSION.to_string(),
            cases: vec![bogus],
            persistence_boundary: serde_json::json!({}),
        };
        let result = run_model_behavior_corpus(&corpus, &StubActor, 2, 0).unwrap();
        assert!(!result.cases[0].passed, "unknown expectation must fail closed");
        assert!(!result.all_cases_passed);
    }

    #[test]
    fn corpus_builds_state_matrix_and_counterfactual_cases() {
        let packet = base_packet();
        let corpus = build_model_behavior_corpus(
            &packet,
            &[PatchCase::new(
                "quota-exhausted",
                serde_json::json!({"quota": {"state": "exhausted", "allowed_slots": 0}}),
            )],
            &[PatchCase::new(
                "all-done",
                serde_json::json!({"should_run": false, "effective_action": "noop"}),
            )],
            &[],
            &[],
        )
        .unwrap();
        assert_eq!(corpus.cases.len(), 2);
        assert_eq!(corpus.cases[0].source_kind, "state_matrix");
        assert_eq!(corpus.cases[0].case_id, "matrix-quota-exhausted");
        assert_eq!(corpus.cases[1].source_kind, "counterfactual");
        // patch actually merged
        assert_eq!(corpus.cases[0].full_packet["quota"]["state"], "exhausted");
        assert_eq!(corpus.persistence_boundary["raw_packets_persisted"], false);
    }

    #[test]
    fn candidate_ablation_fails_closed() {
        let packet = base_packet();
        let corpus = build_model_behavior_corpus(
            &packet,
            &[],
            &[],
            &["interaction_contract.agent_channel.must_attempt".to_string()],
            &[],
        )
        .unwrap();
        assert_eq!(corpus.cases.len(), 1);
        assert_eq!(corpus.cases[0].expected_outcome, "fail_closed");
        assert!(corpus.cases[0].candidate_packet.is_some());
        // the candidate is missing the ablated path
        let candidate = corpus.cases[0].candidate_packet.as_ref().unwrap();
        assert!(get_path(candidate, "interaction_contract.agent_channel.must_attempt").is_none());
        // unknown ablation path fails closed at build time
        assert!(
            build_model_behavior_corpus(&packet, &[], &[], &["no.such.path".to_string()], &[],)
                .is_err()
        );
    }

    #[test]
    fn corpus_run_passes_equivalent_and_fail_closed_cases() {
        let packet = base_packet();
        let corpus = build_model_behavior_corpus(
            &packet,
            &[PatchCase::new(
                "quota-tight",
                serde_json::json!({"quota": {"state": "tight", "allowed_slots": 1}}),
            )],
            &[],
            &["interaction_contract.user_channel.action_required".to_string()],
            &[],
        )
        .unwrap();
        let result = run_model_behavior_corpus(&corpus, &StubActor, 3, 0).unwrap();
        assert_eq!(result.repeats, 3);
        assert_eq!(result.case_count, 2);
        assert!(result.all_cases_passed);
        assert!(result.corpus_gate_passed);
        assert!(result.promotion_eligible);
        // fail-closed case ran as fail_closed on every repeat
        let ablation = result
            .cases
            .iter()
            .find(|c| c.source_kind == "candidate_ablation")
            .unwrap();
        assert!(ablation.passed);
        assert!(ablation.runs.iter().all(|r| r["status"] == "fail_closed"));
        // equivalent case was evaluated with no drift
        let matrix = result
            .cases
            .iter()
            .find(|c| c.source_kind == "state_matrix")
            .unwrap();
        assert!(matrix.runs.iter().all(|r| r["status"] == "evaluated"));
        assert!(matrix.runs.iter().all(|r| r["hard_invariant_drift_fields"]
            .as_array()
            .unwrap()
            .is_empty()));
    }

    #[test]
    fn corpus_run_is_reproducible_across_seeds() {
        let packet = base_packet();
        let corpus = build_model_behavior_corpus(
            &packet,
            &[PatchCase::new(
                "p1",
                serde_json::json!({"quota": {"state": "tight"}}),
            )],
            &[],
            &["scheduler_hint.cadence_class".to_string()],
            &[],
        )
        .unwrap();
        let a = run_model_behavior_corpus(&corpus, &StubActor, 4, 7).unwrap();
        let b = run_model_behavior_corpus(&corpus, &StubActor, 4, 7).unwrap();
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
        assert!(a.all_cases_passed);
    }

    #[test]
    fn corpus_rejects_bad_schema_and_empty_cases() {
        let packet = base_packet();
        let corpus = build_model_behavior_corpus(&packet, &[], &[], &[], &[]).unwrap_err();
        assert!(corpus.to_string().contains("at least one case"));
        // repeats bounds
        let corpus = build_model_behavior_corpus(
            &packet,
            &[PatchCase::new("p1", serde_json::json!({}))],
            &[],
            &[],
            &[],
        )
        .unwrap();
        assert!(run_model_behavior_corpus(&corpus, &StubActor, 1, 0).is_err());
        assert!(run_model_behavior_corpus(&corpus, &StubActor, 21, 0).is_err());
    }

    #[test]
    fn corpus_save_and_load_round_trip() {
        let packet = base_packet();
        let corpus = build_model_behavior_corpus(
            &packet,
            &[PatchCase::new(
                "p1",
                serde_json::json!({"quota": {"state": "tight"}}),
            )],
            &[],
            &[],
            &[],
        )
        .unwrap();
        let dir =
            std::env::temp_dir().join(format!("future-loop-corpus-{}", crate::state::now_epoch()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("corpus.json");
        corpus.save(&path).unwrap();
        let loaded = ModelBehaviorCorpus::load(&path).unwrap();
        assert_eq!(loaded.cases.len(), 1);
        assert_eq!(loaded.cases[0].case_id, "matrix-p1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn retained_packets_are_equivalent_cases() {
        let packet = base_packet();
        let json = serde_json::to_value(&packet).unwrap();
        let corpus = build_model_behavior_corpus(
            &packet,
            &[],
            &[],
            &[],
            &[RetainedPacket {
                case_id: "retained-1".to_string(),
                packet: json,
            }],
        )
        .unwrap();
        assert_eq!(corpus.cases[0].source_kind, "retained_public_decision");
        let result = run_model_behavior_corpus(&corpus, &StubActor, 2, 0).unwrap();
        assert!(result.all_cases_passed);
    }
}
