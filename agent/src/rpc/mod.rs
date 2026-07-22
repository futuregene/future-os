//! RPC Server - Command handling for gRPC

mod approval;
mod commands;
mod prompt_helpers;
mod protocol;
mod session;
mod session_prompt;

use crate::events::EventBus;
use crate::models::Registry as ModelRegistry;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub use approval::{ApprovalDecision, ApprovalDecisionStatus, ApprovalGate};
pub use commands::handle_command_internal;
pub use protocol::{RpcCommand, RpcResponse, SseBroadcaster, SseEvent};
pub use session::ServerSession;

// ─── App State ─────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    /// All live sessions keyed by session_id.  Sessions are equal peers —
    /// there is no privileged "default"/"current" session; clients address
    /// sessions explicitly and the agent hydrates them on demand.
    pub sessions: Arc<RwLock<HashMap<String, Arc<RwLock<ServerSession>>>>>,
    /// On-disk session store (JSONL).  Used for hydration and sessionless
    /// disk operations (delete, fork previews).
    pub session_manager: Arc<crate::session::Manager>,
    pub welcome_version: String,
    pub welcome_cwd: String,
    pub welcome_skills: Arc<RwLock<Vec<String>>>,
    pub welcome_context: Arc<RwLock<Vec<String>>>,
    pub welcome_exts: Vec<String>,
    pub explicit_session: bool,
    pub event_bus: Arc<EventBus>,
    pub approval_gate: ApprovalGate,
    pub verbose: bool,
    /// When true, new prompt/steer/follow_up requests are rejected.  Existing
    /// streaming runs continue to completion.  Read-only and control commands
    /// (abort, status, etc.) are still accepted.
    pub shutting_down: Arc<AtomicBool>,
    /// Cached model registry populated once at startup.  Avoids repeated
    /// blocking network I/O on every get_state → Registry::new() call.
    pub model_registry: Arc<RwLock<ModelRegistry>>,
    /// Template for minting per-session agent loops (`Loop::independent_copy`).
    /// Every session gets its OWN loop — never a shared one — so a streaming
    /// run's long-held read lock can't block another session's `set_model`
    /// (`try_write`), and interrupt flags / steering queues / tool hooks /
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
                return Some(sess.clone());
            }
        }
        if self.session_manager.find(session_id).is_none() {
            return None;
        }

        // Load session from disk OUTSIDE any lock — switch_session parses
        // the JSONL file and can be slow for large histories.
        //
        // The hydrated session gets its OWN agent loop (minted from the
        // template), so switch_session → set_model configures only this
        // session's provider and can never fail with "agent is currently
        // streaming" just because ANOTHER session is mid-run.
        let broadcaster = Arc::new(SseBroadcaster::new());
        let mut new_sess = ServerSession::new(
            session_id.to_string(),
            Arc::new(tokio::sync::RwLock::new(
                self.loop_template.independent_copy(),
            )),
            self.session_manager.clone(),
            &self.welcome_cwd.clone(),
            self.event_bus.clone(),
            broadcaster,
            self.approval_gate.clone(),
            self.model_registry.clone(),
        );
        if new_sess.switch_session(session_id).is_err() {
            return None;
        }
        // If the session file had no model saved, fall back to the default
        // — via set_model, which also rebuilds the loop's provider client.
        // A bare `new_sess.model = ...` would leave the loop pointing at the
        // template's startup model/endpoint.
        if new_sess.model.is_empty() {
            let default_model = crate::models::get_default_model()
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
                return Some(sess.clone());
            }
            let sess_arc = Arc::new(RwLock::new(new_sess));
            sessions.insert(session_id.to_string(), sess_arc.clone());
            Some(sess_arc)
        }
    }

    /// Create a new session and return its ID.
    /// Each session gets its own private SseBroadcaster so events are only
    /// delivered to subscribers of that specific session (not globally).
    pub fn create_session(&self, mut session: ServerSession) -> String {
        let id = session.session_id.clone();
        session.broadcaster = Arc::new(SseBroadcaster::new());
        self.sessions
            .write()
            .insert(id.clone(), Arc::new(RwLock::new(session)));
        id
    }

    /// Refresh the in-memory API key of every live session from auth.json.
    /// Invoked (via the `reload_auth` command) when the GUI changes credentials
    /// out-of-band — FutureGene login/logout, custom-provider key edits — so no
    /// running session keeps using a stale key. Sessions actively streaming are
    /// skipped by `reload_credentials` and pick up the new key on their next
    /// `set_model`.
    pub fn reload_all_credentials(&self) {
        let sessions = self.sessions.read();
        for sess in sessions.values() {
            sess.read().reload_credentials();
        }
    }
}

fn get_state_internal(state: &AppState, session_id: &str) -> Option<serde_json::Value> {
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
    // Query count: number of user messages (prompts, steering, follow-ups).
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

    let parent_session_id = sess
        .session_manager
        .load(&session_id)
        .map(|s| s.parent_session_id)
        .unwrap_or_default();

    Some(serde_json::json!({
        "model": sess.model,
        "imageSupport": image_support,
        "thinkingLevel": sess.thinking_level,
        "isStreaming": sess.is_streaming.load(std::sync::atomic::Ordering::Relaxed),
        "isCompacting": false,
        "steeringMode": sess.steering_mode,
        "followUpMode": sess.follow_up_mode,
        "sessionFile": if session_id.is_empty() { serde_json::Value::Null } else { serde_json::Value::String("".to_string()) },
        "sessionId": if session_id.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(session_id) },
        "session_name": if sess.session_name.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(sess.session_name.clone()) },
        "explicitSession": state.explicit_session,
        "autoCompactionEnabled": sess.auto_compaction,
        "queryCount": query_count,
        "pendingMessageCount": sess.agent_loop.try_read().map(|l|l.pending_message_count()).unwrap_or(0),
        "version": crate::utils::VERSION,
        "cwd": cwd,
        "skills": state.welcome_skills.read().clone(),
        "contextFiles": state.welcome_context.read().clone(),
        "extensions": serde_json::Value::Null,
        "contextWindow": context_window,
        "contextTokens": context_tokens,
        "contextPercent": context_percent,
        "tokensIn": tokens_in,
        "tokensOut": tokens_out,
        "tokensCacheR": cache_r,
        "tokensCacheW": cache_w,
        "totalCost": total_cost,
        "permissionLevel": sess.permission_level.clone(),
        "parentSessionId": if parent_session_id.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(parent_session_id) },
        "createdBy": sess.created_by.clone(),
        "sourceMeta": sess.source_meta.clone(),
    }))
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
