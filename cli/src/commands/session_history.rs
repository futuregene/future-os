//! Read-only recall through the Agent; no direct database access or new model tool.
use crate::output::Output;
use crate::rpc::{grpc_addr, RunClient};
use serde_json::Value;
use std::collections::HashSet;

const HELP: &str = "future session history — search/read original conversation records

Usage:
  future session history search --session <id> --query <text> [--limit 5] [--json]
  future session history get --session <id> --entry <entry-id> [--offset 0] [--limit 8192] [--json]

Search matches literal text/arguments (ASCII case insensitive) or an exact tool-call ID, newest first.
Search limit: 1..20 matches. It includes text, tool arguments and results, not thinking.
Get offset/limit: UTF-8 bytes across readable blocks in entry order (not line numbers).
Get limit: 4..32768 bytes. Follow nextOffset for more; UTF-8 characters are not split.
Search byteOffset can be passed to get --offset. --json preserves exact text chunks.
Queries are scoped to the explicitly selected session; no default-session fallback.
Requires an Agent/CLI version supporting history recall. No model call is made.";

#[derive(Debug, PartialEq, Eq)]
struct Options {
    search: bool,
    session: String,
    value: String,
    offset: i64,
    limit: i64,
    json: bool,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let search = match args.first().map(String::as_str) {
        Some("search") => true,
        Some("get") => false,
        _ => return Err("expected history search or get".into()),
    };
    let mut session = None;
    let mut value = None;
    let mut offset = 0;
    let mut limit = if search { 5 } else { 8192 };
    let mut json = false;
    let mut seen = HashSet::new();
    let mut index = 1;
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
        if !matches!(
            flag,
            "--session" | "--query" | "--entry" | "--offset" | "--limit"
        ) {
            return Err(format!("unknown option: {flag}"));
        }
        let raw = args
            .get(index + 1)
            .ok_or_else(|| format!("missing value for {flag}"))?;
        if raw.is_empty() || raw.starts_with("--") {
            return Err(format!("missing value for {flag}"));
        }
        match flag {
            "--session" => session = Some(raw.clone()),
            "--query" if search => value = Some(raw.clone()),
            "--entry" if !search => value = Some(raw.clone()),
            "--offset" if !search => {
                offset = raw
                    .parse()
                    .map_err(|_| "offset must be a nonnegative integer")?
            }
            "--limit" => limit = raw.parse().map_err(|_| "limit must be an integer")?,
            _ => return Err(format!("{flag} is not valid for this history command")),
        }
        index += 2;
    }
    if offset < 0 || !(if search { 1..=20 } else { 4..=32768 }).contains(&limit) {
        return Err("offset/limit outside the documented range".into());
    }
    let session = session.ok_or("--session is required")?;
    let value = value.ok_or(if search {
        "--query is required"
    } else {
        "--entry is required"
    })?;
    if search && (value.trim().is_empty() || value.chars().count() > 200 || value.contains('\0')) {
        return Err("query must contain 1..200 characters without NUL".into());
    }
    Ok(Options {
        search,
        session,
        value,
        offset,
        limit,
        json,
    })
}

pub(super) async fn run(args: &[String], out: &Output) -> Result<(), String> {
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        out.log(HELP);
        return Ok(());
    }
    let options = parse(args)?;
    let client = RunClient::new(&grpc_addr());
    let result = if options.search {
        client
            .search_session_history(&options.session, &options.value, options.limit)
            .await?
    } else {
        client
            .get_session_history_entry(
                &options.session,
                &options.value,
                options.offset,
                options.limit,
            )
            .await?
    };
    if options.json {
        out.log(&serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?);
    } else {
        out.log(&format_result(&result, options.search));
    }
    Ok(())
}

fn string(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or("").to_string()
}
fn format_result(value: &Value, search: bool) -> String {
    if search {
        let matches = value["matches"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if matches.is_empty() {
            return "No matching history records.".into();
        }
        let mut out = matches
            .iter()
            .map(|m| {
                format!(
                    "entry={} block={} kind={} offset={}\n{}",
                    string(m, "entryId"),
                    m["blockIndex"],
                    string(m, "kind"),
                    m["byteOffset"],
                    string(m, "snippet")
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        if value["hasMore"] == true {
            out.push_str("\n\nMore matches exist; refine the query or increase --limit (max 20).");
        }
        return out;
    }
    let mut out = format!(
        "session={} entry={} role={} totalBytes={}\n",
        string(value, "sessionId"),
        string(value, "entryId"),
        string(value, "role"),
        value["totalBytes"]
    );
    for chunk in value["chunks"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        out.push_str(&format!(
            "\n[block={} kind={} field={} byteOffset={}]\n{}\n",
            chunk["blockIndex"],
            string(chunk, "kind"),
            string(chunk, "field"),
            chunk["blockByteOffset"],
            string(chunk, "text")
        ));
    }
    if value["hasMore"] == true {
        out.push_str(&format!(
            "\nMore content: repeat get with --offset {}.\n",
            value["nextOffset"]
        ));
    }
    if let Some(kinds) = value["omittedKinds"].as_array().filter(|v| !v.is_empty()) {
        out.push_str(&format!(
            "\nOmitted non-recall blocks: {}\n",
            serde_json::json!(kinds)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }
    #[test]
    fn history_options_are_explicit_and_strict() {
        let p = parse(&args(&[
            "search",
            "--session",
            "s",
            "--query",
            "中文 %_",
            "--json",
        ]))
        .unwrap();
        assert!(p.search && p.json);
        assert_eq!(p.limit, 5);
        let p = parse(&args(&[
            "get",
            "--session",
            "s",
            "--entry",
            "e",
            "--offset",
            "4",
            "--limit",
            "8",
        ]))
        .unwrap();
        assert_eq!((p.offset, p.limit), (4, 8));
        for values in [
            vec!["search", "--query", "x"],
            vec!["get", "--session", "s"],
            vec!["get", "--session", "s", "--entry", "e", "--offset", "-1"],
            vec!["search", "--session", "s", "--query", "x", "--limit", "21"],
            vec!["get", "--session", "s", "--entry", "e", "--limit", "3"],
            vec![
                "search",
                "--session",
                "s",
                "--session",
                "other",
                "--query",
                "x",
            ],
            vec!["search", "--session", "s", "--entry", "e"],
            vec!["get", "--session", "s", "--entry", "e", "--unknown", "x"],
        ] {
            assert!(parse(&args(&values)).is_err(), "{values:?}");
        }
    }
    #[test]
    fn formatting_keeps_refs_and_continuation_cursor() {
        let v = serde_json::json!({"sessionId":"s","entryId":"e","role":"tool","totalBytes":20,"chunks":[{"blockIndex":0,"kind":"tool_result","field":"text","blockByteOffset":0,"text":"你好"}],"hasMore":true,"nextOffset":6,"omittedKinds":["reasoning"]});
        let result = format_result(&v, false);
        assert!(result.contains("你好"));
        assert!(result.contains("--offset 6"));
        assert!(result.contains("reasoning"));
        assert_eq!(
            format_result(&serde_json::json!({"matches":[]}), true),
            "No matching history records."
        );
    }
    #[tokio::test]
    async fn cli_history_calls_are_scoped_and_preserve_query_and_cursor() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::respond(
            "search_session_history",
            r#"{"sessionId":"s","matches":[{"entryId":"e","snippet":"中文","blockIndex":0,"kind":"tool_result","byteOffset":2}],"hasMore":false}"#,
        );
        agent.responses.insert(
            "get_session_history_entry".into(),
            r#"{"entryId":"e","chunks":[{"text":"中文"}],"hasMore":false}"#.into(),
        );
        let address = crate::test_server::spawn_mock(agent.clone()).await;
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from(address),
        )]);
        let (out, cap) = Output::memory();
        crate::commands::session::session(
            Some("history"),
            &args(&[
                "search",
                "--session",
                "s",
                "--query",
                "中文 %_",
                "--limit",
                "2",
                "--json",
            ]),
            &out,
        )
        .await
        .unwrap();
        let v: Value = serde_json::from_slice(&cap.out.lock().unwrap()).unwrap();
        assert_eq!(v["matches"][0]["entryId"], "e");
        let seen = agent.seen_of("search_session_history");
        assert_eq!(seen[0].session_id, "s");
        assert_eq!(seen[0].message, "中文 %_");
        assert_eq!(seen[0].limit, Some(2));
        let (out, cap) = Output::memory();
        run(
            &args(&[
                "get",
                "--session",
                "s",
                "--entry",
                "e",
                "--offset",
                "2",
                "--limit",
                "16",
                "--json",
            ]),
            &out,
        )
        .await
        .unwrap();
        let v: Value = serde_json::from_slice(&cap.out.lock().unwrap()).unwrap();
        assert_eq!(v["chunks"][0]["text"], "中文");
        let seen = agent.seen_of("get_session_history_entry");
        assert_eq!(seen[0].entry_id, "e");
        assert_eq!(seen[0].session_id, "s");
        assert_eq!(seen[0].offset, Some(2));
        assert_eq!(seen[0].limit, Some(16));
    }
    #[tokio::test]
    async fn help_does_not_require_a_server() {
        let (out, cap) = Output::memory();
        run(&args(&["search", "--help"]), &out).await.unwrap();
        assert!(String::from_utf8(cap.out.lock().unwrap().clone())
            .unwrap()
            .contains("history search"));
    }

    /// The subcommand is the first argument and nothing else is accepted: a
    /// missing or unknown one is a usage error, not a default read.
    #[test]
    fn a_missing_or_unknown_subcommand_is_refused() {
        for values in [vec![], vec!["--json"], vec!["list"], vec!["searchx"]] {
            assert_eq!(
                parse(&args(&values)).unwrap_err(),
                "expected history search or get",
                "{values:?}"
            );
        }
    }

    /// A flag whose value is missing *or* is itself another flag is a missing
    /// value, so `--query --json` never searches for the literal `--json`.
    #[test]
    fn a_dangling_or_flag_shaped_value_is_a_missing_value() {
        for values in [
            vec!["search", "--session"],
            vec!["search", "--session", "s", "--query"],
            vec!["search", "--session", "s", "--query", "--json"],
            vec!["get", "--session", "s", "--entry", "--json"],
            vec!["search", "--session", "", "--query", "x"],
        ] {
            let err = parse(&args(&values)).unwrap_err();
            assert!(err.contains("missing value for"), "{values:?} → {err}");
        }
    }

    /// The query boundary: 200 characters is the last accepted length, 201 the
    /// first refused, a blank query is refused even though it is short, and a
    /// NUL is refused before anything reaches the agent.
    #[test]
    fn the_query_length_and_content_boundary_is_exact() {
        let ok = "a".repeat(200);
        assert!(parse(&args(&["search", "--session", "s", "--query", &ok])).is_ok());
        let long = "a".repeat(201);
        assert!(parse(&args(&["search", "--session", "s", "--query", &long])).is_err());
        // An all-whitespace or NUL-bearing query is short but still illegitimate;
        // the empty string never gets this far (it is a missing value first).
        for bad in ["   ", "\t", "with\0nul"] {
            let err = parse(&args(&["search", "--session", "s", "--query", bad])).unwrap_err();
            assert_eq!(
                err, "query must contain 1..200 characters without NUL",
                "{bad:?}"
            );
        }
        // 200 CJK characters are 200 *characters*, not 600 bytes.
        let cjk = "中".repeat(200);
        assert!(parse(&args(&["search", "--session", "s", "--query", &cjk])).is_ok());
        // `--entry` has no length rule (it is an id, not a query).
        assert!(parse(&args(&["get", "--session", "s", "--entry", "e"])).is_ok());
    }

    /// A search result is rendered one block per match, with the continuation
    /// hint only when `hasMore` is set — and no hint when the server says it is
    /// the last page.
    #[test]
    fn the_search_table_renders_every_match_and_the_continuation_hint() {
        let v = serde_json::json!({
            "matches": [
                {"entryId":"e1","blockIndex":0,"kind":"text","byteOffset":0,"snippet":"first"},
                {"entryId":"e2","blockIndex":3,"kind":"tool_result","byteOffset":17,"snippet":"第二个 匹配"},
            ],
            "hasMore": true
        });
        let text = format_result(&v, true);
        assert!(
            text.contains("entry=e1 block=0 kind=text offset=0\nfirst"),
            "{text}"
        );
        assert!(
            text.contains("entry=e2 block=3 kind=tool_result offset=17\n第二个 匹配"),
            "{text}"
        );
        assert!(
            text.contains("\n\n"),
            "matches are separated by a blank line: {text}"
        );
        assert!(
            text.contains("More matches exist; refine the query or increase --limit (max 20)."),
            "{text}"
        );
        let text = format_result(
            &serde_json::json!({"matches":[{"entryId":"e"}],"hasMore":false}),
            true,
        );
        assert!(text.contains("entry=e"), "{text}");
        assert!(!text.contains("More matches exist"), "{text}");
    }

    /// A successful search renders as text (not JSON) when `--json` is absent,
    /// through the same `format_result` the tests above pin. The agent is the
    /// in-process mock, so the whole `run` path is exercised.
    #[tokio::test]
    async fn a_search_without_json_prints_the_rendered_table() {
        let _guard = crate::test_env::lock_env().await;
        let agent = crate::test_server::MockAgent::respond(
            "search_session_history",
            r#"{"matches":[{"entryId":"e9","blockIndex":1,"kind":"text","byteOffset":4,"snippet":"中文 snippet"}],"hasMore":true}"#,
        );
        let address = crate::test_server::spawn_mock(agent).await;
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from(address),
        )]);
        let (out, cap) = Output::memory();
        run(&args(&["search", "--session", "s", "--query", "中"]), &out)
            .await
            .expect("search succeeds");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(
            stdout.contains("entry=e9 block=1 kind=text offset=4"),
            "{stdout}"
        );
        assert!(stdout.contains("中文 snippet"), "{stdout}");
        assert!(stdout.contains("More matches exist"), "{stdout}");
        assert!(!stdout.trim_start().starts_with('{'), "not JSON: {stdout}");
    }
}
