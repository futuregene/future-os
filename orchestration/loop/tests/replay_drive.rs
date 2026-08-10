//! Coverage drive for `replay/`: corpus load/validation errors, deep_merge /
//! get_path / delete_path, case-construction gates, the fail-closed ablation
//! path, and decision-replay load/reduce/walk/replay branch matrices.

use future_loop::replay::corpus::ModelBehaviorActor;
use future_loop::replay::corpus::{
    build_model_behavior_corpus, delete_path, get_path, run_model_behavior_corpus,
    ModelBehaviorCorpus, PatchCase, RetainedPacket, StubActor,
};
use future_loop::replay::decision_replay::{
    reduce_public_safe_decision, replay_public_safe_decision_case,
    validate_public_safe_decision_case, walk_public_safe_violations, DecisionReplay,
};
use future_loop::state::{Goal, Todo, TodoStatus};

fn packet_for(goal: &Goal) -> future_loop::contract::ShouldRunPacket {
    future_loop::decision::decide_for(goal, std::time::SystemTime::now(), None)
}

fn base_goal() -> Goal {
    Goal::new("g1", "replay drive", "/tmp")
}

// ── corpus: load / validation ──────────────────────────────────────────────

#[test]
fn corpus_load_errors() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope.json");
    assert!(ModelBehaviorCorpus::load(&missing).is_err());
    let bad_json = dir.path().join("bad.json");
    std::fs::write(&bad_json, "{nope").unwrap();
    assert!(ModelBehaviorCorpus::load(&bad_json).is_err());
    let bad_schema = dir.path().join("schema.json");
    std::fs::write(&bad_schema, "{\"schema_version\":\"nope\",\"cases\":[]}").unwrap();
    assert!(ModelBehaviorCorpus::load(&bad_schema).is_err());
    // Empty case list.
    let goal = base_goal();
    let packet = packet_for(&goal);
    let corpus = build_model_behavior_corpus(
        &packet,
        &[PatchCase::new("p", serde_json::json!({}))],
        &[],
        &[],
        &[],
    )
    .unwrap();
    let empty = dir.path().join("empty.json");
    std::fs::write(
        &empty,
        "{\"schema_version\":\"model_behavior_corpus_v0\",\"cases\":[]}",
    )
    .unwrap();
    assert!(ModelBehaviorCorpus::load(&empty).is_err());
    // Case schema mismatch.
    let mut value = serde_json::to_value(&corpus).unwrap();
    value["cases"][0]
        .as_object_mut()
        .unwrap()
        .insert("schema_version".into(), "nope".into());
    let p = dir.path().join("case-schema.json");
    std::fs::write(&p, serde_json::to_string(&value).unwrap()).unwrap();
    assert!(ModelBehaviorCorpus::load(&p).is_err());
    // Duplicate case ids.
    let mut value = serde_json::to_value(&corpus).unwrap();
    let dup = value["cases"][0].clone();
    value["cases"].as_array_mut().unwrap().push(dup);
    let p = dir.path().join("dup.json");
    std::fs::write(&p, serde_json::to_string(&value).unwrap()).unwrap();
    assert!(ModelBehaviorCorpus::load(&p).is_err());
}

// ── corpus: path helpers + build matrix ────────────────────────────────────

#[test]
fn corpus_path_helpers() {
    let v = serde_json::json!({"a": {"b": {"c": 1}}, "list": [1]});
    assert_eq!(get_path(&v, "a.b.c"), Some(&serde_json::json!(1)));
    assert_eq!(get_path(&v, "a.b.missing"), None);
    assert_eq!(get_path(&v, "a.b.c.deeper"), None, "non-object mid-path");
    assert_eq!(get_path(&v, "list.x"), None);
    // delete_path.
    let mut v = serde_json::json!({"a": {"b": 1}, "c": 2});
    assert!(delete_path(&mut v, "").is_err());
    assert!(delete_path(&mut v, "a.missing.deep").is_err());
    assert!(delete_path(&mut v, "a.nope").is_err());
    assert!(delete_path(&mut v, "c.x").is_err(), "non-object cursor");
    delete_path(&mut v, "a.b").unwrap();
    assert_eq!(v["a"], serde_json::json!({}));
}

#[test]
fn corpus_build_matrix() {
    let goal = base_goal();
    let packet = packet_for(&goal);
    // state_matrix + counterfactuals + retained + ablation (fail-closed on
    // a hard-invariant path passes the gate).
    let retained = RetainedPacket {
        case_id: "retained-1".into(),
        packet: serde_json::to_value(&packet).unwrap(),
    };
    let corpus = build_model_behavior_corpus(
        &packet,
        &[
            PatchCase::new("a", serde_json::json!({"quota": {"extra": 1}})),
            PatchCase::new("b", serde_json::json!({"decision": "replaced"})),
        ],
        &[PatchCase::new(
            "cf",
            serde_json::json!({"should_run": false}),
        )],
        &["decision".to_string()],
        &[retained],
    )
    .unwrap();
    assert_eq!(corpus.cases.len(), 5);
    // Ablation of a hard-invariant path → candidate invalid → fail_closed ✓.
    let result = run_model_behavior_corpus(&corpus, &StubActor, 2, 0).unwrap();
    assert!(
        result.all_cases_passed,
        "{:#?}",
        result
            .cases
            .iter()
            .map(|c| (&c.case_id, c.passed))
            .collect::<Vec<_>>()
    );
    assert!(result.corpus_gate_passed);
    assert!(result.promotion_eligible);
    // Ablation of a nonexistent path → build error.
    assert!(
        build_model_behavior_corpus(&packet, &[], &[], &["nope.nope".to_string()], &[]).is_err()
    );
    // Duplicate ablation paths → duplicate case ids → error.
    assert!(build_model_behavior_corpus(
        &packet,
        &[],
        &[],
        &["decision".to_string(), "decision".to_string()],
        &[]
    )
    .is_err());
    // No cases at all → error.
    assert!(build_model_behavior_corpus(&packet, &[], &[], &[], &[]).is_err());
    // Oversized case id (patch name > 120 chars) → case gate error.
    assert!(build_model_behavior_corpus(
        &packet,
        &[PatchCase::new(&"x".repeat(130), serde_json::json!({}))],
        &[],
        &[],
        &[],
    )
    .is_err());
    // StubActor identity.
    assert_eq!(StubActor.id(), "stub");
    assert!(StubActor.respond("arm", "req").unwrap().contains("req"));
}

// ── decision replay: load / walk / validate ────────────────────────────────

#[test]
fn decision_replay_load_errors() {
    let dir = tempfile::tempdir().unwrap();
    assert!(DecisionReplay::load(&dir.path().join("nope.json")).is_err());
    let bad = dir.path().join("bad.json");
    std::fs::write(&bad, "{nope").unwrap();
    assert!(DecisionReplay::load(&bad).is_err());
    let schema = dir.path().join("schema.json");
    std::fs::write(&schema, "{\"schema_version\":\"nope\",\"cases\":[]}").unwrap();
    assert!(DecisionReplay::load(&schema).is_err());
    let empty = dir.path().join("empty.json");
    std::fs::write(
        &empty,
        "{\"schema_version\":\"public_safe_decision_replay_v0\",\"cases\":[]}",
    )
    .unwrap();
    assert!(DecisionReplay::load(&empty).is_err());
    // A case whose case_id is an absolute path fails the public-safety walk.
    let goal = base_goal();
    let packet = packet_for(&goal);
    let mut case = reduce_public_safe_decision(&packet, &goal, "c1", None);
    case.case_id = "/abs/path".to_string();
    assert!(validate_public_safe_decision_case(&case).is_err());
    let mut replay = DecisionReplay::new();
    replay.add(case);
    let p = dir.path().join("violating.json");
    replay.save(&p).unwrap();
    assert!(
        DecisionReplay::load(&p).is_err(),
        "load validates each case"
    );
    // Bad case schema inside the file.
    let mut value = serde_json::to_value(&replay).unwrap();
    value["cases"][0]
        .as_object_mut()
        .unwrap()
        .insert("case_id".into(), "fine".into());
    value["cases"][0]
        .as_object_mut()
        .unwrap()
        .insert("schema_version".into(), "nope".into());
    let p2 = dir.path().join("badcase.json");
    std::fs::write(&p2, serde_json::to_string(&value).unwrap()).unwrap();
    assert!(DecisionReplay::load(&p2).is_err());
}

#[test]
fn walk_violations_matrix() {
    let v = serde_json::json!({
        "credential": "x",
        "nested": {"trajectory": []},
        "items": [{"path": "/abs/file"}, {"ref": "file://x"}, {"ok": "fine"}],
    });
    let violations = walk_public_safe_violations(&v);
    assert!(
        violations
            .iter()
            .any(|s| s.contains("banned key: credential")),
        "{violations:?}"
    );
    assert!(
        violations.iter().any(|s| s.contains("nested.trajectory")),
        "{violations:?}"
    );
    assert!(
        violations.iter().any(|s| s.contains("local path")),
        "{violations:?}"
    );
    assert!(walk_public_safe_violations(&serde_json::json!({"clean": "yes"})).is_empty());
}

// ── decision replay: reduce / replay matrices ──────────────────────────────

#[test]
fn reduce_covers_todo_matrix() {
    let mut goal = base_goal();
    // Every class + status + optional-field arm.
    let mut gate = Todo::user_gate("tg", "approve?", &[]);
    gate.decision = Some("yes".into());
    gate.goal_bound = true;
    gate.global_gate = true;
    goal.todos.push(gate);
    let mut action = Todo::user_action("ta", "user acts");
    action.status = TodoStatus::Blocked;
    goal.todos.push(action);
    let mut mon = Todo::monitor("tm", "watch", std::time::Duration::from_secs(60));
    mon.resume_when_text = Some("15m".into());
    goal.todos.push(mon);
    let blocker = Todo::blocker("tb", "external", &[]);
    goal.todos.push(blocker);
    let mut done = Todo::advancement("td", "done");
    done.status = TodoStatus::Done;
    goal.todos.push(done);
    let mut sup = Todo::advancement("ts", "old");
    sup.status = TodoStatus::Superseded;
    goal.todos.push(sup);
    let mut def = Todo::advancement("tdef", "later");
    def.status = TodoStatus::Deferred;
    def.resume_when = Some(std::time::SystemTime::now() + std::time::Duration::from_secs(3600));
    goal.todos.push(def);
    let mut claimed = Todo::advancement("tc", "claimed");
    claimed.claimed_by = Some("agent-1".into());
    goal.todos.push(claimed);

    // Gate-open packet → AskUser interaction arms.
    let packet = packet_for(&goal);
    let case = reduce_public_safe_decision(&packet, &goal, "rich-case", None);
    assert!(
        !case.user_todos.is_empty(),
        "gate/action populate user todos"
    );
    validate_public_safe_decision_case(&case).unwrap();
    // Replay it (kernel should reproduce the ask_user decision).
    let comparison = replay_public_safe_decision_case(&case).unwrap();
    assert!(comparison.matched, "{:?}", comparison.mismatches);
}

#[test]
fn replay_mismatch_diff_arms() {
    let mut goal = base_goal();
    goal.todos.push(Todo::advancement("t1", "work"));
    let packet = packet_for(&goal);
    let case = reduce_public_safe_decision(&packet, &goal, "c-diff", None);
    // Tamper: flip recorded should_run → replay diverges → mismatch list.
    let mut tampered = case.clone();
    tampered.decision.should_run = !case.decision.should_run;
    let comparison = replay_public_safe_decision_case(&tampered).unwrap();
    assert!(!comparison.matched);
    assert!(comparison
        .mismatches
        .iter()
        .any(|m| m.contains("should_run")));
    // Save/load roundtrip through DecisionReplay::add.
    let mut replay = DecisionReplay::default();
    replay.add(case);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sub/dir/replay.json");
    replay.save(&path).unwrap();
    let loaded = DecisionReplay::load(&path).unwrap();
    assert_eq!(loaded.cases.len(), 1);
}
