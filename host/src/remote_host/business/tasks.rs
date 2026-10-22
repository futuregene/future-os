//! Remote task commands (phone ↔ desktop). The phone manages tasks and reads
//! their run ledgers; execution stays on the desktop's tick loop.
//!
//! Wire budget: `list_tasks` returns only the fields a phone list needs —
//! never the prompt body — and run summaries are truncated. `get_task` returns
//! the full record for the detail screen.

use future_remote_protocol::IncomingCmd;
use future_remote_server::services::ReplySink;
use serde_json::{json, Value};

use super::{reply, reply_unit};

/// Characters of a run's result summary that may cross the wire.
const RUN_SUMMARY_CHARS: usize = 300;

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

fn open_store() -> Result<future_tasks::Store, crate::Error> {
    let home = crate::future_home_root();
    future_tasks::Store::open(&home).map_err(|e| crate::Error::Message(e.to_string()))
}

fn run_summary_view(run: future_tasks::TaskRun) -> Value {
    json!({
        "id": run.id,
        "kind": format!("{:?}", run.kind).to_lowercase(),
        "origin": format!("{:?}", run.origin).to_lowercase(),
        "status": format!("{:?}", run.status).to_lowercase(),
        "threadId": run.thread_id,
        // The phone opens a run's conversation by session id (its chat is
        // session-based), so the run has to name one.
        "sessionId": run.session_id,
        "startedAt": run.started_at,
        "finishedAt": run.finished_at,
        "promptVersion": run.prompt_version,
        "resultSummary": run.result_summary.as_deref().map(|s| truncate(s, RUN_SUMMARY_CHARS)),
        "errorMessage": run.error_message,
        // The conversation was deleted after the run settled: the ids are gone
        // with it, so the phone hides its "open" link and says why.
        "sessionDeleted": run.session_deleted,
    })
}

/// The list row: identity + trigger state only, no prompt.
fn task_list_view(store: &future_tasks::Store, task: future_tasks::Task) -> Value {
    let latest = store.latest_run_for_task(&task.id).ok().flatten();
    json!({
        "id": task.id,
        "name": task.name,
        "enabled": task.enabled,
        "triggerKind": format!("{:?}", task.trigger_kind).to_lowercase(),
        "trigger": task.trigger_json,
        "depCount": 0,
        "nextDueAt": task.next_due_at,
        "queued": task.pending_request_at.is_some(),
        "latestRun": latest.map(run_summary_view),
    })
}

/// The detail record (the only place the prompt crosses the wire).
fn task_detail_view(store: &future_tasks::Store, task: future_tasks::Task) -> Value {
    let latest = store.latest_run_for_task(&task.id).ok().flatten();
    // Resolved before `task`'s fields are moved into the value below. A phone
    // of an older build ignores the field; a current one shows it and sends it
    // back, so the zone survives a round trip through the editor.
    let timezone = future_tasks::task_timezone(&task).name().to_string();
    json!({
        "timezone": timezone,
        "id": task.id,
        "name": task.name,
        "enabled": task.enabled,
        "prompt": task.prompt,
        "promptVersion": task.prompt_version,
        "cwd": task.cwd,
        "modelId": task.model_id,
        "thinkingLevel": task.thinking_level,
        "sessionPolicy": format!("{:?}", task.session_policy).to_lowercase(),
        "sessionRetention": format!("{:?}", task.session_retention).to_lowercase(),
        "conversationMode": format!("{:?}", task.conversation_mode).to_lowercase(),
        "triggerKind": format!("{:?}", task.trigger_kind).to_lowercase(),
        "trigger": task.trigger_json,
        "depJoin": "all",
        "depCount": 0,
        "nextDueAt": task.next_due_at,
        "queued": task.pending_request_at.is_some(),
        "latestRun": latest.map(run_summary_view),
    })
}

fn parse_conversation_mode(raw: Option<&str>) -> future_tasks::ConversationMode {
    match raw {
        Some("chat") => future_tasks::ConversationMode::Chat,
        _ => future_tasks::ConversationMode::Workspace,
    }
}

fn parse_session_policy(raw: Option<&str>) -> future_tasks::SessionPolicy {
    match raw {
        Some("existing") => future_tasks::SessionPolicy::Existing,
        _ => future_tasks::SessionPolicy::New,
    }
}

/// `delete` only means something for a per-run conversation; the same
/// normalization the desktop commands apply.
fn parse_session_retention(
    raw: Option<&str>,
    policy: future_tasks::SessionPolicy,
) -> future_tasks::SessionRetention {
    match (raw, policy) {
        (Some("delete"), future_tasks::SessionPolicy::New) => {
            future_tasks::SessionRetention::Delete
        }
        _ => future_tasks::SessionRetention::Keep,
    }
}

fn parse_trigger_kind(raw: Option<&str>) -> future_tasks::TriggerKind {
    match raw {
        Some("schedule") => future_tasks::TriggerKind::Schedule,
        _ => future_tasks::TriggerKind::Manual,
    }
}

/// Build a Task from the phone's `task` payload (a whole-record write, the
/// same convention as provider writes).
/// A nullable string field, where **absent** and **explicit null** differ.
///
/// `get(key).and_then(as_str)` conflates the two: a payload that sets a model to
/// `null` — the phone's "use the default model" choice — looked identical to a
/// payload that never mentioned it, and fell back to the value already stored.
/// The user could pick a model from the phone but never go back to the default.
/// Absent means "leave it"; present-null means "clear it".
fn optional_str_field(payload: &Value, key: &str, base: Option<&String>) -> Option<String> {
    match payload.get(key) {
        None => base.cloned(),
        Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.clone()),
        // A wrong type is a caller error, not a instruction to clear the field.
        Some(_) => base.cloned(),
    }
}

/// The zone a payload asks for.
///
/// Absent keeps what the row already carries (and resolves the host's zone for
/// a new task); null or an empty string asks for that zone explicitly; a name
/// this build cannot resolve is refused rather than replaced silently, because
/// a schedule quietly moved to another clock is the failure the setting exists
/// to prevent.
fn timezone_from_payload(
    payload: &Value,
    base: Option<&future_tasks::Task>,
) -> Result<Option<String>, crate::Error> {
    let resolve = |name: Option<&str>| -> Result<String, crate::Error> {
        match name.map(str::trim).filter(|name| !name.is_empty()) {
            None => Ok(future_tasks::resolve_timezone(None)),
            Some(name) => future_tasks::parse_timezone(name)
                .map(|tz| tz.name().to_string())
                .ok_or_else(|| {
                    crate::Error::Message(format!(
                        "unknown timezone {name:?}: expected an IANA name like Asia/Shanghai"
                    ))
                }),
        }
    };
    match payload.get("timezone") {
        None => match base {
            Some(task) => Ok(task.timezone.clone()),
            None => Ok(Some(resolve(None)?)),
        },
        Some(Value::Null) => Ok(Some(resolve(None)?)),
        Some(Value::String(raw)) => Ok(Some(resolve(Some(raw))?)),
        // A wrong type is a caller error, not an instruction to clear the zone.
        Some(_) => Ok(base.and_then(|task| task.timezone.clone())),
    }
}

fn task_from_payload(
    payload: &Value,
    existing: Option<future_tasks::Task>,
) -> Result<future_tasks::Task, crate::Error> {
    let now = now_ms();
    let base = existing;
    let name = payload
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| base.as_ref().map(|t| t.name.clone()))
        .ok_or_else(|| crate::Error::Message("name is required".into()))?;
    let prompt = payload
        .get("prompt")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| base.as_ref().map(|t| t.prompt.clone()))
        .ok_or_else(|| crate::Error::Message("prompt is required".into()))?;
    let cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| base.as_ref().map(|t| t.cwd.clone()))
        .ok_or_else(|| crate::Error::Message("cwd is required".into()))?;

    let trigger_kind = payload
        .get("triggerKind")
        .and_then(Value::as_str)
        .map(|raw| parse_trigger_kind(Some(raw)))
        .or_else(|| base.as_ref().map(|t| t.trigger_kind))
        .unwrap_or(future_tasks::TriggerKind::Manual);
    let trigger = payload
        .get("trigger")
        .cloned()
        .or_else(|| base.as_ref().map(|t| t.trigger_json.clone()))
        .unwrap_or_else(|| json!({}));

    let timezone = timezone_from_payload(payload, base.as_ref())?;

    let prompt_changed = base.as_ref().is_none_or(|t| t.prompt != prompt);
    let mut task = future_tasks::Task {
        id: base
            .as_ref()
            .map(|t| t.id.clone())
            .unwrap_or_else(future_tasks::new_task_id),
        name,
        enabled: payload
            .get("enabled")
            .and_then(Value::as_bool)
            .or_else(|| base.as_ref().map(|t| t.enabled))
            .unwrap_or(true),
        prompt,
        prompt_version: base.as_ref().map(|t| t.prompt_version).unwrap_or(1),
        cwd,
        model_id: optional_str_field(
            payload,
            "modelId",
            base.as_ref().and_then(|t| t.model_id.as_ref()),
        ),
        thinking_level: optional_str_field(
            payload,
            "thinkingLevel",
            base.as_ref().and_then(|t| t.thinking_level.as_ref()),
        ),
        timezone,
        session_policy: payload
            .get("sessionPolicy")
            .and_then(Value::as_str)
            .map(|raw| parse_session_policy(Some(raw)))
            .or_else(|| base.as_ref().map(|t| t.session_policy))
            .unwrap_or(future_tasks::SessionPolicy::New),
        // Absent means "leave it": the phone's form sends it only when the
        // choice applies (a per-run conversation), and an older phone does not
        // send it at all.
        session_retention: payload
            .get("sessionRetention")
            .and_then(Value::as_str)
            .map(|raw| {
                let policy = payload
                    .get("sessionPolicy")
                    .and_then(Value::as_str)
                    .map(|p| parse_session_policy(Some(p)))
                    .or_else(|| base.as_ref().map(|t| t.session_policy))
                    .unwrap_or(future_tasks::SessionPolicy::New);
                parse_session_retention(Some(raw), policy)
            })
            .or_else(|| base.as_ref().map(|t| t.session_retention))
            .unwrap_or(future_tasks::SessionRetention::Keep),
        conversation_mode: payload
            .get("conversationMode")
            .and_then(Value::as_str)
            .map(|raw| parse_conversation_mode(Some(raw)))
            .or_else(|| base.as_ref().map(|t| t.conversation_mode))
            .unwrap_or(future_tasks::ConversationMode::Workspace),
        thread_id: base.as_ref().and_then(|t| t.thread_id.clone()),
        trigger_kind,
        trigger_json: trigger,
        next_due_at: None,
        pending_request_at: base.as_ref().and_then(|t| t.pending_request_at),
        pending_origin: base.as_ref().and_then(|t| t.pending_origin),
        pending_actor: base.as_ref().and_then(|t| t.pending_actor.clone()),
        created_at: base.as_ref().map(|t| t.created_at).unwrap_or(now),
        updated_at: now,
        deleted_at: None,
    };
    if task.trigger_kind == future_tasks::TriggerKind::Schedule {
        task.next_due_at = future_tasks::next_due(&task, now - 1);
    }
    if prompt_changed {
        task.prompt_version = base.as_ref().map(|t| t.prompt_version + 1).unwrap_or(1);
    }
    Ok(task)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub(super) async fn execute(cmd: &IncomingCmd, sink: &dyn ReplySink) {
    match cmd.cmd_type.as_str() {
        "list_tasks" => match open_store() {
            Ok(store) => match store.list_tasks(false) {
                Ok(tasks) => {
                    let items: Vec<Value> = tasks
                        .into_iter()
                        .map(|t| task_list_view(&store, t))
                        .collect();
                    reply(sink, true, json!({ "tasks": items }), None).await
                }
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            },
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
        "get_task" => match open_store() {
            Ok(store) => match store.get_task(&cmd.task_id) {
                Ok(Some(task)) => reply(sink, true, task_detail_view(&store, task), None).await,
                Ok(None) => reply(sink, false, Value::Null, Some("task not found")).await,
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            },
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
        "create_task" => match open_store() {
            Ok(store) => match task_from_payload(&cmd.task, None).and_then(|task| {
                let created = task.clone();
                store
                    .insert_task(&task)
                    .map(|()| created)
                    .map_err(|e| crate::Error::Message(e.to_string()))
            }) {
                Ok(task) => {
                    // The panel in this very process is showing the list this
                    // write just changed: tell it, the same way a write from its
                    // own buttons would. A failed write says nothing.
                    crate::emit_tasks_updated();
                    reply(sink, true, task_detail_view(&store, task), None).await
                }
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            },
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
        "update_task" => match open_store() {
            Ok(store) => {
                let existing = match store.get_task(&cmd.task_id) {
                    Ok(Some(task)) => task,
                    Ok(None) => {
                        reply(sink, false, Value::Null, Some("task not found")).await;
                        return;
                    }
                    Err(error) => {
                        reply(sink, false, Value::Null, Some(&error.to_string())).await;
                        return;
                    }
                };
                // Prompt versions go through the same helper as the CLI and the
                // desktop panel. A hand-rolled row here did three things wrong:
                // it dropped the version it replaced (so the task's first prompt
                // became unreachable), it invented English reason text (rendered
                // verbatim, so it surfaced untranslated in a Chinese panel), and
                // it discarded the insert error, losing history silently.
                //
                // One chain, one error path: the history read cannot fail on
                // its own here (the store recreates its schema on open), so a
                // match arm for it would be a branch nothing can reach.
                let incoming_prompt = cmd
                    .task
                    .get("prompt")
                    .and_then(Value::as_str)
                    .unwrap_or(&existing.prompt)
                    .to_string();
                let result = store
                    .list_revisions(&existing.id)
                    .map_err(|e| crate::Error::Message(e.to_string()))
                    .and_then(|history| {
                        let revisions = future_tasks::prompt_change_revisions(
                            &existing,
                            &history,
                            &incoming_prompt,
                            "user",
                            None,
                            now_ms(),
                        );
                        let task = task_from_payload(&cmd.task, Some(existing))?;
                        for row in &revisions {
                            store
                                .insert_revision(row)
                                .map_err(|e| crate::Error::Message(e.to_string()))?;
                        }
                        store
                            .update_task(&task)
                            .map_err(|e| crate::Error::Message(e.to_string()))?;
                        Ok(task)
                    });
                match result {
                    Ok(task) => {
                        crate::emit_tasks_updated();
                        reply(sink, true, task_detail_view(&store, task), None).await
                    }
                    Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
                }
            }
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
        "delete_task" => match open_store() {
            Ok(store) => {
                let result = store
                    .get_task(&cmd.task_id)
                    .map_err(|e| crate::Error::Message(e.to_string()))
                    .and_then(|task| {
                        task.ok_or_else(|| crate::Error::Message("task not found".into()))
                    })
                    .and_then(|mut task| {
                        task.deleted_at = Some(now_ms());
                        task.enabled = false;
                        task.updated_at = now_ms();
                        store
                            .update_task(&task)
                            .map_err(|e| crate::Error::Message(e.to_string()))?;
                        // Nothing else to clean up: the `task_deps` rows that
                        // used to point at this task are no longer read by
                        // anything (see `future_tasks` store schema).
                        Ok(())
                    });
                if result.is_ok() {
                    crate::emit_tasks_updated();
                }
                reply_unit(sink, result).await
            }
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
        "set_task_enabled" => match open_store() {
            Ok(store) => {
                let enabled = cmd.enabled;
                let result = store
                    .get_task(&cmd.task_id)
                    .map_err(|e| crate::Error::Message(e.to_string()))
                    .and_then(|task| {
                        task.ok_or_else(|| crate::Error::Message("task not found".into()))
                    })
                    .and_then(|mut task| {
                        task.enabled = enabled;
                        task.updated_at = now_ms();
                        store
                            .update_task(&task)
                            .map_err(|e| crate::Error::Message(e.to_string()))?;
                        Ok(task)
                    });
                match result {
                    Ok(task) => {
                        crate::emit_tasks_updated();
                        reply(sink, true, task_detail_view(&store, task), None).await
                    }
                    Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
                }
            }
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
        // **Transitional answers to a removed feature.** Task dependencies are
        // gone. The command names stay because the phone of an older build sends
        // them: a read says "no edges", and a write is refused outright rather
        // than accepted and dropped — a phone that got `ok` would show an edge
        // that nothing will ever act on. (`depCount` in the views answers 0 for
        // the same reason, so those clients label every task by its own trigger.)
        "set_task_dep" | "remove_task_dep" => {
            let message = "task dependencies have been removed; a task now runs on its own schedule or on demand";
            reply(sink, false, Value::Null, Some(message)).await
        }
        "run_task" => match open_store() {
            Ok(store) => {
                let result = store
                    .get_task(&cmd.task_id)
                    .map_err(|e| crate::Error::Message(e.to_string()))
                    .and_then(|task| {
                        task.ok_or_else(|| crate::Error::Message("task not found".into()))
                    })
                    .and_then(|mut task| {
                        task.pending_request_at = Some(now_ms());
                        task.pending_origin = Some(future_tasks::RunOrigin::Ui);
                        task.pending_actor = Some("phone".to_string());
                        task.updated_at = now_ms();
                        store
                            .update_task(&task)
                            .map_err(|e| crate::Error::Message(e.to_string()))?;
                        // The phone's tap starts the run now, exactly like the
                        // desktop button (see `crate::tasks::wake`).
                        crate::tasks::wake();
                        Ok(task)
                    });
                match result {
                    Ok(task) => {
                        // Queued: the panel draws that state, and the run's own
                        // progress reaches it through the tick's notifier.
                        crate::emit_tasks_updated();
                        reply(sink, true, task_detail_view(&store, task), None).await
                    }
                    Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
                }
            }
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
        "list_task_runs" => match open_store() {
            Ok(store) => {
                let limit = if cmd.limit > 0 { cmd.limit } else { 20 };
                match store.list_runs_for_task(&cmd.task_id, limit) {
                    Ok(runs) => {
                        let items: Vec<Value> = runs.into_iter().map(run_summary_view).collect();
                        reply(sink, true, json!({ "runs": items }), None).await
                    }
                    Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
                }
            }
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
        // Transitional answer to a removed feature: always an empty set, never
        // an error, so the phone's dependency list renders as "none" instead of
        // failing a screen it can still open (see the `set_task_dep` arm).
        "list_task_deps" => reply(sink, true, json!({ "deps": [] }), None).await,
        "list_task_revisions" => match open_store() {
            Ok(store) => match store.list_revisions(&cmd.task_id) {
                Ok(revisions) => {
                    let items: Vec<Value> = revisions
                        .into_iter()
                        .map(|r| {
                            json!({
                                "id": r.id,
                                "version": r.version,
                                "source": r.source,
                                "status": r.status,
                                "reason": r.reason,
                                "confidence": r.confidence,
                                "createdAt": r.created_at,
                                // The whole prompt and a preview: the phone
                                // renders the preview, and applying a version
                                // is the desktop's own write either way.
                                "prompt": r.prompt,
                                "promptPreview": truncate(&r.prompt, 160),
                            })
                        })
                        .collect();
                    reply(sink, true, json!({ "revisions": items }), None).await
                }
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            },
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
        "apply_task_revision" => match open_store() {
            Ok(store) => {
                // The same implementation the webview command uses, so
                // accepting a suggestion from the phone marks it applied and
                // records the version the same way.
                match crate::tasks::accept_revision(&store, &cmd.task_id, &cmd.revision_id) {
                    Ok(task) => {
                        crate::emit_tasks_updated();
                        reply(sink, true, task_detail_view(&store, task), None).await
                    }
                    Err(error) => reply(sink, false, Value::Null, Some(&error)).await,
                }
            }
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
        other => {
            reply(
                sink,
                false,
                Value::Null,
                Some(&format!("Unsupported task command: {other}")),
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::test_support::{HomeGuard, RecordingSink};
    use future_remote_protocol::IncomingCmd;

    /// A private FutureOS home: `open_store` resolves the task database from it,
    /// so each test owns its rows.
    fn home(label: &str) -> HomeGuard {
        HomeGuard::new(label)
    }

    fn task(name: &str, policy: future_tasks::SessionPolicy) -> future_tasks::Task {
        let now = 1_000_000;
        future_tasks::Task {
            id: future_tasks::new_task_id(),
            name: name.into(),
            enabled: true,
            prompt: "summarise the week".into(),
            prompt_version: 3,
            cwd: "/tmp/repo".into(),
            model_id: Some("future/gpt-5".into()),
            thinking_level: Some("high".into()),
            timezone: None,
            session_policy: policy,
            session_retention: future_tasks::SessionRetention::Keep,
            conversation_mode: future_tasks::ConversationMode::Workspace,
            thread_id: None,
            trigger_kind: future_tasks::TriggerKind::Schedule,
            trigger_json: serde_json::json!({"mode": "daily", "time": "09:00"}),
            next_due_at: Some(now + 60_000),
            pending_request_at: None,
            pending_origin: None,
            pending_actor: None,
            created_at: now,
            updated_at: now,
            deleted_at: None,
        }
    }

    fn completed_run(task_id: &str) -> future_tasks::TaskRun {
        future_tasks::TaskRun {
            id: future_tasks::new_run_id(),
            task_id: task_id.into(),
            kind: future_tasks::RunKind::Main,
            origin: future_tasks::RunOrigin::Schedule,
            actor: None,
            due_at: Some(1),
            status: future_tasks::RunStatus::Completed,
            thread_id: Some("thr_1".into()),
            session_id: Some("sess_1".into()),
            run_id: None,
            prompt_version: Some(3),
            result_summary: Some("wrote reports/weekly.md".into()),
            feedback: None,
            feedback_note: None,
            started_at: Some(1),
            finished_at: Some(2),
            error_message: None,
            session_deleted: false,
        }
    }

    fn command(cmd_type: &str) -> IncomingCmd {
        IncomingCmd {
            cmd_type: cmd_type.to_string(),
            ..Default::default()
        }
    }

    /// The phone's list carries the trigger, the next due time and the last
    /// run's state — everything the row renders without fetching the detail.
    #[tokio::test]
    async fn the_list_answers_with_each_task_and_its_last_run() {
        let _home = home("business-tasks-list");
        let store = open_store().expect("store");
        let saved = task("weekly report", future_tasks::SessionPolicy::New);
        store.insert_task(&saved).unwrap();
        store.insert_run(&completed_run(&saved.id)).unwrap();

        let sink = RecordingSink::default();
        super::execute(&command("list_tasks"), &sink).await;
        let data = sink.ok_data();
        let items = data["tasks"].as_array().expect("tasks array");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["name"], "weekly report");
        assert_eq!(items[0]["triggerKind"], "schedule");
        assert_eq!(items[0]["nextDueAt"], saved.next_due_at.unwrap());
        assert_eq!(items[0]["latestRun"]["status"], "completed");

        // Nothing is waiting, so the phone is not told otherwise.
        assert_eq!(items[0]["queued"], false);

        // A pending request is surfaced: the phone pressed "run now" while the
        // desktop was busy, and the row has to say so rather than look ignored.
        let mut waiting = store.get_task(&saved.id).unwrap().unwrap();
        waiting.pending_request_at = Some(9_000);
        waiting.pending_origin = Some(future_tasks::RunOrigin::Ui);
        store.update_task(&waiting).unwrap();
        let sink = RecordingSink::default();
        super::execute(&command("list_tasks"), &sink).await;
        assert_eq!(sink.ok_data()["tasks"][0]["queued"], true);
    }

    /// The detail record is the only place the prompt crosses the wire, and it
    /// must state what the run will actually be (model, thinking, conversation).
    #[tokio::test]
    async fn the_detail_carries_the_prompt_and_the_run_settings() {
        let _home = home("business-tasks-detail");
        let store = open_store().expect("store");
        let saved = task("detail", future_tasks::SessionPolicy::Existing);
        store.insert_task(&saved).unwrap();

        let sink = RecordingSink::default();
        let mut cmd = command("get_task");
        cmd.task_id = saved.id.clone();
        super::execute(&cmd, &sink).await;
        let data = sink.ok_data();
        assert_eq!(data["prompt"], "summarise the week");
        assert_eq!(data["promptVersion"], 3);
        assert_eq!(data["modelId"], "future/gpt-5");
        assert_eq!(data["thinkingLevel"], "high");
        assert_eq!(data["sessionPolicy"], "existing");
        assert_eq!(data["conversationMode"], "workspace");
    }

    /// A version crosses the wire with its whole prompt and a preview: the
    /// phone shows the preview in the list, and applying it is the desktop's
    /// own write.
    #[tokio::test]
    async fn a_version_crosses_the_wire_and_applies_like_the_desktop() {
        let _home = home("business-tasks-revision-wire");
        let store = open_store().expect("store");
        let saved = task("versions", future_tasks::SessionPolicy::New);
        store.insert_task(&saved).unwrap();
        store
            .insert_revision(&future_tasks::PromptRevision {
                id: "rev_older".into(),
                task_id: saved.id.clone(),
                version: 2,
                prompt: "summarise the week, then write reports/weekly.md".into(),
                source: future_tasks::REVISION_SOURCE_USER.into(),
                status: future_tasks::REVISION_STATUS_SUPERSEDED.into(),
                reason: Some("the output path was stated".into()),
                confidence: None,
                source_run_id: None,
                created_at: 5,
            })
            .unwrap();

        let sink = RecordingSink::default();
        let mut cmd = command("list_task_revisions");
        cmd.task_id = saved.id.clone();
        super::execute(&cmd, &sink).await;
        let revision = &sink.ok_data()["revisions"][0];
        assert_eq!(
            revision["prompt"],
            "summarise the week, then write reports/weekly.md"
        );
        assert!(revision["promptPreview"].as_str().is_some());

        let sink = RecordingSink::default();
        let mut cmd = command("apply_task_revision");
        cmd.task_id = saved.id.clone();
        cmd.revision_id = "rev_older".into();
        super::execute(&cmd, &sink).await;
        assert_eq!(sink.ok_data()["promptVersion"], 4);
        assert_eq!(
            sink.ok_data()["prompt"],
            "summarise the week, then write reports/weekly.md"
        );

        let history = store.list_revisions(&saved.id).unwrap();
        let live = history
            .iter()
            .find(|r| r.version == 4)
            .expect("the live version");
        assert_eq!(live.source, future_tasks::REVISION_SOURCE_ROLLBACK);
        assert_eq!(live.status, future_tasks::REVISION_STATUS_ACTIVE);
        assert!(
            history
                .iter()
                .any(|r| r.source == future_tasks::REVISION_SOURCE_SUPERSEDED),
            "the replaced version stays in the history: {history:?}"
        );
    }

    /// A phone may ask for the runs of a task that no longer exists; the ledger
    /// is empty, not an error.
    #[tokio::test]
    async fn runs_of_an_unknown_task_are_empty() {
        let _home = home("business-tasks-runs-unknown");
        let sink = RecordingSink::default();
        let mut cmd = command("list_task_runs");
        cmd.task_id = "tsk_gone".into();
        super::execute(&cmd, &sink).await;
        assert!(sink.ok_data()["runs"].as_array().expect("runs").is_empty());
    }

    /// An absent field leaves what the task has; an explicit `null` clears it.
    /// The phone's "default model" choice sends `null`, so conflating the two
    /// meant a model could be picked from the phone but never un-picked.
    #[test]
    fn absent_and_null_fields_are_not_the_same_instruction() {
        let base = Some("future/gpt-5".to_string());

        // Absent: keep what the task already has.
        assert_eq!(
            optional_str_field(&json!({}), "modelId", base.as_ref()),
            Some("future/gpt-5".to_string())
        );
        // Explicit null: clear it.
        assert_eq!(
            optional_str_field(&json!({"modelId": null}), "modelId", base.as_ref()),
            None
        );
        // A value replaces it.
        assert_eq!(
            optional_str_field(&json!({"modelId": "future/o3"}), "modelId", base.as_ref()),
            Some("future/o3".to_string())
        );
        // A wrong type is a caller error, not an instruction to clear.
        assert_eq!(
            optional_str_field(&json!({"modelId": 7}), "modelId", base.as_ref()),
            Some("future/gpt-5".to_string())
        );
        // No base and no value is still "nothing".
        assert_eq!(optional_str_field(&json!({}), "modelId", None), None);
    }

    /// Through the command surface: clearing the model sticks, and a payload
    /// that omits the field leaves it alone.
    #[tokio::test]
    async fn clearing_a_model_from_the_phone_sticks() {
        let _home = home("business-tasks-clear-model");
        let store = open_store().expect("store");
        let saved = task("pinned", future_tasks::SessionPolicy::New);
        store.insert_task(&saved).unwrap();
        assert!(saved.model_id.is_some());

        let sink = RecordingSink::default();
        let mut cmd = command("update_task");
        cmd.task_id = saved.id.clone();
        cmd.task = json!({"modelId": null, "thinkingLevel": null});
        super::execute(&cmd, &sink).await;
        let data = sink.ok_data();
        assert_eq!(data["modelId"], Value::Null, "cleared, not kept");
        assert_eq!(data["thinkingLevel"], Value::Null);
        let stored = store.get_task(&saved.id).unwrap().unwrap();
        assert_eq!(stored.model_id, None);
        assert_eq!(stored.thinking_level, None);

        // Setting one back works, and a later update that omits it keeps it.
        let sink = RecordingSink::default();
        let mut cmd = command("update_task");
        cmd.task_id = saved.id.clone();
        cmd.task = json!({"modelId": "future/o3"});
        super::execute(&cmd, &sink).await;
        assert_eq!(sink.ok_data()["modelId"], "future/o3");

        let sink = RecordingSink::default();
        let mut cmd = command("update_task");
        cmd.task_id = saved.id.clone();
        cmd.task = json!({"name": "renamed"});
        super::execute(&cmd, &sink).await;
        assert_eq!(
            sink.ok_data()["modelId"],
            "future/o3",
            "a field the payload omits is left alone"
        );
    }

    /// An unknown task command is refused rather than silently succeeding.
    #[tokio::test]
    async fn an_unknown_task_command_is_unsupported() {
        let _home = home("business-tasks-unknown");
        let sink = RecordingSink::default();
        super::execute(&command("task_bogus"), &sink).await;
        assert!(sink.error_text().contains("Unsupported task command"));
    }

    /// A prompt edited from the phone records the same history the desktop and
    /// the CLI do. The version being replaced is kept, so the task's original
    /// prompt stays reachable — the hand-rolled row this replaced recorded only
    /// the new one.
    #[tokio::test]
    async fn editing_a_prompt_from_the_phone_keeps_the_version_it_replaced() {
        let _home = home("business-tasks-prompt-history");
        let store = open_store().expect("store");
        let saved = task("weekly", future_tasks::SessionPolicy::New);
        store.insert_task(&saved).unwrap();
        assert_eq!(saved.prompt_version, 3, "the fixture starts at v3");

        let sink = RecordingSink::default();
        let mut cmd = command("update_task");
        cmd.task_id = saved.id.clone();
        cmd.task = json!({
            "name": saved.name,
            "prompt": "a rewritten prompt",
            "cwd": saved.cwd,
            "triggerKind": "schedule",
            "trigger": {"mode": "daily", "time": "09:00"},
        });
        super::execute(&cmd, &sink).await;
        assert_eq!(sink.ok_data()["promptVersion"], 4);

        let history = store.list_revisions(&saved.id).unwrap();
        let by_version = |v: i64| {
            history
                .iter()
                .find(|r| r.version == v)
                .unwrap_or_else(|| panic!("no revision for v{v}: {history:?}"))
        };
        assert_eq!(
            by_version(3).prompt,
            "summarise the week",
            "the replaced prompt is kept"
        );
        assert_eq!(by_version(3).source, "superseded");
        assert_eq!(
            by_version(3).reason,
            None,
            "no invented reason prose: the field renders verbatim"
        );
        assert_eq!(by_version(4).prompt, "a rewritten prompt");
        assert_eq!(by_version(4).source, "user");
        assert_eq!(
            by_version(4).reason,
            None,
            "the phone does not invent English prose for a Chinese panel"
        );

        // A no-op prompt does not spend a version.
        let before = store.list_revisions(&saved.id).unwrap().len();
        let sink = RecordingSink::default();
        let mut again = command("update_task");
        again.task_id = saved.id.clone();
        again.task = json!({ "prompt": "a rewritten prompt" });
        super::execute(&again, &sink).await;
        assert_eq!(sink.ok_data()["promptVersion"], 4, "no version for a no-op");
        assert_eq!(store.list_revisions(&saved.id).unwrap().len(), before);
    }

    /// The dependency commands answer a phone of an older build: the read is an
    /// empty set (a screen that still lists edges renders "none"), and the write
    /// is refused with a message saying why. Deleting a task no longer has to
    /// sweep edges, because nothing reads them.
    #[tokio::test]
    async fn the_dependency_commands_answer_a_removed_feature() {
        let _home = home("business-tasks-dep-transitional");
        let store = open_store().expect("store");
        let upstream = task("upstream", future_tasks::SessionPolicy::New);
        let downstream = task("downstream", future_tasks::SessionPolicy::New);
        store.insert_task(&upstream).unwrap();
        store.insert_task(&downstream).unwrap();

        let sink = RecordingSink::default();
        let mut list = command("list_task_deps");
        list.task_id = downstream.id.clone();
        super::execute(&list, &sink).await;
        assert!(
            sink.ok_data()["deps"].as_array().unwrap().is_empty(),
            "there are no edges to report"
        );

        for cmd_type in ["set_task_dep", "remove_task_dep"] {
            let sink = RecordingSink::default();
            let mut cmd = command(cmd_type);
            cmd.task_id = downstream.id.clone();
            cmd.upstream_task_id = upstream.id.clone();
            cmd.on = "failure".into();
            super::execute(&cmd, &sink).await;
            let refusal = sink.error_text();
            assert!(refusal.contains("removed"), "{cmd_type}: {refusal}");
        }

        // The list row still carries `depCount`, always zero, which is what an
        // older phone labels a task's trigger from.
        let sink = RecordingSink::default();
        super::execute(&command("list_tasks"), &sink).await;
        assert_eq!(sink.ok_data()["tasks"][0]["depCount"], 0);

        // Deleting the upstream is now an ordinary delete.
        let sink = RecordingSink::default();
        let mut remove = command("delete_task");
        remove.task_id = upstream.id.clone();
        super::execute(&remove, &sink).await;
        sink.ok_data();
        assert!(
            store
                .get_task(&upstream.id)
                .unwrap()
                .unwrap()
                .deleted_at
                .is_some(),
            "the task is gone from the list"
        );
    }

    /// The wire budget for a run summary: a longer one is cut and marked, and a
    /// multi-byte character sitting on the cut is never split — the phone
    /// renders the bytes it is handed, and half a code point is not text.
    #[test]
    fn a_summary_longer_than_the_budget_is_cut_on_a_character_boundary() {
        assert_eq!(truncate("short", RUN_SUMMARY_CHARS), "short");
        assert_eq!(
            truncate(&"a".repeat(RUN_SUMMARY_CHARS), RUN_SUMMARY_CHARS),
            "a".repeat(RUN_SUMMARY_CHARS),
            "a summary exactly at the budget is not cut"
        );
        assert_eq!(
            truncate(&"a".repeat(RUN_SUMMARY_CHARS + 1), RUN_SUMMARY_CHARS),
            format!("{}…", "a".repeat(RUN_SUMMARY_CHARS))
        );
        // 301 bytes into a string of 3-byte characters is not a boundary: the
        // cut has to walk back to 300 rather than slice a character in half.
        assert_eq!(
            truncate(&"日".repeat(200), RUN_SUMMARY_CHARS + 1),
            format!("{}…", "日".repeat(100)),
            "the cut must not split a multi-byte character"
        );
    }

    /// Every conversation/retention/trigger field the phone sends is one of a
    /// small closed vocabulary, and an unknown or absent value has to land on
    /// the safe default rather than on whichever variant comes first.
    #[test]
    fn the_phone_vocabulary_maps_to_the_kernel_enums() {
        use future_tasks::{ConversationMode, SessionPolicy, SessionRetention, TriggerKind};

        assert_eq!(
            parse_conversation_mode(Some("chat")),
            ConversationMode::Chat
        );
        for raw in [Some("workspace"), Some("Chat"), Some(""), None] {
            assert_eq!(
                parse_conversation_mode(raw),
                ConversationMode::Workspace,
                "{raw:?}"
            );
        }

        assert_eq!(
            parse_session_policy(Some("existing")),
            SessionPolicy::Existing
        );
        for raw in [Some("new"), Some("Existing"), None] {
            assert_eq!(parse_session_policy(raw), SessionPolicy::New, "{raw:?}");
        }

        assert_eq!(
            parse_session_retention(Some("delete"), SessionPolicy::New),
            SessionRetention::Delete
        );
        assert_eq!(
            parse_session_retention(Some("delete"), SessionPolicy::Existing),
            SessionRetention::Keep,
            "a conversation that is reused cannot be deleted with the run"
        );
        assert_eq!(
            parse_session_retention(Some("keep"), SessionPolicy::New),
            SessionRetention::Keep
        );
        assert_eq!(
            parse_session_retention(None, SessionPolicy::New),
            SessionRetention::Keep
        );

        assert_eq!(parse_trigger_kind(Some("schedule")), TriggerKind::Schedule);
        for raw in [Some("manual"), Some("cron"), None] {
            assert_eq!(parse_trigger_kind(raw), TriggerKind::Manual, "{raw:?}");
        }
    }

    /// The phone's create form asks for a conversation mode and a retention
    /// rule, and both must survive into the stored record: they decide whether
    /// a run reuses a conversation and whether its session is deleted with it.
    #[tokio::test]
    async fn creating_a_task_keeps_the_conversation_and_retention_the_phone_chose() {
        let _home = home("business-tasks-create-options");

        let sink = RecordingSink::default();
        let mut cmd = command("create_task");
        cmd.task = json!({
            "name": "nightly",
            "prompt": "summarise the week",
            "cwd": "/tmp/repo",
            "triggerKind": "schedule",
            "trigger": {"mode": "daily", "time": "09:00"},
            "conversationMode": "chat",
            "sessionPolicy": "new",
            "sessionRetention": "delete",
        });
        super::execute(&cmd, &sink).await;
        let data = sink.ok_data();
        assert_eq!(data["conversationMode"], "chat");
        assert_eq!(data["sessionPolicy"], "new");
        assert_eq!(data["sessionRetention"], "delete");
        assert_eq!(data["triggerKind"], "schedule");
        assert_eq!(data["promptVersion"], 1, "a new task starts at v1");
        assert!(
            data["nextDueAt"].is_i64(),
            "a scheduled task gets its next due time: {data}"
        );

        // A per-run conversation is the only one `delete` applies to: asking for
        // it beside `existing` is normalized to keep, exactly as the desktop's
        // own form does.
        let sink = RecordingSink::default();
        let mut cmd = command("create_task");
        cmd.task = json!({
            "name": "reuses its conversation",
            "prompt": "summarise the week",
            "cwd": "/tmp/repo",
            "conversationMode": "workspace",
            "sessionPolicy": "existing",
            "sessionRetention": "delete",
        });
        super::execute(&cmd, &sink).await;
        let data = sink.ok_data();
        assert_eq!(data["sessionPolicy"], "existing");
        assert_eq!(data["sessionRetention"], "keep");
        assert_eq!(data["conversationMode"], "workspace");
        assert_eq!(data["triggerKind"], "manual", "no trigger stated");
        assert_eq!(
            data["nextDueAt"],
            Value::Null,
            "nothing schedules a manual task"
        );
    }

    /// The zone crosses the wire in both directions: the phone sees the one
    /// the desktop resolved, and what it sends back is validated (a typo is
    /// refused rather than replaced silently) and stored canonically.
    #[tokio::test]
    async fn the_task_zone_round_trips_through_the_phone_payload() {
        let _home = home("business-tasks-timezone");

        let sink = RecordingSink::default();
        let mut cmd = command("create_task");
        cmd.task = json!({
            "name": "tokyo morning",
            "prompt": "p",
            "cwd": "/tmp/repo",
            "triggerKind": "schedule",
            "trigger": {"mode": "daily", "time": "09:00"},
            "timezone": "asia/tokyo",
        });
        super::execute(&cmd, &sink).await;
        let created = sink.ok_data();
        assert_eq!(created["timezone"], "Asia/Tokyo", "stored canonically");
        let task_id = created["id"].as_str().expect("an id").to_string();

        // An update that does not mention the zone keeps the stored one.
        let sink = RecordingSink::default();
        let mut cmd = command("update_task");
        cmd.task_id = task_id.clone();
        cmd.task = json!({ "name": "tokyo morning", "prompt": "p2", "cwd": "/tmp/repo" });
        super::execute(&cmd, &sink).await;
        assert_eq!(sink.ok_data()["timezone"], "Asia/Tokyo");

        // An explicit zone replaces it...
        let sink = RecordingSink::default();
        let mut cmd = command("update_task");
        cmd.task_id = task_id.clone();
        cmd.task = json!({
            "name": "tokyo morning",
            "prompt": "p2",
            "cwd": "/tmp/repo",
            "triggerKind": "schedule",
            "trigger": {"mode": "daily", "time": "09:00"},
            "timezone": "Europe/Berlin",
        });
        super::execute(&cmd, &sink).await;
        assert_eq!(sink.ok_data()["timezone"], "Europe/Berlin");

        // ...and a name nobody can resolve is refused, not quietly replaced.
        let sink = RecordingSink::default();
        let mut cmd = command("update_task");
        cmd.task_id = task_id.clone();
        cmd.task = json!({
            "name": "tokyo morning",
            "prompt": "p2",
            "cwd": "/tmp/repo",
            "timezone": "Mars/Olympus_Mons",
        });
        super::execute(&cmd, &sink).await;
        assert!(
            sink.error_text().contains("unknown timezone"),
            "{}",
            sink.error_text()
        );
        let store = open_store().expect("store");
        assert_eq!(
            store
                .get_task(&task_id)
                .unwrap()
                .unwrap()
                .timezone
                .as_deref(),
            Some("Europe/Berlin"),
            "the refused write changed nothing"
        );
    }

    /// A settled run's summary crosses the list command cut to the wire budget,
    /// and the stored one is left whole for the desktop.
    #[tokio::test]
    async fn a_long_run_summary_is_cut_on_the_wire() {
        let _home = home("business-tasks-run-summary");
        let store = open_store().expect("store");
        let saved = task("weekly", future_tasks::SessionPolicy::New);
        store.insert_task(&saved).unwrap();
        let mut run = completed_run(&saved.id);
        run.result_summary = Some("x".repeat(RUN_SUMMARY_CHARS + 50));
        store.insert_run(&run).unwrap();

        let sink = RecordingSink::default();
        let mut cmd = command("list_task_runs");
        cmd.task_id = saved.id.clone();
        super::execute(&cmd, &sink).await;
        let summary = sink.ok_data()["runs"][0]["resultSummary"]
            .as_str()
            .expect("a summary crosses the wire")
            .to_string();
        assert_eq!(
            summary.chars().count(),
            RUN_SUMMARY_CHARS + 1,
            "the cut summary plus its mark"
        );
        assert!(summary.ends_with('…'));
        assert_eq!(
            store.list_runs_for_task(&saved.id, 20).unwrap()[0]
                .result_summary
                .as_deref()
                .map(str::len),
            Some(RUN_SUMMARY_CHARS + 50),
            "the stored summary is untouched"
        );
    }

    /// Pausing, resuming, running on demand and deleting are the four writes
    /// the phone's detail screen makes. Each answers the stored record, and
    /// "run now" leaves the request the tick loop reads — a request the phone
    /// sees as `queued` rather than as an ignored tap.
    #[tokio::test]
    async fn pausing_running_and_deleting_answer_the_stored_record() {
        let _home = home("business-tasks-writes");
        let store = open_store().expect("store");
        let saved = task("weekly", future_tasks::SessionPolicy::New);
        store.insert_task(&saved).unwrap();

        let sink = RecordingSink::default();
        let mut cmd = command("set_task_enabled");
        cmd.task_id = saved.id.clone();
        cmd.enabled = false;
        super::execute(&cmd, &sink).await;
        assert_eq!(sink.ok_data()["enabled"], false);
        assert!(!store.get_task(&saved.id).unwrap().unwrap().enabled);

        let sink = RecordingSink::default();
        let mut cmd = command("set_task_enabled");
        cmd.task_id = saved.id.clone();
        cmd.enabled = true;
        super::execute(&cmd, &sink).await;
        assert_eq!(sink.ok_data()["enabled"], true);
        assert!(store.get_task(&saved.id).unwrap().unwrap().enabled);

        let sink = RecordingSink::default();
        let mut cmd = command("run_task");
        cmd.task_id = saved.id.clone();
        super::execute(&cmd, &sink).await;
        assert_eq!(
            sink.ok_data()["queued"],
            true,
            "the row says the request is waiting"
        );
        let stored = store.get_task(&saved.id).unwrap().unwrap();
        assert!(stored.pending_request_at.is_some());
        assert_eq!(stored.pending_origin, Some(future_tasks::RunOrigin::Ui));
        assert_eq!(stored.pending_actor.as_deref(), Some("phone"));

        let sink = RecordingSink::default();
        let mut cmd = command("delete_task");
        cmd.task_id = saved.id.clone();
        super::execute(&cmd, &sink).await;
        sink.ok_data();
        let stored = store.get_task(&saved.id).unwrap().unwrap();
        assert!(stored.deleted_at.is_some());
        assert!(!stored.enabled, "a deleted task is not left enabled");
        let sink = RecordingSink::default();
        super::execute(&command("list_tasks"), &sink).await;
        assert!(
            sink.ok_data()["tasks"].as_array().unwrap().is_empty(),
            "a deleted task is hidden from the list"
        );
    }

    /// A FutureOS home whose root cannot be created at all — `.future` is a
    /// plain file, which a damaged install or a stray write leaves behind — is
    /// not an empty task list. No task command may answer as if the user had no
    /// tasks, and none may report a write it never made.
    #[tokio::test]
    async fn a_home_that_cannot_hold_a_store_is_reported_by_every_task_command() {
        let _home = home("business-tasks-unopenable-home");
        let blocked = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(".future");
        std::fs::write(&blocked, b"not a directory").expect("block the home root");
        assert!(
            open_store().is_err(),
            "the fixture must make the tasks store unopenable"
        );

        for cmd_type in [
            "list_tasks",
            "get_task",
            "create_task",
            "update_task",
            "delete_task",
            "set_task_enabled",
            "run_task",
            "list_task_runs",
            "list_task_revisions",
            "apply_task_revision",
        ] {
            let sink = RecordingSink::default();
            let mut cmd = command(cmd_type);
            cmd.task_id = "tsk_1".into();
            cmd.revision_id = "rev_1".into();
            cmd.task = json!({"name": "n", "prompt": "p", "cwd": "/tmp"});
            super::execute(&cmd, &sink).await;
            let (success, _data, error) = sink.last();
            assert!(
                !success,
                "{cmd_type} must not answer without a store it could open"
            );
            assert!(
                !error.unwrap_or_default().is_empty(),
                "{cmd_type} must carry the store's own reason"
            );
        }
    }

    /// A task id that no longer exists is "task not found" for every command
    /// that names one — never a silent success the phone would render as done.
    #[tokio::test]
    async fn an_unknown_task_is_named_by_every_command_that_takes_one() {
        let _home = home("business-tasks-unknown-id");
        for cmd_type in [
            "get_task",
            "update_task",
            "delete_task",
            "set_task_enabled",
            "run_task",
        ] {
            let sink = RecordingSink::default();
            let mut cmd = command(cmd_type);
            cmd.task_id = "tsk_gone".into();
            cmd.task = json!({"name": "n", "prompt": "p", "cwd": "/tmp"});
            super::execute(&cmd, &sink).await;
            let (success, _data, error) = sink.last();
            assert!(!success, "{cmd_type} must not succeed for a missing task");
            assert_eq!(error.as_deref(), Some("task not found"), "{cmd_type}");
        }
    }

    /// A row the record decoder refuses is a *read failure*, not an empty
    /// result: the phone must not be told "no tasks" when the truth is "cannot
    /// read this database", and the same goes for a task's run ledger and its
    /// prompt history. The rows are written through a direct connection,
    /// because the store's own write path cannot produce one.
    #[tokio::test]
    async fn a_row_that_cannot_be_decoded_is_a_read_failure_not_an_empty_answer() {
        let _home = home("business-tasks-poisoned-rows");
        open_store().expect("create the schema");
        let path = future_tasks::store::store_path(&crate::future_home_root());
        {
            let conn = rusqlite::Connection::open(&path).expect("open the tasks database");
            conn.execute_batch(
                "INSERT INTO tasks (id, name, enabled, prompt, prompt_version, cwd,
                                    trigger_kind, trigger_json, session_retention,
                                    created_at, updated_at)
                 VALUES ('tsk_ok', 'readable', 1, 'p', 1, '/tmp', 'manual', '{}', 'keep', 1, 1),
                        -- A BLOB, not a number: TEXT affinity would convert a
                        -- number to text and the row would decode cleanly.
                        ('tsk_poison', X'00FF', 1, 'p', 1, '/tmp', 'manual', '{}', 'keep', 1, 1);
                 INSERT INTO task_runs (id, task_id, kind, origin, status, result_summary)
                 VALUES ('run_poison', 'tsk_ok', X'00FF', 'ui', 'completed', 'x');
                 INSERT INTO task_prompt_revisions (id, task_id, version, prompt, source, status, created_at)
                 VALUES ('rev_poison', 'tsk_ok', 1, X'00FF', 'user', 'active', 1);",
            )
            .expect("write rows the decoder refuses");
        }

        // The list and the detail read the poisoned task row itself.
        let sink = RecordingSink::default();
        super::execute(&command("list_tasks"), &sink).await;
        assert!(
            !sink.last().0,
            "an unreadable task list is not an empty task list"
        );
        let sink = RecordingSink::default();
        let mut cmd = command("get_task");
        cmd.task_id = "tsk_poison".into();
        super::execute(&cmd, &sink).await;
        assert!(!sink.last().0, "an unreadable task is not a missing task");

        // The writes read it before they write.
        for cmd_type in ["update_task", "set_task_enabled", "run_task"] {
            let sink = RecordingSink::default();
            let mut cmd = command(cmd_type);
            cmd.task_id = "tsk_poison".into();
            cmd.enabled = false;
            cmd.task = json!({"name": "renamed"});
            super::execute(&cmd, &sink).await;
            let (success, _data, error) = sink.last();
            assert!(
                !success,
                "{cmd_type} must not write over a row it cannot read"
            );
            assert!(
                !error.unwrap_or_default().is_empty(),
                "{cmd_type} must carry the decoder's reason"
            );
        }

        // The ledger and the prompt history fail the same way: their own
        // commands report the fault instead of answering "no runs" / "no
        // versions", which would render as a task that never ran.
        let sink = RecordingSink::default();
        let mut cmd = command("list_task_runs");
        cmd.task_id = "tsk_ok".into();
        super::execute(&cmd, &sink).await;
        assert!(
            !sink.last().0,
            "an unreadable run ledger is not an empty ledger"
        );
        let sink = RecordingSink::default();
        let mut cmd = command("list_task_revisions");
        cmd.task_id = "tsk_ok".into();
        super::execute(&cmd, &sink).await;
        assert!(
            !sink.last().0,
            "an unreadable prompt history is not an empty history"
        );
    }

    /// Applying a revision that does not exist is refused with
    /// the name the store uses, not silently accepted.
    #[tokio::test]
    async fn applying_a_revision_that_does_not_exist_is_refused() {
        let _home = home("business-tasks-apply-missing-revision");
        let store = open_store().expect("store");
        let saved = task("weekly", future_tasks::SessionPolicy::New);
        store.insert_task(&saved).unwrap();

        let sink = RecordingSink::default();
        let mut cmd = command("apply_task_revision");
        cmd.task_id = saved.id.clone();
        cmd.revision_id = "rev_absent".into();
        super::execute(&cmd, &sink).await;
        let (success, _data, error) = sink.last();
        assert!(!success, "a version that does not exist cannot be applied");
        assert!(
            error.is_some_and(|message| !message.is_empty()),
            "the store's own reason is surfaced"
        );
        assert_eq!(
            sink.last().1,
            Value::Null,
            "a refused apply carries no record"
        );
    }

    /// A name is unique among live tasks (the store's own index says so), and
    /// both writes that can hit it surface the store's refusal to the phone
    /// rather than reporting a task that was never stored: a create of a name
    /// already in use, and a rename onto one.
    #[tokio::test]
    async fn a_task_name_already_in_use_is_refused_by_both_writes() {
        let _home = home("business-tasks-duplicate-name");
        let store = open_store().expect("store");
        let saved = task("weekly", future_tasks::SessionPolicy::New);
        store.insert_task(&saved).unwrap();

        let sink = RecordingSink::default();
        let mut cmd = command("create_task");
        cmd.task = json!({
            "name": "weekly",
            "prompt": "another",
            "cwd": "/tmp/repo",
        });
        super::execute(&cmd, &sink).await;
        let (success, _data, error) = sink.last();
        assert!(!success, "a duplicate name must not be created");
        assert!(
            error.is_some_and(|message| !message.is_empty()),
            "the store's own reason is surfaced"
        );
        assert_eq!(
            store.list_tasks(false).unwrap().len(),
            1,
            "the refused create stored nothing"
        );

        let other = task("notes", future_tasks::SessionPolicy::New);
        store.insert_task(&other).unwrap();
        let sink = RecordingSink::default();
        let mut cmd = command("update_task");
        cmd.task_id = other.id.clone();
        cmd.task = json!({"name": "weekly"});
        super::execute(&cmd, &sink).await;
        let (success, _data, error) = sink.last();
        assert!(!success, "a rename onto a live name must not be stored");
        assert!(
            error.is_some_and(|message| !message.is_empty()),
            "the store's own reason is surfaced"
        );
        assert_eq!(
            store.get_task(&other.id).unwrap().unwrap().name,
            "notes",
            "the refused rename left the stored name alone"
        );
    }
}
