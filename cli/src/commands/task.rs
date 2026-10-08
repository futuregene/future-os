//! `future task` — manage FutureOS tasks (reusable prompt + trigger + full-permission runs).
//!
//! State lives in `<home>/.future/tasks/tasks.db`, shared with the desktop
//! tick loop and the remote bridge. The CLI is a plain client of that store:
//! writes land immediately, and the desktop's next tick picks them up. This
//! command never executes a run itself; `future task run` queues a request
//! that the desktop (or headless desktop) consumes.

use crate::help;
use crate::output::Output;
use future_tasks::{RunStatus, Store, Task};
use serde_json::json;

type Result<T> = std::result::Result<T, String>;

/// `future task <command> [args]`.
pub async fn task(command: Option<&str>, rest: &[String], out: &Output) -> Result<()> {
    match command {
        None | Some("--help" | "-h") => {
            out.log(help::TASK_HELP);
            Ok(())
        }
        Some("list") => list(rest, out),
        Some("show") => show(rest, out),
        Some("add") => add(rest, out),
        Some("edit") => edit(rest, out),
        Some("enable") => set_enabled(rest, out, true),
        Some("disable") => set_enabled(rest, out, false),
        Some("remove") => remove(rest, out),
        Some("run") => run(rest, out),
        Some("runs") => runs(rest, out),
        Some("output") => output(rest, out).await,
        Some("feedback") => feedback(rest, out),
        Some("upstream" | "deps") => upstream(rest, out),
        Some("prompt") => prompt(rest, out),
        Some(other) => Err(format!(
            "Unknown argument: {other}\nUsage: future task [list|show|add|edit|enable|disable|remove|run|runs|output|feedback|upstream|prompt] …\nRun `future task --help` for details."
        )),
    }
}

/// `future task output <run-id>` — the full answer a run gave.
///
/// A run's ledger entry carries a truncated summary (the head and tail, 2000
/// chars); this prints the conversation's actual last answer, which is what an
/// agent reading an upstream result needs when the summary is not enough. The
/// header names the run being read, because a task that reuses one conversation
/// writes every run's answer into it — the *last* one is not necessarily the
/// run that was asked for, and a reader who cannot tell would attribute the
/// wrong answer to the wrong run.
async fn output(args: &[String], out: &Output) -> Result<()> {
    let json_flag = args.iter().any(|a| a == "--json");
    let tail = flag_value(args, "--tail").and_then(|value| value.parse::<usize>().ok());
    let run_id = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .ok_or_else(|| "usage: future task output <run-id> [--tail N] [--json]".to_string())?;
    let store = open_store()?;
    let run = store
        .get_run(run_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("run not found: {run_id}"))?;
    let task = store.get_task(&run.task_id).map_err(|e| e.to_string())?;
    let session_id = run.session_id.clone().ok_or_else(|| {
        format!(
            "run {run_id} has no conversation (it {}).",
            run.error_message
                .as_deref()
                .unwrap_or("failed before prompting")
        )
    })?;
    let newer = store
        .list_runs_for_task(&run.task_id, 200)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|other| {
            other.session_id.as_deref() == Some(session_id.as_str())
                && other.started_at.unwrap_or(0) > run.started_at.unwrap_or(0)
        })
        .max_by_key(|other| other.started_at.unwrap_or(0));
    let text = crate::rpc::RunClient::new(&crate::rpc::grpc_addr())
        .last_assistant_text(&session_id)
        .await
        .map_err(|error| format!("could not read the run's conversation: {error}"))?;

    if json_flag {
        out.log(
            &serde_json::to_string_pretty(&json!({
                "runId": run.id,
                "taskId": run.task_id,
                "task": task.as_ref().map(|t| t.name.clone()),
                "status": format!("{:?}", run.status).to_lowercase(),
                "promptVersion": run.prompt_version,
                "sessionId": session_id,
                "finishedAt": run.finished_at,
                "summary": run.result_summary,
                "text": text,
                "newerRunOnSameConversation": newer.as_ref().map(|other| other.id.clone()),
            }))
            .map_err(|e| e.to_string())?,
        );
        return Ok(());
    }

    out.log(&format!(
        "{} · run {} · {} · prompt v{} · session {}",
        task.as_ref()
            .map(|t| t.name.as_str())
            .unwrap_or("(task gone)"),
        run.id,
        format!("{:?}", run.status).to_lowercase(),
        run.prompt_version
            .map(|v| v.to_string())
            .unwrap_or_else(|| "?".to_string()),
        session_id
    ));
    if let Some(other) = newer {
        out.log(&format!(
            "note: this conversation was used again by run {}; the text below is its latest answer.",
            other.id
        ));
    }
    out.log("");
    let body = tail.map_or_else(
        || text.clone(),
        |lines| {
            let all: Vec<&str> = text.lines().collect();
            all[all.len().saturating_sub(lines)..].join("\n")
        },
    );
    if body.trim().is_empty() {
        out.log("(the run's conversation has no assistant answer yet)");
    } else {
        out.log(&body);
    }
    Ok(())
}

/// `future task prompt <log|apply|revert> …`.
fn prompt(args: &[String], out: &Output) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("log") => prompt_log(&args[1..], out),
        Some("apply") => prompt_apply(&args[1..], out),
        Some("revert") => prompt_revert(&args[1..], out),
        _ => Err(
            "usage: future task prompt log|apply|revert <id|name> [revision-id]\n\
             \n  log     the prompt versions, newest first (who wrote each)\n\
             \n  apply   make a stored version the active prompt again\n\
             \n  revert  go back to the version before the active one"
                .to_string(),
        ),
    }
}

fn open_store() -> Result<Store> {
    let home = future_home();
    Store::open(&home).map_err(|e| format!("open tasks store: {e}"))
}

/// FutureOS home root (`<home>/agent`, `<home>/tasks`, …), normally `~/.future`.
/// `FUTURE_HOME` replaces the whole root, matching the agent.
fn future_home() -> std::path::PathBuf {
    if let Some(override_dir) = future_rpc::home::future_home_override() {
        return override_dir;
    }
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".future")
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn find_task(store: &Store, id_or_name: &str) -> Result<Task> {
    if let Some(t) = store.get_task(id_or_name).map_err(|e| e.to_string())? {
        return Ok(t);
    }
    store
        .find_task_by_name(id_or_name)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("task not found: {id_or_name}"))
}

/// Like [`find_task`], but a removed task still answers: its run ledger and
/// prompt history stay in the database (`remove` is a soft delete), and reading
/// that history is exactly what `runs`/`prompt log` are for. A live task always
/// wins, so a name reused after a removal reads as the new task.
fn find_task_for_history(store: &Store, id_or_name: &str) -> Result<Task> {
    if let Some(live) = store
        .find_task_by_name(id_or_name)
        .map_err(|e| e.to_string())?
    {
        return Ok(live);
    }
    if let Some(by_id) = store.get_task(id_or_name).map_err(|e| e.to_string())? {
        return Ok(by_id);
    }
    store
        .list_tasks(true)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|task| task.name == id_or_name)
        .max_by_key(|task| task.updated_at)
        .ok_or_else(|| format!("task not found: {id_or_name}"))
}

/// How a task is triggered, for the list and the detail header.
///
/// A task with dependencies and no schedule of its own reads as `dependency`:
/// that is what makes it run, and calling it `manual` would describe it as
/// something the user has to start by hand. A task with *both* keeps its
/// schedule here, and its edges show up in `future task upstream` / `deps`.
fn format_trigger(task: &Task, dep_count: usize) -> String {
    match task.trigger_kind {
        future_tasks::TriggerKind::Manual if dep_count > 0 => "dependency".to_string(),
        future_tasks::TriggerKind::Manual => "manual".to_string(),
        future_tasks::TriggerKind::Schedule => {
            let j = &task.trigger_json;
            match j.get("mode").and_then(|m| m.as_str()) {
                Some("once") => format!(
                    "once {} {}",
                    j.get("date").and_then(|v| v.as_str()).unwrap_or("?"),
                    j.get("time").and_then(|v| v.as_str()).unwrap_or("?")
                ),
                Some("interval") => format!(
                    "every {}m",
                    j.get("every_minutes").and_then(|v| v.as_i64()).unwrap_or(0)
                ),
                Some("daily") => format!(
                    "daily {}",
                    j.get("time").and_then(|v| v.as_str()).unwrap_or("?")
                ),
                Some("weekly") => {
                    let days = j
                        .get("days")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|d| d.as_str())
                                .collect::<Vec<_>>()
                                .join(",")
                        })
                        .unwrap_or_default();
                    format!(
                        "weekly {} {}",
                        days,
                        j.get("time").and_then(|v| v.as_str()).unwrap_or("?")
                    )
                }
                Some("monthly") => format!(
                    "monthly {} {}",
                    j.get("day").and_then(|v| v.as_i64()).unwrap_or(0),
                    j.get("time").and_then(|v| v.as_str()).unwrap_or("?")
                ),
                _ => "schedule".to_string(),
            }
        }
    }
}

fn format_ms(ms: Option<i64>) -> String {
    ms.map(|m| {
        chrono::DateTime::from_timestamp_millis(m)
            .map(|d| {
                d.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_else(|| m.to_string())
    })
    .unwrap_or_else(|| "-".to_string())
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
}

fn parse_duration_minutes(s: &str) -> Option<i64> {
    let s = s.trim();
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit())?);
    let n: i64 = num.parse().ok()?;
    match unit {
        "m" => Some(n),
        "h" => Some(n * 60),
        "d" => Some(n * 24 * 60),
        _ => None,
    }
}

/// Parse `A` or `A:failure` into an upstream reference and a condition.
/// The default condition is `success`, so the common case is just a name.
fn parse_dep_spec(spec: &str) -> Result<(String, future_tasks::DepOn)> {
    let (name, condition) = match spec.split_once(':') {
        Some((name, cond)) => (name, Some(cond)),
        None => (spec, None),
    };
    if name.is_empty() {
        return Err(format!("--depends-on needs a task name, got {spec:?}"));
    }
    let on = match condition {
        None | Some("success") => future_tasks::DepOn::Success,
        Some("failure") => future_tasks::DepOn::Failure,
        Some("completed") => future_tasks::DepOn::Completed,
        Some(other) => {
            return Err(format!(
                "unknown dependency condition {other:?}: expected success|failure|completed"
            ));
        }
    };
    Ok((name.to_string(), on))
}

/// Every `--depends-on` value, in order.
fn dep_specs(args: &[String]) -> Result<Vec<(String, future_tasks::DepOn)>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--depends-on" {
            let value = args
                .get(i + 1)
                .ok_or_else(|| "--depends-on requires a value".to_string())?;
            out.push(parse_dep_spec(value)?);
            i += 1;
        }
        i += 1;
    }
    Ok(out)
}

/// Write the dependency edges for `task_id`, resolving each upstream by id or
/// name. Replaces the existing edges so re-running `add`/`edit` is idempotent.
fn write_deps(store: &Store, task_id: &str, specs: &[(String, future_tasks::DepOn)]) -> Result<()> {
    for existing in store.list_deps(task_id).map_err(|e| e.to_string())? {
        store
            .remove_dep(task_id, &existing.upstream_task_id)
            .map_err(|e| e.to_string())?;
    }
    for (reference, on) in specs {
        let upstream = find_task(store, reference)?;
        if upstream.id == task_id {
            return Err(format!("a task cannot depend on itself: {reference}"));
        }
        store
            .add_dep(&future_tasks::TaskDep {
                task_id: task_id.to_string(),
                upstream_task_id: upstream.id,
                on: *on,
            })
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The dependency edges as shown by `upstream`, with each edge's progress.
fn dep_rows(store: &Store, task_id: &str) -> Result<Vec<serde_json::Value>> {
    let mut rows = Vec::new();
    for dep in store.list_deps(task_id).map_err(|e| e.to_string())? {
        let upstream = store
            .get_task(&dep.upstream_task_id)
            .map_err(|e| e.to_string())?;
        let satisfied = store
            .get_dep_state(task_id, &dep.upstream_task_id)
            .map_err(|e| e.to_string())?
            .and_then(|state| state.satisfied_run_id);
        rows.push(json!({
            "upstreamTaskId": dep.upstream_task_id,
            "upstreamName": upstream.map(|t| t.name),
            "on": format!("{:?}", dep.on).to_lowercase(),
            "satisfied": satisfied.is_some(),
            "satisfiedRunId": satisfied,
        }));
    }
    Ok(rows)
}

/// The resolved identity for a `<id|name>` argument, so the commands can print
/// something a caller can use in the next invocation.
fn task_json(t: &Task, dep_count: usize) -> serde_json::Value {
    json!({
        "id": t.id,
        "name": t.name,
        "enabled": t.enabled,
        "trigger": format_trigger(t, dep_count),
        "depCount": dep_count,
        "nextDueAt": t.next_due_at,
        "queued": t.pending_request_at.is_some(),
        "sessionPolicy": format!("{:?}", t.session_policy).to_lowercase(),
        "conversationMode": format!("{:?}", t.conversation_mode).to_lowercase(),
        "reflection": format!("{:?}", t.reflection).to_lowercase(),
    })
}

/// The next slot for a schedule, computed on an enabled clone of `task`: a
/// paused task still resolves its schedule, so pausing and re-enabling does not
/// lose the next due time.
fn compute_next_due(t: &Task, now: i64) -> Option<i64> {
    if t.trigger_kind != future_tasks::TriggerKind::Schedule {
        return None;
    }
    let mut probe = t.clone();
    probe.enabled = true;
    probe.next_due_at = None;
    future_tasks::next_due(&probe, now - 1)
}

fn list(args: &[String], out: &Output) -> Result<()> {
    let json_flag = args.iter().any(|a| a == "--json");
    let include_deleted = args.iter().any(|a| a == "--all");
    let store = open_store()?;
    let tasks = store
        .list_tasks(include_deleted)
        .map_err(|e| e.to_string())?;
    // One query for every edge, so the trigger column can tell a dependency task
    // from one that only ever runs on request.
    let dep_counts = dep_counts(&store)?;
    if json_flag {
        let items: Vec<_> = tasks
            .iter()
            .map(|t| {
                json!({
                    "id": t.id,
                    "name": t.name,
                    "enabled": t.enabled,
                    "trigger": format_trigger(t, dep_count_of(&dep_counts, &t.id)),
                    "depCount": dep_count_of(&dep_counts, &t.id),
                    "nextDueAt": t.next_due_at,
                    "queued": t.pending_request_at.is_some(),
                    "sessionPolicy": format!("{:?}", t.session_policy).to_lowercase(),
                    "conversationMode": format!("{:?}", t.conversation_mode).to_lowercase(),
                    "reflection": format!("{:?}", t.reflection).to_lowercase(),
                })
            })
            .collect();
        out.log(&serde_json::to_string_pretty(&items).map_err(|e| e.to_string())?);
        return Ok(());
    }
    if tasks.is_empty() {
        out.log("No tasks. Create one with `future task add --name … --prompt … --cwd …`.");
        return Ok(());
    }
    for t in &tasks {
        let state = if t.enabled { "on" } else { "off" };
        let next = if t.trigger_kind == future_tasks::TriggerKind::Schedule {
            format!("next {}", format_ms(t.next_due_at))
        } else {
            "manual".to_string()
        };
        out.log(&format!(
            "{:<24} {:<20} {:<4} {:<24} {}",
            t.name,
            format_trigger(t, dep_count_of(&dep_counts, &t.id)),
            state,
            next,
            t.id
        ));
    }
    Ok(())
}

/// Upstream edge count per task id, from one query.
fn dep_counts(store: &Store) -> Result<std::collections::HashMap<String, usize>> {
    let mut counts = std::collections::HashMap::new();
    for dep in store.list_all_deps().map_err(|e| e.to_string())? {
        *counts.entry(dep.task_id).or_insert(0) += 1;
    }
    Ok(counts)
}

fn dep_count_of(counts: &std::collections::HashMap<String, usize>, task_id: &str) -> usize {
    counts.get(task_id).copied().unwrap_or(0)
}

fn show(args: &[String], out: &Output) -> Result<()> {
    let json_flag = args.iter().any(|a| a == "--json");
    let show_prompt = args.iter().any(|a| a == "--prompt");
    let id = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .ok_or_else(|| "usage: future task show <id|name> [--json] [--prompt]".to_string())?;
    let store = open_store()?;
    let t = find_task(&store, id)?;
    let dep_count = dep_count_of(&dep_counts(&store)?, &t.id);
    let latest = store
        .latest_run_for_task(&t.id)
        .map_err(|e| e.to_string())?;
    if json_flag {
        let value = json!({
            "id": t.id,
            "name": t.name,
            "enabled": t.enabled,
            "prompt": t.prompt,
            "promptVersion": t.prompt_version,
            "cwd": t.cwd,
            "modelId": t.model_id,
            "thinkingLevel": t.thinking_level,
            "sessionPolicy": format!("{:?}", t.session_policy).to_lowercase(),
            "conversationMode": format!("{:?}", t.conversation_mode).to_lowercase(),
            "triggerKind": format!("{:?}", t.trigger_kind).to_lowercase(),
            "trigger": t.trigger_json,
            "depJoin": format!("{:?}", t.dep_join).to_lowercase(),
            "depCount": dep_count,
            "nextDueAt": t.next_due_at,
            "queued": t.pending_request_at.is_some(),
            "reflection": format!("{:?}", t.reflection).to_lowercase(),
            "latestRun": latest.map(|r| json!({
                "id": r.id,
                "kind": format!("{:?}", r.kind).to_lowercase(),
                "status": format!("{:?}", r.status).to_lowercase(),
                "startedAt": r.started_at,
                "finishedAt": r.finished_at,
            })),
        });
        out.log(&serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?);
        return Ok(());
    }
    out.log(&format!("{}  ({})", t.name, t.id));
    out.log(&format!("  enabled:  {}", t.enabled));
    out.log(&format!("  trigger:  {}", format_trigger(&t, dep_count)));
    if t.trigger_kind == future_tasks::TriggerKind::Schedule {
        out.log(&format!("  next due: {}", format_ms(t.next_due_at)));
    }
    out.log(&format!("  cwd:      {}", t.cwd));
    out.log(&format!(
        "  model:    {}",
        t.model_id.as_deref().unwrap_or("(default)")
    ));
    out.log(&format!(
        "  thinking: {}",
        t.thinking_level.as_deref().unwrap_or("(default)")
    ));
    out.log(&format!(
        "  session:  {}",
        format!("{:?}", t.session_policy).to_lowercase()
    ));
    out.log(&format!(
        "  opens:    {}",
        format!("{:?}", t.conversation_mode).to_lowercase()
    ));
    out.log(&format!(
        "  reflect:  {}",
        format!("{:?}", t.reflection).to_lowercase()
    ));
    if let Some(r) = latest {
        out.log(&format!(
            "  last run: {} [{:?}] {}",
            r.id,
            r.status,
            format_ms(r.started_at)
        ));
    }
    if show_prompt {
        out.log("");
        out.log(&t.prompt);
    }
    Ok(())
}

fn add(args: &[String], out: &Output) -> Result<()> {
    let mut name: Option<String> = None;
    let mut prompt: Option<String> = None;
    let mut prompt_file: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut model: Option<String> = None;
    let mut thinking: Option<String> = None;
    let mut reflection = "ask".to_string();
    let mut session_policy = "new".to_string();
    let mut conversation_mode = "workspace".to_string();
    let mut trigger_json = serde_json::json!({});
    let mut trigger_kind = future_tasks::TriggerKind::Manual;
    let mut disabled = false;
    let mut dep_join = future_tasks::DepJoin::All;
    let mut json_flag = false;

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "--name" => {
                i += 1;
                name = Some(
                    args.get(i)
                        .cloned()
                        .ok_or_else(|| "--name requires a value".to_string())?,
                );
            }
            "--prompt" => {
                i += 1;
                prompt = Some(
                    args.get(i)
                        .cloned()
                        .ok_or_else(|| "--prompt requires a value".to_string())?,
                );
            }
            "--prompt-file" => {
                i += 1;
                prompt_file = Some(
                    args.get(i)
                        .cloned()
                        .ok_or_else(|| "--prompt-file requires a value".to_string())?,
                );
            }
            "--cwd" => {
                i += 1;
                cwd = Some(
                    args.get(i)
                        .cloned()
                        .ok_or_else(|| "--cwd requires a value".to_string())?,
                );
            }
            "--model" => {
                i += 1;
                model = Some(
                    args.get(i)
                        .cloned()
                        .ok_or_else(|| "--model requires a value".to_string())?,
                );
            }
            "--thinking" => {
                i += 1;
                thinking = Some(
                    args.get(i)
                        .cloned()
                        .ok_or_else(|| "--thinking requires a value".to_string())?,
                );
            }
            "--reflection" => {
                i += 1;
                reflection = args
                    .get(i)
                    .cloned()
                    .ok_or_else(|| "--reflection requires a value".to_string())?;
            }
            "--session" => {
                i += 1;
                session_policy = args
                    .get(i)
                    .cloned()
                    .ok_or_else(|| "--session requires a value".to_string())?;
            }
            "--conversation" => {
                i += 1;
                conversation_mode = args
                    .get(i)
                    .cloned()
                    .ok_or_else(|| "--conversation requires a value".to_string())?;
            }
            "--disabled" => disabled = true,
            "--manual" => {
                trigger_kind = future_tasks::TriggerKind::Manual;
                trigger_json = serde_json::json!({});
            }
            "--join-any" => dep_join = future_tasks::DepJoin::Any,
            "--depends-on" => {
                // Parsed as a group by `dep_specs`; skip its value here.
                i += 1;
            }
            "--json" => json_flag = true,
            "--at" => {
                i += 1;
                let v = args
                    .get(i)
                    .cloned()
                    .ok_or_else(|| "--at requires a value".to_string())?;
                let (date, time) = v
                    .split_once(' ')
                    .ok_or_else(|| "--at expects \"YYYY-MM-DD HH:MM\"".to_string())?;
                trigger_json = serde_json::json!({"mode":"once","date":date,"time":time});
                trigger_kind = future_tasks::TriggerKind::Schedule;
            }
            "--every" => {
                i += 1;
                let v = args
                    .get(i)
                    .cloned()
                    .ok_or_else(|| "--every requires a value".to_string())?;
                let mins = parse_duration_minutes(&v)
                    .ok_or_else(|| "--every expects e.g. 30m, 2h, 1d".to_string())?;
                let anchor = now_ms();
                trigger_json =
                    serde_json::json!({"mode":"interval","every_minutes":mins,"anchor":anchor});
                trigger_kind = future_tasks::TriggerKind::Schedule;
            }
            "--daily" => {
                let time = flag_value(args, "--time").unwrap_or_else(|| "09:00".to_string());
                trigger_json = serde_json::json!({"mode":"daily","time":time});
                trigger_kind = future_tasks::TriggerKind::Schedule;
            }
            "--weekly" => {
                let days = flag_value(args, "--days")
                    .ok_or_else(|| "--weekly requires --days mon,wed,fri".to_string())?;
                let days: Vec<&str> = days.split(',').map(|d| d.trim()).collect();
                let time = flag_value(args, "--time").unwrap_or_else(|| "09:00".to_string());
                trigger_json = serde_json::json!({"mode":"weekly","days":days,"time":time});
                trigger_kind = future_tasks::TriggerKind::Schedule;
            }
            "--monthly" => {
                let day = flag_value(args, "--day")
                    .and_then(|d| d.parse::<i64>().ok())
                    .ok_or_else(|| "--monthly requires --day N (1-31)".to_string())?;
                let time = flag_value(args, "--time").unwrap_or_else(|| "09:00".to_string());
                trigger_json = serde_json::json!({"mode":"monthly","day":day,"time":time});
                trigger_kind = future_tasks::TriggerKind::Schedule;
            }
            "--time" | "--days" | "--day" => {
                // consumed by their parent flag above
                i += 1;
            }
            other => {
                return Err(format!("unknown flag: {other}"));
            }
        }
        i += 1;
    }

    let name = name.ok_or_else(|| "--name is required".to_string())?;
    let prompt = match (prompt, prompt_file) {
        (Some(p), _) => p,
        (None, Some(f)) => {
            std::fs::read_to_string(&f).map_err(|e| format!("read prompt file {f}: {e}"))?
        }
        (None, None) => return Err("--prompt or --prompt-file is required".to_string()),
    };
    let cwd = cwd.ok_or_else(|| "--cwd is required".to_string())?;

    let reflection = match reflection.as_str() {
        "off" => future_tasks::Reflection::Off,
        "ask" => future_tasks::Reflection::Ask,
        "auto" => future_tasks::Reflection::Auto,
        _ => return Err("--reflection must be off|ask|auto".to_string()),
    };
    let session_policy = match session_policy.as_str() {
        "new" => future_tasks::SessionPolicy::New,
        "existing" => future_tasks::SessionPolicy::Existing,
        _ => return Err("--session must be new|existing".to_string()),
    };

    let conversation_mode = match conversation_mode.as_str() {
        "workspace" => future_tasks::ConversationMode::Workspace,
        "chat" => future_tasks::ConversationMode::Chat,
        _ => return Err("--conversation must be workspace|chat".to_string()),
    };

    let now = now_ms();
    let specs = dep_specs(args)?;
    let next_due = compute_next_due(
        &Task {
            id: String::new(),
            name: name.clone(),
            enabled: true,
            prompt: prompt.clone(),
            prompt_version: 1,
            cwd: cwd.clone(),
            model_id: model.clone(),
            thinking_level: thinking.clone(),
            session_policy,
            conversation_mode,
            thread_id: None,
            trigger_kind,
            trigger_json: trigger_json.clone(),
            dep_join,
            next_due_at: None,
            pending_request_at: None,
            pending_origin: None,
            pending_actor: None,
            reflection,
            created_at: now,
            updated_at: now,
            deleted_at: None,
        },
        now,
    );

    let t = Task {
        id: future_tasks::new_task_id(),
        name: name.clone(),
        enabled: !disabled,
        prompt,
        prompt_version: 1,
        cwd,
        model_id: model,
        thinking_level: thinking,
        session_policy,
        conversation_mode,
        thread_id: None,
        trigger_kind,
        trigger_json,
        dep_join,
        next_due_at: next_due,
        pending_request_at: None,
        pending_origin: None,
        pending_actor: None,
        reflection,
        created_at: now,
        updated_at: now,
        deleted_at: None,
    };

    let store = open_store()?;
    store.insert_task(&t).map_err(|e| e.to_string())?;
    write_deps(&store, &t.id, &specs)?;

    if json_flag {
        out.log(
            &serde_json::to_string_pretty(&json!({
                "id": t.id,
                "name": t.name,
                "nextDueAt": t.next_due_at,
                "dependsOn": specs.iter().map(|(name, on)| json!({
                    "upstream": name,
                    "on": format!("{on:?}").to_lowercase(),
                })).collect::<Vec<_>>(),
            }))
            .map_err(|e| e.to_string())?,
        );
    } else {
        out.log(&format!("Created task {} ({})", t.name, t.id));
        if let Some(due) = t.next_due_at {
            out.log(&format!("  next due: {}", format_ms(Some(due))));
        }
        for (name, on) in &specs {
            out.log(&format!(
                "  runs after: {name} ({})",
                format!("{on:?}").to_lowercase()
            ));
        }
    }
    Ok(())
}

/// `future task edit <id|name> [flags]` — change any part of a task.
///
/// A prompt change is recorded as a new version (source `user`), the same way
/// the desktop panel records one: the version history is what makes an
/// unattended prompt improvable rather than a single opaque string.
fn edit(args: &[String], out: &Output) -> Result<()> {
    let json_flag = args.iter().any(|a| a == "--json");
    // The reference is the first bare argument. Its *index* is what the flag
    // scan below skips: matching on the string would also skip a flag value
    // that happens to equal the task's name.
    let reference_index = args
        .iter()
        .position(|a| !a.starts_with('-'))
        .ok_or_else(|| "usage: future task edit <id|name> [--prompt …] [--cwd …]".to_string())?;
    let reference = args[reference_index].clone();

    let store = open_store()?;
    let mut task = find_task(&store, &reference)?;

    let mut prompt_file: Option<String> = None;
    let mut i = 0;
    let mut changed_prompt: Option<String> = None;
    let mut trigger_json: Option<serde_json::Value> = None;
    let mut trigger_kind: Option<future_tasks::TriggerKind> = None;
    let mut join: Option<future_tasks::DepJoin> = None;
    let mut enable: Option<bool> = None;
    let mut rename: Option<String> = None;
    // `--depends-on` replaces the whole edge set, so it is a flag *presence*
    // check rather than an incremental edit.
    let specs = dep_specs(args)?;
    let sets_deps = args.iter().any(|a| a == "--depends-on");

    while i < args.len() {
        if i == reference_index {
            i += 1;
            continue;
        }
        let arg = args[i].as_str();
        let value = |i: usize| -> Result<String> {
            args.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{arg} requires a value"))
        };
        match arg {
            "--name" => {
                rename = Some(value(i)?);
                i += 1;
            }
            "--prompt" => {
                changed_prompt = Some(value(i)?);
                i += 1;
            }
            "--prompt-file" => {
                prompt_file = Some(value(i)?);
                i += 1;
            }
            "--cwd" => {
                task.cwd = value(i)?;
                i += 1;
            }
            "--model" => {
                task.model_id = Some(value(i)?);
                i += 1;
            }
            "--thinking" => {
                task.thinking_level = Some(value(i)?);
                i += 1;
            }
            "--reflection" => {
                task.reflection = match value(i)?.as_str() {
                    "off" => future_tasks::Reflection::Off,
                    "ask" => future_tasks::Reflection::Ask,
                    "auto" => future_tasks::Reflection::Auto,
                    other => {
                        return Err(format!(
                            "unknown --reflection {other:?}: expected off|ask|auto"
                        ));
                    }
                };
                i += 1;
            }
            "--session" => {
                task.session_policy = match value(i)?.as_str() {
                    "new" => future_tasks::SessionPolicy::New,
                    "existing" => future_tasks::SessionPolicy::Existing,
                    other => {
                        return Err(format!(
                            "unknown --session {other:?}: expected new|existing"
                        ));
                    }
                };
                i += 1;
            }
            "--conversation" => {
                task.conversation_mode = match value(i)?.as_str() {
                    "workspace" => future_tasks::ConversationMode::Workspace,
                    "chat" => future_tasks::ConversationMode::Chat,
                    other => {
                        return Err(format!(
                            "unknown --conversation {other:?}: expected workspace|chat"
                        ));
                    }
                };
                i += 1;
            }
            "--manual" => {
                trigger_kind = Some(future_tasks::TriggerKind::Manual);
                trigger_json = Some(serde_json::json!({}));
            }
            "--at" => {
                let v = value(i)?;
                let (date, time) = v
                    .split_once(' ')
                    .ok_or_else(|| "--at expects \"YYYY-MM-DD HH:MM\"".to_string())?;
                trigger_json = Some(serde_json::json!({"mode":"once","date":date,"time":time}));
                trigger_kind = Some(future_tasks::TriggerKind::Schedule);
                i += 1;
            }
            "--every" => {
                let mins = parse_duration_minutes(&value(i)?)
                    .ok_or_else(|| "--every expects e.g. 30m, 2h, 1d".to_string())?;
                trigger_json = Some(
                    serde_json::json!({"mode":"interval","every_minutes":mins,"anchor":now_ms()}),
                );
                trigger_kind = Some(future_tasks::TriggerKind::Schedule);
                i += 1;
            }
            "--daily" => {
                let time = flag_value(args, "--time").unwrap_or_else(|| "09:00".to_string());
                trigger_json = Some(serde_json::json!({"mode":"daily","time":time}));
                trigger_kind = Some(future_tasks::TriggerKind::Schedule);
            }
            "--weekly" => {
                let days = flag_value(args, "--days")
                    .ok_or_else(|| "--weekly requires --days mon,wed,fri".to_string())?;
                let days: Vec<&str> = days.split(',').map(|d| d.trim()).collect();
                let time = flag_value(args, "--time").unwrap_or_else(|| "09:00".to_string());
                trigger_json = Some(serde_json::json!({"mode":"weekly","days":days,"time":time}));
                trigger_kind = Some(future_tasks::TriggerKind::Schedule);
            }
            "--monthly" => {
                let day = flag_value(args, "--day")
                    .and_then(|d| d.parse::<i64>().ok())
                    .ok_or_else(|| "--monthly requires --day N (1-31)".to_string())?;
                let time = flag_value(args, "--time").unwrap_or_else(|| "09:00".to_string());
                trigger_json = Some(serde_json::json!({"mode":"monthly","day":day,"time":time}));
                trigger_kind = Some(future_tasks::TriggerKind::Schedule);
            }
            "--join-any" => join = Some(future_tasks::DepJoin::Any),
            "--join-all" => join = Some(future_tasks::DepJoin::All),
            "--enable" => enable = Some(true),
            "--disable" => enable = Some(false),
            "--json" => {}
            "--time" | "--days" | "--day" => i += 1,
            "--depends-on" => i += 1,
            other => return Err(format!("unknown flag: {other}")),
        }
        i += 1;
    }

    if let Some(file) = prompt_file {
        changed_prompt = Some(
            std::fs::read_to_string(&file).map_err(|e| format!("read prompt file {file}: {e}"))?,
        );
    }
    if let Some(p) = changed_prompt {
        let history = store.list_revisions(&task.id).map_err(|e| e.to_string())?;
        // No reason prose: the field is shown verbatim in both UIs, so a
        // backend-invented English sentence would leak into a Chinese panel.
        let rows =
            future_tasks::prompt_change_revisions(&task, &history, &p, "user", None, now_ms());
        for row in &rows {
            store.insert_revision(row).map_err(|e| e.to_string())?;
        }
        if let Some(last) = rows.last() {
            // The helper owns the version arithmetic: applying its last row is
            // what advances the task.
            task.prompt = last.prompt.clone();
            task.prompt_version = last.version;
        }
    }
    if let Some(name) = rename {
        task.name = name;
    }
    if let Some(kind) = trigger_kind {
        task.trigger_kind = kind;
        task.trigger_json = trigger_json.unwrap_or_else(|| serde_json::json!({}));
        task.next_due_at = compute_next_due(&task, now_ms());
    }
    if let Some(j) = join {
        task.dep_join = j;
    }
    if let Some(on) = enable {
        task.enabled = on;
        // Re-enabling re-resolves the schedule: a task paused across its slot
        // would otherwise have no next due time and never fire again.
        if on && task.trigger_kind == future_tasks::TriggerKind::Schedule {
            task.next_due_at = compute_next_due(&task, now_ms());
        }
    }
    task.updated_at = now_ms();
    store.update_task(&task).map_err(|e| e.to_string())?;
    if sets_deps {
        write_deps(&store, &task.id, &specs)?;
    }

    let dep_count = store
        .list_deps(&task.id)
        .map(|deps| deps.len())
        .unwrap_or(0);
    if json_flag {
        out.log(
            &serde_json::to_string_pretty(&task_json(&task, dep_count))
                .map_err(|e| e.to_string())?,
        );
    } else {
        out.log(&format!("Updated task {} ({})", task.name, task.id));
        out.log(&format!("  trigger:  {}", format_trigger(&task, dep_count)));
        if task.prompt_version > 1 {
            out.log(&format!("  prompt:   v{}", task.prompt_version));
        }
    }
    Ok(())
}

/// `future task enable|disable <id|name>`.
fn set_enabled(args: &[String], out: &Output, enabled: bool) -> Result<()> {
    let reference = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .ok_or_else(|| "usage: future task enable|disable <id|name>".to_string())?;
    let store = open_store()?;
    let mut task = find_task(&store, reference)?;
    task.enabled = enabled;
    if enabled && task.trigger_kind == future_tasks::TriggerKind::Schedule {
        task.next_due_at = compute_next_due(&task, now_ms());
    }
    task.updated_at = now_ms();
    store.update_task(&task).map_err(|e| e.to_string())?;
    out.log(&format!(
        "{} is now {}",
        task.name,
        if enabled { "enabled" } else { "disabled" }
    ));
    if enabled {
        if let Some(due) = task.next_due_at {
            out.log(&format!("  next due: {}", format_ms(Some(due))));
        }
    }
    Ok(())
}

/// `future task remove <id|name> [--yes]`.
///
/// Soft-deletes the task and drops the dependency edges pointing at it, so no
/// downstream keeps waiting on something that can never report again. Runs and
/// the conversations they produced are left alone.
fn remove(args: &[String], out: &Output) -> Result<()> {
    let yes = args.iter().any(|a| a == "--yes");
    let reference = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .ok_or_else(|| "usage: future task remove <id|name> [--yes]".to_string())?;
    let store = open_store()?;
    let mut task = find_task(&store, reference)?;
    if !yes {
        return Err(format!(
            "refusing to remove {} ({}) without --yes\n  Its runs and conversations are kept.",
            task.name, task.id
        ));
    }
    task.deleted_at = Some(now_ms());
    task.enabled = false;
    task.updated_at = now_ms();
    store.update_task(&task).map_err(|e| e.to_string())?;
    store
        .remove_deps_pointing_at(&task.id)
        .map_err(|e| e.to_string())?;
    out.log(&format!("Removed task {} ({})", task.name, task.id));
    Ok(())
}

/// `future task feedback <run-id> good|bad [--note …]`.
fn feedback(args: &[String], out: &Output) -> Result<()> {
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    let run_id = positional
        .first()
        .ok_or_else(|| "usage: future task feedback <run-id> good|bad [--note …]".to_string())?;
    let verdict = positional
        .get(1)
        .ok_or_else(|| "usage: future task feedback <run-id> good|bad [--note …]".to_string())?;
    if verdict.as_str() != "good" && verdict.as_str() != "bad" {
        return Err(format!("verdict must be good|bad, got {verdict:?}"));
    }
    let note = flag_value(args, "--note");
    let store = open_store()?;
    let mut run = store
        .get_run(run_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("run not found: {run_id}"))?;
    run.feedback = Some(verdict.to_string());
    run.feedback_note = note;
    store.update_run(&run).map_err(|e| e.to_string())?;
    out.log(&format!("Recorded {verdict} on run {run_id}"));
    Ok(())
}

/// `future task upstream <id|name> [--json]` — the dependency edges and how far
/// each has got.
fn upstream(args: &[String], out: &Output) -> Result<()> {
    let json_flag = args.iter().any(|a| a == "--json");
    let reference = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .ok_or_else(|| "usage: future task upstream <id|name> [--json]".to_string())?;
    let store = open_store()?;
    let task = find_task(&store, reference)?;
    let rows = dep_rows(&store, &task.id)?;
    if json_flag {
        out.log(
            &serde_json::to_string_pretty(&json!({
                "id": task.id,
                "name": task.name,
                "join": format!("{:?}", task.dep_join).to_lowercase(),
                "upstream": rows,
            }))
            .map_err(|e| e.to_string())?,
        );
        return Ok(());
    }
    if rows.is_empty() {
        out.log(&format!("{} has no upstream tasks.", task.name));
        return Ok(());
    }
    out.log(&format!(
        "{} — join {:?}",
        task.name,
        format!("{:?}", task.dep_join).to_lowercase()
    ));
    for row in &rows {
        let name = row["upstreamName"]
            .as_str()
            .unwrap_or_else(|| row["upstreamTaskId"].as_str().unwrap_or("?"));
        let state = if row["satisfied"] == json!(true) {
            "satisfied"
        } else {
            "waiting"
        };
        out.log(&format!(
            "  {name:<24} on {:<10} {state}",
            row["on"].as_str().unwrap_or("?")
        ));
    }
    Ok(())
}

/// `future task prompt log <id|name> [--json]`.
fn prompt_log(args: &[String], out: &Output) -> Result<()> {
    let json_flag = args.iter().any(|a| a == "--json");
    let reference = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .ok_or_else(|| "usage: future task prompt log <id|name> [--json]".to_string())?;
    let store = open_store()?;
    // Suggestions and versions outlive the task's removal, like its runs do.
    let task = find_task_for_history(&store, reference)?;
    let revisions = store.list_revisions(&task.id).map_err(|e| e.to_string())?;
    if json_flag {
        let items: Vec<_> = revisions
            .iter()
            .map(|r| {
                json!({
                    "id": r.id,
                    // A suggestion is not a version yet: it carries
                    // `PROPOSAL_VERSION` (0) until it is applied.
                    "version": r.version,
                    "source": r.source,
                    "status": r.status,
                    "proposed": r.status == future_tasks::REVISION_STATUS_PROPOSED,
                    "reason": r.reason,
                    "confidence": r.confidence,
                    "sourceRunId": r.source_run_id,
                    "active": r.version == task.prompt_version,
                    "createdAt": r.created_at,
                })
            })
            .collect();
        out.log(&serde_json::to_string_pretty(&items).map_err(|e| e.to_string())?);
        return Ok(());
    }
    let (suggestions, versions): (Vec<_>, Vec<_>) = revisions
        .iter()
        .partition(|r| r.status == future_tasks::REVISION_STATUS_PROPOSED);
    out.log(&format!("{} — active v{}", task.name, task.prompt_version));
    if !suggestions.is_empty() {
        // A suggestion has no version and no effect until it is applied, so it
        // is listed on its own with the exact command that accepts it — a
        // proposal nobody can act on is the same as no proposal.
        out.log("Suggestions (not applied):");
        for r in &suggestions {
            let confidence = r
                .confidence
                .map(|c| format!("confidence {c:.2}"))
                .unwrap_or_else(|| "confidence ?".to_string());
            out.log(&format!(
                "  {:<16} {:<14} {:<16} {}",
                r.id,
                confidence,
                format_ms(Some(r.created_at)),
                r.reason.as_deref().unwrap_or("-")
            ));
            out.log(&format!(
                "    apply with: future task prompt apply {} {}",
                task.id, r.id
            ));
        }
    }
    if versions.is_empty() {
        out.log(&format!(
            "  only its original prompt (v{}) has been in force.",
            task.prompt_version
        ));
        return Ok(());
    }
    let mut ordered: Vec<&future_tasks::PromptRevision> = versions;
    ordered.sort_by_key(|r| r.version);
    for r in ordered {
        let mark = if r.version == task.prompt_version {
            "active"
        } else if r.status == future_tasks::REVISION_STATUS_APPLIED {
            "applied"
        } else {
            ""
        };
        // An accepted suggestion keeps its own identity: it is not a version,
        // so it is not printed as one ("v0" would invite someone to apply it
        // again, or to wonder which version the task came from).
        let label = if r.version == future_tasks::PROPOSAL_VERSION {
            "suggestion".to_string()
        } else {
            format!("v{}", r.version)
        };
        out.log(&format!(
            "  {label:<11} {mark:<8} {:<11} {:<16} {}",
            r.source,
            format_ms(Some(r.created_at)),
            r.reason.as_deref().unwrap_or("-")
        ));
    }
    Ok(())
}

/// `future task prompt apply <id|name> <revision-id>`.
fn prompt_apply(args: &[String], out: &Output) -> Result<()> {
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    let reference = positional
        .first()
        .ok_or_else(|| "usage: future task prompt apply <id|name> <revision-id>".to_string())?;
    let revision_id = positional
        .get(1)
        .ok_or_else(|| "usage: future task prompt apply <id|name> <revision-id>".to_string())?;
    let store = open_store()?;
    let mut task = find_task(&store, reference)?;
    let revision = store
        .list_revisions(&task.id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|r| &r.id == *revision_id)
        .ok_or_else(|| format!("revision not found: {revision_id}"))?;
    let history = store.list_revisions(&task.id).map_err(|e| e.to_string())?;
    // Applying a suggestion *is* accepting it: the version that goes live is
    // recorded as coming from reflection (with the suggestion's own reason),
    // and the suggestion itself is marked applied rather than left pending.
    let is_suggestion = revision.status == future_tasks::REVISION_STATUS_PROPOSED;
    let source = if is_suggestion {
        future_tasks::REVISION_SOURCE_REFLECTION
    } else {
        future_tasks::REVISION_SOURCE_ROLLBACK
    };
    let reason = if is_suggestion {
        revision.reason.clone()
    } else {
        Some(format!(
            "applied revision {} (v{})",
            revision_id, revision.version
        ))
    };
    let rows = future_tasks::prompt_change_revisions(
        &task,
        &history,
        &revision.prompt,
        source,
        reason.as_deref(),
        now_ms(),
    );
    let Some(applied) = rows.last() else {
        out.log(&format!(
            "{} already runs that version; nothing to change.",
            task.name
        ));
        return Ok(());
    };
    for row in &rows {
        store.insert_revision(row).map_err(|e| e.to_string())?;
    }
    if is_suggestion {
        store
            .set_revision_status(
                &task.id,
                &revision.id,
                future_tasks::REVISION_STATUS_APPLIED,
            )
            .map_err(|e| e.to_string())?;
    }
    let previous = task.prompt_version;
    task.prompt = applied.prompt.clone();
    task.prompt_version = applied.version;
    task.updated_at = now_ms();
    store.update_task(&task).map_err(|e| e.to_string())?;
    out.log(&format!(
        "{} now runs v{} (was v{})",
        task.name, task.prompt_version, previous
    ));
    Ok(())
}

/// `future task prompt revert <id|name>` — the version before the active one.
fn prompt_revert(args: &[String], out: &Output) -> Result<()> {
    let reference = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .ok_or_else(|| "usage: future task prompt revert <id|name>".to_string())?;
    let store = open_store()?;
    let task = find_task(&store, reference)?;
    let previous = store
        .list_revisions(&task.id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|r| r.version < task.prompt_version)
        .max_by_key(|r| r.version)
        .ok_or_else(|| format!("{} has no earlier version to revert to", task.name))?;
    prompt_apply(&[task.id.clone(), previous.id], out)
}

fn run(args: &[String], out: &Output) -> Result<()> {
    let wait = args.iter().any(|a| a == "--wait");
    let json_flag = args.iter().any(|a| a == "--json");
    let timeout_ms = flag_value(args, "--timeout")
        .and_then(|s| parse_duration_minutes(&s).map(|m| m * 60_000))
        .unwrap_or(15 * 60_000);
    let id = args.iter().find(|a| !a.starts_with('-')).ok_or_else(|| {
        "usage: future task run <id|name> [--wait] [--timeout 15m] [--json]".to_string()
    })?;
    let store = open_store()?;
    let t = find_task(&store, id)?;

    let now = now_ms();
    let mut task = t.clone();
    task.pending_request_at = Some(now);
    task.pending_origin = Some(future_tasks::RunOrigin::Cli);
    task.pending_actor = Some("cli".to_string());
    task.updated_at = now;
    store.update_task(&task).map_err(|e| e.to_string())?;

    if json_flag {
        out.log(
            &serde_json::to_string_pretty(&json!({
                "queued": true,
                "taskId": t.id,
                "taskName": t.name,
                "wait": wait,
            }))
            .map_err(|e| e.to_string())?,
        );
    } else {
        out.log(&format!("Queued run for {} ({})", t.name, t.id));
        if !wait {
            out.log(
                "  (the desktop/headless tick will pick this up; use --wait to block until done)",
            );
        }
    }

    if wait {
        return wait_for_run(&store, &t.id, now, timeout_ms, out, json_flag);
    }
    Ok(())
}

fn wait_for_run(
    store: &Store,
    task_id: &str,
    after_ms: i64,
    timeout_ms: i64,
    out: &Output,
    json_flag: bool,
) -> Result<()> {
    let deadline = after_ms + timeout_ms;
    loop {
        if now_ms() > deadline {
            return Err(
                "run did not finish within timeout; it may still be queued or running".to_string(),
            );
        }
        if let Some(run) = store
            .latest_run_for_task(task_id)
            .map_err(|e| e.to_string())?
        {
            if run.started_at.map(|s| s >= after_ms).unwrap_or(false)
                && matches!(
                    run.status,
                    RunStatus::Completed | RunStatus::Failed | RunStatus::Skipped
                )
            {
                if json_flag {
                    out.log(
                        &serde_json::to_string_pretty(&json!({
                            "status": format!("{:?}", run.status).to_lowercase(),
                            "runId": run.id,
                            "threadId": run.thread_id,
                            "sessionId": run.session_id,
                            "resultSummary": run.result_summary,
                            "startedAt": run.started_at,
                            "finishedAt": run.finished_at,
                            "errorMessage": run.error_message,
                        }))
                        .map_err(|e| e.to_string())?,
                    );
                } else {
                    out.log(&format!("Run {} [{:?}]", run.id, run.status));
                    if let Some(s) = &run.result_summary {
                        out.log(s);
                    }
                    if let Some(e) = &run.error_message {
                        out.log(&format!("error: {e}"));
                    }
                }
                return Ok(());
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

fn runs(args: &[String], out: &Output) -> Result<()> {
    let json_flag = args.iter().any(|a| a == "--json");
    let limit = flag_value(args, "--limit")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(20);
    let id = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .ok_or_else(|| "usage: future task runs <id|name> [--limit N] [--json]".to_string())?;
    let store = open_store()?;
    // A removed task still answers here: its ledger is the record of what it did.
    let t = find_task_for_history(&store, id)?;
    let runs = store
        .list_runs_for_task(&t.id, limit)
        .map_err(|e| e.to_string())?;
    if json_flag {
        let items: Vec<_> = runs
            .iter()
            .map(|r| {
                json!({
                    "id": r.id,
                    "kind": format!("{:?}", r.kind).to_lowercase(),
                    "origin": format!("{:?}", r.origin).to_lowercase(),
                    "actor": r.actor,
                    "status": format!("{:?}", r.status).to_lowercase(),
                    "dueAt": r.due_at,
                    "startedAt": r.started_at,
                    "finishedAt": r.finished_at,
                    "promptVersion": r.prompt_version,
                    "resultSummary": r.result_summary,
                    "errorMessage": r.error_message,
                    // A verdict recorded with `future task feedback` has to be
                    // readable back, or recording it is a write-only gesture.
                    "feedback": r.feedback,
                    "feedbackNote": r.feedback_note,
                    "threadId": r.thread_id,
                    "sessionId": r.session_id,
                })
            })
            .collect();
        out.log(&serde_json::to_string_pretty(&items).map_err(|e| e.to_string())?);
        return Ok(());
    }
    if runs.is_empty() {
        out.log("No runs yet.");
        return Ok(());
    }
    for r in &runs {
        let status = format!("{:?}", r.status).to_lowercase();
        let kind = format!("{:?}", r.kind).to_lowercase();
        // A verdict is short; show it on the same line as the status.
        let verdict = r
            .feedback
            .as_deref()
            .map(|f| format!(" [{f}]"))
            .unwrap_or_default();
        out.log(&format!(
            "{:<14} {:<10} {:<10}{:<10} {:<18} {}",
            r.id,
            kind,
            status,
            verdict,
            format_ms(r.started_at),
            r.error_message.as_deref().unwrap_or("")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env::EnvGuard;
    use std::path::PathBuf;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    /// A throwaway home with `HOME` pointed at it (matches the desktop/app).
    struct Home {
        dir: tempfile::TempDir,
        _env: EnvGuard,
    }

    impl Home {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let env = EnvGuard::set(&[("HOME", dir.path().as_os_str().to_owned())]);
            Home { dir, _env: env }
        }
        fn tasks_db(&self) -> PathBuf {
            self.dir
                .path()
                .join(".future")
                .join("tasks")
                .join("tasks.db")
        }
    }

    fn text(captured: std::sync::Arc<std::sync::Mutex<Vec<u8>>>) -> String {
        String::from_utf8(captured.lock().unwrap().clone()).expect("utf8")
    }

    #[tokio::test]
    async fn add_then_list_and_show() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (out, captured) = Output::memory();
        add(
            &args(&[
                "--name",
                "daily-report",
                "--prompt",
                "summarize yesterday's commits",
                "--cwd",
                "/tmp",
                "--daily",
            ]),
            &out,
        )
        .unwrap();
        assert!(home.tasks_db().exists(), "a write creates the database");
        assert!(text(captured.out).contains("Created task daily-report"));

        let (out, captured) = Output::memory();
        list(&[], &out).unwrap();
        let out_text = text(captured.out);
        assert!(out_text.contains("daily-report"), "{out_text}");
        assert!(out_text.contains("daily"), "{out_text}");

        let (out, captured) = Output::memory();
        show(&args(&["daily-report", "--prompt"]), &out).unwrap();
        let out_text = text(captured.out);
        assert!(
            out_text.contains("summarize yesterday's commits"),
            "{out_text}"
        );
    }

    #[tokio::test]
    async fn add_requires_name_prompt_and_cwd() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        let (out, _captured) = Output::memory();
        assert!(add(&args(&["--prompt", "p", "--cwd", "/tmp"]), &out).is_err());
        assert!(add(&args(&["--name", "n", "--cwd", "/tmp"]), &out).is_err());
        assert!(add(&args(&["--name", "n", "--prompt", "p"]), &out).is_err());
    }

    #[tokio::test]
    async fn monthly_short_month_rule_is_stored() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        let (out, captured) = Output::memory();
        add(
            &args(&[
                "--name",
                "month-end",
                "--prompt",
                "p",
                "--cwd",
                "/tmp",
                "--monthly",
                "--day",
                "31",
                "--json",
            ]),
            &out,
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert!(v["id"].is_string());
        assert!(v["nextDueAt"].is_number());
    }

    #[tokio::test]
    async fn run_queues_a_pending_request() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (out, _captured) = Output::memory();
        add(
            &args(&["--name", "q", "--prompt", "p", "--cwd", "/tmp"]),
            &out,
        )
        .unwrap();

        let store = Store::open(home.dir.path().join(".future").as_path()).unwrap();
        let t = store.find_task_by_name("q").unwrap().unwrap();
        assert!(t.pending_request_at.is_none());

        let (out, captured) = Output::memory();
        run(&args(&["q", "--json"]), &out).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert_eq!(v["queued"], true);

        let store = Store::open(home.dir.path().join(".future").as_path()).unwrap();
        let t = store.find_task_by_name("q").unwrap().unwrap();
        assert!(
            t.pending_request_at.is_some(),
            "run queued a pending request"
        );
    }

    #[tokio::test]
    async fn runs_lists_empty() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        let (out, _captured) = Output::memory();
        add(
            &args(&["--name", "r", "--prompt", "p", "--cwd", "/tmp"]),
            &out,
        )
        .unwrap();
        let (out, captured) = Output::memory();
        runs(&args(&["r"]), &out).unwrap();
        assert_eq!(text(captured.out), "No runs yet.\n");
    }

    // ─── dispatch ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn the_group_dispatches_every_subcommand() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        // A task to address, so the new read/write commands get past their
        // argument check and exercise their real bodies.
        let (setup, _c) = Output::memory();
        add(
            &args(&["--name", "t", "--prompt", "p", "--cwd", "/tmp"]),
            &setup,
        )
        .unwrap();
        for (values, expect_ok) in [
            (vec![], true),
            (vec!["--help"], true),
            (vec!["list"], true),
            (vec!["show"], false),
            (vec!["add"], false),
            (vec!["run"], false),
            (vec!["runs"], false),
            (vec!["edit"], false),
            (vec!["edit", "t"], true),
            (vec!["enable"], false),
            (vec!["enable", "t"], true),
            (vec!["disable", "t"], true),
            (vec!["upstream", "t"], true),
            (vec!["deps", "t"], true),
            (vec!["upstream"], false),
            (vec!["output"], false),
            // A run id that does not exist is still a usage-level failure, not a
            // panic: `output` reads the ledger before it touches the agent.
            (vec!["output", "trn_missing"], false),
            (vec!["feedback"], false),
            (vec!["prompt"], false),
            (vec!["prompt", "log", "t"], true),
            (vec!["prompt", "apply", "t"], false),
            (vec!["prompt", "revert", "t"], false),
            // Removing is last: the commands above address `t`, and a deleted
            // task is no longer found by name.
            (vec!["remove", "t"], false),
            (vec!["remove", "t", "--yes"], true),
            (vec!["frobnicate"], false),
        ] {
            let (out, _captured) = Output::memory();
            let sub = values.first().copied().unwrap_or("list");
            let rest: Vec<String> = values.iter().skip(1).map(|v| (*v).to_string()).collect();
            let result = task(Some(sub), &rest, &out).await;
            assert_eq!(result.is_ok(), expect_ok, "{values:?}");
        }
        let (out, captured) = Output::memory();
        task(None, &[], &out).await.unwrap();
        assert!(text(captured.out).contains("future task — manage FutureOS tasks"));
    }

    // ─── edit ─────────────────────────────────────────────────────────────

    /// `edit` changes each field on the stored task, not just in its own
    /// output: the point of the CLI is that the desktop's next tick sees it.
    #[tokio::test]
    async fn edit_changes_every_field_on_the_stored_task() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let prompt_file = home.dir.path().join("next.md");
        std::fs::write(&prompt_file, "the second prompt").unwrap();
        let (setup, _c) = Output::memory();
        add(
            &args(&["--name", "before", "--prompt", "p", "--cwd", "/tmp"]),
            &setup,
        )
        .unwrap();

        let (out, _captured) = Output::memory();
        edit(
            &args(&[
                "before",
                "--name",
                "after",
                "--prompt-file",
                prompt_file.to_str().unwrap(),
                "--cwd",
                "/var",
                "--model",
                "future/gpt-5",
                "--thinking",
                "high",
                "--session",
                "existing",
                "--conversation",
                "chat",
                "--reflection",
                "off",
                "--daily",
                "--time",
                "07:30",
                "--join-any",
                "--json",
            ]),
            &out,
        )
        .unwrap();

        let store = Store::open(home.dir.path().join(".future").as_path()).unwrap();
        let t = store.find_task_by_name("after").unwrap().unwrap();
        assert_eq!(t.prompt, "the second prompt");
        assert_eq!(t.cwd, "/var");
        assert_eq!(t.model_id.as_deref(), Some("future/gpt-5"));
        assert_eq!(t.thinking_level.as_deref(), Some("high"));
        assert_eq!(t.session_policy, future_tasks::SessionPolicy::Existing);
        assert_eq!(t.conversation_mode, future_tasks::ConversationMode::Chat);
        assert_eq!(t.reflection, future_tasks::Reflection::Off);
        assert_eq!(t.dep_join, future_tasks::DepJoin::Any);
        assert_eq!(t.trigger_json["mode"], "daily");
        assert_eq!(t.trigger_json["time"], "07:30");
        assert!(t.next_due_at.is_some(), "a schedule resolves its next slot");
    }

    /// Switching back to manual clears the schedule: a manual task that kept a
    /// `next_due_at` would still be picked up by the tick.
    #[tokio::test]
    async fn edit_back_to_manual_clears_the_schedule() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (setup, _c) = Output::memory();
        add(
            &args(&[
                "--name", "sched", "--prompt", "p", "--cwd", "/tmp", "--daily",
            ]),
            &setup,
        )
        .unwrap();
        let store = Store::open(home.dir.path().join(".future").as_path()).unwrap();
        assert!(store
            .find_task_by_name("sched")
            .unwrap()
            .unwrap()
            .next_due_at
            .is_some());

        let (out, _captured) = Output::memory();
        edit(&args(&["sched", "--manual"]), &out).unwrap();
        let t = store.find_task_by_name("sched").unwrap().unwrap();
        assert_eq!(t.trigger_kind, future_tasks::TriggerKind::Manual);
        assert_eq!(t.next_due_at, None);
    }

    /// `--enable` / `--disable` on edit, including the re-enable that has to
    /// re-resolve the schedule.
    #[tokio::test]
    async fn edit_enable_and_disable_round_trip() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (setup, _c) = Output::memory();
        add(
            &args(&[
                "--name", "pause", "--prompt", "p", "--cwd", "/tmp", "--daily",
            ]),
            &setup,
        )
        .unwrap();
        let store = Store::open(home.dir.path().join(".future").as_path()).unwrap();

        let (out, _captured) = Output::memory();
        edit(&args(&["pause", "--disable"]), &out).unwrap();
        assert!(!store.find_task_by_name("pause").unwrap().unwrap().enabled);

        edit(&args(&["pause", "--enable"]), &out).unwrap();
        let t = store.find_task_by_name("pause").unwrap().unwrap();
        assert!(t.enabled);
        assert!(
            t.next_due_at.is_some(),
            "re-enabling lands on a real slot instead of never firing"
        );
    }

    /// `--depends-on` replaces the whole edge set, and a prompt edit keeps the
    /// version it replaced so the original stays reachable.
    #[tokio::test]
    async fn edit_replaces_dependencies_and_keeps_the_replaced_prompt() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (setup, _c) = Output::memory();
        for name in ["up-a", "up-b", "down"] {
            add(
                &args(&["--name", name, "--prompt", "p", "--cwd", "/tmp"]),
                &setup,
            )
            .unwrap();
        }
        let store = Store::open(home.dir.path().join(".future").as_path()).unwrap();

        let (out, _captured) = Output::memory();
        edit(&args(&["down", "--depends-on", "up-a"]), &out).unwrap();
        let down = store.find_task_by_name("down").unwrap().unwrap();
        assert_eq!(store.list_deps(&down.id).unwrap().len(), 1);
        assert_eq!(
            store.list_deps(&down.id).unwrap()[0].on,
            future_tasks::DepOn::Success,
            "a bare name means success"
        );

        edit(
            &args(&[
                "down",
                "--depends-on",
                "up-b:failure",
                "--depends-on",
                "up-a:completed",
            ]),
            &out,
        )
        .unwrap();
        let deps = store.list_deps(&down.id).unwrap();
        assert_eq!(deps.len(), 2, "the old edge was replaced, not added to");
        let on_for = |name: &str| {
            let id = store.find_task_by_name(name).unwrap().unwrap().id;
            deps.iter().find(|d| d.upstream_task_id == id).unwrap().on
        };
        assert_eq!(on_for("up-b"), future_tasks::DepOn::Failure);
        assert_eq!(on_for("up-a"), future_tasks::DepOn::Completed);

        // The prompt edit records v2 and keeps v1, so `prompt revert` can get
        // back to where the task started.
        let (edit_out, _c2) = Output::memory();
        edit(&args(&["down", "--prompt", "the second prompt"]), &edit_out).unwrap();
        let after = store.find_task_by_name("down").unwrap().unwrap();
        assert_eq!(after.prompt_version, 2);
        let versions: Vec<i64> = store
            .list_revisions(&after.id)
            .unwrap()
            .iter()
            .map(|r| r.version)
            .collect();
        assert!(versions.contains(&1), "v1 is kept: {versions:?}");
        assert!(versions.contains(&2));

        // A second edit is idempotent about the history: it does not add
        // another copy of v1.
        edit(&args(&["down", "--prompt", "the third"]), &edit_out).unwrap();
        let v1s = store
            .list_revisions(&after.id)
            .unwrap()
            .iter()
            .filter(|r| r.version == 1)
            .count();
        assert_eq!(v1s, 1);
    }

    #[tokio::test]
    async fn edit_refuses_bad_input_and_unknown_tasks() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        let (setup, _c) = Output::memory();
        add(
            &args(&["--name", "t", "--prompt", "p", "--cwd", "/tmp"]),
            &setup,
        )
        .unwrap();
        let (out, _captured) = Output::memory();
        for (values, expect) in [
            (vec!["t", "--reflection", "sometimes"], "off|ask|auto"),
            (vec!["t", "--session", "maybe"], "new|existing"),
            (vec!["t", "--conversation", "either"], "workspace|chat"),
            (vec!["t", "--bogus"], "unknown flag"),
            (vec!["t", "--cwd"], "requires a value"),
            (vec!["t", "--at", "nonsense"], "YYYY-MM-DD HH:MM"),
            (vec!["t", "--every", "fortnightly"], "30m, 2h, 1d"),
            (vec!["t", "--weekly"], "requires --days"),
            (vec!["t", "--monthly"], "requires --day"),
            (vec!["t", "--depends-on", "t"], "cannot depend on itself"),
            (vec!["t", "--depends-on", ""], "needs a task name"),
            (
                vec!["t", "--depends-on", "up:maybe"],
                "success|failure|completed",
            ),
            (vec!["t", "--depends-on", "ghost"], "not found"),
            (vec!["ghost", "--cwd", "/x"], "not found"),
            (
                vec!["t", "--prompt-file", "/does/not/exist"],
                "read prompt file",
            ),
        ] {
            let error = edit(&args(&values), &out).unwrap_err();
            assert!(
                error.contains(expect),
                "{values:?}: {error:?} should mention {expect:?}"
            );
        }
    }

    /// The positional task name is skipped by the flag scan by *index*: a flag
    /// value that happens to equal the task's name must still be read as a
    /// value. (Matching on the string skipped it and reported "unknown flag".)
    #[tokio::test]
    async fn edit_does_not_mistake_its_own_task_name_for_a_flag() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        let (setup, _c) = Output::memory();
        add(
            &args(&["--name", "same", "--prompt", "p", "--cwd", "/tmp"]),
            &setup,
        )
        .unwrap();
        let (out, _captured) = Output::memory();
        edit(&args(&["same", "--cwd", "same"]), &out).unwrap();
        edit(&args(&["same", "--prompt", "same"]), &out).unwrap();
    }

    // ─── enable / disable ─────────────────────────────────────────────────

    #[tokio::test]
    async fn enable_and_disable_report_and_persist() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (setup, _c) = Output::memory();
        add(
            &args(&[
                "--name", "watch", "--prompt", "p", "--cwd", "/tmp", "--daily",
            ]),
            &setup,
        )
        .unwrap();
        let store = Store::open(home.dir.path().join(".future").as_path()).unwrap();

        let (out, captured) = Output::memory();
        set_enabled(&args(&["watch"]), &out, false).unwrap();
        assert!(text(captured.out).contains("watch is now disabled"));
        assert!(!store.find_task_by_name("watch").unwrap().unwrap().enabled);

        let (out, captured) = Output::memory();
        set_enabled(&args(&["watch"]), &out, true).unwrap();
        let shown = text(captured.out);
        assert!(shown.contains("watch is now enabled"));
        assert!(shown.contains("next due:"));
        assert!(store.find_task_by_name("watch").unwrap().unwrap().enabled);

        // A manual task has no slot to print, and enabling it must not invent
        // one.
        let (out, _c2) = Output::memory();
        add(
            &args(&["--name", "hand", "--prompt", "p", "--cwd", "/tmp"]),
            &out,
        )
        .unwrap();
        let (out, captured) = Output::memory();
        set_enabled(&args(&["hand"]), &out, true).unwrap();
        assert!(!text(captured.out).contains("next due:"));
        assert_eq!(
            store
                .find_task_by_name("hand")
                .unwrap()
                .unwrap()
                .next_due_at,
            None
        );

        let (out, _c3) = Output::memory();
        assert!(set_enabled(&args(&[]), &out, true)
            .unwrap_err()
            .contains("usage: future task enable|disable"));
        assert!(set_enabled(&args(&["nope"]), &out, true)
            .unwrap_err()
            .contains("not found"));
    }

    // ─── remove ───────────────────────────────────────────────────────────

    /// Removing takes the edges that pointed at the task, so no downstream
    /// keeps waiting on something that can never report again. The runs are
    /// left alone: the conversations they produced are the user's.
    #[tokio::test]
    async fn remove_drops_the_edges_pointing_at_it_and_keeps_the_runs() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (setup, _c) = Output::memory();
        for name in ["up", "down"] {
            add(
                &args(&["--name", name, "--prompt", "p", "--cwd", "/tmp"]),
                &setup,
            )
            .unwrap();
        }
        let store = Store::open(home.dir.path().join(".future").as_path()).unwrap();
        let (out, _c2) = Output::memory();
        edit(&args(&["down", "--depends-on", "up"]), &out).unwrap();
        let down = store.find_task_by_name("down").unwrap().unwrap();
        let up = store.find_task_by_name("up").unwrap().unwrap();
        store
            .mark_dep_satisfied(&down.id, &up.id, "trn_1", 1)
            .unwrap();

        // Without --yes it refuses and changes nothing.
        let error = remove(&args(&["up"]), &out).unwrap_err();
        assert!(error.contains("--yes"), "{error}");
        assert!(store.find_task_by_name("up").unwrap().is_some());

        let (out, captured) = Output::memory();
        remove(&args(&["up", "--yes"]), &out).unwrap();
        assert!(text(captured.out).contains("Removed task up"));
        assert!(store.list_deps(&down.id).unwrap().is_empty());
        assert!(store.get_dep_state(&down.id, &up.id).unwrap().is_none());
        // The tombstone is what makes the removal visible to `list --all`.
        // `find_task_by_name` filters deleted rows, so read it by id.
        assert!(store
            .get_task(&up.id)
            .unwrap()
            .unwrap()
            .deleted_at
            .is_some());
        assert!(store.find_task_by_name("up").unwrap().is_none());
        assert!(
            store
                .list_tasks(false)
                .unwrap()
                .iter()
                .all(|t| t.name != "up"),
            "a plain list hides it"
        );

        let (out, _c3) = Output::memory();
        assert!(remove(&args(&[]), &out)
            .unwrap_err()
            .contains("usage: future task remove"));
        assert!(remove(&args(&["ghost", "--yes"]), &out)
            .unwrap_err()
            .contains("not found"));
    }

    // ─── feedback ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn feedback_records_a_verdict_on_the_run() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let store = Store::open(home.dir.path().join(".future").as_path()).unwrap();
        let (setup, _c) = Output::memory();
        add(
            &args(&["--name", "t", "--prompt", "p", "--cwd", "/tmp"]),
            &setup,
        )
        .unwrap();
        let task = store.find_task_by_name("t").unwrap().unwrap();
        store
            .insert_run(&future_tasks::TaskRun {
                id: "trn_fb".into(),
                task_id: task.id.clone(),
                kind: future_tasks::RunKind::Main,
                origin: future_tasks::RunOrigin::Schedule,
                actor: None,
                due_at: Some(1),
                status: RunStatus::Completed,
                thread_id: None,
                session_id: None,
                run_id: None,
                prompt_version: Some(1),
                result_summary: Some("too terse".into()),
                feedback: None,
                feedback_note: None,
                started_at: Some(1),
                finished_at: Some(2),
                error_message: None,
            })
            .unwrap();

        let (out, captured) = Output::memory();
        feedback(
            &args(&["trn_fb", "bad", "--note", "missed the trend"]),
            &out,
        )
        .unwrap();
        assert!(text(captured.out).contains("Recorded bad"));
        let run = store.get_run("trn_fb").unwrap().unwrap();
        assert_eq!(run.feedback.as_deref(), Some("bad"));
        assert_eq!(run.feedback_note.as_deref(), Some("missed the trend"));

        // good, with no note, clears the note rather than keeping a stale one.
        let (out, _c2) = Output::memory();
        feedback(&args(&["trn_fb", "good"]), &out).unwrap();
        let run = store.get_run("trn_fb").unwrap().unwrap();
        assert_eq!(run.feedback.as_deref(), Some("good"));
        assert_eq!(run.feedback_note, None);

        let (out, _c3) = Output::memory();
        for (values, expect) in [
            (vec![], "usage: future task feedback"),
            (vec!["trn_fb"], "usage: future task feedback"),
            (vec!["trn_fb", "maybe"], "must be good|bad"),
            (vec!["trn_missing", "good"], "run not found"),
        ] {
            let error = feedback(&args(&values), &out).unwrap_err();
            assert!(error.contains(expect), "{values:?}: {error:?}");
        }
    }

    // ─── upstream ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn upstream_reports_edges_and_their_progress() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (setup, _c) = Output::memory();
        for name in ["up", "down"] {
            add(
                &args(&["--name", name, "--prompt", "p", "--cwd", "/tmp"]),
                &setup,
            )
            .unwrap();
        }
        let store = Store::open(home.dir.path().join(".future").as_path()).unwrap();

        // No edges yet.
        let (out, captured) = Output::memory();
        upstream(&args(&["down"]), &out).unwrap();
        assert!(text(captured.out).contains("no upstream tasks"));

        let (out, _c2) = Output::memory();
        edit(
            &args(&["down", "--depends-on", "up:failure", "--join-any"]),
            &out,
        )
        .unwrap();

        let (out, captured) = Output::memory();
        upstream(&args(&["down"]), &out).unwrap();
        let shown = text(captured.out);
        assert!(shown.contains("up"));
        assert!(shown.contains("failure"));
        assert!(shown.contains("waiting"));

        let down = store.find_task_by_name("down").unwrap().unwrap();
        let up = store.find_task_by_name("up").unwrap().unwrap();
        store
            .mark_dep_satisfied(&down.id, &up.id, "trn_9", 1)
            .unwrap();
        let (out, captured) = Output::memory();
        upstream(&args(&["down"]), &out).unwrap();
        assert!(text(captured.out).contains("satisfied"));

        let (out, captured) = Output::memory();
        upstream(&args(&["down", "--json"]), &out).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert_eq!(parsed["join"], "any");
        assert_eq!(parsed["upstream"][0]["on"], "failure");
        assert_eq!(parsed["upstream"][0]["satisfied"], true);
        assert_eq!(parsed["upstream"][0]["satisfiedRunId"], "trn_9");
        assert_eq!(parsed["upstream"][0]["upstreamName"], "up");

        // The empty case in JSON, and the argument guards.
        let (out, captured) = Output::memory();
        upstream(&args(&["up", "--json"]), &out).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert_eq!(parsed["upstream"], serde_json::json!([]));

        let (out, _c3) = Output::memory();
        assert!(upstream(&args(&[]), &out)
            .unwrap_err()
            .contains("usage: future task upstream"));
    }

    // ─── prompt versions ──────────────────────────────────────────────────

    #[tokio::test]
    async fn prompt_log_lists_versions_and_labels_the_active_one() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        let (setup, _c) = Output::memory();
        add(
            &args(&["--name", "t", "--prompt", "first", "--cwd", "/tmp"]),
            &setup,
        )
        .unwrap();

        // Nothing has been changed yet.
        let (out, captured) = Output::memory();
        prompt_log(&args(&["t"]), &out).unwrap();
        assert!(text(captured.out).contains("only its original prompt"));

        let (out, _c2) = Output::memory();
        edit(&args(&["t", "--prompt", "second"]), &out).unwrap();
        edit(&args(&["t", "--prompt", "third"]), &out).unwrap();

        let (out, captured) = Output::memory();
        prompt_log(&args(&["t"]), &out).unwrap();
        let shown = text(captured.out);
        assert!(shown.contains("active v3"), "{shown}");
        assert!(
            shown.contains("v1"),
            "the original is in the history: {shown}"
        );
        assert!(shown.contains("superseded"), "{shown}");
        assert!(shown.contains("user"), "{shown}");
        assert!(!shown.contains("v0"), "versions start at 1: {shown}");

        let (out, captured) = Output::memory();
        prompt_log(&args(&["t", "--json"]), &out).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text(captured.out)).unwrap();
        let rows = parsed.as_array().unwrap();
        assert_eq!(rows.len(), 3, "v1, v2, v3: {rows:?}");
        assert_eq!(
            rows.iter().filter(|r| r["active"] == true).count(),
            1,
            "exactly one version is active"
        );

        let (out, _c3) = Output::memory();
        assert!(prompt_log(&args(&[]), &out)
            .unwrap_err()
            .contains("usage: future task prompt log"));
    }

    /// Applying a stored version makes it active again, and reverting walks
    /// back one step — the workflow the skill is built around.
    #[tokio::test]
    async fn prompt_apply_and_revert_move_the_active_version() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let store = Store::open(home.dir.path().join(".future").as_path()).unwrap();
        let (setup, _c) = Output::memory();
        add(
            &args(&["--name", "t", "--prompt", "first", "--cwd", "/tmp"]),
            &setup,
        )
        .unwrap();
        let (out, _c2) = Output::memory();
        edit(&args(&["t", "--prompt", "second"]), &out).unwrap();

        let task = store.find_task_by_name("t").unwrap().unwrap();
        let v1 = store
            .list_revisions(&task.id)
            .unwrap()
            .into_iter()
            .find(|r| r.version == 1)
            .expect("v1 was kept");

        let (out, captured) = Output::memory();
        prompt_apply(&args(&["t", &v1.id]), &out).unwrap();
        assert!(text(captured.out).contains("now runs v3 (was v2)"));
        let applied = store.find_task_by_name("t").unwrap().unwrap();
        assert_eq!(applied.prompt, "first");
        assert_eq!(applied.prompt_version, 3);

        // Re-applying the same version is a no-op, not a version bump.
        let (out, captured) = Output::memory();
        prompt_apply(&args(&["t", &v1.id]), &out).unwrap();
        assert!(text(captured.out).contains("already runs that version"));
        assert_eq!(
            store
                .find_task_by_name("t")
                .unwrap()
                .unwrap()
                .prompt_version,
            3
        );

        let (out, captured) = Output::memory();
        prompt_revert(&args(&["t"]), &out).unwrap();
        assert!(text(captured.out).contains("now runs v4 (was v3)"));
        let reverted = store.find_task_by_name("t").unwrap().unwrap();
        assert_eq!(reverted.prompt, "second", "the version before v3");

        let (out, _c3) = Output::memory();
        assert!(prompt_apply(&args(&["t"]), &out)
            .unwrap_err()
            .contains("usage: future task prompt apply"));
        assert!(prompt_apply(&args(&["t", "rev_missing"]), &out)
            .unwrap_err()
            .contains("revision not found"));

        // A task whose only version is the current one has nothing to revert to.
        let (out, _c4) = Output::memory();
        add(
            &args(&["--name", "fresh", "--prompt", "p", "--cwd", "/tmp"]),
            &out,
        )
        .unwrap();
        assert!(prompt_revert(&args(&["fresh"]), &out)
            .unwrap_err()
            .contains("no earlier version"));
        assert!(prompt_revert(&args(&[]), &out)
            .unwrap_err()
            .contains("usage: future task prompt revert"));
    }

    #[test]
    fn the_prompt_subcommand_refuses_an_unknown_action() {
        let (out, _captured) = Output::memory();
        for values in [vec![], vec!["frobnicate"]] {
            let error = prompt(&args(&values), &out).unwrap_err();
            assert!(error.contains("prompt log|apply|revert"), "{error}");
        }
    }

    // ─── dependency specs ─────────────────────────────────────────────────

    #[test]
    fn dependency_specs_parse_the_condition_and_its_default() {
        assert_eq!(
            parse_dep_spec("up").unwrap(),
            ("up".to_string(), future_tasks::DepOn::Success)
        );
        assert_eq!(
            parse_dep_spec("up:success").unwrap().1,
            future_tasks::DepOn::Success
        );
        assert_eq!(
            parse_dep_spec("up:failure").unwrap().1,
            future_tasks::DepOn::Failure
        );
        assert_eq!(
            parse_dep_spec("up:completed").unwrap().1,
            future_tasks::DepOn::Completed
        );
        assert!(parse_dep_spec("up:maybe")
            .unwrap_err()
            .contains("success|failure|completed"));
        assert!(parse_dep_spec("")
            .unwrap_err()
            .contains("needs a task name"));
        assert!(parse_dep_spec(":failure")
            .unwrap_err()
            .contains("needs a task name"));

        // Collection: repeated flags in order, and a missing value is refused.
        assert!(dep_specs(&args(&["--depends-on"]))
            .unwrap_err()
            .contains("requires a value"));
        assert!(dep_specs(&args(&["--name", "x"])).unwrap().is_empty());
        let specs = dep_specs(&args(&["--depends-on", "a", "--depends-on", "b:failure"])).unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].0, "a");
        assert_eq!(specs[1].1, future_tasks::DepOn::Failure);
    }

    // ─── trigger resolution ───────────────────────────────────────────────

    /// The next-slot helper answers for a schedule and stays silent for a
    /// manual task, so `--disabled`/`--disable` cannot lose the schedule.
    #[tokio::test]
    async fn next_due_helper_answers_only_for_schedules() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (setup, _c) = Output::memory();
        add(
            &args(&[
                "--name",
                "s",
                "--prompt",
                "p",
                "--cwd",
                "/tmp",
                "--daily",
                "--disabled",
            ]),
            &setup,
        )
        .unwrap();
        let store = Store::open(home.dir.path().join(".future").as_path()).unwrap();
        let paused = store.find_task_by_name("s").unwrap().unwrap();
        assert!(!paused.enabled);
        assert!(
            paused.next_due_at.is_some(),
            "a schedule created while disabled still carries its slot"
        );

        let mut manual = paused.clone();
        manual.trigger_kind = future_tasks::TriggerKind::Manual;
        assert_eq!(compute_next_due(&manual, 1_000), None);
        assert!(compute_next_due(&paused, 1_000).is_some());
    }

    /// The help text and the dispatcher must agree, in both directions.
    ///
    /// This is the guard for the failure mode that shipped once already: a
    /// command surface was described (to users, to the skill, to other agents)
    /// while the dispatcher had no arm for it, so the documented invocation
    /// answered "Unknown argument". Reading the help out of the binary is the
    /// only check that cannot drift, because both sides come from this file.
    #[test]
    fn the_help_text_and_the_dispatcher_list_the_same_commands() {
        // Every subcommand `task()` can route, including the aliases.
        let dispatched: std::collections::BTreeSet<&str> = [
            "list", "show", "add", "edit", "enable", "disable", "remove", "run", "runs", "output",
            "feedback", "upstream", "deps", "prompt",
        ]
        .into_iter()
        .collect();
        let prompt_actions_dispatched: std::collections::BTreeSet<&str> =
            ["log", "apply", "revert"].into_iter().collect();

        // What the help advertises: the first word on every `future task …`
        // usage line, with `enable|disable`-style pairs expanded.
        let mut advertised: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut advertised_actions: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        for line in help::TASK_HELP.lines() {
            let Some(rest) = line.trim().strip_prefix("future task ") else {
                continue;
            };
            let mut words = rest.split_whitespace();
            let Some(first) = words.next() else { continue };
            // `enable|disable` names two subcommands on one line; the title line
            // reads `future task — manage …` and names none.
            let names: Vec<&str> = first.split('|').collect();
            if !names
                .iter()
                .all(|n| n.chars().all(|c| c.is_ascii_lowercase()))
            {
                continue;
            }
            for name in names {
                advertised.insert(name.to_string());
                if name == "prompt" {
                    // `future task prompt log|apply|revert <id|name> …`: every
                    // lowercase token before the first placeholder names an
                    // action. Stopping at the placeholder is what makes this
                    // branch-free — the alternative (probe for "the next word")
                    // has a "no next word" arm no help line can reach.
                    let actions = words.by_ref().take_while(|word| {
                        word.chars().all(|c| c.is_ascii_lowercase() || c == '|')
                    });
                    advertised_actions
                        .extend(actions.flat_map(|group| group.split('|').map(str::to_string)));
                }
            }
        }

        assert_eq!(
            advertised,
            dispatched.iter().map(|s| (*s).to_string()).collect(),
            "the help and the dispatcher disagree about which subcommands exist"
        );
        assert_eq!(
            advertised_actions,
            prompt_actions_dispatched
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            "the help and the dispatcher disagree about `future task prompt`'s actions"
        );
    }

    /// `add` takes `--manual` explicitly (the same thing an add with no trigger
    /// flag produces), and records dependencies with their conditions.
    #[tokio::test]
    async fn add_manual_and_dependency_flags_are_recorded() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (out, _c) = Output::memory();
        add(
            &args(&["--name", "up", "--prompt", "p", "--cwd", "/tmp"]),
            &out,
        )
        .unwrap();

        // Explicit --manual, with a plain-text confirmation of the edges.
        let (out, captured) = Output::memory();
        add(
            &args(&[
                "--name",
                "down",
                "--prompt",
                "p",
                "--cwd",
                "/tmp",
                "--manual",
                "--depends-on",
                "up:failure",
                "--join-any",
            ]),
            &out,
        )
        .unwrap();
        let shown = text(captured.out);
        assert!(shown.contains("runs after: up (failure)"), "{shown}");

        let store = Store::open(home.dir.path().join(".future").as_path()).unwrap();
        let down = store.find_task_by_name("down").unwrap().unwrap();
        assert_eq!(down.trigger_kind, future_tasks::TriggerKind::Manual);
        assert_eq!(down.trigger_json, serde_json::json!({}));
        assert_eq!(down.dep_join, future_tasks::DepJoin::Any);
        assert_eq!(store.list_deps(&down.id).unwrap().len(), 1);

        // The JSON form names the edge as the caller wrote it.
        let (out, captured) = Output::memory();
        add(
            &args(&[
                "--name",
                "down2",
                "--prompt",
                "p",
                "--cwd",
                "/tmp",
                "--depends-on",
                "up:completed",
                "--json",
            ]),
            &out,
        )
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert_eq!(parsed["dependsOn"][0]["upstream"], "up");
        assert_eq!(parsed["dependsOn"][0]["on"], "completed");
    }

    /// Every trigger flag `edit` accepts, so a task can be moved between all the
    /// schedule shapes without being recreated (which would lose its runs).
    #[tokio::test]
    async fn edit_accepts_every_trigger_shape() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let store_home = home.dir.path().join(".future");
        let (out, _c) = Output::memory();
        add(
            &args(&["--name", "t", "--prompt", "p", "--cwd", "/tmp"]),
            &out,
        )
        .unwrap();

        for (values, mode, field, expect) in [
            (
                vec!["t", "--at", "2026-12-24 09:00"],
                "once",
                "date",
                "2026-12-24",
            ),
            (
                vec!["t", "--every", "2h"],
                "interval",
                "every_minutes",
                "120",
            ),
            (
                vec!["t", "--daily", "--time", "06:15"],
                "daily",
                "time",
                "06:15",
            ),
            (
                vec!["t", "--weekly", "--days", "mon,wed", "--time", "08:00"],
                "weekly",
                "time",
                "08:00",
            ),
            (
                vec!["t", "--monthly", "--day", "15", "--time", "09:30"],
                "monthly",
                "day",
                "15",
            ),
        ] {
            let (out, _captured) = Output::memory();
            edit(&args(&values), &out).unwrap();
            let store = Store::open(&store_home).unwrap();
            let t = store.find_task_by_name("t").unwrap().unwrap();
            assert_eq!(t.trigger_json["mode"], mode, "{values:?}");
            assert_eq!(
                t.trigger_json[field].to_string().trim_matches('"'),
                expect,
                "{values:?}"
            );
            assert!(t.next_due_at.is_some(), "{values:?} resolves a slot");
        }

        // A weekly edit without --time falls back to the default.
        let (out, _captured) = Output::memory();
        edit(&args(&["t", "--weekly", "--days", "sun"]), &out).unwrap();
        let store = Store::open(&store_home).unwrap();
        let t = store.find_task_by_name("t").unwrap().unwrap();
        assert_eq!(t.trigger_json["time"], "09:00");
        assert_eq!(t.trigger_json["days"][0], "sun");
    }

    /// A bare `--prompt` (not a file) records the new version too — the two
    /// spellings must not diverge.
    #[tokio::test]
    async fn edit_accepts_an_inline_prompt() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (out, _c) = Output::memory();
        add(
            &args(&["--name", "t", "--prompt", "first", "--cwd", "/tmp"]),
            &out,
        )
        .unwrap();
        let (out, captured) = Output::memory();
        edit(&args(&["t", "--prompt", "second", "--json"]), &out).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert!(parsed["id"].as_str().unwrap().starts_with("tsk_"));

        let store = Store::open(home.dir.path().join(".future").as_path()).unwrap();
        let t = store.find_task_by_name("t").unwrap().unwrap();
        assert_eq!(t.prompt, "second");
        assert_eq!(t.prompt_version, 2);
    }

    /// A bare task of the given trigger kind, for the output-shape tests.
    fn task_with_trigger(kind: future_tasks::TriggerKind) -> Task {
        Task {
            id: "tsk_1".into(),
            name: "n".into(),
            enabled: true,
            prompt: "p".into(),
            prompt_version: 1,
            cwd: "/tmp".into(),
            model_id: None,
            thinking_level: None,
            session_policy: future_tasks::SessionPolicy::New,
            conversation_mode: future_tasks::ConversationMode::Workspace,
            thread_id: None,
            trigger_kind: kind,
            trigger_json: serde_json::json!({}),
            dep_join: future_tasks::DepJoin::All,
            next_due_at: None,
            pending_request_at: None,
            pending_origin: None,
            pending_actor: None,
            reflection: future_tasks::Reflection::Ask,
            created_at: 1,
            updated_at: 1,
            deleted_at: None,
        }
    }

    #[test]
    fn task_json_carries_the_identity_a_caller_needs_next() {
        let t = task_with_trigger(future_tasks::TriggerKind::Manual);
        let mut t = t;
        t.pending_request_at = Some(5);
        let json = task_json(&t, 0);
        assert_eq!(json["id"], "tsk_1");
        assert_eq!(json["queued"], true);
        assert_eq!(json["trigger"], "manual");
        assert_eq!(json["depCount"], 0);
        assert_eq!(json["sessionPolicy"], "new");
    }

    /// The trigger reads as what actually starts the task: dependencies make it
    /// `dependency`, not `manual` (nobody starts it by hand), while a task with
    /// its own schedule keeps that schedule.
    #[test]
    fn the_trigger_label_distinguishes_dependencies_from_by_hand_runs() {
        let mut manual = task_with_trigger(future_tasks::TriggerKind::Manual);
        manual.trigger_json = serde_json::json!({});
        assert_eq!(format_trigger(&manual, 0), "manual");
        assert_eq!(format_trigger(&manual, 2), "dependency");

        let mut scheduled = manual.clone();
        scheduled.trigger_kind = future_tasks::TriggerKind::Schedule;
        scheduled.trigger_json = serde_json::json!({"mode": "daily", "time": "09:00"});
        assert_eq!(format_trigger(&scheduled, 0), "daily 09:00");
        assert_eq!(
            format_trigger(&scheduled, 2),
            "daily 09:00",
            "a task that also has upstreams still runs on its own schedule"
        );
    }

    // ─── store location ───────────────────────────────────────────────────

    #[tokio::test]
    async fn future_home_override_owns_the_store() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().unwrap();
        let override_dir = dir.path().join("isolated-home");
        let _env =
            crate::test_env::EnvGuard::set(&[("FUTURE_HOME", override_dir.as_os_str().to_owned())]);
        let (out, _captured) = Output::memory();
        add(
            &args(&["--name", "isolated", "--prompt", "p", "--cwd", "/tmp"]),
            &out,
        )
        .unwrap();
        assert!(
            override_dir.join("tasks").join("tasks.db").exists(),
            "FUTURE_HOME replaces the whole root"
        );
    }

    // ─── list / show output shapes ────────────────────────────────────────

    #[tokio::test]
    async fn list_json_reports_every_task_and_the_empty_case_is_stated() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        let (out, captured) = Output::memory();
        list(&[], &out).unwrap();
        assert!(text(captured.out).contains("No tasks."));

        let (out, _captured) = Output::memory();
        add(
            &args(&[
                "--name", "j", "--prompt", "p", "--cwd", "/tmp", "--every", "2h",
            ]),
            &out,
        )
        .unwrap();
        let (out, captured) = Output::memory();
        list(&args(&["--json"]), &out).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert_eq!(parsed[0]["name"], "j");
        assert_eq!(parsed[0]["trigger"], "every 120m");
        assert_eq!(parsed[0]["sessionPolicy"], "new");
        assert_eq!(parsed[0]["reflection"], "ask");
    }

    #[tokio::test]
    async fn show_names_every_trigger_shape_in_plain_and_json_output() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        let cases: [(&str, &[&str], &str); 6] = [
            ("m1", &["--at", "2026-12-24 09:00"], "once 2026-12-24 09:00"),
            ("m2", &["--every", "30m"], "every 30m"),
            ("m3", &["--daily", "--time", "08:15"], "daily 08:15"),
            (
                "m4",
                &["--weekly", "--days", "mon,wed", "--time", "10:00"],
                "weekly mon,wed 10:00",
            ),
            ("m5", &["--monthly", "--day", "31"], "monthly 31 09:00"),
            ("m6", &[], "manual"),
        ];
        for (name, flags, expected) in cases {
            let (out, _captured) = Output::memory();
            let mut values = vec!["--name", name, "--prompt", "p", "--cwd", "/tmp"];
            values.extend_from_slice(flags);
            add(&args(&values), &out).unwrap();

            let (out, captured) = Output::memory();
            show(&args(&[name, "--prompt"]), &out).unwrap();
            let printed = text(captured.out);
            assert!(printed.contains(expected), "{name}: {printed}");
            assert!(
                printed.contains("p"),
                "--prompt prints the prompt: {printed}"
            );
        }

        let (out, captured) = Output::memory();
        show(&args(&["m2", "--json"]), &out).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert_eq!(parsed["triggerKind"], "schedule");
        assert_eq!(parsed["depJoin"], "all");
    }

    #[tokio::test]
    async fn show_finds_a_task_by_id_and_reports_its_last_run() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (out, captured) = Output::memory();
        add(
            &args(&["--name", "byId", "--prompt", "p", "--cwd", "/tmp", "--json"]),
            &out,
        )
        .unwrap();
        let id = serde_json::from_str::<serde_json::Value>(&text(captured.out)).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();

        // A finished run shows up in the plain output and the JSON summary.
        let store = Store::open(home.dir.path().join(".future").as_path()).unwrap();
        store
            .insert_run(&future_tasks::TaskRun {
                id: future_tasks::new_run_id(),
                task_id: id.clone(),
                kind: future_tasks::RunKind::Main,
                origin: future_tasks::RunOrigin::Schedule,
                actor: None,
                due_at: Some(1),
                status: future_tasks::RunStatus::Completed,
                thread_id: None,
                session_id: None,
                run_id: None,
                prompt_version: Some(1),
                result_summary: Some("done".into()),
                feedback: None,
                feedback_note: None,
                started_at: Some(1_700_000_000_000),
                finished_at: Some(1_700_000_060_000),
                error_message: None,
            })
            .unwrap();

        let (out, captured) = Output::memory();
        show(&args(&[&id]), &out).unwrap();
        let printed = text(captured.out);
        assert!(printed.contains("last run:"), "{printed}");
        assert!(printed.contains("[Completed]"), "{printed}");

        let (out, captured) = Output::memory();
        show(&args(&[&id, "--json"]), &out).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert_eq!(parsed["latestRun"]["status"], "completed");
    }

    #[tokio::test]
    async fn show_refuses_an_unknown_task() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        let (out, _captured) = Output::memory();
        assert!(show(&args(&["nope"]), &out)
            .unwrap_err()
            .contains("task not found"));
    }

    // ─── add flag matrix ──────────────────────────────────────────────────

    #[tokio::test]
    async fn add_accepts_every_documented_flag() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let prompt_file = home.dir.path().join("prompt.md");
        std::fs::write(&prompt_file, "from a file").unwrap();
        let (out, captured) = Output::memory();
        add(
            &args(&[
                "--name",
                "full",
                "--prompt-file",
                prompt_file.to_str().unwrap(),
                "--cwd",
                "/tmp",
                "--model",
                "future/gpt-5",
                "--thinking",
                "high",
                "--session",
                "existing",
                "--conversation",
                "chat",
                "--reflection",
                "auto",
                "--monthly",
                "--day",
                "31",
                "--time",
                "07:30",
                "--disabled",
                "--json",
            ]),
            &out,
        )
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert!(parsed["nextDueAt"].is_number());

        let store = Store::open(home.dir.path().join(".future").as_path()).unwrap();
        let stored = store.find_task_by_name("full").unwrap().unwrap();
        assert_eq!(stored.prompt, "from a file");
        assert_eq!(stored.model_id.as_deref(), Some("future/gpt-5"));
        assert_eq!(stored.thinking_level.as_deref(), Some("high"));
        assert_eq!(stored.session_policy, future_tasks::SessionPolicy::Existing);
        assert_eq!(
            stored.conversation_mode,
            future_tasks::ConversationMode::Chat
        );
        assert_eq!(stored.reflection, future_tasks::Reflection::Auto);
        assert!(!stored.enabled);
        assert_eq!(stored.trigger_json["day"], 31);
    }

    #[tokio::test]
    async fn add_refuses_unknown_flags_and_missing_values() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        let (out, _captured) = Output::memory();
        let base = ["--name", "n", "--prompt", "p", "--cwd", "/tmp"];

        let mut values = args(&base);
        values.push("--nope".into());
        assert!(add(&values, &out).unwrap_err().contains("unknown flag"));

        for (index, flag) in [
            "--name",
            "--prompt",
            "--prompt-file",
            "--cwd",
            "--model",
            "--thinking",
            "--reflection",
            "--session",
            "--at",
            "--every",
        ]
        .into_iter()
        .enumerate()
        {
            // Every attempt gets its own name: the store allows one live task
            // per name, so a repeated name would fail for the wrong reason.
            let mut values = args(&[
                "--name",
                &format!("f{index}"),
                "--prompt",
                "p",
                "--cwd",
                "/tmp",
            ]);
            values.push(flag.into());
            assert!(
                add(&values, &out).is_err(),
                "{flag} without a value must fail"
            );
        }

        // `--time` / `--days` / `--day` belong to the trigger flag above them, so
        // a bare occurrence is consumed rather than reported as dangling.
        for (index, flag) in ["--time", "--days", "--day"].into_iter().enumerate() {
            let mut values = args(&[
                "--name",
                &format!("d{index}"),
                "--prompt",
                "p",
                "--cwd",
                "/tmp",
            ]);
            values.push(flag.into());
            assert!(add(&values, &out).is_ok(), "{flag} alone is ignored");
        }
    }

    #[tokio::test]
    async fn add_validates_trigger_and_policy_values() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        let (out, _captured) = Output::memory();
        let mut attempted = 0;
        let mut with = |extra: &[&str]| {
            attempted += 1;
            let mut values = args(&[
                "--name",
                &format!("n{attempted}"),
                "--prompt",
                "p",
                "--cwd",
                "/tmp",
            ]);
            values.extend(extra.iter().map(|v| v.to_string()));
            values
        };

        assert!(add(&with(&["--reflection", "sometimes"]), &out).is_err());
        assert!(add(&with(&["--session", "sometimes"]), &out).is_err());
        assert!(add(&with(&["--conversation", "sometimes"]), &out).is_err());
        assert!(
            add(&with(&["--at", "2026-12-24"]), &out).is_err(),
            "--at needs a time"
        );
        assert!(add(&with(&["--every", "soon"]), &out).is_err());
        assert!(add(&with(&["--every", "30x"]), &out).is_err());
        assert!(
            add(&with(&["--weekly", "--time", "10:00"]), &out).is_err(),
            "--weekly needs --days"
        );
        assert!(
            add(&with(&["--monthly", "--time", "10:00"]), &out).is_err(),
            "--monthly needs --day"
        );
        assert!(add(&with(&["--reflection", "off"]), &out).is_ok());
        assert!(add(&with(&["--every", "1d"]), &out).is_ok());

        // A prompt file that cannot be read is an error, and is only consulted
        // when no inline prompt was given.
        let file_only = args(&[
            "--name",
            "from-file",
            "--prompt-file",
            "/nonexistent/prompt.md",
            "--cwd",
            "/tmp",
        ]);
        assert!(add(&file_only, &out)
            .unwrap_err()
            .contains("read prompt file"));
    }

    // ─── run / runs ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn run_prints_where_the_request_went() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        let (out, _captured) = Output::memory();
        add(
            &args(&["--name", "q", "--prompt", "p", "--cwd", "/tmp"]),
            &out,
        )
        .unwrap();
        let (out, captured) = Output::memory();
        run(&args(&["q"]), &out).unwrap();
        let printed = text(captured.out);
        assert!(printed.contains("Queued run for q"), "{printed}");
        assert!(printed.contains("--wait"), "{printed}");
    }

    #[tokio::test]
    async fn run_wait_reports_the_finished_run() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (out, _captured) = Output::memory();
        add(
            &args(&["--name", "w", "--prompt", "p", "--cwd", "/tmp"]),
            &out,
        )
        .unwrap();

        // A terminal run that starts after the request is what `--wait` polls for.
        let store = Store::open(home.dir.path().join(".future").as_path()).unwrap();
        let task = store.find_task_by_name("w").unwrap().unwrap();
        let started = now_ms() + 60_000;
        store
            .insert_run(&future_tasks::TaskRun {
                id: future_tasks::new_run_id(),
                task_id: task.id.clone(),
                kind: future_tasks::RunKind::Manual,
                origin: future_tasks::RunOrigin::Cli,
                actor: Some("cli".into()),
                due_at: None,
                status: future_tasks::RunStatus::Completed,
                thread_id: Some("thr_1".into()),
                session_id: Some("sess_1".into()),
                run_id: None,
                prompt_version: Some(1),
                result_summary: Some("the summary".into()),
                feedback: None,
                feedback_note: None,
                started_at: Some(started),
                finished_at: Some(started + 1),
                error_message: None,
            })
            .unwrap();

        // The timeout is measured from the request time the caller reports, so
        // that is what a real `future task run --wait` passes.
        let request_at = now_ms();
        let (out, captured) = Output::memory();
        wait_for_run(&store, &task.id, request_at, 5_000, &out, false).unwrap();
        let printed = text(captured.out);
        assert!(printed.contains("[Completed]"), "{printed}");
        assert!(printed.contains("the summary"), "{printed}");

        let (out, captured) = Output::memory();
        wait_for_run(&store, &task.id, request_at, 5_000, &out, true).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert_eq!(parsed["status"], "completed");
        assert_eq!(parsed["threadId"], "thr_1");
        assert_eq!(parsed["resultSummary"], "the summary");
    }

    #[tokio::test]
    async fn run_wait_gives_up_after_the_timeout_instead_of_hanging() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (out, _captured) = Output::memory();
        add(
            &args(&["--name", "t", "--prompt", "p", "--cwd", "/tmp"]),
            &out,
        )
        .unwrap();
        let store = Store::open(home.dir.path().join(".future").as_path()).unwrap();
        let task = store.find_task_by_name("t").unwrap().unwrap();
        let (out, _captured) = Output::memory();
        // A zero timeout expires on the first poll: no run has been accepted.
        let error = wait_for_run(&store, &task.id, now_ms(), 0, &out, false).unwrap_err();
        assert!(error.contains("did not finish within timeout"), "{error}");
    }

    #[tokio::test]
    async fn run_wait_reports_an_error_run() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (out, _captured) = Output::memory();
        add(
            &args(&["--name", "e", "--prompt", "p", "--cwd", "/tmp"]),
            &out,
        )
        .unwrap();
        let store = Store::open(home.dir.path().join(".future").as_path()).unwrap();
        let task = store.find_task_by_name("e").unwrap().unwrap();
        store
            .insert_run(&future_tasks::TaskRun {
                id: future_tasks::new_run_id(),
                task_id: task.id.clone(),
                kind: future_tasks::RunKind::Main,
                origin: future_tasks::RunOrigin::Schedule,
                actor: None,
                due_at: Some(1),
                status: future_tasks::RunStatus::Failed,
                thread_id: None,
                session_id: None,
                run_id: None,
                prompt_version: Some(1),
                result_summary: None,
                feedback: None,
                feedback_note: None,
                started_at: Some(now_ms() + 60_000),
                finished_at: Some(now_ms() + 60_001),
                error_message: Some("agent unreachable".into()),
            })
            .unwrap();
        let (out, captured) = Output::memory();
        wait_for_run(&store, &task.id, now_ms(), 5_000, &out, false).unwrap();
        assert!(text(captured.out).contains("error: agent unreachable"));
    }

    #[tokio::test]
    async fn runs_lists_the_ledger_and_its_json_form() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (out, _captured) = Output::memory();
        add(
            &args(&["--name", "r", "--prompt", "p", "--cwd", "/tmp"]),
            &out,
        )
        .unwrap();
        let store = Store::open(home.dir.path().join(".future").as_path()).unwrap();
        let task = store.find_task_by_name("r").unwrap().unwrap();
        store
            .insert_run(&future_tasks::TaskRun {
                id: future_tasks::new_run_id(),
                task_id: task.id.clone(),
                kind: future_tasks::RunKind::Chain,
                origin: future_tasks::RunOrigin::Chain,
                actor: Some("task:tsk_up".into()),
                due_at: None,
                status: future_tasks::RunStatus::Skipped,
                thread_id: None,
                session_id: None,
                run_id: None,
                prompt_version: Some(2),
                result_summary: None,
                feedback: None,
                feedback_note: None,
                started_at: Some(1_700_000_000_000),
                finished_at: Some(1_700_000_001_000),
                error_message: Some("overlap".into()),
            })
            .unwrap();

        let (out, captured) = Output::memory();
        runs(&args(&["r", "--limit", "5"]), &out).unwrap();
        let printed = text(captured.out);
        assert!(printed.contains("chain"), "{printed}");
        assert!(printed.contains("skipped"), "{printed}");
        assert!(printed.contains("overlap"), "{printed}");

        let (out, captured) = Output::memory();
        runs(&args(&["r", "--json"]), &out).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert_eq!(parsed[0]["kind"], "chain");
        assert_eq!(parsed[0]["origin"], "chain");
        assert_eq!(parsed[0]["promptVersion"], 2);
        assert_eq!(parsed[0]["actor"], "task:tsk_up");
        assert_eq!(parsed[0]["feedback"], serde_json::Value::Null);
        assert_eq!(parsed[0]["threadId"], serde_json::Value::Null);

        // A verdict written by `feedback` is readable back through the same
        // ledger, in both the JSON and the table — otherwise recording one
        // would be a write-only gesture.
        let run_id = parsed[0]["id"].as_str().unwrap().to_string();
        let (out, _c2) = Output::memory();
        feedback(&args(&[&run_id, "bad", "--note", "too terse"]), &out).unwrap();

        let (out, captured) = Output::memory();
        runs(&args(&["r", "--json"]), &out).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert_eq!(parsed[0]["feedback"], "bad");
        assert_eq!(parsed[0]["feedbackNote"], "too terse");

        let (out, captured) = Output::memory();
        runs(&args(&["r"]), &out).unwrap();
        let printed = text(captured.out);
        assert!(printed.contains("[bad]"), "{printed}");
    }

    #[tokio::test]
    async fn runs_rejects_a_non_numeric_limit() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        let (out, _captured) = Output::memory();
        add(
            &args(&["--name", "r", "--prompt", "p", "--cwd", "/tmp"]),
            &out,
        )
        .unwrap();
        // An unparsable limit falls back to the default rather than failing.
        let (out, captured) = Output::memory();
        runs(&args(&["r", "--limit", "lots"]), &out).unwrap();
        assert!(text(captured.out).contains("No runs yet."));
    }

    #[tokio::test]
    async fn wait_reports_a_run_once_it_settles() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (out, _captured) = Output::memory();
        add(
            &args(&["--name", "settling", "--prompt", "p", "--cwd", "/tmp"]),
            &out,
        )
        .unwrap();
        let store = Store::open(home.dir.path().join(".future").as_path()).unwrap();
        let task = store.find_task_by_name("settling").unwrap().unwrap();
        let started = now_ms() + 60_000;
        store
            .insert_run(&future_tasks::TaskRun {
                id: future_tasks::new_run_id(),
                task_id: task.id.clone(),
                kind: future_tasks::RunKind::Manual,
                origin: future_tasks::RunOrigin::Cli,
                actor: Some("cli".into()),
                due_at: None,
                status: future_tasks::RunStatus::Completed,
                thread_id: None,
                session_id: None,
                run_id: None,
                prompt_version: Some(1),
                result_summary: None,
                feedback: None,
                feedback_note: None,
                started_at: Some(started),
                finished_at: Some(started + 1),
                error_message: None,
            })
            .unwrap();

        // `future task run --wait` hands its own clock to the wait, so this is
        // the shape a caller sees.
        let (out, captured) = Output::memory();
        run(&args(&["settling", "--wait", "--timeout", "5m"]), &out).unwrap();
        let printed = text(captured.out);
        assert!(printed.contains("[Completed]"), "{printed}");
    }

    #[tokio::test]
    async fn wait_keeps_polling_while_the_run_is_still_going() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (out, _captured) = Output::memory();
        add(
            &args(&["--name", "busy", "--prompt", "p", "--cwd", "/tmp"]),
            &out,
        )
        .unwrap();
        let store = Store::open(home.dir.path().join(".future").as_path()).unwrap();
        let task = store.find_task_by_name("busy").unwrap().unwrap();
        // A run that belongs to this request but has not finished is not a
        // result: the wait polls again (one interval, then its deadline).
        let started = now_ms() + 60_000;
        store
            .insert_run(&future_tasks::TaskRun {
                id: future_tasks::new_run_id(),
                task_id: task.id.clone(),
                kind: future_tasks::RunKind::Manual,
                origin: future_tasks::RunOrigin::Cli,
                actor: Some("cli".into()),
                due_at: None,
                status: future_tasks::RunStatus::Running,
                thread_id: None,
                session_id: None,
                run_id: None,
                prompt_version: Some(1),
                result_summary: None,
                feedback: None,
                feedback_note: None,
                started_at: Some(started),
                finished_at: None,
                error_message: None,
            })
            .unwrap();

        let (out, _captured) = Output::memory();
        let error = wait_for_run(&store, &task.id, now_ms(), 1_000, &out, false).unwrap_err();
        assert!(error.contains("did not finish within timeout"), "{error}");
    }

    #[tokio::test]
    async fn list_names_a_manual_task_and_an_unrecognised_schedule() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (out, _captured) = Output::memory();
        add(
            &args(&["--name", "manual-one", "--prompt", "p", "--cwd", "/tmp"]),
            &out,
        )
        .unwrap();
        // A trigger written by a newer build (or by hand) still prints a row.
        let store = Store::open(home.dir.path().join(".future").as_path()).unwrap();
        let mut odd = store.find_task_by_name("manual-one").unwrap().unwrap();
        odd.trigger_kind = future_tasks::TriggerKind::Schedule;
        odd.trigger_json = serde_json::json!({"mode": "fortnightly"});
        store.update_task(&odd).unwrap();

        let (out, captured) = Output::memory();
        list(&[], &out).unwrap();
        let printed = text(captured.out);
        assert!(printed.contains("schedule"), "{printed}");

        let mut manual = store.find_task_by_name("manual-one").unwrap().unwrap();
        manual.trigger_kind = future_tasks::TriggerKind::Manual;
        manual.trigger_json = serde_json::json!({});
        store.update_task(&manual).unwrap();
        let (out, captured) = Output::memory();
        list(&[], &out).unwrap();
        assert!(
            text(captured.out).contains("manual"),
            "a manual task says so"
        );
    }

    /// A removed task keeps its history: `remove` is a soft delete, and the run
    /// ledger and prompt versions are what someone auditing it needs.
    #[tokio::test]
    async fn history_stays_readable_after_a_task_is_removed() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        let (setup, _c) = Output::memory();
        add(
            &args(&[
                "--name",
                "removed",
                "--prompt",
                "the prompt",
                "--cwd",
                "/tmp",
            ]),
            &setup,
        )
        .unwrap();
        let store = open_store().unwrap();
        let task = find_task(&store, "removed").unwrap();
        store
            .insert_run(&future_tasks::TaskRun {
                id: "trn_kept".to_string(),
                task_id: task.id.clone(),
                kind: future_tasks::RunKind::Manual,
                origin: future_tasks::RunOrigin::Cli,
                actor: None,
                due_at: None,
                status: future_tasks::RunStatus::Completed,
                thread_id: None,
                session_id: None,
                run_id: None,
                prompt_version: Some(1),
                result_summary: Some("it ran".to_string()),
                feedback: None,
                feedback_note: None,
                started_at: Some(1),
                finished_at: Some(2),
                error_message: None,
            })
            .unwrap();
        let (out, _c) = Output::memory();
        remove(&args(&["removed", "--yes"]), &out).unwrap();

        // By id and by name: the task is gone from `list`, but its history is
        // reachable in both spellings.
        for reference in [task.id.as_str(), "removed"] {
            let (out, captured) = Output::memory();
            runs(&args(&[reference]), &out).unwrap();
            let shown = text(captured.out.clone());
            assert!(
                shown.contains("trn_kept") && shown.contains("completed"),
                "{reference}: {shown}"
            );

            let (out, captured) = Output::memory();
            prompt_log(&args(&[reference]), &out).unwrap();
            let shown = text(captured.out.clone());
            assert!(shown.contains("active v1"), "{reference}: {shown}");
        }
        let (out, captured) = Output::memory();
        list(&[], &out).unwrap();
        assert!(!text(captured.out).contains("removed"));
    }

    #[test]
    fn duration_parsing_accepts_minutes_hours_and_days() {
        assert_eq!(parse_duration_minutes("30m"), Some(30));
        assert_eq!(parse_duration_minutes("2h"), Some(120));
        assert_eq!(parse_duration_minutes(" 1d "), Some(1440));
        assert_eq!(parse_duration_minutes("30"), None);
        assert_eq!(parse_duration_minutes("m"), None);
        assert_eq!(parse_duration_minutes("30x"), None);
    }

    /// A suggestion is not a version, and it is useless until the user can
    /// accept it: `prompt log` lists it separately with the command that does.
    #[tokio::test]
    async fn prompt_log_lists_a_suggestion_with_the_command_that_applies_it() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        let (setup, _c) = Output::memory();
        add(
            &args(&[
                "--name",
                "suggested",
                "--prompt",
                "the current prompt",
                "--cwd",
                "/tmp",
            ]),
            &setup,
        )
        .unwrap();
        // A suggestion as the reflection pass writes it: outside the version
        // sequence, with the confidence and the run it came from.
        let store = open_store().unwrap();
        let task = find_task(&store, "suggested").unwrap();
        let row = future_tasks::PromptRevision {
            id: "rev_suggestion".to_string(),
            task_id: task.id.clone(),
            version: future_tasks::PROPOSAL_VERSION,
            prompt: "a better prompt".to_string(),
            source: future_tasks::REVISION_SOURCE_REFLECTION.to_string(),
            status: future_tasks::REVISION_STATUS_PROPOSED.to_string(),
            reason: Some("the output path was not stated".to_string()),
            confidence: Some(0.82),
            source_run_id: Some("trn_1".to_string()),
            created_at: now_ms(),
        };
        store.insert_revision(&row).unwrap();

        let (out, captured) = Output::memory();
        prompt_log(&args(&["suggested"]), &out).unwrap();
        let shown = text(captured.out);
        assert!(shown.contains("Suggestions (not applied)"), "{shown}");
        assert!(shown.contains("confidence 0.82"), "{shown}");
        assert!(shown.contains("the output path was not stated"), "{shown}");
        assert!(
            shown.contains("future task prompt apply tsk_") && shown.contains("rev_suggestion"),
            "the suggestion names the command that applies it: {shown}"
        );
        assert!(
            !shown.contains("v0"),
            "a suggestion has no version: {shown}"
        );

        let (out, captured) = Output::memory();
        prompt_log(&args(&["suggested", "--json"]), &out).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text(captured.out)).unwrap();
        assert_eq!(parsed[0]["proposed"], true);
        assert_eq!(parsed[0]["confidence"], 0.82);
        assert_eq!(parsed[0]["sourceRunId"], "trn_1");
    }

    /// Accepting a suggestion is what puts it in force — and the history says
    /// where it came from, with the suggestion itself marked as applied.
    #[tokio::test]
    async fn applying_a_suggestion_makes_it_the_prompt_and_marks_it_applied() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        let (setup, _c) = Output::memory();
        add(
            &args(&[
                "--name",
                "accepts",
                "--prompt",
                "the current prompt",
                "--cwd",
                "/tmp",
            ]),
            &setup,
        )
        .unwrap();
        let store = open_store().unwrap();
        let task = find_task(&store, "accepts").unwrap();
        store
            .insert_revision(&future_tasks::PromptRevision {
                id: "rev_suggestion".to_string(),
                task_id: task.id.clone(),
                version: future_tasks::PROPOSAL_VERSION,
                prompt: "a better prompt".to_string(),
                source: future_tasks::REVISION_SOURCE_REFLECTION.to_string(),
                status: future_tasks::REVISION_STATUS_PROPOSED.to_string(),
                reason: Some("clearer".to_string()),
                confidence: Some(0.9),
                source_run_id: Some("trn_1".to_string()),
                created_at: now_ms(),
            })
            .unwrap();

        let (out, captured) = Output::memory();
        prompt_apply(&args(&["accepts", "rev_suggestion"]), &out).unwrap();
        let shown = text(captured.out.clone());
        assert!(shown.contains("now runs v2 (was v1)"), "{shown}");

        let store = open_store().unwrap();
        let stored = find_task(&store, "accepts").unwrap();
        assert_eq!(stored.prompt, "a better prompt");
        let revisions = store.list_revisions(&stored.id).unwrap();
        let suggestion = revisions.iter().find(|r| r.id == "rev_suggestion").unwrap();
        assert_eq!(suggestion.status, future_tasks::REVISION_STATUS_APPLIED);
        let live = revisions
            .iter()
            .find(|r| r.version == stored.prompt_version)
            .unwrap();
        assert_eq!(live.status, future_tasks::REVISION_STATUS_ACTIVE);
        assert_eq!(live.source, future_tasks::REVISION_SOURCE_REFLECTION);
        assert_eq!(live.reason.as_deref(), Some("clearer"));
        // The version it replaced is kept, so the loop is reversible.
        assert!(revisions.iter().any(|r| r.prompt == "the current prompt"
            && r.status == future_tasks::REVISION_STATUS_SUPERSEDED));

        // …and the log reads as history, not as a version numbered zero.
        let (out, captured) = Output::memory();
        prompt_log(&args(&["accepts"]), &out).unwrap();
        let shown = text(captured.out);
        assert!(shown.contains("active v2"), "{shown}");
        assert!(shown.contains("suggestion"), "{shown}");
        assert!(
            !shown.contains("v0"),
            "a suggestion is not a version: {shown}"
        );
        assert!(!shown.contains("Suggestions (not applied)"), "{shown}");
    }

    /// `output` prints a run's full answer, and says which run the text belongs
    /// to — a reused conversation holds every run's answer, so an unlabelled
    /// text would be attributed to the wrong one.
    #[tokio::test]
    async fn output_prints_a_runs_full_answer_with_its_identity() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let agent = crate::test_server::MockAgent::respond(
            "get_last_assistant_text",
            &serde_json::json!({ "text": "the whole answer\nsecond line" }).to_string(),
        );
        let addr = crate::test_server::spawn_mock(agent.clone()).await;
        let _env = EnvGuard::set(&[("FUTURE_AGENT_GRPC_ADDR", std::ffi::OsString::from(addr))]);

        let (setup, _c) = Output::memory();
        add(
            &args(&["--name", "reads", "--prompt", "p", "--cwd", "/tmp"]),
            &setup,
        )
        .unwrap();
        let store = open_store().unwrap();
        let task = find_task(&store, "reads").unwrap();
        store
            .insert_run(&future_tasks::TaskRun {
                id: "trn_read".to_string(),
                task_id: task.id.clone(),
                kind: future_tasks::RunKind::Manual,
                origin: future_tasks::RunOrigin::Cli,
                actor: None,
                due_at: None,
                status: future_tasks::RunStatus::Completed,
                thread_id: Some("thr_1".to_string()),
                session_id: Some("sess_1".to_string()),
                run_id: None,
                prompt_version: Some(1),
                result_summary: Some("truncated…".to_string()),
                feedback: None,
                feedback_note: None,
                started_at: Some(1_000),
                finished_at: Some(2_000),
                error_message: None,
            })
            .unwrap();

        let (out, captured) = Output::memory();
        output(&args(&["trn_read"]), &out).await.unwrap();
        let shown = text(captured.out);
        assert!(
            shown.contains("reads · run trn_read · completed"),
            "{shown}"
        );
        assert!(shown.contains("session sess_1"), "{shown}");
        assert!(shown.contains("the whole answer"), "{shown}");
        assert_eq!(
            agent.seen_of("get_last_assistant_text")[0].session_id,
            "sess_1",
            "the run's own conversation is read"
        );

        // `--tail` is for reading the end of a long answer.
        let (out, captured) = Output::memory();
        output(&args(&["trn_read", "--tail", "1"]), &out)
            .await
            .unwrap();
        let shown = text(captured.out);
        assert!(shown.trim_end().ends_with("second line"), "{shown}");
        assert!(!shown.contains("the whole answer"), "{shown}");

        // A run with no conversation (it failed before prompting) is a message,
        // not a panic, and the CLI does not call the agent for it.
        let store = open_store().unwrap();
        let mut orphan = store.get_run("trn_read").unwrap().unwrap();
        orphan.id = "trn_orphan".to_string();
        orphan.session_id = None;
        orphan.error_message = Some("agent unreachable".to_string());
        store.insert_run(&orphan).unwrap();
        let (out, _c) = Output::memory();
        let error = output(&args(&["trn_orphan"]), &out).await.unwrap_err();
        assert!(error.contains("has no conversation"), "{error}");
        assert!(error.contains("agent unreachable"), "{error}");
        assert_eq!(agent.seen_of("get_last_assistant_text").len(), 2);
        let _ = home;
    }
}
