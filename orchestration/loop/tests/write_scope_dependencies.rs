//! Real CLI and concurrent-process regressions; no live agent or model calls.
use future_loop::agents::workspace_guard::{
    live_workspace_conflicts, normalize_workspace_path, normalize_workspace_path_at,
};
use future_loop::state::{Goal, Todo};
use future_loop::store::{Event, Store};
use std::process::{Command, Output};
use std::sync::{Arc, Barrier};

struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(dir.path().join("observer")).unwrap();
        let mut store = Store::open(dir.path().join("state").to_str().unwrap()).unwrap();
        store
            .register(&Goal::new("g", "test", project.to_str().unwrap()))
            .unwrap();
        store
            .append(Event::GoalStarted {
                goal_id: "g".into(),
                ts: 1,
            })
            .unwrap();
        Self { dir, store }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_future-loop"));
        cmd.args(args)
            .current_dir(self.dir.path().join("observer"))
            .env("FUTURE_LOOP_ROOT", self.store.root_path())
            .env("HOME", self.dir.path().join("home"))
            .env("USERPROFILE", self.dir.path().join("home"));
        cmd
    }

    fn ok(&self, args: &[&str]) -> String {
        let output = self.command(args).output().unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn rejected(&self, args: &[&str], error: &str) {
        let before = self.store.raw_ledger_lines("g").unwrap();
        let output = self.command(args).output().unwrap();
        assert!(!output.status.success(), "{args:?} unexpectedly succeeded");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(error),
            "{output:?}"
        );
        assert_eq!(
            before,
            self.store.raw_ledger_lines("g").unwrap(),
            "rejection must not append events"
        );
    }

    fn seed(&mut self, id: &str, scopes: &[&str]) {
        let mut todo = Todo::advancement(id, "work");
        todo.required_write_scope = scopes.iter().map(|s| s.to_string()).collect();
        self.store
            .append(Event::TodoAdded {
                goal_id: "g".into(),
                todo,
                ts: 2,
            })
            .unwrap();
    }

    fn onboard(&self, agent: &str) {
        self.ok(&[
            "agent",
            "onboard",
            "--goal",
            "g",
            "--agent-id",
            agent,
            "--workspace",
            self.dir.path().join("project").to_str().unwrap(),
        ]);
    }
}

#[test]
fn cli_add_output_and_dependency_validation_are_transactional() {
    let mut f = Fixture::new();
    f.seed("t1", &[]);
    f.seed("t2", &[]);
    let out = f.ok(&[
        "todo",
        "add",
        "--goal",
        "g",
        "--text",
        "dependent",
        "--blocks",
        " t1 , t2 ",
    ]);
    let words: Vec<_> = out.split_whitespace().collect();
    assert_eq!(words[0], "todo");
    assert!(words[1].starts_with("todo_"));
    assert!(words[1][5..].bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(&words[2..], &["added", "to", "g", "✔"]);
    f.rejected(
        &[
            "todo",
            "add",
            "--goal",
            "g",
            "--text",
            "bad",
            "--blocks",
            "todo_ghost",
        ],
        "unknown todo",
    );
    f.rejected(
        &["todo", "add", "--goal", "g", "--text", "bad", "--blocks"],
        "requires a comma-separated",
    );
    f.rejected(
        &[
            "todo",
            "update",
            "--goal",
            "g",
            "--todo-id",
            "t1",
            "--blocks",
            "garbage",
        ],
        "unknown todo",
    );
    f.rejected(
        &[
            "todo",
            "update",
            "--goal",
            "g",
            "--todo-id",
            "t1",
            "--blocks",
            "t1",
        ],
        "cannot reference itself",
    );
    f.rejected(
        &[
            "todo",
            "update",
            "--goal",
            "g",
            "--todo-id",
            "t1",
            "--blocks",
            words[1],
        ],
        "cycle",
    );
    f.ok(&[
        "todo",
        "update",
        "--goal",
        "g",
        "--todo-id",
        words[1],
        "--blocks",
        "t2",
    ]);
    assert_eq!(
        f.store
            .replay("g")
            .unwrap()
            .unwrap()
            .todo(words[1])
            .unwrap()
            .blocked_by_gate
            .as_deref(),
        Some("t2")
    );
    f.ok(&[
        "todo",
        "update",
        "--goal",
        "g",
        "--todo-id",
        words[1],
        "--blocks",
        "",
    ]);
    assert!(f
        .store
        .replay("g")
        .unwrap()
        .unwrap()
        .todo(words[1])
        .unwrap()
        .blocked_by_gate
        .is_none());
    f.ok(&["task-graph", "--goal", "g"]);
}

#[test]
fn legacy_damage_can_be_repaired_incrementally_including_gate_edges() {
    let mut f = Fixture::new();
    for (id, dep) in [("a", "missing-a"), ("b", "missing-b")] {
        f.store
            .append(Event::TodoAdded {
                goal_id: "g".into(),
                todo: Todo::advancement(id, "legacy").blocking(&[dep]),
                ts: 2,
            })
            .unwrap();
    }
    f.ok(&[
        "todo",
        "update",
        "--goal",
        "g",
        "--todo-id",
        "a",
        "--blocks",
        "",
    ]);
    f.rejected(&["task-graph", "--goal", "g"], "unknown todo");
    f.ok(&[
        "todo",
        "update",
        "--goal",
        "g",
        "--todo-id",
        "b",
        "--blocks",
        "a",
    ]);
    let gate = f.ok(&[
        "todo", "add", "--goal", "g", "--class", "blocker", "--text", "gate", "--blocks", "a",
    ]);
    let gate_id = gate.split_whitespace().nth(1).unwrap();
    // gate -> a -> b; declaring gate -> b duplicates an edge, not a cycle.
    f.ok(&[
        "todo",
        "update",
        "--goal",
        "g",
        "--todo-id",
        gate_id,
        "--blocks",
        "a,b",
    ]);
    f.ok(&["task-graph", "--goal", "g"]);
    // A blocking-source cycle is rejected in the correct direction.
    let gate2 = f.ok(&[
        "todo", "add", "--goal", "g", "--class", "blocker", "--text", "gate2", "--blocks", gate_id,
    ]);
    let gate2_id = gate2.split_whitespace().nth(1).unwrap();
    f.rejected(
        &[
            "todo",
            "update",
            "--goal",
            "g",
            "--todo-id",
            gate_id,
            "--blocks",
            gate2_id,
        ],
        "cycle",
    );
}

fn race(commands: Vec<Command>) -> Vec<Output> {
    let barrier = Arc::new(Barrier::new(commands.len()));
    let handles: Vec<_> = commands
        .into_iter()
        .map(|mut cmd| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                cmd.output().unwrap()
            })
        })
        .collect();
    handles.into_iter().map(|h| h.join().unwrap()).collect()
}

#[test]
fn concurrent_dependency_updates_cannot_jointly_introduce_a_cycle() {
    let mut f = Fixture::new();
    f.seed("a", &[]);
    f.seed("b", &[]);
    let results = race(vec![
        f.command(&[
            "todo",
            "update",
            "--goal",
            "g",
            "--todo-id",
            "a",
            "--blocks",
            "b",
        ]),
        f.command(&[
            "todo",
            "update",
            "--goal",
            "g",
            "--todo-id",
            "b",
            "--blocks",
            "a",
        ]),
    ]);
    assert_eq!(
        results.iter().filter(|o| o.status.success()).count(),
        1,
        "{results:?}"
    );
    assert!(
        String::from_utf8_lossy(&results.iter().find(|o| !o.status.success()).unwrap().stderr)
            .contains("cycle")
    );
    f.ok(&["task-graph", "--goal", "g"]);
}

#[test]
fn four_workers_with_relative_disjoint_scopes_claim_without_force() {
    let mut f = Fixture::new();
    let mut commands = vec![];
    for n in 0..4 {
        let id = format!("t{n}");
        let agent = format!("w{n}");
        f.seed(&id, &[&format!("papers/{n}.md")]);
        f.onboard(&agent);
        commands.push(f.command(&[
            "todo",
            "claim",
            "--goal",
            "g",
            "--todo-id",
            &id,
            "--agent-id",
            &agent,
        ]));
    }
    let results = race(commands);
    assert!(results.iter().all(|o| o.status.success()), "{results:?}");
    let goal = f.store.replay("g").unwrap().unwrap();
    for n in 0..4 {
        assert!(
            live_workspace_conflicts(&goal, &format!("w{n}"), future_loop::state::now_epoch())
                .is_empty()
        );
    }
    let audits: Vec<_> = f
        .store
        .events("g")
        .unwrap()
        .into_iter()
        .filter_map(|e| match e.event {
            Event::WorkspaceLockAcquired { paths, forced, .. } => Some((paths, forced)),
            _ => None,
        })
        .collect();
    assert_eq!(audits.len(), 4);
    for (paths, forced) in audits {
        assert!(!forced);
        assert_eq!(paths.len(), 1);
        // The audited path is the guard's normalized form of the goal-relative
        // scope (canonicalized parent + relative tail, platform-spelled).
        assert!(
            std::path::Path::new(&paths[0]).starts_with(normalize_workspace_path(
                &f.dir.path().join("project").to_string_lossy()
            ))
        );
    }
}

#[test]
fn concurrent_overlapping_claims_have_only_one_winner_and_force_is_audited() {
    let mut f = Fixture::new();
    let mut commands = vec![];
    for n in 0..4 {
        let id = format!("t{n}");
        let agent = format!("w{n}");
        f.seed(&id, &["papers/"]);
        f.onboard(&agent);
        // Cover both manual claim surfaces with the same atomic guard.
        commands.push(f.command(&[
            if n % 2 == 0 { "todo" } else { "lease" },
            "claim",
            "--goal",
            "g",
            "--todo-id",
            &id,
            "--agent-id",
            &agent,
        ]));
    }
    let results = race(commands);
    assert_eq!(
        results.iter().filter(|o| o.status.success()).count(),
        1,
        "{results:?}"
    );
    for output in results.iter().filter(|o| !o.status.success()) {
        assert!(String::from_utf8_lossy(&output.stderr).contains("workspace conflict"));
    }
    let loser = results.iter().position(|o| !o.status.success()).unwrap();
    f.ok(&[
        "lease",
        "claim",
        "--goal",
        "g",
        "--todo-id",
        &format!("t{loser}"),
        "--agent-id",
        &format!("w{loser}"),
        "--force",
    ]);
    assert!(f
        .store
        .events("g")
        .unwrap()
        .iter()
        .any(|e| matches!(e.event, Event::WorkspaceLockAcquired { forced: true, .. })));
}

#[test]
fn missing_scopes_fall_back_and_absolute_aliases_cannot_evade_conflicts() {
    let mut f = Fixture::new();
    f.seed("relative", &["papers/a.md"]);
    let absolute = f.dir.path().join("project").join("papers/a.md");
    f.seed("absolute", &[absolute.to_str().unwrap()]);
    f.seed("fallback", &[]);
    for agent in ["a", "b", "c"] {
        f.onboard(agent);
    }
    f.ok(&[
        "todo",
        "claim",
        "--goal",
        "g",
        "--todo-id",
        "relative",
        "--agent-id",
        "a",
    ]);
    f.rejected(
        &[
            "todo",
            "claim",
            "--goal",
            "g",
            "--todo-id",
            "absolute",
            "--agent-id",
            "b",
        ],
        "workspace conflict",
    );
    f.rejected(
        &[
            "lease",
            "claim",
            "--goal",
            "g",
            "--todo-id",
            "fallback",
            "--agent-id",
            "c",
        ],
        "workspace conflict",
    );
    assert_eq!(
        normalize_workspace_path_at("papers/a.md", &f.dir.path().join("project")),
        normalize_workspace_path_at(absolute.to_str().unwrap(), f.dir.path())
    );
}

#[test]
fn legacy_relative_goal_anchor_is_rejected_instead_of_using_observer_cwd() {
    let mut f = Fixture::new();
    f.seed("t", &["papers/a.md"]);
    f.onboard("w");
    let registry = std::path::Path::new(&f.store.root_path()).join("registry.json");
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&registry).unwrap()).unwrap();
    json[0]["cwd"] = serde_json::json!(".");
    std::fs::write(&registry, serde_json::to_vec(&json).unwrap()).unwrap();
    f.rejected(
        &[
            "todo",
            "claim",
            "--goal",
            "g",
            "--todo-id",
            "t",
            "--agent-id",
            "w",
        ],
        "legacy goal cwd is not absolute",
    );
}

#[test]
fn force_does_not_bypass_ownership_or_another_agents_lease() {
    let mut f = Fixture::new();
    let todo = Todo::advancement("owned", "work").owned_by("a");
    f.store
        .append(Event::TodoAdded {
            goal_id: "g".into(),
            todo,
            ts: 2,
        })
        .unwrap();
    f.seed("leased", &["papers/a.md"]);
    f.onboard("a");
    f.onboard("b");
    f.rejected(
        &[
            "todo",
            "claim",
            "--goal",
            "g",
            "--todo-id",
            "owned",
            "--agent-id",
            "b",
            "--force",
        ],
        "cannot be claimed",
    );
    f.ok(&[
        "todo",
        "claim",
        "--goal",
        "g",
        "--todo-id",
        "leased",
        "--agent-id",
        "a",
    ]);
    f.rejected(
        &[
            "lease",
            "claim",
            "--goal",
            "g",
            "--todo-id",
            "leased",
            "--agent-id",
            "b",
            "--force",
        ],
        "active lease",
    );
}

#[cfg(unix)]
#[test]
fn nonexistent_files_below_symlinked_parents_have_the_same_scope() {
    let f = Fixture::new();
    std::os::unix::fs::symlink(f.dir.path().join("project"), f.dir.path().join("alias")).unwrap();
    assert_eq!(
        normalize_workspace_path_at("alias/papers/new.md", f.dir.path()),
        normalize_workspace_path_at("project/papers/new.md", f.dir.path())
    );
    assert_eq!(
        normalize_workspace_path_at("alias/missing/../papers/new.md", f.dir.path()),
        normalize_workspace_path_at("project/papers/new.md", f.dir.path())
    );
}
