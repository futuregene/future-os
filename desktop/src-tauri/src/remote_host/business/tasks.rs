//! Remote task commands (phone ↔ desktop). The phone manages tasks and reads
//! their run ledgers; execution stays on the desktop's tick loop.
//!
//! Wire budget: `list_tasks` returns only the fields a phone list needs —
//! never the prompt body — and run summaries are truncated. `get_task` returns
//! the full record for the detail screen.

use crate::remote::protocol::IncomingCmd;
use crate::remote::services::ReplySink;
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

fn open_store() -> Result<future_tasks::Store, crate::AppError> {
    let home = crate::future_home_root();
    future_tasks::Store::open(&home).map_err(|e| crate::AppError::Message(e.to_string()))
}

fn run_summary_view(run: future_tasks::TaskRun) -> Value {
    json!({
        "id": run.id,
        "kind": format!("{:?}", run.kind).to_lowercase(),
        "origin": format!("{:?}", run.origin).to_lowercase(),
        "status": format!("{:?}", run.status).to_lowercase(),
        "threadId": run.thread_id,
        "startedAt": run.started_at,
        "finishedAt": run.finished_at,
        "promptVersion": run.prompt_version,
        "resultSummary": run.result_summary.as_deref().map(|s| truncate(s, RUN_SUMMARY_CHARS)),
        "errorMessage": run.error_message,
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
        "nextDueAt": task.next_due_at,
        "lastRunAt": task.last_run_at,
        "reflection": format!("{:?}", task.reflection).to_lowercase(),
        "latestRun": latest.map(run_summary_view),
    })
}

/// The detail record (the only place the prompt crosses the wire).
fn task_detail_view(store: &future_tasks::Store, task: future_tasks::Task) -> Value {
    let latest = store.latest_run_for_task(&task.id).ok().flatten();
    json!({
        "id": task.id,
        "name": task.name,
        "enabled": task.enabled,
        "prompt": task.prompt,
        "promptVersion": task.prompt_version,
        "cwd": task.cwd,
        "modelId": task.model_id,
        "thinkingLevel": task.thinking_level,
        "sessionPolicy": format!("{:?}", task.session_policy).to_lowercase(),
        "triggerKind": format!("{:?}", task.trigger_kind).to_lowercase(),
        "trigger": task.trigger_json,
        "depJoin": format!("{:?}", task.dep_join).to_lowercase(),
        "nextDueAt": task.next_due_at,
        "lastRunAt": task.last_run_at,
        "reflection": format!("{:?}", task.reflection).to_lowercase(),
        "latestRun": latest.map(run_summary_view),
    })
}

fn parse_session_policy(raw: Option<&str>) -> future_tasks::SessionPolicy {
    match raw {
        Some("existing") => future_tasks::SessionPolicy::Existing,
        _ => future_tasks::SessionPolicy::New,
    }
}

fn parse_reflection(raw: Option<&str>) -> future_tasks::Reflection {
    match raw {
        Some("off") => future_tasks::Reflection::Off,
        Some("auto") => future_tasks::Reflection::Auto,
        _ => future_tasks::Reflection::Ask,
    }
}

fn parse_trigger_kind(raw: Option<&str>) -> future_tasks::TriggerKind {
    match raw {
        Some("schedule") => future_tasks::TriggerKind::Schedule,
        _ => future_tasks::TriggerKind::Manual,
    }
}

fn parse_dep_join(raw: Option<&str>) -> future_tasks::DepJoin {
    match raw {
        Some("any") => future_tasks::DepJoin::Any,
        _ => future_tasks::DepJoin::All,
    }
}

/// Build a Task from the phone's `task` payload (a whole-record write, the
/// same convention as provider writes).
fn task_from_payload(
    payload: &Value,
    existing: Option<future_tasks::Task>,
) -> Result<future_tasks::Task, crate::AppError> {
    let now = now_ms();
    let base = existing;
    let name = payload
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| base.as_ref().map(|t| t.name.clone()))
        .ok_or_else(|| crate::AppError::Message("name is required".into()))?;
    let prompt = payload
        .get("prompt")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| base.as_ref().map(|t| t.prompt.clone()))
        .ok_or_else(|| crate::AppError::Message("prompt is required".into()))?;
    let cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| base.as_ref().map(|t| t.cwd.clone()))
        .ok_or_else(|| crate::AppError::Message("cwd is required".into()))?;

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
        model_id: payload
            .get("modelId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| base.as_ref().and_then(|t| t.model_id.clone())),
        thinking_level: payload
            .get("thinkingLevel")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| base.as_ref().and_then(|t| t.thinking_level.clone())),
        session_policy: payload
            .get("sessionPolicy")
            .and_then(Value::as_str)
            .map(|raw| parse_session_policy(Some(raw)))
            .or_else(|| base.as_ref().map(|t| t.session_policy))
            .unwrap_or(future_tasks::SessionPolicy::New),
        thread_id: base.as_ref().and_then(|t| t.thread_id.clone()),
        trigger_kind,
        trigger_json: trigger,
        dep_join: payload
            .get("depJoin")
            .and_then(Value::as_str)
            .map(|raw| parse_dep_join(Some(raw)))
            .or_else(|| base.as_ref().map(|t| t.dep_join))
            .unwrap_or(future_tasks::DepJoin::All),
        next_due_at: None,
        last_run_at: base.as_ref().and_then(|t| t.last_run_at),
        pending_request_at: base.as_ref().and_then(|t| t.pending_request_at),
        pending_origin: base.as_ref().and_then(|t| t.pending_origin),
        pending_actor: base.as_ref().and_then(|t| t.pending_actor.clone()),
        reflection: payload
            .get("reflection")
            .and_then(Value::as_str)
            .map(|raw| parse_reflection(Some(raw)))
            .or_else(|| base.as_ref().map(|t| t.reflection))
            .unwrap_or(future_tasks::Reflection::Ask),
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
                    .map_err(|e| crate::AppError::Message(e.to_string()))
            }) {
                Ok(task) => reply(sink, true, task_detail_view(&store, task), None).await,
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
                let previous_version = existing.prompt_version;
                match task_from_payload(&cmd.task, Some(existing)).and_then(|task| {
                    let saved = task.clone();
                    store
                        .update_task(&task)
                        .map(|()| saved)
                        .map_err(|e| crate::AppError::Message(e.to_string()))
                }) {
                    Ok(task) => {
                        if task.prompt_version != previous_version {
                            let revision = future_tasks::PromptRevision {
                                id: future_tasks::new_revision_id(),
                                task_id: task.id.clone(),
                                version: task.prompt_version,
                                prompt: task.prompt.clone(),
                                source: "user".to_string(),
                                status: "active".to_string(),
                                reason: Some("edited from phone".to_string()),
                                confidence: None,
                                source_run_id: None,
                                created_at: now_ms(),
                            };
                            let _ = store.insert_revision(&revision);
                        }
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
                    .map_err(|e| crate::AppError::Message(e.to_string()))
                    .and_then(|task| {
                        task.ok_or_else(|| crate::AppError::Message("task not found".into()))
                    })
                    .and_then(|mut task| {
                        task.deleted_at = Some(now_ms());
                        task.enabled = false;
                        task.updated_at = now_ms();
                        store
                            .update_task(&task)
                            .map_err(|e| crate::AppError::Message(e.to_string()))
                    });
                reply_unit(sink, result).await
            }
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
        "set_task_enabled" => match open_store() {
            Ok(store) => {
                let enabled = cmd.enabled;
                let result = store
                    .get_task(&cmd.task_id)
                    .map_err(|e| crate::AppError::Message(e.to_string()))
                    .and_then(|task| {
                        task.ok_or_else(|| crate::AppError::Message("task not found".into()))
                    })
                    .and_then(|mut task| {
                        task.enabled = enabled;
                        task.updated_at = now_ms();
                        store
                            .update_task(&task)
                            .map_err(|e| crate::AppError::Message(e.to_string()))?;
                        Ok(task)
                    });
                match result {
                    Ok(task) => reply(sink, true, task_detail_view(&store, task), None).await,
                    Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
                }
            }
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
        "run_task" => match open_store() {
            Ok(store) => {
                let result = store
                    .get_task(&cmd.task_id)
                    .map_err(|e| crate::AppError::Message(e.to_string()))
                    .and_then(|task| {
                        task.ok_or_else(|| crate::AppError::Message("task not found".into()))
                    })
                    .and_then(|mut task| {
                        task.pending_request_at = Some(now_ms());
                        task.pending_origin = Some(future_tasks::RunOrigin::Ui);
                        task.pending_actor = Some("phone".to_string());
                        task.updated_at = now_ms();
                        store
                            .update_task(&task)
                            .map_err(|e| crate::AppError::Message(e.to_string()))?;
                        Ok(task)
                    });
                match result {
                    Ok(task) => reply(sink, true, task_detail_view(&store, task), None).await,
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
        "list_task_deps" => match open_store() {
            Ok(store) => match store.list_deps(&cmd.task_id) {
                Ok(deps) => {
                    let mut items = Vec::new();
                    for dep in deps {
                        let name = store
                            .get_task(&dep.upstream_task_id)
                            .ok()
                            .flatten()
                            .map(|t| t.name)
                            .unwrap_or_else(|| dep.upstream_task_id.clone());
                        let satisfied = store
                            .get_dep_state(&cmd.task_id, &dep.upstream_task_id)
                            .ok()
                            .flatten()
                            .and_then(|s| s.satisfied_run_id)
                            .is_some();
                        items.push(json!({
                            "upstreamTaskId": dep.upstream_task_id,
                            "upstreamName": name,
                            "on": format!("{:?}", dep.on).to_lowercase(),
                            "satisfied": satisfied,
                        }));
                    }
                    reply(sink, true, json!({ "deps": items }), None).await
                }
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            },
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
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
                let result = store
                    .list_revisions(&cmd.task_id)
                    .map_err(|e| crate::AppError::Message(e.to_string()))
                    .and_then(|revisions| {
                        revisions
                            .into_iter()
                            .find(|r| r.id == cmd.revision_id)
                            .ok_or_else(|| crate::AppError::Message("revision not found".into()))
                    })
                    .and_then(|revision| {
                        let mut task = store
                            .get_task(&cmd.task_id)
                            .map_err(|e| crate::AppError::Message(e.to_string()))?
                            .ok_or_else(|| crate::AppError::Message("task not found".into()))?;
                        task.prompt = revision.prompt;
                        task.prompt_version += 1;
                        task.updated_at = now_ms();
                        let applied = future_tasks::PromptRevision {
                            id: future_tasks::new_revision_id(),
                            task_id: task.id.clone(),
                            version: task.prompt_version,
                            prompt: task.prompt.clone(),
                            source: "rollback".to_string(),
                            status: "active".to_string(),
                            reason: Some(format!("applied revision {}", revision.id)),
                            confidence: None,
                            source_run_id: None,
                            created_at: now_ms(),
                        };
                        store
                            .insert_revision(&applied)
                            .map_err(|e| crate::AppError::Message(e.to_string()))?;
                        store
                            .update_task(&task)
                            .map_err(|e| crate::AppError::Message(e.to_string()))?;
                        Ok(task)
                    });
                match result {
                    Ok(task) => reply(sink, true, task_detail_view(&store, task), None).await,
                    Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
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
