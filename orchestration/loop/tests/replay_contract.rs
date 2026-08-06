//! G-19 replay & model-behavior corpus contract tests — the LLM-side
//! extension of the contract tests, exercised deterministically: record a
//! real kernel decision → public-safe reduce (banned keys / local paths fail
//! closed) → replay against the kernel → per-field diff; corpus build
//! (state matrix / counterfactual / ablation) → run with the stub actor →
//! equivalent / fail-closed per case.

use future_loop::replay::corpus::{
    build_model_behavior_corpus, run_model_behavior_corpus, ModelBehaviorCorpus, PatchCase,
    StubActor,
};
use future_loop::replay::decision_replay::{
    reduce_public_safe_decision, replay_public_safe_decision_case,
    validate_public_safe_decision_case, walk_public_safe_violations, DecisionReplay,
};
use future_loop::state::{Goal, Todo};

fn sample_goal() -> Goal {
    let mut g = Goal::new("g1", "Ship the thing", "/tmp");
    g.add(Todo::advancement("T1", "Implement the feature"));
    g.add(Todo::user_gate("G1", "Approve the release", &["T2"]));
    g.add(Todo::advancement("T2", "Release after approval").blocking(&["G1"]));
    g
}

// ── decision replay ───────────────────────────────────────────────────────

#[test]
fn record_reduce_replay_round_trip_matches() {
    let goal = sample_goal();
    let packet = future_loop::decision::decide(&goal, std::time::SystemTime::now());
    let case = reduce_public_safe_decision(&packet, &goal, "case-1", Some("agent-a"));
    assert_eq!(case.schema_version, "public_safe_decision_case_v0");
    assert_eq!(case.agent_id, "agent-a");
    validate_public_safe_decision_case(&case).unwrap();
    let comparison = replay_public_safe_decision_case(&case).unwrap();
    assert!(comparison.matched, "{:?}", comparison.mismatches);
    assert!(comparison.decision.iter().all(|(_, m)| *m));
    assert!(comparison.expected.iter().all(|(_, m)| *m));
    assert!(comparison.interaction.iter().all(|(_, m)| *m));
}

#[test]
fn replay_detects_recorded_drift() {
    let goal = sample_goal();
    let packet = future_loop::decision::decide(&goal, std::time::SystemTime::now());
    let mut case = reduce_public_safe_decision(&packet, &goal, "case-1", None);
    case.expected.scheduler_action = "different".to_string();
    let comparison = replay_public_safe_decision_case(&case).unwrap();
    assert!(!comparison.matched);
    assert!(comparison
        .mismatches
        .iter()
        .any(|m| m.contains("expected.scheduler_action")));
    // a changed decision field is caught too
    let case2 = reduce_public_safe_decision(&packet, &goal, "case-1", None);
    let comparison = replay_public_safe_decision_case(&case2).unwrap();
    assert!(comparison.matched, "unmodified case must still match");
}

#[test]
fn public_safety_reduction_blocks_secrets_and_paths() {
    let goal = sample_goal();
    let packet = future_loop::decision::decide(&goal, std::time::SystemTime::now());
    let case = reduce_public_safe_decision(&packet, &goal, "case-1", None);
    // banned KEY
    let mut value = serde_json::to_value(&case).unwrap();
    value["credentials"] = serde_json::json!({"token": "sekret"});
    assert!(walk_public_safe_violations(&value)
        .iter()
        .any(|v| v.contains("banned key")));
    // local PATH value
    let mut case2 = reduce_public_safe_decision(&packet, &goal, "case-1", None);
    case2.expected.scheduler_action = "/home/user/.ssh/config".to_string();
    assert!(validate_public_safe_decision_case(&case2).is_err());
}

#[test]
fn replay_file_round_trip() {
    let dir = std::env::temp_dir().join(format!(
        "loopx-p4-replay-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let goal = sample_goal();
    let packet = future_loop::decision::decide(&goal, std::time::SystemTime::now());
    let mut replay = DecisionReplay::new();
    replay.add(reduce_public_safe_decision(&packet, &goal, "case-1", None));
    let path = dir.join("replay.json");
    replay.save(&path).unwrap();
    let loaded = DecisionReplay::load(&path).unwrap();
    assert_eq!(loaded.cases.len(), 1);
    assert_eq!(loaded.cases[0].case_id, "case-1");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── model behavior corpus ─────────────────────────────────────────────────

fn base_packet() -> future_loop::contract::ShouldRunPacket {
    let mut g = Goal::new("g1", "Ship it", "/tmp");
    g.add(Todo::advancement("T1", "Implement"));
    future_loop::decision::decide(&g, std::time::SystemTime::now())
}

#[test]
fn corpus_builds_equivalent_and_ablation_cases() {
    let packet = base_packet();
    let corpus = build_model_behavior_corpus(
        &packet,
        &[PatchCase::new(
            "quota-tight",
            serde_json::json!({"quota": {"state": "tight"}}),
        )],
        &[PatchCase::new(
            "all-done",
            serde_json::json!({"should_run": false}),
        )],
        &["interaction_contract.user_channel.action_required".to_string()],
        &[],
    )
    .unwrap();
    assert_eq!(corpus.cases.len(), 3);
    let kinds: Vec<&str> = corpus
        .cases
        .iter()
        .map(|c| c.source_kind.as_str())
        .collect();
    assert_eq!(
        kinds,
        vec!["state_matrix", "counterfactual", "candidate_ablation"]
    );
    // state-matrix patch actually merged
    assert_eq!(corpus.cases[0].full_packet["quota"]["state"], "tight");
    // persistence boundary: raw packets never persisted
    assert_eq!(corpus.persistence_boundary["raw_packets_persisted"], false);
}

#[test]
fn corpus_run_gates_on_all_cases() {
    let packet = base_packet();
    let corpus = build_model_behavior_corpus(
        &packet,
        &[PatchCase::new(
            "tight",
            serde_json::json!({"quota": {"state": "tight"}}),
        )],
        &[],
        &["scheduler_hint.cadence_class".to_string()],
        &[],
    )
    .unwrap();
    let result = run_model_behavior_corpus(&corpus, &StubActor, 3, 0).unwrap();
    assert_eq!(result.repeats, 3);
    assert_eq!(result.case_count, 2);
    assert!(result.all_cases_passed);
    assert!(result.corpus_gate_passed);
    assert!(result.promotion_eligible);
    // ablation case failed closed on every repeat
    let ablation = result
        .cases
        .iter()
        .find(|c| c.source_kind == "candidate_ablation")
        .unwrap();
    assert!(ablation.runs.iter().all(|r| r["status"] == "fail_closed"));
    // equivalent case evaluated with no hard-invariant drift
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
fn corpus_is_reproducible_across_runs() {
    let packet = base_packet();
    let corpus = build_model_behavior_corpus(
        &packet,
        &[PatchCase::new(
            "tight",
            serde_json::json!({"quota": {"state": "tight"}}),
        )],
        &[],
        &["goal_id".to_string()],
        &[],
    )
    .unwrap();
    let a = run_model_behavior_corpus(&corpus, &StubActor, 4, 7).unwrap();
    let b = run_model_behavior_corpus(&corpus, &StubActor, 4, 7).unwrap();
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap()
    );
}

#[test]
fn ablation_of_unknown_path_fails_closed_at_build() {
    let packet = base_packet();
    let err = build_model_behavior_corpus(&packet, &[], &[], &["no.such.path".to_string()], &[])
        .unwrap_err();
    assert!(err.to_string().contains("does not exist"));
}

#[test]
fn corpus_file_round_trip() {
    let dir = std::env::temp_dir().join(format!(
        "loopx-p4-corpus-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
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
    let path = dir.join("corpus.json");
    corpus.save(&path).unwrap();
    let loaded = ModelBehaviorCorpus::load(&path).unwrap();
    assert_eq!(loaded.cases.len(), 1);
    assert_eq!(loaded.cases[0].case_id, "matrix-p1");
    let _ = std::fs::remove_dir_all(&dir);
}
