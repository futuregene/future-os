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
        "depCount": dep_count(store, &task.id),
        "nextDueAt": task.next_due_at,
        "queued": task.pending_request_at.is_some(),
        "reflection": format!("{:?}", task.reflection).to_lowercase(),
        // A suggestion is the one thing on this page the user has to act on, so
        // the list counts them without the detail round-trip.
        "pendingProposals": pending_proposals(store, &task.id),
        "latestRun": latest.map(run_summary_view),
    })
}

/// Upstream dependencies of a task: the phone's form needs the count to say
/// whether the trigger is "dependency" or a schedule, and its list to label a
/// task the same way the desktop does.
fn dep_count(store: &future_tasks::Store, task_id: &str) -> usize {
    store.list_deps(task_id).map(|deps| deps.len()).unwrap_or(0)
}

/// Prompt suggestions awaiting a decision.
fn pending_proposals(store: &future_tasks::Store, task_id: &str) -> usize {
    store
        .list_revisions(task_id)
        .map(|rows| {
            rows.iter()
                .filter(|row| row.status == future_tasks::REVISION_STATUS_PROPOSED)
                .count()
        })
        .unwrap_or(0)
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
        "conversationMode": format!("{:?}", task.conversation_mode).to_lowercase(),
        "triggerKind": format!("{:?}", task.trigger_kind).to_lowercase(),
        "trigger": task.trigger_json,
        "depJoin": format!("{:?}", task.dep_join).to_lowercase(),
        "depCount": dep_count(store, &task.id),
        "nextDueAt": task.next_due_at,
        "queued": task.pending_request_at.is_some(),
        "reflection": format!("{:?}", task.reflection).to_lowercase(),
        "pendingProposals": pending_proposals(store, &task.id),
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

/// The condition a dependency edge fires on. A bare `set_task_dep` (no `on`)
/// means the successful finish, the same default the CLI's `--depends-on NAME`
/// and the desktop's add-row take.
fn parse_dep_on(raw: &str) -> future_tasks::DepOn {
    match raw {
        "failure" => future_tasks::DepOn::Failure,
        "completed" => future_tasks::DepOn::Completed,
        _ => future_tasks::DepOn::Success,
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
        session_policy: payload
            .get("sessionPolicy")
            .and_then(Value::as_str)
            .map(|raw| parse_session_policy(Some(raw)))
            .or_else(|| base.as_ref().map(|t| t.session_policy))
            .unwrap_or(future_tasks::SessionPolicy::New),
        conversation_mode: payload
            .get("conversationMode")
            .and_then(Value::as_str)
            .map(|raw| parse_conversation_mode(Some(raw)))
            .or_else(|| base.as_ref().map(|t| t.conversation_mode))
            .unwrap_or(future_tasks::ConversationMode::Workspace),
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
                    .map_err(|e| crate::AppError::Message(e.to_string()))
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
                                .map_err(|e| crate::AppError::Message(e.to_string()))?;
                        }
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
                            .map_err(|e| crate::AppError::Message(e.to_string()))?;
                        // Edges pointing at this task go with it: a deleted
                        // upstream can never report another run, so a downstream
                        // left holding that edge waits on a name that no longer
                        // resolves. The desktop panel does the same.
                        store
                            .remove_deps_pointing_at(&task.id)
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
        "set_task_dep" | "remove_task_dep" => match open_store() {
            Ok(store) => {
                let result = if cmd.cmd_type == "remove_task_dep" {
                    store
                        .remove_dep(&cmd.task_id, &cmd.upstream_task_id)
                        .map_err(|e| crate::AppError::Message(e.to_string()))
                } else {
                    let dep = future_tasks::TaskDep {
                        task_id: cmd.task_id.clone(),
                        upstream_task_id: cmd.upstream_task_id.clone(),
                        on: parse_dep_on(&cmd.on),
                    };
                    // The same cycle refusal the desktop panel and the CLI go
                    // through: a phone write is a write like any other, and an
                    // edge that closes a loop would never fire again.
                    let with_edge = (|| -> Result<Vec<future_tasks::TaskDep>, crate::AppError> {
                        let mut all = store
                            .list_all_deps()
                            .map_err(|e| crate::AppError::Message(e.to_string()))?;
                        all.retain(|d| {
                            !(d.task_id == dep.task_id
                                && d.upstream_task_id == dep.upstream_task_id)
                        });
                        all.push(dep.clone());
                        Ok(all)
                    })();
                    match with_edge {
                        Err(error) => Err(error),
                        Ok(all) if future_tasks::would_cycle(&dep.task_id, &all) => Err(
                            crate::AppError::Message("dependency cycle detected".to_string()),
                        ),
                        Ok(_) => store
                            .add_dep(&dep)
                            .map_err(|e| crate::AppError::Message(e.to_string())),
                    }
                };
                reply_unit(sink, result).await
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
                        // The phone's tap starts the run now, exactly like the
                        // desktop button (see `crate::tasks::wake`).
                        crate::tasks::wake();
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
                                "sourceRunId": r.source_run_id,
                                // The whole prompt, not just a preview: a
                                // suggestion is something the user accepts or
                                // rejects, and 160 characters cannot be judged.
                                // `promptPreview` stays for older clients.
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
                    Ok(task) => reply(sink, true, task_detail_view(&store, task), None).await,
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
    use crate::remote::protocol::IncomingCmd;
    use crate::remote::test_support::{HomeGuard, RecordingSink};

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
            session_policy: policy,
            conversation_mode: future_tasks::ConversationMode::Workspace,
            thread_id: None,
            trigger_kind: future_tasks::TriggerKind::Schedule,
            trigger_json: serde_json::json!({"mode": "daily", "time": "09:00"}),
            dep_join: future_tasks::DepJoin::All,
            next_due_at: Some(now + 60_000),
            pending_request_at: None,
            pending_origin: None,
            pending_actor: None,
            reflection: future_tasks::Reflection::Ask,
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
        assert_eq!(
            items[0]["pendingProposals"], 0,
            "a suggestion is counted for the phone's row badge"
        );

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

    /// What a suggestion carries over the wire: the whole prompt (a suggestion
    /// is a decision, and 160 characters cannot be decided on) and the run it
    /// read, so the phone can put it under that run's result.
    #[tokio::test]
    async fn a_suggestion_crosses_the_wire_with_its_prompt_and_run() {
        let _home = home("business-tasks-suggestion-wire");
        let store = open_store().expect("store");
        let saved = task("suggests", future_tasks::SessionPolicy::New);
        store.insert_task(&saved).unwrap();
        store
            .insert_revision(&future_tasks::PromptRevision {
                id: "rev_suggestion".into(),
                task_id: saved.id.clone(),
                version: future_tasks::PROPOSAL_VERSION,
                prompt: "summarise the week, then write reports/weekly.md".into(),
                source: future_tasks::REVISION_SOURCE_REFLECTION.into(),
                status: future_tasks::REVISION_STATUS_PROPOSED.into(),
                reason: Some("the output path was not stated".into()),
                confidence: Some(0.82),
                source_run_id: Some("trn_7".into()),
                created_at: 5,
            })
            .unwrap();

        let sink = RecordingSink::default();
        let mut cmd = command("list_task_revisions");
        cmd.task_id = saved.id.clone();
        super::execute(&cmd, &sink).await;
        let suggestion = &sink.ok_data()["revisions"][0];
        assert_eq!(
            suggestion["prompt"],
            "summarise the week, then write reports/weekly.md"
        );
        assert_eq!(suggestion["sourceRunId"], "trn_7");
        // The preview stays for a phone paired with an older desktop.
        assert!(suggestion["promptPreview"].as_str().is_some());
    }

    /// Accepting a suggestion from the phone has to mean what it means on the
    /// desktop. It used to record an unrelated `rollback` row and leave the
    /// suggestion pending: a button that looked like it worked and did not.
    #[tokio::test]
    async fn the_phone_accepts_a_suggestion_the_way_the_desktop_does() {
        let _home = home("business-tasks-suggestion-apply");
        let store = open_store().expect("store");
        let saved = task("accepts", future_tasks::SessionPolicy::New);
        store.insert_task(&saved).unwrap();
        store
            .insert_revision(&future_tasks::PromptRevision {
                id: "rev_suggestion".into(),
                task_id: saved.id.clone(),
                version: future_tasks::PROPOSAL_VERSION,
                prompt: "summarise the week, then write reports/weekly.md".into(),
                source: future_tasks::REVISION_SOURCE_REFLECTION.into(),
                status: future_tasks::REVISION_STATUS_PROPOSED.into(),
                reason: Some("the output path was not stated".into()),
                confidence: Some(0.82),
                source_run_id: Some("trn_7".into()),
                created_at: 5,
            })
            .unwrap();

        let sink = RecordingSink::default();
        let mut cmd = command("apply_task_revision");
        cmd.task_id = saved.id.clone();
        cmd.revision_id = "rev_suggestion".into();
        super::execute(&cmd, &sink).await;
        assert_eq!(sink.ok_data()["promptVersion"], 4);
        assert_eq!(
            sink.ok_data()["prompt"],
            "summarise the week, then write reports/weekly.md"
        );

        let history = store.list_revisions(&saved.id).unwrap();
        let suggestion = history
            .iter()
            .find(|r| r.id == "rev_suggestion")
            .expect("the suggestion row");
        assert_eq!(suggestion.status, future_tasks::REVISION_STATUS_APPLIED);
        assert_eq!(suggestion.version, future_tasks::PROPOSAL_VERSION);
        let live = history
            .iter()
            .find(|r| r.version == 4)
            .expect("the live version");
        assert_eq!(live.source, future_tasks::REVISION_SOURCE_REFLECTION);
        assert_eq!(
            live.reason.as_deref(),
            Some("the output path was not stated"),
            "the version keeps the reason the user read"
        );
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

    /// Deleting from the phone takes the dependency edges pointing at the task,
    /// matching the desktop panel: a downstream left holding that edge would
    /// wait forever on an upstream that can never report again.
    #[tokio::test]
    async fn deleting_from_the_phone_drops_the_edges_pointing_at_it() {
        let _home = home("business-tasks-delete-edges");
        let store = open_store().expect("store");
        let upstream = task("upstream", future_tasks::SessionPolicy::New);
        let mut downstream = task("downstream", future_tasks::SessionPolicy::New);
        downstream.name = "downstream".into();
        store.insert_task(&upstream).unwrap();
        store.insert_task(&downstream).unwrap();
        store
            .add_dep(&future_tasks::TaskDep {
                task_id: downstream.id.clone(),
                upstream_task_id: upstream.id.clone(),
                on: future_tasks::DepOn::Success,
            })
            .unwrap();
        store
            .mark_dep_satisfied(&downstream.id, &upstream.id, "trn_1", 1)
            .unwrap();
        assert_eq!(store.list_deps(&downstream.id).unwrap().len(), 1);

        let sink = RecordingSink::default();
        let mut cmd = command("delete_task");
        cmd.task_id = upstream.id.clone();
        super::execute(&cmd, &sink).await;
        sink.ok_data();

        assert!(
            store.list_deps(&downstream.id).unwrap().is_empty(),
            "the downstream waits on nothing"
        );
        assert!(store
            .get_dep_state(&downstream.id, &upstream.id)
            .unwrap()
            .is_none());
        assert!(store
            .get_task(&upstream.id)
            .unwrap()
            .unwrap()
            .deleted_at
            .is_some());
    }

    /// The phone can wire a dependency the same way the panel and the CLI do:
    /// set an edge with its condition, and take it away again.
    #[tokio::test]
    async fn the_phone_sets_and_removes_a_dependency_edge() {
        let _home = home("business-tasks-set-dep");
        let store = open_store().expect("store");
        let upstream = task("upstream", future_tasks::SessionPolicy::New);
        let downstream = task("downstream", future_tasks::SessionPolicy::New);
        store.insert_task(&upstream).unwrap();
        store.insert_task(&downstream).unwrap();

        let sink = RecordingSink::default();
        let mut cmd = command("set_task_dep");
        cmd.task_id = downstream.id.clone();
        cmd.upstream_task_id = upstream.id.clone();
        cmd.on = "failure".into();
        super::execute(&cmd, &sink).await;
        sink.ok_data();

        let deps = store.list_deps(&downstream.id).unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].upstream_task_id, upstream.id);
        assert_eq!(deps[0].on, future_tasks::DepOn::Failure);

        // A bare set (no condition) means the successful finish, the same
        // default the CLI's `--depends-on NAME` takes.
        let sink = RecordingSink::default();
        let mut again = command("set_task_dep");
        again.task_id = downstream.id.clone();
        again.upstream_task_id = upstream.id.clone();
        super::execute(&again, &sink).await;
        sink.ok_data();
        let deps = store.list_deps(&downstream.id).unwrap();
        assert_eq!(deps.len(), 1, "the same edge, not a second one");
        assert_eq!(deps[0].on, future_tasks::DepOn::Success);

        let sink = RecordingSink::default();
        let mut remove = command("remove_task_dep");
        remove.task_id = downstream.id.clone();
        remove.upstream_task_id = upstream.id.clone();
        super::execute(&remove, &sink).await;
        sink.ok_data();
        assert!(store.list_deps(&downstream.id).unwrap().is_empty());
    }

    /// A phone write is a write like any other: an edge that closes a loop is
    /// refused here too, with the same wording the panel shows.
    #[tokio::test]
    async fn the_phone_refuses_a_dependency_cycle() {
        let _home = home("business-tasks-dep-cycle");
        let store = open_store().expect("store");
        let a = task("a", future_tasks::SessionPolicy::New);
        let b = task("b", future_tasks::SessionPolicy::New);
        store.insert_task(&a).unwrap();
        store.insert_task(&b).unwrap();
        store
            .add_dep(&future_tasks::TaskDep {
                task_id: b.id.clone(),
                upstream_task_id: a.id.clone(),
                on: future_tasks::DepOn::Success,
            })
            .unwrap();

        let sink = RecordingSink::default();
        let mut cmd = command("set_task_dep");
        cmd.task_id = a.id.clone();
        cmd.upstream_task_id = b.id.clone();
        super::execute(&cmd, &sink).await;
        assert!(sink.error_text().contains("dependency cycle"));
        assert!(
            store.list_deps(&a.id).unwrap().is_empty(),
            "a refused edge is not stored"
        );
    }
}
