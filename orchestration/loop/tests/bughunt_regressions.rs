use future_loop::decision::{decide, goal_frontier::terminal::terminal_judgement};
use future_loop::state::{Goal, Todo, TodoStatus};
use future_loop::store::{Event, Store};
use std::time::{Duration, SystemTime};

#[test]
fn deferred_and_unvalidated_work_never_closes() {
    let mut goal = Goal::new("g", "objective", ".");
    goal.add(Todo::deferred("t", "later", Duration::ZERO));
    assert!(!goal.is_terminal());
    assert!(!terminal_judgement(&goal).terminal);
    let todo = goal.todo_mut("t").unwrap();
    todo.status = TodoStatus::Open;
    todo.validator = Some("false".into());
    todo.complete(true, vec![]);
    assert!(!goal.is_terminal());
    assert!(!terminal_judgement(&goal).terminal);
    assert_ne!(
        decide(&goal, SystemTime::now()).interaction_contract.mode,
        future_loop::contract::TurnMode::Terminal
    );
}

#[test]
fn atomic_claim_honors_renewal_owner_completion_and_manual_pid() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path().to_str().unwrap()).unwrap();
    let goal = Goal::new("g", "objective", ".");
    store.register(&goal).unwrap();
    let now = future_loop::state::now_epoch();
    store
        .append(Event::GoalStarted {
            goal_id: "g".into(),
            ts: now,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: "g".into(),
            todo: Todo::advancement("t", "task"),
            ts: now,
        })
        .unwrap();
    store
        .append(Event::TodoClaimed {
            goal_id: "g".into(),
            todo_id: "t".into(),
            agent_id: "a".into(),
            lease_expires_at: now - 1,
            holder_pid: Some(std::process::id()),
            ts: now - 2,
        })
        .unwrap();
    store
        .append(Event::TodoRenewed {
            goal_id: "g".into(),
            todo_id: "t".into(),
            agent_id: "a".into(),
            lease_expires_at: now + 3600,
            ts: now,
        })
        .unwrap();
    assert!(!store.try_claim_todo("g", "t", "b", 3600).unwrap().claimed);
    store
        .append(Event::TodoCompleted {
            goal_id: "g".into(),
            todo_id: "t".into(),
            no_follow_up: true,
            successor_ids: vec![],
            evidence: Some("done".into()),
            ts: now,
        })
        .unwrap();
    let replay = store.replay("g").unwrap().unwrap();
    let todo = replay.todo("t").unwrap();
    assert!(todo.claimed_by.is_none());
    assert!(todo.lease_expires_at.is_none());
    assert!(todo.holder_pid.is_none());
    assert!(!store.try_claim_todo("g", "t", "b", 3600).unwrap().claimed);
    let mut owned = Todo::advancement("owned", "assigned");
    owned.owner = Some("a".into());
    store
        .append(Event::TodoAdded {
            goal_id: "g".into(),
            todo: owned,
            ts: now,
        })
        .unwrap();
    assert!(
        !store
            .try_claim_todo("g", "owned", "b", 3600)
            .unwrap()
            .claimed
    );
    assert!(
        store
            .try_claim_todo("g", "owned", "a", 3600)
            .unwrap()
            .claimed
    );
    assert!(store
        .replay("g")
        .unwrap()
        .unwrap()
        .todo("owned")
        .unwrap()
        .holder_pid
        .is_none());
}

#[test]
fn injected_clock_controls_frontier_and_deferred_monitor() {
    let now = SystemTime::now();
    let mut goal = Goal::new("g", "objective", ".");
    let mut todo = Todo::deferred("t", "later", Duration::from_secs(3600));
    todo.resume_when = Some(now + Duration::from_secs(60));
    goal.add(todo);
    let later = now + Duration::from_secs(120);
    let packet = decide(&goal, later);
    assert_eq!(packet.decision, "run");
    assert_eq!(packet.frontier_projection.unclaimed_advancement, 1);
    goal.todo_mut("t").unwrap().class = future_loop::state::TaskClass::Monitor;
    let packet = decide(&goal, later);
    assert_eq!(packet.reason_code, "monitor_due");
    assert_eq!(packet.frontier_projection.monitors_due, 1);
    goal.todo_mut("t").unwrap().consecutive_no_change = 3;
    assert_eq!(decide(&goal, later).reason_code, "monitor_stalled");
}

#[test]
fn every_due_deferred_class_reenters_its_open_lane_with_one_clock() {
    use future_loop::state::TaskClass;
    let due = SystemTime::UNIX_EPOCH + Duration::from_secs(200);
    let early = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
    let late = SystemTime::UNIX_EPOCH + Duration::from_secs(300);
    for class in [
        TaskClass::Advancement,
        TaskClass::Monitor,
        TaskClass::UserGate,
        TaskClass::Blocker,
        TaskClass::Coordination,
        TaskClass::UserAction,
    ] {
        let mut goal = Goal::new("g", "test", ".");
        let mut todo = Todo::advancement("t", "task");
        todo.class = class;
        todo.resume_when = Some(due);
        goal.add(todo);
        let open = decide(&goal, late);
        goal.todos[0].status = TodoStatus::Deferred;
        let deferred = decide(&goal, late);
        assert_eq!(open.reason_code, deferred.reason_code, "class {class:?}");
        assert_eq!(
            open.interaction_contract.mode,
            deferred.interaction_contract.mode
        );
        let pending = decide(&goal, early);
        assert_eq!(
            pending.reason_code, "deferred_not_due",
            "wall clock must not override injected clock for {class:?}"
        );
        assert_eq!(pending.frontier_projection.monitors_open, 0);
        assert_eq!(pending.agent_todo_summary.unwrap().agent_open, 0);
        assert!(!goal.is_terminal());
    }
    let mut goal = Goal::new("g", "gated", ".");
    goal.add(Todo::advancement("work", "work"));
    let mut gate = Todo::user_gate("gate", "approve?", &["work"]);
    gate.status = TodoStatus::Deferred;
    gate.resume_when = Some(due);
    goal.add(gate);
    assert_eq!(decide(&goal, early).reason_code, "runnable_todo");
    let packet = decide(&goal, late);
    assert_eq!(packet.reason_code, "open_user_gate");
    assert!(packet
        .interaction_contract
        .agent_channel
        .fallback_todo
        .is_none());
    assert_eq!(packet.frontier_projection.unclaimed_advancement, 0);
}

#[test]
fn scheduler_overflow_and_distinct_failure_cache_are_bounded() {
    use future_loop::scheduler::state::*;
    assert_eq!(monitor_cadence_secs("18446744073709551615d"), None);
    assert_eq!(monitor_cadence_secs("2h"), Some(7200));
    let now = future_loop::state::now_epoch();
    let mut failures = vec![];
    for n in 1..=9 {
        failures = merge_host_update_failure(
            &failures,
            HostUpdateFailure {
                schema_version: SCHEDULER_HOST_UPDATE_FAILURE_SCHEMA_VERSION.into(),
                target_rrule: format!("FREQ=MINUTELY;INTERVAL={n}"),
                observed_host_rrule: "FREQ=MINUTELY;INTERVAL=1440".into(),
                failure_kind: "host_stale_rrule".into(),
                failed_at: future_loop::compat::rfc3339(now),
                failure_count: 1,
            },
            now,
        );
    }
    assert_eq!(failures.len(), SCHEDULER_HOST_UPDATE_FAILURE_CACHE_LIMIT);
    assert!(failures.last().unwrap().target_rrule.ends_with("=9"));
}

#[test]
fn privacy_redacts_entire_path_without_mangling_normal_words() {
    use future_loop::projection::privacy::{classify_text, redact, PrivacyLevel};
    for text in ["task-1", "disk-usage", "risk-based", "peak-load"] {
        assert_eq!(classify_text(text), PrivacyLevel::PublicSafe);
        assert_eq!(redact(text, PrivacyLevel::PublicSafe), text);
    }
    for text in [
        "edit /Users/alice/secret.txt",
        "edit /home/alice/secret.txt",
        r"read C:\Users\alice\secret.txt",
        "sk-secret-token",
    ] {
        assert_eq!(classify_text(text), PrivacyLevel::LocalPrivate);
        let output = redact(text, PrivacyLevel::PublicSafe);
        assert!(!output.contains("alice"));
        assert!(!output.contains("secret"));
    }
}

#[test]
fn goal_ids_cannot_escape_the_store_or_run_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_str().unwrap();
    let mut store = Store::open(root).unwrap();
    for id in [
        "../../escape",
        "..\\escape",
        "/absolute",
        "C:\\escape",
        "..",
        "",
    ] {
        assert!(store.register(&Goal::new(id, "test", ".")).is_err());
        assert!(store.goal_dir(id).starts_with(dir.path().join("goals")));
        let runs = future_loop::runtime::runs_dir(root, id);
        assert!(!runs
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir)));
    }
}

#[test]
fn rapid_run_mirrors_do_not_overwrite_each_other() {
    let dir = tempfile::tempdir().unwrap();
    for turn in 1..=16 {
        let record: future_loop::state::RunRecord = serde_json::from_value(serde_json::json!({
            "turn":turn, "todo_id":"t", "run_id":format!("run_{turn}"), "agent_id":"worker-a",
            "terminal_state":"completed", "error":null, "tokens_in_delta":1,"tokens_out_delta":2,
            "cost_delta":0.0,"tools":[],"evidence":"done","recorded_at":100,
        }))
        .unwrap();
        future_loop::compat::write_run(dir.path(), "g", &record).unwrap();
    }
    let files: Vec<_> = std::fs::read_dir(dir.path().join("runs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(
        files
            .iter()
            .filter(|p| p.extension().is_some_and(|s| s == "json"))
            .count(),
        16
    );
    assert_eq!(
        files
            .iter()
            .filter(|p| p.extension().is_some_and(|s| s == "md"))
            .count(),
        16
    );
    for path in files
        .iter()
        .filter(|p| p.extension().is_some_and(|s| s == "json"))
    {
        let payload: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(payload["agent_id"], "worker-a");
    }
}

#[test]
fn help_root_matches_project_root_and_override_with_newline() {
    let dir = tempfile::tempdir().unwrap();
    let bin = env!("CARGO_BIN_EXE_future-loop");
    let output = std::process::Command::new(bin)
        .arg("--help")
        .current_dir(dir.path())
        .env_remove("FUTURE_LOOP_ROOT")
        .env("HOME", dir.path().join("different-home"))
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    // The state root is `std::env::current_dir()/.future/loop`: the cwd as the
    // OS reports it. `canonicalize` resolves symlinked parents (macOS /var),
    // but on Windows it adds an extended-length prefix the cwd never carries.
    let canonical = dir.path().canonicalize().unwrap();
    let canonical = canonical.to_string_lossy();
    let expected = std::path::Path::new(canonical.strip_prefix(r"\\?\").unwrap_or(&canonical))
        .join(".future")
        .join("loop");
    assert!(
        text.ends_with(&format!(
            "State root: {} (env FUTURE_LOOP_ROOT)\n",
            expected.display()
        )),
        "{text}"
    );
    let output = std::process::Command::new(bin)
        .arg("--help")
        .env("FUTURE_LOOP_ROOT", "custom-state-root")
        .output()
        .unwrap();
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .ends_with("State root: custom-state-root (env FUTURE_LOOP_ROOT)\n"));
}

#[test]
fn unicode_truncation_and_metadata_roundtrip() {
    assert_eq!(
        future_loop::decision::truncate("日本語テキスト", 10),
        "日本語テキスト"
    );
    let mut goal = Goal::new("g", "objective", ".");
    let mut todo = Todo::advancement("t", "task");
    todo.note = Some("first\nstatus=done --> %20".into());
    goal.add(todo);
    let markdown = future_loop::compat::render_active_state(&goal);
    let parsed = future_loop::backfill::parse_markdown_todos(&markdown);
    assert_eq!(
        parsed
            .iter()
            .find(|t| t.todo_id.as_deref() == Some("t"))
            .unwrap()
            .note
            .as_deref(),
        Some("first\nstatus=done --> %20")
    );
}
