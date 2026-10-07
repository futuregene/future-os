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
pub fn task(command: Option<&str>, rest: &[String], out: &Output) -> Result<()> {
    match command {
        None | Some("--help" | "-h") => {
            out.log(help::TASK_HELP);
            Ok(())
        }
        Some("list") => list(rest, out),
        Some("show") => show(rest, out),
        Some("add") => add(rest, out),
        Some("run") => run(rest, out),
        Some("runs") => runs(rest, out),
        Some(other) => Err(format!(
            "Unknown argument: {other}\nUsage: future task [list|show|add|run|runs] …\nRun `future task --help` for details."
        )),
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

fn format_trigger(task: &Task) -> String {
    match task.trigger_kind {
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

fn list(args: &[String], out: &Output) -> Result<()> {
    let json_flag = args.iter().any(|a| a == "--json");
    let include_deleted = args.iter().any(|a| a == "--all");
    let store = open_store()?;
    let tasks = store
        .list_tasks(include_deleted)
        .map_err(|e| e.to_string())?;
    if json_flag {
        let items: Vec<_> = tasks
            .iter()
            .map(|t| {
                json!({
                    "id": t.id,
                    "name": t.name,
                    "enabled": t.enabled,
                    "trigger": format_trigger(t),
                    "nextDueAt": t.next_due_at,
                    "lastRunAt": t.last_run_at,
                    "sessionPolicy": format!("{:?}", t.session_policy).to_lowercase(),
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
            format_trigger(t),
            state,
            next,
            t.id
        ));
    }
    Ok(())
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
            "triggerKind": format!("{:?}", t.trigger_kind).to_lowercase(),
            "trigger": t.trigger_json,
            "depJoin": format!("{:?}", t.dep_join).to_lowercase(),
            "nextDueAt": t.next_due_at,
            "lastRunAt": t.last_run_at,
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
    out.log(&format!("  trigger:  {}", format_trigger(&t)));
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
    let mut trigger_json = serde_json::json!({});
    let mut trigger_kind = future_tasks::TriggerKind::Manual;
    let mut disabled = false;
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
            "--disabled" => disabled = true,
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

    let now = now_ms();
    let next_due = if trigger_kind == future_tasks::TriggerKind::Schedule {
        // Computed from an *enabled* clone: `next_due` answers "when would this
        // fire", and a task created with `--disabled` must still carry its next
        // slot so enabling it later schedules it instead of doing nothing.
        let t = Task {
            id: String::new(),
            name: name.clone(),
            enabled: true,
            prompt: prompt.clone(),
            prompt_version: 1,
            cwd: cwd.clone(),
            model_id: model.clone(),
            thinking_level: thinking.clone(),
            session_policy,
            thread_id: None,
            trigger_kind,
            trigger_json: trigger_json.clone(),
            dep_join: future_tasks::DepJoin::All,
            next_due_at: None,
            last_run_at: None,
            pending_request_at: None,
            pending_origin: None,
            pending_actor: None,
            reflection,
            created_at: now,
            updated_at: now,
            deleted_at: None,
        };
        future_tasks::next_due(&t, now - 1)
    } else {
        None
    };

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
        thread_id: None,
        trigger_kind,
        trigger_json,
        dep_join: future_tasks::DepJoin::All,
        next_due_at: next_due,
        last_run_at: None,
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

    if json_flag {
        out.log(
            &serde_json::to_string_pretty(&json!({
                "id": t.id,
                "name": t.name,
                "nextDueAt": t.next_due_at,
            }))
            .map_err(|e| e.to_string())?,
        );
    } else {
        out.log(&format!("Created task {} ({})", t.name, t.id));
        if let Some(due) = t.next_due_at {
            out.log(&format!("  next due: {}", format_ms(Some(due))));
        }
    }
    Ok(())
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
    let t = find_task(&store, id)?;
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
                    "status": format!("{:?}", r.status).to_lowercase(),
                    "dueAt": r.due_at,
                    "startedAt": r.started_at,
                    "finishedAt": r.finished_at,
                    "promptVersion": r.prompt_version,
                    "resultSummary": r.result_summary,
                    "errorMessage": r.error_message,
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
        out.log(&format!(
            "{:<14} {:<10} {:<10} {:<18} {}",
            r.id,
            kind,
            status,
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
        for (values, expect_ok) in [
            (vec![], true),
            (vec!["--help"], true),
            (vec!["list"], true),
            (vec!["show"], false),
            (vec!["add"], false),
            (vec!["run"], false),
            (vec!["runs"], false),
            (vec!["frobnicate"], false),
        ] {
            let (out, _captured) = Output::memory();
            let sub = values.first().copied().unwrap_or("list");
            let rest: Vec<String> = values.iter().skip(1).map(|v| (*v).to_string()).collect();
            let result = task(Some(sub), &rest, &out);
            assert_eq!(result.is_ok(), expect_ok, "{values:?}: {result:?}");
        }
        let (out, captured) = Output::memory();
        task(None, &[], &out).unwrap();
        assert!(text(captured.out).contains("future task — manage FutureOS tasks"));
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

    #[test]
    fn duration_parsing_accepts_minutes_hours_and_days() {
        assert_eq!(parse_duration_minutes("30m"), Some(30));
        assert_eq!(parse_duration_minutes("2h"), Some(120));
        assert_eq!(parse_duration_minutes(" 1d "), Some(1440));
        assert_eq!(parse_duration_minutes("30"), None);
        assert_eq!(parse_duration_minutes("m"), None);
        assert_eq!(parse_duration_minutes("30x"), None);
    }
}
