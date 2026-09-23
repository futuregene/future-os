//! Deterministic mock `FutureAgent` gRPC server for the tmux screen-consistency
//! tests (P4). Serves fixed responses for every command the TUI issues — state,
//! models, sessions, providers/auth, sandbox, permissions, skills, usage,
//! session inspection, diagnostics and the one-shot agent actions — and streams
//! a fixed reply event sequence on `prompt` (text, one tool call with a unified
//! diff body, then the final answer), so a keystroke-driven screen comparison
//! renders byte-identical panes on every run and every host.
//!
//! Usage:
//!   cargo build -p future-tui --example mock_agent
//!   target/debug/examples/mock_agent --port 50051
//!
//! The harness (tests/tmux-diff.sh) starts one instance per TUI on different
//! ports; both instances are deterministic and identical.
//!
//! Every payload below is shaped like the real agent's answer (see the
//! per-const comments for the handler it mirrors). Anything not listed falls
//! through to an empty-but-successful `{}`, which is what the agent's
//! "no data" answers look like; a client that reads a field it needs would
//! then report "-"/"(none)" rather than failing the scenario.

use serde_json::Value;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures_util::stream;
use futures_util::Stream;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

use future_rpc::proto::future_agent_server::{FutureAgent, FutureAgentServer};
use future_rpc::proto::{RpcCommand, RpcResponse, StreamEvent, StreamRequest};

// ─── Deterministic payloads (identical on the wire for both TUIs) ───────────

/// Fixed session state (`get_state` data) — the shape of the agent's typed
/// `GetStatePayload` (`agent/src/rpc/mod.rs::get_state_internal`), camelCase.
///
/// The usage block is deliberately a *long* session: 812k cumulative input
/// tokens of which 780k are cache reads, while the current context is only
/// 48k/128k. The two figures live side by side (the footer's `↑812k` next to
/// `48.0K/128K`, and `/usage`'s token table under the context row), which is
/// what the panels have to keep distinguishable.
///
/// Only `usage.*` is read for the token totals (`tui/src/app.rs` applies
/// `s.usage.input_tokens` … to `state.tokens_*`, and `/usage` parses the same
/// object), so the top-level `tokensIn`/`totalCost` keys the mock used to carry
/// were dead weight and are gone.
const STATE_JSON: &str = r#"{
  "agentInstanceId": "mock-agent-instance",
  "model": "mock-model",
  "thinkingLevel": "off",
  "isStreaming": false,
  "isCompacting": false,
  "sessionFile": "/mock/session.jsonl",
  "sessionId": "mock-session-1",
  "sessionName": "Mock Session One",
  "explicitSession": false,
  "autoCompactionEnabled": false,
  "queryCount": 42,
  "version": "0.0.0-mock",
  "cwd": "/mock",
  "permissionLevel": "all",
  "skills": ["code-review", "future-slides", "future-web"],
  "contextFiles": [],
  "extensions": [],
  "imageSupport": true,
  "contextTokens": 48000,
  "contextWindow": 128000,
  "contextPercent": 37.5,
  "usage": {
    "inputTokens": 812000,
    "outputTokens": 12000,
    "cacheReadTokens": 780000,
    "cacheWriteTokens": 12000,
    "costCny": 8.42,
    "costInputCny": 1.2,
    "costOutputCny": 3.6,
    "costCacheReadCny": 3.12,
    "costCacheWriteCny": 0.5
  },
  "activeRun": null,
  "queuedRuns": []
}"#;

/// Fixed model list (`list_models` / `get_available_models` data).
const MODELS_JSON: &str = r#"{
  "models": [
    {"id": "mock-model", "label": "Mock Model", "provider": "future", "supportsImages": true, "thinkingLevel": "off", "contextWindow": 128000, "isDefault": true},
    {"id": "mock-pro", "label": "Mock Pro", "provider": "future", "supportsImages": true, "thinkingLevel": "high", "contextWindow": 256000, "isDefault": false},
    {"id": "mock-fast", "label": "Mock Fast", "provider": "future", "supportsImages": false, "thinkingLevel": "off", "contextWindow": 64000, "isDefault": false}
  ]
}"#;

/// Fixed session list (`list_sessions` data) for the /sessions overlay.
/// Matches the real agent's list_sessions summaries (`id/cwd/model/updatedAt`
/// — no session_name), so both TUIs parse identical wire data.
const SESSIONS_JSON: &str = r#"{
  "sessions": [
    {"id": "mock-session-1", "cwd": "/mock", "model": "mock-model", "updatedAt": "2026-08-07T00:00:00Z"},
    {"id": "mock-session-2", "cwd": "/mock", "model": "mock-model", "updatedAt": "2026-08-06T00:00:00Z"}
  ]
}"#;

/// `list_providers` data — the agent's provider view
/// (`agent/src/rpc/commands/providers.rs::provider_view`): `{builtin, custom}`,
/// built-ins first. Two built-ins (one with a stored key, one without) so the
/// Built-in tab shows both states, plus one custom provider whose model list
/// carries contextWindow/maxTokens/reasoning/costs and, for the first model,
/// the `modalities` array an older payload carries instead of `supportsImages`.
const PROVIDERS_JSON: &str = r#"{
  "builtin": [
    {"id": "future", "name": "Future", "baseUrl": "https://api.future-os.dev/v1", "hasApiKey": true, "modelCount": 900, "requiresBaseUrl": false},
    {"id": "openai", "name": "OpenAI", "baseUrl": "https://api.openai.com/v1", "hasApiKey": false, "modelCount": 12, "requiresBaseUrl": false}
  ],
  "custom": [
    {
      "id": "acme",
      "name": "Acme Cloud",
      "api": "openai-completions",
      "baseUrl": "https://api.acme.example/v1",
      "hasApiKey": true,
      "modelCount": 2,
      "models": [
        {"id": "acme-large", "name": "Acme Large", "modalities": ["text", "image"], "contextWindow": 200000, "maxTokens": 8192, "reasoning": true, "inputCost": 1.5, "outputCost": 6.0, "cacheReadCost": 0.15, "cacheWriteCost": 0.5},
        {"id": "acme-fast", "name": "Acme Fast", "supportsImages": false, "contextWindow": 128000, "maxTokens": 16384, "reasoning": false, "inputCost": 0.2, "outputCost": 0.8, "cacheReadCost": 0.02, "cacheWriteCost": 0.05}
      ]
    }
  ]
}"#;

/// `upsert_provider` data (`providers.rs::cmd_upsert_provider`).
const UPSERT_PROVIDER_JSON: &str = r#"{"id": "acme", "revision": 7}"#;
/// `delete_provider` data (`providers.rs::cmd_delete_provider`).
const DELETE_PROVIDER_JSON: &str = r#"{"id": "acme", "revision": 8}"#;
/// `set_auth` data (`providers.rs::cmd_set_auth`).
const SET_AUTH_JSON: &str = r#"{"provider": "openai", "revision": 9}"#;
/// `reload_auth` data (`providers.rs::handle_reload_auth`).
const RELOAD_AUTH_JSON: &str = r#"{"revision": 10}"#;
/// `sync_future_models` data (`providers.rs::handle_sync_future_models`).
const SYNC_FUTURE_MODELS_JSON: &str = r#"{"synced": true, "modelCount": 900, "revision": 11}"#;

/// `probe_sandbox` data — the macOS product probe
/// (`agent/src/sandbox/mod.rs::platform_sandbox_probe_product`), i.e. a usable
/// backend so `/sandbox` renders its normal state. A fixed answer (rather than
/// the host's own probe) is what keeps the pane identical on every host.
const PROBE_SANDBOX_JSON: &str = r#"{
  "available": true,
  "code": "available",
  "backend": "macos_seatbelt",
  "path": "/usr/bin/sandbox-exec"
}"#;

/// `get_commands` data (`session_lifecycle.rs::cmd_get_commands`) — the
/// discovered skills, sorted by name, with the Chinese fields `/skills` shows
/// when a row has them. Same set as `get_state.skills`.
const COMMANDS_JSON: &str = r#"{
  "commands": [
    {"name": "code-review", "description": "Review a diff for correctness", "nameZh": "代码审查", "descriptionZh": "审查代码差异的正确性", "source": "skill"},
    {"name": "future-slides", "description": "Build a slide deck from an outline", "nameZh": "幻灯片", "descriptionZh": "把大纲制作成幻灯片", "source": "skill"},
    {"name": "future-web", "description": "Search and fetch web pages", "nameZh": "网页检索", "descriptionZh": "搜索并抓取网页", "source": "skill"}
  ]
}"#;

/// `refresh_skills` data (`providers.rs::cmd_refresh_skills`) — snake_case on
/// the wire, unlike the rest.
const REFRESH_SKILLS_JSON: &str = r#"{
  "skills_count": 3,
  "skills": ["code-review", "future-slides", "future-web"],
  "refreshed": true
}"#;

/// `get_agent_info` data (`providers.rs::get_agent_info_response`).
const AGENT_INFO_JSON: &str = r#"{
  "version": "0.0.0-mock",
  "agentInstanceId": "mock-agent-instance",
  "skillsCount": 3
}"#;

/// `get_session_stats` data (`session.rs::get_session_stats`) — counters plus
/// the same cumulative token/cost totals `get_state.usage` reports.
const SESSION_STATS_JSON: &str = r#"{
  "sessionFile": "/mock/session.jsonl",
  "sessionId": "mock-session-1",
  "userMessages": 42,
  "assistantMessages": 40,
  "toolCalls": 57,
  "toolResults": 57,
  "totalMessages": 139,
  "tokens": {
    "input": 812000,
    "output": 12000,
    "cacheRead": 780000,
    "cacheWrite": 12000,
    "total": 824000
  },
  "cost": 8.42
}"#;

/// `list_tool_calls` data (`agent/src/session/tools.rs::tool_page`).
const TOOL_CALLS_JSON: &str = r#"{
  "tools": [
    {"toolCallId": "call_mock_1", "runId": "run_mock_1", "name": "edit", "arguments": {"path": "src/greeting.rs"}, "status": "completed", "startedAtMs": 1750000000500, "completedAtMs": 1750000000900},
    {"toolCallId": "call_mock_2", "runId": "run_mock_1", "name": "shell", "arguments": {"command": "cargo test -p future-tui"}, "status": "failed", "startedAtMs": 1750000001000, "completedAtMs": 1750000002400}
  ],
  "hasMore": false,
  "nextOffset": 2
}"#;

/// `generate_session_title` data (`session_title.rs::handle`).
const TITLE_JSON: &str = r#"{"title": "Mock session title", "model": "mock-model"}"#;
/// `delete_session` data (`session_lifecycle.rs::cmd_delete_session`).
const DELETE_SESSION_JSON: &str = r#"{"deleted": true}"#;
/// `shutdown` data (`session_lifecycle.rs::cmd_shutdown`).
const SHUTDOWN_JSON: &str =
    r#"{"shutting_down": true, "note": "Existing runs continue; new prompts are rejected."}"#;

/// `get_runtime_metrics` data (`session.rs::get_runtime_metrics`) — an idle
/// session with a couple of non-zero diagnostics, so `/metrics` prints a table
/// rather than a wall of zeros.
const METRICS_JSON: &str = r#"{
  "sessionId": "mock-session-1",
  "activeRunGauge": 0,
  "staleEpochDrops": 2,
  "persistenceDegraded": 0,
  "broadcastLag": 1,
  "ringTruncations": 0,
  "activeRunId": null,
  "queuedRuns": 0,
  "queuedBytes": 0,
  "eventJournalHealthy": true,
  "eventJournalError": null
}"#;

/// `get_run_snapshot` data (`observability.rs::handle_get_run_snapshot`) — the
/// envelope plus a projection whose events `/snapshot` lists as `#idx type`.
const RUN_SNAPSHOT_JSON: &str = r#"{
  "runSnapshot": true,
  "events": [],
  "watermark": 6,
  "nextSinceIdx": 6,
  "hasMore": false,
  "projection": {
    "runId": "run_mock_1",
    "cursor": 6,
    "events": [
      {"type": "user_message", "data": "{\"text\":\"hello\"}", "runId": "run_mock_1", "idx": 0, "sessionId": "mock-session-1", "epoch": 1, "eventId": "evt_mock_0", "timestamp": "2026-08-07T00:00:00.000Z"},
      {"type": "agent_start", "data": "{}", "runId": "run_mock_1", "idx": 1, "sessionId": "mock-session-1", "epoch": 1, "eventId": "evt_mock_1", "timestamp": "2026-08-07T00:00:00.000Z"},
      {"type": "text_chunk", "data": "{\"text\":\"Hello from the mock agent!\\n\\n\",\"delta\":true}", "runId": "run_mock_1", "idx": 2, "sessionId": "mock-session-1", "epoch": 1, "eventId": "evt_mock_2", "timestamp": "2026-08-07T00:00:00.000Z"},
      {"type": "tool_start", "data": "{\"tool_id\":\"call_mock_1\",\"tool_name\":\"edit\"}", "runId": "run_mock_1", "idx": 3, "sessionId": "mock-session-1", "epoch": 1, "eventId": "evt_mock_3", "timestamp": "2026-08-07T00:00:00.000Z"},
      {"type": "tool_end", "data": "{\"tool_id\":\"call_mock_1\"}", "runId": "run_mock_1", "idx": 4, "sessionId": "mock-session-1", "epoch": 1, "eventId": "evt_mock_4", "timestamp": "2026-08-07T00:00:00.000Z"},
      {"type": "agent_end", "data": "{\"state\":\"completed\"}", "runId": "run_mock_1", "idx": 5, "sessionId": "mock-session-1", "epoch": 1, "eventId": "evt_mock_5", "timestamp": "2026-08-07T00:00:00.000Z"}
    ]
  }
}"#;

/// The stored output of `call_mock_1`: a unified diff, so `/tool-output
/// call_mock_1` and the transcript's tool body both exercise the diff renderer
/// (`tui/src/components/chat_area.rs::looks_like_diff` needs a hunk header or a
/// `---`/`+++` pair).
const TOOL_DIFF_TEXT: &str = "--- a/src/greeting.rs\n+++ b/src/greeting.rs\n@@ -1,4 +1,5 @@\n pub fn greeting() -> &'static str {\n-    \"Hello, world!\"\n+    \"Hello from the mock agent!\"\n }\n+\n";

/// The stored output of `call_mock_2` (the failed call), plain text.
const TOOL_FAILURE_TEXT: &str = "running 2 tests\ntest greeting_is_deterministic ... ok\ntest greeting_matches_snapshot ... FAILED\n\nerror: test failed, to rerun pass `-p future-tui --lib`\n";

/// Where `/export` writes, mirroring the agent's own export directory
/// (`observability.rs::export_output_path` — `/tmp` plus a
/// `future_agent_export_<session>_<timestamp>.html` name). The path is fixed
/// rather than timestamped so the panel's message is byte-stable; the mock
/// really writes the file, so the client's size readback shows a real length.
const EXPORT_PATH: &str = "/tmp/future_agent_export_mock-session-1_20260807000000.html";
/// The exported document's fixed body (the TUI never reads it back).
const EXPORT_HTML: &str =
    "<!doctype html>\n<html><body><h1>Mock session export</h1></body></html>\n";

/// `prompt` RunAck (snake_case per the `RunAck` wire contract).
const PROMPT_ACK_JSON: &str = r#"{
  "run_id": "run_mock_1",
  "run_epoch": 1,
  "accepted_state": "running",
  "run_sequence": 1,
  "queue_position": 0
}"#;

const RUN_ID: &str = "run_mock_1";
const SESSION_ID: &str = "mock-session-1";
/// A prompt starting with this asks the mock for a *burst*: three completed
/// `read` calls with nothing in between — the shape the compact view folds
/// (`ctrl+d`). Any other prompt gets the fixed single-call reply.
const BURST_PROMPT_PREFIX: &str = "scan the workspace";
/// The files the burst reads, one call each.
const BURST_PATHS: [&str; 3] = ["src/main.rs", "src/lib.rs", "src/cli.rs"];
/// The session `get_session_entries` has planted history for: the second entry
/// of `list_sessions`, which the paging scenario switches to. The harness's own
/// session is deliberately empty, so every other golden is untouched by the
/// TUI's move to the paged history read.
const HISTORY_SESSION: &str = "mock-session-2";
/// User exchanges the planted history holds: more than one page of ten (the
/// page size `tui/src/rpc/grpc_client.rs` asks for), so a switch loads a tail
/// that cannot contain the oldest row.
const HISTORY_EXCHANGES: usize = 14;
/// The tool call the streamed reply makes, and the id `/tool-output <id>`
/// addresses.
const TOOL_CALL_ID: &str = "call_mock_1";

/// Fixed assistant reply (markdown exercises the renderer on both sides).
const REPLY_TEXT: &str =
    "Hello from the mock agent!\n\nThis is a **deterministic** reply with `code` and a [link](https://example.com).\n";

/// The burst prompt's reply: the same text its `text_chunk`s spell out (the TUI
/// writes `agent_end`'s `text` over the last assistant message).
const BURST_REPLY_TEXT: &str = "Scanning the workspace.\n\nEvery file uses the same header.\n";

#[derive(Clone)]
struct MockAgent {
    /// Active event subscribers (one channel per stream_events connection).
    subs: Arc<std::sync::Mutex<Vec<mpsc::UnboundedSender<StreamEvent>>>>,
    /// Delay between streamed prompt events (ms). 0 (default) keeps the
    /// golden-screen timing deterministic; used by the tmux attach repro to
    /// keep the reply streaming while a client detaches/reattaches.
    stream_delay_ms: u64,
}

impl MockAgent {
    fn ok(&self, cmd: &RpcCommand, data: &str) -> RpcResponse {
        RpcResponse {
            id: cmd.id.clone(),
            r#type: "response".into(),
            command: cmd.r#type.clone(),
            success: true,
            data: data.into(),
            error: String::new(),
            error_code: String::new(),
            error_data: String::new(),
            payload: None,
        }
    }

    /// `shell` data —
    /// `session.rs::execute_shell_at` returns `{output, exitCode}`; the mock
    /// echoes the command instead of running it (no host side effects, and the
    /// pane stays identical on every machine).
    fn shell_response(&self, command: &str) -> String {
        serde_json::json!({
            "output": format!("{command}\n"),
            "exitCode": 0,
        })
        .to_string()
    }

    /// `export_html` data — the agent answers `{path}` and the TUI reads the
    /// file's size back from disk (`tui/src/app.rs::export_result_message`), so
    /// the document is written for real.
    fn export_html_response(&self) -> String {
        // A failed write only costs the size in the message; the command still
        // succeeds so a read-only /tmp cannot fail the scenario.
        let _ = std::fs::write(EXPORT_PATH, EXPORT_HTML);
        serde_json::json!({ "path": EXPORT_PATH }).to_string()
    }

    /// `get_session_entries` data — the display-history pager.
    ///
    /// Only [`HISTORY_SESSION`] has history: the harness's own session stays
    /// empty, so its screens (and goldens) are exactly what they were before the
    /// TUI read history through this pager instead of `get_messages`. The
    /// planted session is longer than one page, and the cursor arithmetic is the
    /// agent's (`agent/src/session/history_index.rs::read_page`): `before` is an
    /// exclusive backward cursor, `limit` counts *user exchanges*, and the page
    /// starts at the `limit`-th user row above `before` (or at zero).
    fn session_entries_response(&self, cmd: &RpcCommand) -> String {
        if cmd.session_id != HISTORY_SESSION {
            return r#"{"entries":[]}"#.to_string();
        }
        let rows: Vec<Value> = (1..=HISTORY_EXCHANGES)
            .flat_map(|i| {
                [
                    ("user", format!("planted-{i:02}")),
                    ("assistant", format!("answer-{i:02}")),
                ]
            })
            .enumerate()
            .map(|(ordinal, (role, text))| {
                serde_json::json!({
                    "id": format!("planted-{ordinal}"),
                    "kind": role,
                    "role": role,
                    "createdAtMs": 1_754_000_000_000_i64 + ordinal as i64,
                    "blocks": [{"kind": "text", "text": text}],
                })
            })
            .collect();
        let end = cmd.before.unwrap_or(i64::MAX).clamp(0, rows.len() as i64) as usize;
        let count = cmd.limit.unwrap_or(10).clamp(1, 100) as usize;
        // The `count`-th user row above `before` (or the start of the history).
        let start = rows[..end]
            .iter()
            .enumerate()
            .filter(|(_, row)| row["role"] == "user")
            .map(|(ordinal, _)| ordinal)
            .rev()
            .nth(count - 1)
            .unwrap_or(0);
        serde_json::json!({
            "entries": &rows[start..end],
            "hasMore": start > 0,
            "nextOffset": start as i64,
        })
        .to_string()
    }

    /// `get_tool_output` data (`tools.rs::tool_output`): the stored result of
    /// one call, or `{"output": null}` for a call this mock has no result for.
    fn tool_output_response(&self, tool_call_id: &str) -> String {
        let stored = match tool_call_id {
            TOOL_CALL_ID => Some((TOOL_DIFF_TEXT, false)),
            "call_mock_2" => Some((TOOL_FAILURE_TEXT, true)),
            _ => None,
        };
        match stored {
            Some((text, is_error)) => serde_json::json!({
                "output": {
                    "toolCallId": tool_call_id,
                    "runId": RUN_ID,
                    "text": text,
                    "isError": is_error,
                    "createdAtMs": 1_750_000_000_900i64,
                }
            })
            .to_string(),
            None => r#"{"output": null}"#.to_string(),
        }
    }

    /// `search_session_history` data (`history_query.rs::search_history`) —
    /// fixed matches over the streamed reply (user text, the tool call, its
    /// diff result), with the caller's query echoed back in `query`.
    fn history_response(&self, query: &str) -> String {
        serde_json::json!({
            "sessionId": SESSION_ID,
            "query": query,
            "matches": [
                {
                    "entryId": "entry_mock_user_1",
                    "entryPosition": 0,
                    "role": "user",
                    "runId": RUN_ID,
                    "timestampMs": 1_750_000_000_000i64,
                    "blockIndex": 0,
                    "kind": "text",
                    "snippet": "hello",
                    "byteOffset": 0,
                },
                {
                    "entryId": "entry_mock_assistant_1",
                    "entryPosition": 1,
                    "role": "assistant",
                    "runId": RUN_ID,
                    "timestampMs": 1_750_000_000_100i64,
                    "blockIndex": 1,
                    "kind": "tool_call",
                    "toolCallId": TOOL_CALL_ID,
                    "toolName": "edit",
                    "snippet": "{\"path\":\"src/greeting.rs\"}",
                    "byteOffset": 0,
                },
                {
                    "entryId": "entry_mock_tool_1",
                    "entryPosition": 2,
                    "role": "tool",
                    "runId": RUN_ID,
                    "timestampMs": 1_750_000_000_200i64,
                    "blockIndex": 0,
                    "kind": "tool_result",
                    "toolCallId": TOOL_CALL_ID,
                    "toolName": "edit",
                    "snippet": "@@ -1,4 +1,5 @@ pub fn greeting()",
                    "byteOffset": 0,
                }
            ],
            "hasMore": false,
        })
        .to_string()
    }

    /// `set_sandbox_policy` data (`settings.rs::handle_set_sandbox_policy`) —
    /// the tier the agent applied, the tier that was asked for, and the probe
    /// it decided with. The mock always has a usable backend, so a `sandbox`
    /// request is honoured (`fallback` stays null) and the tier is echoed
    /// instead of hard-coded: the panel is driven by whatever was picked.
    fn sandbox_policy_response(&self, tier: &str) -> String {
        let tier = if tier.is_empty() { "manual" } else { tier };
        serde_json::json!({
            "tier": tier,
            "requestedTier": tier,
            "sandboxAvailable": true,
            "sandboxCode": "available",
            "sandboxBackend": "macos_seatbelt",
            "fallback": serde_json::Value::Null,
        })
        .to_string()
    }

    /// Broadcast the fixed reply event sequence after the prompt ack has been
    /// delivered (small delay so the ack is processed first on both sides).
    fn schedule_prompt_events(&self, prompt_text: &str) {
        let subs = self.subs.clone();
        let prompt_text = prompt_text.to_string();
        let stream_delay_ms = self.stream_delay_ms;
        // A prompt that asks for a scan grows the run with a *burst* of calls to
        // one tool: that is what the compact view (`ctrl+d`) folds, and a burst
        // cannot be produced by the fixed reply below.
        let burst = prompt_text.starts_with(BURST_PROMPT_PREFIX);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let mut events: Vec<(String, String)> = vec![
                (
                    "user_message".to_string(),
                    format!(
                        r#"{{"text":{},"sessionId":"{SESSION_ID}"}}"#,
                        serde_json::to_string(&prompt_text).unwrap()
                    ),
                ),
                (
                    "agent_start".to_string(),
                    format!(r#"{{"started_at_ms":1750000000000,"run_id":"{RUN_ID}"}}"#),
                ),
            ];
            if burst {
                events.push((
                    "text_chunk".to_string(),
                    r#"{"text":"Scanning the workspace.\n\n","delta":true}"#.to_string(),
                ));
                for (index, path) in BURST_PATHS.iter().enumerate() {
                    let id = format!("{TOOL_CALL_ID}-{index}");
                    events.push((
                        "tool_start".to_string(),
                        format!(
                            r#"{{"tool_id":"{id}","tool_name":"read","tool_args":{{"path":"{path}"}}}}"#
                        ),
                    ));
                    events.push((
                        "tool_end".to_string(),
                        format!(r#"{{"tool_id":"{id}","text":"read {path}\n"}}"#),
                    ));
                }
                events.push((
                    "text_chunk".to_string(),
                    r#"{"text":"Every file uses the same header.\n","delta":true}"#.to_string(),
                ));
            } else {
                events.push((
                    "text_chunk".to_string(),
                    r#"{"text":"Hello from the mock agent!\n\n","delta":true}"#.to_string(),
                ));
                events.push((
                    "tool_start".to_string(),
                    format!(
                        r#"{{"tool_id":"{TOOL_CALL_ID}","tool_name":"edit","tool_args":{{"path":"src/greeting.rs"}}}}"#
                    ),
                ));
                events.push((
                    "tool_end".to_string(),
                    format!(
                        r#"{{"tool_id":"{TOOL_CALL_ID}","text":{}}}"#,
                        serde_json::to_string(TOOL_DIFF_TEXT).unwrap()
                    ),
                ));
                events.push((
                    "text_chunk".to_string(),
                    r#"{"text":"This is a **deterministic** reply with `code` and a [link](https://example.com).\n","delta":true}"#
                        .to_string(),
                ));
            }
            events.push((
                "agent_end".to_string(),
                // `state` is what the real agent puts on this event
                // (`session_prompt.rs`: `"state": terminal_state`, from
                // `run_journal::RUN_STATE_*`), and the TUI gates the desktop
                // notification on it. Without it the mock could never
                // exercise that path at all — the harness would show a
                // silent terminal for a feature whose whole job is to make
                // noise when the user is looking elsewhere.
                //
                // `text` is the run's final answer, which the TUI writes over
                // the last assistant message — so it has to be the same text
                // the chunks above spelled out, exactly as the agent's own
                // `agent_end` carries the assembled reply.
                format!(
                    r#"{{"type":"agent_end","state":"completed","run_id":"{RUN_ID}","duration_ms":500,"usage":{{"prompt_tokens":10,"completion_tokens":20}},"error":null,"text":{}}}"#,
                    serde_json::to_string(if burst { BURST_REPLY_TEXT } else { REPLY_TEXT }).unwrap()
                ),
            ));
            let senders: Vec<mpsc::UnboundedSender<StreamEvent>> = subs.lock().unwrap().clone();
            for (idx, (ty, data)) in events.iter().enumerate() {
                if idx > 0 && stream_delay_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(stream_delay_ms)).await;
                }
                let event = StreamEvent {
                    r#type: ty.clone(),
                    data: data.clone(),
                    run_id: RUN_ID.into(),
                    idx: idx as i64,
                    session_id: SESSION_ID.into(),
                    epoch: 1,
                    event_id: format!("evt_mock_{idx}"),
                    timestamp: "2026-08-07T00:00:00.000Z".into(),
                    ..Default::default()
                };
                for tx in &senders {
                    let _ = tx.send(event.clone());
                }
            }
        });
    }
}

#[tonic::async_trait]
impl FutureAgent for MockAgent {
    async fn execute_command(
        &self,
        request: Request<RpcCommand>,
    ) -> Result<Response<RpcResponse>, Status> {
        let cmd = request.into_inner();
        let data = match cmd.r#type.as_str() {
            // ── State, models, sessions (P4 baseline) ────────────────────
            "get_state" => STATE_JSON.to_string(),
            "list_models" | "get_available_models" => MODELS_JSON.to_string(),
            "new_session" => r#"{"sessionId":"mock-session-1"}"#.to_string(),
            "reload_config" => {
                r#"{"skills":["code-review","future-slides","future-web"],"contextFiles":[]}"#
                    .to_string()
            }
            "get_messages" => r#"{"messages":[]}"#.to_string(),
            "get_session_entries" => self.session_entries_response(&cmd),
            "list_sessions" => SESSIONS_JSON.to_string(),
            "prompt" => {
                self.schedule_prompt_events(&cmd.message);
                PROMPT_ACK_JSON.to_string()
            }
            // ── Providers / auth ─────────────────────────────────────────
            "list_providers" => PROVIDERS_JSON.to_string(),
            "upsert_provider" => UPSERT_PROVIDER_JSON.to_string(),
            "delete_provider" => DELETE_PROVIDER_JSON.to_string(),
            "set_auth" => SET_AUTH_JSON.to_string(),
            "reload_auth" => RELOAD_AUTH_JSON.to_string(),
            "sync_future_models" => SYNC_FUTURE_MODELS_JSON.to_string(),
            // ── Sandbox / permissions ───────────────────────────────────
            "probe_sandbox" => PROBE_SANDBOX_JSON.to_string(),
            "set_sandbox_policy" => self.sandbox_policy_response(
                cmd.sandbox_policy
                    .as_ref()
                    .map(|policy| policy.tier.as_str())
                    .unwrap_or_default(),
            ),
            "set_permission_level" => {
                let level = if cmd.level.is_empty() {
                    "all"
                } else {
                    cmd.level.as_str()
                };
                serde_json::json!({ "permissionLevel": level }).to_string()
            }
            // ── Skills ──────────────────────────────────────────────────
            "get_commands" => COMMANDS_JSON.to_string(),
            "refresh_skills" => REFRESH_SKILLS_JSON.to_string(),
            // ── Session inspection ──────────────────────────────────────
            "get_agent_info" => AGENT_INFO_JSON.to_string(),
            "get_session_stats" => SESSION_STATS_JSON.to_string(),
            "list_tool_calls" => TOOL_CALLS_JSON.to_string(),
            "get_tool_output" => {
                self.tool_output_response(cmd.tool_call_id.as_deref().unwrap_or(""))
            }
            "search_session_history" => self.history_response(&cmd.message),
            "export_html" => self.export_html_response(),
            "generate_session_title" => TITLE_JSON.to_string(),
            "delete_session" => DELETE_SESSION_JSON.to_string(),
            "get_runtime_metrics" => METRICS_JSON.to_string(),
            "get_run_snapshot" => RUN_SNAPSHOT_JSON.to_string(),
            "shell" => self.shell_response(&cmd.command),
            // `shutdown` is deliberately a no-op: it reports the agent's
            // contract (`{shutting_down, note}`) but keeps serving, so the
            // scenario can drive `/quit-agent --yes` without killing the
            // harness's agent (and with it every later scenario).
            "shutdown" => SHUTDOWN_JSON.to_string(),
            // ── Session settings (success, mirroring the agent's data) ──
            "set_tools" => serde_json::json!({ "tools": cmd.tools }).to_string(),
            "set_ephemeral" => serde_json::json!({ "ephemeral": cmd.ephemeral }).to_string(),
            "set_auto_compaction"
            | "set_auto_retry"
            | "set_context_files"
            | "set_system_prompt"
            | "disable_tools"
            | "append_system_prompt"
            | "add_session_rule"
            | "set_session_name" => "{}".to_string(),
            _ => "{}".to_string(),
        };
        Ok(Response::new(self.ok(&cmd, &data)))
    }

    type StreamEventsStream = Pin<Box<dyn Stream<Item = Result<StreamEvent, Status>> + Send>>;

    async fn stream_events(
        &self,
        _request: Request<StreamRequest>,
    ) -> Result<Response<Self::StreamEventsStream>, Status> {
        let (tx, rx) = mpsc::unbounded_channel::<StreamEvent>();
        self.subs.lock().unwrap().push(tx);
        // Push a first "ping" so the client's connected edge fires promptly
        // (the P3 client clears its 5 s first-data watchdog on first data).
        let first = StreamEvent {
            r#type: "ping".into(),
            session_id: SESSION_ID.into(),
            ..Default::default()
        };
        let stream =
            stream::once(async move { Ok(first) }).chain(UnboundedReceiverStream::new(rx).map(Ok));
        Ok(Response::new(Box::pin(stream)))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut port = 50051u16;
    let mut stream_delay_ms = 0u64;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                if let Some(v) = args.next() {
                    port = v.parse().unwrap_or(50051);
                }
            }
            "--stream-delay-ms" => {
                if let Some(v) = args.next() {
                    stream_delay_ms = v.parse().unwrap_or(0);
                }
            }
            "--help" | "-h" => {
                println!("usage: mock_agent --port <port> [--stream-delay-ms <ms>]");
                return Ok(());
            }
            other => {
                eprintln!("unknown arg: {other}");
                std::process::exit(2);
            }
        }
    }

    let addr = format!("127.0.0.1:{port}");
    let agent = MockAgent {
        subs: Arc::new(std::sync::Mutex::new(Vec::new())),
        stream_delay_ms,
    };
    println!("mock agent listening on {addr}");
    Server::builder()
        .add_service(FutureAgentServer::new(agent))
        .serve(addr.parse()?)
        .await?;
    Ok(())
}
