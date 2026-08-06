//! LoopX-compatible on-disk projection.
//!
//! The event ledger stays the source of truth; this layer MATERIALIZES the
//! same file layout and structure the real LoopX persists, so the two
//! implementations produce comparable state:
//!
//!   <project>/GOAL.md                          — goal document
//!   <project>/.loopx/registry.json             — registry (LoopX schema v0.1)
//!   <project>/.codex/goals/<id>/ACTIVE_GOAL_STATE.md — active state (markdown,
//!     todo anchors `<!-- loopx:todo ... -->` exactly like LoopX)
//!   <runtime>/goals/<id>/runs/<ts>.json|.md    — per-run history + index.jsonl
//!
//! Field names and enum VALUES mirror LoopX's outputs (verified against a
//! fresh `loopx demo` run). Only the ledger + compat projection together are
//! a complete state; the markdown is a REBUILT projection, never a second
//! source of truth.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::json;

use crate::state::{Goal, TaskClass, Todo, TodoStatus};

/// LoopX URL-encodes spaces (%20) in anchor values.
fn url_encode(s: &str) -> String {
    s.replace(' ', "%20").replace('+', "%2B")
}

/// RFC3339-ish timestamp matching LoopX (e.g. 2026-08-05T11:03:14+08:00).
pub fn rfc3339(ts: u64) -> String {
    use chrono::{Local, TimeZone};
    let dt = Local
        .timestamp_opt(ts as i64, 0)
        .single()
        .unwrap_or_else(Local::now);
    dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

/// Map our TaskClass to the LoopX task_class value (already equal via serde,
/// but keep the mapping explicit here).
pub fn loopx_task_class(c: TaskClass) -> &'static str {
    match c {
        TaskClass::Advancement => "advancement_task",
        TaskClass::UserGate => "user_gate",
        TaskClass::UserAction => "user_action",
        TaskClass::Monitor => "continuous_monitor",
        TaskClass::Blocker => "blocker",
    }
}

pub fn loopx_status(s: TodoStatus) -> &'static str {
    match s {
        TodoStatus::Open => "open",
        TodoStatus::Done => "done",
        TodoStatus::Superseded => "superseded",
        TodoStatus::Deferred => "deferred",
        TodoStatus::Blocked => "blocked",
    }
}

// ── GOAL.md ────────────────────────────────────────────────────────────────

pub fn write_goal_doc(project: &str, objective: &str) -> Result<()> {
    fs::write(Path::new(project).join("GOAL.md"), format!("{objective}\n")).context("write GOAL.md")
}

// ── .loopx/registry.json (LoopX schema v0.1) ───────────────────────────────

pub fn write_registry(project: &str, goals: &[&Goal], runtime_root: &str) -> Result<()> {
    let dir = Path::new(project).join(".loopx");
    fs::create_dir_all(&dir)?;
    let goal_entries: Vec<serde_json::Value> = goals
        .iter()
        .map(|g| goal_registry_entry(g, project, runtime_root))
        .collect();
    let payload = json!({
        "schema_version": "0.1",
        "updated_at": chrono::Local::now().format("%Y-%m-%d").to_string(),
        "common_runtime_root": runtime_root,
        "goals": goal_entries,
    });
    let pretty = serde_json::to_string_pretty(&payload)?;
    fs::write(dir.join("registry.json"), pretty).context("write .loopx/registry.json")
}

fn goal_registry_entry(g: &Goal, project: &str, runtime_root: &str) -> serde_json::Value {
    let state_file = format!(".codex/goals/{}/ACTIVE_GOAL_STATE.md", g.goal_id);
    let _ = runtime_root;
    json!({
        "id": g.goal_id,
        "domain": "project-goal-control-plane",
        "status": "active",
        "role": "controller",
        "parent_goal_id": null,
        "repo": project,
        "state_file": state_file,
        "authority_sources": [
            {"kind": "goal_doc", "path": "GOAL.md", "role": "primary_goal_document"}
        ],
        "adapter": {"kind": "loopx_native_v0", "status": "connected"},
        "spawn_policy": {"mode": "default", "allowed": false, "max_children": 3, "allowed_domains": []},
        "coordination": {
            "write_scope": g.authority.write_scope,
            "requires_parent_approval": g.authority.requires_approval,
            "registered_agents": g.registered_agents,
        },
        "execution_profile": {
            "cadence": g.execution_profile.cadence,
            "minimum_scale": "multi_surface_or_implementation",
            "must_include": ["coherent_artifact", "targeted_validation", "state_writeback"],
            "spend_rule": g.execution_profile.spend_rule,
            "outcome_floor": {
                "required_when": "after_surface_progress_streak",
                "surface_streak_threshold": g.execution_profile.outcome_floor_streak_threshold,
                "outcome_markers": [],
                "surface_only_hints": [],
                "must_advance": ["primary_goal_outcome"],
            },
        },
        "guards": [
            "read-only by default",
            "do not mutate production systems without explicit user approval",
            "keep private evidence out of public commits",
        ],
        "next_probe": format!("loopx --registry .loopx/registry.json check --scan-root {project}"),
    })
}

// ── .codex/goals/<id>/ACTIVE_GOAL_STATE.md (LoopX markdown) ────────────────

pub fn write_active_state(project: &str, goal: &Goal) -> Result<()> {
    let dir = Path::new(project)
        .join(".codex")
        .join("goals")
        .join(&goal.goal_id);
    fs::create_dir_all(&dir)?;
    let md = render_active_state(goal);
    fs::write(dir.join("ACTIVE_GOAL_STATE.md"), md).context("write ACTIVE_GOAL_STATE.md")?;
    // Lock sidecar (file-tree parity; our real concurrency guard is the
    // advisory file lock in the store).
    fs::write(dir.join("ACTIVE_GOAL_STATE.md.lock"), "").ok();
    Ok(())
}

/// Render the ACTIVE_GOAL_STATE.md markdown (pub for the G-4 multi-projection
/// layer, which grades and redacts the same render through privacy lenses).
pub fn render_active_state(goal: &Goal) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str("status: active\n");
    out.push_str("owner_mode: goal\n");
    out.push_str(&format!("objective: {:?}\n", goal.objective));
    out.push_str(&format!(
        "updated_at: {}\n",
        rfc3339(crate::state::now_epoch())
    ));
    out.push_str(&format!("adapter_id: {}\n", goal.goal_id));
    out.push_str("---\n\n");
    out.push_str("# Active Goal State\n\n");
    out.push_str("## Objective\n\n");
    out.push_str(&format!("{}\n\n", goal.objective));
    out.push_str("## Authority Sources\n\n");
    if std::path::Path::new(&goal.cwd).join("GOAL.md").exists() {
        out.push_str("- Primary goal document: `GOAL.md`\n\n");
    } else {
        out.push_str("- No explicit goal document was provided during bootstrap.\n\n");
    }
    out.push_str("## Operating Contract\n\n");
    for line in [
        "Treat this file as the durable goal state for future agent ticks.",
        "Treat the authority sources above as the first context to inspect before acting.",
        "Read current project evidence before choosing the next action.",
        "Run a bounded progress segment when useful; it does not have to be one tiny step.",
        "Keep private evidence, credentials, local paths, and raw logs out of public commits.",
        "End each tick with changed files, validation, residual risk, and the next action.",
    ] {
        out.push_str(&format!("- {line}\n"));
    }
    out.push('\n');
    out.push_str("## Execution Profile\n\n");
    out.push_str(&format!(
        "- `cadence={} minimum=multi_surface_or_implementation include=coherent_artifact,targeted_validation,state_writeback spend_rule={} small_streak_threshold=2`\n",
        goal.execution_profile.cadence, goal.execution_profile.spend_rule
    ));
    out.push_str("- Repeated small-scale follow-through should expand the next delivery batch or report a blocker before spending quota.\n\n");
    out.push_str("## Non-Goals\n\n");
    out.push_str(
        "- Do not perform irreversible production operations without explicit approval.\n",
    );
    out.push_str("- Do not publish private project evidence.\n");
    out.push_str(
        "- Do not optimize for activity if no useful artifact or decision can be produced.\n\n",
    );

    // Todos, split by role (LoopX sections).
    let user_todos: Vec<&Todo> = goal
        .todos
        .iter()
        .filter(|t| t.role == crate::state::TodoRole::User)
        .collect();
    let agent_todos: Vec<&Todo> = goal
        .todos
        .iter()
        .filter(|t| t.role == crate::state::TodoRole::Agent)
        .collect();
    out.push_str("## User Todo / Owner Review Reading Queue\n\n");
    for t in &user_todos {
        out.push_str(&todo_line(t, &goal.history));
    }
    out.push('\n');
    out.push_str("## Agent Todo\n\n");
    for t in &agent_todos {
        out.push_str(&todo_line(t, &goal.history));
    }
    out.push('\n');
    out.push_str("## Next Action\n\n");
    let next_default = "Initial routing is owned by the connected domain adapter.";
    out.push_str(&format!(
        "- {}\n\n",
        goal.next_action.as_deref().unwrap_or(next_default)
    ));
    out.push_str("## Recent User Feedback\n\n");
    out.push_str("- Initialized by `loopx bootstrap`.\n\n");
    out.push_str("## Progress Ledger\n\n");
    if goal.history.is_empty() {
        out.push_str("- Created the initial goal state and registry connection.\n");
    } else {
        for r in goal.history.iter().rev().take(5) {
            out.push_str(&format!(
                "- turn {} ({}): todo={} state={} tools=[{}] evidence={}\n",
                r.turn,
                rfc3339(r.recorded_at),
                r.todo_id,
                r.terminal_state,
                r.tools.join(","),
                crate::decision::truncate(&r.evidence, 120)
            ));
        }
    }
    out
}

/// One todo bullet with the LoopX `<!-- loopx:todo ... -->` anchor.
fn todo_line(t: &Todo, _history: &[crate::state::RunRecord]) -> String {
    let is_default_advancement = t.class == TaskClass::Advancement && t.action_kind.is_none();
    // LoopX: deferred todos render with a "-" checkbox; done with "x".
    let checkbox = if t.status == TodoStatus::Deferred {
        "-"
    } else if t.status == TodoStatus::Done {
        "x"
    } else {
        " "
    };
    let mut line = if is_default_advancement {
        format!(
            "- [{checkbox}] {}\n  <!-- loopx:todo todo_id={} status={}",
            t.text,
            t.id,
            loopx_status(t.status),
        )
    } else {
        format!(
            "- [{checkbox}] {}\n  <!-- loopx:todo todo_id={} status={} task_class={}",
            t.text,
            t.id,
            loopx_status(t.status),
            loopx_task_class(t.class),
        )
    };
    if let Some(ak) = &t.action_kind {
        line.push_str(&format!(" action_kind={ak}"));
    } else if t.class == TaskClass::UserGate {
        line.push_str(" action_kind=goal_decision");
    }
    if let Some(rw) = &t.resume_when_text {
        line.push_str(&format!(" resume_when={rw}"));
    }
    // G-12: monitor metadata in the anchor (target / policy / cadence).
    if let Some(target) = &t.monitor_target {
        line.push_str(&format!(" monitor_target={}", url_encode(target)));
    }
    if let Some(policy) = &t.monitor_policy {
        line.push_str(&format!(" monitor_policy={}", url_encode(policy)));
    }
    if let Some(cadence) = &t.monitor_cadence {
        line.push_str(&format!(" cadence={cadence}"));
    }
    if let Some(note) = &t.note {
        line.push_str(&format!(" note={note}"));
    }
    if t.goal_bound {
        line.push_str(" goal_bound=true");
    }
    if t.global_gate {
        line.push_str(" global_gate=true");
    }
    if let Some(owner) = &t.claimed_by {
        line.push_str(&format!(" claimed_by={owner}"));
    }
    if t.no_follow_up {
        line.push_str(" no_followup=true");
    }
    if t.status == TodoStatus::Done {
        // LoopX completed anchors carry URL-encoded evidence + completed_at.
        if let Some(ev) = t.evidence.as_deref().filter(|e| !e.is_empty()) {
            line.push_str(&format!(" evidence={}", url_encode(ev)));
        }
        if let Some(ts) = t.completed_at {
            line.push_str(&format!(
                " completed_at={}",
                rfc3339(ts).replace('+', "%2B")
            ));
        }
    }
    line.push_str(" updated_at=");
    // LoopX encodes only '+' -> %2B; colons stay literal.
    let ts = rfc3339(t.updated_at).replace('+', "%2B");
    line.push_str(&ts);
    line.push_str(" -->\n");
    line
}

// ── <runtime>/goals/<id>/runs/ ─────────────────────────────────────────────

pub fn write_run(
    runtime_root: &str,
    goal_id: &str,
    record: &crate::state::RunRecord,
) -> Result<()> {
    let dir = PathBuf::from(runtime_root)
        .join("goals")
        .join(goal_id)
        .join("runs");
    fs::create_dir_all(&dir)?;
    // LoopX run-file names: 2026-08-05T11-03-14-08-00 (offset without +/:)
    let now = chrono::Local::now();
    // "+0800" -> "08-00" (LoopX inserts a dash between hours and minutes).
    let z = now.format("%z").to_string();
    let digits = z.trim_start_matches(['+', '-']);
    let sign = if z.starts_with('-') { "-" } else { "" };
    let (hh, mm) = digits.split_at(2);
    let offset = format!("{sign}{hh}-{mm}");
    let ts = format!("{}-{offset}", now.format("%Y-%m-%dT%H-%M-%S"));

    let json_payload = json!({
        "goal_id": goal_id,
        "timestamp": rfc3339(record.recorded_at),
        "turn": record.turn,
        "todo_id": record.todo_id,
        "run_id": record.run_id,
        "terminal_state": record.terminal_state,
        "tools": record.tools,
        "tokens_in": record.tokens_in_delta,
        "tokens_out": record.tokens_out_delta,
        "cost": record.cost_delta,
        "evidence": record.evidence,
        "error": record.error,
    });
    fs::write(
        dir.join(format!("{ts}.json")),
        serde_json::to_string_pretty(&json_payload)?,
    )?;

    let mut md = String::new();
    md.push_str(&format!("# Run {ts}\n\n"));
    md.push_str(&format!("- goal_id: {goal_id}\n"));
    md.push_str(&format!("- turn: {}\n", record.turn));
    md.push_str(&format!("- todo: {}\n", record.todo_id));
    md.push_str(&format!("- state: {}\n", record.terminal_state));
    md.push_str(&format!("- tools: {}\n", record.tools.join(", ")));
    md.push_str(&format!("- evidence: {}\n", record.evidence));
    fs::write(dir.join(format!("{ts}.md")), md)?;

    // index.jsonl (append)
    let index_line = format!(
        "{}\n",
        json!({
            "goal_id": goal_id,
            "timestamp": rfc3339(record.recorded_at),
            "path": format!("goals/{goal_id}/runs/{ts}.json"),
            "turn": record.turn,
            "classification": record.terminal_state,
        })
    );
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("index.jsonl"))
        .and_then(|mut f| {
            use std::io::Write;
            f.write_all(index_line.as_bytes())
        })
        .context("append runs/index.jsonl")?;
    Ok(())
}
