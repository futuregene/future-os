//! `future-loop` — FutureOS loop control plane CLI.
//!
//! Commands (mirror the reference core tick, implemented natively):
//!   goal init    — create a durable goal (registry + event ledger)
//!   todo add     — add an agent/user/gate/monitor todo
//!   todo claim   — claim a todo (owner identity)
//!   todo complete — complete a todo; REQUIRES --no-follow-up or --successor,
//!     non-empty --evidence for advancement todos, and (when declared)
//!     --acceptance tokens inside the evidence; --force overrides both.
//!   gate resolve — resolve a user gate with a decision payload
//!   status       — project the active state (todos, gaps, next action)
//!   quota should-run — emit the typed ShouldRunPacket (deterministic)
//!   run          — drive one bounded gRPC turn + writeback (needs agent)
//!
//! State lives under `--root` (default `~/.future/loop/`), one goal per
//! directory, event-sourced: `loop status` replays the ledger each time.

use std::collections::HashMap;
use std::time::SystemTime;

use crate::agent_client::TurnProgressTracker;
use crate::cli::registry::{CommandRegistry, Journey};
use crate::decision::{complete_todo, decide_for, MAX_REPAIR_ATTEMPTS};
use crate::executor::{execute_turn, writeback};
use crate::state::{now_epoch, Goal, TaskClass, Todo, TodoStatus};
use crate::store::{Event, Store};
use anyhow::{bail, Context, Result};

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
    if registry.find(&args[0], include_experimental).is_none() {
        bail!("unknown command `{}` (try `{prog} --help`)", args[0]);
    }
    // P0-3②: `<command> --help` / `-h` prints the command's usage from the
    // registry instead of the flag being silently swallowed by argument
    // parsing (previously `--help` on a subcommand was a no-op boolean).
    if args[1..].iter().any(|a| a == "--help" || a == "-h") {
        print!(
            "{}",
            render_command_help(&registry, &args[0], include_experimental)
        );
        return Ok(());
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
        "frontier" => cmd_frontier(&store, &args[1..]),
        "profile" => cmd_profile(&mut store, &args[1..]),
        "status" => cmd_status(&store, &args[1..]),
        "quota" => cmd_quota(&store, &args[1..]),
        "scheduler" => cmd_scheduler(&mut store, &args[1..]),
        "store" => cmd_store(&mut store, &args[1..]),
        "backfill" => cmd_backfill(&mut store, &args[1..]),
        "privacy" => cmd_privacy(&store, &args[1..]),
        "lease" => cmd_lease(&mut store, &args[1..]),
        "runs" => cmd_runs(&store, &args[1..]),
        "heartbeat-prompt" => cmd_heartbeat(&store, &args[1..]),
        "worker-bridge" => cmd_worker_bridge(&mut store, &args[1..]).await,
        "models" => cmd_models(&args[1..]).await,
        "diagnose" => cmd_diagnose(&store, &args[1..]),
        "run" => cmd_run(&mut store, &args[1..]).await,
        // ── P3 commands ──────────────────────────────────────────────────
        "scope" => cmd_scope(&store, &args[1..]),
        "lane" => cmd_lane(&store, &args[1..]),
        "supervisor" => cmd_supervisor(&mut store, &args[1..]),
        "task-graph" => cmd_task_graph(&store, &args[1..]),
        "attention" => cmd_attention(&store, &args[1..]),
        "inbox" => cmd_inbox(&store, &args[1..]),
        "delivery" => cmd_delivery(&mut store, &args[1..]),
        "registry" => cmd_registry(&registry, &args[1..]),
        "commands" => cmd_commands(&registry, &args[1..]),
        // ── P4 commands (G-20 / G-27) ───────────────────────────────────
        "canary" => cmd_canary(&store, &args[1..]),
        "version" => cmd_version(&store, &args[1..]),
        "doctor" => cmd_doctor(&store, &args[1..]).await,
        "history" => cmd_history(&store, &args[1..]),
        "turn" => cmd_turn(&store, &args[1..]),
        "todo-event" => cmd_todo_event(&store, &args[1..]),
        "evidence-log" => cmd_evidence_log(&store, &args[1..]),
        other => bail!("unknown command `{other}` (try `{prog} --help`)"),
    }
}

/// P1-9: operator journey assignments for the statically registered
/// commands (`future loop commands` grouped view). A test
/// (`journey_assignments_cover_every_static_command`) keeps this table in
/// sync with `build_cli_registry`.
const JOURNEY_ASSIGNMENTS: &[(&str, Journey)] = &[
    // Start here — first goal, first status, install checks
    ("goal", Journey::Starter),
    ("status", Journey::Starter),
    ("doctor", Journey::Starter),
    ("agent", Journey::Starter),
    // Daily operator — the day-to-day control surface
    ("todo", Journey::Daily),
    ("gate", Journey::Daily),
    ("replan", Journey::Daily),
    ("frontier", Journey::Daily),
    ("lease", Journey::Daily),
    ("task-graph", Journey::Daily),
    ("quota", Journey::Daily),
    ("scheduler", Journey::Daily),
    ("attention", Journey::Daily),
    ("inbox", Journey::Daily),
    ("delivery", Journey::Daily),
    ("diagnose", Journey::Daily),
    ("evidence-log", Journey::Daily),
    ("todo-event", Journey::Daily),
    ("history", Journey::Daily),
    // Loop driver — per-turn execution surface for the driving agent
    ("run", Journey::Driver),
    ("turn", Journey::Driver),
    ("heartbeat-prompt", Journey::Driver),
    ("worker-bridge", Journey::Driver),
    ("scope", Journey::Driver),
    ("lane", Journey::Driver),
    ("supervisor", Journey::Driver),
    // Setup & automation — one-time configuration
    ("models", Journey::Setup),
    ("authority", Journey::Setup),
    ("profile", Journey::Setup),
    ("store", Journey::Setup),
    ("backfill", Journey::Setup),
    ("privacy", Journey::Setup),
    // Maintainer & adapter — quality gates, retention, introspection
    ("canary", Journey::Maintainer),
    ("runs", Journey::Maintainer),
    ("backup", Journey::Maintainer),
    ("version", Journey::Maintainer),
    ("registry", Journey::Maintainer),
    ("commands", Journey::Maintainer),
];

/// G-26: build the command registry — groups + commands, the aggregated
/// help surface.
fn build_cli_registry() -> CommandRegistry {
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
        "status [--goal G] [--format json]",
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
        "ack a replan obligation / list obligations / inspect or set the replan rule set",
        "replan ack --goal G --delta-kind ... | replan obligations --goal G | replan rules show|set --goal G",
    );
    r.command(
        todo,
        "frontier",
        "goal-frontier projection (G13): frontier + outcome segments + replan rule + terminal judgement + semantic history",
        "frontier show --goal G [--format json]",
    );
    r.command(
        todo,
        "lease",
        "task lease lifecycle (claim/renew/release/expire/status)",
        "lease claim|renew|release|expire|status --goal G --todo-id T [--agent-id A] [--force (claim)] [--format json (status)]",
    );
    r.command(
        todo,
        "task-graph",
        "todo dependency graph (G-14)",
        "task-graph --goal G [--format json]",
    );

    let agent = r.group("agent", "agent sessions");
    r.command(
        agent,
        "agent",
        "register/onboard agents + multi-agent contract/recipe/succession/collective surface (G12)",
        "agent onboard --goal G --agent-id A [--recipe N] | list|contract|recipe|succession|collective --goal G",
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
        "history --goal G [--format json]",
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
        "todo-event --goal G --todo-id T [--format json]",
    );
    r.command(
        ops,
        "evidence-log",
        "evidence trail (attached + run + completion evidence)",
        "evidence-log --goal G [--todo-id T] [--format json]",
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
        "quota should-run / usage / spend / decisions",
        "quota should-run --goal G [--format json] | usage [--goal G] [--all] | spend --goal G  | decisions --goal G [--limit N]",
    );
    r.command(
        ops,
        "scheduler",
        "scheduler tick/show/record-host-failure/ack/liveness",
        "scheduler tick|show|record-host-failure|ack|liveness --goal G [--agent-id A] [--threshold-secs N (liveness)] [--format json (show|liveness)]",
    );
    r.command(
        ops,
        "store",
        "event-store schema migration / ledger integrity / read-model repair",
        "store migrate|verify|bridge --goal G [--repair|--format json (verify)]",
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
        "run",
        "drive one bounded gRPC turn (requires --agent-id; auto-registers)",
        "run --goal G --agent-id A [--model M] [--thinking-level L] [--max-turns N] [--lease-secs N] [--force-workspace]",
    );

    let work_items = r.group(
        "work-items",
        "attention / operator inbox (G-15) / delivery outcome closure (P0-2)",
    );
    r.command(
        work_items,
        "attention",
        "project the attention queue",
        "attention [--goal G] [--all] [--format json]",
    );
    r.command(
        work_items,
        "inbox",
        "project the operator inbox urgency",
        "inbox --project DIR [--scope addressed_only|configured_chat_all] [--name NAME] [--format json]",
    );
    r.command(
        work_items,
        "delivery",
        "post-delivery outcome closure (P0-2): delivered → verified/failed/rework + follow-through",
        "delivery status --goal G [--format json] | record --goal G --todo-id T --outcome verified|failed|rework [--note N] | followthrough --goal G [--turns N]",
    );

    let cli = r.group("cli", "command registry");
    r.command(
        cli,
        "registry",
        "inspect the CLI registry (groups/commands)",
        "registry [--format json|--json] [--include-experimental]",
    );
    r.command(
        cli,
        "commands",
        "grouped operator command reference (P1-9 journey view)",
        "commands [--format json|--json] [--include-experimental]",
    );

    let canary = r.group("canary", "canary smoke (G-20)");
    r.command(
        canary,
        "canary",
        "run a smoke profile (release gate default) or the premerge CI gate",
        "canary smoke [--profile core-control-plane|release-gate|premerge] [--json] | canary premerge [--json]",
    );

    // P1-9: journey metadata overlay (presentation only — the registry
    // itself stays the flat machine catalog).
    for (name, journey) in JOURNEY_ASSIGNMENTS {
        r.set_journey(name, *journey);
    }

    r
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

/// P0-3②: render the per-command help for `<command> --help` — the command's
/// summary + usage from the registry (pure, unit-testable; the caller prints).
fn render_command_help(
    registry: &CommandRegistry,
    command: &str,
    include_experimental: bool,
) -> String {
    if let Some((group, def)) = registry.find(command, include_experimental) {
        let mark = if def.experimental {
            " (experimental)"
        } else {
            ""
        };
        format!(
            "{} — {}{}\n\nusage: {}\n\ngroup: {} — {}\n\nfull command list: {} --help\n",
            def.name,
            def.summary,
            mark,
            def.usage,
            group.name,
            group.summary,
            prog()
        )
    } else {
        // Unreachable: main_from_args validates the command before help.
        format!("unknown command `{command}` (try `{} --help`)\n", prog())
    }
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
    reject_unknown_flags(args, &["--cwd", "--goal-doc", "--goal-id", "--objective"])?;
    parse_pairs(args, |k, v| {
        if k == "--objective" {
            objective = Some(v);
        } else if k == "--cwd" {
            cwd = Some(v);
        } else if k == "--goal-id" {
            goal_id = Some(v);
        } else if k == "--goal-doc" {
            goal_doc = Some(v);
        }
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
    reject_unknown_flags(args, &["--goal", "--reason"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--reason" {
            reason = v;
        }
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
    let next_action = "goal cancelled — automation stopped, state retained";
    store.set_next_action(&goal_id, next_action)?;
    sync_compat(store, &goal_id)?;
    println!("goal {goal_id} cancelled ✔ (automation stopped, state retained — reason: {reason})");
    Ok(())
}

/// `goal delete --goal G [--force]` — remove the registry entry + state.
/// Irreversible; requires --force (tip: `goal cancel` keeps state).
fn cmd_goal_delete(store: &mut Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut force = false;
    reject_unknown_flags(args, &["--force", "--goal"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--force" {
            force = true;
        }
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

/// O4: heuristic for the `todo add` advisory hint — does the text look like a
/// code/implementation task that should carry a `--verify` validator? Matches
/// coding keywords (worktree / commit / cargo / clippy / 测试 / 代码 …) or a
/// `.rs` source path. Advisory only; never changes CLI semantics.
fn looks_like_code_todo(text: &str) -> bool {
    let lower = text.to_lowercase();
    if lower.contains(".rs") {
        return true;
    }
    const CODE_WORDS: &[&str] = &[
        "worktree", "commit", "cargo", "clippy", "rustfmt", "test", "tests", "compile", "build",
        "lint", "refactor", "patch", "crate", "git", "merge", "code", "debug",
    ];
    const CODE_ZH: &[&str] = &["测试", "代码", "编译", "修复"];
    CODE_ZH.iter().any(|k| text.contains(k))
        || lower
            .split(|c: char| !c.is_alphanumeric())
            .any(|tok| CODE_WORDS.contains(&tok))
}

/// O5: heuristic for the `todo add` advisory hint — does the text look like an
/// external-delivery task (platform submission, scored attempt, quota-bounded
/// side effect) whose completion should carry an `--acceptance` contract?
/// Advisory only; never changes CLI semantics.
fn looks_like_external_delivery(text: &str) -> bool {
    const DELIVERY_WORDS: &[&str] = &[
        "submit",
        "submission",
        "attempt",
        "scored",
        "acceptance",
        "quota",
        "platform",
    ];
    const DELIVERY_ZH: &[&str] = &["提交", "评分", "配额", "验收", "排行榜", "上分"];
    let lower = text.to_lowercase();
    DELIVERY_ZH.iter().any(|k| text.contains(k))
        || lower
            .split(|c: char| !c.is_alphanumeric())
            .any(|tok| DELIVERY_WORDS.contains(&tok))
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
    let mut deferred_secs = 0u64;
    let mut title = None;
    let mut task_repository = None;
    let mut continuation_policy = None;
    let mut write_scopes = vec![];
    let mut goal_bound = false;
    let mut global_gate = false;
    let mut resume_when_cond = None;
    let mut note = None;
    let mut monitor_target = None;
    let mut monitor_policy = None;
    let mut cadence = None;
    let mut verify: Option<String> = None;
    let mut max_validation_attempts: Option<u32> = None;
    let mut acceptance: Option<String> = None;
    reject_unknown_flags(
        args,
        &[
            "--acceptance",
            "--action-kind",
            "--blocks",
            "--cadence",
            "--class",
            "--continuation-policy",
            "--defer-secs",
            "--gate-question",
            "--global-gate",
            "--goal",
            "--goal-bound",
            "--max-validation-attempts",
            "--monitor-policy",
            "--monitor-target",
            "--note",
            "--priority",
            "--required-write-scope",
            "--resume-when",
            "--role",
            "--task-repository",
            "--text",
            "--title",
            "--verify",
        ],
    )?;
    parse_pairs(args, |k, v| {
        if k == "--goal-bound" {
            goal_bound = true;
        } else if k == "--global-gate" {
            global_gate = true;
        } else if k == "--resume-when" {
            resume_when_cond = Some(v);
        } else if k == "--note" {
            note = Some(v);
        } else if k == "--monitor-target" {
            monitor_target = Some(v);
        } else if k == "--monitor-policy" {
            monitor_policy = Some(v);
        } else if k == "--cadence" {
            cadence = Some(v);
        } else if k == "--verify" {
            verify = Some(v);
        } else if k == "--acceptance" {
            acceptance = Some(v);
        } else if k == "--max-validation-attempts" {
            max_validation_attempts = v.parse().ok();
        } else if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--role" {
            role = v;
        } else if k == "--class" {
            class = v;
        } else if k == "--text" {
            text = Some(v);
        } else if k == "--gate-question" {
            gate_question = Some(v);
        } else if k == "--blocks" {
            blocks = v.split(',').map(|s| s.to_string()).collect();
        } else if k == "--priority" {
            priority = Some(v);
        } else if k == "--action-kind" {
            action_kind = Some(v);
        } else if k == "--defer-secs" {
            deferred_secs = v.parse().unwrap_or(0);
        } else if k == "--title" {
            title = Some(v);
        } else if k == "--task-repository" {
            task_repository = Some(v);
        } else if k == "--continuation-policy" {
            continuation_policy = Some(v);
        } else if k == "--required-write-scope" {
            write_scopes = v.split(',').map(|s| s.trim().to_string()).collect();
        }
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let text = text.ok_or_else(|| anyhow::anyhow!("--text required"))?;
    // Bare `--blocks` at end-of-line is parsed as the literal "true" by
    // parse_pairs' value-less flag convention; no generated todo id is ever
    // "true", so reject it loudly instead of writing a dangling dependency
    // that later fails task-graph with "references unknown todo `true`".
    if blocks.iter().any(|b| b == "true") {
        bail!("--blocks requires a comma-separated todo id list (bare `--blocks` reads as `true`)");
    }
    // O4: advisory hint — code-like todos without a validator can be marked
    // done by an agent even when the code does not compile. Computed before
    // `verify` is moved into the todo below.
    let wants_verify_hint = verify.is_none() && looks_like_code_todo(&text);
    // O5: advisory hint — external-delivery todos (submit/提交 …) whose
    // completion contract is a text convention unless `--acceptance` pins it.
    let wants_acceptance_hint = acceptance.is_none() && looks_like_external_delivery(&text);
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
        // placeholder (text hint only) — and warns about it (P0-3④).
        match parse_resume_when(&rw) {
            ResumeWhen::Defer(secs) => {
                todo.resume_when =
                    Some(std::time::SystemTime::now() + std::time::Duration::from_secs(secs));
            }
            ResumeWhen::TextHint(text) => {
                eprintln!(
                    "{}",
                    resume_when_text_hint_warning(
                        &text,
                        "a 1-hour placeholder deadline is applied"
                    )
                );
                todo.resume_when =
                    Some(std::time::SystemTime::now() + std::time::Duration::from_secs(3600));
            }
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
    if let Some(v) = verify {
        todo.validator = Some(v);
    }
    if let Some(a) = acceptance {
        todo.acceptance = Some(a);
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
    store.append(Event::TodoAdded {
        goal_id: goal_id.clone(),
        todo,
        ts: now_epoch(),
    })?;
    refresh_next_action(store, &goal_id)?;
    sync_compat(store, &goal_id)?;
    println!("todo {id} added to {goal_id} ✔");
    // O4: pure reminder after a successful add; no semantic change.
    if wants_verify_hint {
        eprintln!("hint: 实现类 todo 建议挂 --verify \"cargo check -p ...\"，防不编译代码被标完成");
    }
    // O5: advisory hint for external-delivery todos (submit/提交 …) — the
    // completion contract is a text convention unless `--acceptance` pins it.
    if wants_acceptance_hint {
        eprintln!(
            "hint: 外部交付类 todo 建议挂 --acceptance \"attempt,scored\"（关单时 evidence 必须包含全部子串，防空交付）"
        );
    }
    Ok(())
}

fn todo_claim(store: &mut Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut todo_id = None;
    let mut agent_id = None;
    let mut lease_secs = 3600u64;
    let mut force = false;
    reject_unknown_flags(
        args,
        &[
            "--agent-id",
            "--force",
            "--goal",
            "--lease-secs",
            "--todo-id",
        ],
    )?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--todo-id" {
            todo_id = Some(v);
        } else if k == "--agent-id" {
            agent_id = Some(v);
        } else if k == "--lease-secs" {
            lease_secs = v.parse().unwrap_or(3600);
        } else if k == "--force" {
            force = true;
        }
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
    // P0-1 workspace guard: refuse (degrade to serial) when a peer holds a
    // live lease in an overlapping declared workspace, unless --force.
    let conflicts = crate::agents::workspace_guard::live_workspace_conflicts(&goal, &agent, now);
    if !conflicts.is_empty() && !force {
        bail!(
            "workspace conflict — claiming would race a peer writing the same workspace:\n{}\
             degrade to serial: retry after the holder's lease expires, or pass --force",
            crate::agents::workspace_guard::render_conflicts(&conflicts, now)
        );
    }
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
        holder_pid: Some(std::process::id()),
        ts: now,
    })?;
    append_workspace_lock(store, &goal_id, &agent, &todo_id, &goal, force)?;
    refresh_next_action(store, &goal_id)?;
    sync_compat(store, &goal_id)?;
    println!("todo {todo_id} claimed by {agent} until epoch {expires} ✔");
    Ok(())
}

/// P0-1: append the advisory write-lock record after a successful claim by
/// a workspace-declaring agent (empty declared set → no record, the guard
/// is fail-open). `goal` must be the pre-claim replay carrying the
/// claimer's profile; `forced` marks a claim that overrode a conflict.
fn append_workspace_lock(
    store: &mut Store,
    goal_id: &str,
    agent_id: &str,
    todo_id: &str,
    goal: &Goal,
    forced: bool,
) -> Result<()> {
    let paths = crate::agents::workspace_guard::agent_workspaces(goal, agent_id);
    if paths.is_empty() {
        return Ok(());
    }
    store.append(Event::WorkspaceLockAcquired {
        goal_id: goal_id.to_string(),
        agent_id: agent_id.to_string(),
        todo_id: todo_id.to_string(),
        paths,
        forced,
        ts: crate::state::now_epoch(),
    })?;
    Ok(())
}

/// Parse a `--workspace` flag value into normalized absolute paths
/// (comma-separated; empty entries dropped).
fn parse_workspaces(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(crate::agents::workspace_guard::normalize_workspace_path)
        .collect()
}

/// `loopx agent register --goal G --agent-id A [--workspace p1,p2]` —
/// register a peer (LoopX: coordination.registered_agents; precondition for
/// quota --agent-id). `--workspace` declares the P0-1 guard write set.
fn cmd_agent(store: &mut Store, args: &[String]) -> Result<()> {
    match args.first().map(|s| s.as_str()) {
        Some("onboard") => return cmd_agent_onboard(store, &args[1..]),
        Some("list") => return cmd_agent_list(store, &args[1..]),
        Some("contract") => return cmd_agent_contract(store, &args[1..]),
        Some("recipe") => return cmd_agent_recipe(store, &args[1..]),
        Some("succession") => return cmd_agent_succession(store, &args[1..]),
        Some("collective") => return cmd_agent_collective(store, &args[1..]),
        _ => {}
    }
    let mut goal_id = None;
    let mut agent_id = None;
    let mut workspaces = vec![];
    reject_unknown_flags(args, &["--agent-id", "--goal", "--workspace"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--agent-id" {
            agent_id = Some(v);
        } else if k == "--workspace" {
            workspaces = parse_workspaces(&v);
        }
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let agent_id = agent_id.ok_or_else(|| anyhow::anyhow!("--agent-id required"))?;
    store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    store.append(Event::AgentRegistered {
        goal_id: goal_id.clone(),
        agent_id: agent_id.clone(),
        workspaces: workspaces.clone(),
        ts: crate::state::now_epoch(),
    })?;
    if workspaces.is_empty() {
        println!("agent `{agent_id}` registered for {goal_id} ✔");
    } else {
        println!("agent `{agent_id}` registered for {goal_id} (workspaces={workspaces:?}) ✔");
    }
    Ok(())
}

/// `loopx agent onboard --goal G --agent-id A [--workspace p1,p2]
/// [--recipe NAME]` — register a peer and declare the P0-1 workspace-guard
/// write set. `--recipe NAME` applies a recorded G12 agent recipe
/// (capabilities + workspaces + default priority, capabilities kept as
/// descriptive metadata) — when given, an explicit `--workspace` flag is
/// rejected (the recipe is the single source).
fn cmd_agent_onboard(store: &mut Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut agent_id = None;
    let mut workspaces = vec![];
    let mut recipe_name = None;
    reject_unknown_flags(args, &["--agent-id", "--goal", "--recipe", "--workspace"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--agent-id" {
            agent_id = Some(v);
        } else if k == "--workspace" {
            workspaces = parse_workspaces(&v);
        } else if k == "--recipe" {
            recipe_name = Some(v);
        }
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let agent_id = agent_id.ok_or_else(|| anyhow::anyhow!("--agent-id required"))?;
    store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    if let Some(name) = recipe_name {
        if !workspaces.is_empty() {
            bail!(
                "--recipe {name} conflicts with an explicit --workspace flag \
                 (the recipe owns the onboarding profile)"
            );
        }
        let recipe = crate::agents::multi_agent::recipe_named(store, &goal_id, &name)?
            .ok_or_else(|| {
                anyhow::anyhow!("no agent recipe named `{name}` for {goal_id} (add one first: agent recipe add)")
            })?;
        crate::agents::multi_agent::apply_recipe_onboard(store, &goal_id, &agent_id, &recipe)?;
        println!(
            "agent `{agent_id}` onboarded via recipe `{name}` \
             (capabilities={:?} workspaces={:?} priority={}) ✔",
            recipe.capabilities, recipe.workspaces, recipe.priority
        );
        return Ok(());
    }
    store.append(Event::AgentOnboarded {
        goal_id: goal_id.clone(),
        agent_id: agent_id.clone(),
        capabilities: vec![],
        workspaces: workspaces.clone(),
        ts: crate::state::now_epoch(),
    })?;
    println!("agent `{agent_id}` onboarded (capabilities=[] workspaces={workspaces:?}) ✔");
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
    reject_unknown_flags(args, &["--format", "--goal", "--json"])?;
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
    let rows = agent_list_rows(&goal, &last_active, now);
    if wants_json(args) {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    println!(
        "agents registered for {goal_id} ({}):",
        goal.registered_agents.len()
    );
    println!(
        "  {:<12} {:<8} {:<28} {:<24} {:<14} {:<12}",
        "agent_id", "status", "work-on", "workspaces", "capabilities", "last-active"
    );
    for row in &rows {
        let work_label = if row.work_on.is_empty() {
            "-".to_string()
        } else {
            row.work_on.join("; ")
        };
        let ws_label = if row.workspaces.is_empty() {
            "-".to_string()
        } else {
            row.workspaces.join(",")
        };
        let caps = if row.capabilities.is_empty() {
            "-".to_string()
        } else {
            row.capabilities.join(",")
        };
        let last = row
            .last_active_ts
            .map(|ts| format!("{} ago", human_dur(now.saturating_sub(ts))))
            .unwrap_or_else(|| "-".to_string());
        println!(
            "  {:<12} {:<8} {:<28} {:<24} {:<14} {:<12}",
            row.agent_id, row.status, work_label, ws_label, caps, last
        );
    }
    // P0-1: live workspace conflicts — who occupies the paths you declared.
    let mut any_conflict = false;
    for row in &rows {
        let conflicts =
            crate::agents::workspace_guard::live_workspace_conflicts(&goal, &row.agent_id, now);
        for c in &conflicts {
            any_conflict = true;
            println!(
                "⚠ workspace conflict: {} ↔ {} share {} (holder lease expires in {})",
                row.agent_id,
                c.holder_agent_id,
                c.overlapping_paths.join(", "),
                human_dur(c.holder_lease_expires_at.saturating_sub(now))
            );
        }
    }
    if any_conflict {
        println!("hint: conflicting claims need `--force` — or wait for the holder's lease");
    }
    println!(
        "hint: agent ids are goal-scoped; check this list before `agent register`/`onboard` \
         to avoid duplicate ids (each parallel worker needs its own unique id)"
    );
    Ok(())
}

/// One row of the `agent list` projection (P0-3③: serializable so the
/// command has a `--format json` form; also keeps the text table testable).
#[derive(Debug, Clone, serde::Serialize)]
struct AgentListRow {
    agent_id: String,
    /// "running" = holds a live lease; "idle" = registered, no live lease.
    status: String,
    /// Human-readable live lease labels (todo id + remaining time).
    work_on: Vec<String>,
    /// P0-1 declared workspace write set (display-shortened; a `✍` suffix
    /// marks paths the agent currently occupies under a live lease).
    workspaces: Vec<String>,
    capabilities: Vec<String>,
    last_active_ts: Option<u64>,
}

/// Shorten a workspace path for the agent-list table: `$HOME` → `~`.
fn shorten_home(path: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() && path.starts_with(&home) {
        return format!("~{}", &path[home.len()..]);
    }
    path.to_string()
}

/// Build the agent-list projection rows (event-derived last-active map +
/// live lease scan). Pure, unit-testable.
fn agent_list_rows(goal: &Goal, last_active: &HashMap<String, u64>, now: u64) -> Vec<AgentListRow> {
    goal.registered_agents
        .iter()
        .map(|aid| {
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
            let occupying = !work.is_empty();
            let profile = goal.agent_profiles.iter().find(|p| p.id == *aid);
            let caps = profile.map(|p| p.capabilities.clone()).unwrap_or_default();
            let workspaces = profile
                .map(|p| {
                    p.workspaces
                        .iter()
                        .map(|w| {
                            let short = shorten_home(w);
                            if occupying {
                                format!("{short} ✍")
                            } else {
                                short
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            AgentListRow {
                agent_id: aid.clone(),
                status: status.to_string(),
                work_on: work,
                workspaces,
                capabilities: caps,
                last_active_ts: last_active.get(aid).copied(),
            }
        })
        .collect()
}

/// `agent contract set --goal G --contract '<json>' | --contract-file PATH`
/// and `agent contract show --goal G [--format json]` — the G12 multi-agent
/// topology surface. Set validates fail-closed before appending
/// (`MultiAgentContractSet`, latest event wins); show projects the current
/// contract plus its validation issues (a drifted on-disk contract that was
/// never re-validated would surface here).
fn cmd_agent_contract(store: &mut Store, args: &[String]) -> Result<()> {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("");
    match sub {
        "set" => {
            let mut goal_id = None;
            let mut inline = None;
            let mut file = None;
            reject_unknown_flags(&args[1..], &["--contract", "--contract-file", "--goal"])?;
            parse_pairs(&args[1..], |k, v| match k {
                "--goal" => goal_id = Some(v),
                "--contract" => inline = Some(v),
                "--contract-file" => file = Some(v),
                _ => {}
            });
            let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
            let raw = match (inline, file) {
                (Some(v), _) => v,
                (None, Some(path)) => std::fs::read_to_string(&path)
                    .with_context(|| format!("read contract file {path}"))?,
                (None, None) => {
                    bail!("contract required: --contract '<json>' or --contract-file PATH")
                }
            };
            let contract: crate::agents::multi_agent::MultiAgentContract =
                serde_json::from_str(&raw).context("parse contract JSON")?;
            store
                .replay(&goal_id)?
                .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
            let event_id = crate::agents::multi_agent::record_contract(store, &goal_id, &contract)?;
            println!(
                "multi-agent contract set for {goal_id}: {} peer(s), {} handoff rule(s), {} collective(s) (event {event_id}) ✔",
                contract.peers.len(),
                contract.handoff_rules.len(),
                contract.collectives.len()
            );
        }
        "show" => {
            let mut goal_id = None;
            reject_unknown_flags(&args[1..], &["--format", "--goal", "--json"])?;
            parse_pairs(&args[1..], |k, v| {
                if k == "--goal" {
                    goal_id = Some(v);
                }
            });
            let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
            store
                .replay(&goal_id)?
                .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
            match crate::agents::multi_agent::latest_contract(store, &goal_id)? {
                None => println!("no multi-agent contract set for {goal_id}"),
                Some(contract) => {
                    let issues = crate::agents::multi_agent::contract_issues(&contract);
                    if wants_json(&args[1..]) {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&serde_json::json!({
                                "ok": true,
                                "goal_id": goal_id,
                                "contract": contract,
                                "validation_issues": issues,
                            }))?
                        );
                    } else {
                        println!(
                            "multi-agent contract for {goal_id}: {} peer(s), {} handoff rule(s), {} collective(s){}",
                            contract.peers.len(),
                            contract.handoff_rules.len(),
                            contract.collectives.len(),
                            if issues.is_empty() {
                                String::new()
                            } else {
                                format!(" — ⚠ validation issues: {}", issues.join("; "))
                            }
                        );
                        for (id, role) in &contract.peers {
                            let backup = role
                                .backup_for
                                .as_deref()
                                .map(|b| format!(" backups {b}"))
                                .unwrap_or_default();
                            let caps = if role.capabilities.is_empty() {
                                "-".to_string()
                            } else {
                                role.capabilities.join(",")
                            };
                            let ws = if role.workspaces.is_empty() {
                                "-".to_string()
                            } else {
                                role.workspaces.join(",")
                            };
                            println!("  peer {id}{backup} capabilities={caps} workspaces={ws}");
                        }
                        for rule in &contract.handoff_rules {
                            println!("  handoff: {} → {}", rule.from_event, rule.to_role);
                        }
                        for (name, members) in &contract.collectives {
                            println!("  collective {name}: {}", members.join(","));
                        }
                    }
                }
            }
        }
        other => bail!("unknown agent contract subcommand `{other}` (set|show)"),
    }
    Ok(())
}

/// `agent recipe add --goal G --name N [--capabilities c1,c2] [--workspace p]
/// [--priority P0]` and `agent recipe show --goal G [--name N] [--format json]`
/// — the G12 named-recipe surface consumed by `agent onboard --recipe N`.
fn cmd_agent_recipe(store: &mut Store, args: &[String]) -> Result<()> {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("");
    match sub {
        "add" => {
            let mut goal_id = None;
            let mut name = None;
            let mut capabilities = vec![];
            let mut workspaces = vec![];
            let mut priority = crate::state::Priority::P1;
            reject_unknown_flags(
                &args[1..],
                &[
                    "--capabilities",
                    "--capability",
                    "--goal",
                    "--name",
                    "--priority",
                    "--workspace",
                ],
            )?;
            parse_pairs(&args[1..], |k, v| match k {
                "--goal" => goal_id = Some(v),
                "--name" => name = Some(v),
                "--capability" | "--capabilities" => {
                    capabilities = v.split(',').map(|s| s.trim().to_string()).collect()
                }
                "--workspace" => workspaces = parse_workspaces(&v),
                "--priority" => {
                    priority = match v.to_uppercase().as_str() {
                        "P0" => crate::state::Priority::P0,
                        "P2" => crate::state::Priority::P2,
                        _ => crate::state::Priority::P1,
                    }
                }
                _ => {}
            });
            let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
            let name = name.ok_or_else(|| anyhow::anyhow!("--name required"))?;
            store
                .replay(&goal_id)?
                .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
            let recipe = crate::agents::multi_agent::AgentRecipe {
                schema_version: crate::agents::multi_agent::MULTI_AGENT_RECIPE_SCHEMA_VERSION
                    .to_string(),
                name: name.clone(),
                capabilities: capabilities.clone(),
                workspaces: workspaces.clone(),
                priority,
            };
            let event_id = crate::agents::multi_agent::record_recipe(store, &goal_id, &recipe)?;
            println!(
                "agent recipe `{name}` added for {goal_id} \
                 (capabilities={capabilities:?} workspaces={workspaces:?} priority={priority}) \
                 (event {event_id}) ✔"
            );
        }
        "show" => {
            let mut goal_id = None;
            let mut name = None;
            reject_unknown_flags(&args[1..], &["--format", "--goal", "--json", "--name"])?;
            parse_pairs(&args[1..], |k, v| match k {
                "--goal" => goal_id = Some(v),
                "--name" => name = Some(v),
                _ => {}
            });
            let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
            let recipes = crate::agents::multi_agent::recipes(store, &goal_id)?;
            let shown: Vec<_> = recipes
                .iter()
                .filter(|r| name.as_deref().is_none_or(|n| r.name == n))
                .collect();
            if wants_json(&args[1..]) {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "ok": true,
                        "goal_id": goal_id,
                        "recipe_count": shown.len(),
                        "recipes": shown,
                    }))?
                );
            } else {
                if shown.is_empty() {
                    let label = name.map(|n| format!(" named `{n}`")).unwrap_or_default();
                    println!("no agent recipes{label} for {goal_id}");
                } else {
                    println!("agent recipes for {goal_id} ({}):", shown.len());
                    for r in &shown {
                        println!(
                            "  {:<20} priority={:<3} capabilities={} workspaces={}",
                            r.name,
                            r.priority,
                            if r.capabilities.is_empty() {
                                "-".to_string()
                            } else {
                                r.capabilities.join(",")
                            },
                            if r.workspaces.is_empty() {
                                "-".to_string()
                            } else {
                                r.workspaces.join(",")
                            }
                        );
                    }
                }
            }
        }
        other => bail!("unknown agent recipe subcommand `{other}` (add|show)"),
    }
    Ok(())
}

/// `agent succession show|apply --goal G ...` — the G12 role-succession
/// surface. `show` projects recorded successions plus the currently-met
/// (unrecorded) triggers; `apply` records them (`SuccessionOccurred`, one
/// per trigger episode — idempotent).
fn cmd_agent_succession(store: &mut Store, args: &[String]) -> Result<()> {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("");
    let mut goal_id = None;
    let mut primary = None;
    let mut reason = None;
    reject_unknown_flags(
        &args[1..],
        &["--format", "--goal", "--json", "--primary", "--reason"],
    )?;
    parse_pairs(&args[1..], |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--primary" => primary = Some(v),
        "--reason" => reason = Some(v),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let contract =
        crate::agents::multi_agent::latest_contract(store, &goal_id)?.ok_or_else(|| {
            anyhow::anyhow!(
                "no multi-agent contract set for {goal_id} (set one first: agent contract set)"
            )
        })?;
    let now = crate::state::now_epoch();
    let mut candidates = crate::agents::multi_agent::succession_candidates(&goal, &contract, now);
    if let Some(p) = &primary {
        candidates.retain(|c| &c.primary == p);
    }
    if let Some(r) = &reason {
        candidates.retain(|c| &c.reason == r);
    }
    let recorded = crate::agents::multi_agent::successions(store, &goal_id)?;
    match sub {
        "show" => {
            if wants_json(&args[1..]) {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "ok": true,
                        "schema_version": crate::agents::multi_agent::ROLE_SUCCESSOR_PROJECTION_SCHEMA_VERSION,
                        "goal_id": goal_id,
                        "offline_threshold_secs": crate::agents::multi_agent::successor_offline_threshold_secs(),
                        "candidates": candidates,
                        "recorded": recorded,
                    }))?
                );
            } else {
                println!("role succession for {goal_id}:");
                if recorded.is_empty() && candidates.is_empty() {
                    println!("  no succession triggers met, no successions recorded");
                }
                for r in &recorded {
                    println!(
                        "  recorded: primary `{}` → backup `{}` ({}) [event {}]",
                        r.primary, r.backup, r.reason, r.event_id
                    );
                }
                for c in &candidates {
                    println!(
                        "  pending: primary `{}` → backup `{}` ({}) — run `agent succession apply` to record",
                        c.primary, c.backup, c.reason
                    );
                }
            }
        }
        "apply" => {
            if candidates.is_empty() {
                println!(
                    "no succession triggers met for {goal_id}{}",
                    primary
                        .map(|p| format!(" (primary {p})"))
                        .unwrap_or_default()
                );
                return Ok(());
            }
            for candidate in &candidates {
                let already = recorded.iter().any(|r| {
                    r.primary == candidate.primary
                        && r.backup == candidate.backup
                        && r.reason == candidate.reason
                });
                let event_id =
                    crate::agents::multi_agent::record_succession(store, &goal_id, candidate)?;
                println!(
                    "succession {}: primary `{}` → backup `{}` ({}) (event {event_id}) ✔",
                    if already {
                        "already recorded"
                    } else {
                        "recorded"
                    },
                    candidate.primary,
                    candidate.backup,
                    candidate.reason
                );
            }
        }
        other => bail!("unknown agent succession subcommand `{other}` (show|apply)"),
    }
    Ok(())
}

/// `agent collective show --goal G [--collective NAME] [--format json]` —
/// the G12 collective projection: per-agent turn counts (claims) plus the
/// round-robin wake roster for the next collective turn.
fn cmd_agent_collective(store: &Store, args: &[String]) -> Result<()> {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("");
    if sub != "show" {
        bail!("unknown agent collective subcommand `{sub}` (show)");
    }
    let mut goal_id = None;
    let mut collective = None;
    reject_unknown_flags(
        &args[1..],
        &["--collective", "--format", "--goal", "--json"],
    )?;
    parse_pairs(&args[1..], |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--collective" => collective = Some(v),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let contract =
        crate::agents::multi_agent::latest_contract(store, &goal_id)?.ok_or_else(|| {
            anyhow::anyhow!(
                "no multi-agent contract set for {goal_id} (set one first: agent contract set)"
            )
        })?;
    let names: Vec<String> = match &collective {
        Some(name) => {
            if !contract.collectives.contains_key(name) {
                bail!("collective `{name}` is not part of the contract for {goal_id}");
            }
            vec![name.clone()]
        }
        None => contract.collectives.keys().cloned().collect(),
    };
    let mut ledgers = vec![];
    for name in &names {
        if let Some(ledger) =
            crate::agents::multi_agent::collective_turn_ledger(store, &goal_id, &contract, name)?
        {
            let roster = crate::agents::multi_agent::wake_roster(
                &contract,
                name,
                ledger.full_participation_rounds,
            );
            ledgers.push(serde_json::json!({
                "collective": name,
                "agents": ledger.agents,
                "per_agent": ledger.per_agent,
                "full_participation_rounds": ledger.full_participation_rounds,
                "total_claims": ledger.total_claims,
                "wake_roster": roster,
            }));
        }
    }
    if wants_json(&args[1..]) {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "ok": true,
                "schema_version": crate::agents::multi_agent::COLLECTIVE_TURN_LEDGER_SCHEMA_VERSION,
                "goal_id": goal_id,
                "collective_count": ledgers.len(),
                "collectives": ledgers,
            }))?
        );
    } else {
        if ledgers.is_empty() {
            println!("no collectives in the contract for {goal_id}");
            return Ok(());
        }
        for entry in &ledgers {
            let name = entry["collective"].as_str().unwrap_or("");
            let agents = entry["agents"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            let rounds = entry["full_participation_rounds"].as_u64().unwrap_or(0);
            let total = entry["total_claims"].as_u64().unwrap_or(0);
            println!(
                "collective `{name}`: agents={agents} full_participation_rounds={rounds} total_claims={total}"
            );
            if let Some(per) = entry["per_agent"].as_object() {
                for (agent, row) in per {
                    let turns = row["turns"].as_u64().unwrap_or(0);
                    let last = row["last_turn_ts"]
                        .as_u64()
                        .map(|ts| format!(" (last {ts})"))
                        .unwrap_or_default();
                    println!("    {agent}: {turns} turn(s){last}");
                }
            }
            if let Some(roster) = entry["wake_roster"].as_object() {
                let current = roster["current"].as_str().unwrap_or("-");
                let order = roster["order"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str())
                            .collect::<Vec<_>>()
                            .join(" → ")
                    })
                    .unwrap_or_default();
                println!("    next wake: {current} (roster: {order})");
            }
        }
    }
    Ok(())
}

/// P0-3③: JSON projection of one todo's lease state
/// (`lease status --format json`). Pure, unit-testable.
fn lease_status_json(
    todo_id: &str,
    status: &crate::work_items::task_lease::LeaseStatus,
) -> serde_json::Value {
    use crate::work_items::task_lease::LeaseStatus;
    match status {
        LeaseStatus::Free => serde_json::json!({"todo_id": todo_id, "lease": "free"}),
        LeaseStatus::Active { owner, expires_at } => serde_json::json!({
            "todo_id": todo_id, "lease": "active", "owner": owner, "expires_at": expires_at,
        }),
        LeaseStatus::Expired { owner, expires_at } => serde_json::json!({
            "todo_id": todo_id, "lease": "expired", "owner": owner, "expired_at": expires_at,
        }),
    }
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
    let mut force = false;
    reject_unknown_flags(
        args,
        &[
            "--evidence",
            "--force",
            "--goal",
            "--no-follow-up",
            "--successor",
            "--todo-id",
        ],
    )?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--todo-id" {
            todo_id = Some(v);
        } else if k == "--no-follow-up" {
            no_follow_up = true;
        } else if k == "--force" {
            force = true;
        } else if k == "--successor" {
            successor = Some(v);
        } else if k == "--evidence" {
            evidence = Some(v);
        }
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
    let is_advancement = t.class == TaskClass::Advancement;
    // O6: completion evidence contract (retrospective: 11/33 completions
    // shipped <60-char evidence, several fully empty, and every one of those
    // todos had to be reopened by hand). Advancement todos must carry real,
    // non-empty evidence of what landed — `--force` is the explicit override
    // for mechanical closeouts the operator owns.
    let evidence_trim = evidence.as_deref().map(str::trim).unwrap_or("");
    if is_advancement && evidence_trim.is_empty() && !force {
        bail!(
            "todo {todo_id} needs non-empty --evidence (what actually landed: attempt ids, \
             paths, outputs, measurements). Add --force only for an explicit operator closeout."
        );
    }
    // O7: acceptance contract (`todo add --acceptance "a,b"`) — evidence must
    // contain every declared token (case-insensitive) before completion is
    // accepted; `--force` overrides. Turns the ACCEPTANCE text convention
    // (e.g. a platform attempt id) into a hard check.
    if is_advancement {
        if let Some(acc) = t.acceptance.as_deref() {
            let tokens: Vec<&str> = acc
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            let lower = evidence_trim.to_lowercase();
            let missing: Vec<&str> = tokens
                .iter()
                .filter(|tok| !lower.contains(&tok.to_lowercase()))
                .copied()
                .collect();
            if !missing.is_empty() && !force {
                bail!(
                    "todo {todo_id} acceptance contract unmet: evidence must contain [{}] \
                     (missing: [{}]). Declared via `todo add --acceptance`; --force overrides.",
                    acc,
                    missing.join(", ")
                );
            }
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
    // P0-2①: a completed advancement todo is a delivery pending verification
    // ("delivered ≠ succeeded") — record the outcome signal so it can be
    // resolved (verified/failed/rework) and aged by the follow-through scan.
    if is_advancement {
        let delivered_turn = goal.history.iter().map(|r| r.turn).max().unwrap_or(0);
        let seq = goal
            .delivery_state(&todo_id)
            .map(|d| d.seq + 1)
            .unwrap_or(1);
        store.append(Event::DeliveryOutcomeRecorded {
            goal_id: goal_id.clone(),
            todo_id: todo_id.clone(),
            outcome: crate::work_items::delivery_outcome::OUTCOME_DELIVERED.to_string(),
            note: None,
            delivered_turn,
            seq,
            ts: now_epoch(),
        })?;
    }
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
    reject_unknown_flags(args, &["--decision", "--goal", "--note", "--todo-id"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--todo-id" {
            todo_id = Some(v);
        } else if k == "--decision" {
            decision = Some(v);
        } else if k == "--note" {
            note = Some(v);
        }
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
    reject_unknown_flags(args, &["--goal", "--list", "--restore"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--list" {
            list = true;
        } else if k == "--restore" {
            restore = Some(v);
        }
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
    reject_unknown_flags(args, &["--goal", "--require-approval", "--write-scope"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--write-scope" {
            write_scope = Some(v);
        } else if k == "--require-approval" {
            require = Some(v);
        }
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
/// `loopx replan rules show|set --goal G [--rule-ids R1,R2,...]` inspects or
/// updates the goal's replan rule set (G13 ②).
fn cmd_replan(store: &mut Store, args: &[String]) -> Result<()> {
    if args.first().map(|s| s.as_str()) == Some("rules") {
        return cmd_replan_rules(store, &args[1..]);
    }
    if args.first().map(|s| s.as_str()) == Some("obligations") {
        let mut goal_id = None;
        reject_unknown_flags(
            &args[1..],
            &["--delta-kind", "--format", "--goal", "--json"],
        )?;
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
        if wants_json(&args[1..]) {
            println!("{}", serde_json::to_string_pretty(&obligations)?);
            return Ok(());
        }
        if obligations.is_empty() {
            println!("no unfulfilled replan obligations for {goal_id}");
            return Ok(());
        }
        println!("unfulfilled replan obligations ({goal_id}):");
        for obligation in &obligations {
            print_obligation(obligation);
        }
        return Ok(());
    }
    let mut goal_id = None;
    let mut delta_kinds: Vec<String> = vec![];
    reject_unknown_flags(args, &["--delta-kind", "--format", "--goal", "--json"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--delta-kind" {
            delta_kinds.push(v);
        }
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

/// `loopx replan rules show --goal G [--format json]` — the goal's active
/// replan rule set, the ordered policy table with per-rule match status,
/// and the selected rule decision (disposition → replan decision +
/// obligation).
/// `loopx replan rules set --goal G [--rule-ids R1,R2,...]` — declare an
/// explicit rule set (full replace; `--rule-ids ""` resets to the default
/// set) via a `ReplanRuleSetUpdated` event.
fn cmd_replan_rules(store: &mut Store, args: &[String]) -> Result<()> {
    let sub = args.first().map(|s| s.as_str());
    match sub {
        Some("show") => {}
        Some("set") => {}
        _ => bail!("replan rules subcommand must be `show` or `set`"),
    }
    let mut goal_id = None;
    let mut rule_ids: Option<String> = None;
    reject_unknown_flags(&args[1..], &["--format", "--goal", "--json", "--rule-ids"])?;
    parse_pairs(&args[1..], |k, v| match k {
        "--goal" => goal_id = Some(v),
        "--rule-ids" => rule_ids = Some(v),
        _ => {}
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    if sub == Some("set") {
        let ids: Vec<String> = match rule_ids {
            Some(raw) => {
                if raw.trim().is_empty() {
                    vec![]
                } else {
                    raw.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                }
            }
            None => vec![],
        };
        // Validate against the builtin vocabulary up front (unknown ids are
        // allowed — selection skips them — but typos deserve a warning).
        for id in &ids {
            if !crate::decision::goal_frontier::replan_rules::is_known_rule(id) {
                println!("warning: unknown rule id `{id}` — selection will skip it");
            }
        }
        store
            .replay(&goal_id)?
            .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
        store.append(Event::ReplanRuleSetUpdated {
            goal_id: goal_id.clone(),
            rule_set_version:
                crate::decision::goal_frontier::replan_rules::DEFAULT_REPLAN_RULE_SET_VERSION
                    .to_string(),
            rule_ids: ids.clone(),
            ts: crate::state::now_epoch(),
        })?;
        if ids.is_empty() {
            println!("replan rule set reset to the default set ✔");
        } else {
            println!("replan rule set updated (rule_ids={ids:?}) ✔");
        }
        return Ok(());
    }
    // show
    let goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let active = crate::decision::goal_frontier::replan_rules::active_rule_set(&goal);
    let facts = crate::decision::goal_frontier::replan_rules::facts_for_goal(&goal);
    let decision = crate::decision::goal_frontier::replan_rules::select_replan_rule(&goal);
    if wants_json(&args[1..]) {
        let table: Vec<serde_json::Value> = active
            .effective_rule_ids()
            .iter()
            .map(|id| {
                serde_json::json!({
                    "rule": id,
                    "selected": id == &decision.rule,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "schema_version": crate::decision::goal_frontier::replan_rules::DEFAULT_REPLAN_RULE_SET_VERSION,
                "goal_id": goal_id,
                "rule_set": active,
                "table": table,
                "facts": {
                    "existing_replan_required": facts.existing_replan_required,
                    "blocking_user_open_count": facts.blocking_user_open_count,
                    "succession_gap_count": facts.succession_gap_count,
                    "acceptance_gap_count": facts.acceptance_gap_count,
                    "selectable_frontier_advancement": facts.selectable_frontier_advancement,
                    "monitor_count": facts.monitor_count,
                    "monitor_no_change_streak_triggered": facts.monitor_no_change_streak_triggered,
                    "monitor_only_lane": facts.monitor_only_lane,
                },
                "decision": decision,
            }))?
        );
        return Ok(());
    }
    println!(
        "replan rule set ({goal_id}) — version {}:",
        active.schema_version
    );
    for id in active.effective_rule_ids() {
        let mark = if id == decision.rule {
            "◀ selected"
        } else {
            ""
        };
        println!("  {id} {mark}");
    }
    println!(
        "decision: rule={} derives_obligation={} obligation={} reason=\"{}\"",
        decision.rule,
        decision.derives_obligation,
        decision.obligation_kind.as_deref().unwrap_or("-"),
        decision.reason
    );
    Ok(())
}

// ── frontier (G13) ────────────────────────────────────────────────────────

/// `loopx frontier show --goal G [--format json]` — the composed goal
/// frontier: the existing frontier projection plus the four G13 layers
/// (outcome segments / replan rule decision / terminal judgement /
/// semantic history).
fn cmd_frontier(store: &Store, args: &[String]) -> Result<()> {
    if args.first().map(|s| s.as_str()) != Some("show") {
        bail!(
            "frontier subcommand must be `show` (try `{} frontier show --help`)",
            prog()
        );
    }
    let mut goal_id = None;
    reject_unknown_flags(&args[1..], &["--format", "--goal", "--json"])?;
    parse_pairs(&args[1..], |k, v| {
        if k == "--goal" {
            goal_id = Some(v)
        }
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let show = crate::decision::goal_frontier::frontier_show(&goal);
    if wants_json(&args[1..]) {
        println!("{}", serde_json::to_string_pretty(&show)?);
        return Ok(());
    }
    let fp = &show.frontier_projection;
    println!("goal frontier ({goal_id}):");
    println!(
        "  lane={} replan_required={} unclaimed_advancement={} acceptance_gaps={} monitors_open={} monitors_due={}",
        show.lane,
        fp.replan_required,
        fp.unclaimed_advancement,
        fp.acceptance_gaps,
        fp.monitors_open,
        fp.monitors_due
    );
    if show.outcome_segments.is_empty() {
        println!("  outcome_segments: (no runs yet)");
    } else {
        let segments = show
            .outcome_segments
            .iter()
            .map(|s| format!("{} [{} ×{}]", s.segment_id, s.kind, s.length))
            .collect::<Vec<_>>()
            .join(", ");
        println!("  outcome_segments: {segments}");
    }
    println!(
        "  replan rule: {} (derives_obligation={}{})",
        show.replan_rule.rule,
        show.replan_rule.derives_obligation,
        show.replan_rule
            .obligation_kind
            .as_deref()
            .map(|k| format!(", obligation={k}"))
            .unwrap_or_default()
    );
    if show.terminal_judgement.terminal {
        println!("  terminal: yes (kind=no_followup, source=validated_goal_closure)");
    } else {
        println!(
            "  terminal: no ({} gap(s)):",
            show.terminal_judgement.gaps.len()
        );
        for gap in &show.terminal_judgement.gaps {
            println!("    - [{}] {}", gap.kind, gap.description);
        }
    }
    let recent: Vec<&crate::decision::goal_frontier::semantic_history::SemanticEvent> =
        show.semantic_history.iter().rev().take(5).collect();
    println!("  semantic_history (last {}):", recent.len());
    for e in recent.iter().rev() {
        println!("    {} {} — {}", e.ts, e.kind, e.summary);
    }
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
    reject_unknown_flags(&args[1..], &["--goal", "--outcome-floor"])?;
    parse_pairs(&args[1..], |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--outcome-floor" {
            outcome_floor = Some(v);
        }
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
    reject_unknown_flags(args, &["--format", "--goal"])?;
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
        print_ledger_read_note(store, &g);
        return Ok(());
    }
    if store.registry().is_empty() {
        println!("no goals registered (root {})", root_dir());
        return Ok(());
    }
    for entry in store.registry() {
        if let Ok(Some(goal)) = store.replay(&entry.goal_id) {
            print_goal_status(&goal);
            print_ledger_read_note(store, &entry.goal_id);
            println!();
        }
    }
    Ok(())
}

/// O1: surface the ledger read diagnostics note (unknown-kind lines skipped
/// because this binary is older than the ledger) below a goal's status.
fn print_ledger_read_note(store: &Store, goal_id: &str) {
    if let Some(note) = store.ledger_read_diagnostics(goal_id).and_then(|d| {
        d.get("note")
            .and_then(|n| n.as_str())
            .map(|s| s.to_string())
    }) {
        println!("note      : {note}");
    }
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
            "ledger_read_diagnostics": store.ledger_read_diagnostics(&gid),
            "turn_no_progress": goal.turn_no_progress.iter().map(|np| serde_json::json!({
                "todo_id": np.todo_id,
                "agent_id": np.agent_id,
                "idle_secs": np.idle_secs,
                "tool_calls_total": np.tool_calls_total,
                "ts": np.ts,
            })).collect::<Vec<_>>(),
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
    // O3: idle-turn no-progress breaches (recent last) — the orchestrator's
    // signal to nudge via `todo update` steering.
    for np in goal.turn_no_progress.iter().rev().take(3) {
        println!(
            "no-progress: turn todo={} agent={} idle={}s tools={} ts={}",
            np.todo_id,
            np.agent_id.as_deref().unwrap_or("anonymous"),
            np.idle_secs,
            np.tool_calls_total,
            np.ts
        );
    }
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
        Some("decisions") => quota_decisions(store, &args[1..]),
        _ => bail!("quota subcommand must be `should-run`, `usage`, `spend`, or `decisions`"),
    }
}

/// `loopx quota decisions --goal G [--limit N] [--format json]` — the
/// persisted decision_summary projection (P1-1②): recent compact decisions
/// (newest first) read straight from the ledger, so status/TUI/desktop-style
/// consumers reuse the kernel's decision without re-running it.
fn quota_decisions(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut format_json = false;
    let mut limit = 10usize;
    reject_unknown_flags(args, &["--format", "--goal", "--limit"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--format" {
            format_json = v == "json";
        } else if k == "--limit" {
            limit = v.parse().unwrap_or(10);
        }
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let events = store.events(&goal_id)?;
    let summaries = crate::quota::decision_summary::decision_summaries(&events);
    let recent: Vec<_> = summaries.into_iter().rev().take(limit).collect();
    if format_json {
        println!("{}", serde_json::to_string_pretty(&recent)?);
        return Ok(());
    }
    if recent.is_empty() {
        println!("goal {goal_id}: no decision summaries recorded yet (run a turn first)");
        return Ok(());
    }
    for s in recent {
        println!(
            "turn={} decision={} action={} code={} selected={} slots={}/{}",
            s.turn,
            s.decision,
            s.effective_action,
            s.reason_code,
            s.selected_todo.as_deref().unwrap_or("-"),
            s.spent_slots,
            s.allowed_slots
        );
    }
    Ok(())
}

/// `loopx quota should-run --goal G [--format json] [--agent-id A]` — emit
/// the typed ShouldRunPacket. Text mode renders the CLI projection (G-9):
/// decision banner + quota breakdown by spend source + scheduler hint +
/// stall hint + arbitration. JSON mode emits the full typed packet.
fn quota_should_run(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut format_json = false;
    let mut agent_id = None;
    reject_unknown_flags(args, &["--agent-id", "--format", "--goal"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--format" {
            format_json = v == "json";
        } else if k == "--agent-id" {
            agent_id = Some(v);
        }
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
    reject_unknown_flags(args, &["--all", "--format", "--goal"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--format" {
            format_json = v == "json";
        } else if k == "--all" {
            all = true;
        }
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
    reject_unknown_flags(args, &["--goal"])?;
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
fn cmd_scheduler(store: &mut Store, args: &[String]) -> Result<()> {
    match args.first().map(|s| s.as_str()) {
        Some("tick") => scheduler_tick(store, &args[1..]),
        Some("show") => scheduler_show(store, &args[1..]),
        Some("record-host-failure") => scheduler_record_failure(store, &args[1..]),
        Some("ack") => scheduler_ack(store, &args[1..]),
        Some("liveness") => scheduler_liveness(store, &args[1..]),
        _ => bail!(
            "scheduler subcommand must be `tick`, `show`, `record-host-failure`, `ack`, or `liveness`"
        ),
    }
}

/// `loopx scheduler ack --goal G [--agent-id A] --action tick_next
/// [--cadence-class C] [--rrule R] [--source S]` — record the host
/// scheduler's acknowledgement that it applied the cadence hint (P1-1③;
/// LoopX `scheduler_ack`). Projection-only audit event; scheduler state
/// itself is still owned by `scheduler tick`.
fn scheduler_ack(store: &mut Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut agent_id = None;
    let mut action = None;
    let mut cadence_class = String::new();
    let mut rrule = None;
    let mut source = "scheduler_cli".to_string();
    reject_unknown_flags(
        args,
        &[
            "--action",
            "--agent-id",
            "--cadence-class",
            "--goal",
            "--rrule",
            "--source",
        ],
    )?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--agent-id" {
            agent_id = Some(v);
        } else if k == "--action" {
            action = Some(v);
        } else if k == "--cadence-class" {
            cadence_class = v;
        } else if k == "--rrule" {
            rrule = Some(v);
        } else if k == "--source" {
            source = v;
        }
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let action = action.ok_or_else(|| anyhow::anyhow!("--action required"))?;
    store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let agent = agent_id.unwrap_or_else(|| "codex-app".to_string());
    store.append(Event::SchedulerAcked {
        goal_id: goal_id.clone(),
        agent_id: agent.clone(),
        action: action.clone(),
        cadence_class,
        rrule: rrule.clone(),
        source,
        ts: now_epoch(),
    })?;
    println!(
        "scheduler ack recorded: goal={goal_id} agent={agent} action={action} rrule={}",
        rrule.as_deref().unwrap_or("-")
    );
    Ok(())
}

fn scheduler_scope(
    store: &Store,
    args: &[String],
    default_agent: &str,
) -> Result<(String, String)> {
    let mut goal_id = None;
    let mut agent_id = None;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--agent-id" {
            agent_id = Some(v);
        }
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
/// P1-3: each tick also lands a `SchedulerTicked` heartbeat (liveness) and
/// projects the monitor poll plan (tick-driven poll policy executor).
fn scheduler_tick(store: &mut Store, args: &[String]) -> Result<()> {
    let mut cadence_class = "monitor_backoff".to_string();
    let mut progression: Vec<i64> = vec![];
    let mut action = "tick_next".to_string();
    reject_unknown_flags(
        args,
        &[
            "--action",
            "--agent-id",
            "--cadence-class",
            "--goal",
            "--progression",
        ],
    )?;
    parse_pairs(args, |k, v| {
        if k == "--cadence-class" {
            cadence_class = v;
        } else if k == "--progression" {
            progression = v
                .split(',')
                .filter_map(|s| s.trim().parse::<i64>().ok())
                .filter(|m| *m > 0)
                .collect();
        } else if k == "--action" {
            action = v;
        }
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
        )
        .expect("bootstrap scheduler state matches its own scope");
        st::write_scheduler_state(&goal_dir, &state)?;
        print!("{}", crate::cli_projection::render_scheduler_state(&state));
        println!(
            "→ bootstrapped (initial rrule {}); next tick advances progression",
            initial_rrule
        );
        record_tick_heartbeat(store, &goal_id, &agent, &action, &state)?;
        print_monitor_poll_plan(store, &goal_id)?;
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
    record_tick_heartbeat(store, &goal_id, &agent, &action, &state)?;
    print_monitor_poll_plan(store, &goal_id)?;
    Ok(())
}

/// P1-3①: land the tick heartbeat event (the liveness check's data source).
fn record_tick_heartbeat(
    store: &mut Store,
    goal_id: &str,
    agent: &str,
    action: &str,
    state: &crate::scheduler::state::SchedulerState,
) -> Result<()> {
    store.append(Event::SchedulerTicked {
        goal_id: goal_id.to_string(),
        agent_id: agent.to_string(),
        action: action.to_string(),
        rrule: if state.last_applied_rrule.is_empty() {
            None
        } else {
            Some(state.last_applied_rrule.clone())
        },
        ts: now_epoch(),
    })?;
    Ok(())
}

/// P1-3②: project the monitor poll plan after each tick (the tick-driven
/// poll policy executor — due monitors with target/policy/cadence and
/// no-spend eligibility; the run loop executes the actual observation).
fn print_monitor_poll_plan(store: &Store, goal_id: &str) -> Result<()> {
    let Some(goal) = store.replay(goal_id)? else {
        return Ok(());
    };
    let plan = crate::scheduler::monitor_poll::build_poll_plan(&goal, std::time::SystemTime::now());
    if plan.due_monitors.is_empty() && plan.stalled_monitors.is_empty() {
        if let Some(next) = plan.next_due_at {
            let wait = next.saturating_sub(now_epoch());
            println!("→ monitor poll plan: none due (next poll in {wait}s)");
        }
        return Ok(());
    }
    println!(
        "→ monitor poll plan: {} due, {} stalled",
        plan.due_monitors.len(),
        plan.stalled_monitors.len()
    );
    for item in &plan.due_monitors {
        println!(
            "   poll {} (target={}, policy={}, overdue {}s{})",
            item.todo_id,
            item.target.as_deref().unwrap_or("-"),
            item.policy.as_deref().unwrap_or("default"),
            item.overdue_secs,
            if item.no_spend_if_unchanged {
                ", no-spend on unchanged"
            } else {
                ""
            }
        );
    }
    for id in &plan.stalled_monitors {
        println!("   stalled {id} (decision kernel replans)");
    }
    Ok(())
}

/// `loopx scheduler liveness --goal G [--agent-id A] [--threshold-secs N]
/// [--format json]` — P1-3① automation liveness check: compare now against
/// the latest tick heartbeat (event log ∪ persisted scheduler state). A
/// breach records an `AutomationLivenessAlert` (cooldown-deduped) and drops
/// an operator-inbox alert file; the attention projection escalates the
/// goal until a fresh heartbeat recovers the automation.
fn scheduler_liveness(store: &mut Store, args: &[String]) -> Result<()> {
    use crate::scheduler::liveness as lv;
    let mut threshold = lv::DEFAULT_LIVENESS_THRESHOLD_SECS;
    reject_unknown_flags(
        args,
        &[
            "--agent-id",
            "--format",
            "--goal",
            "--json",
            "--threshold-secs",
        ],
    )?;
    parse_pairs(args, |k, v| {
        if k == "--threshold-secs" {
            if let Ok(n) = v.parse::<u64>() {
                if n > 0 {
                    threshold = n;
                }
            }
        }
    });
    let (goal_id, agent) = scheduler_scope(store, args, "codex-app")?;
    let goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let now = now_epoch();
    // Last heartbeat = max(heartbeat event ts, persisted state updated_at)
    // — the state file predates heartbeat events (back-compat).
    let mut last_tick = goal.scheduler_heartbeats.get(&agent).copied();
    use crate::scheduler::state as st;
    if let Some(s) = st::load_scheduler_state(
        &store.goal_dir(&goal_id),
        &agent,
        st::CODEX_APP_SURFACE,
        st::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY,
    ) {
        last_tick = Some(last_tick.map_or(s.updated_at, |t| t.max(s.updated_at)));
    }
    let eval = lv::evaluate_liveness(&goal_id, &agent, last_tick, now, threshold);
    let mut alert_note = String::new();
    if eval.state == lv::LIVENESS_BREACH {
        let alerts: Vec<u64> = goal
            .liveness_alerts
            .iter()
            .filter(|a| a.agent_id == agent)
            .map(|a| a.ts)
            .collect();
        if lv::alert_due(alerts.iter().max().copied(), now) {
            store.append(Event::AutomationLivenessAlert {
                goal_id: goal_id.clone(),
                agent_id: agent.clone(),
                elapsed_secs: eval.elapsed_secs.unwrap_or(0),
                threshold_secs: threshold,
                consecutive: alerts.len() as u32 + 1,
                ts: now,
            })?;
            write_liveness_inbox_alert(&goal, &agent, &eval);
            alert_note =
                " → alert recorded (attention escalates; operator inbox notified)".to_string();
        } else {
            alert_note = " (alert suppressed: cooldown)".to_string();
        }
    }
    if wants_json(args) {
        println!("{}", serde_json::to_string_pretty(&eval)?);
        return Ok(());
    }
    match eval.state.as_str() {
        lv::LIVENESS_BREACH => println!(
            "liveness: BREACH goal={goal_id} agent={agent} silent={}s threshold={}s{alert_note}",
            eval.elapsed_secs.unwrap_or(0),
            threshold
        ),
        lv::LIVENESS_NO_HEARTBEAT => println!(
            "liveness: no heartbeat for goal={goal_id} agent={agent} (automation never ticked — run `scheduler tick` to install)"
        ),
        _ => println!(
            "liveness: alive goal={goal_id} agent={agent} last tick {}s ago (threshold {}s)",
            eval.elapsed_secs.unwrap_or(0),
            threshold
        ),
    }
    Ok(())
}

/// Best-effort operator-inbox alert file (LoopX: breach → operator_inbox).
/// Written under the goal's project `.future/loop/inbox/` so the `inbox`
/// urgency projection surfaces it as a direct mention.
fn write_liveness_inbox_alert(
    goal: &crate::state::Goal,
    agent: &str,
    eval: &crate::scheduler::liveness::LivenessEvaluation,
) {
    let inbox = std::path::Path::new(&goal.cwd)
        .join(".future")
        .join("loop")
        .join("inbox");
    if std::fs::create_dir_all(&inbox).is_err() {
        return;
    }
    let clean = |s: &str| {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .collect::<String>()
    };
    let path = inbox.join(format!(
        "liveness-{}-{}-{}.json",
        clean(&goal.goal_id),
        clean(agent),
        now_epoch()
    ));
    let payload = serde_json::json!({
        "message_id": format!("liveness-{}-{}", clean(&goal.goal_id), now_epoch()),
        "create_time": now_epoch().to_string(),
        "content": format!(
            "@operator automation liveness breach: goal {} agent {} silent {}s (> {}s threshold) — check/restart the host automation",
            goal.goal_id,
            agent,
            eval.elapsed_secs.unwrap_or(0),
            eval.threshold_secs
        ),
    });
    if let Ok(text) = serde_json::to_string_pretty(&payload) {
        let _ = std::fs::write(path, format!("{text}\n"));
    }
}

/// `loopx scheduler show --goal G [--agent-id A] [--format json]` — print the
/// persisted scheduler state (or "no state yet").
fn scheduler_show(store: &Store, args: &[String]) -> Result<()> {
    reject_unknown_flags(args, &["--agent-id", "--format", "--goal", "--json"])?;
    let (goal_id, agent) = scheduler_scope(store, args, "codex-app")?;
    use crate::scheduler::state as st;
    let state = st::load_scheduler_state(
        &store.goal_dir(&goal_id),
        &agent,
        st::CODEX_APP_SURFACE,
        st::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY,
    );
    match state {
        Some(s) => {
            if wants_json(args) {
                println!("{}", serde_json::to_string_pretty(&s)?);
            } else {
                print!("{}", crate::cli_projection::render_scheduler_state(&s));
            }
        }
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
    reject_unknown_flags(
        args,
        &[
            "--agent-id",
            "--failure-count",
            "--failure-kind",
            "--goal",
            "--observed-rrule",
            "--target-rrule",
        ],
    )?;
    parse_pairs(args, |k, v| {
        if k == "--target-rrule" {
            target_rrule = Some(v);
        } else if k == "--observed-rrule" {
            observed_rrule = Some(v);
        } else if k == "--failure-kind" {
            failure_kind = Some(v);
        } else if k == "--failure-count" {
            count = v.parse().unwrap_or(1);
        }
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
            )
            .expect("bootstrap scheduler state matches its own scope")
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
    reject_unknown_flags(args, &["--format", "--json"])?;
    let json = args.iter().any(|a| a == "--format" || a == "--json");
    let mut client =
        crate::agent_client::AgentClient::connect(&crate::agent_client::agent_addr()).await?;
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
/// Auto-registration declares the process cwd as the agent's P0-1 workspace
/// (parallel runs launched from the same checkout then trip the workspace
/// guard instead of silently overwriting each other). `--anonymous` opts
/// back into the legacy uncoordinated one-shot path.
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
                let cwd = std::env::current_dir()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let workspaces = auto_register_workspaces(&cwd);
                store.append(Event::AgentRegistered {
                    goal_id: goal_id.to_string(),
                    agent_id: aid.to_string(),
                    workspaces,
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
    let mut force_workspace = false;
    reject_unknown_flags(
        args,
        &[
            "--agent-id",
            "--anonymous",
            "--force-workspace",
            "--goal",
            "--lease-secs",
            "--max-turn-secs",
            "--max-turns",
            "--model",
            "--thinking-level",
        ],
    )?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--model" {
            model = Some(v);
        } else if k == "--thinking-level" {
            thinking = Some(v);
        } else if k == "--max-turns" {
            max_turns = v.parse().unwrap_or(6);
        } else if k == "--max-turn-secs" {
            max_turn_secs = v.parse().unwrap_or(0);
        } else if k == "--agent-id" {
            agent_id = Some(v);
        } else if k == "--lease-secs" {
            lease_secs = v.parse().unwrap_or(DEFAULT_RUN_LEASE_SECS);
        } else if k == "--anonymous" {
            anonymous = true;
        } else if k == "--force-workspace" {
            force_workspace = true;
        }
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;

    // Identity gate BEFORE any gRPC/session work — fail fast with a hint
    // (and stays unit-testable without an agent server).
    let agent_id = ensure_run_identity(store, &goal_id, agent_id.as_deref(), anonymous)?;

    let mut client =
        crate::agent_client::AgentClient::connect(&crate::agent_client::agent_addr()).await?;
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
        force_workspace,
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
    let read = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::open(events_path)?;
        f.seek(SeekFrom::Start(offset))?;
        f.read_to_string(&mut buf)?;
        Ok(())
    })();
    if read.is_err() {
        return offset;
    }
    let new_offset = meta.len();
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
    #[cfg(test)]
    let mut polls = 0usize;
    loop {
        tokio::time::sleep(steer_poll_interval()).await;
        offset = steer_poll_once(&events_path, offset, &todo_id, &mut client, &session_id).await;
        #[cfg(test)]
        {
            polls += 1;
            if steer_test_should_stop(polls) {
                break;
            }
        }
    }
}

/// Steer watch poll cadence (short under cfg(test) so the seam test runs
/// instantly without tokio's test-util time control).
fn steer_poll_interval() -> std::time::Duration {
    #[cfg(test)]
    {
        std::time::Duration::from_millis(1)
    }
    #[cfg(not(test))]
    {
        std::time::Duration::from_secs(10)
    }
}

/// Test seam: bounds the (otherwise infinite) steer watch loop so tests can
/// drive one poll and observe a clean exit.
#[cfg(test)]
static STEER_TEST_MAX_POLLS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
fn steer_test_should_stop(polls: usize) -> bool {
    let max = STEER_TEST_MAX_POLLS.load(std::sync::atomic::Ordering::Relaxed);
    max > 0 && polls >= max
}

/// Claim the packet's selected todo under a lease BEFORE executing —
/// atomically (check+append under one lock) so two concurrent
/// `run --agent-id` workers can never both win the same todo; on contention,
/// re-decide against the fresh ledger and pick the next runnable todo (up to
/// 3 re-decides). Returns the claimed todo id, or None when the fresh ledger
/// has no executable selection for this turn.
fn claim_selected_with_lease(
    store: &mut Store,
    goal_id: &str,
    packet: &mut crate::contract::ShouldRunPacket,
    agent_id: Option<&str>,
    lease_secs: u64,
) -> Result<Option<String>> {
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
                let fresh = store
                    .replay(goal_id)?
                    .ok_or_else(|| goal_vanished_error(goal_id))?;
                *packet = decide_for(&fresh, SystemTime::now(), agent_id);
                if packet.interaction_contract.mode != crate::contract::TurnMode::BoundedDelivery
                    && packet.interaction_contract.mode != crate::contract::TurnMode::MonitorPoll
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
    Ok(todo_id_opt)
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
    force_workspace: bool,
) -> Result<()> {
    let mut turn = 0u32;
    // P1-2③: read-model self-healing — a drifted run index means run-history
    // consumers (status, stale-latest-run, run history projection) read stale
    // state; rebuild it from the run files before the first decision and
    // record the ProjectionRepaired audit event.
    run_index_self_heal(store, goal_id)?;
    loop {
        turn += 1;
        if turn > max_turns {
            bail!("max-turns ({max_turns}) reached without validated closure");
        }
        let goal = store
            .replay(goal_id)?
            .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found (deleted while running?)"))?;
        let packet = decide_for(&goal, SystemTime::now(), agent_id);
        // P1-1②③: persist the compact decision projection + the heartbeat
        // receipt for this turn (projection-only; replay ignores both).
        crate::quota::decision_summary::record_turn_decision(store, &packet, agent_id, turn)?;
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
        //
        // P0-1 workspace guard: if a PEER agent holds a live lease in an
        // overlapping declared workspace, degrade to serial — stop the run
        // with a retry hint (the scheduler will relaunch later) unless the
        // operator passed --force-workspace.
        let mut forced_ws = false;
        if let Some(aid) = agent_id {
            let now = crate::state::now_epoch();
            let conflicts =
                crate::agents::workspace_guard::live_workspace_conflicts(&goal, aid, now);
            if !conflicts.is_empty() && !force_workspace {
                bail!(
                    "workspace conflict — running would race a peer writing the same workspace:\n{}\
                     degrade to serial: rerun after the holder's lease expires, \
                     or pass --force-workspace",
                    crate::agents::workspace_guard::render_conflicts(&conflicts, now)
                );
            }
            forced_ws = !conflicts.is_empty() && force_workspace;
        }
        let mut packet = packet;
        let Some(todo_id) =
            claim_selected_with_lease(store, goal_id, &mut packet, agent_id, lease_secs)?
        else {
            println!("   no selected todo; stopping");
            break;
        };
        // P0-1: record the advisory write lock for the claimed todo (audit
        // trail for agent list / history). Best-effort against the
        // turn-start replay — profiles rarely change mid-turn.
        if let Some(aid) = agent_id {
            let goal = store
                .replay(goal_id)?
                .ok_or_else(|| goal_vanished_error(goal_id))?;
            append_workspace_lock(store, goal_id, aid, &todo_id, &goal, forced_ws)?;
        }
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
        // O3: progress signals for this turn (tool starts observed on the
        // stream; read at turn end, including the budget-truncation path).
        let progress = std::sync::Arc::new(TurnProgressTracker::new(now_epoch()));
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
            Some(&progress),
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
                    // O3: budget truncation is a turn end — evaluate the
                    // no-progress window against the observed tool starts
                    // before stopping the run.
                    record_no_progress_if_idle(store, goal_id, &todo_id, agent_id, &progress)?;
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
                validation_status_label(&v.status),
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
        // O3: normal turn end — evaluate the no-progress window and ledger
        // the breach (detection + bookkeeping; no auto-injection).
        record_no_progress_if_idle(store, goal_id, &todo_id, agent_id, &progress)?;
        // G-3: quota spend lands as a durable event alongside the run ledger
        // (source mirrors slot accounting; monitor no-change never spends).
        if monitor_changed != Some(false) {
            store
                .append(Event::QuotaSpent {
                    goal_id: goal_id.to_string(),
                    run_id: record.run_id.clone(),
                    todo_id: todo_id.clone(),
                    source: record
                        .spend_source
                        .clone()
                        .unwrap_or_else(|| "run".to_string()),
                    slots: 1,
                    ts: now_epoch(),
                })
                .expect("quota spend append only fails on disk IO");
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
            // P0-2①: a completed advancement todo is a delivery pending
            // verification — record the outcome signal at this turn.
            record_delivery_if_advancement(store, &g, goal_id, &todo_id, record.turn)?;
        } else {
            // A missing todo (deleted mid-turn) carries no budget signal.
            let stop = g
                .todo(&todo_id)
                .map(|t| {
                    if t.failed_attempts > MAX_REPAIR_ATTEMPTS {
                        println!("   ✘ repair budget exhausted — stopping");
                        return true;
                    }
                    // Validation-gated repair: a todo with an attached
                    // validator stays open until exit 0, bounded by its own
                    // max_validation_attempts.
                    if t.validator.is_some() && t.failed_attempts >= t.max_validation_attempts {
                        println!(
                            "   ✘ validation budget exhausted ({}/{}) — replan required; stopping",
                            t.failed_attempts, t.max_validation_attempts
                        );
                        return true;
                    }
                    false
                })
                .unwrap_or(false);
            if stop {
                break;
            }
        }
        // P0-2②: outcome_followthrough — auto-derive a follow-up todo for any
        // delivery left unverified past the threshold, then refresh the read
        // model when the follow-up(s) joined the frontier.
        g = run_followthrough_and_refresh(store, goal_id, g)?;
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

/// O3: evaluate the no-progress window at turn end (normal or budget-truncated)
/// and append a `TurnNoProgress` ledger event when breached. Detection +
/// bookkeeping only — nudge injection is the orchestrator's job via the
/// existing todo update steering channel.
fn record_no_progress_if_idle(
    store: &mut Store,
    goal_id: &str,
    todo_id: &str,
    agent_id: Option<&str>,
    progress: &TurnProgressTracker,
) -> Result<()> {
    let now = now_epoch();
    let snap = progress.snapshot();
    let threshold = crate::state::no_progress_idle_secs();
    let Some(idle_secs) = crate::executor::no_progress_idle_secs(
        snap.turn_start_at,
        snap.last_write_tool_at,
        now,
        threshold,
    ) else {
        return Ok(());
    };
    store.append(Event::TurnNoProgress {
        goal_id: goal_id.to_string(),
        todo_id: todo_id.to_string(),
        agent_id: agent_id.map(String::from),
        idle_secs,
        tool_calls_total: snap.tool_calls_total,
        ts: now,
    })?;
    println!(
        "   ⏳ TurnNoProgress: no write-class tool started for {idle_secs}s (threshold {threshold}s, {} tool calls)",
        snap.tool_calls_total
    );
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
            let mut goal_id = None;
            let mut repair = false;
            reject_unknown_flags(args, &["--format", "--goal", "--json", "--repair"])?;
            parse_pairs(args, |k, v| {
                if k == "--goal" {
                    goal_id = Some(v);
                } else if k == "--repair" {
                    repair = true;
                }
            });
            let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
            let report = store.verify(&goal_id)?;
            // P1-2①: run-index drift detection (read-model self-diagnosis)
            // alongside the ledger integrity check.
            let drift =
                crate::runtime::run_index::detect_index_drift(&store.root_path(), &goal_id)?;
            // P1-2③: `--repair` rebuilds a drifted index (non-destructive)
            // and records the ProjectionRepaired audit event.
            let repaired = if repair {
                crate::runtime::run_index::repair_index_if_drifted(store, &goal_id)?
            } else {
                None
            };
            if wants_json(args) {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "ledger": report,
                        "run_index_drift": drift,
                        "repaired": repaired,
                    }))?
                );
                return Ok(());
            }
            println!(
                "ledger {goal_id}: schema={} events={} unique={} idempotent_dups={} legacy_without_id={} skipped_unknown_kinds={} conflicts={:?} → {}",
                report.schema_version,
                report.total_events,
                report.unique_events,
                report.idempotent_duplicates,
                report.legacy_lines_without_id,
                report.skipped_unknown_kinds,
                report.conflicts,
                if report.ok { "ok" } else { "CONFLICT" }
            );
            if report.skipped_unknown_kinds > 0 {
                println!(
                    "note: {} unknown-kind event(s) [{}] skipped by the read path — binary older than ledger, please upgrade",
                    report.skipped_unknown_kinds,
                    report.unknown_kinds.join(", ")
                );
            }
            println!(
                "run_index {goal_id}: rows={} files={} missing={} stale={} duplicates={} → {}",
                drift.index_rows,
                drift.run_files,
                drift.missing_rows,
                drift.stale_rows,
                drift.duplicate_rows,
                if drift.repair_recommended {
                    "DRIFT (repair with `store verify --repair`)"
                } else {
                    "ok"
                }
            );
            if let Some(outcome) = repaired {
                println!(
                    "repaired run_index {goal_id}: {} drift rows → rebuilt {} rows (backup {})",
                    outcome.drift.drift_count,
                    outcome.rebuilt.rows_written,
                    if outcome.rebuilt.backup_path.is_empty() {
                        "none (index was missing)"
                    } else {
                        outcome.rebuilt.backup_path.as_str()
                    }
                );
            } else if repair {
                println!("repair: no drift — nothing to rebuild");
            }
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
    reject_unknown_flags(args, &["--goal"])?;
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
    reject_unknown_flags(args, &["--dry-run", "--from", "--goal", "--privacy"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--from" {
            from = Some(v);
        } else if k == "--privacy" {
            privacy = v;
        } else if k == "--dry-run" {
            dry_run = true;
        }
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
                backfill_event_label(&event.event)
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
    reject_unknown_flags(args, &["--format", "--goal", "--level"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--level" {
            level = v;
        } else if k == "--format" {
            format_json = v == "json";
        }
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
    // build_projections always populates the cache.
    let cache = projections
        .status_cache
        .as_ref()
        .expect("status cache is always built");
    crate::projection::status_cache::write_status_cache(&goal_dir, cache)?;
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
    let json = wants_json(args);
    let mut goal_id = None;
    let mut todo_id = None;
    let mut agent_id = None;
    let mut lease_secs = 0u64;
    let mut force = false;
    reject_unknown_flags(
        &args[1..],
        &[
            "--agent-id",
            "--force",
            "--format",
            "--goal",
            "--json",
            "--lease-secs",
            "--todo-id",
        ],
    )?;
    parse_pairs(&args[1..], |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--todo-id" {
            todo_id = Some(v);
        } else if k == "--agent-id" {
            agent_id = Some(v);
        } else if k == "--lease-secs" {
            lease_secs = v.parse().unwrap_or(0);
        } else if k == "--force" {
            force = true;
        }
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
        let status = lease::lease_status(todo, now);
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&lease_status_json(&todo_id, &status))?
            );
            return Ok(());
        }
        match status {
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

    // P0-1 workspace guard (claim only): checked before the mutable todo
    // borrow below; same conflict semantics as `todo claim`.
    if sub == "claim" {
        let conflicts =
            crate::agents::workspace_guard::live_workspace_conflicts(&goal, &agent, now);
        if !conflicts.is_empty() && !force {
            bail!(
                "workspace conflict — claiming would race a peer writing the same workspace:\n{}\
                 degrade to serial: retry after the holder's lease expires, or pass --force",
                crate::agents::workspace_guard::render_conflicts(&conflicts, now)
            );
        }
    }

    let todo = goal
        .todo_mut(&todo_id)
        .ok_or_else(|| anyhow::anyhow!("todo {todo_id} not found"))?;
    match sub {
        "claim" => {
            let op = lease::claim(todo, &agent, lease_secs, now)?;
            let expires = todo.lease_expires_at.unwrap_or(now);
            if !op.idempotent {
                if op.steal {
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
                    holder_pid: Some(std::process::id()),
                    ts: now,
                })?;
                // P0-1: advisory workspace write lock (audit for agent list).
                append_workspace_lock(store, &goal_id, &agent, &todo_id, &goal, force)?;
            }
            let _ = sync_compat(store, &goal_id);
            println!(
                "todo {todo_id} lease acquired by {agent} until {expires} {}✔",
                if op.steal {
                    "(steal after expiry) "
                } else {
                    ""
                }
            );
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
fn cmd_runs(store: &Store, args: &[String]) -> Result<()> {
    let sub = args.first().map(|s| s.as_str()).ok_or_else(|| {
        anyhow::anyhow!("runs requires a subcommand (history|compact|index|retention|stale)")
    })?;
    let mut goal_id = None;
    let mut keep = 50usize;
    let mut cutoff = None;
    let mut rebuild = false;
    let mut format_json = false;
    reject_unknown_flags(
        &args[1..],
        &["--cutoff", "--format", "--goal", "--keep", "--rebuild"],
    )?;
    parse_pairs(&args[1..], |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--keep" {
            keep = v.parse().unwrap_or(50);
        } else if k == "--cutoff" {
            cutoff = Some(v);
        } else if k == "--rebuild" {
            rebuild = true;
        } else if k == "--format" {
            format_json = v == "json";
        }
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

/// Print one replan obligation line pair (todo-bound or free-floating).
fn print_obligation(obligation: &crate::work_items::replan_obligation::ReplanObligation) {
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

/// The goal disappeared between decide and claim (deleted mid-run).
fn goal_vanished_error(goal_id: &str) -> anyhow::Error {
    anyhow::anyhow!("goal {goal_id} not found (deleted while running?)")
}

/// Auto-registration workspace declaration: the process cwd (P0-1 write set),
/// empty when the cwd could not be resolved (deleted directory / failure).
fn auto_register_workspaces(cwd: &str) -> Vec<String> {
    if cwd.is_empty() {
        vec![]
    } else {
        vec![crate::agents::workspace_guard::normalize_workspace_path(
            cwd,
        )]
    }
}

/// P1-2③: run the drift self-heal at run start and print the repair summary.
/// Extracted so the drifted-index projection (and both backup-path variants)
/// are unit-testable without a live agent client.
fn run_index_self_heal(store: &mut Store, goal_id: &str) -> Result<()> {
    if let Some(outcome) = crate::runtime::run_index::repair_index_if_drifted(store, goal_id)? {
        print_run_index_self_heal(&outcome);
    }
    Ok(())
}

fn print_run_index_self_heal(outcome: &crate::runtime::run_index::IndexRepairOutcome) {
    let backup = if outcome.rebuilt.backup_path.is_empty() {
        "none".to_string()
    } else {
        outcome.rebuilt.backup_path.clone()
    };
    println!(
        "⚒ projection self-heal: run_index drifted ({} rows) — rebuilt {} rows (backup {backup})",
        outcome.drift.drift_count, outcome.rebuilt.rows_written,
    );
}

/// P0-2①: a completed advancement todo is a delivery pending verification —
/// record the outcome signal at this turn. Non-advancement completions carry
/// no delivery signal (monitor/gate work is not a shipped artifact).
fn record_delivery_if_advancement(
    store: &mut Store,
    goal: &crate::state::Goal,
    goal_id: &str,
    todo_id: &str,
    turn: u32,
) -> Result<()> {
    if !goal
        .todo(todo_id)
        .map(|t| t.class == crate::state::TaskClass::Advancement)
        .unwrap_or(false)
    {
        return Ok(());
    }
    let seq = goal.delivery_state(todo_id).map(|d| d.seq + 1).unwrap_or(1);
    store.append(Event::DeliveryOutcomeRecorded {
        goal_id: goal_id.to_string(),
        todo_id: todo_id.to_string(),
        outcome: crate::work_items::delivery_outcome::OUTCOME_DELIVERED.to_string(),
        note: None,
        delivered_turn: turn,
        seq,
        ts: now_epoch(),
    })?;
    Ok(())
}

/// P0-2②: run the delivery follow-through check, print any auto-created
/// follow-up todos, and refresh the read model when they joined the frontier
/// (so the Next Action sync cannot project a gap).
fn run_followthrough_and_refresh(
    store: &mut Store,
    goal_id: &str,
    goal: crate::state::Goal,
) -> Result<crate::state::Goal> {
    let followups = run_followthrough_check(
        store,
        goal_id,
        crate::work_items::delivery_outcome::DEFAULT_FOLLOWTHROUGH_TURNS,
    )?;
    for followup in &followups {
        println!("   ↻ follow-through: todo {followup} auto-created (unverified delivery)");
    }
    if followups.is_empty() {
        Ok(goal)
    } else {
        store
            .replay(goal_id)?
            .ok_or_else(|| goal_vanished_error(goal_id))
    }
}

/// One-line summary of a backfilled event for `backfill --dry-run` output.
fn backfill_event_label(event: &Event) -> String {
    match event {
        Event::TodoAdded { todo, .. } => format!("add {}", todo.id),
        Event::TodoClaimed { todo_id, .. } => format!("claim {todo_id}"),
        Event::TodoCompleted { todo_id, .. } => format!("complete {todo_id}"),
        _ => "?".to_string(),
    }
}
/// Print the status projection, rendered for the CLI (G-9).
fn validation_status_label(status: &crate::state::ValidationStatus) -> &'static str {
    match status {
        crate::state::ValidationStatus::Passed => "passed",
        crate::state::ValidationStatus::Progress => "progress",
        crate::state::ValidationStatus::Failed => "failed",
        crate::state::ValidationStatus::Inconclusive => "inconclusive",
        crate::state::ValidationStatus::Unavailable => "unavailable",
        crate::state::ValidationStatus::NotRequired => "not_required",
    }
}

/// P0-3①: reject unknown `--flags` instead of silently ignoring them.
///
/// Every command handler validates its argument list against the flags it
/// actually parses, so a typo (`--gaol`) fails loudly with a help hint
/// instead of being silently swallowed (which used to surface as a
/// confusing "--goal required" or, worse, as silently ignored input).
/// `--help` and the global `--include-experimental` are always allowed.
fn reject_unknown_flags(args: &[String], known: &[&str]) -> Result<()> {
    for a in args {
        if !a.starts_with("--") || a == "--help" || a == "--include-experimental" {
            continue;
        }
        if !known.contains(&a.as_str()) {
            bail!("unknown flag `{a}` (try `{} --help`)", prog());
        }
    }
    Ok(())
}

/// P0-3③: does the arg list request JSON output? Accepts both `--json`
/// and `--format json` so every read-only command speaks the same dialect.
fn wants_json(args: &[String]) -> bool {
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--json" {
            return true;
        }
        if args[i] == "--format" && args.get(i + 1).map(|s| s.as_str()) == Some("json") {
            return true;
        }
        i += 1;
    }
    false
}

/// P0-3④: classify a `--resume-when` value — numeric N means "defer N
/// seconds from now" (a real deadline); anything else is a text-only hint.
#[derive(Debug, PartialEq)]
enum ResumeWhen {
    Defer(u64),
    TextHint(String),
}

fn parse_resume_when(value: &str) -> ResumeWhen {
    match value.trim().parse::<u64>() {
        Ok(secs) => ResumeWhen::Defer(secs),
        Err(_) => ResumeWhen::TextHint(value.to_string()),
    }
}

/// P0-3④: the warning printed when `--resume-when` is a text hint —
/// previously the no-deadline behavior was silent (FUTURE.md known quirk).
fn resume_when_text_hint_warning(value: &str, consequence: &str) -> String {
    format!(
        "warning: `--resume-when \"{value}\"` is not numeric — storing it as a text hint only \
         ({consequence}). Use a numeric value (seconds) to schedule a real deadline."
    )
}

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
    reject_unknown_flags(args, &["--agent-id", "--goal"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--agent-id" {
            agent_id = Some(v);
        }
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
    reject_unknown_flags(args, &["--agent-id", "--goal", "--max-turns"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--agent-id" {
            agent_id = Some(v);
        } else if k == "--max-turns" {
            max_turns = v.parse().unwrap_or(6);
        }
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

/// Join todo ids for CLI display (empty → "(none)").
fn join_ids(ids: &[String]) -> String {
    if ids.is_empty() {
        "(none)".to_string()
    } else {
        ids.join(", ")
    }
}

/// `loopx scope --goal G --agent-id A [--exclude X]` — the identity-scoped
/// frontier. P3 acceptance: two agents under one goal each hold a frontier
/// that never crosses into the other's claimed slices.
fn cmd_scope(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut agent_id = None;
    let mut exclude: Vec<String> = vec![];
    reject_unknown_flags(args, &["--agent-id", "--exclude", "--goal"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--agent-id" {
            agent_id = Some(v);
        } else if k == "--exclude" {
            exclude = v.split(',').map(|s| s.to_string()).collect();
        }
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
    reject_unknown_flags(args, &["--agent-id", "--goal"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--agent-id" {
            agent_id = Some(v);
        }
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
            reject_unknown_flags(
                &args[1..],
                &[
                    "--adapter-id",
                    "--agent-id",
                    "--authority-ref",
                    "--capabilities",
                    "--decision-id",
                    "--goal",
                    "--host-capabilities",
                    "--kind",
                    "--outcome",
                    "--receipt-id",
                    "--summary",
                    "--target-agent-id",
                ],
            )?;
            parse_pairs(&args[1..], |k, v| {
                if k == "--goal" {
                    goal_id = Some(v);
                } else if k == "--agent-id" {
                    supervisor_id = Some(v);
                } else if k == "--decision-id" {
                    decision_id = Some(v);
                } else if k == "--target-agent-id" {
                    target_agent_id = Some(v);
                } else if k == "--kind" {
                    kind = v;
                } else if k == "--capabilities" {
                    capabilities = v.split(',').map(|s| s.to_string()).collect();
                } else if k == "--summary" {
                    summary = Some(v);
                }
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
            reject_unknown_flags(
                &args[1..],
                &[
                    "--adapter-id",
                    "--agent-id",
                    "--authority-ref",
                    "--capabilities",
                    "--decision-id",
                    "--goal",
                    "--host-capabilities",
                    "--kind",
                    "--outcome",
                    "--receipt-id",
                    "--summary",
                    "--target-agent-id",
                ],
            )?;
            parse_pairs(&args[1..], |k, v| {
                if k == "--goal" {
                    goal_id = Some(v);
                } else if k == "--decision-id" {
                    decision_id = Some(v);
                } else if k == "--receipt-id" {
                    receipt_id = Some(v);
                } else if k == "--adapter-id" {
                    adapter_id = Some(v);
                } else if k == "--outcome" {
                    outcome = v;
                } else if k == "--authority-ref" {
                    authority_ref = Some(v);
                } else if k == "--host-capabilities" {
                    host_capabilities = v.split(',').map(|s| s.to_string()).collect();
                }
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
            reject_unknown_flags(
                &args[1..],
                &[
                    "--adapter-id",
                    "--agent-id",
                    "--authority-ref",
                    "--capabilities",
                    "--decision-id",
                    "--goal",
                    "--host-capabilities",
                    "--kind",
                    "--outcome",
                    "--receipt-id",
                    "--summary",
                    "--target-agent-id",
                ],
            )?;
            parse_pairs(&args[1..], |k, v| {
                if k == "--goal" {
                    goal_id = Some(v);
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

// ── task-graph (G-14) ──────────────────────────────────────────────────────

/// `loopx task-graph --goal G` — the todo dependency graph with topological
/// order; cycles fail closed.
fn cmd_task_graph(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    reject_unknown_flags(args, &["--format", "--goal", "--json"])?;
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
    if wants_json(args) {
        println!("{}", serde_json::to_string_pretty(&graph)?);
        return Ok(());
    }
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
    reject_unknown_flags(args, &["--all", "--format", "--goal", "--json"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--all" {
            all = true;
        }
    });
    let mut items = vec![];
    if let Some(g) = goal_id {
        if let Some(goal) = store.replay(&g)? {
            if let Some(item) = crate::work_items::attention::goal_attention_item(&goal) {
                items.push(item);
            }
            // G12: role-succession hints join the queue (one per succeeded
            // role slot; a fresh primary heartbeat recovers/suppresses).
            items.extend(crate::agents::multi_agent::succession_attention_items(
                store, &goal,
            )?);
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
    if wants_json(args) {
        println!("{}", serde_json::to_string_pretty(&queue)?);
        return Ok(());
    }
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
    reject_unknown_flags(
        args,
        &["--format", "--json", "--name", "--project", "--scope"],
    )?;
    parse_pairs(args, |k, v| {
        if k == "--project" {
            project = v;
        } else if k == "--scope" {
            scope = v;
        } else if k == "--name" {
            name = v;
        }
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
    if wants_json(args) {
        println!("{}", serde_json::to_string_pretty(&urgency)?);
        return Ok(());
    }
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

// ── delivery (P0-2: post-delivery outcome closure) ────────────────────────

/// `delivery <status|record|followthrough>` — the P0-2 signal chain:
/// delivered → verified/failed/rework, plus the manual follow-through scan
/// (the run path also scans automatically after every turn).
fn cmd_delivery(store: &mut Store, args: &[String]) -> Result<()> {
    match args.first().map(|s| s.as_str()) {
        Some("status") => delivery_status(store, &args[1..]),
        Some("record") => delivery_record(store, &args[1..]),
        Some("followthrough") => delivery_followthrough(store, &args[1..]),
        _ => bail!("delivery subcommand must be `status`, `record`, or `followthrough`"),
    }
}

/// `delivery status --goal G [--format json]` — the per-work-item delivery
/// outcome read model (state, age in turns, follow-through linkage).
fn delivery_status(store: &Store, args: &[String]) -> Result<()> {
    reject_unknown_flags(args, &["--format", "--goal", "--json"])?;
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
    let current_turn = goal.history.iter().map(|r| r.turn).max().unwrap_or(0);
    if wants_json(args) {
        let items: Vec<serde_json::Value> = goal
            .delivery_states
            .iter()
            .map(|d| {
                serde_json::json!({
                    "todo_id": d.todo_id,
                    "todo_text": goal.todo(&d.todo_id).map(|t| t.text.clone()),
                    "outcome": d.outcome,
                    "delivered_turn": d.delivered_turn,
                    "turns_since_delivery": current_turn.saturating_sub(d.delivered_turn),
                    "pending_verification": d.outcome
                        == crate::work_items::delivery_outcome::OUTCOME_DELIVERED,
                    "followthrough_todo_id": d.followthrough_todo_id,
                    "note": d.note,
                    "updated_at": d.updated_at,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }
    if goal.delivery_states.is_empty() {
        println!(
            "no deliveries recorded for goal {goal_id} (a delivery is recorded when an advancement todo completes)"
        );
        return Ok(());
    }
    println!(
        "deliveries for {goal_id} (current turn {current_turn}, follow-through threshold {} turns):",
        crate::work_items::delivery_outcome::DEFAULT_FOLLOWTHROUGH_TURNS
    );
    for d in &goal.delivery_states {
        let age = current_turn.saturating_sub(d.delivered_turn);
        let pending = d.outcome == crate::work_items::delivery_outcome::OUTCOME_DELIVERED;
        let follow = d
            .followthrough_todo_id
            .as_deref()
            .map(|f| format!(" follow-through={f}"))
            .unwrap_or_default();
        let age = if pending {
            format!(" age={age}turns")
        } else {
            String::new()
        };
        let note = d
            .note
            .as_deref()
            .map(|n| format!(" note={n}"))
            .unwrap_or_default();
        println!(
            "  {} [{}] delivered_turn={}{}{}{}",
            d.todo_id, d.outcome, d.delivered_turn, age, follow, note
        );
    }
    Ok(())
}

/// `delivery record --goal G --todo-id T --outcome <delivered|verified|
/// failed|rework> [--note ...]` — manual outcome writeback (the operator /
/// validator verification signal). Transitions are validated against the
/// current state before the event lands.
fn delivery_record(store: &mut Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut todo_id = None;
    let mut outcome_raw = None;
    let mut note = None;
    reject_unknown_flags(args, &["--goal", "--note", "--outcome", "--todo-id"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--todo-id" {
            todo_id = Some(v);
        } else if k == "--outcome" {
            outcome_raw = Some(v);
        } else if k == "--note" {
            note = Some(v);
        }
    });
    use crate::work_items::delivery_outcome as dov;
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let todo_id = todo_id.ok_or_else(|| anyhow::anyhow!("--todo-id required"))?;
    let outcome_raw = outcome_raw.ok_or_else(|| anyhow::anyhow!("--outcome required"))?;
    let outcome = dov::normalize_outcome(&outcome_raw).ok_or_else(|| {
        anyhow::anyhow!(
            "--outcome must be one of: {}",
            dov::DELIVERY_OUTCOME_CHOICES.join(", ")
        )
    })?;
    let goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    if goal.todo(&todo_id).is_none() {
        bail!("todo {todo_id} not found in goal {goal_id}");
    }
    dov::validate_transition(goal.delivery_state(&todo_id), outcome)
        .map_err(|e| anyhow::anyhow!("delivery {todo_id}: {e}"))?;
    let current_turn = goal.history.iter().map(|r| r.turn).max().unwrap_or(0);
    let seq = goal
        .delivery_state(&todo_id)
        .map(|d| d.seq + 1)
        .unwrap_or(1);
    store.append(Event::DeliveryOutcomeRecorded {
        goal_id: goal_id.clone(),
        todo_id: todo_id.clone(),
        outcome: outcome.to_string(),
        note: note.clone(),
        delivered_turn: current_turn,
        seq,
        ts: now_epoch(),
    })?;
    println!("delivery {todo_id} → {outcome} ✔");
    Ok(())
}

/// `delivery followthrough --goal G [--turns N]` — manually run the
/// outcome_followthrough scan (P0-2②); the run path calls the same driver
/// automatically after every turn.
fn delivery_followthrough(store: &mut Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut turns = crate::work_items::delivery_outcome::DEFAULT_FOLLOWTHROUGH_TURNS;
    reject_unknown_flags(args, &["--goal", "--turns"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--turns" {
            turns = v.parse().unwrap_or(turns);
        }
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let created = run_followthrough_check(store, &goal_id, turns)?;
    if created.is_empty() {
        println!("no overdue deliveries (all deliveries resolved or under {turns} turns old)");
    } else {
        for id in created {
            println!("follow-through todo {id} auto-created ✔ (unverified delivery aged past {turns} turns)");
        }
    }
    Ok(())
}

/// P0-2② outcome_followthrough driver: scan delivered-but-unverified work
/// items and auto-derive a follow-up todo for each one overdue by at least
/// `threshold` turns. Fires exactly once per delivery cycle — the
/// `FollowthroughCreated` event stamps the source delivery. Returns the ids
/// of the follow-up todos created.
fn run_followthrough_check(
    store: &mut Store,
    goal_id: &str,
    threshold: u32,
) -> Result<Vec<String>> {
    use crate::work_items::delivery_outcome as dov;
    let goal = store
        .replay(goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    let current_turn = goal.history.iter().map(|r| r.turn).max().unwrap_or(0);
    let overdue = dov::overdue_deliveries(&goal, current_turn, threshold);
    let mut created = Vec::new();
    for od in overdue {
        let followup_id = gen_id("todo");
        let mut todo = Todo::advancement(&followup_id, &dov::followthrough_todo_text(&od));
        todo.note = Some(format!(
            "auto-created by outcome_followthrough (source {}, turn {current_turn}, threshold {threshold})",
            od.todo_id
        ));
        store.append(Event::TodoAdded {
            goal_id: goal_id.to_string(),
            todo,
            ts: now_epoch(),
        })?;
        store.append(Event::FollowthroughCreated {
            goal_id: goal_id.to_string(),
            source_todo_id: od.todo_id.clone(),
            followup_todo_id: followup_id.clone(),
            turns_overdue: od.turns_overdue,
            ts: now_epoch(),
        })?;
        created.push(followup_id);
    }
    Ok(created)
}
/// `loopx registry [--format json|--json] [--include-experimental]` — inspect
/// the CLI registry (groups + commands) — the aggregated help surface.
fn cmd_registry(registry: &CommandRegistry, args: &[String]) -> Result<()> {
    reject_unknown_flags(args, &["--format", "--json"])?;
    let include_experimental = args.iter().any(|a| a == "--include-experimental");
    if wants_json(args) {
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

/// P1-9: `future loop commands` — the operator journey view (LoopX `loopx
/// commands` five-group presentation). The registry stays the flat machine
/// catalog; this is a pure presentation overlay over the same metadata.
fn cmd_commands(registry: &CommandRegistry, args: &[String]) -> Result<()> {
    reject_unknown_flags(args, &["--format", "--json"])?;
    let include_experimental = args.iter().any(|a| a == "--include-experimental");
    if wants_json(args) {
        let payload: Vec<serde_json::Value> = Journey::ALL
            .iter()
            .map(|j| {
                let cmds: Vec<serde_json::Value> = registry
                    .commands_in_journey(*j, include_experimental)
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
                serde_json::json!({
                    "journey": j.key(),
                    "title": j.title(),
                    "summary": j.summary(),
                    "commands": cmds,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }
    print!("{}", registry.render_journeys(include_experimental));
    Ok(())
}

// ── P4: canary smoke (G-20) ────────────────────────────────────────────────

/// `loopx canary smoke [--profile X] [--json]` — run a smoke profile
/// (default release-gate). Fails closed when any check fails.
/// `loopx canary premerge [--json]` — P1-6 CI merge gate (isolated root).
fn cmd_canary(store: &Store, args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("premerge") => cmd_canary_premerge(&args[1..]),
        Some("smoke") => cmd_canary_smoke(store, &args[1..]),
        Some(other) if !other.starts_with('-') => {
            anyhow::bail!("unknown canary subcommand `{other}` (expected `smoke` or `premerge`)")
        }
        // Legacy bare `canary` (or flags first) keeps the smoke default.
        _ => cmd_canary_smoke(store, args),
    }
}

/// `loopx canary smoke [--profile X] [--json]` — run a smoke profile
/// (default release-gate). Fails closed when any check fails.
fn cmd_canary_smoke(store: &Store, args: &[String]) -> Result<()> {
    let mut profile = None;
    let mut json = false;
    reject_unknown_flags(args, &["--json", "--profile"])?;
    parse_pairs(args, |k, v| {
        if k == "--profile" {
            profile = Some(v);
        } else if k == "--json" {
            json = true;
        }
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

/// `loopx canary premerge [--json]` — the P1-6 CI merge gate. Runs against an
/// isolated temporary state root (never the operator's live root) with a
/// seeded fixture goal so the run is non-vacuous, then applies the release
/// gate's all-passed rule. Exits non-zero when the gate fails.
fn cmd_canary_premerge(args: &[String]) -> Result<()> {
    let mut json = false;
    reject_unknown_flags(args, &["--json"])?;
    parse_pairs(args, |k, _v| {
        if k == "--json" {
            json = true;
        }
    });
    let report = crate::canary::run_premerge_gate_isolated()?;
    render_premerge_gate(&report, json)
}

/// Render the premerge gate report and fail the command when the gate did not
/// pass. Extracted so the failure arm is unit-testable with a failing report.
fn render_premerge_gate(report: &crate::canary::PremergeGateReport, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
    } else {
        println!(
            "canary premerge (suite {}): {} check(s) over {} goal(s)",
            report.run.suite,
            report.run.checks.len(),
            report.gate.goals_checked
        );
        for check in &report.run.checks {
            println!(
                "  [{}] {:<24} {}",
                if check.passed { "ok" } else { "FAIL" },
                check.id,
                check.detail
            );
        }
        println!(
            "gate: {} — {}",
            if report.gate.passed { "PASS" } else { "FAIL" },
            report.gate.reason
        );
    }
    if !report.gate.passed {
        anyhow::bail!("canary premerge gate failed: {}", report.gate.reason);
    }
    Ok(())
}

// ── P4: diagnostics & command surface (G-27) ───────────────────────────────

/// `loopx version` — version + schema surface.
fn cmd_version(store: &Store, args: &[String]) -> Result<()> {
    reject_unknown_flags(args, &[])?;
    println!("future-loop {}", env!("CARGO_PKG_VERSION"));
    println!("crate  : future-loop");
    println!("schemas:");
    println!("  canary_smoke_run_v0 (G-20)");
    println!("  canary_premerge_gate_v0 (P1-6)");
    println!("  future_loop_turn_envelope_v0 (G-9)");
    println!("  scheduler_arbitration_v0 (G-2/G-11)");
    let _ = store;
    Ok(())
}

/// `future-loop diagnose --goal G [--format json]` — per-goal diagnostic
/// surface: current decision, open todos/gates, projection gaps, closure
/// status, and recent run evidence.
fn cmd_diagnose(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut format_json = false;
    reject_unknown_flags(args, &["--format", "--goal"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--format" {
            format_json = v == "json";
        }
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
            "ledger_read_diagnostics": store.ledger_read_diagnostics(&goal_id),
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
    if let Some(note) = store.ledger_read_diagnostics(&goal_id).and_then(|d| {
        d.get("note")
            .and_then(|n| n.as_str())
            .map(|s| s.to_string())
    }) {
        println!("note      : {note}");
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
    reject_unknown_flags(args, &["--agent-addr", "--goal"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_filter = Some(v);
        } else if k == "--agent-addr" {
            agent_addr = Some(v);
        }
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
    reject_unknown_flags(args, &["--format", "--goal", "--json"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        }
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let goal = store
        .replay(&goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {goal_id} not found"))?;
    if wants_json(args) {
        println!("{}", serde_json::to_string_pretty(&goal.history)?);
        return Ok(());
    }
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
    reject_unknown_flags(args, &["--agent-id", "--goal", "--todo-id"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--todo-id" {
            todo_id = Some(v);
        } else if k == "--agent-id" {
            agent_id = Some(v);
        }
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
    reject_unknown_flags(args, &["--format", "--goal", "--json", "--todo-id"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--todo-id" {
            todo_id = Some(v);
        }
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let todo_id = todo_id.ok_or_else(|| anyhow::anyhow!("--todo-id required"))?;
    let events = store.events(&goal_id)?;
    let relevant: Vec<&crate::store::StoredEvent> = events
        .iter()
        .filter(|se| event_touches_todo(&se.event, &todo_id))
        .collect();
    if wants_json(args) {
        println!("{}", serde_json::to_string_pretty(&relevant)?);
        return Ok(());
    }
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
        | Event::WorkspaceLockAcquired { todo_id: id, .. }
        | Event::DeliveryOutcomeRecorded { todo_id: id, .. }
        | Event::GateResolved { todo_id: id, .. } => id == todo_id,
        Event::FollowthroughCreated {
            source_todo_id,
            followup_todo_id,
            ..
        } => source_todo_id == todo_id || followup_todo_id == todo_id,
        Event::RunRecorded { record, .. } => record.todo_id == todo_id,
        Event::TurnNoProgress { todo_id: id, .. } => id == todo_id,
        Event::HeartbeatReceiptRecorded { todo_id: id, .. } => id.as_deref() == Some(todo_id),
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
        Event::WorkspaceLockAcquired {
            agent_id,
            todo_id,
            paths,
            forced,
            ..
        } => {
            return format!(
                "workspace_lock agent={agent_id} todo={todo_id} paths={}{}",
                paths.join(","),
                if *forced { " (forced)" } else { "" }
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
        Event::TurnNoProgress {
            todo_id,
            agent_id,
            idle_secs,
            tool_calls_total,
            ..
        } => {
            return format!(
                "turn_no_progress todo={todo_id} agent={} idle={idle_secs}s tool_calls={tool_calls_total}",
                agent_id.as_deref().unwrap_or("anonymous")
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
        Event::DeliveryOutcomeRecorded {
            todo_id, outcome, ..
        } => {
            return format!("delivery_outcome todo={todo_id} outcome={outcome}");
        }
        Event::FollowthroughCreated {
            source_todo_id,
            followup_todo_id,
            ..
        } => {
            return format!(
                "followthrough_created source={source_todo_id} followup={followup_todo_id}"
            );
        }
        Event::DecisionSummaryRecorded { summary, .. } => {
            return format!(
                "decision_summary decision={} action={} code={} turn={}",
                summary.decision, summary.effective_action, summary.reason_code, summary.turn
            );
        }
        Event::HeartbeatReceiptRecorded {
            agent_id,
            turn_instance_id,
            todo_id,
            ..
        } => {
            return format!(
                "heartbeat_receipt agent={} turn={} todo={}",
                agent_id.as_deref().unwrap_or("anonymous"),
                turn_instance_id,
                todo_id.as_deref().unwrap_or("-")
            );
        }
        Event::SchedulerAcked {
            agent_id,
            action,
            rrule,
            ..
        } => {
            return format!(
                "scheduler_ack agent={agent_id} action={action} rrule={}",
                rrule.as_deref().unwrap_or("-")
            );
        }
        Event::SchedulerTicked {
            agent_id,
            action,
            rrule,
            ..
        } => {
            return format!(
                "scheduler_tick agent={agent_id} action={action} rrule={}",
                rrule.as_deref().unwrap_or("-")
            );
        }
        Event::AutomationLivenessAlert {
            agent_id,
            elapsed_secs,
            threshold_secs,
            consecutive,
            ..
        } => {
            return format!(
                "liveness_alert agent={agent_id} silent={elapsed_secs}s threshold={threshold_secs}s alert#{consecutive}"
            );
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
        Event::ProjectionRepaired {
            projection,
            drift_count,
            rows_written,
            ..
        } => {
            return format!(
                "projection_repaired projection={projection} drift={drift_count} rows_written={rows_written}"
            );
        }
        Event::MultiAgentContractSet { contract, .. } => {
            return format!(
                "multi_agent_contract_set peers={} handoff_rules={} collectives={}",
                contract.peers.len(),
                contract.handoff_rules.len(),
                contract.collectives.len()
            );
        }
        Event::AgentRecipeAdded { recipe, .. } => {
            return format!(
                "agent_recipe_added name={} capabilities={} priority={}",
                recipe.name,
                recipe.capabilities.join(","),
                recipe.priority
            );
        }
        Event::SuccessionOccurred {
            primary,
            backup,
            reason,
            ..
        } => {
            return format!(
                "succession_occurred primary={primary} backup={backup} reason={reason}"
            );
        }
        Event::ReplanRuleSetUpdated {
            rule_set_version,
            rule_ids,
            ..
        } => {
            return format!(
                "replan_rule_set_updated version={rule_set_version} rules={}",
                rule_ids.join(",")
            );
        }
    };
    kind.to_string()
}

/// `loopx evidence-log --goal G [--todo-id T]` — the evidence trail:
/// EvidenceAttached events + run evidence per todo.
fn cmd_evidence_log(store: &Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut todo_id = None;
    reject_unknown_flags(args, &["--format", "--goal", "--json", "--todo-id"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--todo-id" {
            todo_id = Some(v);
        }
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let events = store.events(&goal_id)?;
    let entries = collect_evidence_entries(&events, todo_id.as_deref());
    if wants_json(args) {
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }
    for entry in &entries {
        let evidence = crate::decision::truncate(&entry.evidence, 200);
        match entry.source.as_str() {
            "attached" => println!("[attached] todo={}: {evidence}", entry.todo_id),
            "run" => println!(
                "[run #{}] todo={}: {evidence}",
                entry.turn.unwrap_or_default(),
                entry.todo_id
            ),
            _ => println!("[completed] todo={}: {evidence}", entry.todo_id),
        }
    }
    if entries.is_empty() {
        println!(
            "goal {goal_id}: no evidence recorded{}",
            todo_id
                .map(|t| format!(" for todo {t}"))
                .unwrap_or_default()
        );
    } else {
        println!("({} evidence item(s))", entries.len());
    }
    Ok(())
}

/// One evidence-log entry (P0-3③: serializable so `evidence-log` has a
/// `--format json` form; the text view renders the same rows truncated).
#[derive(Debug, Clone, serde::Serialize)]
struct EvidenceEntry {
    /// attached | run | completed
    source: String,
    todo_id: String,
    turn: Option<u32>,
    evidence: String,
}

/// Project the evidence-bearing events of a goal into evidence-log rows
/// (optionally filtered to one todo). Pure, unit-testable.
fn collect_evidence_entries(
    events: &[crate::store::StoredEvent],
    todo_filter: Option<&str>,
) -> Vec<EvidenceEntry> {
    let matches = |tid: &str| todo_filter.map(|t| t == tid).unwrap_or(true);
    let mut out = Vec::new();
    for se in events {
        use crate::store::Event;
        match &se.event {
            Event::EvidenceAttached {
                todo_id: tid,
                evidence,
                ..
            } if matches(tid) => out.push(EvidenceEntry {
                source: "attached".to_string(),
                todo_id: tid.clone(),
                turn: None,
                evidence: evidence.clone(),
            }),
            Event::RunRecorded { record, .. }
                if matches(&record.todo_id) && !record.evidence.trim().is_empty() =>
            {
                out.push(EvidenceEntry {
                    source: "run".to_string(),
                    todo_id: record.todo_id.clone(),
                    turn: Some(record.turn),
                    evidence: record.evidence.clone(),
                });
            }
            Event::TodoCompleted {
                todo_id: tid,
                evidence: Some(evidence),
                ..
            } if matches(tid) => out.push(EvidenceEntry {
                source: "completed".to_string(),
                todo_id: tid.clone(),
                turn: None,
                evidence: evidence.clone(),
            }),
            _ => {}
        }
    }
    out
}

/// `loopx todo archive --goal G --todo-id T` — archive a todo
/// (LoopX: archive_state "archived").
fn todo_archive(store: &mut Store, args: &[String]) -> Result<()> {
    let mut goal_id = None;
    let mut todo_id = None;
    reject_unknown_flags(args, &["--goal", "--todo-id"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--todo-id" {
            todo_id = Some(v);
        }
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
    reject_unknown_flags(args, &["--goal", "--reason", "--todo-id"])?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--todo-id" {
            todo_id = Some(v);
        } else if k == "--reason" {
            reason = Some(v);
        }
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
    let mut acceptance: Option<String> = None;
    reject_unknown_flags(
        args,
        &[
            "--acceptance",
            "--blocks",
            "--evidence",
            "--goal",
            "--note",
            "--priority",
            "--resume-when",
            "--status",
            "--text",
            "--todo-id",
        ],
    )?;
    parse_pairs(args, |k, v| {
        if k == "--goal" {
            goal_id = Some(v);
        } else if k == "--todo-id" {
            todo_id = Some(v);
        } else if k == "--text" {
            text = Some(v);
        } else if k == "--status" {
            status = Some(v);
        } else if k == "--evidence" {
            evidence = Some(v);
        } else if k == "--note" {
            note = Some(v);
        } else if k == "--priority" {
            priority = Some(v);
        } else if k == "--resume-when" {
            resume_when = Some(v);
        } else if k == "--blocks" {
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
        } else if k == "--acceptance" {
            acceptance = Some(v);
        }
    });
    let goal_id = goal_id.ok_or_else(|| anyhow::anyhow!("--goal required"))?;
    let todo_id = todo_id.ok_or_else(|| anyhow::anyhow!("--todo-id required"))?;
    // Same bare-flag trap as todo add: a value-less `--blocks` parses as
    // the literal "true" — never a real todo id, so reject it loudly.
    if let Some(ids) = &blocks {
        if ids.iter().any(|b| b == "true") {
            bail!("--blocks requires a comma-separated todo id list (bare `--blocks` reads as `true`)");
        }
    }
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
    // (resume_when_text hint, no real deadline) — and now warns about it
    // (P0-3④) instead of silently scheduling nothing.
    let resume_when_parsed = resume_when
        .as_deref()
        .map(|rw| match parse_resume_when(rw) {
            ResumeWhen::Defer(secs) => format!("defer:{secs}"),
            ResumeWhen::TextHint(text) => {
                eprintln!(
                    "{}",
                    resume_when_text_hint_warning(
                        &text,
                        "no deadline is scheduled; the todo stays deferred until updated again"
                    )
                );
                text
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
        acceptance: acceptance.clone(),
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
            Event::GoalStarted {
                goal_id: "g".into(),
                ts: 1,
            },
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
            Event::TodoSuperseded {
                goal_id: "g".into(),
                todo_id: todo_id.into(),
                ts: 1,
            },
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
                acceptance: None,
                ts: 1,
            },
            Event::GoalCancelled {
                goal_id: "g".into(),
                reason: "r".into(),
                ts: 1,
            },
            Event::GateResolved {
                goal_id: "g".into(),
                todo_id: todo_id.into(),
                decision: "d".into(),
                note: None,
                ts: 1,
            },
            Event::GapSatisfied {
                goal_id: "g".into(),
                gap_id: "gap1".into(),
                ts: 1,
            },
            Event::RunRecorded {
                goal_id: "g".into(),
                record: record(todo_id),
                ts: 1,
            },
            Event::TodoClaimed {
                goal_id: "g".into(),
                todo_id: todo_id.into(),
                agent_id: "a".into(),
                lease_expires_at: 9,
                holder_pid: None,
                ts: 1,
            },
            Event::AgentRegistered {
                goal_id: "g".into(),
                agent_id: "a".into(),
                workspaces: vec![],
                ts: 1,
            },
            Event::AgentOnboarded {
                goal_id: "g".into(),
                agent_id: "a".into(),
                capabilities: vec!["shell".into()],
                workspaces: vec![],
                ts: 1,
            },
            Event::ReplanAcked {
                goal_id: "g".into(),
                delta_kinds: vec!["vision_patch".into()],
                ts: 1,
            },
            Event::ProfileSet {
                goal_id: "g".into(),
                outcome_floor_streak_threshold: 2,
                ts: 1,
            },
            Event::AuthoritySet {
                goal_id: "g".into(),
                write_scope: vec!["src".into()],
                requires_approval: vec!["publish".into()],
                ts: 1,
            },
            Event::TodoArchived {
                goal_id: "g".into(),
                todo_id: todo_id.into(),
                ts: 1,
            },
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
            Event::TodoExpired {
                goal_id: "g".into(),
                todo_id: todo_id.into(),
                ts: 1,
            },
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
            Event::FollowthroughCreated {
                goal_id: "g".into(),
                source_todo_id: todo_id.into(),
                followup_todo_id: "fu".into(),
                turns_overdue: 2,
                ts: 1,
            },
            Event::DecisionSummaryRecorded {
                goal_id: "g".into(),
                summary: crate::quota::decision_summary::DecisionSummary {
                    schema_version: "quota_decision_summary_v0".into(),
                    goal_id: "g".into(),
                    agent_id: None,
                    decision: "run".into(),
                    should_run: true,
                    effective_action: "bounded_delivery".into(),
                    reason_code: "runnable".into(),
                    mode: "bounded_delivery".into(),
                    state: "active".into(),
                    selected_todo: Some(todo_id.into()),
                    spent_slots: 1,
                    allowed_slots: 10,
                    normal_delivery_allowed: true,
                    recovery_delivery_allowed: false,
                    self_repair_allowed: false,
                    safe_bypass_allowed: false,
                    safe_bypass_kind: None,
                    blocked_action_scope: None,
                    turn: 1,
                },
                ts: 1,
            },
            Event::HeartbeatReceiptRecorded {
                goal_id: "g".into(),
                agent_id: Some("a".into()),
                turn_instance_id: "turn-1".into(),
                todo_id: Some(todo_id.into()),
                decision: "run".into(),
                reason_code: "runnable".into(),
                ts: 1,
            },
            Event::SchedulerAcked {
                goal_id: "g".into(),
                agent_id: "a".into(),
                action: "tick_next".into(),
                cadence_class: "monitor_backoff".into(),
                rrule: Some("FREQ=MINUTELY;INTERVAL=15".into()),
                source: "scheduler_cli".into(),
                ts: 1,
            },
            Event::SchedulerTicked {
                goal_id: "g".into(),
                agent_id: "a".into(),
                action: "tick_next".into(),
                rrule: Some("FREQ=MINUTELY;INTERVAL=15".into()),
                ts: 1,
            },
            Event::AutomationLivenessAlert {
                goal_id: "g".into(),
                agent_id: "a".into(),
                elapsed_secs: 3600,
                threshold_secs: 900,
                consecutive: 1,
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
        assert!(event_touches_todo(
            &Event::TodoAdded {
                goal_id: "g".into(),
                todo: Todo::advancement("todo_1", "task"),
                ts: 1,
            },
            "todo_1"
        ));
        assert!(!event_touches_todo(
            &Event::GoalStarted {
                goal_id: "g".into(),
                ts: 1
            },
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

    #[tokio::test]
    async fn steer_watch_loop_exits_via_test_seam() {
        // Two polls: the first does not stop (false edge), the second does.
        STEER_TEST_MAX_POLLS.store(2, std::sync::atomic::Ordering::Relaxed);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        steer_todo_updates(path, "t1".to_string(), "sess".to_string()).await;
        STEER_TEST_MAX_POLLS.store(0, std::sync::atomic::Ordering::Relaxed);
    }

    #[tokio::test]
    async fn steer_poll_read_failures_and_missing_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let mut client = None;
        // Missing file → metadata guard leaves the offset unchanged.
        let off = steer_poll_once(&path, 0, "t1", &mut client, "sess").await;
        assert_eq!(off, 0);
        // Non-UTF8 content → read_to_string fails → offset unchanged.
        std::fs::write(&path, [0xffu8, 0xfe, 0xfd]).unwrap();
        let off = steer_poll_once(&path, 0, "t1", &mut client, "sess").await;
        assert_eq!(off, 0);
        // A todo_updated line without `text` is skipped (no steer connect).
        std::fs::write(&path, "{\"kind\":\"todo_updated\",\"todo_id\":\"t1\"}\n").unwrap();
        let off = steer_poll_once(&path, 0, "t1", &mut client, "sess").await;
        assert!(off > 0);
        assert!(client.is_none());
    }

    #[tokio::test]
    async fn steer_poll_connect_failure_leaves_client_none() {
        std::env::set_var("FUTURE_LOOP_AGENT_ADDR", "127.0.0.1:1");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        std::fs::write(
            &path,
            "{\"kind\":\"todo_updated\",\"todo_id\":\"t1\",\"text\":\"new\"}\n",
        )
        .unwrap();
        let mut client = None;
        let off = steer_poll_once(&path, 0, "t1", &mut client, "sess").await;
        assert!(off > 0);
        assert!(
            client.is_none(),
            "connect to a closed port fails → client None"
        );
        std::env::remove_var("FUTURE_LOOP_AGENT_ADDR");
    }

    #[test]
    fn claim_loop_breaks_when_nothing_is_selected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        let mut store = Store::open(&root).unwrap();
        let goal = crate::state::Goal::new("g", "obj", "/tmp");
        store.register(&goal).unwrap();
        store
            .append(Event::GoalStarted {
                goal_id: "g".into(),
                ts: 1,
            })
            .unwrap();
        store
            .append(Event::TodoAdded {
                goal_id: "g".into(),
                todo: crate::state::Todo::advancement("t1", "w"),
                ts: 2,
            })
            .unwrap();
        let g = store.replay("g").unwrap().unwrap();
        // No selection: the claim loop exits immediately, nothing claimed.
        let mut packet = decide_for(&g, SystemTime::now(), Some("racer"));
        packet.interaction_contract.agent_channel.selected_todo = None;
        let r =
            claim_selected_with_lease(&mut store, "g", &mut packet, Some("racer"), 3600).unwrap();
        assert_eq!(r, None);
    }

    #[test]
    fn claim_loop_stops_when_the_re_decide_changes_mode() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        let mut store = Store::open(&root).unwrap();
        let goal = crate::state::Goal::new("g", "obj", "/tmp");
        store.register(&goal).unwrap();
        store
            .append(Event::GoalStarted {
                goal_id: "g".into(),
                ts: 1,
            })
            .unwrap();
        store
            .append(Event::TodoAdded {
                goal_id: "g".into(),
                todo: crate::state::Todo::advancement("t1", "w"),
                ts: 2,
            })
            .unwrap();
        // A live lease held by ANOTHER agent: the atomic claim fails.
        store
            .append(Event::TodoClaimed {
                goal_id: "g".into(),
                todo_id: "t1".into(),
                agent_id: "other".into(),
                lease_expires_at: crate::state::now_epoch() + 3600,
                holder_pid: None,
                ts: 3,
            })
            .unwrap();
        let g = store.replay("g").unwrap().unwrap();
        let mut packet = decide_for(&g, SystemTime::now(), Some("racer"));
        // Force a stale selection (as if t1 were free at decide time).
        packet.interaction_contract.agent_channel.selected_todo = Some("t1".to_string());
        packet.interaction_contract.mode = crate::contract::TurnMode::BoundedDelivery;
        let r =
            claim_selected_with_lease(&mut store, "g", &mut packet, Some("racer"), 3600).unwrap();
        // The fresh decide filters other-claimed todos → mode change → stop.
        assert_eq!(r, None);
    }

    #[test]
    fn print_obligation_with_and_without_todo() {
        let base = crate::work_items::replan_obligation::ReplanObligation {
            schema_version: "replan_obligation_v0".to_string(),
            kind: "surface_only_progress_streak".to_string(),
            goal_id: "g".to_string(),
            todo_id: None,
            raised_at: 7,
            evidence: "e".to_string(),
            cleared: false,
            cleared_reason: None,
            cleared_at: None,
        };
        print_obligation(&base);
        let bound = crate::work_items::replan_obligation::ReplanObligation {
            todo_id: Some("t1".to_string()),
            ..base.clone()
        };
        print_obligation(&bound);
    }

    #[test]
    fn registry_render_skips_groups_with_no_visible_commands() {
        let mut registry = CommandRegistry::new();
        let g = registry.group("exp-only", "experimental-only group");
        registry.command_experimental(g, "exp-cmd", "experimental", "exp-cmd");
        // Without --include-experimental the group renders no commands and is
        // skipped; with it, the group header prints.
        cmd_registry(&registry, &[]).unwrap();
        cmd_registry(&registry, &["--include-experimental".to_string()]).unwrap();
    }

    #[test]
    fn goal_vanished_error_message() {
        let e = goal_vanished_error("g1");
        assert!(format!("{e:#}").contains("deleted while running"));
    }

    #[test]
    fn backfill_event_label_covers_the_catch_all() {
        let ghost = Event::GoalCancelled {
            goal_id: "g".to_string(),
            reason: "r".to_string(),
            ts: 1,
        };
        assert_eq!(backfill_event_label(&ghost), "?");
    }

    #[test]
    fn label_fns_cover_every_variant() {
        use crate::state::ValidationStatus;
        let statuses = [
            (ValidationStatus::Passed, "passed"),
            (ValidationStatus::Progress, "progress"),
            (ValidationStatus::Failed, "failed"),
            (ValidationStatus::Inconclusive, "inconclusive"),
            (ValidationStatus::Unavailable, "unavailable"),
            (ValidationStatus::NotRequired, "not_required"),
        ];
        for (status, label) in statuses {
            assert_eq!(validation_status_label(&status), label);
        }
    }

    #[test]
    fn parse_pairs_edge_cases() {
        let mut seen: Vec<(String, String)> = vec![];
        parse_pairs(
            &[
                "--flag".to_string(),     // boolean-ish flag at end
                "positional".to_string(), // skipped
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
            .append(Event::GoalStarted {
                goal_id: "gj".into(),
                ts: 1,
            })
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
        assert!(refresh_next_action(&store, "goal_ghost").is_err());
        // And the write path for a real goal (produces ACTIVE_GOAL_STATE.md).
        let goal = Goal::new("gs", "sync goal", "/tmp");
        store.register(&goal).unwrap();
        store
            .append(Event::GoalStarted {
                goal_id: "gs".into(),
                ts: 1,
            })
            .unwrap();
        refresh_next_action(&store, "gs").unwrap();
        sync_compat(&store, "gs").unwrap();
        assert!(store.goal_dir("gs").join("ACTIVE_GOAL_STATE.md").exists());
    }
}

// ── P0-3 CLI quirks tests ─────────────────────────────────────────────────

#[cfg(test)]
mod cli_quirks_tests {
    use super::*;

    fn tmp_store(tag: &str) -> Store {
        let dir = std::env::temp_dir().join(format!(
            "future-loop-p03-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Store::open(dir.to_string_lossy().as_ref()).unwrap()
    }

    fn open_goal_with_todo(store: &mut Store, goal_id: &str) {
        let goal = Goal::new(goal_id, "objective", "/tmp");
        store.register(&goal).unwrap();
        let ts = goal.created_at;
        store
            .append(Event::GoalStarted {
                goal_id: goal_id.into(),
                ts,
            })
            .unwrap();
        store
            .append(Event::TodoAdded {
                goal_id: goal_id.into(),
                todo: Todo::advancement("t1", "shared work"),
                ts,
            })
            .unwrap();
    }

    // ① unknown flags are rejected, not silently ignored ───────────────────

    #[test]
    fn reject_unknown_flags_accepts_known_and_positionals() {
        let args = vec!["--goal".to_string(), "g1".to_string(), "status".to_string()];
        assert!(reject_unknown_flags(&args, &["--goal"]).is_ok());
    }

    #[test]
    fn reject_unknown_flags_fails_loudly_on_typo() {
        let args = vec!["--gaol".to_string(), "g1".to_string()];
        let err = reject_unknown_flags(&args, &["--goal"]).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("unknown flag `--gaol`"), "got: {msg}");
        assert!(msg.contains("--help"), "hint missing: {msg}");
    }

    #[test]
    fn reject_unknown_flags_allows_help_and_global_flags() {
        let args = vec!["--help".to_string(), "--include-experimental".to_string()];
        assert!(reject_unknown_flags(&args, &["--goal"]).is_ok());
    }

    #[test]
    fn unknown_flag_errors_end_to_end_on_read_and_write_commands() {
        let mut store = tmp_store("e2e-unknown");
        open_goal_with_todo(&mut store, "g1");
        // read-only command
        let err = cmd_status(&store, &["--bogus".to_string()]).unwrap_err();
        assert!(format!("{err}").contains("unknown flag `--bogus`"));
        // write command
        let err = todo_update(
            &mut store,
            &[
                "--goal".to_string(),
                "g1".to_string(),
                "--todo-id".to_string(),
                "t1".to_string(),
                "--bogus".to_string(),
            ],
        )
        .unwrap_err();
        assert!(format!("{err}").contains("unknown flag `--bogus`"));
    }

    // ② subcommand --help renders from the registry ────────────────────────

    #[test]
    fn render_command_help_shows_usage_for_registered_command() {
        let registry = build_cli_registry();
        let help = render_command_help(&registry, "status", false);
        assert!(help.contains("status [--goal G]"), "got: {help}");
        assert!(help.contains("usage: "), "got: {help}");
        assert!(help.contains("group: goal"), "got: {help}");
    }

    #[test]
    fn render_command_help_unknown_command_falls_back() {
        let registry = build_cli_registry();
        let help = render_command_help(&registry, "nope-not-a-command", false);
        assert!(help.contains("unknown command"), "got: {help}");
    }

    #[test]
    fn shorten_home_contracts_the_home_prefix() {
        let home = std::env::var("HOME").expect("HOME is set in the test environment");
        assert_eq!(shorten_home(&format!("{home}/sub/dir")), "~/sub/dir");
        // A path outside HOME is returned unchanged.
        assert_eq!(shorten_home("/definitely/not/home"), "/definitely/not/home");
    }

    // P1-9: journey metadata + grouped command reference ──────────────────

    #[test]
    fn journey_assignments_cover_every_static_command() {
        use std::collections::HashSet;
        let registry = build_cli_registry();
        let assigned: HashSet<&str> = JOURNEY_ASSIGNMENTS.iter().map(|(n, _)| *n).collect();
        for c in registry.commands(true) {
            assert!(
                assigned.contains(c.name.as_str()),
                "registered command `{}` has no journey assignment",
                c.name
            );
        }
        for name in &assigned {
            assert!(
                registry.find(name, true).is_some(),
                "journey assignment `{name}` matches no registered command"
            );
        }
    }

    #[test]
    fn commands_reference_groups_by_journey() {
        let registry = build_cli_registry();
        let text = registry.render_journeys(false);
        for title in [
            "Start here",
            "Daily operator",
            "Loop driver",
            "Setup & automation",
            "Maintainer & adapter",
        ] {
            assert!(text.contains(title), "missing journey `{title}`: {text}");
        }
        // spot-check placement
        let starter = text.find("goal init --objective").unwrap();
        let daily_pos = text.find("── Daily operator ──").unwrap();
        assert!(starter < daily_pos, "goal must be in Start here: {text}");
        let run_pos = text.find("run --goal G --agent-id A").unwrap();
        assert!(run_pos > daily_pos, "run must come after daily: {text}");
    }

    #[test]
    fn cmd_commands_rejects_unknown_flags() {
        let registry = build_cli_registry();
        let err = cmd_commands(&registry, &["--journey".to_string()]).unwrap_err();
        assert!(format!("{err}").contains("unknown flag `--journey`"));
    }

    // ③ --format json detection + read-only JSON projections ───────────────

    #[test]
    fn wants_json_detects_both_dialects() {
        assert!(wants_json(&["--json".to_string()]));
        assert!(wants_json(&["--format".to_string(), "json".to_string()]));
        assert!(!wants_json(&["--format".to_string(), "text".to_string()]));
        assert!(!wants_json(&["--goal".to_string(), "g1".to_string()]));
        assert!(!wants_json(&[]));
    }

    #[test]
    fn lease_status_json_projects_all_three_states() {
        use crate::work_items::task_lease::LeaseStatus;
        let free = lease_status_json("t1", &LeaseStatus::Free);
        assert_eq!(free["lease"], "free");
        assert_eq!(free["todo_id"], "t1");
        let active = lease_status_json(
            "t1",
            &LeaseStatus::Active {
                owner: "alice".to_string(),
                expires_at: 123,
            },
        );
        assert_eq!(active["lease"], "active");
        assert_eq!(active["owner"], "alice");
        assert_eq!(active["expires_at"], 123);
        let expired = lease_status_json(
            "t1",
            &LeaseStatus::Expired {
                owner: "bob".to_string(),
                expires_at: 99,
            },
        );
        assert_eq!(expired["lease"], "expired");
        assert_eq!(expired["expired_at"], 99);
    }

    #[test]
    fn agent_list_rows_marks_live_lease_holder_running() {
        let mut goal = Goal::new("g1", "objective", "/tmp");
        goal.registered_agents = vec!["alice".to_string(), "bob".to_string()];
        goal.agent_profiles = vec![crate::state::AgentProfile {
            id: "alice".to_string(),
            capabilities: vec!["code".to_string()],
            workspaces: vec![],
        }];
        let mut todo = Todo::advancement("t1", "work");
        todo.claimed_by = Some("alice".to_string());
        todo.lease_expires_at = Some(2_000);
        goal.todos.push(todo);
        let mut last_active = HashMap::new();
        last_active.insert("alice".to_string(), 900u64);
        let rows = agent_list_rows(&goal, &last_active, 1_000);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].agent_id, "alice");
        assert_eq!(rows[0].status, "running");
        assert_eq!(rows[0].work_on.len(), 1);
        assert_eq!(rows[0].capabilities, vec!["code".to_string()]);
        assert_eq!(rows[0].last_active_ts, Some(900));
        assert_eq!(rows[1].agent_id, "bob");
        assert_eq!(rows[1].status, "idle");
        // rows serialize (the --format json path)
        let json = serde_json::to_string(&rows).unwrap();
        assert!(json.contains("\"status\":\"running\""));
    }

    #[test]
    fn collect_evidence_entries_covers_all_sources_and_filter() {
        use crate::store::StoredEvent;
        let mk = |event: Event| StoredEvent {
            event_id: String::new(),
            producer: None,
            source_ref: None,
            source_section: None,
            source_line: None,
            privacy: None,
            fencing_token: None,
            event,
        };
        let events = vec![
            mk(Event::GoalStarted {
                goal_id: "g1".into(),
                ts: 0,
            }),
            mk(Event::EvidenceAttached {
                goal_id: "g1".into(),
                todo_id: "t1".into(),
                evidence: "attached-ev".into(),
                ts: 1,
            }),
            mk(Event::RunRecorded {
                goal_id: "g1".into(),
                record: crate::state::RunRecord {
                    turn: 3,
                    todo_id: "t2".into(),
                    run_id: "r1".into(),
                    validation: None,
                    terminal_state: "continue".into(),
                    error: None,
                    tokens_in_delta: 0,
                    tokens_out_delta: 0,
                    cost_delta: 0.0,
                    tools: vec![],
                    evidence: "run-ev".into(),
                    recorded_at: 2,
                    spend_source: None,
                },
                ts: 2,
            }),
            mk(Event::TodoCompleted {
                goal_id: "g1".into(),
                todo_id: "t1".into(),
                no_follow_up: true,
                successor_ids: vec![],
                evidence: Some("completed-ev".into()),
                ts: 3,
            }),
        ];
        let all = collect_evidence_entries(&events, None);
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].source, "attached");
        assert_eq!(all[1].source, "run");
        assert_eq!(all[1].turn, Some(3));
        assert_eq!(all[2].source, "completed");
        let filtered = collect_evidence_entries(&events, Some("t1"));
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|e| e.todo_id == "t1"));
        // entries serialize (the --format json path)
        assert!(serde_json::to_string(&all).is_ok());
    }

    #[test]
    fn json_flags_accepted_end_to_end_on_new_read_commands() {
        let mut store = tmp_store("e2e-json");
        open_goal_with_todo(&mut store, "g1");
        let json = "--format".to_string();
        let val = "json".to_string();
        // lease status
        cmd_lease(
            &mut store,
            &[
                "status".to_string(),
                "--goal".to_string(),
                "g1".to_string(),
                "--todo-id".to_string(),
                "t1".to_string(),
                json.clone(),
                val.clone(),
            ],
        )
        .unwrap();
        // agent list (empty registry → text "no agents"; json flag accepted)
        cmd_agent_list(
            &store,
            &[
                "--goal".to_string(),
                "g1".to_string(),
                json.clone(),
                val.clone(),
            ],
        )
        .unwrap();
        // task-graph
        cmd_task_graph(
            &store,
            &[
                "--goal".to_string(),
                "g1".to_string(),
                json.clone(),
                val.clone(),
            ],
        )
        .unwrap();
        // history
        cmd_history(
            &store,
            &[
                "--goal".to_string(),
                "g1".to_string(),
                json.clone(),
                val.clone(),
            ],
        )
        .unwrap();
        // todo-event
        cmd_todo_event(
            &store,
            &[
                "--goal".to_string(),
                "g1".to_string(),
                "--todo-id".to_string(),
                "t1".to_string(),
                json.clone(),
                val.clone(),
            ],
        )
        .unwrap();
        // evidence-log
        cmd_evidence_log(
            &store,
            &["--goal".to_string(), "g1".to_string(), json.clone(), val],
        )
        .unwrap();
        // replan obligations
        cmd_replan(
            &mut store,
            &[
                "obligations".to_string(),
                "--goal".to_string(),
                "g1".to_string(),
                "--json".to_string(),
            ],
        )
        .unwrap();
    }

    // ④ text --resume-when warns (no deadline) ─────────────────────────────

    #[test]
    fn parse_resume_when_classifies_numeric_vs_text() {
        assert_eq!(parse_resume_when("300"), ResumeWhen::Defer(300));
        assert_eq!(parse_resume_when("  60  "), ResumeWhen::Defer(60));
        assert_eq!(
            parse_resume_when("when the build is green"),
            ResumeWhen::TextHint("when the build is green".to_string())
        );
    }

    #[test]
    fn resume_when_text_hint_warning_names_value_and_consequence() {
        let w = resume_when_text_hint_warning("next week", "no deadline is scheduled");
        assert!(w.contains("`--resume-when \"next week\"`"), "got: {w}");
        assert!(w.contains("text hint only"), "got: {w}");
        assert!(w.contains("no deadline is scheduled"), "got: {w}");
        assert!(w.contains("numeric value (seconds)"), "got: {w}");
    }

    #[test]
    fn todo_update_text_resume_when_defers_without_deadline() {
        let mut store = tmp_store("e2e-resume-text");
        open_goal_with_todo(&mut store, "g1");
        todo_update(
            &mut store,
            &[
                "--goal".to_string(),
                "g1".to_string(),
                "--todo-id".to_string(),
                "t1".to_string(),
                "--resume-when".to_string(),
                "after review".to_string(),
            ],
        )
        .unwrap();
        let goal = store.replay("g1").unwrap().unwrap();
        let todo = goal.todo("t1").unwrap();
        assert_eq!(todo.status, crate::state::TodoStatus::Deferred);
        assert_eq!(todo.resume_when_text.as_deref(), Some("after review"));
        // text hint → NO real deadline
        assert!(todo.resume_when.is_none());
    }

    #[test]
    fn todo_update_numeric_resume_when_sets_real_deadline() {
        let mut store = tmp_store("e2e-resume-num");
        open_goal_with_todo(&mut store, "g1");
        let before = SystemTime::now();
        todo_update(
            &mut store,
            &[
                "--goal".to_string(),
                "g1".to_string(),
                "--todo-id".to_string(),
                "t1".to_string(),
                "--resume-when".to_string(),
                "120".to_string(),
            ],
        )
        .unwrap();
        let goal = store.replay("g1").unwrap().unwrap();
        let todo = goal.todo("t1").unwrap();
        assert_eq!(todo.status, crate::state::TodoStatus::Deferred);
        let deadline = todo.resume_when.expect("numeric sets a deadline");
        assert!(deadline >= before + std::time::Duration::from_secs(120));
    }
}

// ── P0-1 workspace guard CLI contract tests ───────────────────────────────

#[cfg(test)]
mod workspace_guard_cli_tests {
    use super::*;

    fn tmp_store(tag: &str) -> Store {
        let dir = std::env::temp_dir().join(format!(
            "future-loop-p01-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Store::open(dir.to_string_lossy().as_ref()).unwrap()
    }

    fn open_goal(store: &mut Store, goal_id: &str, todo_ids: &[&str]) {
        let goal = Goal::new(goal_id, "objective", "/tmp");
        store.register(&goal).unwrap();
        let ts = goal.created_at;
        store
            .append(Event::GoalStarted {
                goal_id: goal_id.into(),
                ts,
            })
            .unwrap();
        for id in todo_ids {
            store
                .append(Event::TodoAdded {
                    goal_id: goal_id.into(),
                    todo: Todo::advancement(id, "work"),
                    ts,
                })
                .unwrap();
        }
    }

    fn onboard(store: &mut Store, goal: &str, agent: &str, workspace: &str) {
        cmd_agent_onboard(
            store,
            &[
                "--goal".to_string(),
                goal.to_string(),
                "--agent-id".to_string(),
                agent.to_string(),
                "--workspace".to_string(),
                workspace.to_string(),
            ],
        )
        .unwrap();
    }

    fn claim_args(goal: &str, todo: &str, agent: &str, extra: &[&str]) -> Vec<String> {
        let mut v = vec![
            "--goal".to_string(),
            goal.to_string(),
            "--todo-id".to_string(),
            todo.to_string(),
            "--agent-id".to_string(),
            agent.to_string(),
        ];
        v.extend(extra.iter().map(|s| s.to_string()));
        v
    }

    fn lock_events(store: &Store, goal: &str) -> Vec<(String, String, bool)> {
        store
            .events(goal)
            .unwrap()
            .iter()
            .filter_map(|se| match &se.event {
                Event::WorkspaceLockAcquired {
                    agent_id,
                    todo_id,
                    forced,
                    ..
                } => Some((agent_id.clone(), todo_id.clone(), *forced)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn register_and_onboard_workspace_roundtrip_through_replay() {
        let mut store = tmp_store("declare");
        open_goal(&mut store, "g1", &[]);
        cmd_agent(
            &mut store,
            &[
                "--goal".to_string(),
                "g1".to_string(),
                "--agent-id".to_string(),
                "a1".to_string(),
                "--workspace".to_string(),
                "/definitely/not/here/wt1".to_string(),
            ],
        )
        .unwrap();
        onboard(&mut store, "g1", "a2", "/definitely/not/here/wt2");
        let goal = store.replay("g1").unwrap().unwrap();
        assert_eq!(
            crate::agents::workspace_guard::agent_workspaces(&goal, "a1"),
            vec!["/definitely/not/here/wt1".to_string()]
        );
        assert_eq!(
            crate::agents::workspace_guard::agent_workspaces(&goal, "a2"),
            vec!["/definitely/not/here/wt2".to_string()]
        );
    }

    #[test]
    fn legacy_agent_registered_event_without_workspaces_replays_empty() {
        // Old ledger line (pre-P0-1) — no `workspaces` field.
        let json = r#"{"kind":"agent_registered","goal_id":"g1","agent_id":"old","ts":1}"#;
        let event: Event = serde_json::from_str(json).unwrap();
        assert!(
            matches!(
                &event,
                Event::AgentRegistered { workspaces, .. } if workspaces.is_empty()
            ),
            "wrong variant or non-empty workspaces: {event:?}"
        );
    }

    #[test]
    fn claim_refuses_on_live_workspace_conflict_and_stays_serial() {
        let mut store = tmp_store("conflict");
        open_goal(&mut store, "g1", &["t1", "t2"]);
        onboard(&mut store, "g1", "agent-a", "/definitely/not/here/wt1");
        onboard(&mut store, "g1", "agent-b", "/definitely/not/here/wt1");
        // agent-b claims t1 first (a is idle → no conflict).
        todo_claim(&mut store, &claim_args("g1", "t1", "agent-b", &[])).unwrap();
        // agent-a claiming t2 in the same workspace must be refused.
        let err = todo_claim(&mut store, &claim_args("g1", "t2", "agent-a", &[])).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("workspace conflict"), "got: {msg}");
        assert!(msg.contains("agent-b"), "holder missing: {msg}");
        assert!(msg.contains("--force"), "force hint missing: {msg}");
        // No claim for t2 landed in the ledger.
        let goal = store.replay("g1").unwrap().unwrap();
        assert!(goal.todo("t2").unwrap().claimed_by.is_none());
    }

    #[test]
    fn claim_force_overrides_conflict_and_records_forced_lock() {
        let mut store = tmp_store("force");
        open_goal(&mut store, "g1", &["t1", "t2"]);
        onboard(&mut store, "g1", "agent-a", "/definitely/not/here/wt1");
        onboard(&mut store, "g1", "agent-b", "/definitely/not/here/wt1");
        todo_claim(&mut store, &claim_args("g1", "t1", "agent-b", &[])).unwrap();
        todo_claim(&mut store, &claim_args("g1", "t2", "agent-a", &["--force"])).unwrap();
        let goal = store.replay("g1").unwrap().unwrap();
        assert_eq!(
            goal.todo("t2").unwrap().claimed_by.as_deref(),
            Some("agent-a")
        );
        let locks = lock_events(&store, "g1");
        assert_eq!(locks.len(), 2, "one lock per workspace-declaring claim");
        assert_eq!(locks[0], ("agent-b".to_string(), "t1".to_string(), false));
        assert_eq!(locks[1], ("agent-a".to_string(), "t2".to_string(), true));
    }

    #[test]
    fn claim_without_conflict_records_unforced_lock_only_for_declaring_agents() {
        let mut store = tmp_store("lock");
        open_goal(&mut store, "g1", &["t1", "t2"]);
        onboard(&mut store, "g1", "agent-a", "/definitely/not/here/wt1");
        // agent-b registers WITHOUT a workspace declaration.
        cmd_agent(
            &mut store,
            &[
                "--goal".to_string(),
                "g1".to_string(),
                "--agent-id".to_string(),
                "agent-b".to_string(),
            ],
        )
        .unwrap();
        todo_claim(&mut store, &claim_args("g1", "t1", "agent-a", &[])).unwrap();
        todo_claim(&mut store, &claim_args("g1", "t2", "agent-b", &[])).unwrap();
        let locks = lock_events(&store, "g1");
        assert_eq!(
            locks,
            vec![("agent-a".to_string(), "t1".to_string(), false)],
            "undeclared agents leave no lock record (fail-open)"
        );
    }

    #[test]
    fn disjoint_workspaces_claim_freely() {
        let mut store = tmp_store("disjoint");
        open_goal(&mut store, "g1", &["t1", "t2"]);
        onboard(&mut store, "g1", "agent-a", "/definitely/not/here/wt1");
        onboard(&mut store, "g1", "agent-b", "/definitely/not/here/wt2");
        todo_claim(&mut store, &claim_args("g1", "t1", "agent-b", &[])).unwrap();
        todo_claim(&mut store, &claim_args("g1", "t2", "agent-a", &[])).unwrap();
    }

    #[test]
    fn lease_claim_applies_the_same_guard() {
        let mut store = tmp_store("lease-guard");
        open_goal(&mut store, "g1", &["t1", "t2"]);
        onboard(&mut store, "g1", "agent-a", "/definitely/not/here/wt1");
        onboard(&mut store, "g1", "agent-b", "/definitely/not/here/wt1");
        let lease_args = |todo: &str, agent: &str, extra: &[&str]| {
            let mut v = vec!["claim".to_string()];
            v.extend(claim_args("g1", todo, agent, &["--lease-secs", "3600"]));
            v.extend(extra.iter().map(|s| s.to_string()));
            v
        };
        cmd_lease(&mut store, &lease_args("t1", "agent-b", &[])).unwrap();
        let err = cmd_lease(&mut store, &lease_args("t2", "agent-a", &[])).unwrap_err();
        assert!(format!("{err}").contains("workspace conflict"));
        cmd_lease(&mut store, &lease_args("t2", "agent-a", &["--force"])).unwrap();
        let locks = lock_events(&store, "g1");
        assert_eq!(locks.len(), 2);
        assert!(locks[1].2, "forced lease claim must record forced=true");
    }

    #[test]
    fn agent_list_rows_show_declared_workspaces_and_occupancy() {
        let mut goal = Goal::new("g1", "objective", "/tmp");
        goal.registered_agents = vec!["alice".to_string()];
        goal.agent_profiles = vec![crate::state::AgentProfile {
            id: "alice".to_string(),
            capabilities: vec![],
            workspaces: vec!["/repo/wt1".to_string()],
        }];
        // idle: declared, not occupied.
        let rows = agent_list_rows(&goal, &HashMap::new(), 1_000);
        assert_eq!(rows[0].workspaces, vec!["/repo/wt1".to_string()]);
        // running: occupancy marker on the declared path.
        let mut todo = Todo::advancement("t1", "work");
        todo.claimed_by = Some("alice".to_string());
        todo.lease_expires_at = Some(2_000);
        goal.todos.push(todo);
        let rows = agent_list_rows(&goal, &HashMap::new(), 1_000);
        assert_eq!(rows[0].workspaces, vec!["/repo/wt1 ✍".to_string()]);
        let json = serde_json::to_string(&rows).unwrap();
        assert!(json.contains("workspaces"));
    }

    #[test]
    fn describe_event_renders_workspace_lock() {
        let event = Event::WorkspaceLockAcquired {
            goal_id: "g1".to_string(),
            agent_id: "a1".to_string(),
            todo_id: "t1".to_string(),
            paths: vec!["/repo/wt1".to_string()],
            forced: true,
            ts: 1,
        };
        let text = describe_event(&event);
        assert!(text.contains("workspace_lock"), "got: {text}");
        assert!(text.contains("a1"), "got: {text}");
        assert!(text.contains("(forced)"), "got: {text}");
        // todo-event filtering picks the lock event up for its todo.
        assert!(event_touches_todo(&event, "t1"));
        assert!(!event_touches_todo(&event, "t2"));
    }
}

#[cfg(test)]
mod read_model_repair_cli_tests {
    use super::*;

    #[test]
    fn describe_event_renders_projection_repair() {
        let event = Event::ProjectionRepaired {
            goal_id: "g1".to_string(),
            projection: "run_index".to_string(),
            drift_count: 2,
            missing_rows: 1,
            stale_rows: 1,
            duplicate_rows: 0,
            rows_written: 3,
            backup_path: "/tmp/backup.jsonl".to_string(),
            ts: 1,
        };
        let text = describe_event(&event);
        assert!(text.contains("projection_repaired"), "got: {text}");
        assert!(text.contains("run_index"), "got: {text}");
        assert!(text.contains("drift=2"), "got: {text}");
        // The repair audit is goal-scoped, not todo-scoped.
        assert!(!event_touches_todo(&event, "t1"));
    }
}
#[cfg(test)]
mod event_touch_filter_tests {
    use super::*;

    #[test]
    fn todo_filter_covers_delivery_and_followthrough_events() {
        let delivery = Event::DeliveryOutcomeRecorded {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            outcome: "verified".into(),
            note: None,
            delivered_turn: 3,
            seq: 1,
            ts: 1,
        };
        assert!(event_touches_todo(&delivery, "t1"));
        assert!(!event_touches_todo(&delivery, "t2"));
        let follow = Event::FollowthroughCreated {
            goal_id: "g1".into(),
            source_todo_id: "t1".into(),
            followup_todo_id: "t9".into(),
            turns_overdue: 4,
            ts: 1,
        };
        // Visible from BOTH the source delivery and the derived follow-up.
        assert!(event_touches_todo(&follow, "t1"));
        assert!(event_touches_todo(&follow, "t9"));
        assert!(!event_touches_todo(&follow, "t2"));
    }
}

#[cfg(test)]
mod residual_branch_tests {
    use super::*;
    use crate::state::{Goal, Todo};
    use crate::store::{Event, Store};

    fn tmp_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        let store = Store::open(&root).unwrap();
        (dir, store)
    }

    fn registered(store: &mut Store, id: &str) {
        let goal = Goal::new(id, "obj", "/tmp");
        store.register(&goal).unwrap();
        store
            .append(Event::GoalStarted {
                goal_id: id.into(),
                ts: 1,
            })
            .unwrap();
    }

    // ── print_monitor_poll_plan: goal-vanished early return ────────────────
    #[test]
    fn monitor_poll_plan_missing_goal_returns_ok() {
        let (_dir, store) = tmp_store();
        // Goal never registered → replay() returns Ok(None) → early return.
        print_monitor_poll_plan(&store, "goal_missing").unwrap();
    }

    // ── record_tick_heartbeat: empty rrule → None branch ───────────────────
    #[test]
    fn record_tick_heartbeat_empty_rrule_records_none() {
        let (_dir, mut store) = tmp_store();
        registered(&mut store, "g");
        let state = crate::scheduler::state::SchedulerState {
            schema_version: String::new(),
            goal_id: "g".into(),
            agent_id: "a".into(),
            surface: String::new(),
            state_key: String::new(),
            reset_token: String::new(),
            identity_signature: String::new(),
            progression_index: 0,
            progression_minutes: vec![],
            last_applied_rrule: String::new(),
            updated_at: 0,
            host_update_failures: vec![],
        };
        record_tick_heartbeat(&mut store, "g", "a", "tick_next", &state).unwrap();
        // The heartbeat landed; empty rrule projected as no recurrence.
        let goal = store.replay("g").unwrap().unwrap();
        assert!(goal.todos.is_empty());
    }

    // ── write_liveness_inbox_alert: create_dir_all failure → early return ──
    #[test]
    fn liveness_inbox_alert_bails_when_inbox_dir_is_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("not-a-dir");
        std::fs::write(&blocker, "x").unwrap();
        let goal = Goal::new("g", "obj", &blocker.to_string_lossy());
        let eval = crate::scheduler::liveness::evaluate_liveness("g", "a", Some(1), 1000, 60);
        // `.future/loop/inbox` cannot be created under a regular file.
        write_liveness_inbox_alert(&goal, "a", &eval);
    }

    // ── auto_register_workspaces: empty vs non-empty cwd ───────────────────
    #[test]
    fn auto_register_workspaces_empty_cwd_declares_nothing() {
        assert!(auto_register_workspaces("").is_empty());
        assert_eq!(
            auto_register_workspaces("/tmp/w"),
            vec!["/tmp/w".to_string()]
        );
    }

    // ── print_run_index_self_heal: empty + non-empty backup ────────────────
    #[test]
    fn run_index_self_heal_prints_both_backup_variants() {
        let drift = crate::runtime::run_index::IndexDriftReport {
            goal_id: "g".into(),
            index_path: String::new(),
            index_rows: 0,
            run_files: 1,
            missing_rows: 1,
            stale_rows: 0,
            duplicate_rows: 0,
            drift_count: 1,
            repair_recommended: true,
            missing_identities: vec![],
            stale_identities: vec![],
        };
        let mk = |backup: String| crate::runtime::run_index::IndexRepairOutcome {
            drift: drift.clone(),
            rebuilt: crate::runtime::run_index::RebuildReport {
                index_path: String::new(),
                backup_path: backup,
                rows_written: 1,
                non_destructive: true,
            },
        };
        print_run_index_self_heal(&mk(String::new()));
        print_run_index_self_heal(&mk("index.pre-rebuild-123.jsonl".into()));
    }

    // ── record_delivery_if_advancement: advancement vs non-advancement ─────
    #[test]
    fn record_delivery_only_for_advancement_todos() {
        let (_dir, mut store) = tmp_store();
        registered(&mut store, "g");
        // Non-advancement (monitor) todo → early return, no delivery event.
        let mut goal = store.replay("g").unwrap().unwrap();
        goal.todos.push(Todo::monitor(
            "m1",
            "watch",
            std::time::Duration::from_secs(60),
        ));
        record_delivery_if_advancement(&mut store, &goal, "g", "m1", 3).unwrap();
        // Advancement todo → records a delivery outcome.
        let mut goal = store.replay("g").unwrap().unwrap();
        goal.todos.push(Todo::advancement("t1", "ship"));
        record_delivery_if_advancement(&mut store, &goal, "g", "t1", 3).unwrap();
        let goal = store.replay("g").unwrap().unwrap();
        assert!(goal.delivery_state("t1").is_some());
        assert!(goal.delivery_state("m1").is_none());
    }

    // ── run_index_self_heal: drifted index drives the repair summary ────────
    #[test]
    fn run_index_self_heal_detects_and_repairs_drift() {
        let (_dir, mut store) = tmp_store();
        registered(&mut store, "g");
        // Seed a run file (source of truth) with no index row → drift.
        let runs = store.goal_dir("g").join("runs");
        std::fs::create_dir_all(&runs).unwrap();
        std::fs::write(
            runs.join("a.json"),
            r#"{"timestamp":"123","turn":1,"terminal_state":"completed"}"#,
        )
        .unwrap();
        run_index_self_heal(&mut store, "g").unwrap();
        // A clean index now reports no drift (the None arm).
        run_index_self_heal(&mut store, "g").unwrap();
    }

    fn seed_overdue_delivery(store: &mut Store) {
        store
            .append(Event::DeliveryOutcomeRecorded {
                goal_id: "g".into(),
                todo_id: "t1".into(),
                outcome: crate::work_items::delivery_outcome::OUTCOME_DELIVERED.into(),
                note: None,
                delivered_turn: 1,
                seq: 1,
                ts: 2,
            })
            .unwrap();
        let run = crate::state::RunRecord {
            turn: 4,
            todo_id: "t1".into(),
            run_id: "run-1".into(),
            terminal_state: "completed".into(),
            error: None,
            tokens_in_delta: 1,
            tokens_out_delta: 2,
            cost_delta: 0.1,
            tools: vec!["shell".into()],
            evidence: "proof".into(),
            recorded_at: 1,
            spend_source: Some("run".into()),
            validation: None,
        };
        std::fs::write(
            store.goal_dir("g").join("runs.jsonl"),
            format!("{}\n", serde_json::to_string(&run).unwrap()),
        )
        .unwrap();
    }

    // ── run_followthrough_and_refresh: empty vs overdue-delivery ────────────
    #[test]
    fn followthrough_refresh_no_overdue_returns_same_goal() {
        let (_dir, mut store) = tmp_store();
        registered(&mut store, "g");
        let goal = store.replay("g").unwrap().unwrap();
        let next = run_followthrough_and_refresh(&mut store, "g", goal).unwrap();
        assert!(next.todos.is_empty());
    }

    #[test]
    fn followthrough_refresh_overdue_creates_followup_and_refreshes() {
        let (_dir, mut store) = tmp_store();
        registered(&mut store, "g");
        seed_overdue_delivery(&mut store);
        let goal = store.replay("g").unwrap().unwrap();
        let next = run_followthrough_and_refresh(&mut store, "g", goal).unwrap();
        assert!(
            next.todos.iter().any(|t| t.text.contains("Follow-through")),
            "a follow-through todo joined the frontier"
        );
    }

    #[cfg(unix)]
    #[test]
    fn followthrough_refresh_read_only_ledger_hits_error_edge() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, mut store) = tmp_store();
        registered(&mut store, "g");
        seed_overdue_delivery(&mut store);
        let ledger = store.goal_dir("g").join("events.jsonl");
        let mut perms = std::fs::metadata(&ledger).unwrap().permissions();
        perms.set_mode(0o444);
        std::fs::set_permissions(&ledger, perms).unwrap();
        let goal = store.replay("g").unwrap().unwrap();
        let err = run_followthrough_and_refresh(&mut store, "g", goal).unwrap_err();
        assert!(format!("{err:#}").contains("append"), "{err:#}");
        let mut perms = std::fs::metadata(&ledger).unwrap().permissions();
        perms.set_mode(0o644);
        std::fs::set_permissions(&ledger, perms).unwrap();
    }

    // ── render_premerge_gate: failing gate hits the bail arm ────────────────
    #[test]
    fn render_premerge_gate_failing_report_bails() {
        let report = crate::canary::PremergeGateReport {
            schema_version: "v1".into(),
            gate: crate::canary::GateDecision {
                gate: "premerge".into(),
                passed: false,
                goals_checked: 1,
                failed_checks: vec!["root_writable".into()],
                reason: "root_writable check failed".into(),
            },
            run: crate::canary::SmokeRunResult {
                schema_version: "v1".into(),
                profile_id: "premerge".into(),
                suite: "premerge".into(),
                all_passed: false,
                checks: vec![crate::canary::SmokeCheckOutcome {
                    id: "root_writable".into(),
                    module: "state".into(),
                    passed: false,
                    detail: "root NOT writable".into(),
                }],
            },
        };
        let err = render_premerge_gate(&report, false).unwrap_err();
        assert!(format!("{err:#}").contains("gate failed"), "{err:#}");
    }
}

#[cfg(test)]
mod todo_verify_hint_tests {
    use super::looks_like_code_todo;

    #[test]
    fn code_keywords_detect_implementation_todos() {
        // ASCII keywords (case-insensitive), Chinese keywords, and .rs paths.
        for text in [
            "cargo check -p future-loop",
            "Commit worktree changes",
            "clippy 全绿",
            "写测试用例",
            "修改代码",
            "修复编译错误",
            "refactor console.rs",
            "fix orchestration/loop/src/console.rs",
            "merge main 后补回归",
        ] {
            assert!(
                looks_like_code_todo(text),
                "{text:?} should look like a code todo"
            );
        }
    }

    #[test]
    fn ordinary_todos_do_not_trigger_the_hint() {
        for text in [
            "整理会议纪要",
            "ship the widget",
            "gate one",
            "approve the plan",
            "latest release notes", // "test" substring inside "latest" must not match
            "version 1.2.3 bump",
        ] {
            assert!(
                !looks_like_code_todo(text),
                "{text:?} should not look like a code todo"
            );
        }
    }
}
