//! `future session` — 1:1 port of cli/src/commands/session.ts.
//!
//! list / info / rename / delete agent sessions via gRPC.

use crate::output::Output;
use crate::rpc::{grpc_addr, RunClient};
use chrono::TimeZone;
use serde_json::Value;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// `help()` from session.ts — printed to stdout.
pub const SESSION_HELP: &str = "future session — manage agent sessions

Usage:
  future session list [--json]                       List all sessions
  future session info <id>                           Show session details + stats
  future session rename <id> <name>                  Give a session a readable name
  future session delete <id>                         Delete a session

Session data is stored at ~/.future/agent/sessions/";

fn help(out: &Output) {
    out.log(SESSION_HELP);
}

/// `truncate(s, n)` — cut to n chars (last char replaced with "…").
fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n - 1).collect();
        out.push('…');
        out
    }
}

/// `humanTokens(n)` from session.ts.
fn human_tokens(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{}K", (n as f64 / 1_000.0).round() as i64)
    } else {
        n.to_string()
    }
}

/// `ago(iso)` — relative time from `now_ms` (injected for testability).
fn ago(iso: &str, now_ms: i64) -> String {
    let ms = now_ms - parse_timestamp_ms(iso);
    let mins = ms / 60_000;
    if mins < 1 {
        "just now".to_string()
    } else if mins < 60 {
        format!("{mins}m ago")
    } else {
        let hrs = mins / 60;
        if hrs < 24 {
            format!("{hrs}h ago")
        } else {
            format!("{}d ago", hrs / 24)
        }
    }
}

/// `new Date(iso).getTime()` — RFC3339, or local "YYYY-MM-DD HH:MM:SS"
/// (the `updated_at` format), else 0 (JS gives NaN).
fn parse_timestamp_ms(iso: &str) -> i64 {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(iso) {
        return dt.timestamp_millis();
    }
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(iso, "%Y-%m-%d %H:%M:%S") {
        // Ambiguous/nonexistent local times (DST gaps) fall back to 0 (NaN).
        return chrono::Local
            .from_local_datetime(&naive)
            .single()
            .map(|local| local.timestamp_millis())
            .unwrap_or(0);
    }
    0
}

/// `Date.now()`.
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

// ─── List ─────────────────────────────────────────────────────────────────

/// `listSessions(jsonFlag)`.
async fn list_sessions(json_flag: bool, out: &Output) -> Result<(), String> {
    let client = RunClient::new(&grpc_addr());
    // Not wrapped in try/catch — errors propagate to main().catch.
    let data = client.list_sessions().await?;
    // `const { sessions } = await client.listSessions();`
    let sessions: Vec<Value> = data
        .get("sessions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    if json_flag {
        // Serializing plain JSON values is infallible.
        out.log(&serde_json::to_string_pretty(&json_value(&sessions)).expect("json serializes"));
        return Ok(());
    }

    if sessions.is_empty() {
        out.log("No sessions found.");
        return Ok(());
    }

    // `sessions.sort((a, b) => new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime())`
    let mut sessions = sessions;
    sessions.sort_by(|a, b| {
        let a_ms = a
            .get("updated_at")
            .and_then(Value::as_str)
            .map(parse_timestamp_ms)
            .unwrap_or(0);
        let b_ms = b
            .get("updated_at")
            .and_then(Value::as_str)
            .map(parse_timestamp_ms)
            .unwrap_or(0);
        b_ms.cmp(&a_ms)
    });

    // Header.
    out.log(&format!(
        "  {} {} {} {} QUERIES",
        pad_end("SESSION ID", 24),
        pad_end("TITLE", 38),
        pad_end("UPDATED", 10),
        pad_end("MODEL", 28)
    ));
    out.log(&format!(
        "  {} {} {} {} ———————",
        "—".repeat(24),
        "—".repeat(38),
        "—".repeat(10),
        "—".repeat(28)
    ));

    for s in &sessions {
        let session_name = s.get("session_name").and_then(Value::as_str).unwrap_or("");
        let first_message = s.get("first_message").and_then(Value::as_str).unwrap_or("");
        // `s.session_name || s.first_message ? truncate(...) : "(untitled)"`
        let title = if !session_name.is_empty() || !first_message.is_empty() {
            truncate(
                if !session_name.is_empty() {
                    session_name
                } else {
                    first_message
                },
                42,
            )
        } else {
            "(untitled)".to_string()
        };
        // `s.model.length > 28 ? s.model.slice(0, 27) + "…" : s.model`
        let model_raw = s.get("model").and_then(Value::as_str).unwrap_or("");
        let model = if model_raw.chars().count() > 28 {
            let mut m: String = model_raw.chars().take(27).collect();
            m.push('…');
            m
        } else {
            model_raw.to_string()
        };
        // `s.query_count ? \`${s.query_count}\` : "—"`
        let q = match s.get("query_count").and_then(Value::as_i64) {
            Some(n) if n != 0 => n.to_string(),
            _ => "—".to_string(),
        };
        let id = s.get("id").and_then(Value::as_str).unwrap_or("");
        let updated = s.get("updated_at").and_then(Value::as_str).unwrap_or("");
        out.log(&format!(
            "  {} {} {} {} {}",
            pad_end(id, 24),
            pad_end(&title, 38),
            pad_end(&ago(updated, now_ms()), 10),
            pad_end(&model, 28),
            q
        ));
    }
    out.log(&format!("\n{} sessions.", sessions.len()));
    Ok(())
}

/// `s.padEnd(n)`.
fn pad_end(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count >= n {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(n - count))
    }
}

// ─── Info ─────────────────────────────────────────────────────────────────

/// `info(sessionId)`.
async fn info(session_id: &str, out: &Output) -> Result<(), String> {
    let client = RunClient::new(&grpc_addr());
    let data = client.get_session_entries(session_id).await?;
    // `const { entries } = ...; if (!data.entries || data.entries.length === 0)`
    let entries: Vec<Value> = data
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if entries.is_empty() {
        out.log_err(&format!("Session not found: {session_id}"));
        return Err(crate::HANDLED_EXIT.to_string());
    }

    // `data.entries.find(e => e.role === "system")` — the session_info entry.
    let info_entry = entries
        .iter()
        .find(|e| e.get("role").and_then(Value::as_str) == Some("system"));
    // `(infoEntry?.content ?? {})` — session_info content is the raw JSON object.
    let content = info_entry
        .and_then(|e| e.get("content"))
        .cloned()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));

    // `(infoEntry?.model as string) || (content?.model as string) || "?"`
    let model = info_entry
        .and_then(|e| e.get("model").and_then(Value::as_str))
        .or_else(|| content.get("model").and_then(Value::as_str))
        .unwrap_or("?")
        .to_string();
    let thinking_level = info_entry
        .and_then(|e| e.get("thinking_level").and_then(Value::as_str))
        .or_else(|| content.get("thinking_level").and_then(Value::as_str))
        .unwrap_or("?")
        .to_string();
    let session_name = content
        .get("session_name")
        .and_then(Value::as_str)
        .unwrap_or("(untitled)")
        .to_string();
    let cwd = content
        .get("cwd")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    // Count entries by role/type.
    let mut roles: HashMap<String, i64> = HashMap::new();
    let mut tool_calls = 0i64;
    for e in &entries {
        // `(e.type as string) || (e.role as string) || "?"`
        let t = e
            .get("type")
            .and_then(Value::as_str)
            .or_else(|| e.get("role").and_then(Value::as_str))
            .unwrap_or("?")
            .to_string();
        *roles.entry(t).or_insert(0) += 1;
        // `if (e.tool_calls && Array.isArray(e.tool_calls)) toolCalls += e.tool_calls.length`
        if let Some(tc) = e.get("tool_calls") {
            if let Some(arr) = tc.as_array() {
                tool_calls += arr.len() as i64;
            }
        }
    }
    let users = roles.get("user").copied().unwrap_or(0);
    let assistants = roles.get("assistant").copied().unwrap_or(0);
    let tools = roles.get("tool").copied().unwrap_or(0);
    // `roles.get("session_info") ?? roles.get("system") ?? 0`
    let system = roles
        .get("session_info")
        .copied()
        .or_else(|| roles.get("system").copied())
        .unwrap_or(0);
    let compacted = roles.get("compaction").copied().unwrap_or(0);

    out.log(&format!("Session:  {session_id}"));
    out.log(&format!("  Name:        {session_name}"));
    out.log(&format!("  Model:       {model}"));
    out.log(&format!("  Thinking:    {thinking_level}"));
    if !cwd.is_empty() {
        out.log(&format!("  CWD:         {cwd}"));
    }

    // `Number(content?.tokens_in ?? 0)` etc.
    let tokens_in = content
        .get("tokens_in")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let tokens_out = content
        .get("tokens_out")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let tokens_cache_r = content
        .get("tokens_cache_r")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let tokens_cache_w = content
        .get("tokens_cache_w")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let total_cost = content
        .get("total_cost")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);

    out.log(&format!(
        "  Messages:    {} ({} user, {} assistant, {} tool{}{})",
        entries.len(),
        users,
        assistants,
        tools,
        if system > 0 {
            format!(", {system} system")
        } else {
            String::new()
        },
        if compacted > 0 {
            format!(", {compacted} compacted")
        } else {
            String::new()
        }
    ));
    out.log(&format!("  Tool calls:  {tool_calls}"));
    if tokens_in + tokens_out > 0 {
        out.log(&format!(
            "  Tokens:      in={} out={}",
            human_tokens(tokens_in),
            human_tokens(tokens_out)
        ));
        if tokens_cache_r + tokens_cache_w > 0 {
            out.log(&format!(
                "  Cache:       r={} w={}",
                human_tokens(tokens_cache_r),
                human_tokens(tokens_cache_w)
            ));
        }
        if total_cost > 0.0 {
            out.log(&format!("  Cost:        ${total_cost:.6}"));
        }
    }
    Ok(())
}

// ─── Rename ──────────────────────────────────────────────────────────────

/// `rename(sessionId, name)`.
async fn rename(session_id: &str, name: &str, out: &Output) -> Result<(), String> {
    let client = RunClient::new(&grpc_addr());
    client.rename_session(session_id, name).await?;
    out.log(&format!("Renamed session {session_id} → \"{name}\""));
    Ok(())
}

// ─── Delete ───────────────────────────────────────────────────────────────

/// `deleteSession(sessionId)`.
async fn delete_session(session_id: &str, out: &Output) -> Result<(), String> {
    let client = RunClient::new(&grpc_addr());
    let data = match client.delete_session(session_id).await {
        Ok(data) => data,
        Err(msg) => {
            // `msg.startsWith("failed to delete") ? msg : \`Failed to delete: ${msg}\``
            let msg = if msg.starts_with("failed to delete") {
                msg
            } else {
                format!("Failed to delete: {msg}")
            };
            out.log_err(&msg);
            return Err(crate::HANDLED_EXIT.to_string());
        }
    };
    // `const { deleted } = await client.deleteSession(sessionId);`
    let deleted = data
        .get("deleted")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if deleted {
        out.log(&format!("Deleted session {session_id}"));
    } else {
        out.log_err(&format!("Session not found: {session_id}"));
        return Err(crate::HANDLED_EXIT.to_string());
    }
    Ok(())
}

// ─── Entry ────────────────────────────────────────────────────────────────

/// `session(subcommand, args)`.
pub async fn session(
    subcommand: Option<&str>,
    args: &[String],
    out: &Output,
) -> Result<(), String> {
    // `if (subcommand === "--help" || subcommand === "-h" || !subcommand)`
    let Some(subcommand) = subcommand else {
        help(out);
        return Ok(());
    };
    if subcommand == "--help" || subcommand == "-h" {
        help(out);
        return Ok(());
    }

    if subcommand == "list" {
        list_sessions(args.iter().any(|a| a == "--json"), out).await?;
        return Ok(());
    }

    // `const targetId = args[0]; if (!targetId)`
    let target_id = args.first().cloned().unwrap_or_default();
    if target_id.is_empty() {
        out.log_err(&format!(
            "Usage: future session {subcommand} <session-id>{}",
            if subcommand == "rename" {
                " <name>"
            } else {
                ""
            }
        ));
        return Err(crate::HANDLED_EXIT.to_string());
    }

    match subcommand {
        "info" => {
            info(&target_id, out).await?;
        }
        "rename" => {
            // `const name = args.slice(1).join(" ");`
            let name = args[1..].join(" ");
            if name.is_empty() {
                out.log_err("Usage: future session rename <session-id> <name>");
                return Err(crate::HANDLED_EXIT.to_string());
            }
            rename(&target_id, &name, out).await?;
        }
        "delete" => {
            delete_session(&target_id, out).await?;
        }
        _ => {
            out.log_err(&format!("Unknown command: {subcommand}"));
            help(out);
            return Err(crate::HANDLED_EXIT.to_string());
        }
    }
    Ok(())
}

/// Small helper for the JSON list output (avoids a json! round-trip).
fn json_value(sessions: &[Value]) -> Value {
    Value::Object(
        [("sessions".to_string(), Value::Array(sessions.to_vec()))]
            .into_iter()
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_behavior() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hell…");
        assert_eq!(truncate("", 5), "");
        // UTF-8 safe.
        assert_eq!(truncate("你好世界", 3), "你好…");
    }

    #[test]
    fn human_tokens_behavior() {
        assert_eq!(human_tokens(2_500_000), "2.5M");
        assert_eq!(human_tokens(1_000_000), "1.0M");
        assert_eq!(human_tokens(128_000), "128K");
        assert_eq!(human_tokens(999), "999");
        assert_eq!(human_tokens(0), "0");
    }

    #[test]
    fn ago_behavior() {
        let now = 1_000_000_000_000i64; // fixed epoch (2001-09-09)
                                        // ~30s ago.
        let iso = format_ts(now - 30_000);
        assert_eq!(ago(&iso, now), "just now");
        // 5m ago.
        let iso = format_ts(now - 5 * 60_000);
        assert_eq!(ago(&iso, now), "5m ago");
        // 3h ago.
        let iso = format_ts(now - 3 * 3_600_000);
        assert_eq!(ago(&iso, now), "3h ago");
        // 2d ago.
        let iso = format_ts(now - 2 * 86_400_000);
        assert_eq!(ago(&iso, now), "2d ago");
        // Unparseable → treated as epoch → far past.
        assert_eq!(ago("bogus", now), "11574d ago");
    }

    /// Render `now - delta` as local "YYYY-MM-DD HH:MM:SS".
    fn format_ts(ms: i64) -> String {
        use chrono::TimeZone;
        chrono::Local
            .timestamp_millis_opt(ms)
            .single()
            .unwrap()
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
    }

    #[test]
    fn parse_timestamp_rfc3339() {
        // "2026-08-06T12:00:00Z" — 12:00 UTC on 2026-08-06.
        let ms = parse_timestamp_ms("2026-08-06T12:00:00Z");
        assert_eq!(ms, 1786017600000);
    }

    #[tokio::test]
    async fn session_list_agent_down_propagates() {
        let _guard = crate::test_env::lock_env().await;
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from("127.0.0.1:1"),
        )]);
        let (out, cap) = Output::memory();
        let result = list_sessions(false, &out).await;
        assert!(result.is_err());
        assert_eq!(
            String::from_utf8(cap.out.lock().unwrap().clone()).unwrap(),
            ""
        );
    }

    #[tokio::test]
    async fn session_unknown_subcommand_usage_and_help() {
        // TS prints the missing-target usage BEFORE dispatching subcommands,
        // so `bogus` with no target shows the usage line.
        let (out, cap) = Output::memory();
        let result = session(Some("bogus"), &[], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert_eq!(stderr, "Usage: future session bogus <session-id>\n");

        // With a target id, the unknown-command branch fires: stderr error +
        // help on stdout.
        let (out, cap) = Output::memory();
        let result = session(Some("bogus"), &["sess-1".to_string()], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert_eq!(stderr, "Unknown command: bogus\n");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.starts_with("future session — manage agent sessions"));
    }

    #[tokio::test]
    async fn session_missing_target_id() {
        let (out, cap) = Output::memory();
        let result = session(Some("info"), &[], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert_eq!(stderr, "Usage: future session info <session-id>\n");
    }

    #[tokio::test]
    async fn session_rename_missing_name() {
        let (out, cap) = Output::memory();
        let result = session(Some("rename"), &["sess-1".to_string()], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert_eq!(stderr, "Usage: future session rename <session-id> <name>\n");
    }

    // ── Mock-agent backed flows ─────────────────────────────────────

    /// Point FUTURE_AGENT_GRPC_ADDR at a spawned mock (caller holds ENV_LOCK).
    async fn mock_env(
        agent: crate::test_server::MockAgent,
    ) -> (crate::test_server::MockAgent, crate::test_env::EnvGuard) {
        let addr = crate::test_server::spawn_mock(agent.clone()).await;
        let env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from(addr),
        )]);
        (agent, env)
    }

    #[tokio::test]
    async fn list_empty_sessions() {
        let _guard = crate::test_env::lock_env().await;
        let agent = crate::test_server::MockAgent::respond("list_sessions", "{\"sessions\":[]}");
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        session(Some("list"), &[], &out).await.expect("list");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert_eq!(stdout, "No sessions found.\n");
    }

    #[tokio::test]
    async fn list_json_passthrough() {
        let _guard = crate::test_env::lock_env().await;
        let agent = crate::test_server::MockAgent::respond(
            "list_sessions",
            "{\"sessions\":[{\"id\":\"s1\",\"session_name\":\"one\"}]}",
        );
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        session(Some("list"), &["--json".to_string()], &out)
            .await
            .expect("list");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(parsed["sessions"][0]["id"], "s1");
    }

    #[tokio::test]
    async fn list_table_rendering_and_sorting() {
        let _guard = crate::test_env::lock_env().await;
        let long_model = "a".repeat(30);
        let body = format!(
            "{{\"sessions\":[\
                {{\"id\":\"untitled-old\",\"updated_at\":\"2020-01-01T00:00:00Z\"}},\
                {{\"id\":\"named\",\"session_name\":\"My Session\",\"model\":\"k3\",\"query_count\":3,\"updated_at\":\"2099-01-01T00:00:00Z\"}},\
                {{\"id\":\"first-msg\",\"first_message\":\"hello world this is the first message of the session\",\"model\":\"{long_model}\",\"query_count\":0,\"updated_at\":\"2098-01-01T00:00:00Z\"}}\
            ]}}"
        );
        let agent = crate::test_server::MockAgent::respond("list_sessions", &body);
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        session(Some("list"), &[], &out).await.expect("list");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        // Header + rule + 3 rows + count.
        assert!(stdout.contains("SESSION ID"), "stdout: {stdout}");
        assert!(stdout.contains("QUERIES"), "stdout: {stdout}");
        // Sorted by updated_at desc: named (2099) before first-msg (2098).
        let named = stdout.find("My Session").expect("named row");
        let first_msg = stdout.find("hello world").expect("first-msg row");
        assert!(named < first_msg);
        // Untitled row + zero query count → "—", long model truncated.
        assert!(stdout.contains("(untitled)"), "stdout: {stdout}");
        assert!(
            stdout.contains(&format!("{}…", "a".repeat(27))),
            "stdout: {stdout}"
        );
        assert!(stdout.ends_with("\n3 sessions.\n"), "stdout: {stdout}");
        // A row with a real query count renders the number (single line: a
        // short-circuit chain split across lines leaves the tail uncovered).
        let has_three =
            stdout.contains(" 3\n") || stdout.contains(" 3 \n") || stdout.contains("3\n");
        assert!(has_three, "stdout: {stdout}");
    }

    #[tokio::test]
    async fn list_agent_down_propagates_raw_error() {
        let _guard = crate::test_env::lock_env().await;
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from("127.0.0.1:1"),
        )]);
        let (out, _cap) = Output::memory();
        // Not wrapped in HANDLED_EXIT — the transport error propagates.
        let err = session(Some("list"), &[], &out).await.unwrap_err();
        assert_ne!(err, crate::HANDLED_EXIT);
    }

    #[tokio::test]
    async fn info_full_rendering() {
        let _guard = crate::test_env::lock_env().await;
        let agent = crate::test_server::MockAgent::respond(
            "get_session_entries",
            "{\"entries\":[\
                {\"role\":\"system\",\"content\":{\"session_name\":\"Named\",\"cwd\":\"/work\",\"tokens_in\":2500000,\"tokens_out\":128000,\"tokens_cache_r\":5000,\"tokens_cache_w\":1500,\"total_cost\":0.012345}},\
                {\"role\":\"user\"},\
                {\"role\":\"assistant\",\"tool_calls\":[{},{}]},\
                {\"role\":\"assistant\"},\
                {\"role\":\"tool\"},\
                {\"type\":\"compaction\"},\
                {}\
            ]}",
        );
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        session(Some("info"), &["sess-1".to_string()], &out)
            .await
            .expect("info");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("Session:  sess-1"), "stdout: {stdout}");
        assert!(stdout.contains("Name:        Named"));
        // model falls back to "?" (no model anywhere), thinking "?" too.
        assert!(stdout.contains("Model:       ?"), "stdout: {stdout}");
        assert!(stdout.contains("Thinking:    ?"), "stdout: {stdout}");
        assert!(stdout.contains("CWD:         /work"));
        // 7 entries: 1 user, 2 assistant, 1 tool, 1 system, 1 compaction,
        // and the {} entry counts as "?".
        assert!(
            stdout.contains("Messages:    7 (1 user, 2 assistant, 1 tool, 1 system, 1 compacted)"),
            "stdout: {stdout}"
        );
        assert!(stdout.contains("Tool calls:  2"));
        assert!(
            stdout.contains("Tokens:      in=2.5M out=128K"),
            "stdout: {stdout}"
        );
        assert!(
            stdout.contains("Cache:       r=5K w=2K"),
            "stdout: {stdout}"
        );
        assert!(
            stdout.contains("Cost:        $0.012345"),
            "stdout: {stdout}"
        );
    }

    #[tokio::test]
    async fn info_minimal_entry_and_model_fallbacks() {
        let _guard = crate::test_env::lock_env().await;
        // Model from the entry itself, thinking from content; no cwd/tokens.
        let agent = crate::test_server::MockAgent::respond(
            "get_session_entries",
            "{\"entries\":[{\"role\":\"system\",\"model\":\"m-entry\",\"content\":{\"thinking_level\":\"high\"}}]}",
        );
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        session(Some("info"), &["s1".to_string()], &out)
            .await
            .expect("info");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("Model:       m-entry"), "stdout: {stdout}");
        assert!(stdout.contains("Thinking:    high"), "stdout: {stdout}");
        assert!(
            stdout.contains("Name:        (untitled)"),
            "stdout: {stdout}"
        );
        assert!(!stdout.contains("CWD:"), "stdout: {stdout}");
        assert!(!stdout.contains("Tokens:"), "stdout: {stdout}");
        assert!(!stdout.contains("Cache:"), "stdout: {stdout}");
        assert!(!stdout.contains("Cost:"), "stdout: {stdout}");
    }

    #[tokio::test]
    async fn info_session_not_found() {
        let _guard = crate::test_env::lock_env().await;
        let agent =
            crate::test_server::MockAgent::respond("get_session_entries", "{\"entries\":[]}");
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        let result = session(Some("info"), &["ghost".to_string()], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert_eq!(stderr, "Session not found: ghost\n");
    }

    #[tokio::test]
    async fn info_and_rename_rpc_failures_propagate() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::default();
        agent.fail_types.insert("get_session_entries".into());
        agent.fail_types.insert("set_session_name".into());
        let (_agent, _env) = mock_env(agent).await;
        let (out, _cap) = Output::memory();
        let result = session(Some("info"), &["s1".to_string()], &out).await;
        assert!(result.is_err());
        let result = session(Some("rename"), &["s1".to_string(), "x".to_string()], &out).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn info_sparse_entries_take_default_arms() {
        let _guard = crate::test_env::lock_env().await;
        // No system-role entry, non-array tool_calls, zero cost: all the
        // default/false arms in the info renderer.
        let agent = crate::test_server::MockAgent::respond(
            "get_session_entries",
            "{\"entries\":[{\"role\":\"user\",\"tool_calls\":5,\"content\":null}]}",
        );
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        session(Some("info"), &["s1".to_string()], &out)
            .await
            .expect("info");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("Model:       ?"), "stdout: {stdout}");
        assert!(stdout.contains("user"), "stdout: {stdout}");
    }

    #[tokio::test]
    async fn rename_success() {
        let _guard = crate::test_env::lock_env().await;
        let agent = crate::test_server::MockAgent::default();
        let (agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        session(
            Some("rename"),
            &["sess-1".to_string(), "new".to_string(), "name".to_string()],
            &out,
        )
        .await
        .expect("rename");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert_eq!(stdout, "Renamed session sess-1 → \"new name\"\n");
        let seen = agent.seen_of("set_session_name");
        assert_eq!(seen[0].name, "new name");
        assert_eq!(seen[0].session_id, "sess-1");
    }

    #[tokio::test]
    async fn delete_outcomes() {
        let _guard = crate::test_env::lock_env().await;
        // deleted: true.
        let agent = crate::test_server::MockAgent::respond("delete_session", "{\"deleted\":true}");
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        session(Some("delete"), &["s1".to_string()], &out)
            .await
            .expect("delete");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert_eq!(stdout, "Deleted session s1\n");
        drop(_env);

        // deleted: false → not found.
        let agent = crate::test_server::MockAgent::respond("delete_session", "{\"deleted\":false}");
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        let result = session(Some("delete"), &["ghost".to_string()], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert_eq!(stderr, "Session not found: ghost\n");
        drop(_env);

        // RPC failure → "Failed to delete: <msg>".
        let mut agent = crate::test_server::MockAgent::default();
        agent.fail_types.insert("delete_session".into());
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        let result = session(Some("delete"), &["s1".to_string()], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert_eq!(stderr, "Failed to delete: boom\n");
        drop(_env);

        // Failure already prefixed passes through untouched.
        let mut agent = crate::test_server::MockAgent::default();
        agent
            .fail_with
            .insert("delete_session".into(), "failed to delete: busy".into());
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        let result = session(Some("delete"), &["s1".to_string()], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert_eq!(stderr, "failed to delete: busy\n");
    }

    #[test]
    fn parse_timestamp_ms_naive_local() {
        // Naive "YYYY-MM-DD HH:MM:SS" parses in LOCAL time — compare against
        // the UTC epoch allowing any timezone offset (±14h max).
        let naive = parse_timestamp_ms("2001-09-09 01:46:40");
        let utc = parse_timestamp_ms("2001-09-09T01:46:40Z");
        assert!(naive != 0);
        assert!(
            (naive - utc).abs() <= 14 * 3_600_000,
            "naive={naive} utc={utc}"
        );
        assert_eq!(parse_timestamp_ms("junk"), 0);
    }

    #[test]
    fn json_value_wraps_sessions() {
        let value = json_value(&[serde_json::json!({"id": "s1"})]);
        assert_eq!(value["sessions"][0]["id"], "s1");
        assert!(json_value(&[])["sessions"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn session_no_subcommand_prints_help() {
        let (out, cap) = Output::memory();
        session(None, &[], &out).await.expect("ok");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert_eq!(stdout, format!("{}\n", SESSION_HELP));
    }

    #[tokio::test]
    async fn session_rename_usage_includes_name_placeholder() {
        let (out, cap) = Output::memory();
        let result = session(Some("rename"), &[], &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("Usage: future session rename <session-id> <name>"),
            "stderr: {stderr}"
        );
    }
}
