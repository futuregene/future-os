//! RPC Server - Command handling for gRPC

mod approval;
mod commands;
mod prompt_helpers;
mod protocol;
mod session;
mod session_prompt;

// Wire payload carriers live in the future-rpc crate (typed-RPC milestone);
// the agent keeps constructing them via these re-exports.
pub(crate) use future_rpc::payloads;

use crate::models::Registry as ModelRegistry;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub use approval::{ApprovalDecision, ApprovalDecisionStatus, ApprovalGate};
pub use commands::handle_command_internal;
pub use protocol::{RpcCommand, RpcResponse, SseBroadcaster, SseEvent};
pub use session::ServerSession;

/// Map one broadcaster/journal event into its replay payload carrier. The
/// wire type lives in the future-rpc crate; the mapping needs the
/// agent-internal `SseEvent`, so this adapter stays on the agent side.
pub(crate) fn replay_event_payload(event: &SseEvent) -> payloads::ReplayEventPayload {
    payloads::ReplayEventPayload {
        event_type: event.event_type.clone(),
        data: event.data.clone(),
        run_id: event.run_id.clone(),
        idx: event.idx,
        session_id: event.session_id.clone(),
        epoch: event.epoch,
        event_id: event.event_id.clone(),
        timestamp: event.timestamp.clone(),
        session_idx: event.session_idx,
        run_sequence: event.run_sequence,
    }
}

// ─── App State ─────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    /// Changes on every Agent process start. Clients use it to distinguish a
    /// reconnect to the same in-memory scheduler from a restart that dropped
    /// queued runs and process-local idempotency state.
    pub agent_instance_id: String,
    /// All live sessions keyed by session_id.  Sessions are equal peers —
    /// there is no privileged "default"/"current" session; clients address
    /// sessions explicitly and the agent hydrates them on demand.
    pub sessions: Arc<RwLock<HashMap<String, Arc<RwLock<ServerSession>>>>>,
    pub queue_budget: Arc<crate::runtime::GlobalQueueBudget>,
    /// On-disk session store (JSONL).  Used for hydration and sessionless
    /// disk operations (delete, fork previews).
    pub session_manager: Arc<crate::session::Manager>,
    pub welcome_version: String,
    pub welcome_cwd: String,
    pub welcome_skills: Arc<RwLock<Vec<String>>>,
    pub welcome_context: Arc<RwLock<Vec<String>>>,
    pub welcome_exts: Vec<String>,
    pub explicit_session: bool,
    pub approval_gate: ApprovalGate,
    pub verbose: bool,
    /// When true, new prompt requests are rejected. Existing
    /// streaming runs continue to completion.  Read-only and control commands
    /// (abort, status, etc.) are still accepted.
    pub shutting_down: Arc<AtomicBool>,
    /// Cached model registry populated once at startup.  Avoids repeated
    /// blocking network I/O on every get_state → Registry::new() call.
    pub model_registry: Arc<RwLock<ModelRegistry>>,
    /// Template for minting per-session agent loops (`Loop::independent_copy`).
    /// Every session gets its OWN loop — never a shared one — so a streaming
    /// run's long-held read lock can't block another session's `set_model`
    /// (`try_write`), and interrupt flags / tool hooks /
    /// token counters stay session-local.  The template itself is never used
    /// to run prompts.
    pub loop_template: Arc<crate::agent::Loop>,
}

impl AppState {
    /// Resolve a session by id: in-memory hit, else hydrate from disk.
    /// Returns None for an empty id or an id that exists neither in memory
    /// nor on disk — callers NEVER silently receive a different session
    /// (the old default-session fallback leaked one conversation's state
    /// into another's caller).
    ///
    /// Disk loading (switch_session → JSONL parse) happens **outside** the
    /// write lock.  Only the final map insertion acquires the write lock
    /// (with a double-check), so a slow session load never stalls concurrent
    /// session lookups.
    pub fn get_session(&self, session_id: &str) -> Option<Arc<RwLock<ServerSession>>> {
        if session_id.is_empty() {
            return None;
        }
        {
            let sessions = self.sessions.read();
            if let Some(sess) = sessions.get(session_id) {
                let sess = sess.clone();
                drop(sessions);
                ServerSession::ensure_scheduler_worker(&sess);
                return Some(sess);
            }
        }
        self.session_manager.find(session_id)?;

        // Load session from disk OUTSIDE any lock — switch_session parses
        // the JSONL file and can be slow for large histories.
        //
        // The hydrated session gets its OWN agent loop (minted from the
        // template), so switch_session → set_model configures only this
        // session's provider and can never fail with "agent is currently
        // streaming" just because ANOTHER session is mid-run.
        let broadcaster = Arc::new(SseBroadcaster::new());
        let mut new_sess = ServerSession::new_with_queue_budget(
            session_id.to_string(),
            Arc::new(tokio::sync::RwLock::new(
                self.loop_template.independent_copy(),
            )),
            self.session_manager.clone(),
            &self.welcome_cwd.clone(),
            broadcaster,
            self.approval_gate.clone(),
            self.model_registry.clone(),
            self.queue_budget.clone(),
        );
        if new_sess.switch_session(session_id).is_err() {
            return None;
        }
        // If the session file had no model saved, fall back to the default
        // — via set_model, which also rebuilds the loop's provider client.
        // A bare `new_sess.model = ...` would leave the loop pointing at the
        // template's startup model/endpoint.
        if new_sess.model.is_empty() {
            let default_model = crate::models::get_default_model_with(&self.model_registry.read())
                .unwrap_or_else(|| self.loop_template.model.clone());
            if !default_model.is_empty() {
                if let Err(e) = new_sess.set_model(&default_model) {
                    tracing::warn!("[session] could not apply default model on hydrate: {e}");
                }
            }
        }

        // Only acquire the write lock for the final insertion — double-check
        // that another caller didn't beat us to it while we were loading.
        {
            let mut sessions = self.sessions.write();
            if let Some(sess) = sessions.get(session_id) {
                let sess = sess.clone();
                drop(sessions);
                ServerSession::ensure_scheduler_worker(&sess);
                return Some(sess);
            }
            let sess_arc = Arc::new(RwLock::new(new_sess));
            sessions.insert(session_id.to_string(), sess_arc.clone());
            drop(sessions);
            ServerSession::ensure_scheduler_worker(&sess_arc);
            Some(sess_arc)
        }
    }

    /// Create a new session and return its ID.
    /// Each session gets its own private SseBroadcaster so events are only
    /// delivered to subscribers of that specific session (not globally) —
    /// fork/clone pass the parent's broadcaster in and must not keep sharing
    /// it. The journal is (re)bound to the broadcaster that will actually
    /// broadcast: construction configured one that may be discarded here, and
    /// an unbound broadcaster silently holds events in memory only.
    pub fn create_session(&self, mut session: ServerSession) -> String {
        let id = session.session_id.clone();
        session.broadcaster = Arc::new(SseBroadcaster::new());
        if let Err(error) = session
            .broadcaster
            .configure_journal(id.clone(), session.session_manager.run_data_path(&id))
        {
            tracing::error!(session_id = %id, "failed to configure event journal: {error:#}");
        }
        let session = Arc::new(RwLock::new(session));
        self.sessions.write().insert(id.clone(), session.clone());
        ServerSession::ensure_scheduler_worker(&session);
        id
    }

    /// Refresh the in-memory API key of every live session from auth.json.
    /// Invoked by the `reload_auth` command and, since audit item 2, inline by
    /// the config-write commands (`set_auth` / `upsert_provider` /
    /// `delete_provider`) after the agent writes its own auth.json/models.json
    /// — FutureGene login/logout, custom-provider key edits — so no running
    /// session keeps using a stale key. Sessions actively streaming are
    /// skipped by `reload_credentials` and pick up the new key on their next
    /// `set_model`.
    ///
    /// Sessions that were created before any credentials existed (model is empty)
    /// get a default model resolved and applied, so the user can prompt immediately
    /// after login without switching threads.
    pub fn reload_all_credentials(&self) {
        let sessions = self.sessions.read();
        for sess in sessions.values() {
            let model_is_empty = { sess.read().model.is_empty() };
            if model_is_empty {
                // Session created before login — resolve and apply default model.
                // Re-check emptiness inside the write lock to avoid TOCTOU: another
                // thread may have set a model between our read and write locks.
                if let Some(mut session) = sess.try_write() {
                    if session.model.is_empty() {
                        let registry = self.model_registry.read();
                        let default_model =
                            crate::models::get_default_model_with(&registry).unwrap_or_default();
                        if !default_model.is_empty() {
                            let _ = session.set_model(&default_model);
                        }
                    } else {
                        // Model was set concurrently — just refresh credentials
                        session.reload_credentials();
                    }
                }
            } else {
                sess.read().reload_credentials();
            }
        }
    }
}

fn get_state_internal(
    state: &AppState,
    session_id: &str,
    requested_run_id: Option<&str>,
) -> Option<serde_json::Value> {
    let session = state.get_session(session_id)?;
    let sess = session.read();

    // Resolve context window: use the cached model registry from AppState.
    // Avoids repeated blocking network I/O from Registry::new() on every poll.
    let registry = state.model_registry.read();
    let context_window = registry
        .resolve(&sess.model)
        .map(|m| m.context_window)
        .or_else(|| {
            crate::models::builtin_models()
                .into_iter()
                .find(|m| m.id == sess.model)
                .map(|m| m.context_window)
        })
        .unwrap_or(200000) as i64;

    let image_support = registry
        .resolve(&sess.model)
        .map(|m| m.input.contains(&"image".to_string()))
        .unwrap_or(false);

    let session_id = sess.session_id();
    let cwd = sess.cwd.clone();

    // Read cumulative token usage directly from Arc<AtomicI64> — lock-free
    use std::sync::atomic::Ordering;
    let tokens_in = sess.tokens_in.load(Ordering::Relaxed);
    let tokens_out = sess.tokens_out.load(Ordering::Relaxed);
    let cache_r = sess.tokens_cache_r.load(Ordering::Relaxed);
    let cache_w = sess.tokens_cache_w.load(Ordering::Relaxed);

    // Prefer API-reported cost (Future platform returns `credit_cost` in
    // the usage chunk).  When absent (most non-Future providers don't
    // report it), fall back to token-count × model-price estimation.
    let api_cost = *sess.cumulative_cost.lock();
    let total_cost = if api_cost > 0.0 {
        api_cost
    } else if let Some(model_config) = registry.resolve(&sess.model) {
        let input_cost = (tokens_in as f64 / 1_000_000.0) * model_config.cost.input;
        let output_cost = (tokens_out as f64 / 1_000_000.0) * model_config.cost.output;
        let cache_read_cost = (cache_r as f64 / 1_000_000.0) * model_config.cost.cache_read;
        let cache_write_cost = (cache_w as f64 / 1_000_000.0) * model_config.cost.cache_write;
        input_cost + output_cost + cache_read_cost + cache_write_cost
    } else {
        0.0
    };

    // Use API-reported prompt_tokens from the last request as actual context usage
    let context_tokens = sess.last_prompt_tokens.load(Ordering::Relaxed);
    // Query count: number of user messages (prompts and follow-ups).
    // Excludes internal tool/assistant messages.
    let query_count = sess
        .messages
        .read()
        .iter()
        .filter(|m| m.role == "user")
        .count();
    let context_percent = if context_window > 0 {
        (context_tokens as f64 / context_window as f64) * 100.0
    } else {
        0.0
    };

    let loaded = sess.session_manager.load(&session_id).ok();
    let parent_session_id = loaded
        .as_ref()
        .map(|s| s.parent_session_id.clone())
        .unwrap_or_default();
    let active_run = sess
        .runtime
        .snapshot()
        .map(|run| payloads::RunStateSnapshot {
            run_id: run.run_id,
            epoch: Some(run.epoch),
            run_sequence: run.run_sequence,
            state: run.phase.as_str().to_string(),
            last_event_idx: Some(sess.broadcaster.last_idx()),
        });
    let queued_runs = sess
        .scheduler
        .queued()
        .into_iter()
        .enumerate()
        .map(|(index, run)| {
            let display_text = run
                .payload
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            payloads::QueuedRunState {
                run_id: run.run_id,
                run_sequence: run.run_sequence,
                client_request_id: run.client_request_id,
                state: "queued".to_string(),
                queue_position: index + 1,
                accepted_at: run.accepted_at,
                display_text,
            }
        })
        .collect::<Vec<_>>();
    let queued_count = queued_runs.len();
    let recent_terminal_acks = sess
        .scheduler
        .recent_terminal_acks()
        .into_iter()
        .map(|ack| payloads::TerminalAck {
            run_id: ack.run_id,
            run_sequence: ack.run_sequence,
            client_request_id: ack.client_request_id,
            state: ack.state,
            reason: ack.reason,
        })
        .collect::<Vec<_>>();
    // Restart recovery: when no run is live but the journal still records a run
    // that began without committing (a run_started marker with no run_terminal),
    // the previous run was interrupted by a crash or agent restart. Surface it
    // explicitly so clients never mistake it for a completed run.
    let interrupted_run = if active_run.is_none() {
        loaded
            .as_ref()
            .and_then(|s| crate::session::find_unterminated_run(&s.entries))
            .map(|run_id| payloads::RunStateSnapshot {
                run_id,
                epoch: None,
                run_sequence: None,
                state: crate::session::RUN_STATE_INTERRUPTED_BY_RESTART.to_string(),
                last_event_idx: None,
            })
    } else {
        None
    };
    let requested_run = requested_run_id
        .filter(|run_id| !run_id.is_empty())
        .and_then(|run_id| {
            loaded
                .as_ref()
                .and_then(|session| crate::session::find_run_terminal(&session.entries, run_id))
        });
    // Approvals this session is parked on, with the full card payload. A client
    // that (re)connects after a crash uses this to rebuild approval UI it
    // missed — the in-memory pending map is the live source, the broadcast
    // event is only a notification.
    let pending_approvals = state.approval_gate.pending_for_session(&session_id);

    // Typed payload (audit item 1): canonical camelCase keys from the
    // GetStatePayload struct, plus legacy aliases for the spellings that
    // pre-migration clients still read (`session_name`, snake_case ack keys).
    let mut payload = serde_json::to_value(payloads::GetStatePayload {
        agent_instance_id: state.agent_instance_id.clone(),
        model: sess.model.clone(),
        image_support,
        thinking_level: sess.thinking_level.clone(),
        is_streaming: sess.is_streaming.load(std::sync::atomic::Ordering::Relaxed),
        is_compacting: false,
        session_file: if session_id.is_empty() {
            None
        } else {
            Some(String::new())
        },
        session_id: if session_id.is_empty() {
            None
        } else {
            Some(session_id.clone())
        },
        session_name: if sess.session_name.is_empty() {
            None
        } else {
            Some(sess.session_name.clone())
        },
        explicit_session: state.explicit_session,
        auto_compaction_enabled: sess.auto_compaction,
        query_count,
        version: crate::utils::VERSION.to_string(),
        cwd,
        skills: state.welcome_skills.read().clone(),
        context_files: state.welcome_context.read().clone(),
        extensions: None,
        context_window,
        context_tokens,
        context_percent,
        tokens_in,
        tokens_out,
        tokens_cache_r: cache_r,
        tokens_cache_w: cache_w,
        total_cost,
        permission_level: sess.permission_level.clone(),
        parent_session_id: if parent_session_id.is_empty() {
            None
        } else {
            Some(parent_session_id)
        },
        created_by: sess.created_by.clone(),
        source_meta: sess.source_meta.clone(),
        active_run,
        queued_runs,
        recent_terminal_acks,
        queued_count,
        interrupted_run,
        requested_run,
        pending_approvals,
    })
    .unwrap_or_default();
    payloads::inject_legacy_aliases(&mut payload, &[("sessionName", "session_name")]);
    if let Some(acks) = payload
        .get_mut("recentTerminalAcks")
        .and_then(serde_json::Value::as_array_mut)
    {
        for ack in acks {
            payloads::inject_legacy_aliases(
                ack,
                &[
                    ("runId", "run_id"),
                    ("runSequence", "run_sequence"),
                    ("clientRequestId", "client_request_id"),
                ],
            );
        }
    }
    Some(payload)
}

/// Generate HTML representation of a session (matches Go exportSessionToHTML)
pub(super) fn generate_session_html(
    session_id: &str,
    model: &str,
    cwd: &str,
    messages: &[crate::types::Message],
) -> String {
    let mut html = String::new();

    html.push_str("<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\">");
    html.push_str(&format!(
        "<title>FutureAgent session {}</title>",
        session_id
    ));
    html.push_str("<style>");
    html.push_str("body{font-family:system-ui;max-width:800px;margin:auto;padding:20px;background:#1a1a2e;color:#e0e0e0}");
    html.push_str(".user{background:#16213e;padding:10px;margin:5px 0;border-radius:8px}");
    html.push_str(".assistant{background:#0f3460;padding:10px;margin:5px 0;border-radius:8px}");
    html.push_str(
        ".tool{background:#1a1a1a;padding:10px;margin:5px 0;border-radius:8px;font-size:0.9em}",
    );
    html.push_str("pre{white-space:pre-wrap;word-wrap:break-word}");
    html.push_str("</style></head><body>\n");
    html.push_str(&format!("<h1>FutureAgent Session: {}</h1>\n", session_id));
    html.push_str(&format!("<p>Model: {} | CWD: {}</p>\n", model, cwd));

    for msg in messages {
        let cls = match msg.role.as_str() {
            "assistant" => "assistant",
            "tool" => "tool",
            _ => "user",
        };
        let content = match &msg.content {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
            None => String::new(),
        };
        html.push_str(&format!(
            "<div class=\"{}\"><strong>{}</strong><pre>{}</pre></div>\n",
            cls,
            escape_html(&msg.role),
            escape_html(&content)
        ));
    }

    html.push_str("</body></html>");
    html
}

/// Escape HTML special characters
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// The file path a tool call operates on: the first of `path` / `file_path` /
/// `filePath` present in its arguments (a JSON object, or a JSON string that
/// parses to one). Shared by the approval gate and the prompt path rewriter.
fn argument_path(arguments: &serde_json::Value) -> Option<String> {
    let normalized = match arguments {
        serde_json::Value::String(raw) => {
            serde_json::from_str::<serde_json::Value>(raw).unwrap_or_else(|_| arguments.clone())
        }
        _ => arguments.clone(),
    };
    ["path", "file_path", "filePath"]
        .iter()
        .find_map(|key| normalized.get(*key).and_then(|value| value.as_str()))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── escape_html ───────────────────────────────────────────────────────

    #[test]
    fn escape_html_escapes_all_specials() {
        assert_eq!(
            escape_html("<script>alert('xss')</script>"),
            "&lt;script&gt;alert(&apos;xss&apos;)&lt;/script&gt;"
        );
    }

    #[test]
    fn escape_html_escapes_ampersand_first() {
        assert_eq!(escape_html("a & b"), "a &amp; b");
    }

    #[test]
    fn escape_html_escapes_quotes() {
        assert_eq!(escape_html("\"quoted\""), "&quot;quoted&quot;");
        assert_eq!(escape_html("'single'"), "&apos;single&apos;");
    }

    #[test]
    fn escape_html_empty_string() {
        assert_eq!(escape_html(""), "");
    }

    #[test]
    fn escape_html_no_specials() {
        assert_eq!(escape_html("hello world"), "hello world");
    }

    // ─── argument_path ─────────────────────────────────────────────────────

    #[test]
    fn argument_path_extracts_path() {
        let args = serde_json::json!({"path": "/tmp/file.txt"});
        assert_eq!(argument_path(&args), Some("/tmp/file.txt".to_string()));
    }

    #[test]
    fn argument_path_extracts_file_path() {
        let args = serde_json::json!({"file_path": "/tmp/file.txt"});
        assert_eq!(argument_path(&args), Some("/tmp/file.txt".to_string()));
    }

    #[test]
    fn argument_path_extracts_camel_case() {
        let args = serde_json::json!({"filePath": "/tmp/file.txt"});
        assert_eq!(argument_path(&args), Some("/tmp/file.txt".to_string()));
    }

    #[test]
    fn argument_path_prefers_path_over_others() {
        let args = serde_json::json!({"path": "/tmp/a.txt", "file_path": "/tmp/b.txt"});
        assert_eq!(argument_path(&args), Some("/tmp/a.txt".to_string()));
    }

    #[test]
    fn argument_path_from_string_json() {
        let args = serde_json::json!("{\"path\": \"/tmp/file.txt\"}");
        assert_eq!(argument_path(&args), Some("/tmp/file.txt".to_string()));
    }

    #[test]
    fn argument_path_no_path_returns_none() {
        let args = serde_json::json!({"command": "ls"});
        assert_eq!(argument_path(&args), None);
    }

    #[test]
    fn argument_path_empty_json_returns_none() {
        let args = serde_json::json!({});
        assert_eq!(argument_path(&args), None);
    }

    // ─── generate_session_html ─────────────────────────────────────────────

    #[test]
    fn generate_session_html_contains_title() {
        let html = generate_session_html("sess-123", "gpt-4o", "/tmp/test", &[]);
        assert!(html.contains("FutureAgent session sess-123"));
        assert!(html.contains("gpt-4o"));
        assert!(html.contains("/tmp/test"));
    }

    #[test]
    fn generate_session_html_with_messages() {
        let messages = vec![
            crate::types::Message {
                role: "user".to_string(),
                content: Some(serde_json::json!("hello")),
                ..Default::default()
            },
            crate::types::Message {
                role: "assistant".to_string(),
                content: Some(serde_json::json!("hi there")),
                ..Default::default()
            },
            crate::types::Message {
                role: "tool".to_string(),
                content: Some(serde_json::json!("result")),
                ..Default::default()
            },
        ];
        let html = generate_session_html("s1", "model", "/cwd", &messages);
        assert!(html.contains("class=\"user\""));
        assert!(html.contains("class=\"assistant\""));
        assert!(html.contains("class=\"tool\""));
        assert!(html.contains("hello"));
        assert!(html.contains("hi there"));
    }

    #[test]
    fn generate_session_html_escapes_content() {
        let messages = vec![crate::types::Message {
            role: "user".to_string(),
            content: Some(serde_json::json!("<script>alert('xss')</script>")),
            ..Default::default()
        }];
        let html = generate_session_html("s1", "model", "/cwd", &messages);
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>alert"));
    }

    #[test]
    fn generate_session_html_empty_messages() {
        let html = generate_session_html("s1", "model", "/cwd", &[]);
        assert!(html.contains("<body>"));
        assert!(html.contains("</body>"));
    }

    #[test]
    fn generate_session_html_null_content() {
        let messages = vec![crate::types::Message {
            role: "assistant".to_string(),
            content: None,
            ..Default::default()
        }];
        let html = generate_session_html("s1", "model", "/cwd", &messages);
        assert!(html.contains("assistant"));
    }

    // ─── AppState helpers ──────────────────────────────────────────────────

    #[test]
    fn app_state_get_session_empty_id_returns_none() {
        // Use EmptyProvider from session tests (defined in the same crate)
        struct EmptyP;
        #[async_trait::async_trait]
        impl crate::types::LLMProvider for EmptyP {
            async fn stream_chat(
                &self,
                _model: String,
                _messages: Vec<crate::types::Message>,
                _tools: Vec<crate::types::ToolDef>,
                _system_prompt: String,
            ) -> anyhow::Result<tokio_stream::wrappers::ReceiverStream<crate::types::StreamEvent>>
            {
                let (_tx, rx) = tokio::sync::mpsc::channel(1);
                Ok(tokio_stream::wrappers::ReceiverStream::new(rx))
            }
        }
        let state = AppState {
            agent_instance_id: "agent-test-instance".to_string(),
            sessions: std::sync::Arc::new(parking_lot::RwLock::new(
                std::collections::HashMap::new(),
            )),
            queue_budget: std::sync::Arc::new(crate::runtime::GlobalQueueBudget::defaults()),
            session_manager: std::sync::Arc::new(crate::session::Manager::new(
                std::path::PathBuf::from("/tmp/futureos-test-sessions"),
            )),
            welcome_version: "1.0".to_string(),
            welcome_cwd: "/tmp".to_string(),
            welcome_skills: std::sync::Arc::new(parking_lot::RwLock::new(vec![])),
            welcome_context: std::sync::Arc::new(parking_lot::RwLock::new(vec![])),
            welcome_exts: vec![],
            explicit_session: false,
            approval_gate: crate::rpc::ApprovalGate::default(),
            verbose: false,
            shutting_down: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            model_registry: std::sync::Arc::new(parking_lot::RwLock::new(
                crate::models::Registry::new(),
            )),
            loop_template: std::sync::Arc::new(crate::agent::Loop::new(
                std::sync::Arc::new(EmptyP),
                "test-model",
            )),
        };
        assert!(state.get_session("").is_none());
    }

    // ─── coverage batch: hydrate + get_state arms ──────────────────────────

    fn bare_app_state() -> (tempfile::TempDir, AppState) {
        struct EmptyP;
        #[async_trait::async_trait]
        impl crate::types::LLMProvider for EmptyP {
            async fn stream_chat(
                &self,
                _model: String,
                _messages: Vec<crate::types::Message>,
                _tools: Vec<crate::types::ToolDef>,
                _system_prompt: String,
            ) -> anyhow::Result<tokio_stream::wrappers::ReceiverStream<crate::types::StreamEvent>>
            {
                let (_tx, rx) = tokio::sync::mpsc::channel(1);
                Ok(tokio_stream::wrappers::ReceiverStream::new(rx))
            }
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let session_dir = dir.path().join("sessions");
        let state = AppState {
            agent_instance_id: "agent-test-instance".to_string(),
            sessions: std::sync::Arc::new(parking_lot::RwLock::new(
                std::collections::HashMap::new(),
            )),
            queue_budget: std::sync::Arc::new(crate::runtime::GlobalQueueBudget::defaults()),
            session_manager: std::sync::Arc::new(crate::session::Manager::new(session_dir)),
            welcome_version: "1.0".to_string(),
            welcome_cwd: "/tmp".to_string(),
            welcome_skills: std::sync::Arc::new(parking_lot::RwLock::new(vec![])),
            welcome_context: std::sync::Arc::new(parking_lot::RwLock::new(vec![])),
            welcome_exts: vec![],
            explicit_session: false,
            approval_gate: crate::rpc::ApprovalGate::default(),
            verbose: false,
            shutting_down: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            model_registry: std::sync::Arc::new(parking_lot::RwLock::new(
                crate::models::Registry::new(),
            )),
            loop_template: std::sync::Arc::new(crate::agent::Loop::new(
                std::sync::Arc::new(EmptyP),
                "test-model",
            )),
        };
        (dir, state)
    }

    #[test]
    fn get_session_hydrates_disk_session_without_model() {
        let (_dir, state) = bare_app_state();
        // A persisted session with no model info anywhere → the hydrate path
        // applies the registry/loop default model.
        let snapshot = crate::session::Session::snapshot(
            "hyd-1".to_string(),
            "/tmp".to_string(),
            String::new(),
            String::new(),
            String::new(),
            vec![crate::session::SessionEntry::new_user(
                "user",
                serde_json::json!("hello"),
            )],
        );
        state.session_manager.save(&snapshot).unwrap();

        let session = state.get_session("hyd-1").expect("hydrated from disk");
        assert!(
            !session.read().model.is_empty(),
            "default model applied on hydrate"
        );
        // Second fetch hits the live map.
        assert!(state.get_session("hyd-1").is_some());
    }

    #[test]
    fn get_state_reports_active_run_and_estimates_cost() {
        let (_dir, state) = bare_app_state();
        let snapshot = crate::session::Session::snapshot(
            "s-run".to_string(),
            "/tmp".to_string(),
            "deepseek/deepseek-chat".to_string(),
            String::new(),
            String::new(),
            vec![
                crate::session::SessionEntry::session_info(
                    serde_json::json!({"cwd": "/tmp", "model": "deepseek/deepseek-chat"}),
                    "deepseek/deepseek-chat".to_string(),
                    "low".to_string(),
                ),
                crate::session::SessionEntry::run_started("run-done", 1),
                crate::session::SessionEntry::run_terminal(
                    "run-done",
                    crate::session::RUN_STATE_COMPLETED,
                    5,
                    100,
                    None,
                ),
            ],
        );
        state.session_manager.save(&snapshot).unwrap();

        let session = state.get_session("s-run").unwrap();
        session
            .read()
            .runtime
            .begin(Some("run-live"), Some("request-live"))
            .unwrap();
        // Token counters make the token×price estimation arm observable.
        session
            .read()
            .tokens_in
            .store(1_000_000, std::sync::atomic::Ordering::Relaxed);

        let value = get_state_internal(&state, "s-run", Some("run-done")).expect("state");
        assert_eq!(value["activeRun"]["runId"], "run-live");
        assert_eq!(value["requestedRun"]["run_id"], "run-done");
        // deepseek-chat is in the catalog with a non-zero price, so the
        // estimate replaces the (zero) API cost.
        assert!(value["totalCost"].as_f64().unwrap() > 0.0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn get_state_lists_recent_terminal_acks() {
        let (_dir, state) = bare_app_state();
        let snapshot = crate::session::Session::snapshot(
            "s-acks".to_string(),
            "/tmp".to_string(),
            "mock".to_string(),
            String::new(),
            String::new(),
            vec![
                crate::session::SessionEntry::session_info(
                    serde_json::json!({"cwd": "/tmp", "model": "mock"}),
                    "mock".to_string(),
                    "low".to_string(),
                ),
                crate::session::SessionEntry::new_user("user", serde_json::json!("hi")),
            ],
        );
        state.session_manager.save(&snapshot).unwrap();
        let session = state.get_session("s-acks").unwrap();
        // An active run forces the next prompt to queue; cancelling it then
        // records a terminal ack.
        session
            .read()
            .runtime
            .begin(Some("run-blocker"), Some("request-blocker"))
            .unwrap();
        {
            let mut sess = session.write();
            sess.enqueue_prompt(
                "queued",
                &[],
                &[],
                None,
                "req-ack",
                crate::runtime::BusyPolicy::EnqueueIfBusy,
            )
            .unwrap();
            let queued = sess.scheduler.queued();
            let run_id = queued[0].run_id.clone();
            sess.cancel_queued_run(&run_id, crate::runtime::QueuedCancellationReason::Cancelled)
                .unwrap();
        }

        let value = get_state_internal(&state, "s-acks", None).expect("state");
        let acks = value["recentTerminalAcks"].as_array().unwrap();
        assert_eq!(acks.len(), 1);
        assert_eq!(acks[0]["clientRequestId"], "req-ack");
        // Legacy snake_case aliases are dual-written.
        assert!(acks[0].get("client_request_id").is_some());
    }

    #[test]
    fn reload_all_credentials_applies_default_to_model_less_sessions() {
        let _home = crate::test_support::TestHome::new();
        let auth_path = _home.auth_path();
        std::fs::create_dir_all(auth_path.parent().unwrap()).unwrap();
        std::fs::write(&auth_path, r#"{"deepseek": {"type": "api_key", "key": "k"}}"#).unwrap();

        let (_dir, state) = bare_app_state();
        // A live session with NO model — the reload resolves + applies one.
        let session = crate::rpc::ServerSession::new_with_queue_budget(
            "bare".to_string(),
            std::sync::Arc::new(tokio::sync::RwLock::new(crate::agent::Loop::new(
                std::sync::Arc::new({
                    struct EmptyP;
                    #[async_trait::async_trait]
                    impl crate::types::LLMProvider for EmptyP {
                        async fn stream_chat(
                            &self,
                            _model: String,
                            _messages: Vec<crate::types::Message>,
                            _tools: Vec<crate::types::ToolDef>,
                            _system_prompt: String,
                        ) -> anyhow::Result<
                            tokio_stream::wrappers::ReceiverStream<crate::types::StreamEvent>,
                        > {
                            let (_tx, rx) = tokio::sync::mpsc::channel(1);
                            Ok(tokio_stream::wrappers::ReceiverStream::new(rx))
                        }
                    }
                    EmptyP
                }),
                "test-model",
            ))),
            state.session_manager.clone(),
            "/tmp",
            std::sync::Arc::new(SseBroadcaster::new()),
            state.approval_gate.clone(),
            state.model_registry.clone(),
            state.queue_budget.clone(),
        );
        state.create_session(session);

        state.reload_all_credentials();
        let session = state.get_session("bare").unwrap();
        assert!(
            !session.read().model.is_empty(),
            "model-less session got the credentialled default"
        );
    }

    #[test]
    fn generate_session_html_formats_non_string_content() {
        let messages = vec![
            crate::types::Message {
                role: "user".to_string(),
                content: Some(serde_json::json!([{"type": "text", "text": "hi"}])),
                ..Default::default()
            },
            crate::types::Message {
                role: "assistant".to_string(),
                content: None,
                ..Default::default()
            },
        ];
        let html = generate_session_html("s1", "model", "/cwd", &messages);
        assert!(html.contains("user"));
        assert!(html.contains("text"));
    }
}
