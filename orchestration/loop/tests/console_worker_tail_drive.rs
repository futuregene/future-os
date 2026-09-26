//! Drive for `worker tail` — the orchestrator's window into a live worker turn
//! log. Covers the condensed view, the `--raw` dump, and every refusal edge
//! (no goal, unknown goal, no runs, unknown agent, unknown flag, bad `--lines`).

mod common;

use common::{cli_err, cli_ok, cli_root, init_goal};

/// Seed `runs/<run_id>.live.jsonl` the way the executor writes it: a run header
/// followed by the event stream an orchestrator scans.
fn seed_live_run(root: &str, run_id: &str, agent_id: &str, goal_id: &str) {
    let dir = std::path::Path::new(root).join("runs");
    std::fs::create_dir_all(&dir).unwrap();
    let header = serde_json::json!({
        "type": "run_header",
        "wall_ts": 1_700_000_000u64,
        "run_id": run_id,
        "session_id": format!("sess-{agent_id}"),
        "agent_id": agent_id,
        "todo_id": "todo_1",
        "goal_id": goal_id,
    })
    .to_string();
    let body = [
        r#"{"type":"tool_start","tool":"shell"}"#,
        r#"{"type":"tool_end"}"#,
        r#"{"type":"usage","usage":{"total_tokens":42,"credit_cost":0.25}}"#,
        r#"{"type":"agent_end"}"#,
    ]
    .join("\n");
    std::fs::write(
        dir.join(format!("{run_id}.live.jsonl")),
        format!("{header}\n{body}\n"),
    )
    .unwrap();
}

#[test]
fn worker_tail_condensed_raw_and_refusal_edges() {
    let cr = cli_root();
    let gid = init_goal(&cr, "worker tail");

    // No goal at all, an unknown goal, and a goal with nothing running yet.
    let err = cli_err(&["worker", "tail"]);
    assert!(err.contains("--goal required"), "{err}");
    let err = cli_err(&["worker", "tail", "--goal", "ghost"]);
    assert!(err.contains("not found"), "{err}");
    let err = cli_err(&["worker", "tail", "--goal", &gid]);
    assert!(err.contains("no runs yet"), "{err}");

    // A registered-but-idle worker is not a run: still nothing to tail.
    cli_ok(&["agent", "register", "--goal", &gid, "--agent-id", "w1"]);
    let err = cli_err(&["worker", "tail", "--goal", &gid]);
    assert!(err.contains("no runs yet"), "{err}");

    seed_live_run(&cr.root, "run-tail", "w1", &gid);
    let err = cli_err(&["worker", "tail", "--goal", &gid, "--agent-id", "ghost"]);
    assert!(err.contains("no run found"), "{err}");
    let err = cli_err(&["worker", "tail", "--goal", &gid, "--bogus"]);
    assert!(err.contains("--bogus"), "{err}");

    // Default (latest run, condensed), an explicit worker with a line budget,
    // a non-numeric budget (falls back rather than failing), and the raw dump.
    cli_ok(&["worker", "tail", "--goal", &gid]);
    cli_ok(&[
        "worker",
        "tail",
        "--goal",
        &gid,
        "--agent-id",
        "w1",
        "--lines",
        "2",
    ]);
    cli_ok(&["worker", "tail", "--goal", &gid, "--lines", "not-a-number"]);
    cli_ok(&["worker", "tail", "--goal", &gid, "--raw"]);
}
