//! RPC Server - Command handling for gRPC

mod approval;
mod approval_policy;
mod commands;
mod protocol;
mod session;
mod session_prompt;

use crate::events::EventBus;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub use approval::{ApprovalDecision, ApprovalDecisionStatus, ApprovalGate};
pub use commands::handle_command_internal;
pub use protocol::{RpcCommand, RpcResponse, SseBroadcaster, SseEvent};
pub use session::ServerSession;

// ─── App State ─────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    /// Default session (used when no session_id specified)
    pub session: Arc<RwLock<ServerSession>>,
    /// Additional sessions keyed by session_id
    pub sessions: Arc<RwLock<HashMap<String, Arc<RwLock<ServerSession>>>>>,
    /// Active session ID (for get_state display)
    pub active_session_id: Arc<RwLock<String>>,
    pub welcome_version: String,
    pub welcome_cwd: String,
    pub welcome_skills: Arc<RwLock<Vec<String>>>,
    pub welcome_context: Arc<RwLock<Vec<String>>>,
    pub welcome_exts: Vec<String>,
    pub explicit_session: bool,
    pub broadcaster: Arc<SseBroadcaster>,
    pub event_bus: Arc<EventBus>,
    pub approval_gate: ApprovalGate,
    pub verbose: bool,
}

impl AppState {
    /// Get session by ID, or return default session if id is empty/None
    pub fn get_session(&self, session_id: &str) -> Arc<RwLock<ServerSession>> {
        if session_id.is_empty() {
            return self.session.clone();
        }
        {
            let sessions = self.sessions.read().unwrap();
            if let Some(sess) = sessions.get(session_id) {
                return sess.clone();
            }
        }
        // Session not found in map — if it matches the default session's own
        // ID, return it silently.
        let default_id = self.session.read().unwrap().session_id.clone();
        if session_id == default_id {
            return self.session.clone();
        }

        // Try loading from disk under a write lock to prevent races:
        // two concurrent callers (e.g. StreamEvents + prompt) could
        // both miss the map lookup and create duplicate Session objects
        // with different broadcasters, breaking event delivery.
        {
            let mut sessions = self.sessions.write().unwrap();
            // Double-check: another caller may have loaded it while we waited
            if let Some(sess) = sessions.get(session_id) {
                return sess.clone();
            }

            let (agent_loop, session_manager, event_bus, cwd, approval_gate) = {
                let sess = self.session.read().unwrap();
                if !sess.session_manager.find(session_id).is_some() {
                    return self.session.clone(); // not on disk either
                }
                (
                    sess.agent_loop.clone(),
                    sess.session_manager.clone(),
                    sess.event_bus.clone(),
                    sess.cwd.clone(),
                    sess.approval_gate.clone(),
                )
            };

            let broadcaster = Arc::new(SseBroadcaster::new());
            let mut new_sess = ServerSession::new_with_shared_loop(
                session_id.to_string(),
                agent_loop,
                session_manager.clone(),
                &cwd,
                event_bus,
                broadcaster,
                approval_gate,
            );
            if new_sess.switch_session(session_id).is_ok() {
                // If the session file had no model saved, copy from default
                if new_sess.model.is_empty() {
                    let default_model = self.session.read().unwrap().model.clone();
                    if !default_model.is_empty() {
                        new_sess.model = default_model;
                    }
                }
                let sess_arc = Arc::new(RwLock::new(new_sess));
                sessions.insert(session_id.to_string(), sess_arc.clone());
                return sess_arc;
            }
        }

        self.session.clone()
    }

    pub fn find_session(&self, session_id: &str) -> Option<Arc<RwLock<ServerSession>>> {
        let sess = self.get_session(session_id);
        // get_session already handles empty, map lookup, default match,
        // and disk fallback. If it returned the default session as a
        // fallback, check whether we actually wanted the default or if
        // the requested session truly doesn't exist.
        if session_id.is_empty() {
            return Some(sess);
        }
        let found_id = sess.read().unwrap().session_id.clone();
        if found_id == session_id || session_id == self.session.read().unwrap().session_id.clone() {
            Some(sess)
        } else {
            // get_session fell back to default but the requested session
            // doesn't match the default — session not found.
            None
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
            .unwrap()
            .insert(id.clone(), Arc::new(RwLock::new(session)));
        if let Ok(mut active_id) = self.active_session_id.try_write() {
            *active_id = id.clone();
        }
        id
    }

    /// Get active session ID
    pub fn get_active_session_id(&self) -> String {
        self.active_session_id.read().unwrap().clone()
    }
}

fn get_state_internal(state: &AppState, session_id: &str) -> serde_json::Value {
    let session = state.get_session(session_id);
    let sess = session.read().unwrap();

    // Resolve context window: registry first (user models), then builtin, then default
    let registry = crate::models::Registry::new();
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

    // Compute cost from model pricing
    let total_cost = if let Some(model_config) = registry.resolve(&sess.model) {
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
    let msg_count = sess.messages.read().unwrap().len();
    let context_percent = if context_window > 0 {
        (context_tokens as f64 / context_window as f64) * 100.0
    } else {
        0.0
    };

    serde_json::json!({
        "model": sess.model,
        "imageSupport": image_support,
        "thinkingLevel": sess.thinking_level,
        "isStreaming": sess.is_streaming.load(std::sync::atomic::Ordering::Relaxed),
        "isCompacting": false,
        "steeringMode": sess.steering_mode,
        "followUpMode": sess.follow_up_mode,
        "sessionFile": if session_id.is_empty() { serde_json::Value::Null } else { serde_json::Value::String("".to_string()) },
        "sessionId": if session_id.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(session_id) },
        "sessionName": serde_json::Value::Null,
        "explicitSession": state.explicit_session,
        "autoCompactionEnabled": sess.auto_compaction,
        "messageCount": msg_count,
        "pendingMessageCount": sess.agent_loop.try_read().map(|l|l.pending_message_count()).unwrap_or(0),
        "version": crate::utils::VERSION,
        "cwd": cwd,
        "skills": state.welcome_skills.read().unwrap().clone(),
        "contextFiles": state.welcome_context.read().unwrap().clone(),
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
        "cwd": sess.cwd.clone(),
    })
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
