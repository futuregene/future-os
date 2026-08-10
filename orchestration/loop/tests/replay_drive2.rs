//! Coverage drive 2 for `replay/`: hand-crafted corpus JSON reaching the
//! fail-closed full-packet arm and the semantic-drift gate, plus the
//! decision-replay per-group mismatch arms.

use future_loop::replay::corpus::{
    build_model_behavior_corpus, run_model_behavior_corpus, ModelBehaviorCorpus, PatchCase, StubActor,
};
use future_loop::replay::decision_replay::{
    reduce_public_safe_decision, replay_public_safe_decision_case,
};
use future_loop::state::{Goal, Todo};

fn base_packet_json() -> serde_json::Value {
    let mut goal = Goal::new("g1", "replay drive 2", "/tmp");
    goal.todos.push(Todo::advancement("t1", "work"));
    let packet = future_loop::decision::decide_for(&goal, std::time::SystemTime::now(), None);
    serde_json::to_value(&packet).unwrap()
}

fn corpus_json_with(cases: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "schema_version": "model_behavior_corpus_v0",
        "persistence_boundary": {"note": "in-memory test corpus"},
        "cases": cases,
    })
}

fn case_json(id: &str, expected: &str, full: serde_json::Value, candidate: Option<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "schema_version": "model_behavior_corpus_case_v0",
        "case_id": id,
        "source_kind": "state_matrix",
        "expected_outcome": expected,
        "full_packet": full,
        "candidate_packet": candidate,
    })
}

#[test]
fn corpus_run_full_packet_fail_closed() {
    // full_packet missing hard invariants → the pair fails closed before the
    // actor runs; expected fail_closed → the case passes the gate.
    let case = case_json("c-fail", "fail_closed", serde_json::json!({}), None);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corpus.json");
    std::fs::write(&path, serde_json::to_string(&corpus_json_with(vec![case])).unwrap()).unwrap();
    let corpus = ModelBehaviorCorpus::load(&path).unwrap();
    let result = run_model_behavior_corpus(&corpus, &StubActor, 2, 0).unwrap();
    assert!(result.all_cases_passed, "{:?}", result.cases.iter().map(|c| (&c.case_id, c.passed)).collect::<Vec<_>>());
}

#[test]
fn corpus_run_semantic_drift_fails_gate() {
    // candidate = full minus the selected todo fields → the rendered request
    // loses the "TODO " section → semantic contract incomplete → equivalent
    // case fails → corpus gate bails.
    let full = base_packet_json();
    let mut candidate = full.clone();
    {
        let obj = candidate.as_object_mut().unwrap();
        obj.remove("selected_todo");
        obj.remove("selected_todo_id");
    }
    let case = case_json("c-drift", "equivalent", full, Some(candidate));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corpus.json");
    std::fs::write(&path, serde_json::to_string(&corpus_json_with(vec![case])).unwrap()).unwrap();
    let corpus = ModelBehaviorCorpus::load(&path).unwrap();
    let result = run_model_behavior_corpus(&corpus, &StubActor, 2, 0).unwrap();
    assert!(!result.all_cases_passed);
    assert!(!result.corpus_gate_passed);
    assert!(!result.promotion_eligible);
    // And the case reports the failed arm detail.
    let c = &result.cases[0];
    assert!(!c.passed);
    assert!(!c.runs.is_empty());
}

#[test]
fn corpus_build_patch_merge_arms() {
    // deep_merge: object-into-object merges, scalar replaces, array replaces.
    let mut goal = Goal::new("g", "merge", "/tmp");
    goal.todos.push(Todo::advancement("t1", "w"));
    let packet = future_loop::decision::decide_for(&goal, std::time::SystemTime::now(), None);
    let corpus = build_model_behavior_corpus(
        &packet,
        &[
            PatchCase::new("nested", serde_json::json!({"quota": {"spent_slots": 3}})),
            PatchCase::new("scalar", serde_json::json!({"should_run": false})),
            PatchCase::new("array", serde_json::json!({"boundary": {"leaks": ["x"]}})),
        ],
        &[],
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(corpus.cases.len(), 3);
    let merged: serde_json::Value = corpus.cases[0].full_packet.clone();
    assert_eq!(merged["quota"]["spent_slots"], serde_json::json!(3));
}

#[test]
fn decision_replay_mismatch_groups() {
    let mut goal = Goal::new("g", "tamper", "/tmp");
    goal.todos.push(Todo::advancement("t1", "work"));
    let packet = future_loop::decision::decide_for(&goal, std::time::SystemTime::now(), None);
    let case = reduce_public_safe_decision(&packet, &goal, "c1", None);
    assert_eq!(case.case_id(), "c1");
    // Tamper one field per comparison group: decision / expected /
    // interaction (string + Option + bool closure).
    let mut t1 = case.clone();
    t1.decision.effective_action = "tampered".to_string();
    let cmp = replay_public_safe_decision_case(&t1).unwrap();
    assert!(cmp.mismatches.iter().any(|m| m.contains("decision.effective_action")));

    let mut t2 = case.clone();
    t2.expected.scheduler_action = "tampered".to_string();
    let cmp = replay_public_safe_decision_case(&t2).unwrap();
    assert!(cmp.mismatches.iter().any(|m| m.contains("expected.scheduler_action")));

    let mut t3 = case.clone();
    t3.interaction_contract.mode = "terminal".to_string();
    let cmp = replay_public_safe_decision_case(&t3).unwrap();
    assert!(cmp.mismatches.iter().any(|m| m.contains("interaction.mode")));

    let mut t4 = case.clone();
    t4.interaction_contract.user_channel.notify = Some("tampered".to_string());
    let cmp = replay_public_safe_decision_case(&t4).unwrap();
    assert!(cmp.mismatches.iter().any(|m| m.contains("user_channel.notify")));

    let mut t5 = case.clone();
    t5.interaction_contract.agent_channel.must_attempt = Some(!case.interaction_contract.agent_channel.must_attempt.unwrap_or(false));
    let cmp = replay_public_safe_decision_case(&t5).unwrap();
    assert!(cmp.mismatches.iter().any(|m| m.contains("must_attempt")), "{:?}", cmp.mismatches);
}
