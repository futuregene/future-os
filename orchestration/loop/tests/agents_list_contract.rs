//! `agent list` contract: registered agents + live execution status.
//! Pre-flight check so parallel workers reuse existing ids instead of
//! re-registering. Deterministic — no gRPC/LLM.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_future-loop")
}

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "future-loop-agent-list-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

fn run(root: &str, args: &[&str]) -> (String, String, i32) {
    let output = Command::new(bin())
        .env("FUTURE_LOOP_ROOT", root)
        .args(args)
        .output()
        .expect("future-loop binary runs");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

fn init_goal(root: &str, id: &str) {
    let (_out, err, code) = run(
        root,
        &[
            "goal",
            "init",
            "--objective",
            "agents contract",
            "--goal-id",
            id,
            "--cwd",
            "/tmp",
        ],
    );
    assert_eq!(code, 0, "goal init failed: {err}");
}

/// Todo ids from status output (todo-<hex>=open tokens).
fn todo_ids(root: &str, goal: &str) -> Vec<String> {
    let (out, _, _) = run(root, &["status", "--goal", goal]);
    out.lines()
        .flat_map(|l| l.split_whitespace())
        .filter_map(|w| w.strip_suffix("=open").map(|s| s.to_string()))
        .filter(|s| s.starts_with("todo_"))
        .collect()
}

/// Empty goal: no agents registered.
#[test]
fn agent_list_empty_goal() {
    let root = tmp_root("empty");
    init_goal(&root, "g1");
    let (out, err, code) = run(&root, &["agent", "list", "--goal", "g1"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("no agents registered"), "{out}");
}

/// Registered agents are listed; unregistered ids are not.
#[test]
fn agent_list_shows_registered_ids_only() {
    let root = tmp_root("ids");
    init_goal(&root, "g1");
    let (_o, err, code) = run(
        &root,
        &["agent", "register", "--goal", "g1", "--agent-id", "alice"],
    );
    assert_eq!(code, 0, "{err}");
    let (_o, err, code) = run(
        &root,
        &[
            "agent",
            "recipe",
            "add",
            "--goal",
            "g1",
            "--name",
            "scientist",
            "--capabilities",
            "lammps,abacus",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let (_o, err, code) = run(
        &root,
        &[
            "agent",
            "onboard",
            "--goal",
            "g1",
            "--agent-id",
            "bob",
            "--recipe",
            "scientist",
        ],
    );
    assert_eq!(code, 0, "{err}");

    let (out, _, code) = run(&root, &["agent", "list", "--goal", "g1"]);
    assert_eq!(code, 0);
    assert!(out.contains("alice"), "{out}");
    assert!(out.contains("bob"), "{out}");
    assert!(out.contains("lammps,abacus"), "capabilities shown: {out}");
    // Both registered but holding no lease → idle, nothing running.
    assert!(out.contains("idle"), "idle status shown: {out}");
    assert!(!out.contains("running"), "no live leases yet: {out}");
    assert!(
        !out.contains("carol"),
        "unregistered id must not appear: {out}"
    );
}

/// A live lease flips the agent's status to running with the todo + lease
/// remaining; releasing it flips back to idle.
#[test]
fn agent_list_reflects_live_leases() {
    let root = tmp_root("leases");
    init_goal(&root, "g1");
    run(
        &root,
        &["agent", "onboard", "--goal", "g1", "--agent-id", "alice"],
    );
    let (_o, err, code) = run(
        &root,
        &[
            "todo",
            "add",
            "--goal",
            "g1",
            "--role",
            "agent",
            "--class",
            "advancement",
            "--text",
            "do work",
        ],
    );
    assert_eq!(code, 0, "{err}");
    let ids = todo_ids(&root, "g1");
    assert!(!ids.is_empty(), "todo added");
    let t = &ids[0];

    let (_o, err, code) = run(
        &root,
        &[
            "todo",
            "claim",
            "--goal",
            "g1",
            "--todo-id",
            t,
            "--agent-id",
            "alice",
        ],
    );
    assert_eq!(code, 0, "claim failed: {err}");

    let (out, _, code) = run(&root, &["agent", "list", "--goal", "g1"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("running"),
        "alice holds a live lease → running: {out}"
    );
    assert!(out.contains(t), "claimed todo shown: {out}");
    assert!(out.contains("lease"), "lease remaining shown: {out}");

    // Release the lease → back to idle.
    let (_o, err, code) = run(
        &root,
        &[
            "lease",
            "release",
            "--goal",
            "g1",
            "--todo-id",
            t,
            "--agent-id",
            "alice",
        ],
    );
    assert_eq!(code, 0, "release failed: {err}");
    let (out, _, _) = run(&root, &["agent", "list", "--goal", "g1"]);
    assert!(out.contains("idle"), "after release: {out}");
    assert!(
        !out.contains("running"),
        "no live lease after release: {out}"
    );
}
