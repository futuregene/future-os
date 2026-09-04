//! Coverage drive for the agent / frontier / replan / profile console
//! commands added after the first coverage pass — the parse tables, JSON
//! projections, and error edges of the multi-agent and frontier surfaces.

mod common;

use common::{cli_err, cli_ok, cli_root, init_goal};

#[test]
fn replan_rules_show_and_set() {
    let cr = cli_root();
    let gid = init_goal(&cr, "replan rules");
    // Unknown subcommand.
    let err = cli_err(&["replan", "rules", "bogus", "--goal", &gid]);
    assert!(err.contains("must be `show` or `set`"), "{err}");

    // show text + JSON.
    cli_ok(&["replan", "rules", "show", "--goal", &gid]);
    cli_ok(&["replan", "rules", "show", "--goal", &gid, "--json"]);
    // set with known + unknown ids (unknown → warning).
    cli_ok(&[
        "replan",
        "rules",
        "set",
        "--goal",
        &gid,
        "--rule-ids",
        "not_monitor_only,some_unknown_rule",
    ]);
    // set with empty rule-ids resets to default.
    cli_ok(&["replan", "rules", "set", "--goal", &gid, "--rule-ids", ""]);
    // set without rule-ids resets to default too.
    cli_ok(&["replan", "rules", "set", "--goal", &gid]);
}

#[test]
fn frontier_show_text_json_and_errors() {
    let cr = cli_root();
    let gid = init_goal(&cr, "frontier show");
    // Wrong subcommand.
    let err = cli_err(&["frontier", "bogus", "--goal", &gid]);
    assert!(err.contains("must be `show`"), "{err}");
    // Missing goal.
    let err = cli_err(&["frontier", "show"]);
    assert!(err.contains("--goal required"), "{err}");
    // Unknown goal.
    let err = cli_err(&["frontier", "show", "--goal", "ghost"]);
    assert!(err.contains("not found"), "{err}");

    cli_ok(&["frontier", "show", "--goal", &gid]);
    cli_ok(&["frontier", "show", "--goal", &gid, "--json"]);
}

#[test]
fn profile_set_edges() {
    let cr = cli_root();
    let gid = init_goal(&cr, "profile set");
    // Wrong subcommand.
    let err = cli_err(&["profile", "bogus", "--goal", &gid]);
    assert!(err.contains("must be `set`"), "{err}");
    // Non-numeric outcome-floor.
    let err = cli_err(&["profile", "set", "--goal", &gid, "--outcome-floor", "abc"]);
    assert!(err.contains("must be a number"), "{err}");

    cli_ok(&["profile", "set", "--goal", &gid, "--outcome-floor", "5"]);
    cli_ok(&["profile", "set", "--goal", &gid]);
}

#[test]
fn status_json_and_webui_help() {
    let cr = cli_root();
    let gid = init_goal(&cr, "status json");
    cli_ok(&["status", "--goal", &gid, "--format", "json"]);
    // status with no goal filter (all goals) in JSON.
    cli_ok(&["status", "--format", "json"]);
    // status with an unknown goal filter errors.
    let err = cli_err(&["status", "--goal", "ghost"]);
    assert!(err.contains("not found"), "{err}");
}

fn write_live_run(root: &str, name: &str, header: &str) {
    let dir = std::path::Path::new(root).join("runs");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(name), format!("{header}\n")).unwrap();
}

#[test]
fn worker_list_and_stop_edges() {
    let cr = cli_root();
    let gid = init_goal(&cr, "worker edges");
    // Register an agent so `worker list` shows it idle.
    cli_ok(&["agent", "register", "--goal", &gid, "--agent-id", "w1"]);

    // Unknown subcommand.
    let err = cli_err(&["worker", "bogus", "--goal", &gid]);
    assert!(err.contains("unknown worker subcommand"), "{err}");
    // Missing goal.
    let err = cli_err(&["worker", "list"]);
    assert!(err.contains("--goal required"), "{err}");

    // list with a registered-but-idle agent (no live run yet).
    cli_ok(&["worker", "list", "--goal", &gid]);
    cli_ok(&["worker", "list", "--goal", &gid, "--json"]);

    // stop with neither --agent-id nor --all → error.
    let err = cli_err(&["worker", "stop", "--goal", &gid]);
    assert!(err.contains("--agent-id"), "{err}");
    // stop --all with no live sessions → nothing to stop (no agent needed).
    cli_ok(&["worker", "stop", "--goal", &gid, "--all"]);
    // stop by a non-live agent → nothing to stop.
    cli_ok(&["worker", "stop", "--goal", &gid, "--agent-id", "ghost"]);
}

#[test]
fn worker_list_scans_live_sessions() {
    let cr = cli_root();
    let gid = init_goal(&cr, "worker scan");
    cli_ok(&["agent", "register", "--goal", &gid, "--agent-id", "w1"]);
    // Seed a run header (fresh) so scan_worker_sessions returns a session;
    // the agent is unreachable here, so streaming stays false → "ended".
    let header = serde_json::json!({
        "type": "run_header",
        "wall_ts": 1_700_000_000u64,
        "run_id": "run-a",
        "session_id": "sess-a",
        "agent_id": "w1",
        "todo_id": "todo_1",
        "goal_id": gid,
    })
    .to_string();
    write_live_run(&cr.root, "run_a.live.jsonl", &header);
    cli_ok(&["worker", "list", "--goal", &gid]);
    cli_ok(&["worker", "list", "--goal", &gid, "--json"]);
}

#[test]
fn diagnose_text_and_json() {
    let cr = cli_root();
    let gid = init_goal(&cr, "diagnose");
    // Missing goal.
    let err = cli_err(&["diagnose"]);
    assert!(err.contains("--goal required"), "{err}");
    // Unknown goal.
    let err = cli_err(&["diagnose", "--goal", "ghost"]);
    assert!(err.contains("not found"), "{err}");

    cli_ok(&["diagnose", "--goal", &gid]);
    cli_ok(&["diagnose", "--goal", &gid, "--format", "json"]);
}

#[test]
fn attention_goal_all_and_json() {
    let cr = cli_root();
    let gid = init_goal(&cr, "attention");
    cli_ok(&["attention", "--goal", &gid]);
    cli_ok(&["attention", "--goal", &gid, "--json"]);
    cli_ok(&["attention", "--all"]);
    let err = cli_err(&["attention"]);
    assert!(err.contains("--goal"), "{err}");
}
