//! `future session status` — the *live* state of one session.
//!
//! `session info` reads the persisted journal: what the conversation contains
//! and what it has cost. This reads `get_state` instead — what the session is
//! doing *now* and how it is configured — which is the only place several
//! answers exist:
//!
//! * the effective tool **permission level** (a session can be `workspace`
//!   while the global default is `all`, so `config get` genuinely cannot
//!   answer it);
//! * **context** occupancy (`contextTokens` / `contextWindow` / percent), as
//!   opposed to `session info`'s lifetime token totals, which a reader easily
//!   mistakes for it;
//! * the context files loaded and the skills discovered for this session;
//! * the active and queued runs, and any approval the session is parked on.
//!
//! Read-only: it mutates nothing and makes no model call.

use crate::output::Output;
use crate::rpc::{grpc_addr, RunClient};
use serde_json::Value;
use std::collections::HashSet;

const HELP: &str = "future session status — one session's live state and configuration

Usage:
  future session status <session-id> [--json] [--metrics]

Reads the agent's own view of the session (get_state), so it reports what the
session would actually use, not what the global settings file says:
  Permission     the effective tool permission level (all|workspace|none)
  Context        tokens in context / the model's window / percent used
  Context files  CLAUDE.md and friends that were loaded
  Skills         how many skills were discovered for this session
  Runs           the active run, queued runs and recent terminal outcomes
  Approvals      pending approval requests, if the session is waiting on one

Options:
  --json          Print the agent's canonical state object verbatim
  --metrics       Also include runtime metrics (journal health, broadcast lag)
  -h, --help      Show this help

This is the live view; `session info` is the persisted one (messages, tool
counts, lifetime tokens and cost). No model call is made.";

#[derive(Debug, PartialEq, Eq)]
struct Options {
    session: String,
    json: bool,
    metrics: bool,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut session = None;
    let mut json = false;
    let mut metrics = false;
    let mut seen = HashSet::new();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        if flag == "--json" {
            json = true;
        } else if flag == "--metrics" {
            metrics = true;
        } else if flag.starts_with("--") {
            return Err(format!(
                "unknown option: {flag}; see future session status --help"
            ));
        } else if session.is_none() {
            session = Some(flag.to_string());
        } else {
            return Err("status takes exactly one session id".into());
        }
        if !seen.insert(flag.to_string()) && flag == "--" {
            return Err(format!("duplicate option: {flag}"));
        }
        index += 1;
    }
    Ok(Options {
        session: session.ok_or("a session id is required")?,
        json,
        metrics,
    })
}

/// `human_tokens` from the session module, which owns the shared rendering.
use super::session::human_tokens;

/// An integer field, accepting both spellings. The typed decode yields a JSON
/// number, but proto-JSON (and therefore the legacy `data` fallback an older
/// Agent still sends) encodes `int64` as a *string*, and a caller piping one
/// agent's `--json` into another is a real case — so read both rather than
/// silently reporting zero.
fn number(value: &Value, key: &str) -> i64 {
    match &value[key] {
        Value::Number(n) => n.as_i64().unwrap_or(0),
        Value::String(s) => s.parse().unwrap_or(0),
        _ => 0,
    }
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or("").to_string()
}

fn yes_no(flag: bool) -> &'static str {
    if flag {
        "yes"
    } else {
        "no"
    }
}

/// One line per pending approval: enough to decide it from a terminal, and
/// enough for `session approve <id> <request-id>` to be usable next.
fn pending_approvals(state: &Value) -> Vec<Value> {
    state["pendingApprovals"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn format_status(state: &Value, metrics: Option<&Value>) -> String {
    let mut out = String::new();
    let session_id = text(state, "sessionId");
    out.push_str(&format!("Session:  {session_id}\n"));
    let name = text(state, "sessionName");
    if !name.is_empty() {
        out.push_str(&format!("  Name:          {name}\n"));
    }
    out.push_str(&format!("  Model:         {}\n", text(state, "model")));
    out.push_str(&format!(
        "  Thinking:      {}\n",
        text(state, "thinkingLevel")
    ));
    let cwd = text(state, "cwd");
    if !cwd.is_empty() {
        out.push_str(&format!("  CWD:           {cwd}\n"));
    }
    out.push_str(&format!(
        "  Permission:    {}\n",
        text(state, "permissionLevel")
    ));
    // `sandboxTier` is absent until a policy is set, which is not the same as
    // an explicit "off" — say which one this is.
    let sandbox = state
        .get("sandboxTier")
        .and_then(Value::as_str)
        .filter(|tier| !tier.is_empty());
    out.push_str(&format!(
        "  Sandbox:       {}\n",
        sandbox.unwrap_or("(default)")
    ));
    out.push_str(&format!(
        "  Auto-compact:  {}\n",
        yes_no(state["autoCompactionEnabled"] == true)
    ));
    out.push_str(&format!(
        "  Streaming:     {}{}\n",
        yes_no(state["isStreaming"] == true),
        if state["isCompacting"] == true {
            " (compacting)"
        } else {
            ""
        }
    ));

    // The live context window, which is what "how full is this session" means.
    let tokens = number(state, "contextTokens");
    let window = number(state, "contextWindow");
    let percent = state["contextPercent"].as_f64().unwrap_or(0.0);
    if window > 0 {
        out.push_str(&format!(
            "  Context:       {} / {} tokens ({percent:.1}%)\n",
            human_tokens(tokens),
            human_tokens(window)
        ));
    } else {
        out.push_str(&format!(
            "  Context:       {} tokens\n",
            human_tokens(tokens)
        ));
    }

    let files = state["contextFiles"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    out.push_str(&format!(
        "  Context files: {}\n",
        if files.is_empty() {
            "(none)".to_string()
        } else {
            files
                .iter()
                .map(|f| f.as_str().unwrap_or("").to_string())
                .collect::<Vec<_>>()
                .join(", ")
        }
    ));
    out.push_str(&format!(
        "  Skills:        {} loaded\n",
        state["skills"].as_array().map_or(0, Vec::len)
    ));
    out.push_str(&format!(
        "  Queries:       {}\n",
        number(state, "queryCount")
    ));
    if let Some(parent) = state.get("parentSessionId").and_then(Value::as_str) {
        if !parent.is_empty() {
            out.push_str(&format!("  Parent:        {parent}\n"));
        }
    }

    // Runs: the active one, anything queued behind it, and the last outcomes.
    if let Some(active) = state.get("activeRun").filter(|v| !v.is_null()) {
        out.push_str(&format!(
            "  Active run:    {} ({})\n",
            text(active, "runId"),
            text(active, "state")
        ));
    }
    let queued = state["queuedRuns"].as_array().cloned().unwrap_or_default();
    if !queued.is_empty() {
        out.push_str(&format!("  Queued:        {}\n", queued.len()));
        for run in &queued {
            out.push_str(&format!(
                "    {} ({})\n",
                text(run, "runId"),
                text(run, "state")
            ));
        }
    }
    if let Some(interrupted) = state.get("interruptedRun").filter(|v| !v.is_null()) {
        out.push_str(&format!(
            "  Interrupted:   {} ({})\n",
            text(interrupted, "runId"),
            text(interrupted, "state")
        ));
    }

    let approvals = pending_approvals(state);
    if approvals.is_empty() {
        out.push_str("  Approvals:     none pending\n");
    } else {
        out.push_str(&format!("  Approvals:     {} pending\n", approvals.len()));
        for approval in &approvals {
            // The card's own keys: id, tool name and a one-line title.
            let id = approval["approval_request_id"]
                .as_str()
                .or_else(|| approval["approvalRequestId"].as_str())
                .unwrap_or("");
            let tool = approval["tool_name"]
                .as_str()
                .or_else(|| approval["toolName"].as_str())
                .unwrap_or("");
            let title = approval["title"].as_str().unwrap_or("");
            out.push_str(&format!("    {id}  {tool}  {title}\n"));
        }
    }

    if let Some(metrics) = metrics {
        out.push_str(&format!(
            "  Journal:       {}",
            if metrics["eventJournalHealthy"] == true {
                "healthy"
            } else {
                "degraded"
            }
        ));
        if let Some(error) = metrics
            .get("eventJournalError")
            .and_then(Value::as_str)
            .filter(|e| !e.is_empty())
        {
            out.push_str(&format!(" ({error})"));
        }
        out.push('\n');
        out.push_str(&format!(
            "  Broadcast lag: {} event(s), {} ring truncation(s), {} stale-epoch drop(s)\n",
            number(metrics, "broadcastLag"),
            number(metrics, "ringTruncations"),
            number(metrics, "staleEpochDrops")
        ));
    }
    out
}

pub(super) async fn run(args: &[String], out: &Output) -> Result<(), String> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        out.log(HELP);
        return Ok(());
    }
    let options = parse(args)?;
    let client = RunClient::new(&grpc_addr());
    let state = client
        .get_state(Some(&options.session))
        .await
        .map_err(|err| super::session::missing_session_error(err, &options.session))?;

    let metrics = if options.metrics {
        Some(client.get_runtime_metrics(&options.session).await?)
    } else {
        None
    };

    if options.json {
        let mut value = state;
        if let Some(metrics) = metrics {
            value["runtimeMetrics"] = metrics;
        }
        out.log(&serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?);
    } else {
        out.log(&format_status(&state, metrics.as_ref()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::MockAgent;
    use serde_json::json;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    /// A representative get_state payload — a session pinned to `workspace`
    /// while the global default is `all`, which is the whole point of the
    /// command.
    fn state_payload() -> Value {
        json!({
            "sessionId": "s1",
            "sessionName": "Demo",
            "model": "future/deepseek-flash",
            "thinkingLevel": "medium",
            "cwd": "/work",
            "permissionLevel": "workspace",
            "autoCompactionEnabled": true,
            "isStreaming": false,
            "isCompacting": false,
            "contextTokens": "252821",
            "contextWindow": "1000000",
            "contextPercent": 25.28,
            "contextFiles": ["CLAUDE.md"],
            "skills": ["a", "b", "c"],
            "queryCount": 4,
            "parentSessionId": "",
            "activeRun": {"runId": "run-1", "state": "running"},
            "queuedRuns": [{"runId": "run-2", "state": "queued"}],
            "pendingApprovals": [{
                "approval_request_id": "ap-1", "tool_name": "shell", "title": "Run ls"
            }],
        })
    }

    /// Point the CLI at a mock agent (caller holds the env lock).
    async fn spawn(agent: MockAgent) -> (MockAgent, crate::test_env::EnvGuard) {
        let addr = crate::test_server::spawn_mock(agent.clone()).await;
        let env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from(addr),
        )]);
        (agent, env)
    }

    #[test]
    fn options_are_strict_and_require_a_session() {
        let parsed = parse(&args(&["s1"])).unwrap();
        assert_eq!(parsed.session, "s1");
        assert!(!parsed.json && !parsed.metrics);
        assert!(
            parse(&args(&["s1", "--json", "--metrics"]))
                .unwrap()
                .metrics
        );
        assert!(parse(&args(&[])).is_err(), "a session id is required");
        assert!(parse(&args(&["s1", "s2"])).is_err(), "one id only");
        assert!(parse(&args(&["--bogus"])).is_err());
    }

    #[test]
    fn text_reports_the_effective_permission_and_context_window() {
        let rendered = format_status(&state_payload(), None);
        assert!(rendered.contains("Permission:    workspace"), "{rendered}");
        // No policy set reads as the default, not as "off".
        assert!(rendered.contains("Sandbox:       (default)"), "{rendered}");
        // Context is the live window, not the lifetime total.
        assert!(
            rendered.contains("Context:       253K / 1.0M tokens (25.3%)"),
            "{rendered}"
        );
        assert!(rendered.contains("Context files: CLAUDE.md"));
        assert!(rendered.contains("Skills:        3 loaded"));
        assert!(rendered.contains("Active run:    run-1 (running)"));
        assert!(rendered.contains("Queued:        1"));
        assert!(rendered.contains("Approvals:     1 pending"));
        assert!(rendered.contains("ap-1  shell  Run ls"), "{rendered}");
    }

    #[test]
    fn a_chosen_sandbox_tier_is_reported_as_chosen() {
        for tier in ["off", "manual", "sandbox"] {
            let mut state = state_payload();
            state["sandboxTier"] = serde_json::json!(tier);
            let rendered = format_status(&state, None);
            assert!(
                rendered.contains(&format!("Sandbox:       {tier}")),
                "{rendered}"
            );
            assert!(!rendered.contains("(default)"), "{rendered}");
        }
        // An empty string is absent, not a tier called "".
        let mut state = state_payload();
        state["sandboxTier"] = serde_json::json!("");
        assert!(format_status(&state, None).contains("Sandbox:       (default)"));
    }

    #[test]
    fn text_says_none_when_nothing_is_pending() {
        let mut state = state_payload();
        state["pendingApprovals"] = json!([]);
        state["activeRun"] = Value::Null;
        state["queuedRuns"] = json!([]);
        state["contextFiles"] = json!([]);
        let rendered = format_status(&state, None);
        assert!(rendered.contains("Approvals:     none pending"));
        assert!(rendered.contains("Context files: (none)"));
        assert!(!rendered.contains("Active run:"));
        assert!(!rendered.contains("Queued:"));
    }

    #[test]
    fn numbers_are_read_from_either_json_spelling() {
        // The typed decode sends numbers; proto-JSON (and the legacy `data`
        // fallback) sends int64 as a string. Both must read the same.
        let numeric = serde_json::json!({"contextTokens": 1000, "contextWindow": 2000});
        assert_eq!(number(&numeric, "contextTokens"), 1000);
        let textual = serde_json::json!({"contextTokens": "1000", "contextWindow": "2000"});
        assert_eq!(number(&textual, "contextTokens"), 1000);
        assert_eq!(number(&serde_json::json!({}), "missing"), 0);
        assert_eq!(number(&serde_json::json!({"x": "not a number"}), "x"), 0);
    }

    #[test]
    fn text_without_a_context_window_omits_the_ratio() {
        let mut state = state_payload();
        state["contextWindow"] = json!(0);
        let rendered = format_status(&state, None);
        assert!(
            rendered.contains("Context:       253K tokens"),
            "{rendered}"
        );
        assert!(!rendered.contains("tokens ("), "{rendered}");
    }

    #[test]
    fn metrics_block_reports_journal_health_and_lag() {
        let metrics = json!({
            "eventJournalHealthy": false,
            "eventJournalError": "disk full",
            "broadcastLag": 3,
            "ringTruncations": 1,
            "staleEpochDrops": 2,
        });
        let rendered = format_status(&state_payload(), Some(&metrics));
        assert!(
            rendered.contains("Journal:       degraded (disk full)"),
            "{rendered}"
        );
        assert!(rendered
            .contains("Broadcast lag: 3 event(s), 1 ring truncation(s), 2 stale-epoch drop(s)"));
    }

    #[tokio::test]
    async fn status_prints_the_state_and_asks_for_the_session() {
        let _guard = crate::test_env::lock_env().await;
        let agent = MockAgent::respond("get_state", &state_payload().to_string());
        let (agent, _env) = spawn(agent).await;
        let (out, cap) = Output::memory();
        run(&args(&["s1"]), &out).await.expect("status");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("Permission:    workspace"));
        assert_eq!(agent.seen_of("get_state")[0].session_id, "s1");
        // Metrics are only fetched on request.
        assert!(agent.seen_of("get_runtime_metrics").is_empty());
    }

    #[tokio::test]
    async fn status_json_is_the_canonical_state() {
        let _guard = crate::test_env::lock_env().await;
        let agent = MockAgent::respond("get_state", &state_payload().to_string());
        let (_agent, _env) = spawn(agent).await;
        let (out, cap) = Output::memory();
        run(&args(&["s1", "--json"]), &out).await.expect("status");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(parsed["permissionLevel"], "workspace");
        assert_eq!(parsed["contextPercent"], json!(25.28));
        assert!(parsed.get("runtimeMetrics").is_none());
    }

    #[tokio::test]
    async fn status_metrics_flag_merges_runtime_metrics() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = MockAgent::respond("get_state", &state_payload().to_string());
        agent.responses.insert(
            "get_runtime_metrics".into(),
            json!({"eventJournalHealthy": true, "broadcastLag": 0}).to_string(),
        );
        let (agent, _env) = spawn(agent).await;
        let (out, cap) = Output::memory();
        run(&args(&["s1", "--json", "--metrics"]), &out)
            .await
            .expect("status");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(parsed["runtimeMetrics"]["eventJournalHealthy"], true);
        assert_eq!(agent.seen_of("get_runtime_metrics").len(), 1);
    }

    #[tokio::test]
    async fn status_maps_a_missing_session_to_the_cli_wording() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = MockAgent::default();
        agent.fail_with.insert(
            "get_state".into(),
            "session not found — pass a valid session_id (new_session creates one)".into(),
        );
        let (_agent, _env) = spawn(agent).await;
        let (out, _cap) = Output::memory();
        let error = run(&args(&["ghost"]), &out).await.unwrap_err();
        assert_eq!(error, "Session not found: ghost");
    }

    #[tokio::test]
    async fn status_help_needs_no_agent() {
        let (out, cap) = Output::memory();
        run(&args(&["--help"]), &out).await.expect("help");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.starts_with("future session status —"));
    }
}
