//! `future-loop` — FutureOS loop control plane CLI.
//!
//! Commands (mirror the reference core tick, implemented natively):
//!   goal init    — create a durable goal (registry + event ledger)
//!   todo add     — add an agent/user/gate/monitor todo
//!   todo claim   — claim a todo (owner identity)
//!   todo complete — complete a todo; REQUIRES --no-follow-up or --successor
//!   gate resolve — resolve a user gate with a decision payload
//!   status       — project the active state (todos, gaps, next action)
//!   quota should-run — emit the typed ShouldRunPacket (deterministic)
//!   run          — drive one bounded gRPC turn + writeback (needs agent)
//!
//! State lives under `--root` (default `~/.future/loop/`), one goal per
//! directory, event-sourced: `loop status` replays the ledger each time.

use std::collections::HashMap;
use std::time::SystemTime;

use crate::cli::registry::CommandRegistry;
use crate::decision::{complete_todo, decide_for, MAX_REPAIR_ATTEMPTS};
use crate::executor::{execute_turn, writeback};
use crate::state::{now_epoch, Goal, RunRecord, TaskClass, Todo, TodoStatus};
use crate::store::{Event, Store};
use anyhow::{bail, Result};

/// Materialize the project-local active-state projection for one goal:
/// `<cwd>/.future/loop/goals/<id>/ACTIVE_GOAL_STATE.md`.
fn sync_compat(store: &Store, goal_id: &str) -> Result<()> {
    let Some(goal) = store.replay(goal_id)? else {
        return Ok(());
    };
    crate::compat::write_active_state(&store.goal_dir(goal_id), &goal)?;
    Ok(())
}

/// Recompute and persist the active-state Next Action line. Every todo /
/// gate mutation must call this BEFORE `sync_compat` so `status` never shows
/// a stale next action (previously only the run writeback path refreshed
/// `next_action.txt`).
fn refresh_next_action(store: &Store, goal_id: &str) -> Result<()> {
    let goal = store
        .replay(goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let next = goal
        .runnable_advancement()
        .next()
        .map(|t| t.text.clone())
        .unwrap_or_else(|| "all todos complete; no further action".to_string());
    store.set_next_action(goal_id, &next)?;
    Ok(())
}

/// Project-local state root: `<cwd>/.future/loop/` (run future-loop from the
/// project dir, or override with FUTURE_LOOP_ROOT). All goal state stays
/// inside the project.
fn root_dir() -> String {
    std::env::var("FUTURE_LOOP_ROOT").unwrap_or_else(|_| {
        format!(
            "{}/.future/loop",
            std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| ".".into())
        )
    })
}

fn gen_id(prefix: &str) -> String {
    // Loop-style ids: <prefix>_<12hex> (underscore, no dash) — matches the
    // reference format and pre-existing goal/todo ids.
    format!(
        "{}_{}",
        prefix,
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    )
}

/// Entry point shared by the standalone `future-loop` binary and the
/// embedded `future loop` CLI command. `prog` is the invocation name used
/// in error hints ("future-loop" vs "future loop"); it is stored in PROG so
/// any command handler can reference it.
pub fn run(prog: &str, args: Vec<String>) -> Result<()> {
    let _ = PROG.set(prog.to_string());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(main_from_args(prog, args))
}

/// Invocation name for error hints — set by `run`, falls back to the
/// standalone binary name.
fn prog() -> &'static str {
    PROG.get().map(String::as_str).unwrap_or("future-loop")
}

static PROG: std::sync::OnceLock<String> = std::sync::OnceLock::new();

async fn main_from_args(prog: &str, args: Vec<String>) -> Result<()> {
    let include_experimental = args.iter().any(|a| a == "--include-experimental");
    if args.is_empty() || matches!(args[0].as_str(), "--help" | "-h" | "help") {
        return cli_help(&build_cli_registry(), include_experimental);
    }
    let registry = build_cli_registry();
    // G-26: the registry is the dispatch validation point — unknown commands
    // fail with a hint (aggregated help) instead of a bare match fallthrough.
    if registry.find(&args[0], include_experimental).is_none()
        && resolve_capability_hook(&args[0]).is_none()
    {
        bail!("unknown command `{}` (try `{prog} --help`)", args[0]);
    }
    let mut store = Store::open(&root_dir())?;
    match args[0].as_str() {
        "goal" => cmd_goal(&mut store, &args[1..]),
        "agent" => cmd_agent(&mut store, &args[1..]),
        "todo" => cmd_todo(&mut store, &args[1..]),
        "gate" => cmd_gate(&mut store, &args[1..]),
        "backup" => cmd_backup(&store, &args[1..]),
        "authority" => cmd_authority(&mut store, &args[1..]),
        "replan" => cmd_replan(&mut store, &args[1..]),
        "profile" => cmd_profile(&mut store, &args[1..]),
        "status" => cmd_status(&store, &args[1..]),
        "quota" => cmd_quota(&store, &args[1..]),
        "scheduler" => cmd_scheduler(&store, &args[1..]),
        "store" => cmd_store(&mut store, &args[1..]),
        "backfill" => cmd_backfill(&mut store, &args[1..]),
        "privacy" => cmd_privacy(&store, &args[1..]),
        "lease" => cmd_lease(&mut store, &args[1..]),
        "runs" => cmd_runs(&store, &args[1..]),
        "heartbeat-prompt" => cmd_heartbeat(&store, &args[1..]),
        "worker-bridge" => cmd_worker_bridge(&mut store, &args[1..]).await,
        "serve-status" => cmd_serve_status(&store, &args[1..]),
        "capability" => cmd_capability(&store, &args[1..]),
        "models" => cmd_models(&args[1..]).await,
        "diagnose" => cmd_diagnose(&store, &args[1..]),
        "run" => cmd_run(&mut store, &args[1..]).await,
        // ── P3 commands ──────────────────────────────────────────────────
        "extension" => cmd_extension(&store, &args[1..]),
        "catalog" => cmd_catalog(&store, &args[1..]),
        "scope" => cmd_scope(&store, &args[1..]),
        "lane" => cmd_lane(&store, &args[1..]),
        "supervisor" => cmd_supervisor(&mut store, &args[1..]),
        "handoff" => cmd_handoff(&store, &args[1..]),
        "task-graph" => cmd_task_graph(&store, &args[1..]),
        "attention" => cmd_attention(&store, &args[1..]),
        "inbox" => cmd_inbox(&store, &args[1..]),
        "registry" => cmd_registry(&registry, &args[1..]),
        // ── P4 commands (G-18 / G-19 / G-20 / G-27) ───────────────────────
        "benchmark" => cmd_benchmark(&store, &args[1..]).await,
        "replay" => cmd_replay(&store, &args[1..]),
        "canary" => cmd_canary(&store, &args[1..]),
        "version" => cmd_version(&store, &args[1..]),
        "doctor" => cmd_doctor(&store, &args[1..]).await,
        "history" => cmd_history(&store, &args[1..]),
        "turn" => cmd_turn(&store, &args[1..]),
        "todo-event" => cmd_todo_event(&store, &args[1..]),
        "evidence-log" => cmd_evidence_log(&store, &args[1..]),
        other => {
            // G-24 per-capability command hook (e.g. `loopx issue-fix --input ...`).
            if let Some((capability_id, _purpose)) = resolve_capability_hook(other) {
                return cmd_capability_hook(other, &capability_id, &args[1..]);
            }
            bail!("unknown command `{other}` (try `{prog} --help`)")
        }
    }
}

/// G-26: build the command registry — groups + commands + capability command
/// hooks (G-24), the aggregated help surface.
fn build_cli_registry() -> CommandRegistry {
    use crate::capabilities::catalog::CapabilityCatalog;
    let mut r = CommandRegistry::new();

    let goal = r.group("goal", "goal lifecycle");
    r.command(
        goal,
        "goal",
        "create a durable goal",
        "goal init --objective \"...\" [--cwd DIR] [--goal-doc TEXT] [--goal-id ID]",
    );
    r.command(
        goal,
        "status",
        "project the active state",
        "status [--goal G]",
    );
    r.command(
        goal,
        "models",
        "list models available from the agent",
        "models [--format json]",
    );
    r.command(
        goal,
        "diagnose",
        "per-goal diagnostic surface (decision / gaps / runs)",
        "diagnose --goal G [--format json]",
    );

    let todo = r.group("todo", "todo work graph");
    r.command(
        todo,
        "todo",
        "add/claim/complete/archive todos",
        "todo add|claim|complete|archive --goal G ...",
    );
    r.command(
        todo,
        "gate",
        "resolve a user gate",
        "gate resolve --goal G --todo-id G1 --decision \"...\"",
    );
    r.command(
        todo,
        "replan",
        "ack a replan obligation / list obligations",
        "replan ack --goal G --delta-kind ... | replan obligations --goal G",
    );
    r.command(
        todo,
        "lease",
        "task lease lifecycle (claim/renew/release/expire/status)",
        "lease claim|renew|release|expire|status --goal G --todo-id T --agent-id A",
    );
    r.command(
        todo,
        "task-graph",
        "todo dependency graph (G-14)",
        "task-graph --goal G",
    );

    let agent = r.group("agent", "agent sessions");
    r.command(
        agent,
        "agent",
        "register/onboard agents",
        "agent onboard --goal G --agent-id A [--capabilities c1,c2]",
    );
    r.command(
        agent,
        "list",
        "registered agents + live execution status (leases)",
        "agent list --goal G",
    );
    r.command(
        agent,
        "scope",
        "identity-scoped agent frontier (G-16)",
        "scope --goal G --agent-id A [--exclude X]",
    );
    r.command(
        agent,
        "lane",
        "agent lane recommendation (G-16)",
        "lane --goal G --agent-id A",
    );
    r.command(
        agent,
        "supervisor",
        "supervisor proposal/receipt events (G-16)",
        "supervisor propose|receipt|events --goal G ...",
    );

    let capability = r.group("capability", "capability framework");
    r.command(
        capability,
        "capability",
        "list / propose / commands for capabilities",
        "capability list|propose|commands [--name X] [--input \"...\"]",
    );
    r.command(
        capability,
        "catalog",
        "capability catalog metadata: status/stage/provider (G-23)",
        "catalog [--name X] [--json]",
    );
    // G-24 per-capability command hooks (from the catalog; experimental
    // commands hidden unless --include-experimental).
    let catalog = CapabilityCatalog::with_builtin();
    for record in catalog.records(true) {
        for c in &record.commands {
            let usage = format!("{} --input \"...\" [--include-experimental]", c.name);
            if record.is_experimental() {
                r.command_experimental(capability, &c.name, &c.purpose, &usage);
            } else {
                r.command(capability, &c.name, &c.purpose, &usage);
            }
        }
    }

    let extension = r.group("extension", "extension lifecycle (G-21)");
    r.command(extension, "extension", "install/upgrade/enable/disable/rollback/status/capabilities", "extension install --manifest PATH [--execute] | enable|disable|rollback --id X [--execute] | status [--id X] | capabilities");

    let ops = r.group("ops", "operations / diagnostics");
    r.command(
        ops,
        "version",
        "print version + schema surface (G-27)",
        "version",
    );
    r.command(
        ops,
        "doctor",
        "run the diagnostic surface (smoke + ledger + agent probe)",
        "doctor [--goal G] [--agent-addr ADDR]",
    );
    r.command(
        ops,
        "history",
        "goal run history (ledger-derived)",
        "history --goal G",
    );
    r.command(
        ops,
        "turn",
        "render the per-turn envelope for a todo",
        "turn --goal G --todo-id T [--agent-id A]",
    );
    r.command(
        ops,
        "todo-event",
        "event history of one todo",
        "todo-event --goal G --todo-id T",
    );
    r.command(
        ops,
        "evidence-log",
        "evidence trail (attached + run + completion evidence)",
        "evidence-log --goal G [--todo-id T]",
    );
    r.command(ops, "backup", "back up a goal", "backup --goal G");
    r.command(
        ops,
        "authority",
        "set authority declaration",
        "authority set --goal G --write-scope ... [--requires-approval ...]",
    );
    r.command(
        ops,
        "profile",
        "set execution profile",
        "profile set --goal G --outcome-floor N",
    );
    r.command(
        ops,
        "quota",
        "quota should-run / usage / spend",
        "quota should-run --goal G [--format json] | usage [--goal G] [--all] | spend --goal G",
    );
    r.command(
        ops,
        "scheduler",
        "scheduler tick/show/record-host-failure",
        "scheduler tick|show|record-host-failure --goal G [--agent-id A]",
    );
    r.command(
        ops,
        "store",
        "event-store schema migration / ledger integrity",
        "store migrate|verify|bridge --goal G",
    );
    r.command(
        ops,
        "backfill",
        "markdown backfill into the event ledger",
        "backfill --goal G [--from PATH] [--privacy LEVEL] [--dry-run]",
    );
    r.command(
        ops,
        "privacy",
        "privacy-graded projection",
        "privacy --goal G [--level LEVEL] [--json]",
    );
    r.command(
        ops,
        "runs",
        "run history lifecycle (history/compact/index/retention/stale)",
        "runs history|compact|index|retention|stale --goal G [--keep N] [--cutoff TS] [--rebuild]",
    );
    r.command(
        ops,
        "heartbeat-prompt",
        "render the per-turn re-entry packet",
        "heartbeat-prompt --goal G [--agent-id A]",
    );
    r.command(
        ops,
        "worker-bridge",
        "run the worker bridge",
        "worker-bridge --goal G [--agent-id A] [--max-turns N]",
    );
    r.command(
        ops,
        "serve-status",
        "serve the status projection",
        "serve-status [--goal G]",
    );
    r.command(
        ops,
        "run",
        "drive one bounded gRPC turn (requires --agent-id; auto-registers)",
        "run --goal G --agent-id A [--model M] [--thinking-level L] [--max-turns N] [--lease-secs N]",
    );

    let work_items = r.group("work-items", "attention / operator inbox (G-15)");
    r.command(
        work_items,
        "attention",
        "project the attention queue",
        "attention [--goal G] [--all]",
    );
    r.command(
        work_items,
        "inbox",
        "project the operator inbox urgency",
        "inbox --project DIR [--scope addressed_only|configured_chat_all] [--name NAME]",
    );

    let handoff = r.group("handoff", "project handoff (G-17)");
    r.command(
        handoff,
        "handoff",
        "generate the handoff document + delivery contract",
        "handoff --goal G [--write]",
    );

    let cli = r.group("cli", "command registry");
    r.command(
        cli,
        "registry",
        "inspect the CLI registry (groups/commands)",
        "registry [--json] [--include-experimental]",
    );

    let benchmark = r.group("benchmark", "benchmark closed loop (G-18)");
    r.command(
        benchmark,
        "benchmark",
        "benchmark protocol|run|ledger — loop protocol, qualification run, run ledger",
        "benchmark protocol --route R | run --benchmark-id X --case-id Y --task \"...\" [--agent-addr A] | ledger [--benchmark-id X]",
    );

    let replay = r.group("replay", "decision replay / behavior corpus (G-19)");
    r.command(
        replay,
        "replay",
        "record / replay public-safe decisions + model-behavior corpus",
        "replay record --goal G [--out PATH] | replay run --case PATH | replay corpus build|run ...",
    );

    let canary = r.group("canary", "canary smoke (G-20)");
    r.command(
        canary,
        "canary",
        "run a smoke profile (release gate default)",
        "canary smoke [--profile core-control-plane|extension-runtime|release-gate] [--json]",
    );

    r
}

/// Resolve a top-level command to a capability command hook (G-24). Returns
/// (capability_id, purpose) when the command matches a catalog command.
fn resolve_capability_hook(command: &str) -> Option<(String, String)> {
    let catalog = crate::capabilities::catalog::CapabilityCatalog::with_builtin();
    for record in catalog.records(true) {
        for c in &record.commands {
            if c.name == command {
                return Some((record.id.clone(), c.purpose.clone()));
            }
        }
    }
    None
}

/// Run a per-capability command hook: `loopx <cap-command> --input "..."` —
/// the capability's propose pipeline, printed like `capability propose`.
fn cmd_capability_hook(command: &str, capability_id: &str, args: &[String]) -> Result<()> {
    let registry = crate::capabilities::CapabilityRegistry::with_builtin();
    let mut input = None;
    parse_pairs(args, |k, v| {
        if k == "--input" {
            input = Some(v)
        }
    });
    let input = input.unwrap_or_default();
    let Some(cap) = registry.get(capability_id) else {
        bail!(
            "unknown capability `{capability_id}` (see `{} capability list`)",
            prog()
        );
    };
    let proposals = cap.propose(&input);
    println!(
        "capability hook `{command}` ({capability_id}) → {} proposal(s):",
        proposals.len()
    );
    for p in proposals {
        let kind = match p.kind {
            crate::capabilities::ProposalKind::SuccessorTodo => "successor_todo",
            crate::capabilities::ProposalKind::NoFollowUp => "no_followup",
            crate::capabilities::ProposalKind::Repair => "repair",
            crate::capabilities::ProposalKind::Gate => "gate",
            crate::capabilities::ProposalKind::Monitor => "monitor",
        };
        println!("  [{kind}] {}", p.reason);
        if let Some(t) = p.todo {
            println!("    → todo: {}", t.text);
        }
        if let Some(q) = p.gate_question {
            println!("    → gate: {q}");
        }
    }
    Ok(())
}

fn cli_help(registry: &CommandRegistry, include_experimental: bool) -> Result<()> {
    let mut text = registry.render_help(include_experimental);
    // When invoked as `future loop`, adapt the USAGE line to the actual
    // invocation (the standalone binary keeps "future-loop").
    if prog() != "future-loop" {
        text = text.replacen(
            "USAGE: future-loop <command> [args]",
            &format!("USAGE: {} <command> [args]", prog()),
            1,
        );
    }
    print!("{text}");
    Ok(())
}

// ── goal ───────────────────────────────────────────────────────────────────

fn cmd_goal(store: &mut Store, args: &[String]) -> Result<()> {
    // goal cancel / goal delete (lifecycle management)
    if args.first().map(|s| s.as_str()) == Some("cancel") {
        return cmd_goal_cancel(store, &args[1..]);
    }
    if args.first().map(|s| s.as_str()) == Some("delete") {
        return cmd_goal_delete(store, &args[1..]);
    }
    let mut objective = None;
    let mut cwd = None;
    let mut goal_id = None;
    let mut goal_doc = None;
    parse_pairs(args, |k, v| match k {
        "--objective" => objective = Some(v),
        "--cwd" => cwd = Some(v),
        "--goal-id" => goal_id = Some(v),
        "--goal-doc" => goal_doc = Some(v),
        _ => {}
    });
    let objective = objective.ok_or_else(|| anyhow::anyhow!("goal init requires --objective"))?;
    let goal_id = goal_id.unwrap_or_else(|| gen_id("goal"));
    let cwd = cwd.unwrap_or_else(|| {
        std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    let mut goal = Goal::new(&goal_id, &objective, &cwd);
    let ts = now_epoch();
    goal.created_at = ts;
    store.register(&goal)?;
    store.append(Event::GoalStarted {
        goal_id: goal_id.clone(),
        ts,
    })?;
    if let Some(doc) = goal_doc {
        let _ = std::fs::write(
            std::path::Path::new(&cwd).join("GOAL.md"),
            format!("{doc}\n"),
        );
    }
    // Bootstrap auto-adds an onboarding-connection-validation todo.
    let onboarding = Todo::advancement(
        &gen_id("todo"),
        "[P1] Run `future loop status` for this goal and record the goal \
         count as evidence, or declare an explicit no-follow-up rationale.",
    )
    .at_priority(crate::state::Priority::P1)
    .with_action_kind("onboarding_connection_validation");
    store.append(Event::TodoAdded {
        goal_id: goal_id.clone(),
        todo: onboarding.clone(),
        ts,
    })?;
    store.set_next_action(&goal_id, &onboarding.text)?;
    refresh_next_action(store, &goal_id)?;
    sync_compat(store, &goal_id)?;
    println!("goal {} created ✔ (root {})", goal_id, root_dir());
    Ok(())
}

/// `goal cancel --goal G [--reason ...]` — stop automation while retaining
/// state (the reference: cancel keeps the goal reviewable; automation stops).
fn cmd_goal_cancel(store: &mut Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut reason = "cancelled by user".to_string();
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--reason" => reason = v,
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    store.append(Event::GoalCancelled {
        goal_id: goal_id.clone(),
        reason: reason.clone(),
        ts: crate::state::now_epoch(),
    })?;
    refresh_next_action(store, &goal_id)?;
    // Cancelled goals never run — surface that as the Next Action.
    store.set_next_action(
        &goal_id,
        "goal cancelled — automation stopped, state retained",
    )?;
    sync_compat(store, &goal_id)?;
    println!("goal {goal_id} cancelled ✔ (automation stopped, state retained — reason: {reason})");
    Ok(())
}

/// `goal delete --goal G [--force]` — remove the registry entry + state.
/// Irreversible; requires --force (tip: `goal cancel` keeps state).
fn cmd_goal_delete(store: &mut Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut force = false;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--force" => force = true,
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    if !force {
        bail!(
            "goal delete is irreversible — pass --force to delete goal {goal_id} \
             (tip: use `goal cancel` to stop automation while keeping state)"
        );
    }
    store.delete_goal(&goal_id)?;
    println!("goal {goal_id} deleted ✔ (registry entry + state removed)");
    Ok(())
}

// ── todo ───────────────────────────────────────────────────────────────────

fn cmd_todo(store: &mut Store, args: &[String]) -> Result<()> {
    if args.is_empty() {
        bail!("todo requires add|claim|complete");
    }
    match args[0].as_str() {
        "add" => todo_add(store, &args[1..]),
        "claim" => todo_claim(store, &args[1..]),
        "complete" => todo_complete(store, &args[1..]),
        "archive" => todo_archive(store, &args[1..]),
        "supersede" => todo_supersede(store, &args[1..]),
        "update" => todo_update(store, &args[1..]),
        other => bail!("unknown todo subcommand `{other}`"),
    }
}

fn todo_add(store: &mut Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut role = "agent".to_string();
    let mut class = "advancement".to_string();
    let mut text = None;
    let mut gate_question = None;
    let mut blocks: Vec<String> = vec![];
    let mut priority = None;
    let mut action_kind = None;
    let mut required_capability = None;
    let mut deferred_secs = 0u64;
    let mut title = None;
    let mut task_repository = None;
    let mut continuation_policy = None;
    let mut write_scopes = vec![];
    let mut capability_binding = None;
    let mut goal_bound = false;
    let mut global_gate = false;
    let mut resume_when_cond = None;
    let mut note = None;
    let mut monitor_target = None;
    let mut monitor_policy = None;
    let mut cadence = None;
    let mut verify: Option<String> = None;
    let mut max_validation_attempts: Option<u32> = None;
    parse_pairs(args, |k, v| match k {
        "--goal-bound" => goal_bound = true,
        "--global-gate" => global_gate = true,
        "--resume-when" => resume_when_cond = Some(v),
        "--note" => note = Some(v),
        "--monitor-target" => monitor_target = Some(v),
        "--monitor-policy" => monitor_policy = Some(v),
        "--cadence" => cadence = Some(v),
        "--verify" => verify = Some(v),
        "--max-validation-attempts" => max_validation_attempts = v.parse().ok(),
        "--goal" => goal_id = Some(v),
        "--role" => role = v,
        "--class" => class = v,
        "--text" => text = Some(v),
        "--gate-question" => gate_question = Some(v),
        "--blocks" => blocks = v.split(',').map(|s| s.to_string()).collect(),
        "--priority" => priority = Some(v),
        "--action-kind" => action_kind = Some(v),
        "--required-capability" => required_capability = Some(v),
        "--defer-secs" => deferred_secs = v.parse().unwrap_or(0),
        "--title" => title = Some(v),
        "--task-repository" => task_repository = Some(v),
        "--continuation-policy" => continuation_policy = Some(v),
        "--required-write-scope" => {
            write_scopes = v.split(',').map(|s| s.trim().to_string()).collect()
        }
        "--capability-binding-ref" => capability_binding = Some(v),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let text = text.ok_or_else(|| anyhow::anyhow!("--text required"))?;
    store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let id = gen_id("todo");
    let mut todo = match (role.as_str(), class.as_str()) {
        ("agent", "advancement") => Todo::advancement(&id, &text),
        ("agent", "monitor") => Todo::monitor(&id, &text, std::time::Duration::from_secs(60)),
        ("user", "user_gate") | (_, "user_gate") => {
            let q = gate_question.unwrap_or_else(|| text.clone());
            let b: Vec<&str> = blocks.iter().map(|s| s.as_str()).collect();
            Todo::user_gate(&id, &q, &b)
        }
        ("user", "user_action") => Todo::user_action(&id, &text),
        ("agent", "blocker") => {
            let b: Vec<&str> = blocks.iter().map(|s| s.as_str()).collect();
            Todo::blocker(&id, &text, &b)
        }
        _ => Todo::advancement(&id, &text),
    };
    // Apply --blocks for every task class (previously only user_gate/blocker
    // attached them; advancement/monitor silently dropped the dependency chain).
    if !blocks.is_empty() {
        let b: Vec<&str> = blocks.iter().map(|s| s.as_str()).collect();
        todo = todo.blocking(&b);
    }
    todo.goal_bound = goal_bound;
    todo.global_gate = global_gate;
    // reference rule: user_gate + global_gate implies goal_bound=true.
    if todo.global_gate && todo.class == crate::state::TaskClass::UserGate {
        todo.goal_bound = true;
    }
    if let Some(rw) = resume_when_cond {
        todo.resume_when_text = Some(rw.clone());
        todo.status = crate::state::TodoStatus::Deferred;
        // Numeric `--resume-when N` defers N seconds from now (real deadline,
        // same semantics as --defer-secs); non-numeric keeps legacy +3600s
        // placeholder (text hint only).
        if let Ok(secs) = rw.trim().parse::<u64>() {
            todo.resume_when =
                Some(std::time::SystemTime::now() + std::time::Duration::from_secs(secs));
        } else {
            todo.resume_when =
                Some(std::time::SystemTime::now() + std::time::Duration::from_secs(3600));
        }
    }
    if let Some(n) = note {
        todo.note = Some(n);
    }
    if deferred_secs > 0 {
        todo.status = crate::state::TodoStatus::Deferred;
        todo.resume_when =
            Some(std::time::SystemTime::now() + std::time::Duration::from_secs(deferred_secs));
    }
    if let Some(p) = priority {
        let pr = match p.to_uppercase().as_str() {
            "P0" => crate::state::Priority::P0,
            "P2" => crate::state::Priority::P2,
            _ => crate::state::Priority::P1,
        };
        todo.priority = pr;
        // reference convention: todo text carries the [P0]/[P1]/[P2] prefix.
        let tag = format!("[{pr}] ");
        if !todo.text.starts_with(&tag) {
            todo.text = format!("{tag}{}", todo.text);
            if todo.title == todo.text.trim_start_matches(&tag) || todo.title.starts_with(&tag) {
                todo.title = format!("{tag}{}", todo.title.trim_start_matches(&tag));
            }
        }
    }
    if let Some(a) = action_kind {
        todo.action_kind = Some(a);
    }
    if let Some(c) = required_capability {
        todo.required_capability = Some(c);
    }
    if let Some(v) = verify {
        todo.validator = Some(v);
    }
    if let Some(n) = max_validation_attempts {
        todo.max_validation_attempts = n.max(1);
    }
    // G-12: monitor metadata (target / policy / cadence). Cadence drives the
    // first due time via the scheduler cadence parser (15m/1h/2d or class).
    if let Some(t) = monitor_target {
        todo.monitor_target = Some(t);
    }
    if let Some(p) = monitor_policy {
        todo.monitor_policy = Some(p);
    }
    if let Some(c) = cadence {
        todo.monitor_cadence = Some(c.clone());
        if let Some(secs) = crate::scheduler::state::monitor_cadence_secs(&c) {
            todo.resume_when =
                Some(std::time::SystemTime::now() + std::time::Duration::from_secs(secs));
        }
    }
    if let Some(t) = title {
        todo.title = t;
    }
    if let Some(r) = task_repository {
        todo.task_repository = Some(r);
    }
    if let Some(cp) = continuation_policy {
        todo.continuation_policy = Some(cp);
    }
    if !write_scopes.is_empty() {
        todo.required_write_scope = write_scopes;
    }
    if let Some(cb) = capability_binding {
        todo.capability_binding_ref = Some(cb);
    }
    store.append(Event::TodoAdded {
        goal_id: goal_id.clone(),
        todo,
        ts: now_epoch(),
    })?;
    refresh_next_action(store, &goal_id)?;
    sync_compat(store, &goal_id)?;
    println!("todo {id} added to {goal_id} ✔");
    Ok(())
}

fn todo_claim(store: &mut Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut todo_id = None;
    let mut agent_id = None;
    let mut lease_secs = 3600u64;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--todo-id" => todo_id = Some(v),
        "--agent-id" => agent_id = Some(v),
        "--lease-secs" => lease_secs = v.parse().unwrap_or(3600),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let todo_id = todo_id.ok_or_else(|| anyhow::anyhow!("--todo-id required"))?;
    let mut goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let agent = agent_id.unwrap_or_else(|| "default-agent".to_string());
    if !goal.is_registered_agent(Some(&agent)) {
        bail!("agent `{agent}` is not registered for goal {goal_id} — `{} agent register --goal {goal_id} --agent-id {agent}` first", prog());
    }
    let now = crate::state::now_epoch();
    let claimed = goal
        .todo_mut(&todo_id)
        .map(|t| t.claim(&agent, lease_secs, now))
        .unwrap_or(false);
    if !claimed {
        bail!("todo {todo_id} cannot be claimed: not open, or another agent holds a live lease");
    }
    let expires = now + lease_secs;
    store.append(Event::TodoClaimed {
        goal_id: goal_id.clone(),
        todo_id: todo_id.clone(),
        agent_id: agent.clone(),
        lease_expires_at: expires,
        ts: now,
    })?;
    refresh_next_action(store, &goal_id)?;
    sync_compat(store, &goal_id)?;
    println!("todo {todo_id} claimed by {agent} until epoch {expires} ✔");
    Ok(())
}

/// `loopx agent register --goal G --agent-id A` — register a peer (LoopX:
/// coordination.registered_agents; precondition for quota --agent-id).
fn cmd_agent(store: &mut Store, args: &[String]) -> Result<()> {
    if args.first().map(|s| s.as_str()) == Some("onboard") {
        return cmd_agent_onboard(store, &args[1..]);
    }
    if args.first().map(|s| s.as_str()) == Some("list") {
        return cmd_agent_list(store, &args[1..]);
    }
    let mut goal_id = None;
    let mut agent_id = None;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--agent-id" => agent_id = Some(v),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let agent_id = agent_id.ok_or_else(|| anyhow::anyhow!("--agent-id required"))?;
    store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    store.append(Event::AgentRegistered {
        goal_id: goal_id.clone(),
        agent_id: agent_id.clone(),
        ts: crate::state::now_epoch(),
    })?;
    println!("agent `{agent_id}` registered for {goal_id} ✔");
    Ok(())
}

/// `loopx agent onboard --goal G --agent-id A [--capability shell,github]`
/// — register a peer AND declare its capabilities (LoopX: agent_profiles;
/// input to the capability gate).
fn cmd_agent_onboard(store: &mut Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut agent_id = None;
    let mut capabilities = vec![];
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--agent-id" => agent_id = Some(v),
        "--capability" | "--capabilities" => {
            capabilities = v.split(',').map(|s| s.trim().to_string()).collect()
        }
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let agent_id = agent_id.ok_or_else(|| anyhow::anyhow!("--agent-id required"))?;
    store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    store.append(Event::AgentOnboarded {
        goal_id: goal_id.clone(),
        agent_id: agent_id.clone(),
        capabilities: capabilities.clone(),
        ts: crate::state::now_epoch(),
    })?;
    println!("agent `{agent_id}` onboarded (capabilities={capabilities:?}) ✔");
    Ok(())
}

/// `future loop agent list --goal G` — registered agents + their current
/// execution status. Status is derived from the live task-lease ledger:
/// `running` = the agent holds a live lease on a todo right now (shown with
/// the lease's remaining time); `idle` = registered but holding no lease.
/// Also shows declared capabilities and the agent's most recent activity.
/// Intended as the pre-flight check before `agent register`/`onboard`, so
/// parallel workers reuse existing ids instead of re-registering the same
/// one (each concurrent run needs its own unique id).
fn cmd_agent_list(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        }
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;

    // Event-derived metadata: most recent activity timestamp per agent
    // (registration, onboarding, or any lease transition).
    let now = now_epoch();
    let mut last_active: HashMap<String, u64> = HashMap::new();
    for ev in store.events(&goal_id).unwrap_or_default() {
        let (agent, ts) = match &ev.event {
            Event::AgentRegistered { agent_id, ts, .. }
            | Event::AgentOnboarded { agent_id, ts, .. }
            | Event::TodoClaimed { agent_id, ts, .. }
            | Event::TodoRenewed { agent_id, ts, .. }
            | Event::TodoReleased { agent_id, ts, .. } => (agent_id.as_str(), *ts),
            _ => continue,
        };
        last_active
            .entry(agent.to_string())
            .and_modify(|t| *t = (*t).max(ts))
            .or_insert(ts);
    }

    if goal.registered_agents.is_empty() {
        println!("no agents registered for {goal_id}");
        return Ok(());
    }
    println!(
        "agents registered for {goal_id} ({}):",
        goal.registered_agents.len()
    );
    println!(
        "  {:<12} {:<8} {:<32} {:<14} {:<12}",
        "agent_id", "status", "work-on", "capabilities", "last-active"
    );
    for aid in &goal.registered_agents {
        let mut work: Vec<String> = Vec::new();
        for t in goal.todos.iter() {
            if t.claimed_by.as_deref() == Some(aid.as_str())
                && t.lease_expires_at.map(|e| e > now).unwrap_or(false)
            {
                let left = t.lease_expires_at.unwrap().saturating_sub(now);
                work.push(format!("{} (lease {} left)", t.id, human_dur(left)));
            }
        }
        let status = if work.is_empty() { "idle" } else { "running" };
        let work_label = if work.is_empty() {
            "-".to_string()
        } else {
            work.join("; ")
        };
        let caps = goal
            .agent_profiles
            .iter()
            .find(|p| p.id == *aid)
            .map(|p| p.capabilities.join(","))
            .unwrap_or_else(|| "-".to_string());
        let last = last_active
            .get(aid)
            .map(|ts| format!("{} ago", human_dur(now.saturating_sub(*ts))))
            .unwrap_or_else(|| "-".to_string());
        println!(
            "  {:<12} {:<8} {:<32} {:<14} {:<12}",
            aid, status, work_label, caps, last
        );
    }
    println!(
        "hint: agent ids are goal-scoped; check this list before `agent register`/`onboard` \
         to avoid duplicate ids (each parallel worker needs its own unique id)"
    );
    Ok(())
}

/// Compact human duration ("59s" / "4m12s" / "3h59m") for lease/activity
/// display in `agent list`.
fn human_dur(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}

fn todo_complete(store: &mut Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut todo_id = None;
    let mut no_follow_up = false;
    let mut successor = None;
    let mut evidence = None;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--todo-id" => todo_id = Some(v),
        "--no-follow-up" => no_follow_up = true,
        "--successor" => successor = Some(v),
        "--evidence" => evidence = Some(v),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let todo_id = todo_id.ok_or_else(|| anyhow::anyhow!("--todo-id required"))?;
    let mut goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    // reference completion contract: a completed advancement todo must declare
    // closure intent — successor OR no-follow-up; silent completion is rejected.
    let t = goal
        .todo(&todo_id)
        .ok_or_else(|| anyhow::anyhow!("todo {todo_id} not found"))?;
    if t.class == TaskClass::Advancement && !no_follow_up && successor.is_none() {
        bail!(
            "agent todo completion must declare --no-follow-up or --successor \
             (completion policy, successor, and no-follow-up contracts are enforced)"
        );
    }
    // Gate enforcement at the CLI too — aligned with the run loop's semantics:
    // any OPEN user gate freezes gated work (decision step 1 → AskUser) until
    // it is resolved. Completing a non-gate todo while a gate is open would
    // bypass that contract; the run loop already refuses to schedule it, this
    // closes the manual `todo complete` bypass. Resolving the gate itself is
    // the gate's own path (`gate resolve`) — gates are never completed here.
    if t.class != TaskClass::UserGate && t.class != TaskClass::Blocker {
        let open_gates: Vec<String> = goal.open_gates().map(|g| g.id.clone()).collect();
        if !open_gates.is_empty() {
            bail!(
                "todo {todo_id} cannot be completed while open gate(s) [{}] are pending — \
                 resolve them first (`future loop gate resolve --goal {goal_id} --todo-id <gate> --decision \"...\"`)",
                open_gates.join(", ")
            );
        }
    }
    let successors = successor.clone().into_iter().collect::<Vec<_>>();
    store.append(Event::TodoCompleted {
        goal_id: goal_id.clone(),
        todo_id: todo_id.clone(),
        no_follow_up,
        successor_ids: successors.clone(),
        evidence: evidence.clone(),
        ts: now_epoch(),
    })?;
    complete_todo(&mut goal, &todo_id, no_follow_up, successors);
    if let Some(ev) = &evidence {
        if let Some(t) = goal.todo_mut(&todo_id) {
            t.evidence = Some(ev.clone());
        }
    }
    refresh_next_action(store, &goal_id)?;
    sync_compat(store, &goal_id)?;
    println!("todo {todo_id} → done (no_follow_up={no_follow_up}) ✔");
    Ok(())
}

// ── gate ───────────────────────────────────────────────────────────────────

fn cmd_gate(store: &mut Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut todo_id = None;
    let mut decision = None;
    let mut note = None;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--todo-id" => todo_id = Some(v),
        "--decision" => decision = Some(v),
        "--note" => note = Some(v),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let todo_id = todo_id.ok_or_else(|| anyhow::anyhow!("--todo-id required"))?;
    let decision = decision.ok_or_else(|| anyhow::anyhow!("--decision required"))?;
    let mut goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    store.append(Event::GateResolved {
        goal_id: goal_id.clone(),
        todo_id: todo_id.clone(),
        decision: decision.clone(),
        note: note.clone(),
        ts: now_epoch(),
    })?;
    if let Some(t) = goal.todo_mut(&todo_id) {
        t.status = TodoStatus::Done;
        t.decision = Some(decision);
        t.note = note.or(t.note.take());
    }
    refresh_next_action(store, &goal_id)?;
    sync_compat(store, &goal_id)?;
    println!("gate {todo_id} resolved ✔ (decision recorded, flows into blocked todos' packets)");
    Ok(())
}

// ── backup / authority ─────────────────────────────────────────────────────

/// `loopx backup --goal G [--list] [--restore <dir>]` — point-in-time state
/// snapshots (LoopX: state_backup).
fn cmd_backup(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut list = false;
    let mut restore = None;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--list" => list = true,
        "--restore" => restore = Some(v),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    if list {
        for b in store.backups(&goal_id) {
            println!("{b}");
        }
        return Ok(());
    }
    if let Some(dir) = restore {
        store.restore_goal(&goal_id, &dir)?;
        println!("restored {goal_id} from {dir} ✔");
        return Ok(());
    }
    let dest = store.backup_goal(&goal_id)?;
    println!("backup created: {dest} ✔");
    Ok(())
}

/// `loopx authority --goal G [--write-scope /path] [--require-approval publish]`
/// — declare goal authority (write scope + approval-gated action kinds).
fn cmd_authority(store: &mut Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut write_scope = None;
    let mut require = None;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--write-scope" => write_scope = Some(v),
        "--require-approval" => require = Some(v),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let mut goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    if let Some(ws) = write_scope {
        goal.authority.write_scope = ws.split(',').map(|s| s.trim().to_string()).collect();
    }
    if let Some(ra) = require {
        goal.authority.requires_approval = ra.split(',').map(|s| s.trim().to_string()).collect();
    }
    store
        .append(Event::AuthoritySet {
            goal_id: goal_id.clone(),
            write_scope: goal.authority.write_scope.clone(),
            requires_approval: goal.authority.requires_approval.clone(),
            ts: crate::state::now_epoch(),
        })
        .ok();
    println!(
        "authority set: write_scope={:?} requires_approval={:?} ✔",
        goal.authority.write_scope, goal.authority.requires_approval
    );
    Ok(())
}

// ── replan ─────────────────────────────────────────────────────────────────

/// `loopx replan ack --goal G --delta-kind vision_patch|no_followup|...`
/// Records a replan acknowledgment. Clearing a replan obligation requires a
/// frontier-changing delta (LoopX: replan ACK contract).
/// `loopx replan obligations --goal G` lists the unfulfilled replan
/// obligations (G-13 bookkeeping).
fn cmd_replan(store: &mut Store, args: &[String]) -> Result<()> {
    if args.first().map(|s| s.as_str()) == Some("obligations") {
        let mut goal_id = None;
        parse_pairs(&args[1..], |k, v| {
            if k == "--goal" {
                goal_id = Some(v)
            }
        });
        let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
        let goal = store
            .replay(&goal_id)?
            .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
        let obligations = crate::work_items::replan_obligation::unfulfilled_obligations(&goal);
        if obligations.is_empty() {
            println!("no unfulfilled replan obligations for {goal_id}");
            return Ok(());
        }
        println!("unfulfilled replan obligations ({goal_id}):");
        for obligation in &obligations {
            println!("  [{}] {}", obligation.kind, obligation.evidence,);
            if let Some(todo_id) = &obligation.todo_id {
                println!(
                    "       todo_id={todo_id} raised_at={}",
                    obligation.raised_at
                );
            } else {
                println!("       raised_at={}", obligation.raised_at);
            }
        }
        return Ok(());
    }
    let mut goal_id = None;
    let mut delta_kinds: Vec<String> = vec![];
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--delta-kind" => delta_kinds.push(v),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    if delta_kinds.is_empty() {
        bail!("replan ack requires --delta-kind (vision_patch|no_followup|successor_or_supersede|runnable_todo_set|...)");
    }
    if !delta_kinds
        .iter()
        .any(|k| crate::state::delta_kind_changes_frontier(k))
    {
        bail!("--delta-kind must change the machine-visible frontier; got {delta_kinds:?}");
    }
    store.append(Event::ReplanAcked {
        goal_id: goal_id.clone(),
        delta_kinds: delta_kinds.clone(),
        ts: crate::state::now_epoch(),
    })?;
    println!("replan ack recorded (delta_kinds={delta_kinds:?}) ✔");
    Ok(())
}

// ── profile ────────────────────────────────────────────────────────────────

/// `loopx profile set --goal G [--outcome-floor N]` — set execution profile
/// knobs (outcome floor streak threshold, etc).
fn cmd_profile(store: &mut Store, args: &[String]) -> Result<()> {
    if args.first().map(|s| s.as_str()) != Some("set") {
        bail!("profile subcommand must be `set`");
    }
    let mut goal_id = None;
    let mut outcome_floor = None;
    parse_pairs(&args[1..], |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--outcome-floor" => outcome_floor = Some(v),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let mut goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    if let Some(n) = outcome_floor {
        let n: u32 = n
            .parse()
            .map_err(|_| anyhow::anyhow!("--outcome-floor must be a number"))?;
        goal.execution_profile.outcome_floor_streak_threshold = n;
    }
    // Persist the profile via an idempotent re-registration event (registry
    // carries the profile; simplest durable home for the prototype).
    store
        .append(Event::ProfileSet {
            goal_id: goal_id.clone(),
            outcome_floor_streak_threshold: goal.execution_profile.outcome_floor_streak_threshold,
            ts: crate::state::now_epoch(),
        })
        .ok();
    println!(
        "profile set: outcome_floor_streak_threshold={} ✔",
        goal.execution_profile.outcome_floor_streak_threshold
    );
    Ok(())
}

// ── status ─────────────────────────────────────────────────────────────────

fn cmd_status(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_filter = None;
    let mut format = String::new();
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_filter = Some(v)
        } else if k == "--format" {
            format = v;
        }
    });
    if format == "json" {
        return print_status_json(store, goal_filter);
    }
    if let Some(g) = goal_filter {
        let goal = store
            .replay(&g)?
            .ok_or_else(|| anyhow::anyhow!("goal {g} not found"))?;
        print_goal_status(&goal);
        return Ok(());
    }
    if store.registry().is_empty() {
        println!("no goals registered (root {})", root_dir());
        return Ok(());
    }
    for entry in store.registry() {
        if let Ok(Some(goal)) = store.replay(&entry.goal_id) {
            print_goal_status(&goal);
            println!();
        }
    }
    Ok(())
}

/// `loop status --format json` — machine-readable projection (goal, todos
/// with priority/class/status/blocks, terminal flag). Added because piping
/// the human-readable status used to silently yield nothing scriptable.
fn print_status_json(store: &Store, goal_filter: Option<String>) -> Result<()> {
    let mut out = vec![];
    let ids: Vec<String> = match &goal_filter {
        Some(g) => vec![g.clone()],
        None => store.registry().iter().map(|e| e.goal_id.clone()).collect(),
    };
    for gid in ids {
        let goal = store
            .replay(&gid)?
            .ok_or_else(|| anyhow::anyhow!("goal {gid} not found"))?;
        out.push(serde_json::json!({
            "goal_id": goal.goal_id,
            "objective": goal.objective,
            "status": goal.status,
            "todos": goal.todos.iter().map(|t| serde_json::json!({
                "id": t.id,
                "text": t.text,
                "class": format!("{:?}", t.class),
                "status": status_label(t),
                "priority": format!("{:?}", t.priority),
                "blocks": t.blocked_by_gate.clone().unwrap_or_default(),
            })).collect::<Vec<_>>(),
        }));
    }
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

fn print_goal_status(goal: &Goal) {
    println!("goal      : {}", goal.goal_id);
    println!("objective : {}", goal.objective);
    println!(
        "todos     : {}",
        goal.todos
            .iter()
            .map(|t| format!("{}={}", t.id, status_label(t)))
            .collect::<Vec<_>>()
            .join("  ")
    );
    println!(
        "gaps      : {}",
        if goal.acceptance.is_empty() {
            "(none)".to_string()
        } else {
            goal.acceptance
                .iter()
                .map(|g| {
                    format!(
                        "{}{}",
                        g.id,
                        if g.satisfied { "=satisfied" } else { "=OPEN" }
                    )
                })
                .collect::<Vec<_>>()
                .join("  ")
        }
    );
    let next = goal.next_action.as_deref().unwrap_or("-");
    println!("next      : {next}");
    for line in crate::cli_projection::monitor_metadata_lines(goal) {
        println!("{line}");
    }
    if let Some(gap) = crate::store::projection_gap(goal) {
        println!("⚠ projection gap: {gap}");
    }
    let s = goal.todo_summary();
    println!(
        "summary   : user open={} done={} | agent open={} done={} | monitor={} | closure_proof={}",
        s.user_open,
        s.user_done,
        s.agent_open,
        s.agent_done,
        s.monitor_open,
        if s.terminal_closure_proof.all_todos_done {
            "valid"
        } else {
            "pending"
        }
    );
    println!(
        "terminal  : {} (validated closure, not open_count)",
        goal.terminal_closure().is_some()
    );
    let spent: u64 = goal.history.len() as u64;
    println!("spent     : {spent} turns");
}

fn status_label(t: &Todo) -> &'static str {
    if t.status == TodoStatus::Done {
        if t.no_follow_up {
            "done(no-follow-up)"
        } else if !t.successor_ids.is_empty() {
            "done(+successor)"
        } else {
            "done"
        }
    } else if t.status == TodoStatus::Superseded {
        "superseded"
    } else {
        match t.class {
            TaskClass::Advancement => "open",
            TaskClass::UserGate => "GATE",
            TaskClass::UserAction => "action",
            TaskClass::Monitor => "monitor",
            TaskClass::Blocker => "blocker",
        }
    }
}

// ── quota ─────────────────────────────────────────────────────────────────

fn cmd_quota(store: &Store, args: &[String]) -> Result<()> {
    match args.first().map(|s| s.as_str()) {
        Some("should-run") => quota_should_run(store, &args[1..]),
        Some("usage") => quota_usage(store, &args[1..]),
        Some("spend") => quota_spend(store, &args[1..]),
        _ => bail!("quota subcommand must be `should-run`, `usage`, or `spend`"),
    }
}

/// `loopx quota should-run --goal G [--format json] [--agent-id A]` — emit
/// the typed ShouldRunPacket. Text mode renders the CLI projection (G-9):
/// decision banner + quota breakdown by spend source + scheduler hint +
/// stall hint + arbitration. JSON mode emits the full typed packet.
fn quota_should_run(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut format_json = false;
    let mut agent_id = None;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--format" => format_json = v == "json",
        "--agent-id" => agent_id = Some(v),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let packet = decide_for(&goal, SystemTime::now(), agent_id.as_deref());
    if format_json {
        println!("{}", serde_json::to_string_pretty(&packet)?);
        return Ok(());
    }
    let breakdown = crate::quota::usage_summary::breakdown(&goal.history);
    let stall = crate::quota::stall_repair::detect_stall(&goal);
    let usage =
        crate::quota::usage_summary::build_usage_summary(&goal_id, &goal.history, now_epoch());
    print!(
        "{}",
        crate::cli_projection::render_quota_projection(&packet, Some(&breakdown), stall.as_ref())
    );
    print!("{}", crate::cli_projection::render_usage_summary(&usage));
    Ok(())
}

/// `loopx quota usage --goal G [--format json]` — 24h/7d usage summary
/// (P1 acceptance: spend by source is queryable + the quota command renders
/// the usage summary).
fn quota_usage(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut format_json = false;
    let mut all = false;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--format" => format_json = v == "json",
        "--all" => all = true,
        _ => {}
    });
    let now = now_epoch();
    if let Some(g) = goal_id {
        let goal = store
            .replay(&g)?
            .ok_or_else(|| anyhow::anyhow!("goal {g} not found"))?;
        let summary = crate::quota::usage_summary::build_usage_summary(&g, &goal.history, now);
        if format_json {
            println!("{}", serde_json::to_string_pretty(&summary)?);
        } else {
            print!("{}", crate::cli_projection::render_usage_summary(&summary));
        }
        return Ok(());
    }
    if !all {
        bail!("quota usage requires --goal G or --all");
    }
    // Aggregate across every registered goal (reference run_history projection).
    let rows: Vec<(&str, Vec<crate::state::RunRecord>)> = store
        .registry()
        .iter()
        .filter_map(|e| {
            store
                .replay(&e.goal_id)
                .ok()
                .flatten()
                .map(|g| (e.goal_id.as_str(), g.history))
        })
        .collect();
    let refs: Vec<(&str, &[crate::state::RunRecord])> = rows
        .iter()
        .map(|(id, history)| (*id, history.as_slice()))
        .collect();
    let summary = crate::quota::usage_summary::build_usage_summary_for_goals(&refs, now);
    if format_json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        print!("{}", crate::cli_projection::render_usage_summary(&summary));
    }
    Ok(())
}

/// `loopx quota spend --goal G` — per-source slot spend breakdown.
fn quota_spend(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v)
        }
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let b = crate::quota::usage_summary::breakdown(&goal.history);
    println!(
        "spend: allowed={} spent={} (run={} agent={} heartbeat={})",
        b.allowed_slots,
        b.spent_slots,
        b.source_count(crate::quota::slot_accounting::SlotSpendSource::Run),
        b.source_count(crate::quota::slot_accounting::SlotSpendSource::Agent),
        b.source_count(crate::quota::slot_accounting::SlotSpendSource::Heartbeat),
    );
    Ok(())
}

// ── scheduler (G-10) ───────────────────────────────────────────────────────

/// `loopx scheduler <tick|show|record-host-failure> --goal G [--agent-id A]`
/// — drive the persisted scheduler state machine across decision cycles.
fn cmd_scheduler(store: &Store, args: &[String]) -> Result<()> {
    match args.first().map(|s| s.as_str()) {
        Some("tick") => scheduler_tick(store, &args[1..]),
        Some("show") => scheduler_show(store, &args[1..]),
        Some("record-host-failure") => scheduler_record_failure(store, &args[1..]),
        _ => bail!("scheduler subcommand must be `tick`, `show`, or `record-host-failure`"),
    }
}

fn scheduler_scope(
    store: &Store,
    args: &[String],
    default_agent: &str,
) -> Result<(String, String)> {
    let mut goal_id = None;
    let mut agent_id = None;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--agent-id" => agent_id = Some(v),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let agent = agent_id.unwrap_or_else(|| default_agent.to_string());
    Ok((goal_id, agent))
}

/// `loopx scheduler tick --goal G [--agent-id A] [--cadence-class hourly]
/// [--progression 15,30,60] [--action tick_next]` — load the persisted state
/// (or bootstrap it from the cadence profile), advance the progression, and
/// write the new state. Restart-safe: progression persists across cycles.
fn scheduler_tick(store: &Store, args: &[String]) -> Result<()> {
    let mut cadence_class = "monitor_backoff".to_string();
    let mut progression: Vec<i64> = vec![];
    let mut action = "tick_next".to_string();
    parse_pairs(args, |k, v| match k {
        "--cadence-class" => cadence_class = v,
        "--progression" => {
            progression = v
                .split(',')
                .filter_map(|s| s.trim().parse::<i64>().ok())
                .filter(|m| *m > 0)
                .collect()
        }
        "--action" => action = v,
        _ => {}
    });
    let (goal_id, agent) = scheduler_scope(store, args, "codex-app")?;
    let goal_dir = store.goal_dir(&goal_id);
    use crate::scheduler::state as st;
    let state_key = st::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY;
    let surface = st::CODEX_APP_SURFACE;
    let now = now_epoch();

    // Bootstrap: first tick for this (goal, agent) builds the initial state
    // from the cadence profile; later ticks advance the persisted cursor.
    let state = st::load_scheduler_state(&goal_dir, &agent, surface, state_key);
    if state.is_none() {
        let identity = st::identity_signature(&goal_id, &agent, surface);
        let intervals = if progression.is_empty() {
            st::MONITOR_WAIT_PROGRESSION_MINUTES.to_vec()
        } else {
            progression
        };
        let initial_minutes = intervals.first().copied().unwrap_or(15);
        let initial_rrule = st::rrule_for_minutes(initial_minutes);
        let state = st::build_scheduler_state(
            &goal_id,
            &agent,
            surface,
            state_key,
            &st::reset_token(&action, &identity, &initial_rrule),
            &identity,
            0,
            intervals,
            &initial_rrule,
            now,
            vec![],
        )?;
        st::write_scheduler_state(&goal_dir, &state)?;
        print!("{}", crate::cli_projection::render_scheduler_state(&state));
        println!(
            "→ bootstrapped (initial rrule {}); next tick advances progression",
            initial_rrule
        );
        return Ok(());
    }

    let mut state = state.unwrap();
    let rrule = st::apply_next_progression(&mut state, now);
    st::write_scheduler_state(&goal_dir, &state)?;
    print!("{}", crate::cli_projection::render_scheduler_state(&state));
    match rrule {
        Some(r) => println!("→ advanced progression to {r} (persisted)"),
        None => println!("→ no progression (single-execution cadence)"),
    }
    Ok(())
}

/// `loopx scheduler show --goal G [--agent-id A]` — print the persisted
/// scheduler state (or "no state yet").
fn scheduler_show(store: &Store, args: &[String]) -> Result<()> {
    let (goal_id, agent) = scheduler_scope(store, args, "codex-app")?;
    use crate::scheduler::state as st;
    let state = st::load_scheduler_state(
        &store.goal_dir(&goal_id),
        &agent,
        st::CODEX_APP_SURFACE,
        st::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY,
    );
    match state {
        Some(s) => print!("{}", crate::cli_projection::render_scheduler_state(&s)),
        None => println!(
            "no scheduler state for goal {goal_id} agent {agent} (run `scheduler tick` first)"
        ),
    }
    Ok(())
}

/// `loopx scheduler record-host-failure --goal G --agent-id A
/// --target-rrule "FREQ=MINUTELY;INTERVAL=15" --observed-rrule "..."
/// --failure-kind host_stale_rrule` — merge a host-update failure into the
/// retained cache (bounded, TTL'd; reference scheduler state).
fn scheduler_record_failure(store: &Store, args: &[String]) -> Result<()> {
    let mut target_rrule = None;
    let mut observed_rrule = None;
    let mut failure_kind = None;
    let mut count = 1u32;
    parse_pairs(args, |k, v| match k {
        "--target-rrule" => target_rrule = Some(v),
        "--observed-rrule" => observed_rrule = Some(v),
        "--failure-kind" => failure_kind = Some(v),
        "--failure-count" => count = v.parse().unwrap_or(1),
        _ => {}
    });
    let (goal_id, agent) = scheduler_scope(store, args, "codex-app")?;
    let target_rrule = target_rrule.ok_or_else(|| anyhow::anyhow!("--target-rrule required"))?;
    let observed_rrule = observed_rrule.unwrap_or_default();
    let failure_kind = failure_kind.ok_or_else(|| anyhow::anyhow!("--failure-kind required"))?;
    use crate::scheduler::state as st;
    let goal_dir = store.goal_dir(&goal_id);
    let state_key = st::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY;
    let surface = st::CODEX_APP_SURFACE;
    let now = now_epoch();
    let existing = st::load_scheduler_state(&goal_dir, &agent, surface, state_key);
    let mut failures = existing
        .as_ref()
        .map(|s| s.host_update_failures.clone())
        .unwrap_or_default();
    let failure = st::HostUpdateFailure {
        schema_version: st::SCHEDULER_HOST_UPDATE_FAILURE_SCHEMA_VERSION.to_string(),
        target_rrule: st::normalize_scheduler_rrule(&target_rrule),
        observed_host_rrule: st::normalize_scheduler_rrule(&observed_rrule),
        failure_kind,
        failed_at: crate::compat::rfc3339(now),
        failure_count: count,
    };
    failures = st::merge_host_update_failure(&failures, failure, now);
    let state = match existing {
        Some(mut s) => {
            s.host_update_failures = failures;
            s.updated_at = now;
            s
        }
        None => {
            // No state yet: bootstrap from the observed rrule so the failure
            // has a home (reference tolerates a state-less failure record by
            // keeping the failure list on the next build).
            let identity = st::identity_signature(&goal_id, &agent, surface);
            st::build_scheduler_state(
                &goal_id,
                &agent,
                surface,
                state_key,
                &st::reset_token("tick_next", &identity, &target_rrule),
                &identity,
                0,
                vec![15],
                &st::normalize_scheduler_rrule(&target_rrule),
                now,
                failures,
            )?
        }
    };
    st::write_scheduler_state(&goal_dir, &state)?;
    println!(
        "host update failure recorded (kind={} target={} observed={}) → {} retained",
        state
            .host_update_failures
            .last()
            .map(|f| f.failure_kind.clone())
            .unwrap_or_default(),
        state
            .host_update_failures
            .last()
            .map(|f| f.target_rrule.clone())
            .unwrap_or_default(),
        state
            .host_update_failures
            .last()
            .map(|f| f.observed_host_rrule.clone())
            .unwrap_or_default(),
        state.host_update_failures.len()
    );
    Ok(())
}

// ── run (one bounded gRPC turn + writeback) ────────────────────────────────

/// `future-loop models [--format json]` — list models available from the
/// agent (auth.json / models.json merged with the built-in catalog).
async fn cmd_models(args: &[String]) -> Result<()> {
    let json = args.iter().any(|a| a == "--format" || a == "--json");
    let mut client = crate::agent_client::AgentClient::connect(&crate::agent_client::agent_addr()).await?;
    let data = client.list_models().await?;
    let models = data["models"].as_array().cloned().unwrap_or_default();
    let default_model = data["defaultModel"].as_str().unwrap_or("");
    if json {
        println!("{}", serde_json::to_string_pretty(&data)?);
        return Ok(());
    }
    println!("Available models (default: {}):", default_model);
    for m in &models {
        let id = m["id"].as_str().unwrap_or("");
        let provider = m["provider"].as_str().unwrap_or("");
        let label = m["label"].as_str().unwrap_or(id);
        let full = if provider.is_empty() {
            id.to_string()
        } else {
            format!("{provider}/{id}")
        };
        let thinking = m["thinkingLevel"].as_str().unwrap_or("off");
        let ctx = m["contextWindow"].as_i64().unwrap_or(0);
        let recommended = m["recommended"].as_bool().unwrap_or(false);
        let is_default = m["isDefault"].as_bool().unwrap_or(false);
        let mut flags = String::new();
        if is_default {
            flags.push_str(" [default]");
        }
        if recommended {
            flags.push_str(" [recommended]");
        }
        println!("- {full}  {label}  thinking={thinking} ctx={ctx}{flags}");
    }
    Ok(())
}

/// Default task-lease length for a `run` turn (4h — long LLM/compute turns
/// routinely exceed the old 1h lease, which would let another worker steal
/// the todo mid-turn once the lease expired).
const DEFAULT_RUN_LEASE_SECS: u64 = 4 * 3600;

/// Resolve run identity (G-27): `run` REQUIRES `--agent-id` so the lease
/// mechanism actually engages — an anonymous run claims nothing and hides
/// nothing, so two agentless runs deterministically race on the same todo.
/// An id that is not yet registered is auto-registered on first use (replay
/// is idempotent, so `run` never needs a separate `agent register` step).
/// `--anonymous` opts back into the legacy uncoordinated one-shot path.
/// Returns the resolved agent id (None for `--anonymous`).
pub fn ensure_run_identity(
    store: &mut Store,
    goal_id: &str,
    agent_id: Option<&str>,
    anonymous: bool,
) -> Result<Option<String>> {
    match (agent_id, anonymous) {
        (Some(aid), _) => {
            let goal = store
                .replay(goal_id)?
                .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
            if !goal.is_registered_agent(Some(aid)) {
                store.append(Event::AgentRegistered {
                    goal_id: goal_id.to_string(),
                    agent_id: aid.to_string(),
                    ts: now_epoch(),
                })?;
                println!("agent `{aid}` auto-registered for {goal_id} ✔");
            }
            Ok(Some(aid.to_string()))
        }
        (None, true) => {
            println!(
                "⚠ anonymous run: no lease coordination — parallel runs may race on the same todo"
            );
            Ok(None)
        }
        (None, false) => bail!(
            "run requires --agent-id <name> (or --anonymous for an uncoordinated one-shot run); \
             check existing ids with `{} agent list --goal {goal_id}`",
            prog()
        ),
    }
}

async fn cmd_run(store: &mut Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut model = None;
    let mut thinking = None;
    let mut max_turns = 6u32;
    let mut max_turn_secs = 0u64;
    let mut agent_id = None;
    let mut anonymous = false;
    let mut lease_secs = DEFAULT_RUN_LEASE_SECS;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--model" => model = Some(v),
        "--thinking-level" => thinking = Some(v),
        "--max-turns" => max_turns = v.parse().unwrap_or(6),
        "--max-turn-secs" => max_turn_secs = v.parse().unwrap_or(0),
        "--agent-id" => agent_id = Some(v),
        "--lease-secs" => lease_secs = v.parse().unwrap_or(DEFAULT_RUN_LEASE_SECS),
        "--anonymous" => anonymous = true,
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;

    // Identity gate BEFORE any gRPC/session work — fail fast with a hint
    // (and stays unit-testable without an agent server).
    let agent_id = ensure_run_identity(store, &goal_id, agent_id.as_deref(), anonymous)?;

    let mut client = crate::agent_client::AgentClient::connect(&crate::agent_client::agent_addr()).await?;
    let goal0 = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let session_id = client.new_session(&goal0.cwd).await?;
    if let Some(m) = model {
        client.set_model(&session_id, &m).await?;
    }
    if let Some(l) = thinking {
        client.set_thinking_level(&session_id, &l).await?;
    }

    // Run the turn loop, then always delete this run's scratch session — on
    // every exit path — instead of letting ~/.future/agent/sessions/ pile up
    // one file per run. The agent session is a per-run workspace: context is
    // replayed via the turn envelope from the goal events.jsonl, so nothing
    // durable lives in it.
    let result = run_turns(
        &mut client,
        store,
        &goal_id,
        &session_id,
        max_turns,
        lease_secs,
        agent_id.as_deref(),
        max_turn_secs,
    )
    .await;
    if let Err(e) = client.delete_session(&session_id).await {
        println!("   ⚠ session cleanup failed (best-effort): {e}");
    }
    result
}

/// One steer poll step: read newly appended ledger lines since `offset` and
/// inject any `todo_updated` text for `todo_id` into the session. Returns
/// the new offset. Extracted from the watch loop for testability.
#[doc(hidden)] // test-visible seam for steer_todo_updates
pub async fn steer_poll_once(
    events_path: &std::path::Path,
    offset: u64,
    todo_id: &str,
    client: &mut Option<crate::agent_client::AgentClient>,
    session_id: &str,
) -> u64 {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(meta) = std::fs::metadata(events_path) else {
        return offset;
    };
    if meta.len() <= offset {
        return offset;
    }
    let mut buf = String::new();
    let mut new_offset = offset;
    {
        let Ok(mut f) = std::fs::File::open(events_path) else {
            return offset;
        };
        if f.seek(SeekFrom::Start(offset)).is_err() {
            return offset;
        }
        if f.read_to_string(&mut buf).is_err() {
            return offset;
        }
        new_offset = meta.len();
    }
    for line in buf.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("kind").and_then(|k| k.as_str()) != Some("todo_updated") {
            continue;
        }
        if v.get("todo_id").and_then(|t| t.as_str()) != Some(todo_id) {
            continue;
        }
        let Some(new_text) = v.get("text").and_then(|t| t.as_str()) else {
            continue;
        };
        if client.is_none() {
            *client = crate::agent_client::AgentClient::connect(&crate::agent_client::agent_addr())
                .await
                .ok();
        }
        if let Some(c) = client.as_mut() {
            let msg = format!(
                "ORCHESTRATOR STEERING (todo {todo_id} updated mid-turn — new instructions below; adjust your current work accordingly):\n{new_text}"
            );
            if c.steer(session_id, &msg).await.is_err() {
                *client = None; // reconnect on the next event
            }
        }
    }
    new_offset
}

/// Mid-turn steering watcher: tail the goal ledger; when the orchestrator
/// updates the CURRENT todo's text mid-turn (`todo update --text`), inject the
/// new instructions into the running session via the `steer` RPC (drained by
/// the agent at its next step boundary). Runs as a background task for the
/// duration of one turn; never completes on its own (all error paths retry).
async fn steer_todo_updates(events_path: std::path::PathBuf, todo_id: String, session_id: String) {
    let mut offset = std::fs::metadata(&events_path)
        .map(|m| m.len())
        .unwrap_or(0);
    let mut client: Option<crate::agent_client::AgentClient> = None;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        offset = steer_poll_once(&events_path, offset, &todo_id, &mut client, &session_id).await;
    }
}

/// One `run` = one bounded turn loop against a fresh agent session. `cmd_run`
/// owns the session lifecycle (create before, delete after); this function
/// only executes turns and writes back their ledger effects.
#[allow(clippy::too_many_arguments)]
async fn run_turns(
    client: &mut crate::agent_client::AgentClient,
    store: &mut Store,
    goal_id: &str,
    session_id: &str,
    max_turns: u32,
    lease_secs: u64,
    agent_id: Option<&str>,
    max_turn_secs: u64,
) -> Result<()> {
    let mut turn = 0u32;
    loop {
        turn += 1;
        if turn > max_turns {
            bail!("max-turns ({max_turns}) reached without validated closure");
        }
        let goal = store
            .replay(goal_id)?
            .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found (deleted while running?)"))?;
        let packet = decide_for(&goal, SystemTime::now(), agent_id);
        println!(
            "── turn {turn}: decision={} mode={} | {}",
            packet.decision,
            packet.interaction_contract.mode.as_str(),
            packet.reason
        );
        let mode = packet.interaction_contract.mode;
        if mode == crate::contract::TurnMode::Terminal {
            println!("✔ validated closure — loop stops");
            break;
        }
        if mode == crate::contract::TurnMode::Replan {
            if let Some(gap) = crate::store::projection_gap(&goal) {
                println!("↻ self-repair: {gap}");
                store.set_next_action(goal_id, "all todos complete; no further action")?;
                continue;
            }
            println!("↻ replan required — no auto path; stopping (see status)");
            break;
        }
        if mode == crate::contract::TurnMode::AskUser {
            let q = packet
                .interaction_contract
                .user_channel
                .question
                .clone()
                .unwrap_or_default();
            println!("⟳ USER GATE: {q}");
            break;
        }
        if mode == crate::contract::TurnMode::WaitMonitor {
            println!("   waiting… (monitor not due)");
            break;
        }

        // bounded_delivery / monitor_poll: execute one turn.
        // Claim with a lease BEFORE executing — atomically (check+append under
        // one lock) so two concurrent `run --agent-id` workers can never both
        // win the same todo; on contention, re-decide against the fresh
        // ledger and pick the next runnable todo (up to 3 re-decides).
        let mut packet = packet;
        let mut todo_id_opt = None;
        for _ in 0..3 {
            let Some(tid) = packet
                .interaction_contract
                .agent_channel
                .selected_todo
                .clone()
            else {
                break;
            };
            match &agent_id {
                Some(aid) => {
                    if store.try_claim_todo(goal_id, &tid, aid, lease_secs)? {
                        todo_id_opt = Some(tid);
                        break;
                    }
                    println!("   ⚔ claim race lost on {tid} — re-deciding");
                    let fresh = store.replay(goal_id)?.ok_or_else(|| {
                        anyhow::anyhow!("goal {goal_id} not found (deleted while running?)")
                    })?;
                    packet = decide_for(&fresh, SystemTime::now(), agent_id);
                    if packet.interaction_contract.mode
                        != crate::contract::TurnMode::BoundedDelivery
                        && packet.interaction_contract.mode
                            != crate::contract::TurnMode::MonitorPoll
                    {
                        todo_id_opt = None;
                        break;
                    }
                }
                None => {
                    todo_id_opt = Some(tid);
                    break;
                }
            }
        }
        let Some(todo_id) = todo_id_opt else {
            println!("   no selected todo; stopping");
            break;
        };
        let goal = store
            .replay(goal_id)?
            .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found (deleted while running?)"))?;
        let todo = goal.todo(&todo_id).unwrap().clone();
        let boundary = store
            .replay(goal_id)?
            .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found (deleted while running?)"))?;
        let runs_dir = std::path::PathBuf::from(store.root_path()).join("runs");
        let _ = std::fs::create_dir_all(&runs_dir);
        // Mid-turn steering: watch the ledger for orchestrator `todo update`s
        // targeting THIS todo and inject them into the running session (the
        // turn envelope is only composed at turn start, so without this the
        // agent never sees updates until the next turn). Aborted after the turn.
        let steer_handle = tokio::spawn(steer_todo_updates(
            store.goal_dir(goal_id).join("events.jsonl"),
            todo_id.clone(),
            session_id.to_string(),
        ));
        let turn_future = execute_turn(
            client,
            session_id,
            &boundary,
            &todo,
            turn,
            goal.history.last(),
            true,
            // G-9: embed the decision summary (mode/reason/arbitration) in
            // the turn envelope.
            Some(&packet),
            Some(runs_dir),
        );
        let record = if max_turn_secs > 0 {
            // Wall-clock budget per turn: a long turn that never sees new
            // instructions is an observability hole; bound it so orchestrators
            // can relaunch on a safe cadence (context replays from the ledger).
            match tokio::time::timeout(std::time::Duration::from_secs(max_turn_secs), turn_future)
                .await
            {
                Ok(r) => r?,
                Err(_) => {
                    steer_handle.abort();
                    println!(
                        "   ⏱ turn exceeded --max-turn-secs ({max_turn_secs}s) — stopping run gracefully; relaunch to continue"
                    );
                    return Ok(());
                }
            }
        } else {
            turn_future.await?
        };
        steer_handle.abort();
        println!(
            "   run={} state={} tools=[{}] cost=¥{:.4}",
            record.run_id,
            record.terminal_state,
            record.tools.join(", "),
            record.cost_delta
        );
        println!(
            "   live log: .future/loop/runs/{}.live.jsonl",
            record.run_id
        );
        if let Some(v) = &record.validation {
            println!(
                "   validation: status={} ok={} ({}), exit={}",
                match v.status {
                    crate::state::ValidationStatus::Passed => "passed",
                    crate::state::ValidationStatus::Progress => "progress",
                    crate::state::ValidationStatus::Failed => "failed",
                    crate::state::ValidationStatus::Inconclusive => "inconclusive",
                    crate::state::ValidationStatus::Unavailable => "unavailable",
                    crate::state::ValidationStatus::NotRequired => "not_required",
                },
                v.ok,
                v.summary,
                v.exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "-".to_string())
            );
        }
        // Writeback: complete with closure intent — remaining open todos
        // become successors; the LAST todo declares no-follow-up (LoopX
        // completion contract, verified against the real control plane).
        let successors: Vec<String> = goal
            .runnable_advancement()
            .filter(|t| t.id != todo_id)
            .map(|t| t.id.clone())
            .collect();
        let is_last = successors.is_empty();
        // Validation-gated completion: `todo add --verify` keeps the todo open
        // until the independent validator exits 0 (bounded retry below).
        let succeeded = crate::executor::turn_succeeded(&record);
        let completion = if succeeded {
            Some((is_last, successors.clone()))
        } else {
            None
        };
        let mut g = store
            .replay(goal_id)?
            .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found (deleted while running?)"))?;
        let monitor_changed = if mode == crate::contract::TurnMode::MonitorPoll {
            Some(record.evidence.to_uppercase().contains("EXISTS"))
        } else {
            None
        };
        // G-7: stamp the spend source on the ledger entry before writeback
        // so quota accounting classifies it (run/agent/heartbeat).
        let mut record = record;
        record.spend_source = Some(
            crate::quota::slot_accounting::classify_mode(mode)
                .as_str()
                .to_string(),
        );
        writeback(&mut g, &record, monitor_changed, completion);
        store.append_run(goal_id, &record)?;
        // Project-local per-run mirror (runs/ under the goal state dir).
        let _ = crate::compat::write_run(&store.goal_dir(goal_id), goal_id, &record);
        store.append(Event::RunRecorded {
            goal_id: goal_id.to_string(),
            record: record.clone(),
            ts: now_epoch(),
        })?;
        // G-3: quota spend lands as a durable event alongside the run ledger
        // (source mirrors slot accounting; monitor no-change never spends).
        if monitor_changed != Some(false) {
            store.append(Event::QuotaSpent {
                goal_id: goal_id.to_string(),
                run_id: record.run_id.clone(),
                todo_id: todo_id.clone(),
                source: record
                    .spend_source
                    .clone()
                    .unwrap_or_else(|| "run".to_string()),
                slots: 1,
                ts: now_epoch(),
            })?;
        }
        // G-8: monitor poll results land as durable events (decision-path
        // writeback): changed closes the monitor, no_change advances the
        // counter — replayed exactly via store::apply.
        if mode == crate::contract::TurnMode::MonitorPoll {
            let (result, no_change_count) = crate::decision::monitor_poll_classification(
                monitor_changed.unwrap_or(false),
                goal.todo(&todo_id)
                    .map(|t| t.consecutive_no_change)
                    .unwrap_or(0),
            );
            store.append(Event::MonitorPolled {
                goal_id: goal_id.to_string(),
                todo_id: todo_id.clone(),
                result: result.to_string(),
                no_change_count,
                ts: now_epoch(),
            })?;
            println!("   poll result: {result} (no_change_count={no_change_count})");
        }
        if succeeded && mode != crate::contract::TurnMode::MonitorPoll {
            store.append(Event::TodoCompleted {
                goal_id: goal_id.to_string(),
                todo_id: todo_id.clone(),
                no_follow_up: is_last,
                successor_ids: successors.clone(),
                evidence: Some(record.evidence.clone()),
                ts: now_epoch(),
            })?;
        } else if let Some(t) = g.todo(&todo_id) {
            if t.failed_attempts > MAX_REPAIR_ATTEMPTS {
                println!("   ✘ repair budget exhausted — stopping");
                break;
            }
            // Validation-gated repair: a todo with an attached validator stays
            // open until exit 0, bounded by its own max_validation_attempts.
            if t.validator.is_some() && t.failed_attempts >= t.max_validation_attempts {
                println!(
                    "   ✘ validation budget exhausted ({}/{}) — replan required; stopping",
                    t.failed_attempts, t.max_validation_attempts
                );
                break;
            }
        }
        // Sync Next Action to the frontier (avoid projection gap).
        let next_text = g
            .runnable_advancement()
            .next()
            .map(|t| t.text.clone())
            .unwrap_or_else(|| "all todos complete; no further action".to_string());
        store.set_next_action(goal_id, &next_text)?;
        println!("   ✔ writeback ok — next action synced");
    }
    Ok(())
}

// ── store (G-3 / G-6) ─────────────────────────────────────────────────────

/// `loopx store <migrate|verify|bridge> --goal G` — event-store schema
/// migration, ledger id/conflict integrity, and the fail-closed migration
/// bridge status (G-6).
fn cmd_store(store: &mut Store, args: &[String]) -> Result<()> {
    match args.first().map(|s| s.as_str()) {
        Some("migrate") => {
            let goal_id = goal_arg(args)?;
            let report = crate::migration::apply_migrations(&store.goal_dir(&goal_id), &goal_id)?;
            println!(
                "migrated {goal_id}: {} → {} ({} lines, backup {})",
                report.from, report.to, report.migrated_lines, report.backup_path
            );
            println!("rollback: {}", report.rollback_plan);
            Ok(())
        }
        Some("verify") => {
            let goal_id = goal_arg(args)?;
            let report = store.verify(&goal_id)?;
            println!(
                "ledger {goal_id}: schema={} events={} unique={} idempotent_dups={} legacy_without_id={} conflicts={:?} → {}",
                report.schema_version,
                report.total_events,
                report.unique_events,
                report.idempotent_duplicates,
                report.legacy_lines_without_id,
                report.conflicts,
                if report.ok { "ok" } else { "CONFLICT" }
            );
            Ok(())
        }
        Some("bridge") => {
            let goal_id = goal_arg(args)?;
            let goal_dir = store.goal_dir(&goal_id);
            let bridge = crate::migration::migration_bridge_status(store, &goal_id, &goal_dir);
            println!(
                "migration bridge {goal_id}: stage={} promotion_allowed={}",
                bridge.stage, bridge.promotion_allowed
            );
            println!("  next_action: {}", bridge.next_action);
            println!(
                "  checks: event_read_path={} active_state_projection={} dual_read_parity={} rollback={} canary={} idempotency={} public_boundary={} head_matches={}",
                bridge.checks.event_read_path_ready,
                bridge.checks.active_state_projection_ready,
                bridge.checks.dual_read_parity_clean,
                bridge.checks.rollback_plan_recorded,
                bridge.checks.bounded_canary_passed,
                bridge.checks.idempotency_conflicts_clean,
                bridge.checks.public_boundary_clean,
                bridge.checks.event_projection_head_matches_store,
            );
            println!(
                "  missing: shadow={:?} canary={:?} promotion={:?}",
                bridge.missing_for_shadow, bridge.missing_for_canary, bridge.missing_for_promotion
            );
            Ok(())
        }
        _ => bail!("store subcommand must be `migrate`, `verify`, or `bridge`"),
    }
}

fn goal_arg(args: &[String]) -> Result<String> {
    let mut goal_id = None;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v)
        }
    });
    goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))
}

// ── backfill (G-3) ────────────────────────────────────────────────────────

/// `loopx backfill --goal G [--from PATH] [--privacy public_safe|local_private|private_pointer]
/// [--dry-run]` — reconstruct idempotent events from an ACTIVE_GOAL_STATE.md
/// workbench (read-only import with source_ref/section/line provenance).
fn cmd_backfill(store: &mut Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut from = None;
    let mut privacy = "local_private".to_string();
    let mut dry_run = false;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--from" => from = Some(v),
        "--privacy" => privacy = v,
        "--dry-run" => dry_run = true,
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let level: crate::projection::privacy::PrivacyLevel =
        privacy.parse().map_err(|e: String| anyhow::anyhow!(e))?;
    let markdown = match from {
        Some(path) => {
            std::fs::read_to_string(&path).map_err(|e| anyhow::anyhow!("read {path}: {e}"))?
        }
        None => crate::backfill::active_state_markdown(&goal.cwd, &goal_id)?,
    };
    let outcome = crate::backfill::backfill_todo_events(&markdown, &goal_id, level)?;
    if dry_run {
        println!(
            "backfill dry-run: {goal_id} → {} todos, {} events (not appended)",
            outcome.todo_count, outcome.event_count
        );
        for event in &outcome.events {
            println!(
                "  {} [{}] {}:{} — {}",
                event.event_id,
                event.privacy.as_str(),
                event.source_section,
                event.source_line,
                match &event.event {
                    Event::TodoAdded { todo, .. } => format!("add {}", todo.id),
                    Event::TodoClaimed { todo_id, .. } => format!("claim {todo_id}"),
                    Event::TodoCompleted { todo_id, .. } => format!("complete {todo_id}"),
                    _ => "?".to_string(),
                }
            );
        }
        return Ok(());
    }
    let mut appended = 0usize;
    for event in &outcome.events {
        store.append_with_meta(
            event.event.clone(),
            Some(event.event_id.clone()),
            Some(crate::backfill::MARKDOWN_BACKFILL_PRODUCER.to_string()),
            Some(event.source_ref.clone()),
            Some(event.source_section.clone()),
            Some(event.source_line),
            Some(event.privacy.as_str().to_string()),
        )?;
        appended += 1;
    }
    let _ = sync_compat(store, &goal_id);
    println!(
        "backfill {goal_id}: {} todos → {} events appended (producer={}, privacy={}) ✔",
        outcome.todo_count,
        appended,
        crate::backfill::MARKDOWN_BACKFILL_PRODUCER,
        level.as_str()
    );
    Ok(())
}

// ── privacy (G-4) ─────────────────────────────────────────────────────────

/// `loopx privacy --goal G [--level LEVEL] [--json]` — grade the goal's
/// todos by privacy tier, render the multi-projection (public-safe markdown
/// + status cache), and persist the status cache.
fn cmd_privacy(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut level = "public_safe".to_string();
    let mut format_json = false;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--level" => level = v,
        "--format" => format_json = v == "json",
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let privacy: crate::projection::privacy::PrivacyLevel =
        level.parse().map_err(|e: String| anyhow::anyhow!(e))?;
    let goal_dir = store.goal_dir(&goal_id);
    let projections = crate::projection::build_projections(&goal, privacy, &goal_dir);
    // Persist the status cache projection (multi-projection write path).
    if let Some(cache) = &projections.status_cache {
        crate::projection::status_cache::write_status_cache(&goal_dir, cache)?;
    }
    if format_json {
        println!("{}", serde_json::to_string_pretty(&projections)?);
        return Ok(());
    }
    let report = &projections.privacy_report;
    println!(
        "privacy {goal_id}: overall={} items={} (public={} private={} pointer={})",
        report.overall.as_str(),
        report.item_count,
        report.public_safe_count,
        report.local_private_count,
        report.private_pointer_count
    );
    for item in &report.items {
        println!(
            "  {} → {} {}",
            item.todo_id,
            item.level.as_str(),
            if item.private_fields.is_empty() {
                String::new()
            } else {
                format!("(private fields: {})", item.private_fields.join(","))
            }
        );
    }
    println!(
        "--- public-safe projection (level={}) ---",
        privacy.as_str()
    );
    println!("{}", projections.public_markdown);
    let digest = crate::projection::status_cache::ledger_digest(&goal_dir);
    let cache = crate::projection::status_cache::read_status_cache(&goal_dir);
    if let Some(cache) = cache {
        println!(
            "--- status cache: ledger_digest={} stale={} ---",
            cache.ledger_digest,
            crate::projection::status_cache::status_cache_stale(&cache, &digest)
        );
    }
    Ok(())
}

// ── lease (G-13) ──────────────────────────────────────────────────────────

/// `loopx lease <claim|renew|release|expire|status> --goal G --todo-id T
/// [--agent-id A] [--lease-secs N]` — the task-lease state machine over the
/// event base (TodoClaimed + TodoRenewed/TodoReleased/TodoExpired).
fn cmd_lease(store: &mut Store, args: &[String]) -> Result<()> {
    let sub = args.first().map(|s| s.as_str()).ok_or_else(|| {
        anyhow::anyhow!("lease requires a subcommand (claim|renew|release|expire|status)")
    })?;
    let mut goal_id = None;
    let mut todo_id = None;
    let mut agent_id = None;
    let mut lease_secs = 0u64;
    parse_pairs(&args[1..], |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--todo-id" => todo_id = Some(v),
        "--agent-id" => agent_id = Some(v),
        "--lease-secs" => lease_secs = v.parse().unwrap_or(0),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let todo_id = todo_id.ok_or_else(|| anyhow::anyhow!("--todo-id required"))?;
    let mut goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let agent = agent_id.unwrap_or_else(|| "default-agent".to_string());
    use crate::work_items::task_lease as lease;
    let now = crate::state::now_epoch();

    if sub == "status" {
        let todo = goal
            .todo(&todo_id)
            .ok_or_else(|| anyhow::anyhow!("todo {todo_id} not found"))?;
        match lease::lease_status(todo, now) {
            lease::LeaseStatus::Free => println!("todo {todo_id}: lease FREE"),
            lease::LeaseStatus::Active { owner, expires_at } => {
                println!("todo {todo_id}: lease ACTIVE (owner={owner} expires_at={expires_at})")
            }
            lease::LeaseStatus::Expired { owner, expires_at } => {
                println!("todo {todo_id}: lease EXPIRED (owner={owner} expired_at={expires_at})")
            }
        }
        return Ok(());
    }

    let todo = goal
        .todo_mut(&todo_id)
        .ok_or_else(|| anyhow::anyhow!("todo {todo_id} not found"))?;
    match sub {
        "claim" => {
            let op = lease::claim(todo, &agent, lease_secs, now)?;
            let expires = todo.lease_expires_at.unwrap_or(now);
            match op {
                lease::LeaseOp::Acquired { idempotent, steal } => {
                    if !idempotent {
                        if steal {
                            store.append(Event::TodoExpired {
                                goal_id: goal_id.clone(),
                                todo_id: todo_id.clone(),
                                ts: now,
                            })?;
                        }
                        store.append(Event::TodoClaimed {
                            goal_id: goal_id.clone(),
                            todo_id: todo_id.clone(),
                            agent_id: agent.clone(),
                            lease_expires_at: expires,
                            ts: now,
                        })?;
                    }
                    let _ = sync_compat(store, &goal_id);
                    println!(
                        "todo {todo_id} lease acquired by {agent} until {expires} {}✔",
                        if steal { "(steal after expiry) " } else { "" }
                    );
                }
                _ => unreachable!(),
            }
        }
        "renew" => {
            let _ = lease::renew(todo, &agent, lease_secs, now)?;
            let expires = todo.lease_expires_at.unwrap_or(now);
            store.append(Event::TodoRenewed {
                goal_id: goal_id.clone(),
                todo_id: todo_id.clone(),
                agent_id: agent.clone(),
                lease_expires_at: expires,
                ts: now,
            })?;
            let _ = sync_compat(store, &goal_id);
            println!("todo {todo_id} lease renewed by {agent} until {expires} ✔");
        }
        "release" => {
            let op = lease::release(todo, &agent, now)?;
            if !matches!(op, lease::LeaseOp::Released { missing: true }) {
                store.append(Event::TodoReleased {
                    goal_id: goal_id.clone(),
                    todo_id: todo_id.clone(),
                    agent_id: agent.clone(),
                    ts: now,
                })?;
            }
            let _ = sync_compat(store, &goal_id);
            println!("todo {todo_id} lease released by {agent} ✔");
        }
        "expire" => {
            let op = lease::expire(todo, now)?;
            if matches!(op, lease::LeaseOp::Expired { had_lease: true }) {
                store.append(Event::TodoExpired {
                    goal_id: goal_id.clone(),
                    todo_id: todo_id.clone(),
                    ts: now,
                })?;
            }
            let _ = sync_compat(store, &goal_id);
            println!("todo {todo_id} lease expiry recorded ✔");
        }
        _ => bail!("lease subcommand must be claim|renew|release|expire|status"),
    }
    Ok(())
}

// ── runs (G-5) ────────────────────────────────────────────────────────────

/// `loopx runs <history|compact|index|retention|stale> --goal G [--keep N]
/// [--cutoff TS] [--rebuild]` — run-history projection + lifecycle
/// (compaction archives, never deletes; index dedup/rebuild; retention
/// policy; stale-latest-run warning).
fn cmd_runs(store: &Store, args: &[String]) -> Result<()> {
    let sub = args.first().map(|s| s.as_str()).ok_or_else(|| {
        anyhow::anyhow!("runs requires a subcommand (history|compact|index|retention|stale)")
    })?;
    let mut goal_id = None;
    let mut keep = 50usize;
    let mut cutoff = None;
    let mut rebuild = false;
    let mut format_json = false;
    parse_pairs(&args[1..], |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--keep" => keep = v.parse().unwrap_or(50),
        "--cutoff" => cutoff = Some(v),
        "--rebuild" => rebuild = true,
        "--format" => format_json = v == "json",
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let root = store.root_path();
    use crate::runtime as rt;

    match sub {
        "history" => {
            let projection = rt::run_history::build_run_history(&root, &goal_id, now_epoch())?;
            match projection {
                Some(p) => {
                    if format_json {
                        println!("{}", serde_json::to_string_pretty(&p)?);
                    } else {
                        println!(
                            "run history {goal_id}: {} runs (24h={} 7d={}) latest={:?}",
                            p.sample_run_count,
                            p.totals.events_24h,
                            p.totals.events_7d,
                            p.latest.as_ref().map(|r| r.classification.as_str())
                        );
                        println!("  by_class_24h: {:?}", p.totals.by_class_24h);
                        println!("  by_class_7d:  {:?}", p.totals.by_class_7d);
                    }
                }
                None => println!("no run history for {goal_id} (runtime {root})"),
            }
        }
        "compact" => {
            let report = match cutoff {
                Some(c) => rt::run_compaction::archive_runs_before(
                    &root,
                    &goal_id,
                    c.parse()
                        .map_err(|_| anyhow::anyhow!("--cutoff must be epoch secs"))?,
                )?,
                None => rt::run_compaction::archive_keeping_latest(&root, &goal_id, keep)?,
            };
            println!(
                "compaction {goal_id}: archived {} runs (recoverable, never deleted), kept {} → {}",
                report.archived.len(),
                report.kept,
                report.archive_dir
            );
        }
        "index" => {
            if rebuild {
                let report = rt::run_index::rebuild_index(&root, &goal_id)?;
                println!(
                    "index rebuilt {goal_id}: {} rows (backup {}) non_destructive={}",
                    report.rows_written, report.backup_path, report.non_destructive
                );
                return Ok(());
            }
            let index = rt::index_path(&root, &goal_id);
            let report = rt::run_index::detect_duplicates(&index)?;
            println!(
                "index {goal_id}: {} rows, {} duplicate group(s), repairable={}",
                report.total_rows,
                report.duplicate_groups.len(),
                report.repairable
            );
            for group in &report.duplicate_groups {
                println!(
                    "  {} lines {:?} → {} ({})",
                    group.duplicate_kind, group.line_numbers, group.action, group.severity
                );
            }
        }
        "retention" => {
            let policy = rt::run_context_retention::retention_policy(keep, None);
            let report =
                rt::run_context_retention::retention_report(&root, &goal_id, &policy, now_epoch())?;
            println!(
                "retention {goal_id}: keep_latest={} total={} retained={} candidates={}",
                policy.keep_latest,
                report.total,
                report.retained,
                report.candidates.len()
            );
            for candidate in &report.candidates {
                println!("  candidate: {candidate}");
            }
        }
        "stale" => {
            let goal = store
                .replay(&goal_id)?
                .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
            let goal_dir = store.goal_dir(&goal_id);
            match rt::stale_latest_run::stale_latest_run(&goal, &goal_dir) {
                Some(warning) => {
                    println!(
                        "⚠ {} (severity={}): {} (state@{} vs latest run@{})",
                        warning.kind,
                        warning.severity,
                        warning.reason,
                        warning.active_state_updated_at.unwrap_or(0),
                        warning.latest_run_recorded_at.unwrap_or(0)
                    );
                    println!("  → {}", warning.recommended_action);
                }
                None => println!("no stale-latest-run warning for {goal_id}"),
            }
        }
        _ => bail!("runs subcommand must be history|compact|index|retention|stale"),
    }
    Ok(())
}

// ── helpers ────────────────────────────────────────────────────────────────

fn parse_pairs(args: &[String], mut f: impl FnMut(&str, String)) {
    let mut i = 0;
    while i < args.len() {
        let k = args[i].as_str();
        if k.starts_with("--") {
            // boolean flags (no value) are followed by another flag or end.
            if matches!(k, "--no-follow-up" | "--anonymous" | "--help" | "-h") {
                f(k, "true".to_string());
                i += 1;
            } else if i + 1 < args.len() && !args[i + 1].starts_with("--") {
                f(k, args[i + 1].clone());
                i += 2;
            } else {
                f(k, "true".to_string());
                i += 1;
            }
        } else {
            i += 1;
        }
    }
}

/// `loopx heartbeat-prompt --goal G [--agent-id A]` — render the per-turn
/// re-entry packet for a host executor (reference heartbeat contract).
fn cmd_heartbeat(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut agent_id = None;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--agent-id" => agent_id = Some(v),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let packet = decide_for(&goal, SystemTime::now(), agent_id.as_deref());
    print!(
        "{}",
        crate::heartbeat::render_heartbeat_prompt(&goal, &packet)
    );
    Ok(())
}

/// `loopx worker-bridge --goal G [--agent-id A] [--max-turns N]` — run the
/// stdio worker contract (packet out, result in, writeback).
async fn cmd_worker_bridge(store: &mut Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut agent_id = None;
    let mut max_turns = 6u32;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--agent-id" => agent_id = Some(v),
        "--max-turns" => max_turns = v.parse().unwrap_or(6),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    crate::worker_bridge::run_bridge(
        store,
        &crate::worker_bridge::BridgeOptions {
            goal_id,
            agent_id,
            max_turns,
        },
    )
    .await
}

/// `loopx serve-status [--port 8791]` — zero-dependency HTTP dashboard
/// (GET / , GET /goals.json). Read-only projection; ledger stays the truth.
fn cmd_serve_status(store: &Store, args: &[String]) -> Result<()> {
    let mut port = 8791u16;
    parse_pairs(args, |k, v| {
        if k == "--port" {
            port = v.parse().unwrap_or(8791)
        }
    });
    crate::status_server::serve(store, &format!("127.0.0.1:{port}"))
}

/// `loopx capability list` / `loopx capability propose --name issue_fix --input "..."`.
fn cmd_capability(store: &Store, args: &[String]) -> Result<()> {
    let registry = crate::capabilities::CapabilityRegistry::with_builtin();
    if args.first().map(|s| s.as_str()) == Some("list") {
        println!("capabilities:");
        for cap in registry.all() {
            let n = cap.name();
            let d = cap.describe();
            println!("  {n} — {d}");
        }
        return Ok(());
    }
    // G-24: per-capability command hooks, gated by catalog status/stage
    // (experimental capability commands hidden unless --include-experimental).
    if args.first().map(|s| s.as_str()) == Some("commands") {
        let catalog = crate::capabilities::catalog::CapabilityCatalog::with_builtin();
        let include_experimental = args.iter().any(|a| a == "--include-experimental");
        let mut name = None;
        parse_pairs(&args[1..], |k, v| {
            if k == "--name" {
                name = Some(v)
            }
        });
        match name {
            Some(n) => {
                let cmds = catalog.commands_for(&n, include_experimental);
                if cmds.is_empty() {
                    println!(
                        "no registered commands for `{n}`{}",
                        if catalog
                            .get(&n)
                            .map(|r| r.is_experimental())
                            .unwrap_or(false)
                        {
                            " (experimental — pass --include-experimental)"
                        } else {
                            ""
                        }
                    );
                } else {
                    println!("capability `{n}` commands:");
                    for c in cmds {
                        println!("  {:<24} {}", c.name, c.purpose);
                    }
                }
            }
            None => {
                println!("per-capability command hooks:");
                for record in catalog.records(true) {
                    let mark = if record.is_experimental() {
                        " (experimental)"
                    } else {
                        ""
                    };
                    let cmds = catalog.commands_for(&record.id, include_experimental);
                    for c in cmds {
                        println!("  {:<24} [{}{}] {}", c.name, record.id, mark, c.purpose);
                    }
                }
            }
        }
        return Ok(());
    }
    if args.first().map(|s| s.as_str()) != Some("propose") {
        bail!("capability subcommand must be `list`, `commands`, or `propose`");
    }
    let mut name = None;
    let mut input = None;
    parse_pairs(&args[1..], |k, v| match k {
        "--name" => name = Some(v),
        "--input" => input = Some(v),
        _ => {}
    });
    let name = name.ok_or_else(|| anyhow::anyhow!("--name required"))?;
    let input = input.unwrap_or_default();
    let Some(cap) = registry.get(&name) else {
        bail!("unknown capability `{name}` (see `future-loop capability list`)");
    };
    let proposals = cap.propose(&input);
    let n = proposals.len();
    println!("capability `{name}` → {n} proposal(s):");
    for p in proposals {
        let kind = match p.kind {
            crate::capabilities::ProposalKind::SuccessorTodo => "successor_todo",
            crate::capabilities::ProposalKind::NoFollowUp => "no_followup",
            crate::capabilities::ProposalKind::Repair => "repair",
            crate::capabilities::ProposalKind::Gate => "gate",
            crate::capabilities::ProposalKind::Monitor => "monitor",
        };
        let r = &p.reason;
        println!("  [{kind}] {r}");
        if let Some(t) = p.todo {
            let tt = &t.text;
            println!("    → todo: {tt}");
        }
        if let Some(q) = p.gate_question {
            println!("    → gate: {q}");
        }
    }
    let _ = store;
    Ok(())
}

/// Join todo ids for CLI display (empty → "(none)").
fn join_ids(ids: &[String]) -> String {
    if ids.is_empty() {
        "(none)".to_string()
    } else {
        ids.join(", ")
    }
}

// ── extension (G-21) ───────────────────────────────────────────────────────

/// Extension runtime state file (next to goal state under the loop root).
fn extension_state_file() -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}/extensions/state.json", root_dir()))
}

/// `loopx extension <install|upgrade|enable|disable|rollback|status|capabilities>` —
/// the declarative extension lifecycle (v1: no native code is executed).
fn cmd_extension(store: &Store, args: &[String]) -> Result<()> {
    let state_file = extension_state_file();
    let sub = args.first().map(|s| s.as_str()).unwrap_or("");
    match sub {
        "install" | "upgrade" => {
            let mut manifest_path = None;
            let mut execute = false;
            parse_pairs(&args[1..], |k, v| match k {
                "--manifest" => manifest_path = Some(v),
                "--execute" => execute = true,
                _ => {}
            });
            let manifest_path =
                manifest_path.ok_or_else(|| anyhow::anyhow!("--manifest required"))?;
            let manifest = crate::extensions::manifest::load_extension_manifest(
                std::path::Path::new(&manifest_path),
            )
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            let op = crate::extensions::runtime::install_extension(
                &manifest,
                &state_file,
                sub,
                execute,
            )
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            println!(
                "extension {} `{}` → revision {} (dry_run={} changed={})",
                op.operation,
                op.extension_id,
                op.revision.unwrap_or_default(),
                op.dry_run,
                op.changed
            );
        }
        "enable" | "disable" | "rollback" => {
            let mut id = None;
            let mut execute = false;
            parse_pairs(&args[1..], |k, v| match k {
                "--id" => id = Some(v),
                "--execute" => execute = true,
                _ => {}
            });
            let id = id.ok_or_else(|| anyhow::anyhow!("--id required"))?;
            let op = match sub {
                "enable" => crate::extensions::runtime::enable_extension(&id, &state_file, execute),
                "disable" => crate::extensions::runtime::disable_extension(&id, &state_file, execute),
                _ => crate::extensions::runtime::rollback_extension(&id, &state_file, execute),
            }
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            println!(
                "extension {} `{}` → revision {} (dry_run={} changed={})",
                op.operation,
                op.extension_id,
                op.revision.unwrap_or_default(),
                op.dry_run,
                op.changed
            );
        }
        "status" => {
            let mut id = None;
            parse_pairs(&args[1..], |k, v| {
                if k == "--id" {
                    id = Some(v)
                }
            });
            let rows = crate::extensions::runtime::extension_status(&state_file, id.as_deref())
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            if rows.is_empty() {
                println!("no extensions installed");
            }
            for r in rows {
                println!(
                    "{:<20} enabled={} rev={} rollback={} doctor_verified={} revisions={}",
                    r.id, r.enabled, r.active_revision, r.rollback_available, r.doctor_verified, r.revision_count
                );
            }
        }
        "capabilities" => {
            let entries = crate::extensions::runtime::extension_catalog_entries(&state_file)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            if entries.is_empty() {
                println!("no extensions installed");
            }
            for e in entries {
                println!(
                    "{} v{} ready={} provides=[{}] implements=[{}]",
                    e.id,
                    e.version,
                    e.lifecycle.ready,
                    e.provides.join(","),
                    e.implements.join(",")
                );
            }
        }
        _ => bail!(
            "extension subcommand must be install|upgrade|enable|disable|rollback|status|capabilities"
        ),
    }
    let _ = store;
    Ok(())
}

// ── catalog (G-23) ─────────────────────────────────────────────────────────

/// `loopx catalog [--name X] [--json]` — query the capability catalog
/// (status / stage / provider / commands / packets). P3 acceptance: the
/// catalog is queryable.
fn cmd_catalog(store: &Store, args: &[String]) -> Result<()> {
    let catalog = crate::capabilities::catalog::CapabilityCatalog::with_builtin();
    let mut name = None;
    let mut json = false;
    parse_pairs(args, |k, v| match k {
        "--name" => name = Some(v),
        "--format" => json = v == "json",
        "--json" => json = true,
        _ => {}
    });
    match name {
        Some(n) => {
            let record = catalog
                .get(&n)
                .ok_or_else(|| anyhow::anyhow!("unknown capability `{n}`"))?;
            if json {
                println!("{}", serde_json::to_string_pretty(record)?);
                return Ok(());
            }
            let lifecycle = catalog
                .provider_lifecycle_for(&n)
                .map(|lc| lc.stage().label().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            println!("capability `{}`", record.id);
            println!("  title    : {}", record.title);
            println!("  status   : {} (stage {})", record.status, record.stage);
            println!(
                "  provider : {} [{}] → {lifecycle}",
                record.provider_id, record.origin
            );
            println!("  commands :");
            for c in &record.commands {
                println!("    {:<24} {}", c.name, c.purpose);
            }
            println!("  packets  :");
            for p in &record.packets {
                println!("    {} @ {}", p.schema_version, p.module);
            }
            println!("  user_value: {}", record.user_value);
            println!("  next_real_step: {}", record.next_real_step);
        }
        None => {
            println!(
                "capability catalog ({} records):",
                catalog.records(false).len()
            );
            for record in catalog.records(false) {
                println!(
                    "  {:<24} {:<22} stage={} provider={}",
                    record.id, record.status, record.stage, record.provider_id
                );
            }
        }
    }
    let _ = store;
    Ok(())
}

// ── scope / lane / supervisor (G-16) ───────────────────────────────────────

/// `loopx scope --goal G --agent-id A [--exclude X]` — the identity-scoped
/// frontier. P3 acceptance: two agents under one goal each hold a frontier
/// that never crosses into the other's claimed slices.
fn cmd_scope(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut agent_id = None;
    let mut exclude: Vec<String> = vec![];
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--agent-id" => agent_id = Some(v),
        "--exclude" => exclude = v.split(',').map(|s| s.to_string()).collect(),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let agent_id = agent_id.ok_or_else(|| anyhow::anyhow!("--agent-id required"))?;
    let goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let frontier = crate::agents::scope::identity_scoped_frontier(&goal, &agent_id, &exclude);
    println!(
        "agent scope `{}` (task_scope={}):",
        frontier.agent_id, frontier.task_scope
    );
    println!(
        "  visible agent todos : {}",
        join_ids(&frontier.visible_agent_todo_ids)
    );
    println!(
        "  claimed by self     : {}",
        join_ids(&frontier.claimed_todo_ids)
    );
    println!(
        "  other agent claims  : {}  ← outside this frontier",
        join_ids(&frontier.other_agent_claimed_ids)
    );
    println!(
        "  open user gates     : {}",
        join_ids(&frontier.open_user_gate_ids)
    );
    println!(
        "  unclaimed advancement: {}",
        frontier.unclaimed_advancement_count
    );
    Ok(())
}

/// `loopx lane --goal G --agent-id A` — compact agent lane recommendation.
fn cmd_lane(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut agent_id = None;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--agent-id" => agent_id = Some(v),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let agent_id = agent_id.ok_or_else(|| anyhow::anyhow!("--agent-id required"))?;
    let goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    match crate::agents::lane::compact_agent_lane_recommendation(&goal, &agent_id) {
        Some(rec) => {
            println!(
                "agent lane `{}` ({}): classification={} generated_at={}",
                rec.agent_id, rec.progress_scope, rec.classification, rec.generated_at
            );
            if let Some(action) = &rec.recommended_action {
                println!("  recommended action: {action}");
            }
        }
        None => println!("no lane run for agent `{agent_id}` yet"),
    }
    Ok(())
}

/// `loopx supervisor <propose|receipt|events> --goal G ...` — supervisor
/// proposal/receipt events (projection-only).
fn cmd_supervisor(store: &mut Store, args: &[String]) -> Result<()> {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("");
    match sub {
        "propose" => {
            let mut goal_id = None;
            let mut supervisor_id = None;
            let mut decision_id = None;
            let mut target_agent_id = None;
            let mut kind = "observe".to_string();
            let mut capabilities: Vec<String> = vec![];
            let mut summary = None;
            parse_pairs(&args[1..], |k, v| match k {
                "--goal" => goal_id = Some(v),
                "--agent-id" => supervisor_id = Some(v),
                "--decision-id" => decision_id = Some(v),
                "--target-agent-id" => target_agent_id = Some(v),
                "--kind" => kind = v,
                "--capabilities" => capabilities = v.split(',').map(|s| s.to_string()).collect(),
                "--summary" => summary = Some(v),
                _ => {}
            });
            let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
            let supervisor_id =
                supervisor_id.ok_or_else(|| anyhow::anyhow!("--agent-id (supervisor) required"))?;
            let decision_id =
                decision_id.ok_or_else(|| anyhow::anyhow!("--decision-id required"))?;
            let target_agent_id =
                target_agent_id.ok_or_else(|| anyhow::anyhow!("--target-agent-id required"))?;
            let summary = summary.unwrap_or_default();
            let decision = if kind == "execute" {
                crate::agents::supervisor::SupervisorDecision::execute(
                    &decision_id,
                    &target_agent_id,
                    capabilities,
                    &summary,
                )
            } else {
                crate::agents::supervisor::SupervisorDecision::observe(
                    &decision_id,
                    &target_agent_id,
                    &summary,
                )
            };
            let event_id = crate::agents::supervisor::record_supervisor_proposal(
                store,
                &goal_id,
                &supervisor_id,
                &decision,
            )?;
            println!("supervisor proposal recorded (event {event_id})");
        }
        "receipt" => {
            let mut goal_id = None;
            let mut decision_id = None;
            let mut receipt_id = None;
            let mut adapter_id = None;
            let mut outcome = "rejected".to_string();
            let mut authority_ref = None;
            let mut host_capabilities: Vec<String> = vec![];
            parse_pairs(&args[1..], |k, v| match k {
                "--goal" => goal_id = Some(v),
                "--decision-id" => decision_id = Some(v),
                "--receipt-id" => receipt_id = Some(v),
                "--adapter-id" => adapter_id = Some(v),
                "--outcome" => outcome = v,
                "--authority-ref" => authority_ref = Some(v),
                "--host-capabilities" => {
                    host_capabilities = v.split(',').map(|s| s.to_string()).collect()
                }
                _ => {}
            });
            let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
            let decision_id =
                decision_id.ok_or_else(|| anyhow::anyhow!("--decision-id required"))?;
            let receipt_id = receipt_id.ok_or_else(|| anyhow::anyhow!("--receipt-id required"))?;
            let adapter_id = adapter_id.ok_or_else(|| anyhow::anyhow!("--adapter-id required"))?;
            let outcome_enum = match outcome.as_str() {
                "executed" => crate::agents::supervisor::SupervisorReceiptOutcome::Executed,
                "failed" => crate::agents::supervisor::SupervisorReceiptOutcome::Failed,
                _ => crate::agents::supervisor::SupervisorReceiptOutcome::Rejected,
            };
            let receipt = crate::agents::supervisor::SupervisorReceipt {
                receipt_id,
                decision_id,
                adapter_id,
                outcome: outcome_enum,
                authority_ref,
                rollback_ref: None,
                evidence_refs: vec![],
                reason_codes: vec![],
            };
            let event_id = crate::agents::supervisor::record_supervisor_receipt(
                store,
                &goal_id,
                &receipt,
                &host_capabilities,
            )?;
            println!("supervisor receipt recorded (event {event_id})");
        }
        "events" => {
            let mut goal_id = None;
            parse_pairs(&args[1..], |k, v| {
                if k == "--goal" {
                    goal_id = Some(v)
                }
            });
            let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
            let projection =
                crate::agents::supervisor::build_supervisor_event_projection(store, &goal_id)?;
            println!("{}", serde_json::to_string_pretty(&projection)?);
        }
        _ => bail!("supervisor subcommand must be propose|receipt|events"),
    }
    Ok(())
}

// ── handoff (G-17) ─────────────────────────────────────────────────────────

/// `loopx handoff --goal G [--write]` — generate the handoff document
/// (projection-derived) + the delivery contract from run-history signals.
fn cmd_handoff(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut write = false;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--write" => write = true,
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    // Delivery contract consumes newest-first runs.
    let mut runs: Vec<RunRecord> = goal.history.clone();
    runs.reverse();
    let contract = crate::handoff::delivery_contract::handoff_delivery_contract(&goal, &runs);
    if let Some(c) = &contract {
        println!("delivery contract: {}", c.mode);
        println!("  summary: {}", c.summary);
    } else {
        println!("delivery contract: none (no degradation)");
    }
    let handoff = crate::handoff::project_handoff::build_project_handoff(
        &goal,
        contract.as_ref().map(|c| c.summary.as_str()),
    );
    if write {
        crate::handoff::project_handoff::write_project_handoff(
            &store.goal_dir(&goal_id),
            &goal,
            &handoff,
        )
        .map_err(|e| anyhow::anyhow!("write handoff: {e}"))?;
        println!(
            "handoff written to .future/loop/goals/{}/HANDOFF.md",
            goal.goal_id
        );
    } else {
        println!(
            "{}",
            crate::handoff::project_handoff::render_project_handoff_markdown(&handoff)
        );
    }
    Ok(())
}

// ── task-graph (G-14) ──────────────────────────────────────────────────────

/// `loopx task-graph --goal G` — the todo dependency graph with topological
/// order; cycles fail closed.
fn cmd_task_graph(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v)
        }
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let graph = crate::work_items::task_graph::build_task_graph(&goal)
        .map_err(|e| anyhow::anyhow!("task graph failed closed: {e}"))?;
    println!(
        "task graph: {} nodes, {} edges",
        graph.nodes.len(),
        graph.edges.len()
    );
    for e in &graph.edges {
        println!("  {} → {}", e.from, e.to);
    }
    if let Some(cycle) = &graph.cycle {
        println!("⚠ cycle: {}", cycle.join(" → "));
    } else if let Some(order) = &graph.topological_order {
        println!("topological order: {}", order.join(" → "));
    }
    Ok(())
}

// ── attention / inbox (G-15) ───────────────────────────────────────────────

/// `loopx attention [--goal G] [--all]` — project the attention queue.
fn cmd_attention(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut all = false;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--all" => all = true,
        _ => {}
    });
    let mut items = vec![];
    if let Some(g) = goal_id {
        if let Some(goal) = store.replay(&g)? {
            if let Some(item) = crate::work_items::attention::goal_attention_item(&goal) {
                items.push(item);
            }
        }
    } else if all {
        for entry in store.registry() {
            if let Ok(Some(goal)) = store.replay(&entry.goal_id) {
                if let Some(item) = crate::work_items::attention::goal_attention_item(&goal) {
                    items.push(item);
                }
            }
        }
    } else {
        bail!("attention requires --goal G or --all");
    }
    let queue = crate::work_items::attention::build_attention_queue(items);
    println!(
        "attention queue: {} item(s) | user/controller={} controller={} codex={} monitor={}",
        queue.item_count,
        queue.needs_user_or_controller,
        queue.needs_controller,
        queue.needs_codex,
        queue.watching_monitor
    );
    for item in &queue.items {
        println!(
            "  [{}] {} waiting_on={} severity={}",
            item.goal_id, item.status, item.waiting_on, item.severity
        );
        println!("      → {}", item.recommended_action);
    }
    Ok(())
}

/// `loopx inbox --project DIR [--scope S] [--name NAME]` — project the
/// operator inbox urgency from `.loopx/inbox/*.json` events (content never
/// returned).
fn cmd_inbox(store: &Store, args: &[String]) -> Result<()> {
    let mut project = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| ".".to_string());
    let mut scope = "addressed_only".to_string();
    let mut name = "operator".to_string();
    parse_pairs(args, |k, v| match k {
        "--project" => project = v,
        "--scope" => scope = v,
        "--name" => name = v,
        _ => {}
    });
    let config = crate::work_items::operator_inbox::OperatorInboxConfig {
        enabled: true,
        capture_scope: scope,
        inbox_dir: "inbox".to_string(),
        operator_display_name: name,
        reply_enabled: true,
    };
    let pending = crate::work_items::operator_inbox::load_pending_inbox_events(&project, "inbox")
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let urgency =
        crate::work_items::operator_inbox::project_operator_inbox_urgency(&config, &pending);
    println!(
        "operator inbox: enabled={} pending={} question={} mention={} reply={} attention_required={} reply_due={}",
        urgency.enabled,
        urgency.pending_count,
        urgency.direct_question_count,
        urgency.direct_mention_count,
        urgency.reply_to_operator_count,
        urgency.attention_required_count,
        urgency.reply_due
    );
    let _ = store;
    Ok(())
}

// ── registry (G-26) ────────────────────────────────────────────────────────

/// `loopx registry [--json] [--include-experimental]` — inspect the CLI
/// registry (groups + commands) — the aggregated help surface.
fn cmd_registry(registry: &CommandRegistry, args: &[String]) -> Result<()> {
    let include_experimental = args.iter().any(|a| a == "--include-experimental");
    if args.iter().any(|a| a == "--json") {
        let payload: serde_json::Value = registry
            .groups()
            .iter()
            .enumerate()
            .map(|(idx, g)| {
                let cmds: Vec<serde_json::Value> = registry
                    .commands_in(idx, include_experimental)
                    .iter()
                    .map(|c| {
                        serde_json::json!({
                            "name": c.name,
                            "summary": c.summary,
                            "usage": c.usage,
                            "experimental": c.experimental,
                        })
                    })
                    .collect();
                serde_json::json!({ "group": g.name, "summary": g.summary, "commands": cmds })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }
    println!(
        "CLI registry: {} groups, {} commands{}",
        registry.group_count(),
        registry.command_count(include_experimental),
        if include_experimental {
            " (incl. experimental)"
        } else {
            ""
        }
    );
    for (idx, group) in registry.groups().iter().enumerate() {
        let cmds = registry.commands_in(idx, include_experimental);
        if cmds.is_empty() {
            continue;
        }
        println!("── {} ──", group.name);
        for c in cmds {
            println!("  {:<24} {}", c.name, c.summary);
        }
    }
    Ok(())
}

// ── P4: benchmark (G-18) ──────────────────────────────────────────────────

/// `loopx benchmark protocol --route R [--json]` — the loop protocol
/// contract for a route (blind / product-mode / packet-only classification).
fn cmd_benchmark_protocol(store: &Store, args: &[String]) -> Result<()> {
    let mut route = None;
    let mut max_rounds = None;
    let mut json = false;
    parse_pairs(args, |k, v| match k {
        "--route" => route = Some(v),
        "--max-rounds" => max_rounds = v.parse::<u32>().ok(),
        "--json" => json = true,
        _ => {}
    });
    let route = route.ok_or_else(|| anyhow::anyhow!("--route required"))?;
    let contract =
        crate::benchmark::loop_protocol::build_benchmark_loop_contract(&route, max_rounds, None);
    if json {
        println!("{}", serde_json::to_string_pretty(&contract)?);
        return Ok(());
    }
    println!("route        : {}", contract.route);
    println!("protocol_id  : {}", contract.protocol_id);
    println!("max_rounds   : {}", contract.max_rounds_budget);
    println!(
        "classification: {}",
        if contract.product_mode {
            "product_mode"
        } else if contract.blind_loop {
            "blind_loop"
        } else {
            "custom_or_legacy"
        }
    );
    println!(
        "feedback     : blinded={} forwarded={}",
        contract.official_feedback_blinded, contract.official_feedback_forwarded
    );
    println!(
        "strict_claim : {}",
        if contract.strict_treatment_claim_allowed {
            "allowed".to_string()
        } else {
            format!("blocked ({})", contract.claim_blocker)
        }
    );
    let _ = store;
    Ok(())
}

/// `loopx benchmark ledger [--benchmark-id X] [--case-id Y] [--json]
/// [--dir DIR]` — query the benchmark run ledger (default dir:
/// `<cwd>/.future/loop/benchmarks`).
fn cmd_benchmark_ledger(store: &Store, args: &[String]) -> Result<()> {
    let mut benchmark_id = None;
    let mut case_id = None;
    let mut json = false;
    let mut dir = None;
    parse_pairs(args, |k, v| match k {
        "--benchmark-id" => benchmark_id = Some(v),
        "--case-id" => case_id = Some(v),
        "--json" => json = true,
        "--dir" => dir = Some(v),
        _ => {}
    });
    let dir = dir.unwrap_or_else(|| format!("{}/benchmarks", store.root_path()));
    let dir = std::path::PathBuf::from(&dir);
    std::fs::create_dir_all(&dir).ok();
    let ledger = crate::benchmark::ledger::BenchmarkLedger::open(&dir)?;
    if json {
        let entries: Vec<serde_json::Value> = ledger
            .query(benchmark_id.as_deref(), case_id.as_deref(), None)
            .iter()
            .map(|e| serde_json::to_value(e).unwrap_or(serde_json::Value::Null))
            .collect();
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }
    let matched = ledger.query(benchmark_id.as_deref(), case_id.as_deref(), None);
    println!(
        "benchmark ledger {} ({} run(s)):",
        dir.display(),
        matched.len()
    );
    for e in &matched {
        println!(
            "  {} {} {} case={} score={} passed={} class={}",
            e.run_id,
            e.benchmark_id,
            e.arm_id,
            e.case_ids.join(","),
            e.score,
            e.passed,
            e.failure_class
        );
    }
    let agg = ledger.aggregate(benchmark_id.as_deref());
    println!(
        "aggregate: {} run(s), {} passed, avg best score {:.3}",
        agg["run_count"], agg["passed"], agg["avg_best_score"]
    );
    let _ = store;
    Ok(())
}

/// `loopx benchmark run --benchmark-id X --case-id Y --task "..." ...` —
/// the minimal closed loop: preflight → rounds → ledger. Uses the gRPC
/// adapter when `--agent-addr` is given, else a deterministic scripted
/// adapter (dry-run).
async fn cmd_benchmark_run(store: &Store, args: &[String]) -> Result<()> {
    use crate::benchmark::adapter::BenchmarkAdapter;
    let mut benchmark_id = None;
    let mut case_id = None;
    let mut task = None;
    let mut route = None;
    let mut arm_id = None;
    let mut max_rounds = 5u32;
    let mut expected_evidence = None;
    let mut agent_addr = None;
    let mut ledger_dir = None;
    let mut stub = false;
    parse_pairs(args, |k, v| match k {
        "--benchmark-id" => benchmark_id = Some(v),
        "--case-id" => case_id = Some(v),
        "--task" => task = Some(v),
        "--route" => route = Some(v),
        "--arm-id" => arm_id = Some(v),
        "--max-rounds" => max_rounds = v.parse::<u32>().unwrap_or(5),
        "--expected-evidence" => expected_evidence = Some(v),
        "--agent-addr" => agent_addr = Some(v),
        "--ledger-dir" => ledger_dir = Some(v),
        "--stub" => stub = true,
        _ => {}
    });
    let benchmark_id = benchmark_id.ok_or_else(|| anyhow::anyhow!("--benchmark-id required"))?;
    let case_id = case_id.ok_or_else(|| anyhow::anyhow!("--case-id required"))?;
    let task = task.ok_or_else(|| anyhow::anyhow!("--task required"))?;
    let route = route.unwrap_or_else(|| "future-loop-product-mode".to_string());
    let mut case = crate::benchmark::qualification::QualificationCase::new(
        &benchmark_id,
        &case_id,
        &task,
        max_rounds,
    );
    case.route = route.clone();
    if let Some(arm) = arm_id {
        case.arm_id = arm;
    } else {
        case.arm_id = if route == "future-loop-goal-start-product-mode" {
            "future_loop_goal_start_product_mode".to_string()
        } else {
            "future_loop_product_mode".to_string()
        };
    }
    case.expected_evidence = expected_evidence;

    let ledger_dir = ledger_dir.map(std::path::PathBuf::from);
    let mut adapter: Box<dyn BenchmarkAdapter> = match (&agent_addr, stub) {
        (Some(addr), _) => {
            let model = std::env::var("FUTURE_LOOP_MODEL")
                .unwrap_or_else(|_| "future/deepseek-v4-flash".to_string());
            Box::new(
                crate::benchmark::adapter::GrpcLoopxAdapter::connect(addr, "/tmp")
                    .await?
                    .with_model(&model),
            )
        }
        (None, _) => {
            println!("(no --agent-addr: deterministic scripted dry-run)");
            Box::new(crate::benchmark::adapter::ScriptedAdapter::new(vec![
                "completed".to_string(),
            ]))
        }
    };
    let result = crate::benchmark::qualification::run_qualification_case(
        &mut *adapter,
        &case,
        ledger_dir.as_deref(),
    )?;
    println!(
        "benchmark run {} {} (route={}, arm={}): passed={} rounds={}/{}",
        result.benchmark_id,
        result.case_id,
        result.route,
        result.arm_id,
        result.passed,
        result.rounds_used,
        result.max_rounds
    );
    println!(
        "headline: best={} final={} first_success={:?} declared_done={}",
        result.headline.best_score,
        result.headline.final_score,
        result.headline.first_success_round,
        result.headline.declared_done_score
    );
    println!(
        "failure : class={} scope={}",
        result.failure_class, result.failure_scope
    );
    for record in &result.round_reward_trace.records {
        println!(
            "  round {}: passed={} reward={}",
            record.agent_round, record.passed, record.reward
        );
    }
    let _ = store;
    Ok(())
}

/// `loopx benchmark protocol|run|ledger ...`
async fn cmd_benchmark(store: &Store, args: &[String]) -> Result<()> {
    match args.first().map(|s| s.as_str()) {
        Some("protocol") => cmd_benchmark_protocol(store, &args[1..]),
        Some("ledger") => cmd_benchmark_ledger(store, &args[1..]),
        Some("run") => cmd_benchmark_run(store, &args[1..]).await,
        Some(other) => {
            anyhow::bail!("unknown benchmark subcommand `{other}` (protocol|run|ledger)")
        }
        None => anyhow::bail!("benchmark requires a subcommand (protocol|run|ledger)"),
    }
}

// ── P4: replay & corpus (G-19) ─────────────────────────────────────────────

/// `loopx replay record --goal G [--case-id X] [--agent-id A] [--out PATH]` —
/// reduce the current kernel decision to a public-safe case; with `--out`,
/// append it to a replay file.
fn cmd_replay_record(store: &Store, args: &[String]) -> Result<()> {
    use crate::replay::decision_replay::{reduce_public_safe_decision, DecisionReplay};
    let mut goal_id = None;
    let mut case_id = None;
    let mut agent_id = None;
    let mut out = None;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--case-id" => case_id = Some(v),
        "--agent-id" => agent_id = Some(v),
        "--out" => out = Some(v),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let packet =
        crate::decision::decide_for(&goal, std::time::SystemTime::now(), agent_id.as_deref());
    let case_id = case_id.unwrap_or_else(|| format!("{goal_id}-{}", crate::state::now_epoch()));
    let case = reduce_public_safe_decision(&packet, &goal, &case_id, agent_id.as_deref());
    crate::replay::decision_replay::validate_public_safe_decision_case(&case)?;
    match out {
        Some(path) => {
            let path = std::path::PathBuf::from(&path);
            let mut replay = if path.exists() {
                DecisionReplay::load(&path)?
            } else {
                DecisionReplay::new()
            };
            replay.add(case);
            replay.save(&path)?;
            println!("decision case `{case_id}` appended to {}", path.display());
        }
        None => {
            println!("{}", serde_json::to_string_pretty(&case)?);
        }
    }
    Ok(())
}

/// `loopx replay run --case PATH` — replay every recorded case against the
/// kernel and diff. Fails closed on the first mismatch (regression canary).
fn cmd_replay_run(store: &Store, args: &[String]) -> Result<()> {
    use crate::replay::decision_replay::DecisionReplay;
    let mut path = None;
    let mut json = false;
    parse_pairs(args, |k, v| match k {
        "--case" => path = Some(v),
        "--json" => json = true,
        _ => {}
    });
    let path = path.ok_or_else(|| anyhow::anyhow!("--case required"))?;
    let replay = DecisionReplay::load(std::path::Path::new(&path))?;
    let mut any_mismatch = false;
    for case in &replay.cases {
        let comparison = crate::replay::decision_replay::replay_public_safe_decision_case(case)?;
        if json {
            println!("{}", serde_json::to_string(&comparison)?);
        } else {
            println!(
                "case {}: {}",
                comparison.case_id,
                if comparison.matched {
                    "MATCHED"
                } else {
                    "MISMATCH"
                }
            );
            for m in &comparison.mismatches {
                println!("  ✗ {m}");
            }
        }
        if !comparison.matched {
            any_mismatch = true;
        }
    }
    if any_mismatch {
        anyhow::bail!("decision replay failed: kernel behavior drifted from recorded cases");
    }
    let _ = store;
    Ok(())
}

/// `loopx replay corpus build --goal G [--out PATH] [--ablate PATH]...
/// [--patch NAME=JSON]...` — build a model-behavior corpus from the live
/// packet.
fn cmd_replay_corpus_build(store: &Store, args: &[String]) -> Result<()> {
    use crate::replay::corpus::{build_model_behavior_corpus, PatchCase};
    let mut goal_id = None;
    let mut out = None;
    let mut ablations: Vec<String> = vec![];
    let mut patches: Vec<PatchCase> = vec![];
    let mut patch_name = "p".to_string();
    let mut patch_index = 0usize;
    let mut i = 0;
    let argv: Vec<String> = args.to_vec();
    while i < argv.len() {
        match argv[i].as_str() {
            "--goal" => {
                i += 1;
                goal_id = argv.get(i).cloned();
            }
            "--out" => {
                i += 1;
                out = argv.get(i).cloned();
            }
            "--ablate" => {
                i += 1;
                if let Some(p) = argv.get(i) {
                    ablations.push(p.clone());
                }
            }
            "--patch-name" => {
                i += 1;
                if let Some(n) = argv.get(i) {
                    patch_name = n.clone();
                }
            }
            "--patch" => {
                i += 1;
                if let Some(raw) = argv.get(i) {
                    let value: serde_json::Value = serde_json::from_str(raw)
                        .map_err(|e| anyhow::anyhow!("--patch must be a JSON object: {e}"))?;
                    patches.push(PatchCase::new(&format!("{patch_name}{patch_index}"), value));
                    patch_index += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let packet = crate::decision::decide(&goal, std::time::SystemTime::now());
    let corpus = build_model_behavior_corpus(&packet, &patches, &[], &ablations, &[])?;
    match out {
        Some(path) => {
            corpus.save(std::path::Path::new(&path))?;
            println!(
                "corpus with {} case(s) written to {}",
                corpus.cases.len(),
                path
            );
        }
        None => {
            println!("{}", serde_json::to_string_pretty(&corpus)?);
        }
    }
    Ok(())
}

/// `loopx replay corpus run --corpus PATH [--repeats N] [--seed S] [--json]` —
/// run the corpus against the deterministic stub actor.
fn cmd_replay_corpus_run(store: &Store, args: &[String]) -> Result<()> {
    use crate::replay::corpus::{run_model_behavior_corpus, ModelBehaviorCorpus, StubActor};
    let mut corpus_path = None;
    let mut repeats = 3u32;
    let mut seed = 0u64;
    let mut json = false;
    parse_pairs(args, |k, v| match k {
        "--corpus" => corpus_path = Some(v),
        "--repeats" => repeats = v.parse::<u32>().unwrap_or(3),
        "--seed" => seed = v.parse::<u64>().unwrap_or(0),
        "--json" => json = true,
        _ => {}
    });
    let corpus_path = corpus_path.ok_or_else(|| anyhow::anyhow!("--corpus required"))?;
    let corpus = ModelBehaviorCorpus::load(std::path::Path::new(&corpus_path))?;
    let result = run_model_behavior_corpus(&corpus, &StubActor, repeats, seed)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }
    println!(
        "corpus run: {} case(s) x {} repeat(s), seed={}",
        result.case_count, result.repeats, result.seed
    );
    for case in &result.cases {
        println!(
            "  case {} ({}): passed={}",
            case.case_id, case.source_kind, case.passed
        );
    }
    println!(
        "gate: all_cases_passed={} corpus_gate_passed={} promotion_eligible={}",
        result.all_cases_passed, result.corpus_gate_passed, result.promotion_eligible
    );
    if !result.corpus_gate_passed {
        anyhow::bail!("corpus gate NOT passed");
    }
    let _ = store;
    Ok(())
}

/// `loopx replay record|run|corpus ...`
fn cmd_replay(store: &Store, args: &[String]) -> Result<()> {
    match args.first().map(|s| s.as_str()) {
        Some("record") => cmd_replay_record(store, &args[1..]),
        Some("run") => cmd_replay_run(store, &args[1..]),
        Some("corpus") => match args.get(1).map(|s| s.as_str()) {
            Some("build") => cmd_replay_corpus_build(store, &args[2..]),
            Some("run") => cmd_replay_corpus_run(store, &args[2..]),
            Some(other) => {
                anyhow::bail!("unknown replay corpus subcommand `{other}` (build|run)")
            }
            None => anyhow::bail!("replay corpus requires a subcommand (build|run)"),
        },
        Some(other) => {
            anyhow::bail!("unknown replay subcommand `{other}` (record|run|corpus)")
        }
        None => anyhow::bail!("replay requires a subcommand (record|run|corpus)"),
    }
}

// ── P4: canary smoke (G-20) ────────────────────────────────────────────────

/// `loopx canary smoke [--profile X] [--json]` — run a smoke profile
/// (default release-gate). Fails closed when any check fails.
fn cmd_canary(store: &Store, args: &[String]) -> Result<()> {
    let mut profile = None;
    let mut json = false;
    parse_pairs(args, |k, v| match k {
        "--profile" => profile = Some(v),
        "--json" => json = true,
        _ => {}
    });
    let profile = profile.unwrap_or_else(|| "release-gate".to_string());
    let result = crate::canary::run_smoke(store, &profile)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }
    println!(
        "canary smoke `{}` (suite {}): {} check(s)",
        result.profile_id,
        result.suite,
        result.checks.len()
    );
    for check in &result.checks {
        println!(
            "  [{}] {:<24} {}",
            if check.passed { "ok" } else { "FAIL" },
            check.id,
            check.detail
        );
    }
    println!(
        "result: {}",
        if result.all_passed {
            "ALL PASSED"
        } else {
            "FAILED"
        }
    );
    if !result.all_passed {
        anyhow::bail!("canary smoke `{profile}` failed");
    }
    Ok(())
}

// ── P4: diagnostics & command surface (G-27) ───────────────────────────────

/// `loopx version` — version + schema surface.
fn cmd_version(store: &Store, args: &[String]) -> Result<()> {
    println!("future-loop {}", env!("CARGO_PKG_VERSION"));
    println!("crate  : future-loop");
    println!("schemas:");
    println!("  benchmark_run_ledger_v0 (G-18)");
    println!("  public_safe_decision_replay_v0 (G-19)");
    println!("  model_behavior_corpus_v0 (G-19)");
    println!("  canary_smoke_run_v0 (G-20)");
    println!("  future_loop_turn_envelope_v0 (G-9)");
    println!("  scheduler_arbitration_v0 (G-2/G-11)");
    let _ = store;
    let _ = args;
    Ok(())
}

/// `future-loop diagnose --goal G [--format json]` — per-goal diagnostic
/// surface: current decision, open todos/gates, projection gaps, closure
/// status, and recent run evidence.
fn cmd_diagnose(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut format_json = false;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--format" => format_json = v == "json",
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let packet = decide_for(&goal, SystemTime::now(), None);
    if format_json {
        let diag = serde_json::json!({
            "goal_id": goal.goal_id,
            "objective": goal.objective,
            "status": goal.status,
            "decision": packet.decision,
            "mode": packet.interaction_contract.mode.as_str(),
            "reason": packet.reason,
            "open_todos": goal.todos.iter().filter(|t| t.status == TodoStatus::Open).count(),
            "open_gates": goal.open_gates().count(),
            "projection_gap": crate::store::projection_gap(&goal),
            "terminal": goal.terminal_closure().is_some(),
            "runs": goal.history.len(),
            "recent_evidence": goal.history.iter().rev().take(3).map(|r| serde_json::json!({
                "turn": r.turn, "todo": r.todo_id, "state": r.terminal_state,
                "tools": r.tools, "cost": r.cost_delta, "evidence": crate::decision::truncate(&r.evidence, 200),
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&diag)?);
        return Ok(());
    }
    println!("== diagnose {goal_id} ==");
    println!("objective : {}", goal.objective);
    println!(
        "status    : {} | terminal: {}",
        goal.status,
        goal.terminal_closure().is_some()
    );
    println!(
        "decision  : {} / {} | {}",
        packet.decision,
        packet.interaction_contract.mode.as_str(),
        packet.reason
    );
    println!(
        "todos     : {} open / {} gates",
        goal.todos
            .iter()
            .filter(|t| t.status == TodoStatus::Open)
            .count(),
        goal.open_gates().count()
    );
    if let Some(gap) = crate::store::projection_gap(&goal) {
        println!("gap       : {gap}");
    }
    for r in goal.history.iter().rev().take(3) {
        println!(
            "run       : #{} todo={} state={} cost=¥{:.4} tools=[{}]",
            r.turn,
            r.todo_id,
            r.terminal_state,
            r.cost_delta,
            r.tools.join(", ")
        );
    }
    Ok(())
}

/// `loopx doctor [--goal G] [--agent-addr ADDR]` — run the diagnostic
/// surface: canary release-gate + per-goal ledger/decision checks.
async fn cmd_doctor(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_filter = None;
    let mut agent_addr = None;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_filter = Some(v),
        "--agent-addr" => agent_addr = Some(v),
        _ => {}
    });
    println!("doctor: state root {}", store.root_path());
    let mut failures: Vec<String> = vec![];
    // Release-gate smoke (the deterministic surface).
    let smoke = crate::canary::run_smoke(store, "release-gate")?;
    for check in &smoke.checks {
        let mark = if check.passed { "ok" } else { "FAIL" };
        println!("  [{mark}] {:<24} {}", check.id, check.detail);
        if !check.passed {
            failures.push(format!("smoke.{}", check.id));
        }
    }
    // Per-goal ledger + decision checks.
    let goals: Vec<String> = match &goal_filter {
        Some(g) => vec![g.clone()],
        None => store.registry().iter().map(|e| e.goal_id.clone()).collect(),
    };
    for goal_id in goals {
        match store.replay(&goal_id) {
            Ok(Some(goal)) => {
                let verify = store.verify(&goal_id)?;
                println!(
                    "  goal {}: {} event(s) ({} unique, {} conflict(s)), {} todo(s)",
                    goal_id,
                    verify.total_events,
                    verify.unique_events,
                    verify.conflicts.len(),
                    goal.todos.len()
                );
                if !verify.ok {
                    failures.push(format!("goal {goal_id} ledger conflicts"));
                }
                if let Some(gap) = crate::store::projection_gap(&goal) {
                    failures.push(format!("goal {goal_id}: {gap}"));
                }
            }
            Ok(None) => failures.push(format!("goal {goal_id} not found")),
            Err(e) => failures.push(format!("goal {goal_id}: {e}")),
        }
    }
    // gRPC reachability probe (only when --agent-addr is given). Awaited
    // directly — a nested block_on here would panic inside console::run's
    // runtime (the probe used to build its own current_thread runtime).
    if let Some(addr) = &agent_addr {
        let probe = async {
            let mut client = crate::agent_client::AgentClient::connect(addr).await?;
            let session = client.new_session("/tmp").await?;
            let totals = client.session_totals(&session).await?;
            anyhow::Ok(format!(
                "session {session} live (tokens_in={} tokens_out={})",
                totals.tokens_in, totals.tokens_out
            ))
        }
        .await;
        match probe {
            Ok(detail) => println!("  [ok] agent gRPC {addr}: {detail}"),
            Err(e) => {
                println!("  [FAIL] agent gRPC {addr}: {e}");
                failures.push("agent gRPC unreachable".to_string());
            }
        }
    } else {
        println!("  (agent gRPC probe skipped; pass --agent-addr to check)");
    }
    if failures.is_empty() {
        println!("doctor: ALL CHECKS PASSED");
        Ok(())
    } else {
        anyhow::bail!(
            "doctor: {} failure(s): {}",
            failures.len(),
            failures.join("; ")
        )
    }
}

/// `loopx history --goal G` — the goal's run history (ledger-derived) +
/// decision summary per run.
fn cmd_history(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        }
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    if goal.history.is_empty() {
        println!("goal {goal_id}: no runs recorded");
        return Ok(());
    }
    println!(
        "goal {goal_id} run history ({} run(s)):",
        goal.history.len()
    );
    for record in &goal.history {
        let state = record.terminal_state.as_str();
        let tools = if record.tools.is_empty() {
            "".to_string()
        } else {
            format!(" tools={}", record.tools.join(","))
        };
        let evidence = if record.evidence.trim().is_empty() {
            String::new()
        } else {
            format!(
                " evidence=\"{}\"",
                crate::decision::truncate(&record.evidence, 60)
            )
        };
        println!(
            "  #{} todo={} state={}{}{} tokens={}+{} cost={:.4}",
            record.turn,
            record.todo_id,
            state,
            tools,
            evidence,
            record.tokens_in_delta,
            record.tokens_out_delta,
            record.cost_delta
        );
    }
    Ok(())
}

/// `loopx turn --goal G --todo-id T [--agent-id A]` — render the per-turn
/// envelope (instruction + context + evidence + decision summary) the agent
/// would receive for this todo.
fn cmd_turn(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut todo_id = None;
    let mut agent_id = None;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--todo-id" => todo_id = Some(v),
        "--agent-id" => agent_id = Some(v),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let todo_id = todo_id.ok_or_else(|| anyhow::anyhow!("--todo-id required"))?;
    let goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let todo = goal
        .todo(&todo_id)
        .ok_or_else(|| anyhow::anyhow!("todo {todo_id} not found in goal {goal_id}"))?;
    let packet =
        crate::decision::decide_for(&goal, std::time::SystemTime::now(), agent_id.as_deref());
    let prev = goal.history.last().filter(|r| r.todo_id == todo_id);
    let envelope = crate::turn_envelope::compose_turn_envelope(&goal, todo, Some(&packet), prev);
    println!("{envelope}");
    Ok(())
}

/// `loopx todo-event --goal G --todo-id T` — the event history of one todo.
fn cmd_todo_event(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut todo_id = None;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--todo-id" => todo_id = Some(v),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let todo_id = todo_id.ok_or_else(|| anyhow::anyhow!("--todo-id required"))?;
    let events = store.events(&goal_id)?;
    let relevant: Vec<&crate::store::StoredEvent> = events
        .iter()
        .filter(|se| event_touches_todo(&se.event, &todo_id))
        .collect();
    if relevant.is_empty() {
        println!("todo {todo_id}: no events in goal {goal_id}");
        return Ok(());
    }
    println!(
        "todo {todo_id} event history ({} event(s)):",
        relevant.len()
    );
    for se in relevant {
        println!("  {}", describe_event(&se.event));
    }
    Ok(())
}

/// Does an event reference this todo id?
fn event_touches_todo(event: &crate::store::Event, todo_id: &str) -> bool {
    use crate::store::Event;
    match event {
        Event::TodoAdded { todo, .. } => todo.id == todo_id,
        Event::TodoCompleted { todo_id: id, .. }
        | Event::TodoSuperseded { todo_id: id, .. }
        | Event::TodoClaimed { todo_id: id, .. }
        | Event::TodoArchived { todo_id: id, .. }
        | Event::MonitorPolled { todo_id: id, .. }
        | Event::QuotaSpent { todo_id: id, .. }
        | Event::EvidenceAttached { todo_id: id, .. }
        | Event::TodoRenewed { todo_id: id, .. }
        | Event::TodoReleased { todo_id: id, .. }
        | Event::TodoExpired { todo_id: id, .. }
        | Event::GateResolved { todo_id: id, .. } => id == todo_id,
        Event::RunRecorded { record, .. } => record.todo_id == todo_id,
        _ => false,
    }
}

/// One-line description of an event (todo-event / evidence-log surface).
fn describe_event(event: &crate::store::Event) -> String {
    use crate::store::Event;
    let kind = match event {
        Event::GoalStarted { .. } => "goal_started",
        Event::TodoAdded { .. } => "todo_added",
        Event::TodoCompleted {
            todo_id,
            no_follow_up,
            successor_ids,
            ..
        } => {
            return format!(
                "todo_completed todo={todo_id} no_follow_up={no_follow_up} successors={}",
                successor_ids.join(",")
            );
        }
        Event::TodoSuperseded { todo_id, .. } => {
            return format!("todo_superseded todo={todo_id}");
        }
        Event::TodoUpdated { todo_id, .. } => {
            return format!("todo_updated todo={todo_id}");
        }
        Event::GoalCancelled { .. } => {
            return "goal_cancelled".to_string();
        }
        Event::GateResolved {
            todo_id, decision, ..
        } => {
            return format!("gate_resolved todo={todo_id} decision=\"{decision}\"");
        }
        Event::GapSatisfied { gap_id, .. } => {
            return format!("gap_satisfied gap={gap_id}");
        }
        Event::RunRecorded { record, .. } => {
            return format!(
                "run_recorded todo={} state={} tokens={}+{}",
                record.todo_id,
                record.terminal_state,
                record.tokens_in_delta,
                record.tokens_out_delta
            );
        }
        Event::TodoClaimed {
            todo_id, agent_id, ..
        } => {
            return format!("todo_claimed todo={todo_id} agent={agent_id}");
        }
        Event::AgentRegistered { agent_id, .. } => {
            return format!("agent_registered agent={agent_id}");
        }
        Event::AgentOnboarded {
            agent_id,
            capabilities,
            ..
        } => {
            return format!(
                "agent_onboarded agent={agent_id} capabilities={}",
                capabilities.join(",")
            );
        }
        Event::ReplanAcked { delta_kinds, .. } => {
            return format!("replan_acked deltas={}", delta_kinds.join(","));
        }
        Event::ProfileSet {
            outcome_floor_streak_threshold,
            ..
        } => {
            return format!("profile_set outcome_floor={outcome_floor_streak_threshold}");
        }
        Event::AuthoritySet {
            write_scope,
            requires_approval,
            ..
        } => {
            return format!(
                "authority_set write_scope={} requires_approval={}",
                write_scope.join(","),
                requires_approval.join(",")
            );
        }
        Event::TodoArchived { todo_id, .. } => {
            return format!("todo_archived todo={todo_id}");
        }
        Event::MonitorPolled {
            todo_id,
            result,
            no_change_count,
            ..
        } => {
            return format!(
                "monitor_polled todo={todo_id} result={result} no_change_count={no_change_count}"
            );
        }
        Event::QuotaSpent {
            run_id,
            todo_id,
            source,
            slots,
            ..
        } => {
            return format!(
                "quota_spent run={run_id} todo={todo_id} source={source} slots={slots}"
            );
        }
        Event::EvidenceAttached { todo_id, .. } => {
            return format!("evidence_attached todo={todo_id}");
        }
        Event::TodoRenewed {
            todo_id, agent_id, ..
        } => {
            return format!("todo_renewed todo={todo_id} agent={agent_id}");
        }
        Event::TodoReleased {
            todo_id, agent_id, ..
        } => {
            return format!("todo_released todo={todo_id} agent={agent_id}");
        }
        Event::TodoExpired { todo_id, .. } => {
            return format!("todo_expired todo={todo_id}");
        }
        Event::SupervisorProposed {
            decision_id,
            target_agent_id,
            ..
        } => {
            return format!("supervisor_proposed decision={decision_id} target={target_agent_id}");
        }
        Event::SupervisorReceiptRecorded {
            decision_id,
            outcome,
            ..
        } => {
            return format!("supervisor_receipt decision={decision_id} outcome={outcome}");
        }
    };
    kind.to_string()
}

/// `loopx evidence-log --goal G [--todo-id T]` — the evidence trail:
/// EvidenceAttached events + run evidence per todo.
fn cmd_evidence_log(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut todo_id = None;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--todo-id" => todo_id = Some(v),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let events = store.events(&goal_id)?;
    let mut printed = 0usize;
    for se in &events {
        use crate::store::Event;
        match &se.event {
            Event::EvidenceAttached {
                todo_id: tid,
                evidence,
                ..
            } => {
                if todo_id.as_deref().map(|t| t == tid).unwrap_or(true) {
                    println!(
                        "[attached] todo={tid}: {}",
                        crate::decision::truncate(evidence, 200)
                    );
                    printed += 1;
                }
            }
            Event::RunRecorded { record, .. } => {
                if todo_id
                    .as_deref()
                    .map(|t| t == record.todo_id)
                    .unwrap_or(true)
                    && !record.evidence.trim().is_empty()
                {
                    println!(
                        "[run #{}] todo={}: {}",
                        record.turn,
                        record.todo_id,
                        crate::decision::truncate(&record.evidence, 200)
                    );
                    printed += 1;
                }
            }
            Event::TodoCompleted {
                todo_id: tid,
                evidence: Some(evidence),
                ..
            } if todo_id.as_deref().map(|t| t == tid).unwrap_or(true) => {
                println!(
                    "[completed] todo={tid}: {}",
                    crate::decision::truncate(evidence, 200)
                );
                printed += 1;
            }
            _ => {}
        }
    }
    if printed == 0 {
        println!(
            "goal {goal_id}: no evidence recorded{}",
            todo_id
                .map(|t| format!(" for todo {t}"))
                .unwrap_or_default()
        );
    } else {
        println!("({printed} evidence item(s))");
    }
    Ok(())
}

/// `loopx todo archive --goal G --todo-id T` — archive a todo
/// (LoopX: archive_state "archived").
fn todo_archive(store: &mut Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut todo_id = None;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--todo-id" => todo_id = Some(v),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let todo_id = todo_id.ok_or_else(|| anyhow::anyhow!("--todo-id required"))?;
    let mut goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    goal.archive_todo(&todo_id);
    store
        .append(Event::TodoArchived {
            goal_id: goal_id.clone(),
            todo_id: todo_id.clone(),
            ts: crate::state::now_epoch(),
        })
        .ok();
    refresh_next_action(store, &goal_id)?;
    sync_compat(store, &goal_id)?;
    println!("todo {todo_id} archived ✔");
    Ok(())
}

/// `todo supersede --goal G --todo-id T [--reason ...]` — mark an unfinished
/// todo superseded (obsolete; runnable frontier and closure both ignore it).
fn todo_supersede(store: &mut Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut todo_id = None;
    let mut reason = None;
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--todo-id" => todo_id = Some(v),
        "--reason" => reason = Some(v),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let todo_id = todo_id.ok_or_else(|| anyhow::anyhow!("--todo-id required"))?;
    let mut goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let Some(target) = goal.todo(&todo_id) else {
        bail!("todo {todo_id} not found in goal {goal_id}");
    };
    if target.status == crate::state::TodoStatus::Done {
        bail!("todo {todo_id} is already done — supersede only applies to unfinished todos");
    }
    goal.supersede(&todo_id);
    store.append(Event::TodoSuperseded {
        goal_id: goal_id.clone(),
        todo_id: todo_id.clone(),
        ts: crate::state::now_epoch(),
    })?;
    refresh_next_action(store, &goal_id)?;
    sync_compat(store, &goal_id)?;
    println!(
        "todo {todo_id} superseded ✔{}",
        reason
            .as_deref()
            .map(|r| format!(" (reason: {r})"))
            .unwrap_or_default()
    );
    Ok(())
}

/// `todo update --goal G --todo-id T [--text ...] [--status open|blocked|deferred|superseded]
/// [--evidence ...] [--note ...] [--priority P0|P1|P2] [--resume-when ...]` — field-level
/// update on an open todo. `--status done` is rejected (completion policy is
/// enforced by `todo complete`).
fn todo_update(store: &mut Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut todo_id = None;
    let mut text = None;
    let mut status = None;
    let mut evidence = None;
    let mut note = None;
    let mut priority = None;
    let mut resume_when = None;
    let mut blocks: Option<Vec<String>> = None;
    let mut unknown_flags: Vec<String> = vec![];
    parse_pairs(args, |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--todo-id" => todo_id = Some(v),
        "--text" => text = Some(v),
        "--status" => status = Some(v),
        "--evidence" => evidence = Some(v),
        "--note" => note = Some(v),
        "--priority" => priority = Some(v),
        "--resume-when" => resume_when = Some(v),
        "--blocks" => {
            // `--blocks a,b` replaces the blocking set; `--blocks ""` clears
            // it (empty string → Some(vec![])); absent → leave untouched.
            let trimmed = v.trim();
            blocks = Some(if trimmed.is_empty() {
                vec![]
            } else {
                trimmed
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            });
        }
        "--help" | "-h" => {
            eprintln!("usage: todo update --goal G --todo-id T [--text T] [--status S] [--evidence E] [--note N] [--priority P0|P1|P2] [--resume-when N|TEXT] [--blocks a,b]");
            std::process::exit(0);
        }
        other => unknown_flags.push(other.to_string()),
    });
    if !unknown_flags.is_empty() {
        anyhow::bail!("todo update: unknown flag(s): {}", unknown_flags.join(", "));
    }
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let todo_id = todo_id.ok_or_else(|| anyhow::anyhow!("--todo-id required"))?;
    let goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    if goal.todo(&todo_id).is_none() {
        anyhow::bail!("todo {todo_id} not found in goal {goal_id}");
    }
    if status.as_deref() == Some("done") {
        bail!(
            "todo update --status done is not allowed — use `todo complete --no-follow-up|--successor` \
             (completion policy, successor, and no-follow-up contracts are enforced)"
        );
    }
    // `--resume-when N` with a numeric N means "defer N seconds from now"
    // (same semantics as `--defer-secs`), so a deferred/monitor todo actually
    // becomes due. A non-numeric value keeps the legacy text-only behavior
    // (resume_when_text hint, no real deadline).
    let resume_when_parsed = resume_when.as_deref().map(|rw| {
        if let Ok(secs) = rw.trim().parse::<u64>() {
            format!("defer:{secs}")
        } else {
            rw.to_string()
        }
    });
    store.append(Event::TodoUpdated {
        goal_id: goal_id.clone(),
        todo_id: todo_id.clone(),
        text: text.clone(),
        status: status.clone(),
        evidence: evidence.clone(),
        note: note.clone(),
        priority: priority.clone(),
        resume_when: resume_when_parsed,
        blocks: blocks.clone(),
        ts: crate::state::now_epoch(),
    })?;
    refresh_next_action(store, &goal_id)?;
    sync_compat(store, &goal_id)?;
    println!("todo {todo_id} updated ✔");
    Ok(())
}

// ── in-module coverage tests ───────────────────────────────────────────────
// Helpers and render paths that the CLI surface cannot reach (private fns,
// defensive arms, non-todo event descriptions, the `future loop` prog name).

#[cfg(test)]
mod coverage_tests {
    use super::*;
    use crate::state::{Goal, RunRecord, TaskClass, Todo, TodoStatus};
    use crate::store::Event;

    fn record(todo_id: &str) -> RunRecord {
        RunRecord {
            turn: 1,
            todo_id: todo_id.to_string(),
            run_id: "run-1".to_string(),
            terminal_state: "completed".to_string(),
            error: None,
            tokens_in_delta: 1,
            tokens_out_delta: 2,
            cost_delta: 0.1,
            tools: vec!["shell".to_string()],
            evidence: "proof".to_string(),
            recorded_at: 1_700_000_000,
            spend_source: Some("run".to_string()),
            validation: None,
        }
    }

    fn all_events(todo_id: &str) -> Vec<Event> {
        vec![
            Event::GoalStarted { goal_id: "g".into(), ts: 1 },
            Event::TodoAdded {
                goal_id: "g".into(),
                todo: Todo::advancement(todo_id, "task"),
                ts: 1,
            },
            Event::TodoCompleted {
                goal_id: "g".into(),
                todo_id: todo_id.into(),
                no_follow_up: true,
                successor_ids: vec!["s1".into()],
                evidence: Some("e".into()),
                ts: 1,
            },
            Event::TodoSuperseded { goal_id: "g".into(), todo_id: todo_id.into(), ts: 1 },
            Event::TodoUpdated {
                goal_id: "g".into(),
                todo_id: todo_id.into(),
                text: Some("t".into()),
                status: None,
                evidence: None,
                note: None,
                priority: None,
                resume_when: None,
                blocks: None,
                ts: 1,
            },
            Event::GoalCancelled { goal_id: "g".into(), reason: "r".into(), ts: 1 },
            Event::GateResolved {
                goal_id: "g".into(),
                todo_id: todo_id.into(),
                decision: "d".into(),
                note: None,
                ts: 1,
            },
            Event::GapSatisfied { goal_id: "g".into(), gap_id: "gap1".into(), ts: 1 },
            Event::RunRecorded { goal_id: "g".into(), record: record(todo_id), ts: 1 },
            Event::TodoClaimed {
                goal_id: "g".into(),
                todo_id: todo_id.into(),
                agent_id: "a".into(),
                lease_expires_at: 9,
                ts: 1,
            },
            Event::AgentRegistered { goal_id: "g".into(), agent_id: "a".into(), ts: 1 },
            Event::AgentOnboarded {
                goal_id: "g".into(),
                agent_id: "a".into(),
                capabilities: vec!["shell".into()],
                ts: 1,
            },
            Event::ReplanAcked { goal_id: "g".into(), delta_kinds: vec!["vision_patch".into()], ts: 1 },
            Event::ProfileSet { goal_id: "g".into(), outcome_floor_streak_threshold: 2, ts: 1 },
            Event::AuthoritySet {
                goal_id: "g".into(),
                write_scope: vec!["src".into()],
                requires_approval: vec!["publish".into()],
                ts: 1,
            },
            Event::TodoArchived { goal_id: "g".into(), todo_id: todo_id.into(), ts: 1 },
            Event::MonitorPolled {
                goal_id: "g".into(),
                todo_id: todo_id.into(),
                result: "changed".into(),
                no_change_count: 0,
                ts: 1,
            },
            Event::QuotaSpent {
                goal_id: "g".into(),
                run_id: "r1".into(),
                todo_id: todo_id.into(),
                source: "run".into(),
                slots: 1,
                ts: 1,
            },
            Event::EvidenceAttached {
                goal_id: "g".into(),
                todo_id: todo_id.into(),
                evidence: "e".into(),
                ts: 1,
            },
            Event::TodoRenewed {
                goal_id: "g".into(),
                todo_id: todo_id.into(),
                agent_id: "a".into(),
                lease_expires_at: 9,
                ts: 1,
            },
            Event::TodoReleased {
                goal_id: "g".into(),
                todo_id: todo_id.into(),
                agent_id: "a".into(),
                ts: 1,
            },
            Event::TodoExpired { goal_id: "g".into(), todo_id: todo_id.into(), ts: 1 },
            Event::SupervisorProposed {
                goal_id: "g".into(),
                supervisor_agent_id: "sup".into(),
                decision_id: "d1".into(),
                decision_kind: "observe".into(),
                target_agent_id: "w1".into(),
                required_host_capabilities: vec![],
                decision: "watch".into(),
                ts: 1,
            },
            Event::SupervisorReceiptRecorded {
                goal_id: "g".into(),
                decision_id: "d1".into(),
                receipt_id: "r1".into(),
                adapter_id: "ad".into(),
                outcome: "executed".into(),
                authority_ref: Some("auth".into()),
                rollback_ref: None,
                ts: 1,
            },
        ]
    }

    #[test]
    fn describe_event_covers_every_variant() {
        for ev in all_events("todo_1") {
            let s = describe_event(&ev);
            assert!(!s.is_empty(), "{ev:?}");
        }
    }

    #[test]
    fn event_touches_todo_matrix() {
        let mut wrongly_touched = false;
        for ev in all_events("todo_1") {
            let _touches = event_touches_todo(&ev, "todo_1");
            // No event variant may match a different todo id.
            wrongly_touched |= event_touches_todo(&ev, "todo_other");
        }
        assert!(!wrongly_touched);
        assert!(event_touches_todo(&Event::TodoAdded {
            goal_id: "g".into(),
            todo: Todo::advancement("todo_1", "task"),
            ts: 1,
        }, "todo_1"));
        assert!(!event_touches_todo(
            &Event::GoalStarted { goal_id: "g".into(), ts: 1 },
            "todo_1"
        ));
    }

    #[test]
    fn human_dur_ranges() {
        assert_eq!(human_dur(59), "59s");
        assert_eq!(human_dur(90), "1m30s");
        assert_eq!(human_dur(3600), "1h0m");
        assert_eq!(human_dur(3705), "1h1m");
    }

    #[test]
    fn status_label_matrix() {
        let mut t = Todo::advancement("t", "x");
        t.status = TodoStatus::Done;
        t.no_follow_up = true;
        assert_eq!(status_label(&t), "done(no-follow-up)");
        t.no_follow_up = false;
        t.successor_ids = vec!["s".into()];
        assert_eq!(status_label(&t), "done(+successor)");
        t.successor_ids = vec![];
        assert_eq!(status_label(&t), "done");
        t.status = TodoStatus::Superseded;
        assert_eq!(status_label(&t), "superseded");
        for (class, label) in [
            (TaskClass::Advancement, "open"),
            (TaskClass::UserGate, "GATE"),
            (TaskClass::UserAction, "action"),
            (TaskClass::Monitor, "monitor"),
            (TaskClass::Blocker, "blocker"),
        ] {
            let mut t = Todo::advancement("t", "x");
            t.class = class;
            assert_eq!(status_label(&t), label);
        }
    }

    #[test]
    fn parse_pairs_edge_cases() {
        let mut seen: Vec<(String, String)> = vec![];
        parse_pairs(
            &[
                "--flag".to_string(),       // boolean-ish flag at end
                "positional".to_string(),   // skipped
                "--key".to_string(),
                "value".to_string(),
                "--no-follow-up".to_string(), // known boolean, followed by value-less
                "--after-bool".to_string(),
            ],
            |k, v| seen.push((k.to_string(), v)),
        );
        // "--flag" is followed by a non-flag arg → consumes it as its value
        // (parse_pairs has no arity table; only the four known booleans are
        // value-less).
        assert!(seen.contains(&("--flag".to_string(), "positional".to_string())));
        assert!(seen.contains(&("--key".to_string(), "value".to_string())));
        assert!(seen.contains(&("--no-follow-up".to_string(), "true".to_string())));
        // "--after-bool" at the end gets "true".
        assert!(seen.contains(&("--after-bool".to_string(), "true".to_string())));
        assert!(!seen.iter().any(|(k, _)| k == "positional"));
    }

    #[test]
    fn join_ids_both() {
        assert_eq!(join_ids(&[]), "(none)");
        assert_eq!(join_ids(&["a".to_string(), "b".to_string()]), "a, b");
    }

    #[test]
    fn print_goal_status_full() {
        // Acceptance gaps (satisfied + open), monitor metadata, projection
        // gap, history — every print arm.
        let mut goal = Goal::new("g1", "objective", "/tmp")
            .with_acceptance(vec![("gap1", "needs proof"), ("gap2", "more proof")]);
        goal.satisfy_gap("gap2");
        goal.todos.push(Todo::advancement("t1", "open task"));
        let mut mon = Todo::monitor("m1", "watch", std::time::Duration::from_secs(60));
        mon.monitor_target = Some("file:x".into());
        mon.monitor_policy = Some("exists".into());
        mon.monitor_cadence = Some("15m".into());
        goal.todos.push(mon);
        goal.history.push(record("t1"));
        // No next_action → "-" arm; the frontier disagrees → projection gap.
        print_goal_status(&goal);
        goal.next_action = Some("open task".into());
        print_goal_status(&goal);
    }

    #[test]
    fn prog_and_help_surface() {
        // prog() falls back to the standalone name before run() sets it, or
        // reflects the OnceLock afterwards — either way it is exercised.
        let _ = prog();
        // cli_help adapts the USAGE line when invoked as `future loop`.
        let _ = PROG.set("future loop".to_string());
        let registry = build_cli_registry();
        cli_help(&registry, false).unwrap();
        cli_help(&registry, true).unwrap();
    }

    #[test]
    fn root_dir_env_override() {
        std::env::set_var("FUTURE_LOOP_ROOT", "/tmp/loop-root-dir-test");
        assert_eq!(root_dir(), "/tmp/loop-root-dir-test");
        std::env::remove_var("FUTURE_LOOP_ROOT");
        assert!(root_dir().ends_with("/.future/loop"), "{}", root_dir());
    }

    #[test]
    fn gen_id_format() {
        let id = gen_id("todo");
        assert!(id.starts_with("todo_"), "{id}");
        assert_eq!(id.len(), "todo_".len() + 12);
    }

    #[test]
    fn capability_hook_unknown_capability_bails() {
        let err = cmd_capability_hook("ghost-cmd", "ghost_cap", &[]).unwrap_err();
        assert!(format!("{err:#}").contains("unknown capability"));
    }

    #[test]
    fn print_status_json_paths() {
        let dir = std::env::temp_dir().join(format!(
            "future-loop-console-json-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let root = dir.to_string_lossy().into_owned();
        let mut store = Store::open(&root).unwrap();
        let goal = Goal::new("gj", "json goal", "/tmp");
        store.register(&goal).unwrap();
        store
            .append(Event::GoalStarted { goal_id: "gj".into(), ts: 1 })
            .unwrap();
        // No filter → iterates the registry; with filter → single goal;
        // unknown filter → error.
        print_status_json(&store, None).unwrap();
        print_status_json(&store, Some("gj".to_string())).unwrap();
        assert!(print_status_json(&store, Some("goal_nope".to_string())).is_err());
    }

    #[test]
    fn sync_helpers_tolerate_missing_goals() {
        let dir = std::env::temp_dir().join(format!(
            "future-loop-console-sync-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let root = dir.to_string_lossy().into_owned();
        let mut store = Store::open(&root).unwrap();
        // sync_compat on a goal with no ledger → Ok no-op; refresh_next_action
        // on the same → not-found error.
        sync_compat(&store, "goal_ghost").unwrap();
        assert!(refresh_next_action(&mut store, "goal_ghost").is_err());
        // And the write path for a real goal (produces ACTIVE_GOAL_STATE.md).
        let goal = Goal::new("gs", "sync goal", "/tmp");
        store.register(&goal).unwrap();
        store
            .append(Event::GoalStarted { goal_id: "gs".into(), ts: 1 })
            .unwrap();
        refresh_next_action(&mut store, "gs").unwrap();
        sync_compat(&store, "gs").unwrap();
        assert!(store.goal_dir("gs").join("ACTIVE_GOAL_STATE.md").exists());
    }
}
