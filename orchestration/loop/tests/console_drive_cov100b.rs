//! Coverage drive for the agent / frontier / replan / profile console
//! commands added after the first coverage pass — the parse tables, JSON
//! projections, and error edges of the multi-agent and frontier surfaces.

mod common;

use common::{cli_err, cli_ok, cli_root, init_goal};

const CONTRACT: &str = r#"{"schema_version":"multi_agent_contract_v0","peers":{"primary":{"backup_for":null,"capabilities":["shell"],"workspaces":[]},"backup":{"backup_for":"primary","capabilities":[],"workspaces":[]}},"handoff_rules":[{"from_event":"lease_expired","to_role":"backup"}],"collectives":{"crew":["primary","backup"]}}"#;

#[test]
fn agent_contract_set_and_show() {
    let cr = cli_root();
    let gid = init_goal(&cr, "contract set/show");

    // Missing --goal.
    let err = cli_err(&["agent", "contract", "set", "--contract", CONTRACT]);
    assert!(err.contains("--goal required"), "{err}");
    // Missing contract (neither inline nor file).
    let err = cli_err(&["agent", "contract", "set", "--goal", &gid]);
    assert!(err.contains("contract required"), "{err}");
    // Unknown subcommand.
    let err = cli_err(&["agent", "contract", "bogus", "--goal", &gid]);
    assert!(err.contains("unknown agent contract subcommand"), "{err}");

    // Set + show (text, with peers/handoff/collective lines).
    cli_ok(&[
        "agent",
        "contract",
        "set",
        "--goal",
        &gid,
        "--contract",
        CONTRACT,
    ]);
    cli_ok(&["agent", "contract", "show", "--goal", &gid]);
    // JSON show.
    cli_ok(&["agent", "contract", "show", "--goal", &gid, "--json"]);
}

#[test]
fn agent_contract_set_from_file_and_missing_goal() {
    let cr = cli_root();
    let gid = init_goal(&cr, "contract file");
    // Read from a file.
    let path = std::env::temp_dir().join(format!("contract-{gid}.json"));
    std::fs::write(&path, CONTRACT).unwrap();
    cli_ok(&[
        "agent",
        "contract",
        "set",
        "--goal",
        &gid,
        "--contract-file",
        path.to_str().unwrap(),
    ]);
    // Missing file → read error.
    let err = cli_err(&[
        "agent",
        "contract",
        "set",
        "--goal",
        &gid,
        "--contract-file",
        "/nonexistent/contract.json",
    ]);
    assert!(err.contains("read contract file"), "{err}");
    // Unknown goal.
    let err = cli_err(&[
        "agent",
        "contract",
        "set",
        "--goal",
        "ghost",
        "--contract",
        CONTRACT,
    ]);
    assert!(err.contains("not found"), "{err}");
    // show with no contract set (fresh goal) — release the CLI_LOCK first.
    drop(cr);
    let cr2 = cli_root();
    let g2 = init_goal(&cr2, "no contract yet");
    cli_ok(&["agent", "contract", "show", "--goal", &g2]);
}

#[test]
fn agent_recipe_add_and_show() {
    let cr = cli_root();
    let gid = init_goal(&cr, "recipe add/show");
    // Missing name.
    let err = cli_err(&["agent", "recipe", "add", "--goal", &gid]);
    assert!(err.contains("--name required"), "{err}");
    // Unknown subcommand.
    let err = cli_err(&["agent", "recipe", "bogus", "--goal", &gid]);
    assert!(err.contains("unknown agent recipe subcommand"), "{err}");

    cli_ok(&[
        "agent",
        "recipe",
        "add",
        "--goal",
        &gid,
        "--name",
        "researcher",
        "--capabilities",
        "shell,github",
        "--workspace",
        "/ws/a,/ws/b",
        "--priority",
        "P0",
    ]);
    // Another recipe with P2 (upper-case parse).
    cli_ok(&[
        "agent",
        "recipe",
        "add",
        "--goal",
        &gid,
        "--name",
        "worker",
        "--priority",
        "p2",
    ]);
    // Text show + JSON show + named filter.
    cli_ok(&["agent", "recipe", "show", "--goal", &gid]);
    cli_ok(&["agent", "recipe", "show", "--goal", &gid, "--json"]);
    cli_ok(&[
        "agent", "recipe", "show", "--goal", &gid, "--name", "worker",
    ]);
    // Empty named filter → "no agent recipes".
    cli_ok(&["agent", "recipe", "show", "--goal", &gid, "--name", "ghost"]);
    // Fresh goal with no recipes — release the CLI_LOCK first.
    drop(cr);
    let cr2 = cli_root();
    let g2 = init_goal(&cr2, "no recipes");
    cli_ok(&["agent", "recipe", "show", "--goal", &g2]);
    cli_ok(&["agent", "recipe", "show", "--goal", &g2, "--json"]);
}

#[test]
fn agent_succession_show_and_apply_edges() {
    let cr = cli_root();
    let gid = init_goal(&cr, "succession edges");
    // No contract → error (checked before the subcommand dispatch).
    let err = cli_err(&["agent", "succession", "show", "--goal", &gid]);
    assert!(err.contains("no multi-agent contract set"), "{err}");

    cli_ok(&[
        "agent",
        "contract",
        "set",
        "--goal",
        &gid,
        "--contract",
        CONTRACT,
    ]);
    // Unknown subcommand (contract is set now, so dispatch is reached).
    let err = cli_err(&["agent", "succession", "bogus", "--goal", &gid]);
    assert!(err.contains("unknown agent succession subcommand"), "{err}");
    // show (text + JSON) with no candidates met.
    cli_ok(&["agent", "succession", "show", "--goal", &gid]);
    cli_ok(&["agent", "succession", "show", "--goal", &gid, "--json"]);
    // apply with no candidates met.
    cli_ok(&["agent", "succession", "apply", "--goal", &gid]);
    // apply filtered by primary that has no candidate.
    cli_ok(&[
        "agent",
        "succession",
        "apply",
        "--goal",
        &gid,
        "--primary",
        "ghost",
    ]);
}

#[test]
fn agent_collective_show_edges() {
    let cr = cli_root();
    let gid = init_goal(&cr, "collective edges");
    // Unknown subcommand.
    let err = cli_err(&["agent", "collective", "bogus", "--goal", &gid]);
    assert!(err.contains("unknown agent collective subcommand"), "{err}");
    // No contract.
    let err = cli_err(&["agent", "collective", "show", "--goal", &gid]);
    assert!(err.contains("no multi-agent contract set"), "{err}");

    cli_ok(&[
        "agent",
        "contract",
        "set",
        "--goal",
        &gid,
        "--contract",
        CONTRACT,
    ]);
    // show text + JSON.
    cli_ok(&["agent", "collective", "show", "--goal", &gid]);
    cli_ok(&["agent", "collective", "show", "--goal", &gid, "--json"]);
    // Named collective that exists + one that doesn't (bail).
    cli_ok(&[
        "agent",
        "collective",
        "show",
        "--goal",
        &gid,
        "--collective",
        "crew",
    ]);
    let err = cli_err(&[
        "agent",
        "collective",
        "show",
        "--goal",
        &gid,
        "--collective",
        "nope",
    ]);
    assert!(err.contains("not part of the contract"), "{err}");
}

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

#[test]
fn contract_show_with_validation_issues() {
    let cr = cli_root();
    let gid = init_goal(&cr, "contract issues");
    // A contract with an empty peer id is invalid → show reports issues.
    let bad = r#"{"schema_version":"multi_agent_contract_v0","peers":{"":{"backup_for":null,"capabilities":[],"workspaces":[]}},"handoff_rules":[],"collectives":{}}"#;
    // `set` fails closed on invalid contracts.
    let err = cli_err(&[
        "agent",
        "contract",
        "set",
        "--goal",
        &gid,
        "--contract",
        bad,
    ]);
    assert!(err.contains("invalid multi-agent contract"), "{err}");
}
