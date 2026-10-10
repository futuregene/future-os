//! `future session transcript` — a filtered, windowed view of one session.
//!
//! `session info` gives a session's shape, and `session history search|get`
//! finds one passage; this command answers "show me what is *in* this session",
//! through a lens the caller chooses: which roles and block kinds, which tool,
//! a tool's input vs its output vs just the file paths either mentions, a
//! literal content filter, and a window that can be paged with an opaque cursor.
//! It exists so a personal agent can process a conversation without a model
//! call and without loading everything into context.
//!
//! It is read-only: no model call, no tool execution, no session mutation. It
//! reads the same display projection `session info` already uses
//! (`get_session_entries`, honouring its forward-page cursor), so `reasoning`
//! blocks are visible here even though `session history search` excludes them
//! by design. Media blocks are not emitted, matching history recall.

use super::session::human_tokens;
use crate::output::Output;
use crate::rpc::{grpc_addr, RunClient};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

const HELP: &str = "future session transcript — filter and window one session's records

Usage:
  future session transcript --session <id> [options]

Content selection (default: user,assistant,tool-call,tool-result; add thinking
to include reasoning, which dominates the byte count of a real session):
  --select <list>   Comma-separated: user, assistant, thinking, tool-call,
                    tool-result, session, compaction, all
  --tool <list>     Only these tools' calls/results (comma-separated names)
  --input           Tool calls: arguments only (drop name/toolCallId)
  --output          Tool results: result text only (drop toolCallId/isError)
  --paths           Tool calls/results: extract file paths only (add
                    --select tool-call,tool-result to get paths and nothing else)
  --grep <text>     Keep only blocks whose content contains this text
                    (case-insensitive; matches text, thinking, tool name/args)

Window (entries, in display order):
  --cursor <n>      Start at this ordinal (default 0; from a prior nextCursor)
  --limit <n>       Stop after n matching entries (1..500, default 50)
  --all             Emit every match (bounded by --max-bytes)
  --max-bytes <n>   Stop before the output passes n bytes (default 262144; 0 =
                    off); with --counts it bounds the records tallied

Shape:
  --truncate <n>    Truncate every emitted string longer than n characters
  --counts          Print a distribution summary of the window instead of entries
  --runs            Print one row per run: status, duration, tokens, error
  --json            Machine-readable output
  -h, --help        Show this help

Entries are matching *display* entries; a window empty after filtering is normal.
Every emitted entry also carries its run outcome (`run`, `runId`) and, where the
run recorded one, its token `usage`.

--counts and --runs are summaries: they ignore --select/--tool/--grep, always
start at the beginning of the session, and cover the whole session unless
--limit or --all is given (--runs counts distinct runs, not entries).
--tool filters tool results by the name of the call they pair with; a result
whose call is outside the scanned window cannot be attributed and is skipped
(the summary reports how many).
Reads original records through the Agent; no model call is made.";

/// Entries requested per RPC page. Well inside the Agent's 1..=1000 clamp and
/// its 8 MiB per-page materialization budget.
const RPC_PAGE: i64 = 200;
const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 500;
const DEFAULT_MAX_BYTES: usize = 262_144;
const MAX_TRUNCATE: usize = 1_000_000;
/// Stop collecting paths per block; a pathological result cannot balloon.
const MAX_PATHS_PER_BLOCK: usize = 100;

/// One selectable slice of the transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Sel {
    Session,
    Compaction,
    User,
    Assistant,
    Thinking,
    ToolCall,
    ToolResult,
}

impl Sel {
    /// Accepts `tool-call`, `tool_call` and `toolcall` spellings.
    fn parse(raw: &str) -> Option<Sel> {
        let key: String = raw
            .trim()
            .chars()
            .filter(|c| *c != '_' && *c != '-')
            .flat_map(|c| c.to_lowercase())
            .collect();
        match key.as_str() {
            "session" | "sessioninfo" => Some(Sel::Session),
            "compaction" | "checkpoint" => Some(Sel::Compaction),
            "user" => Some(Sel::User),
            "assistant" => Some(Sel::Assistant),
            "thinking" | "reasoning" => Some(Sel::Thinking),
            "toolcall" | "tools" | "call" => Some(Sel::ToolCall),
            "toolresult" | "result" | "results" => Some(Sel::ToolResult),
            _ => None,
        }
    }
}

/// The conversation by default; `thinking` is opt-in because reasoning blocks
/// dominate the byte count of a real session.
fn default_select() -> Vec<Sel> {
    vec![Sel::User, Sel::Assistant, Sel::ToolCall, Sel::ToolResult]
}

#[derive(Debug, PartialEq, Eq)]
struct Options {
    session: String,
    select: Vec<Sel>,
    tools: Vec<String>,
    input_only: bool,
    output_only: bool,
    paths_only: bool,
    grep: Option<String>,
    cursor: i64,
    limit: i64,
    limit_given: bool,
    all: bool,
    max_bytes: usize,
    /// Whether `--max-bytes` was the caller's own choice. The summaries cover
    /// the whole session unless they asked for a bound, so that a distribution
    /// is never quietly partial.
    max_bytes_given: bool,
    truncate: Option<usize>,
    counts: bool,
    runs: bool,
    json: bool,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut session = None;
    let mut select: Option<Vec<Sel>> = None;
    let mut tools: Vec<String> = Vec::new();
    let (mut input_only, mut output_only, mut paths_only) = (false, false, false);
    let mut grep = None;
    let mut cursor = 0i64;
    let mut limit = DEFAULT_LIMIT;
    let mut limit_given = false;
    let mut all = false;
    let mut max_bytes = DEFAULT_MAX_BYTES;
    let mut max_bytes_given = false;
    let mut truncate = None;
    let mut counts = false;
    let mut runs = false;
    let mut json = false;
    let mut seen = HashSet::new();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        if !seen.insert(flag) {
            return Err(format!("duplicate option: {flag}"));
        }
        if flag == "--json" {
            json = true;
            index += 1;
            continue;
        }
        if matches!(
            flag,
            "--input" | "--output" | "--paths" | "--all" | "--counts" | "--runs"
        ) {
            match flag {
                "--input" => input_only = true,
                "--output" => output_only = true,
                "--paths" => paths_only = true,
                "--all" => all = true,
                "--counts" => counts = true,
                _ => runs = true,
            }
            index += 1;
            continue;
        }
        if !matches!(
            flag,
            "--session"
                | "--select"
                | "--tool"
                | "--grep"
                | "--cursor"
                | "--limit"
                | "--max-bytes"
                | "--truncate"
        ) {
            return Err(format!(
                "unknown option: {flag}; see future session transcript --help"
            ));
        }
        let raw = args
            .get(index + 1)
            .ok_or_else(|| format!("missing value for {flag}"))?;
        if raw.is_empty() || raw.starts_with("--") {
            return Err(format!("missing value for {flag}"));
        }
        match flag {
            "--session" => {
                if raw.trim().is_empty() {
                    return Err("--session must not be empty".into());
                }
                session = Some(raw.clone());
            }
            "--select" => {
                let mut parsed = Vec::new();
                for token in raw.split(',') {
                    if token.trim().is_empty() {
                        return Err("--select contains an empty item".into());
                    }
                    if token.trim().eq_ignore_ascii_case("all") {
                        parsed = vec![
                            Sel::Session,
                            Sel::Compaction,
                            Sel::User,
                            Sel::Assistant,
                            Sel::Thinking,
                            Sel::ToolCall,
                            Sel::ToolResult,
                        ];
                        continue;
                    }
                    parsed.push(Sel::parse(token).ok_or_else(|| {
                        format!(
                            "unknown --select item: {token}; \
                             expected user, assistant, thinking, tool-call, tool-result, session, compaction, all"
                        )
                    })?);
                }
                parsed.dedup();
                select = Some(parsed);
            }
            "--tool" => {
                for token in raw.split(',') {
                    if token.trim().is_empty() {
                        return Err("--tool contains an empty item".into());
                    }
                    tools.push(token.trim().to_string());
                }
            }
            "--grep" => {
                if raw.trim().is_empty() || raw.chars().count() > 200 || raw.contains('\0') {
                    return Err("--grep must be 1..200 characters without NUL".into());
                }
                grep = Some(raw.clone());
            }
            "--cursor" => {
                cursor = raw
                    .parse()
                    .map_err(|_| "--cursor must be a nonnegative integer")?;
                if cursor < 0 {
                    return Err("--cursor must be a nonnegative integer".into());
                }
            }
            "--limit" => {
                limit = raw.parse().map_err(|_| "--limit must be an integer")?;
                limit_given = true;
            }
            "--max-bytes" => {
                max_bytes = raw.parse().map_err(|_| "--max-bytes must be an integer")?;
                if max_bytes > 64 * 1024 * 1024 {
                    return Err("--max-bytes must be at most 64 MiB".into());
                }
                max_bytes_given = true;
            }
            "--truncate" => {
                let value: usize = raw.parse().map_err(|_| "--truncate must be an integer")?;
                if value == 0 || value > MAX_TRUNCATE {
                    return Err(format!("--truncate must be between 1 and {MAX_TRUNCATE}"));
                }
                truncate = Some(value);
            }
            _ => unreachable!("flag list checked above"),
        }
        index += 2;
    }
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(format!("--limit must be between 1 and {MAX_LIMIT}"));
    }
    if all && limit_given {
        return Err("--all and --limit are mutually exclusive".into());
    }
    if counts && runs {
        return Err("--counts and --runs are mutually exclusive".into());
    }
    // The two summary modes read the whole window; paging a summary would
    // report a partial distribution as if it were the session's.
    if (counts || runs) && cursor != 0 {
        return Err("--cursor is not valid with --counts or --runs".into());
    }
    // Truncating a run's error text would hide the part worth reading, so the
    // flag is refused rather than silently ignored.
    if runs && truncate.is_some() {
        return Err("--truncate is not valid with --runs".into());
    }
    Ok(Options {
        session: session.ok_or("--session is required")?,
        select: select.unwrap_or_else(default_select),
        tools,
        input_only,
        output_only,
        paths_only,
        grep,
        cursor,
        limit,
        limit_given,
        all,
        max_bytes,
        max_bytes_given,
        truncate,
        counts,
        runs,
        json,
    })
}

// ── block helpers ───────────────────────────────────────────────────────────

/// A string field, or "" when absent/wrong-typed.
fn string(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or("").to_string()
}

/// The parsed arguments of a `tool_call` block. The typed contract carries
/// `arguments` already parsed; the legacy JSON fallback spells the same value
/// as an `argumentsJson` string, which is decoded here.
fn block_arguments(block: &Value) -> Option<Value> {
    if let Some(value) = block.get("arguments") {
        if !value.is_null() {
            return Some(value.clone());
        }
    }
    block
        .get("argumentsJson")
        .and_then(Value::as_str)
        .and_then(|raw| serde_json::from_str(raw).ok())
        .or_else(|| block.get("argumentsJson").filter(|v| !v.is_null()).cloned())
}

/// Everything a `--grep` may match inside one already-selected block.
fn searchable(block: &Value) -> String {
    let mut haystack = string(block, "text");
    if let Some(name) = block.get("name").and_then(Value::as_str) {
        haystack.push('\n');
        haystack.push_str(name);
    }
    if let Some(arguments) = block_arguments(block) {
        haystack.push('\n');
        haystack.push_str(&arguments.to_string());
    }
    haystack
}

fn grep_matches(grep: &str, needle: &str) -> bool {
    needle.to_lowercase().contains(&grep.to_lowercase())
}

// ── path extraction ─────────────────────────────────────────────────────────

/// Argument keys whose value is a filesystem path. Normalised by lowercasing
/// and dropping `_`/`-`, so `file_path` and `filePath` both match.
const PATH_KEYS: &[&str] = &[
    "path",
    "file",
    "filepath",
    "filename",
    "dir",
    "directory",
    "cwd",
    "workdir",
    "target",
    "targetpath",
    "notebookpath",
    "imagepath",
    "outputpath",
    "inputpath",
    "sourcepath",
    "destinationpath",
    "savepath",
];

fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|c| *c != '_' && *c != '-')
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn is_path_key(key: &str) -> bool {
    let normalized = normalize_key(key);
    PATH_KEYS.contains(&normalized.as_str())
}

/// Recursively collect the string values under path-ish keys.
fn collect_paths_from_json(value: &Value, out: &mut Vec<String>) {
    if out.len() >= MAX_PATHS_PER_BLOCK {
        return;
    }
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if is_path_key(key) {
                    if let Some(path) = child.as_str() {
                        out.push(path.to_string());
                    }
                }
                collect_paths_from_json(child, out);
                if out.len() >= MAX_PATHS_PER_BLOCK {
                    return;
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_paths_from_json(item, out);
                if out.len() >= MAX_PATHS_PER_BLOCK {
                    return;
                }
            }
        }
        _ => {}
    }
}

/// True for tokens that unambiguously look like a filesystem path: absolute
/// POSIX, home-relative, dot-relative, or a Windows drive path. A bare
/// `src/main.rs` is deliberately not one — shell output is full of them and
/// they are indistinguishable from prose.
fn looks_like_path(token: &str) -> bool {
    let bytes = token.as_bytes();
    if token.starts_with("~/") || token.starts_with("./") || token.starts_with("../") {
        return token.chars().any(|c| c.is_alphanumeric());
    }
    if token.starts_with('/') {
        return token.len() > 1
            && !token.starts_with("//")
            && token.chars().any(|c| c.is_alphanumeric());
    }
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
        && token.chars().any(|c| c.is_alphanumeric())
}

/// Tokenise free text and keep the path-looking pieces. Meant for tool
/// *results*, where the platform wrote no structured arguments.
fn collect_paths_from_text(text: &str, out: &mut Vec<String>) {
    // `:` is deliberately not a separator: it is a Windows drive separator
    // (`C:\dev\a.rs`), and no path starts with a bare `key:` prefix anyway.
    let is_separator = |c: char| c.is_whitespace() || ",;()[]{}<>|\"'`".contains(c);
    for raw in text.split(is_separator) {
        if out.len() >= MAX_PATHS_PER_BLOCK {
            return;
        }
        let token = raw.trim_end_matches(['.', ',', ':', ';', ')', ']', '}']);
        if !token.is_empty() && looks_like_path(token) {
            out.push(token.to_string());
        }
    }
}

fn dedupe(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

/// The paths a block mentions: structured keys for a call's arguments, free
/// text for a result.
fn block_paths(block: &Value, kind: &str) -> Vec<String> {
    let mut paths = Vec::new();
    if kind == "tool_call" {
        if let Some(arguments) = block_arguments(block) {
            collect_paths_from_json(&arguments, &mut paths);
        }
    } else if let Some(text) = block.get("text").and_then(Value::as_str) {
        collect_paths_from_text(text, &mut paths);
    }
    paths.truncate(MAX_PATHS_PER_BLOCK);
    dedupe(paths)
}

// ── value shaping ───────────────────────────────────────────────────────────

/// Truncate every string in `value` longer than `limit` characters, appending
/// `…`. Recurses so a long string *inside* tool arguments is bounded too.
fn truncate_strings(value: &mut Value, limit: usize) {
    match value {
        Value::String(text) => {
            if text.chars().count() > limit {
                let mut cut: String = text.chars().take(limit).collect();
                cut.push('…');
                *text = cut;
            }
        }
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| truncate_strings(item, limit)),
        Value::Object(map) => map
            .values_mut()
            .for_each(|item| truncate_strings(item, limit)),
        _ => {}
    }
}

// ── observation ─────────────────────────────────────────────────────────────

/// A tool name resolved from the calls already scanned in this window.
type CallNames = HashMap<String, String>;

/// The distribution of one window, for `--counts`.
#[derive(Debug, Default, PartialEq, Eq)]
struct Counts {
    entries: BTreeMap<String, i64>,
    blocks: BTreeMap<String, i64>,
    tools: BTreeMap<String, i64>,
    tool_errors: i64,
    /// Run outcomes by status — the answer to "did anything fail here".
    runs: BTreeMap<String, i64>,
    content_bytes: usize,
    tokens: BTreeMap<String, i64>,
    first_at_ms: Option<i64>,
    last_at_ms: Option<i64>,
}

impl Counts {
    fn tally(&mut self, entry: &Value) {
        // Run annotations ride on ordinary entries, so they are folded in
        // before the per-kind work below.
        if let Some(run) = entry.get("run").filter(|v| v.is_object()) {
            let status = run["status"].as_str().unwrap_or("unknown");
            *self.runs.entry(status.to_string()).or_insert(0) += 1;
        }
        if let Some(usage) = entry.get("usage").filter(|v| v.is_object()) {
            for (key, field) in [
                ("input", "inputTokens"),
                ("output", "outputTokens"),
                ("cacheRead", "cacheReadTokens"),
                ("cacheWrite", "cacheWriteTokens"),
            ] {
                if let Some(count) = usage[field].as_i64() {
                    *self.tokens.entry(key.to_string()).or_insert(0) += count;
                }
            }
        }
        let kind = string(entry, "kind");
        *self.entries.entry(kind.clone()).or_insert(0) += 1;
        if let Some(ms) = entry["createdAtMs"].as_i64() {
            self.first_at_ms = Some(self.first_at_ms.map_or(ms, |current| current.min(ms)));
            self.last_at_ms = Some(self.last_at_ms.map_or(ms, |current| current.max(ms)));
        }
        for block in entry["blocks"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
            let block_kind = string(block, "kind");
            *self.blocks.entry(block_kind.clone()).or_insert(0) += 1;
            self.content_bytes += block
                .get("text")
                .and_then(Value::as_str)
                .map_or(0, str::len);
            if block_kind == "tool_call" {
                if let Some(name) = block.get("name").and_then(Value::as_str) {
                    *self.tools.entry(name.to_string()).or_insert(0) += 1;
                }
            }
            if block_kind == "tool_result" && block["isError"] == true {
                self.tool_errors += 1;
            }
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "entries": self.entries,
            "blocks": self.blocks,
            "tools": self.tools,
            "toolErrors": self.tool_errors,
            "runs": self.runs,
            "tokens": self.tokens,
            "contentBytes": self.content_bytes,
            "firstAtMs": self.first_at_ms,
            "lastAtMs": self.last_at_ms,
        })
    }
}

/// One entry that survived the selection.
struct Kept {
    entry: Value,
}

/// Copy the run outcome and per-run token usage onto a projected entry.
///
/// The display projection already carries both (every entry of a run shares the
/// same `run` object, and the run's final assistant carries `usage`), and they
/// are tens of bytes against kilobytes of text — so dropping them would make
/// "which run failed" and "what did this run cost" unanswerable without
/// re-reading the journal. `--counts` and `--runs` summarise the same fields.
fn attach_run_outcome(projected: &mut Value, entry: &Value) {
    if let Some(run_id) = entry.get("runId").filter(|v| !v.is_null()) {
        projected["runId"] = run_id.clone();
    }
    if let Some(run) = entry
        .get("run")
        .filter(|v| !v.is_null() && v.as_object().is_some_and(|o| !o.is_empty()))
    {
        projected["run"] = run.clone();
    }
    if let Some(usage) = entry
        .get("usage")
        .filter(|v| !v.is_null() && v.as_object().is_some_and(|o| !o.is_empty()))
    {
        projected["usage"] = usage.clone();
    }
}

/// One run's outcome, as the `--runs` ledger reports it.
#[derive(Debug, Default, Clone)]
struct RunRow {
    run_id: String,
    status: String,
    duration_ms: Option<i64>,
    error: Option<String>,
    first_ordinal: i64,
    usage: Option<Value>,
}

impl RunRow {
    fn to_json(&self) -> Value {
        let mut value = json!({
            "runId": self.run_id,
            "status": self.status,
            "firstOrdinal": self.first_ordinal,
        });
        if let Some(duration) = self.duration_ms {
            value["durationMs"] = json!(duration);
        }
        if let Some(error) = self.error.as_deref().filter(|e| !e.is_empty()) {
            value["error"] = json!(error);
        }
        if let Some(usage) = &self.usage {
            value["usage"] = usage.clone();
        }
        value
    }
}

/// Fold one entry's run annotation into the ledger, keeping the first position
/// and upgrading to the entry that actually carries the token usage.
fn record_run(ledger: &mut Vec<RunRow>, entry: &Value, ordinal: i64) {
    let Some(run) = entry
        .get("run")
        .filter(|v| v.as_object().is_some_and(|o| !o.is_empty()))
    else {
        return;
    };
    let run_id = entry
        .get("runId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let usage = entry
        .get("usage")
        .filter(|v| v.as_object().is_some_and(|o| !o.is_empty()))
        .cloned();
    if let Some(row) = ledger.iter_mut().find(|row| row.run_id == run_id) {
        if row.usage.is_none() {
            row.usage = usage;
        }
        return;
    }
    ledger.push(RunRow {
        run_id,
        status: run["status"].as_str().unwrap_or("unknown").to_string(),
        duration_ms: run["durationMs"].as_i64(),
        error: run["error"].as_str().map(str::to_string),
        first_ordinal: ordinal,
        usage,
    });
}

/// Project one display entry through the selection. `call_names` accumulates
/// tool-call ids → tool names so a later result can be attributed; `keep_ids`
/// records the ids whose call matched `--tool`.
fn filter_entry(
    entry: &Value,
    options: &Options,
    ordinal: i64,
    call_names: &mut CallNames,
    keep_ids: &mut HashSet<String>,
    unattributed: &mut i64,
) -> Option<Kept> {
    let kind = string(entry, "kind");
    let selected = |needle: Sel| options.select.contains(&needle);

    // `session`/`compaction` entries carry no blocks; their whole payload is
    // the observation.
    if kind == "session_info" || kind == "compaction" {
        let want = if kind == "session_info" {
            (Sel::Session, "session")
        } else {
            (Sel::Compaction, "checkpoint")
        };
        if !selected(want.0) {
            return None;
        }
        let payload = entry.get(want.1).filter(|v| !v.is_null())?;
        if let Some(grep) = &options.grep {
            if !grep_matches(grep, &payload.to_string()) {
                return None;
            }
        }
        let mut payload = payload.clone();
        if let Some(limit) = options.truncate {
            truncate_strings(&mut payload, limit);
        }
        let mut projected = json!({
            "ordinal": ordinal,
            "id": string(entry, "id"),
            "kind": kind,
            "role": string(entry, "role"),
            "createdAtMs": entry["createdAtMs"],
        });
        projected[want.1] = payload;
        attach_run_outcome(&mut projected, entry);
        return Some(Kept { entry: projected });
    }

    let mut blocks = Vec::new();
    for block in entry["blocks"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        let block_kind = string(block, "kind");
        let tool_call_id = string(block, "toolCallId");
        let (allowed, name) = match block_kind.as_str() {
            "text" => (
                (kind == "user" && selected(Sel::User))
                    || (kind == "assistant" && selected(Sel::Assistant)),
                None,
            ),
            "reasoning" => (kind == "assistant" && selected(Sel::Thinking), None),
            "tool_call" => {
                let name = block.get("name").and_then(Value::as_str).unwrap_or("");
                // Attribution is recorded even when the call itself is not
                // selected: `--select tool-result --tool shell` must still be
                // able to name the results it keeps.
                if !tool_call_id.is_empty() && !name.is_empty() {
                    call_names.insert(tool_call_id.clone(), name.to_string());
                }
                let matches = tool_selected(&options.tools, name);
                if matches && !tool_call_id.is_empty() {
                    keep_ids.insert(tool_call_id.clone());
                }
                (selected(Sel::ToolCall) && matches, Some(name.to_string()))
            }
            "tool_result" => {
                if !selected(Sel::ToolResult) {
                    (false, None)
                } else if options.tools.is_empty() {
                    (true, None)
                } else {
                    // A result carries no name of its own: resolve it from the
                    // call that was scanned earlier in this window — or from a
                    // call that matched --tool and was kept.
                    let resolved = call_names.get(&tool_call_id).cloned().or_else(|| {
                        (!tool_call_id.is_empty() && keep_ids.contains(&tool_call_id))
                            .then(|| options.tools.first().cloned().unwrap_or_default())
                    });
                    match resolved {
                        Some(name) => {
                            let matches = tool_selected(&options.tools, &name);
                            (matches, Some(name))
                        }
                        None => {
                            *unattributed += 1;
                            (false, None)
                        }
                    }
                }
            }
            // Media and opaque provider blocks are not part of this view.
            _ => (false, None),
        };
        if !allowed {
            continue;
        }
        if let Some(grep) = &options.grep {
            if !grep_matches(grep, &searchable(block)) {
                continue;
            }
        }
        let mut projected = match block_kind.as_str() {
            "text" | "reasoning" => json!({
                "kind": block_kind,
                "text": block.get("text").cloned().unwrap_or(Value::Null),
            }),
            "tool_call" => {
                if options.paths_only {
                    let mut projected = json!({
                        "kind": "tool_call",
                        "toolCallId": tool_call_id,
                        "paths": block_paths(block, "tool_call"),
                    });
                    insert_name(&mut projected, &name);
                    projected
                } else if options.input_only {
                    json!({
                        "kind": "tool_call",
                        "toolCallId": tool_call_id,
                        "arguments": block_arguments(block).unwrap_or(Value::Null),
                    })
                } else {
                    let mut projected = json!({
                        "kind": "tool_call",
                        "toolCallId": tool_call_id,
                        "arguments": block_arguments(block).unwrap_or(Value::Null),
                    });
                    insert_name(&mut projected, &name);
                    projected
                }
            }
            "tool_result" => {
                if options.paths_only {
                    let mut projected = json!({
                        "kind": "tool_result",
                        "toolCallId": tool_call_id,
                        "paths": block_paths(block, "tool_result"),
                    });
                    insert_name(&mut projected, &name);
                    projected
                } else if options.output_only {
                    json!({
                        "kind": "tool_result",
                        "text": block.get("text").cloned().unwrap_or(Value::Null),
                    })
                } else {
                    let mut projected = json!({
                        "kind": "tool_result",
                        "toolCallId": tool_call_id,
                        "text": block.get("text").cloned().unwrap_or(Value::Null),
                    });
                    if block["isError"] == true {
                        projected["isError"] = Value::Bool(true);
                    }
                    projected
                }
            }
            _ => unreachable!("allowed kinds enumerated above"),
        };
        if let Some(limit) = options.truncate {
            truncate_strings(&mut projected, limit);
        }
        blocks.push(projected);
    }
    if blocks.is_empty() {
        return None;
    }
    let mut projected = json!({
        "ordinal": ordinal,
        "id": string(entry, "id"),
        "kind": kind,
        "role": string(entry, "role"),
        "createdAtMs": entry["createdAtMs"],
    });
    attach_run_outcome(&mut projected, entry);
    projected["blocks"] = Value::Array(blocks);
    Some(Kept { entry: projected })
}

/// Record a tool name on a projected block, but only when it is known: an
/// unattributed result has none, and `null` there reads as "a tool named
/// null" rather than "unknown".
fn insert_name(projected: &mut Value, name: &Option<String>) {
    if let Some(name) = name.as_deref().filter(|name| !name.is_empty()) {
        projected["name"] = json!(name);
    }
}

/// An empty `--tool` list accepts every tool.
fn tool_selected(tools: &[String], name: &str) -> bool {
    tools.is_empty() || tools.iter().any(|tool| tool.as_str() == name)
}

fn json_len(value: &Value) -> usize {
    serde_json::to_vec(value).map_or(0, |bytes| bytes.len())
}

fn format_timestamp(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_else(|| ms.to_string())
}

fn format_text(
    session: &str,
    scanned: i64,
    emitted: &[Value],
    bytes: usize,
    has_more: bool,
    next_cursor: i64,
    unattributed: i64,
) -> String {
    let mut out = format!(
        "session={session} scanned={scanned} entries={} bytes={bytes} hasMore={has_more} nextCursor={next_cursor}\n",
        emitted.len()
    );
    if unattributed > 0 {
        out.push_str(&format!(
            "note: {unattributed} tool result(s) had no call in this window and were skipped; start at --cursor 0 to include them\n"
        ));
    }
    for entry in emitted {
        out.push_str(&format!(
            "\n[{}] {} {}\n",
            string(entry, "kind"),
            format_timestamp(entry["createdAtMs"].as_i64().unwrap_or(0)),
            entry["ordinal"]
        ));
        for block in entry["blocks"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
            let kind = string(block, "kind");
            match kind.as_str() {
                "text" | "reasoning" => {
                    out.push_str(&format!("  {kind}: {}\n", string(block, "text")));
                }
                "tool_call" => {
                    if let Some(paths) = block.get("paths") {
                        out.push_str(&format!(
                            "  tool-call {}: paths {}\n",
                            string(block, "name"),
                            compact_paths(paths)
                        ));
                    } else {
                        let name = string(block, "name");
                        let label = if name.is_empty() {
                            "tool-call".to_string()
                        } else {
                            format!("tool-call {name}")
                        };
                        out.push_str(&format!(
                            "  {label}: {}\n",
                            block.get("arguments").cloned().unwrap_or(Value::Null)
                        ));
                    }
                }
                "tool_result" => {
                    if let Some(paths) = block.get("paths") {
                        out.push_str(&format!("  tool-result: paths {}\n", compact_paths(paths)));
                    } else {
                        out.push_str(&format!(
                            "  tool-result{}: {}\n",
                            if block["isError"] == true {
                                " (error)"
                            } else {
                                ""
                            },
                            string(block, "text")
                        ));
                    }
                }
                _ => {}
            }
        }
    }
    if has_more {
        out.push_str(&format!(
            "\nMore entries exist: repeat with --cursor {next_cursor}.\n"
        ));
    }
    out
}

/// `["/a", "/b"]` → `/a, /b`, with an ellipsis when the list was capped.
fn compact_paths(paths: &Value) -> String {
    let items = paths.as_array().map(Vec::as_slice).unwrap_or(&[]);
    let joined = items
        .iter()
        .map(|item| item.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(", ");
    if joined.is_empty() {
        "(none)".to_string()
    } else {
        joined
    }
}

/// The `--runs` ledger: one line per distinct run, in the order they started.
fn format_run_ledger(
    session: &str,
    ledger: &[RunRow],
    has_more: bool,
    cut_by_bytes: bool,
) -> String {
    let failed = ledger
        .iter()
        .filter(|row| !matches!(row.status.as_str(), "completed"))
        .count();
    let mut out = format!("session={session} runs={} failed={failed}\n", ledger.len());
    if ledger.is_empty() {
        out.push_str("\nNo runs recorded in this window.\n");
        return out;
    }
    for row in ledger {
        let duration = row
            .duration_ms
            .map(|ms| format!(" {:.1}s", ms as f64 / 1000.0))
            .unwrap_or_default();
        let tokens = row
            .usage
            .as_ref()
            .and_then(|usage| usage["inputTokens"].as_i64())
            .map(|input| format!(" in={} ", human_tokens(input)))
            .unwrap_or_default();
        out.push_str(&format!(
            "\n[{:>4}] {}  {}{}{}",
            row.first_ordinal, row.run_id, row.status, duration, tokens
        ));
        if let Some(error) = row.error.as_deref().filter(|e| !e.is_empty()) {
            out.push_str(&format!("\n       error: {error}"));
        }
        out.push('\n');
    }
    if has_more {
        out.push_str(if cut_by_bytes {
            "\nMore runs exist; this cut is from --max-bytes. Raise it for the whole ledger.\n"
        } else {
            "\nMore runs exist; raise --limit or pass --all.\n"
        });
    }
    out
}

fn format_counts(
    session: &str,
    scanned: i64,
    counts: &Counts,
    has_more: bool,
    next_cursor: i64,
    cut_by_bytes: bool,
) -> String {
    let mut out = format!(
        "session={session} scanned={scanned} hasMore={has_more} nextCursor={next_cursor}\n"
    );
    let mut section = |title: &str, map: &BTreeMap<String, i64>| {
        if map.is_empty() {
            return;
        }
        out.push_str(&format!("\n{title}:\n"));
        for (key, value) in map {
            out.push_str(&format!("  {key}: {value}\n"));
        }
    };
    section("entries", &counts.entries);
    section("blocks", &counts.blocks);
    section("tools", &counts.tools);
    section("runs", &counts.runs);
    section("tokens", &counts.tokens);
    out.push_str(&format!(
        "\ntoolErrors={} contentBytes={}\n",
        counts.tool_errors, counts.content_bytes
    ));
    if let (Some(first), Some(last)) = (counts.first_at_ms, counts.last_at_ms) {
        out.push_str(&format!(
            "window={}..{}\n",
            format_timestamp(first),
            format_timestamp(last)
        ));
    }
    if has_more && cut_by_bytes {
        // `--cursor` is refused with the summaries, so pointing at it would be
        // advice the caller cannot follow.
        out.push_str("\nPartial distribution: raise --max-bytes for the whole session.\n");
    } else if has_more {
        out.push_str(&format!(
            "\nMore entries exist: repeat with --cursor {next_cursor}.\n"
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

    // The two summary modes describe the window: they ignore the content
    // filters, and cover the whole session unless the caller bounded them.
    let summary = options.counts || options.runs;
    let bound: Option<i64> = if options.all || (summary && !options.limit_given) {
        None
    } else {
        Some(options.limit)
    };

    // The byte budget is the caller's bound in the entry modes (a page of
    // entries must stay readable), but in the summary modes it is only a bound
    // they asked for: a distribution that silently covered part of a session
    // would be worse than a slow one.
    let byte_budget = !summary || options.max_bytes_given;

    let mut next_cursor = options.cursor;
    let mut scanned = 0i64;
    let mut emitted: Vec<Value> = Vec::new();
    let mut counts = Counts::default();
    let mut ledger: Vec<RunRow> = Vec::new();
    let mut bytes = 0usize;
    let mut cut_by_bytes = false;
    let mut unattributed = 0i64;
    let mut call_names: CallNames = HashMap::new();
    let mut keep_ids: HashSet<String> = HashSet::new();

    let has_more = loop {
        let page = client
            .get_session_entries_page(&options.session, next_cursor, RPC_PAGE)
            .await?;
        let entries = page["entries"].as_array().cloned().unwrap_or_default();
        let page_has_more = page["hasMore"] == true;
        if entries.is_empty() {
            break false;
        }
        let mut consumed = 0i64;
        // A stop leaves the unread entries for the next window, so the cursor
        // must still advance past everything this scan did consume.
        let mut stop = false;
        for entry in &entries {
            let ordinal = next_cursor + consumed;
            // `--runs` counts distinct runs, so a run already in the ledger
            // does not spend the caller's budget twice.
            let produced_so_far = if options.counts {
                scanned
            } else if options.runs {
                // A run currently open (its id already in the ledger) does not
                // add a row, so the bound is checked after the merge below.
                ledger.len() as i64
            } else {
                emitted.len() as i64
            };
            if let Some(bound) = bound {
                if produced_so_far >= bound {
                    stop = true;
                    break;
                }
            }
            let produced = if summary {
                None
            } else {
                filter_entry(
                    entry,
                    &options,
                    ordinal,
                    &mut call_names,
                    &mut keep_ids,
                    &mut unattributed,
                )
            };
            let size = produced
                .as_ref()
                .map(|kept| json_len(&kept.entry))
                .unwrap_or(0);
            if byte_budget && options.max_bytes > 0 && bytes > 0 && bytes + size > options.max_bytes
            {
                // This entry does not fit; leave it for the next window so the
                // cursor always points at unread content.
                stop = true;
                cut_by_bytes = true;
                break;
            }
            consumed += 1;
            scanned += 1;
            if options.counts {
                counts.tally(entry);
                bytes += json_len(entry);
            } else if options.runs {
                record_run(&mut ledger, entry, ordinal);
            } else if let Some(kept) = produced {
                bytes += size;
                emitted.push(kept.entry);
            }
        }
        next_cursor += consumed;
        if stop {
            break true;
        }
        if page_has_more && consumed == entries.len() as i64 {
            continue;
        }
        break consumed < entries.len() as i64;
    };

    if options.counts {
        if options.json {
            let report = json!({
                "sessionId": options.session,
                "cursor": options.cursor,
                "nextCursor": next_cursor,
                "scannedEntries": scanned,
                "hasMore": has_more,
                "counts": counts.to_json(),
            });
            out.log(&serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?);
        } else {
            out.log(&format_counts(
                &options.session,
                scanned,
                &counts,
                has_more,
                next_cursor,
                cut_by_bytes,
            ));
        }
        return Ok(());
    }

    if options.runs {
        let failed = ledger
            .iter()
            .filter(|row| !matches!(row.status.as_str(), "completed"))
            .count();
        if options.json {
            let report = json!({
                "sessionId": options.session,
                "scannedEntries": scanned,
                "hasMore": has_more,
                "runs": ledger.len(),
                "failedRuns": failed,
                "ledger": ledger.iter().map(RunRow::to_json).collect::<Vec<_>>(),
            });
            out.log(&serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?);
        } else {
            out.log(&format_run_ledger(
                &options.session,
                &ledger,
                has_more,
                cut_by_bytes,
            ));
        }
        return Ok(());
    }

    if options.json {
        let report = json!({
            "sessionId": options.session,
            "cursor": options.cursor,
            "nextCursor": next_cursor,
            "scannedEntries": scanned,
            "emittedEntries": emitted.len(),
            "hasMore": has_more,
            "bytes": bytes,
            "skippedUnattributedToolResults": unattributed,
            "entries": emitted,
        });
        out.log(&serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?);
    } else {
        out.log(&format_text(
            &options.session,
            scanned,
            &emitted,
            bytes,
            has_more,
            next_cursor,
            unattributed,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    /// Test shim: the scan-level unattributed tally is not what these cases
    /// assert, so it is threaded through a throwaway counter here.
    fn filter_entry(
        entry: &Value,
        options: &Options,
        ordinal: i64,
        call_names: &mut CallNames,
        keep_ids: &mut HashSet<String>,
    ) -> Option<Kept> {
        let mut unattributed = 0i64;
        super::filter_entry(
            entry,
            options,
            ordinal,
            call_names,
            keep_ids,
            &mut unattributed,
        )
    }

    fn entry(kind: &str, blocks: Value) -> Value {
        json!({
            "id": format!("{kind}-1"),
            "kind": kind,
            "role": kind,
            "createdAtMs": 1_785_900_000_000i64,
            "blocks": blocks,
        })
    }

    fn text_block(text: &str) -> Value {
        json!({"kind": "text", "text": text})
    }

    fn call_block(name: &str, id: &str, arguments: Value) -> Value {
        json!({"kind": "tool_call", "toolCallId": id, "name": name, "arguments": arguments})
    }

    fn result_block(id: &str, text: &str) -> Value {
        json!({"kind": "tool_result", "toolCallId": id, "text": text})
    }

    fn options(extra: &[&str]) -> Options {
        let mut argv = args(&["--session", "s1"]);
        argv.extend(args(extra));
        parse(&argv).expect("options parse")
    }

    #[test]
    fn options_defaults_and_strictness() {
        let parsed = options(&[]);
        assert_eq!(parsed.session, "s1");
        assert_eq!(parsed.select, default_select());
        assert_eq!(parsed.limit, DEFAULT_LIMIT);
        assert_eq!(parsed.max_bytes, DEFAULT_MAX_BYTES);
        assert!(!parsed.all && !parsed.counts && !parsed.json);

        // Duplicates and unknown flags are rejected, not ignored.
        assert!(parse(&args(&["--session", "s", "--session", "t"])).is_err());
        assert!(parse(&args(&["--session", "s", "--bogus"])).is_err());
        assert!(parse(&args(&["--session", "s", "--limit", "0"])).is_err());
        assert!(parse(&args(&["--session", "s", "--limit", "501"])).is_err());
        assert!(parse(&args(&["--session", "s", "--cursor", "-1"])).is_err());
        assert!(parse(&args(&["--session", "s", "--truncate", "0"])).is_err());
        assert!(parse(&args(&["--session", "s", "--all", "--limit", "3"])).is_err());
        assert!(parse(&args(&[])).is_err(), "--session is required");
        assert!(parse(&args(&["--session", "s", "--select", "bogus"])).is_err());
        assert!(parse(&args(&["--session", "s", "--select", "user,,tool-call"])).is_err());
        assert!(parse(&args(&["--session", "s", "--grep", " "])).is_err());
    }

    #[test]
    fn select_parses_aliases_and_all() {
        let parsed = options(&["--select", "user,tool_call,reasoning,Session"]);
        assert_eq!(
            parsed.select,
            vec![Sel::User, Sel::ToolCall, Sel::Thinking, Sel::Session]
        );
        let all = options(&["--select", "all"]);
        assert_eq!(all.select.len(), 7);
    }

    #[test]
    fn select_user_drops_non_user_entries_entirely() {
        let assistant = entry(
            "assistant",
            json!([
                {"kind": "reasoning", "text": "thought"},
                text_block("answer"),
                call_block("read", "c1", json!({"path": "/tmp/a"})),
            ]),
        );
        let mut names = CallNames::new();
        let mut ids = HashSet::new();
        assert!(filter_entry(
            &assistant,
            &options(&["--select", "user"]),
            0,
            &mut names,
            &mut ids
        )
        .is_none());

        let user = entry("user", json!([text_block("question")]));
        let mut names = CallNames::new();
        let mut ids = HashSet::new();
        let kept = filter_entry(
            &user,
            &options(&["--select", "user"]),
            0,
            &mut names,
            &mut ids,
        )
        .expect("user text kept");
        assert_eq!(kept.entry["blocks"][0]["text"], json!("question"));
    }

    #[test]
    fn tool_filter_attributes_results_even_when_calls_are_not_selected() {
        let mut names = CallNames::new();
        let mut ids = HashSet::new();
        let opts = options(&["--tool", "shell", "--select", "tool-result"]);
        let call = entry(
            "assistant",
            json!([call_block("shell", "c1", json!({"command": "ls"}))]),
        );
        assert!(filter_entry(&call, &opts, 0, &mut names, &mut ids).is_none());
        let result = entry("tool", json!([result_block("c1", "out")]));
        let kept = filter_entry(&result, &opts, 1, &mut names, &mut ids).expect("output kept");
        assert_eq!(kept.entry["blocks"][0]["text"], json!("out"));
    }

    #[test]
    fn select_assistant_text_excludes_thinking_and_calls() {
        let entry = entry(
            "assistant",
            json!([{"kind": "reasoning", "text": "thought"}, text_block("answer")]),
        );
        let mut names = CallNames::new();
        let mut ids = HashSet::new();
        let kept = filter_entry(
            &entry,
            &options(&["--select", "assistant"]),
            3,
            &mut names,
            &mut ids,
        )
        .expect("assistant text kept");
        assert_eq!(kept.entry["ordinal"], json!(3));
        assert_eq!(kept.entry["blocks"].as_array().unwrap().len(), 1);
        assert_eq!(kept.entry["blocks"][0]["kind"], json!("text"));
    }

    #[test]
    fn thinking_is_selectable_even_though_history_search_excludes_it() {
        let entry = entry("assistant", json!([{"kind": "reasoning", "text": "deep"}]));
        let mut names = CallNames::new();
        let mut ids = HashSet::new();
        let kept = filter_entry(
            &entry,
            &options(&["--select", "thinking"]),
            0,
            &mut names,
            &mut ids,
        )
        .expect("reasoning kept");
        assert_eq!(kept.entry["blocks"][0]["text"], json!("deep"));
    }

    #[test]
    fn tool_filter_keeps_the_call_and_its_result_and_marks_the_rest() {
        let mut names = CallNames::new();
        let mut ids = HashSet::new();
        let opts = options(&["--tool", "shell", "--select", "tool-call,tool-result"]);

        let call = entry(
            "assistant",
            json!([
                call_block("read", "c-read", json!({"path": "/tmp/a"})),
                call_block("shell", "c-shell", json!({"command": "ls"})),
            ]),
        );
        let kept = filter_entry(&call, &opts, 0, &mut names, &mut ids).expect("shell kept");
        assert_eq!(kept.entry["blocks"].as_array().unwrap().len(), 1);
        assert_eq!(kept.entry["blocks"][0]["name"], json!("shell"));

        // The read result's call was filtered out, so it is unattributed.
        let other = entry("tool", json!([result_block("c-read", "contents")]));
        assert!(filter_entry(&other, &opts, 1, &mut names, &mut ids).is_none());
        // The shell result arrives after its call and is attributed by name.
        let shell = entry("tool", json!([result_block("c-shell", "output")]));
        let kept = filter_entry(&shell, &opts, 2, &mut names, &mut ids).expect("shell result");
        assert_eq!(kept.entry["blocks"][0]["text"], json!("output"));

        let mut names = CallNames::new();
        let mut ids = HashSet::new();
        let orphan = filter_entry(&shell, &opts, 0, &mut names, &mut ids);
        assert!(orphan.is_none(), "no call in this window to attribute it");
    }

    #[test]
    fn input_and_output_modes_drop_the_other_half() {
        let call = entry(
            "assistant",
            json!([call_block("read", "c1", json!({"path": "/a"}))]),
        );
        let mut names = CallNames::new();
        let mut ids = HashSet::new();
        let kept = filter_entry(&call, &options(&["--input"]), 0, &mut names, &mut ids).unwrap();
        let block = &kept.entry["blocks"][0];
        assert!(block.get("name").is_none());
        assert_eq!(block["arguments"]["path"], json!("/a"));

        let result = entry("tool", json!([result_block("c1", "body")]));
        let mut names = CallNames::new();
        let mut ids = HashSet::new();
        let kept = filter_entry(&result, &options(&["--output"]), 0, &mut names, &mut ids).unwrap();
        let block = &kept.entry["blocks"][0];
        assert!(block.get("toolCallId").is_none());
        assert_eq!(block["text"], json!("body"));
    }

    #[test]
    fn paths_mode_extracts_arguments_for_calls_and_text_for_results() {
        let call = entry(
            "assistant",
            json!([call_block(
                "write",
                "c1",
                json!({"file_path": "/work/out.md", "content": "see /etc/hosts and src/lib.rs"})
            )]),
        );
        let mut names = CallNames::new();
        let mut ids = HashSet::new();
        let kept = filter_entry(&call, &options(&["--paths"]), 0, &mut names, &mut ids).unwrap();
        assert_eq!(kept.entry["blocks"][0]["paths"], json!(["/work/out.md"]));

        let result = entry(
            "tool",
            json!([result_block(
                "c1",
                "wrote /work/out.md and ~/notes.md; src/lib.rs"
            )]),
        );
        let mut names = CallNames::new();
        let mut ids = HashSet::new();
        let kept = filter_entry(&result, &options(&["--paths"]), 0, &mut names, &mut ids).unwrap();
        assert_eq!(
            kept.entry["blocks"][0]["paths"],
            json!(["/work/out.md", "~/notes.md"])
        );
    }

    #[test]
    fn path_lookalikes_are_rejected() {
        let mut out = Vec::new();
        collect_paths_from_text(
            "and/or 24/7 http://x/y C:\\dev\\a.rs ../up.txt ./here.txt nope",
            &mut out,
        );
        assert_eq!(out, vec!["C:\\dev\\a.rs", "../up.txt", "./here.txt"]);
    }

    #[test]
    fn grep_is_case_insensitive_and_scoped_to_selected_fields() {
        let entry = entry(
            "assistant",
            json!([
                text_block("ExpoSharing is used"),
                {"kind": "reasoning", "text": "unrelated"},
            ]),
        );
        // Thinking is not selected, so its text cannot match.
        let mut names = CallNames::new();
        let mut ids = HashSet::new();
        let kept = filter_entry(
            &entry,
            &options(&["--select", "assistant", "--grep", "exposharing"]),
            0,
            &mut names,
            &mut ids,
        )
        .expect("assistant text matches");
        assert_eq!(kept.entry["blocks"].as_array().unwrap().len(), 1);

        let mut names = CallNames::new();
        let mut ids = HashSet::new();
        assert!(filter_entry(
            &entry,
            &options(&["--select", "assistant", "--grep", "unrelated"]),
            0,
            &mut names,
            &mut ids,
        )
        .is_none());
    }

    #[test]
    fn grep_matches_tool_arguments() {
        let call = entry(
            "assistant",
            json!([call_block(
                "shell",
                "c1",
                json!({"command": "cargo test -p future-cli"})
            )]),
        );
        let mut names = CallNames::new();
        let mut ids = HashSet::new();
        assert!(filter_entry(
            &call,
            &options(&["--select", "tool-call", "--grep", "future-cli"]),
            0,
            &mut names,
            &mut ids,
        )
        .is_some());
    }

    #[test]
    fn truncate_bounds_nested_argument_strings() {
        let call = entry(
            "assistant",
            json!([call_block(
                "write",
                "c1",
                json!({"path": "/a", "content": "abcdefghij"})
            )]),
        );
        let mut names = CallNames::new();
        let mut ids = HashSet::new();
        let kept = filter_entry(
            &call,
            &options(&["--select", "tool-call", "--truncate", "4"]),
            0,
            &mut names,
            &mut ids,
        )
        .unwrap();
        assert_eq!(
            kept.entry["blocks"][0]["arguments"]["content"],
            json!("abcd…")
        );
        assert_eq!(kept.entry["blocks"][0]["arguments"]["path"], json!("/a"));
    }

    #[test]
    fn session_and_compaction_entries_carry_their_payload() {
        let info = json!({
            "id": "i1", "kind": "session_info", "role": "system", "createdAtMs": 1i64,
            "session": {"cwd": "/work", "model": "m", "thinkingLevel": "high"},
        });
        let mut names = CallNames::new();
        let mut ids = HashSet::new();
        assert!(filter_entry(&info, &options(&[]), 0, &mut names, &mut ids).is_none());
        let kept = filter_entry(
            &info,
            &options(&["--select", "session"]),
            0,
            &mut names,
            &mut ids,
        )
        .unwrap();
        assert_eq!(kept.entry["session"]["cwd"], json!("/work"));

        let compaction = json!({
            "id": "c1", "kind": "compaction", "role": "system", "createdAtMs": 1i64,
            "checkpoint": {"cutoffEntryId": "e-9"},
        });
        assert!(filter_entry(&compaction, &options(&[]), 0, &mut names, &mut ids).is_none());
        let kept = filter_entry(
            &compaction,
            &options(&["--select", "compaction"]),
            0,
            &mut names,
            &mut ids,
        )
        .unwrap();
        assert_eq!(kept.entry["checkpoint"]["cutoffEntryId"], json!("e-9"));
    }

    #[test]
    fn counts_tally_entries_blocks_tools_and_errors() {
        let mut counts = Counts::default();
        counts.tally(&entry(
            "assistant",
            json!([
                {"kind": "reasoning", "text": "ab"},
                text_block("cd"),
                call_block("shell", "c1", json!({"command": "ls"})),
            ]),
        ));
        counts.tally(&entry(
            "tool",
            json!([{"kind": "tool_result", "toolCallId": "c1", "text": "x", "isError": true}]),
        ));
        assert_eq!(counts.entries["assistant"], 1);
        assert_eq!(counts.blocks["tool_call"], 1);
        assert_eq!(counts.blocks["tool_result"], 1);
        assert_eq!(counts.tools["shell"], 1);
        assert_eq!(counts.tool_errors, 1);
        assert_eq!(counts.content_bytes, 5);
    }

    #[test]
    fn legacy_arguments_json_string_is_decoded() {
        let call = entry(
            "assistant",
            json!([{"kind": "tool_call", "toolCallId": "c1", "name": "read",
                    "argumentsJson": "{\"path\":\"/legacy\"}"}]),
        );
        let mut names = CallNames::new();
        let mut ids = HashSet::new();
        let kept = filter_entry(
            &call,
            &options(&["--select", "tool-call"]),
            0,
            &mut names,
            &mut ids,
        )
        .unwrap();
        assert_eq!(
            kept.entry["blocks"][0]["arguments"]["path"],
            json!("/legacy")
        );
    }

    // ── RPC-backed flows ───────────────────────────────────────────────────

    use crate::test_server::MockAgent;

    /// A mock agent answering `get_session_entries` with one JSON page.
    fn entries_agent(value: Value) -> MockAgent {
        MockAgent::respond("get_session_entries", &value.to_string())
    }

    async fn mock_env(agent: MockAgent) -> (MockAgent, crate::test_env::EnvGuard) {
        let addr = crate::test_server::spawn_mock(agent.clone()).await;
        let env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from(addr),
        )]);
        (agent, env)
    }

    /// Page one: entries 0..2 plus a continuation cursor; page two: the tail.
    fn two_page_agent() -> MockAgent {
        let mut agent = MockAgent::default();
        agent.responses.insert(
            "get_session_entries".into(),
            json!({
                "entries": [
                    entry("user", json!([text_block("question")])),
                    entry("assistant", json!([text_block("answer")])),
                ],
                "hasMore": true,
                "nextOffset": 2,
            })
            .to_string(),
        );
        agent
    }

    #[tokio::test]
    async fn transcript_pages_with_cursor_and_reports_more() {
        let _guard = crate::test_env::lock_env().await;
        let (_agent, _env) = mock_env(two_page_agent()).await;
        let (out, cap) = Output::memory();
        run(&args(&["--session", "s1", "--json", "--limit", "1"]), &out)
            .await
            .expect("transcript");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let report: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(report["emittedEntries"], json!(1));
        assert_eq!(report["hasMore"], json!(true));
        assert_eq!(report["nextCursor"], json!(1));
        assert_eq!(report["entries"][0]["blocks"][0]["text"], json!("question"));
    }

    #[tokio::test]
    async fn transcript_follows_pages_until_limit() {
        let _guard = crate::test_env::lock_env().await;
        let (_agent, _env) = mock_env(two_page_agent()).await;
        let (out, cap) = Output::memory();
        run(&args(&["--session", "s1", "--json", "--limit", "2"]), &out)
            .await
            .expect("transcript");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let report: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(report["emittedEntries"], json!(2));
        assert_eq!(report["hasMore"], json!(true));
        assert_eq!(report["nextCursor"], json!(2));
    }

    #[tokio::test]
    async fn transcript_resumes_from_a_cursor() {
        let _guard = crate::test_env::lock_env().await;
        let agent = entries_agent(json!({"entries": [entry("user", json!([text_block("tail")]))]}));
        let (agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        run(&args(&["--session", "s1", "--json", "--cursor", "7"]), &out)
            .await
            .expect("transcript");
        let sent = agent.seen_of("get_session_entries");
        assert_eq!(sent[0].offset, Some(7));
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let report: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(report["cursor"], json!(7));
        assert_eq!(report["entries"][0]["ordinal"], json!(7));
        assert_eq!(report["hasMore"], json!(false));
    }

    #[tokio::test]
    async fn transcript_counts_ignores_filters_and_reports_distributions() {
        let _guard = crate::test_env::lock_env().await;
        let agent = entries_agent(json!({
            "entries": [
                entry("user", json!([text_block("hi")])),
                entry("assistant", json!([{"kind": "reasoning", "text": "t"},
                                          call_block("shell", "c1", json!({"command": "ls"}))])),
                entry("tool", json!([result_block("c1", "out")])),
            ]
        }));
        let (agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        run(
            &args(&[
                "--session",
                "s1",
                "--counts",
                "--json",
                "--select",
                "user",
                "--tool",
                "nope",
            ]),
            &out,
        )
        .await
        .expect("counts");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let report: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(report["scannedEntries"], json!(3));
        assert_eq!(report["counts"]["entries"]["tool"], json!(1));
        assert_eq!(report["counts"]["tools"]["shell"], json!(1));
        // Counts read the whole window even though --limit was never given.
        assert_eq!(
            agent.seen_of("get_session_entries")[0].limit,
            Some(RPC_PAGE)
        );
    }

    #[tokio::test]
    async fn transcript_reports_unattributed_tool_results() {
        let _guard = crate::test_env::lock_env().await;
        let agent = entries_agent(
            json!({"entries": [entry("tool", json!([result_block("orphan", "body")]))]}),
        );
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        run(
            &args(&[
                "--session",
                "s1",
                "--json",
                "--tool",
                "shell",
                "--select",
                "tool-result",
            ]),
            &out,
        )
        .await
        .expect("transcript");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let report: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(report["skippedUnattributedToolResults"], json!(1));
        assert_eq!(report["emittedEntries"], json!(0));
    }

    #[tokio::test]
    async fn transcript_errors_when_the_agent_rejects_the_command() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = MockAgent::default();
        agent.fail_with.insert(
            "get_session_entries".into(),
            "Unable to load session history".into(),
        );
        let (_agent, _env) = mock_env(agent).await;
        let (out, _cap) = Output::memory();
        let error = run(&args(&["--session", "s1"]), &out).await.unwrap_err();
        assert!(error.contains("Unable to load session history"), "{error}");
    }

    /// An entry as the Agent actually sends it: a run outcome on every entry of
    /// the run, and the token usage on the run's final assistant.
    fn run_entry(kind: &str, run_id: &str, status: &str, usage: Option<Value>) -> Value {
        let mut entry = entry(kind, json!([text_block("body")]));
        entry["runId"] = json!(run_id);
        let mut run = json!({"status": status, "durationMs": 1500});
        if status == "failed" {
            run["error"] = json!("upstream disconnected");
        }
        entry["run"] = run;
        if let Some(usage) = usage {
            entry["usage"] = usage;
        }
        entry
    }

    #[test]
    fn emitted_entries_carry_their_run_outcome_and_usage() {
        let options = options(&["--select", "assistant"]);
        let mut names = CallNames::new();
        let mut ids = HashSet::new();
        let usage = json!({"inputTokens": 100, "outputTokens": 5});
        let kept = filter_entry(
            &run_entry("assistant", "run-1", "failed", Some(usage.clone())),
            &options,
            0,
            &mut names,
            &mut ids,
        )
        .expect("kept");
        assert_eq!(kept.entry["runId"], json!("run-1"));
        assert_eq!(kept.entry["run"]["status"], json!("failed"));
        assert_eq!(kept.entry["run"]["error"], json!("upstream disconnected"));
        assert_eq!(kept.entry["run"]["durationMs"], json!(1500));
        assert_eq!(kept.entry["usage"], usage);
    }

    #[test]
    fn entries_without_a_run_annotation_do_not_gain_empty_objects() {
        let options = options(&["--select", "assistant"]);
        let mut names = CallNames::new();
        let mut ids = HashSet::new();
        let plain = entry("assistant", json!([text_block("hi")]));
        let kept = filter_entry(&plain, &options, 0, &mut names, &mut ids).expect("kept");
        assert!(kept.entry.get("run").is_none());
        assert!(kept.entry.get("usage").is_none());
        assert!(kept.entry.get("runId").is_none());
    }

    #[test]
    fn the_run_ledger_dedupes_by_run_and_keeps_the_usage_carrying_entry() {
        let mut ledger = Vec::new();
        // Three entries of one run: the middle one carries the usage.
        record_run(
            &mut ledger,
            &run_entry("user", "run-1", "completed", None),
            0,
        );
        record_run(
            &mut ledger,
            &run_entry(
                "assistant",
                "run-1",
                "completed",
                Some(json!({"inputTokens": 9})),
            ),
            1,
        );
        record_run(
            &mut ledger,
            &run_entry("tool", "run-1", "completed", None),
            2,
        );
        record_run(&mut ledger, &run_entry("user", "run-2", "failed", None), 3);

        assert_eq!(ledger.len(), 2, "one row per run, not per entry");
        assert_eq!(ledger[0].run_id, "run-1");
        assert_eq!(ledger[0].first_ordinal, 0, "the run's first position wins");
        assert_eq!(ledger[0].usage.as_ref().unwrap()["inputTokens"], json!(9));
        assert_eq!(ledger[1].status, "failed");
        assert_eq!(ledger[1].error.as_deref(), Some("upstream disconnected"));
        // An entry with no run annotation contributes nothing.
        record_run(
            &mut ledger,
            &entry("assistant", json!([text_block("x")])),
            4,
        );
        assert_eq!(ledger.len(), 2);
    }

    #[test]
    fn counts_report_run_outcomes_and_tokens() {
        let mut counts = Counts::default();
        counts.tally(&run_entry("assistant", "r1", "completed", None));
        counts.tally(&run_entry(
            "assistant",
            "r2",
            "failed",
            Some(json!({"inputTokens": 100, "outputTokens": 7, "cacheReadTokens": 3})),
        ));
        assert_eq!(counts.runs["completed"], 1);
        assert_eq!(counts.runs["failed"], 1);
        assert_eq!(counts.tokens["input"], 100);
        assert_eq!(counts.tokens["output"], 7);
        assert_eq!(counts.tokens["cacheRead"], 3);
        assert_eq!(counts.tokens.get("cacheWrite"), None, "unset stays absent");
    }

    #[test]
    fn the_ledger_renders_status_duration_tokens_and_error() {
        let mut ledger = Vec::new();
        record_run(
            &mut ledger,
            &run_entry("user", "run-1", "completed", None),
            0,
        );
        record_run(
            &mut ledger,
            &run_entry(
                "assistant",
                "run-2",
                "failed",
                Some(json!({"inputTokens": 2_500_000})),
            ),
            7,
        );
        let rendered = format_run_ledger("s1", &ledger, false, false);
        assert!(rendered.contains("runs=2 failed=1"), "{rendered}");
        assert!(rendered.contains("run-1  completed 1.5s"), "{rendered}");
        assert!(rendered.contains("in=2.5M"), "{rendered}");
        assert!(
            rendered.contains("error: upstream disconnected"),
            "{rendered}"
        );
        assert!(!rendered.contains("More runs exist"));
        assert!(format_run_ledger("s1", &ledger, true, false).contains("raise --limit"));
        assert!(format_run_ledger("s1", &ledger, true, true).contains("--max-bytes"));
        assert!(format_run_ledger("s1", &[], false, false).contains("No runs recorded"));
    }

    /// A summary is either complete or says why it is not. `--cursor` is
    /// refused with the summaries, so the truncation message must not send the
    /// caller there.
    #[test]
    fn a_byte_cut_in_a_summary_points_at_max_bytes_not_the_cursor() {
        let counts = Counts::default();
        let cut = format_counts("s1", 10, &counts, true, 10, true);
        assert!(cut.contains("Partial distribution"), "{cut}");
        assert!(!cut.contains("--cursor"), "{cut}");
        // An entry page keeps the cursor advice, which is the right move there.
        let page = format_counts("s1", 10, &counts, true, 10, false);
        assert!(page.contains("repeat with --cursor 10"), "{page}");
    }

    #[test]
    fn summaries_cover_the_whole_session_unless_the_caller_bounds_them() {
        // An explicit --max-bytes is respected by the summary...
        let bounded = options(&["--counts", "--max-bytes", "1024"]);
        assert!(bounded.max_bytes_given && bounded.counts);
        // ...while the default is only the entry page's own budget.
        let default = options(&["--counts"]);
        assert!(!default.max_bytes_given);
    }

    #[tokio::test]
    async fn transcript_runs_mode_lists_runs_not_entries() {
        let _guard = crate::test_env::lock_env().await;
        let agent = entries_agent(json!({
            "entries": [
                run_entry("user", "run-1", "completed", None),
                run_entry("assistant", "run-1", "completed", Some(json!({"inputTokens": 10}))),
                run_entry("user", "run-2", "failed", None),
            ]
        }));
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        run(&args(&["--session", "s1", "--runs", "--json"]), &out)
            .await
            .expect("runs");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let report: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(report["runs"], json!(2));
        assert_eq!(report["failedRuns"], json!(1));
        assert_eq!(report["ledger"][0]["runId"], json!("run-1"));
        assert_eq!(report["ledger"][0]["usage"]["inputTokens"], json!(10));
        assert_eq!(report["ledger"][1]["status"], json!("failed"));
    }

    #[tokio::test]
    async fn transcript_runs_mode_honours_limit_as_a_run_count() {
        let _guard = crate::test_env::lock_env().await;
        let agent = entries_agent(json!({
            "entries": [
                run_entry("user", "run-1", "completed", None),
                run_entry("assistant", "run-1", "completed", None),
                run_entry("user", "run-2", "completed", None),
                run_entry("user", "run-3", "completed", None),
            ]
        }));
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        run(
            &args(&["--session", "s1", "--runs", "--json", "--limit", "2"]),
            &out,
        )
        .await
        .expect("runs");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let report: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(report["runs"], json!(2), "the limit counts runs");
        assert_eq!(report["hasMore"], json!(true));
    }

    #[test]
    fn summary_modes_reject_paging_and_truncate() {
        assert!(parse(&args(&["--session", "s1", "--counts", "--cursor", "5"])).is_err());
        assert!(parse(&args(&["--session", "s1", "--runs", "--cursor", "5"])).is_err());
        assert!(parse(&args(&["--session", "s1", "--counts", "--runs"])).is_err());
        // Refused at parse time — before any agent call — so a long session
        // cannot be scanned only to fail at the end.
        let error = parse(&args(&["--session", "s1", "--runs", "--truncate", "10"])).unwrap_err();
        assert!(
            error.contains("--truncate is not valid with --runs"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn transcript_help_needs_no_agent() {
        let (out, cap) = Output::memory();
        run(&args(&["--help"]), &out).await.expect("help");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.starts_with("future session transcript — filter and window"));
    }

    #[test]
    fn text_output_shows_the_cursor_and_blocks() {
        let entry = json!({
            "ordinal": 0, "id": "e1", "kind": "assistant", "role": "assistant",
            "createdAtMs": 0i64,
            "blocks": [{"kind": "text", "text": "hello"}],
        });
        let rendered = format_text("s1", 1, &[entry], 10, true, 1, 0);
        assert!(rendered.contains("hasMore=true nextCursor=1"));
        assert!(rendered.contains("text: hello"));
        assert!(rendered.contains("repeat with --cursor 1"));
    }
}
