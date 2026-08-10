//! Coverage drive — console command group C (P3/P4): capability / catalog /
//! scope / lane / supervisor / handoff / task-graph / attention / inbox /
//! extension / benchmark / replay, plus the per-capability command hooks.

mod common;

use common::mock_agent::{completed_events, spawn_mock, MockState};
use common::{cli_err, cli_ok, cli_root, first_todo_id, init_goal, open_store};
use future_loop::state::now_epoch;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

// ── capability ─────────────────────────────────────────────────────────────

#[test]
fn capability_surface() {
    let _cr = cli_root();
    cli_ok(&["capability", "list"]);
    cli_ok(&[
        "capability",
        "propose",
        "--name",
        "issue_fix",
        "--input",
        "crash on startup",
    ]);
    cli_ok(&["capability", "propose", "--name", "issue_fix"]);
    assert!(
        cli_err(&["capability", "propose", "--name", "no_such_cap"]).contains("unknown capability")
    );
    assert!(cli_err(&["capability", "propose"]).contains("--name required"));
    assert!(cli_err(&["capability", "bogus"]).contains("list"));
    // commands: full listing, per-name, and an experimental gated name.
    cli_ok(&["capability", "commands"]);
    cli_ok(&["capability", "commands", "--include-experimental"]);
    cli_ok(&["capability", "commands", "--name", "issue_fix"]);
    cli_ok(&["capability", "commands", "--name", "no_such_cap"]);
}

#[test]
fn capability_command_hook() {
    let _cr = cli_root();
    // `issue-fix` is a catalog command hook → dispatched via the `other` arm.
    cli_ok(&["issue-fix", "--input", "panic in parser"]);
    cli_ok(&["issue-fix"]);
}

// ── catalog ────────────────────────────────────────────────────────────────

#[test]
fn catalog_surface() {
    let _cr = cli_root();
    cli_ok(&["catalog"]);
    cli_ok(&["catalog", "--name", "issue_fix"]);
    cli_ok(&["catalog", "--name", "issue_fix", "--format", "json"]);
    cli_ok(&["catalog", "--name", "issue_fix", "--json"]);
    assert!(cli_err(&["catalog", "--name", "no_such_cap"]).contains("unknown capability"));
}

// ── scope / lane ───────────────────────────────────────────────────────────

#[test]
fn scope_and_lane_surface() {
    let cr = cli_root();
    let gid = init_goal(&cr, "scope goal");
    cli_ok(&["agent", "register", "--goal", &gid, "--agent-id", "w1"]);
    let first = first_todo_id(&cr.root, &gid);
    cli_ok(&[
        "todo",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--agent-id",
        "w1",
    ]);
    cli_ok(&["scope", "--goal", &gid, "--agent-id", "w1"]);
    cli_ok(&[
        "scope",
        "--goal",
        &gid,
        "--agent-id",
        "w1",
        "--exclude",
        "todo_x,todo_y",
    ]);
    // lane: no runs yet → None path. Then seed a run → recommendation paths.
    cli_ok(&["lane", "--goal", &gid, "--agent-id", "w1"]);
    {
        let store = open_store(&cr);
        store
            .append_run(&gid, &common::run_record(&first, "completed", now_epoch()))
            .unwrap();
    }
    cli_ok(&["lane", "--goal", &gid, "--agent-id", "w1"]);
    // errors.
    assert!(cli_err(&["scope", "--goal", &gid]).contains("--agent-id required"));
    assert!(cli_err(&["scope", "--agent-id", "w1"]).contains("--goal required"));
    assert!(cli_err(&["scope", "--goal", "goal_nope", "--agent-id", "w1"]).contains("not found"));
    assert!(cli_err(&["lane", "--goal", &gid]).contains("--agent-id required"));
    assert!(cli_err(&["lane", "--agent-id", "w1"]).contains("--goal required"));
    assert!(cli_err(&["lane", "--goal", "goal_nope", "--agent-id", "w1"]).contains("not found"));
}

// ── supervisor ─────────────────────────────────────────────────────────────

#[test]
fn supervisor_surface() {
    let cr = cli_root();
    let gid = init_goal(&cr, "supervisor goal");
    cli_ok(&[
        "supervisor",
        "propose",
        "--goal",
        &gid,
        "--agent-id",
        "sup",
        "--decision-id",
        "d1",
        "--target-agent-id",
        "w1",
        "--kind",
        "observe",
        "--summary",
        "watching",
    ]);
    cli_ok(&[
        "supervisor",
        "propose",
        "--goal",
        &gid,
        "--agent-id",
        "sup",
        "--decision-id",
        "d2",
        "--target-agent-id",
        "w1",
        "--kind",
        "execute",
        "--capabilities",
        "shell,web",
    ]);
    for (decision, outcome, caps) in [
        ("d2", "executed", "shell,web"),
        ("d2", "failed", "shell"),
        ("d2", "bogus-outcome", "shell"), // → Rejected via the catch-all arm
    ] {
        cli_ok(&[
            "supervisor",
            "receipt",
            "--goal",
            &gid,
            "--decision-id",
            decision,
            "--receipt-id",
            &format!("r-{outcome}"),
            "--adapter-id",
            "adapter-1",
            "--outcome",
            outcome,
            "--authority-ref",
            "auth-1",
            "--host-capabilities",
            caps,
        ]);
    }
    // An observe decision rejects host-execution receipts, and an executed
    // receipt must carry the decision's required capabilities.
    assert!(cli_err(&[
        "supervisor",
        "receipt",
        "--goal",
        &gid,
        "--decision-id",
        "d1",
        "--receipt-id",
        "r-bad",
        "--adapter-id",
        "a",
        "--outcome",
        "executed",
    ])
    .contains("observe"));
    assert!(cli_err(&[
        "supervisor",
        "receipt",
        "--goal",
        &gid,
        "--decision-id",
        "d2",
        "--receipt-id",
        "r-bad2",
        "--adapter-id",
        "a",
        "--outcome",
        "executed",
        "--host-capabilities",
        "shell",
    ])
    .contains("capabilities"));
    cli_ok(&["supervisor", "events", "--goal", &gid]);
    // errors.
    assert!(cli_err(&["supervisor", "propose", "--goal", &gid]).contains("--agent-id"));
    assert!(
        cli_err(&["supervisor", "propose", "--goal", &gid, "--agent-id", "sup",])
            .contains("--decision-id")
    );
    assert!(cli_err(&[
        "supervisor",
        "propose",
        "--goal",
        &gid,
        "--agent-id",
        "sup",
        "--decision-id",
        "d",
    ])
    .contains("--target-agent-id"));
    assert!(cli_err(&["supervisor", "receipt", "--goal", &gid]).contains("--decision-id"));
    assert!(cli_err(&[
        "supervisor",
        "receipt",
        "--goal",
        &gid,
        "--decision-id",
        "d",
    ])
    .contains("--receipt-id"));
    assert!(cli_err(&[
        "supervisor",
        "receipt",
        "--goal",
        &gid,
        "--decision-id",
        "d",
        "--receipt-id",
        "r",
    ])
    .contains("--adapter-id"));
    assert!(cli_err(&["supervisor", "events"]).contains("--goal required"));
    assert!(cli_err(&["supervisor", "bogus", "--goal", &gid]).contains("propose|receipt|events"));
}

// ── handoff / task-graph / attention ───────────────────────────────────────

#[test]
fn handoff_surface() {
    let cr = cli_root();
    let gid = init_goal(&cr, "handoff goal");
    cli_ok(&["handoff", "--goal", &gid]);
    cli_ok(&["handoff", "--goal", &gid, "--write"]);
    let handoff = open_store(&cr).goal_dir(&gid).join("HANDOFF.md");
    assert!(handoff.exists(), "handoff written");
    // With a degraded run in history → delivery contract present.
    {
        let store = open_store(&cr);
        let first = first_todo_id(&cr.root, &gid);
        store
            .append_run(&gid, &common::run_record(&first, "error", now_epoch()))
            .unwrap();
    }
    cli_ok(&["handoff", "--goal", &gid]);
    assert!(cli_err(&["handoff"]).contains("--goal required"));
    assert!(cli_err(&["handoff", "--goal", "goal_nope"]).contains("not found"));
}

#[test]
fn task_graph_surface() {
    let cr = cli_root();
    let gid = init_goal(&cr, "graph goal");
    // --blocks must reference real todos (the graph fails closed on unknown
    // refs), so chain off the onboarding todo.
    let first = first_todo_id(&cr.root, &gid);
    cli_ok(&[
        "todo", "add", "--goal", &gid, "--text", "second", "--blocks", &first,
    ]);
    cli_ok(&["task-graph", "--goal", &gid]);
    // A cycle does NOT fail — it is reported (⚠ cycle) with no topo order.
    let gid2 = init_goal(&cr, "cycle goal");
    let x = first_todo_id(&cr.root, &gid2);
    cli_ok(&[
        "todo", "add", "--goal", &gid2, "--text", "y", "--blocks", &x,
    ]);
    let y = common::todo_id_by_text(&cr.root, &gid2, "y");
    cli_ok(&[
        "todo",
        "update",
        "--goal",
        &gid2,
        "--todo-id",
        &x,
        "--blocks",
        &y,
    ]);
    cli_ok(&["task-graph", "--goal", &gid2]);
    // Unknown refs fail closed.
    let gid3 = init_goal(&cr, "unknown ref goal");
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid3,
        "--text",
        "dangling",
        "--blocks",
        "todo_ghost",
    ]);
    assert!(cli_err(&["task-graph", "--goal", &gid3]).contains("unknown todo"));
    assert!(cli_err(&["task-graph"]).contains("--goal required"));
    assert!(cli_err(&["task-graph", "--goal", "goal_nope"]).contains("not found"));
}

#[test]
fn attention_surface() {
    let cr = cli_root();
    let gid = init_goal(&cr, "attention goal");
    cli_ok(&["attention", "--goal", &gid]);
    cli_ok(&["attention", "--all"]);
    // A goal with an open gate ranks differently in the queue.
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--text",
        "needs approval",
        "--class",
        "user_gate",
    ]);
    cli_ok(&["attention", "--goal", &gid]);
    cli_ok(&["attention", "--all"]);
    assert!(cli_err(&["attention"]).contains("--all"));
    // Unknown goal id is skipped (no error).
    cli_ok(&["attention", "--goal", "goal_nope"]);
}

// ── inbox ──────────────────────────────────────────────────────────────────

#[test]
fn inbox_surface() {
    let cr = cli_root();
    // Empty project → zero pending.
    cli_ok(&["inbox", "--project", &cr.cwd]);
    // Craft inbox events under <project>/.future/loop/inbox/.
    let inbox = std::path::Path::new(&cr.cwd).join(".future/loop/inbox");
    std::fs::create_dir_all(&inbox).unwrap();
    std::fs::write(
        inbox.join("m1.json"),
        r#"{"message_id":"m1","create_time":"2026-08-10T00:00:00Z","content":"@operator can you review?","reply_context_verified":false,"reply_to_operator":false}"#,
    )
    .unwrap();
    std::fs::write(
        inbox.join("m2.json"),
        r#"{"message_id":"m2","create_time":"2026-08-10T00:01:00Z","content":"reply here","reply_context_verified":true,"reply_to_operator":true}"#,
    )
    .unwrap();
    // A malformed event file is skipped, a non-json file ignored.
    std::fs::write(inbox.join("bad.json"), "{nope").unwrap();
    std::fs::write(inbox.join("note.txt"), "hello").unwrap();
    cli_ok(&["inbox", "--project", &cr.cwd]);
    cli_ok(&[
        "inbox",
        "--project",
        &cr.cwd,
        "--scope",
        "all",
        "--name",
        "operator",
    ]);
    cli_ok(&["inbox", "--project", &cr.cwd, "--scope", "mentions"]);
}

// ── extension ──────────────────────────────────────────────────────────────

fn write_manifest(cr: &common::CliRoot, id: &str, version: &str) -> String {
    let raw = serde_json::json!({
        "schema_version": future_loop::extensions::manifest::EXTENSION_MANIFEST_SCHEMA_VERSION,
        "id": id,
        "version": version,
        "requires_future_loop_api": ">=1,<3",
        "permissions": ["shell"],
        "runtime": {
            "protocol": "command_json_v0",
            "entrypoint": "sh",
            "args": [],
            "doctor_args": ["-c", "true"],
            "required_permissions": ["shell"],
            "timeout_seconds": 30
        },
        "provides": [{"id": format!("{id}_cap"), "kind": "domain_rule", "visibility": "public"}],
        "implements": [{"capability_id": format!("{id}_cap"), "protocol": "command_json_v0"}]
    });
    let path = std::path::Path::new(&cr.cwd).join(format!("manifest-{id}-{version}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(&raw).unwrap()).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn extension_surface() {
    let cr = cli_root();
    // status/capabilities with no extensions installed.
    cli_ok(&["extension", "status"]);
    cli_ok(&["extension", "capabilities"]);
    let m1 = write_manifest(&cr, "ext-a", "1.0.0");
    // dry-run install, then executed install, then lifecycle.
    cli_ok(&["extension", "install", "--manifest", &m1]);
    cli_ok(&["extension", "install", "--manifest", &m1, "--execute"]);
    cli_ok(&["extension", "status"]);
    cli_ok(&["extension", "status", "--id", "ext-a"]);
    cli_ok(&["extension", "capabilities"]);
    let m2 = write_manifest(&cr, "ext-a", "1.1.0");
    cli_ok(&["extension", "upgrade", "--manifest", &m2, "--execute"]);
    cli_ok(&["extension", "disable", "--id", "ext-a", "--execute"]);
    cli_ok(&["extension", "enable", "--id", "ext-a", "--execute"]);
    cli_ok(&["extension", "rollback", "--id", "ext-a", "--execute"]);
    // errors.
    assert!(cli_err(&["extension", "install"]).contains("--manifest required"));
    assert!(cli_err(&["extension", "install", "--manifest", "/nonexistent.json"]).contains(""));
    let bad = std::path::Path::new(&cr.cwd).join("bad-manifest.json");
    std::fs::write(&bad, "{\"schema_version\":\"nope\"}").unwrap();
    assert!(cli_err(&["extension", "install", "--manifest", bad.to_str().unwrap()]).contains(""));
    assert!(cli_err(&["extension", "enable"]).contains("--id required"));
    assert!(cli_err(&["extension", "bogus"]).contains("install|upgrade"));
}

// ── benchmark ──────────────────────────────────────────────────────────────

#[test]
fn benchmark_protocol_and_ledger() {
    let cr = cli_root();
    cli_ok(&[
        "benchmark",
        "protocol",
        "--route",
        "future-loop-product-mode",
    ]);
    cli_ok(&[
        "benchmark",
        "protocol",
        "--route",
        "future-loop-product-mode",
        "--json",
    ]);
    cli_ok(&[
        "benchmark",
        "protocol",
        "--route",
        "custom-route",
        "--max-rounds",
        "3",
    ]);
    assert!(cli_err(&["benchmark", "protocol"]).contains("--route required"));
    // Empty ledger (text + json), then seeded via a scripted run below.
    cli_ok(&["benchmark", "ledger"]);
    cli_ok(&["benchmark", "ledger", "--json"]);
    cli_ok(&[
        "benchmark",
        "ledger",
        "--benchmark-id",
        "b1",
        "--case-id",
        "c1",
    ]);
    // errors.
    assert!(cli_err(&["benchmark"]).contains("protocol|run|ledger"));
    assert!(cli_err(&["benchmark", "bogus"]).contains("protocol|run|ledger"));
    let _ = cr;
}

#[test]
fn benchmark_run_scripted_dry_run() {
    let cr = cli_root();
    let ledger_dir = std::path::Path::new(&cr.cwd).join("bench-ledger");
    // No --agent-addr → deterministic scripted adapter.
    cli_ok(&[
        "benchmark",
        "run",
        "--benchmark-id",
        "b1",
        "--case-id",
        "c1",
        "--task",
        "do the thing",
        "--ledger-dir",
        ledger_dir.to_str().unwrap(),
        "--expected-evidence",
        "completed",
    ]);
    // Ledger now has an entry → text render with rows.
    cli_ok(&["benchmark", "ledger", "--dir", ledger_dir.to_str().unwrap()]);
    // goal-start route picks the other default arm id.
    cli_ok(&[
        "benchmark",
        "run",
        "--benchmark-id",
        "b2",
        "--case-id",
        "c2",
        "--task",
        "t",
        "--route",
        "future-loop-goal-start-product-mode",
        "--ledger-dir",
        ledger_dir.to_str().unwrap(),
    ]);
    cli_ok(&[
        "benchmark",
        "run",
        "--benchmark-id",
        "b3",
        "--case-id",
        "c3",
        "--task",
        "t",
        "--arm-id",
        "custom_arm",
        "--max-rounds",
        "2",
        "--ledger-dir",
        ledger_dir.to_str().unwrap(),
    ]);
    // errors.
    assert!(cli_err(&["benchmark", "run"]).contains("--benchmark-id required"));
    assert!(cli_err(&["benchmark", "run", "--benchmark-id", "b"]).contains("--case-id required"));
    assert!(
        cli_err(&["benchmark", "run", "--benchmark-id", "b", "--case-id", "c"])
            .contains("--task required")
    );
}

#[test]
fn benchmark_run_grpc_adapter() {
    let cr = cli_root();
    let rt = rt();
    let (addr, _shared) = rt.block_on(spawn_mock(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    }));
    let ledger_dir = std::path::Path::new(&cr.cwd).join("bench-ledger-grpc");
    cli_ok(&[
        "benchmark",
        "run",
        "--benchmark-id",
        "bg",
        "--case-id",
        "cg",
        "--task",
        "write the artifact",
        "--agent-addr",
        &addr,
        "--expected-evidence",
        "artifact",
        "--ledger-dir",
        ledger_dir.to_str().unwrap(),
    ]);
}

// ── replay ─────────────────────────────────────────────────────────────────

#[test]
fn replay_record_and_run() {
    let cr = cli_root();
    let gid = init_goal(&cr, "replay goal");
    // Print mode.
    cli_ok(&["replay", "record", "--goal", &gid]);
    // --out: create then append. (Keep the agent REGISTERED — a case recorded
    // for an unregistered agent captures the identity-gate packet, which the
    // minimal case snapshot cannot replay faithfully; documented limitation.)
    cli_ok(&["agent", "register", "--goal", &gid, "--agent-id", "w1"]);
    let case_file = std::path::Path::new(&cr.cwd).join("cases.json");
    cli_ok(&[
        "replay",
        "record",
        "--goal",
        &gid,
        "--out",
        case_file.to_str().unwrap(),
    ]);
    cli_ok(&[
        "replay",
        "record",
        "--goal",
        &gid,
        "--out",
        case_file.to_str().unwrap(),
        "--case-id",
        "case-2",
        "--agent-id",
        "w1",
    ]);
    // Replay: the recorded cases must match the current kernel.
    cli_ok(&["replay", "run", "--case", case_file.to_str().unwrap()]);
    cli_ok(&[
        "replay",
        "run",
        "--case",
        case_file.to_str().unwrap(),
        "--json",
    ]);
    // errors.
    assert!(cli_err(&["replay", "record"]).contains("--goal required"));
    assert!(cli_err(&["replay", "record", "--goal", "goal_nope"]).contains("not found"));
    assert!(cli_err(&["replay", "run"]).contains("--case required"));
    assert!(cli_err(&["replay", "run", "--case", "/nonexistent.json"]).contains(""));
    assert!(cli_err(&["replay"]).contains("record|run|corpus"));
    assert!(cli_err(&["replay", "bogus"]).contains("record|run|corpus"));
}

#[test]
fn replay_corpus_surface() {
    let cr = cli_root();
    let gid = init_goal(&cr, "corpus goal");
    // A base packet with no patches/ablations produces ZERO cases (error
    // path); print mode needs at least one patch.
    assert!(cli_err(&["replay", "corpus", "build", "--goal", &gid]).contains("at least one case"));
    cli_ok(&[
        "replay",
        "corpus",
        "build",
        "--goal",
        &gid,
        "--patch",
        "{\"quota_spent\": 0}",
    ]);
    // Full flag set: patches + out file (patches merge INTO the packet, so
    // the candidate stays equivalent and the gate passes).
    let corpus = std::path::Path::new(&cr.cwd).join("corpus.json");
    cli_ok(&[
        "replay",
        "corpus",
        "build",
        "--goal",
        &gid,
        "--out",
        corpus.to_str().unwrap(),
        "--patch-name",
        "p",
        "--patch",
        "{\"quota_spent\": 1}",
        "--patch",
        "{\"quota_spent\": 2}",
    ]);
    // Corpus run: text + json.
    cli_ok(&[
        "replay",
        "corpus",
        "run",
        "--corpus",
        corpus.to_str().unwrap(),
    ]);
    cli_ok(&[
        "replay",
        "corpus",
        "run",
        "--corpus",
        corpus.to_str().unwrap(),
        "--repeats",
        "2",
        "--seed",
        "7",
        "--json",
    ]);
    // An ablation case the stub actor does NOT fail closed on → gate bail.
    let bad_corpus = std::path::Path::new(&cr.cwd).join("corpus-bad.json");
    cli_ok(&[
        "replay",
        "corpus",
        "build",
        "--goal",
        &gid,
        "--out",
        bad_corpus.to_str().unwrap(),
        "--ablate",
        "quota",
    ]);
    assert!(cli_err(&[
        "replay",
        "corpus",
        "run",
        "--corpus",
        bad_corpus.to_str().unwrap()
    ])
    .contains("gate"));
    // repeats bounds.
    assert!(cli_err(&[
        "replay",
        "corpus",
        "run",
        "--corpus",
        corpus.to_str().unwrap(),
        "--repeats",
        "1",
    ])
    .contains("between 2 and 20"));
    // errors.
    assert!(cli_err(&["replay", "corpus", "build"]).contains("--goal required"));
    assert!(cli_err(&["replay", "corpus", "build", "--goal", "goal_nope"]).contains("not found"));
    assert!(cli_err(&[
        "replay",
        "corpus",
        "build",
        "--goal",
        &gid,
        "--patch",
        "{not json",
    ])
    .contains("JSON"));
    assert!(cli_err(&["replay", "corpus", "run"]).contains("--corpus required"));
    assert!(cli_err(&["replay", "corpus", "run", "--corpus", "/nonexistent.json"]).contains(""));
    assert!(cli_err(&["replay", "corpus"]).contains("build|run"));
    assert!(cli_err(&["replay", "corpus", "bogus"]).contains("build|run"));
}
