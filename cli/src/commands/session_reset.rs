//! `future session reset` — in-place history reset.
//!
//! The user-invoked counterpart of compaction, for when a session's own history
//! is the problem: it deletes the turns and the entry kinds the caller names,
//! instead of projecting them. Everything about the interface follows from that
//! being destructive: the default is a dry run, applying needs `--yes`, and the
//! Agent writes a lossless backup of the complete pre-reset history first.
//!
//! Deliberately *not* the same shape as `future session compact`: that command
//! acknowledges asynchronously because it spends a model call. This one has no
//! model call and the caller needs the counts back to judge the result, so it is
//! one bounded round trip. What it shares with compaction is the invariant that
//! only the Agent writes history — this command never touches `agent.db`.

use super::session::human_tokens;
use crate::output::Output;
use crate::rpc::{grpc_addr, RunClient};
use future_rpc::proto::HistoryReset;
use serde_json::Value;

const HELP: &str = "future session reset — delete part of a session's own history, in place

Usage:
  future session reset <session-id> [selectors] [--dry-run | --yes] [--json]

Selectors (combinable; at least one is required):
  --all                 Drop every turn, keeping only the session record
                        (id, name, cwd, model, prompts, permissions)
  --keep-first <n>      Keep the first n user turns
  --keep-last <n>       Keep the last n user turns (combines with --keep-first:
                        keep a head and a tail, drop the middle)
  --drop <kinds>        Remove these from the part that is kept, comma-separated:
                          thinking       reasoning blocks (and the legacy field)
                          tool-calls     tool_call blocks on assistant entries
                          tool-results   the tool entries that answer them
                          attachments    attached files/images and their manifest
                          compaction     compaction checkpoints
                        Repeatable; an unknown kind is an error, never ignored.

A user turn is one user message plus everything up to the next user message —
the same boundary `future session forks` shows.

Options:
  --dry-run             Print what would be deleted and kept; change nothing
                        (this is the default)
  --yes                 Apply the reset
  --json                Print the Agent's report as JSON
  -h, --help            Show this help

Safety: with --yes the Agent writes a lossless JSON backup of the session's
complete entry list first and prints its path, so a wrong call is recoverable
(the file is a full copy, not a summary; a future `--restore` reads it back).
An active run or a running compaction is refused rather than interrupted, and
the session record is never modified: only entries are removed.

Why this exists: automatic compaction must never drop a user original, so a
session whose user turns alone exceed the compaction ceiling (128 000 tokens,
`compaction/budget.rs`) reports `context compaction budget exceeded` and cannot
recover on its own. This is the tool that lifts it — `--keep-last 2` or
`--drop thinking,tool-calls,tool-results` is usually enough.

Examples:
  future session reset <id> --keep-last 3                  # preview
  future session reset <id> --keep-last 3 --yes             # apply
  future session reset <id> --drop thinking,tool-results --yes
  future session reset <id> --all --yes                     # keep the session, drop its history";

#[derive(Debug, PartialEq, Eq)]
struct Options {
    session: String,
    all: bool,
    keep_first: Option<u64>,
    keep_last: Option<u64>,
    drop: Vec<String>,
    apply: bool,
    json: bool,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut session = None;
    let mut all = false;
    let mut keep_first = None;
    let mut keep_last = None;
    let mut drop: Vec<String> = Vec::new();
    let mut apply = false;
    let mut dry_run = false;
    let mut json = false;
    let mut seen: Vec<&str> = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        match flag {
            "--json" => {
                json = true;
                index += 1;
            }
            "--all" => {
                all = true;
                index += 1;
            }
            "--yes" => {
                apply = true;
                index += 1;
            }
            "--dry-run" => {
                dry_run = true;
                index += 1;
            }
            "--keep-first" | "--keep-last" | "--drop" => {
                // `--drop` is repeatable (its value is a list); the keep bounds
                // are not, so a second one is a mistake worth reporting.
                if flag != "--drop" {
                    if seen.contains(&flag) {
                        return Err(format!("duplicate option: {flag}"));
                    }
                    seen.push(flag);
                }
                let value = args
                    .get(index + 1)
                    .filter(|value| !value.starts_with("--"))
                    .ok_or_else(|| format!("missing value for {flag}"))?;
                match flag {
                    "--keep-first" | "--keep-last" => {
                        let count: u64 = value
                            .parse()
                            .map_err(|_| format!("{flag} expects a whole number, got {value}"))?;
                        if flag == "--keep-first" {
                            keep_first = Some(count);
                        } else {
                            keep_last = Some(count);
                        }
                    }
                    _ => {
                        for kind in value.split(',').map(str::trim).filter(|k| !k.is_empty()) {
                            drop.push(kind.to_string());
                        }
                    }
                }
                index += 2;
            }
            _ if flag.starts_with("--") => {
                return Err(format!(
                    "unknown option: {flag}; see future session reset --help"
                ))
            }
            _ => {
                if session.is_some() {
                    return Err(format!("unexpected argument: {flag}"));
                }
                session = Some(flag.to_string());
                index += 1;
            }
        }
    }
    if apply && dry_run {
        return Err("--yes and --dry-run are mutually exclusive".into());
    }
    let options = Options {
        session: session.ok_or("Usage: future session reset <session-id> [selectors]")?,
        all,
        keep_first,
        keep_last,
        drop,
        apply,
        json,
    };
    // The Agent validates these too (it is the authority on the kinds); failing
    // here just avoids a round trip and a confusing partial command.
    if !options.all
        && options.keep_first.is_none()
        && options.keep_last.is_none()
        && options.drop.is_empty()
    {
        return Err(
            "nothing selected: pass --all, --keep-first N, --keep-last N or --drop <kinds>".into(),
        );
    }
    if options.all && (options.keep_first.is_some() || options.keep_last.is_some()) {
        return Err("--all cannot be combined with --keep-first or --keep-last".into());
    }
    Ok(options)
}

fn selectors(options: &Options) -> HistoryReset {
    HistoryReset {
        apply: options.apply,
        all: options.all.then_some(true),
        keep_first: options.keep_first.map(|value| value as i64),
        keep_last: options.keep_last.map(|value| value as i64),
        drop: options.drop.clone(),
    }
}

fn selectors_line(options: &Options) -> String {
    let mut parts = Vec::new();
    if options.all {
        parts.push("--all".to_string());
    }
    if let Some(count) = options.keep_first {
        parts.push(format!("--keep-first {count}"));
    }
    if let Some(count) = options.keep_last {
        parts.push(format!("--keep-last {count}"));
    }
    if !options.drop.is_empty() {
        parts.push(format!("--drop {}", options.drop.join(",")));
    }
    parts.join("  ")
}

fn preview(entry: &Value) -> String {
    let id = entry["id"].as_str().unwrap_or("");
    let kind = entry["kind"].as_str().unwrap_or("");
    let text = entry["text"].as_str().unwrap_or("");
    format!("{kind} {id}  {text}")
}

pub(super) async fn run(args: &[String], out: &Output) -> Result<(), String> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        out.log(HELP);
        return Ok(());
    }
    let options = parse(args)?;
    let report = RunClient::new(&grpc_addr())
        .reset_session_history(&options.session, &selectors(&options))
        .await?;
    if options.json {
        out.log(&serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?);
        return Ok(());
    }
    let applied = report["applied"].as_bool().unwrap_or(false);
    let number = |value: &Value| value.as_u64().unwrap_or(0);
    out.log(&format!(
        "future session reset — {} for {}\n",
        if applied { "applied" } else { "dry run" },
        options.session
    ));
    out.log(&format!("Selectors  {}\n", selectors_line(&options)));
    let before_entries = number(&report["entriesBefore"]);
    let after_entries = number(&report["entriesAfter"]);
    let before_turns = number(&report["turnsBefore"]);
    let after_turns = number(&report["turnsKept"]);
    let before_tokens = number(&report["estimatedTokensBefore"]);
    let after_tokens = number(&report["estimatedTokensAfter"]);
    out.log(&format!(
        "Entries    {before_entries} → {after_entries}   ({})",
        if before_entries >= after_entries {
            format!("{} removed", before_entries - after_entries)
        } else {
            "unchanged".to_string()
        }
    ));
    out.log(&format!("Turns      {before_turns} → {after_turns}\n"));
    out.log(&format!(
        "History    ~{} → ~{} tokens (estimated by the compactor's estimator; the live\n           context figure only moves after the next request)\n",
        human_tokens(before_tokens as i64),
        human_tokens(after_tokens as i64)
    ));
    if let Some(effects) = report["dropEffects"]
        .as_array()
        .filter(|list| !list.is_empty())
    {
        out.log("Dropped by kind");
        for effect in effects {
            out.log(&format!(
                "  {:<14} {} entry(ies), {} block(s)",
                effect["kind"].as_str().unwrap_or(""),
                number(&effect["entries"]),
                number(&effect["blocks"])
            ));
        }
        out.log("");
    }
    for label in ["firstKept", "lastKept"] {
        if let Some(entry) = report[label].as_object() {
            out.log(&format!(
                "{} {}",
                if label == "firstKept" {
                    "First kept"
                } else {
                    "Last kept "
                },
                preview(&Value::Object(entry.clone()))
            ));
        }
    }
    if report["firstKept"].is_object() {
        out.log("");
    }
    for note in report["notes"].as_array().into_iter().flatten() {
        out.log(&format!("Note       {}", note.as_str().unwrap_or("")));
    }
    match report["backup"].as_str() {
        Some(backup) => out.log(&format!(
            "\nBackup     {backup}\n           Complete pre-reset history (JSON, lossless); restore it later by\n           writing its `entries` array back through the Agent."
        )),
        None => out.log(
            "\nDry run: nothing was changed. Re-run the same command with --yes to apply\n(a lossless backup of the full history is written first).",
        ),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn selectors_parse_and_combine() {
        assert_eq!(
            parse(&args(&[
                "s1",
                "--keep-first",
                "2",
                "--keep-last",
                "3",
                "--drop",
                "thinking,tool-results",
                "--yes"
            ]))
            .unwrap(),
            Options {
                session: "s1".into(),
                all: false,
                keep_first: Some(2),
                keep_last: Some(3),
                drop: vec!["thinking".into(), "tool-results".into()],
                apply: true,
                json: false,
            }
        );
        assert!(
            parse(&args(&["s1", "--all"])).unwrap().all,
            "a bare selector is a dry run"
        );
        // --drop is repeatable, and each occurrence may carry a list.
        assert_eq!(
            parse(&args(&[
                "s1",
                "--drop",
                "thinking",
                "--drop",
                "attachments"
            ]))
            .unwrap()
            .drop,
            vec!["thinking".to_string(), "attachments".to_string()]
        );
    }

    #[test]
    fn usage_errors_are_explicit() {
        for (input, expected) in [
            (vec!["s1"], "nothing selected"),
            (vec!["s1", "--keep-last"], "missing value for --keep-last"),
            (vec!["s1", "--keep-last", "two"], "expects a whole number"),
            (
                vec!["s1", "--all", "--keep-last", "1"],
                "cannot be combined",
            ),
            (vec!["s1", "--yes", "--dry-run"], "mutually exclusive"),
            (
                vec!["s1", "--keep-last", "1", "--keep-last", "2"],
                "duplicate option",
            ),
            (vec!["s1", "--force"], "unknown option: --force"),
            (vec!["s1", "s2", "--all"], "unexpected argument"),
            (vec!["--all"], "Usage: future session reset"),
        ] {
            let error = parse(&args(&input)).unwrap_err();
            assert!(error.contains(expected), "{input:?} → {error}");
        }
        // A flag-shaped value is a missing value, not a session id.
        assert!(parse(&args(&["s1", "--drop", "--yes"]))
            .unwrap_err()
            .contains("missing value for --drop"));
    }

    #[tokio::test]
    async fn help_needs_no_server() {
        let (out, captured) = Output::memory();
        run(&args(&["--help"]), &out).await.unwrap();
        let help = String::from_utf8(captured.out.lock().unwrap().clone()).unwrap();
        assert!(help.contains("--keep-last"));
        assert!(help.contains("lossless JSON backup"));
        assert!(help.contains("context compaction budget exceeded"));
    }

    #[tokio::test]
    async fn dispatch_sends_the_selectors_and_prints_the_report() {
        let _guard = crate::test_env::lock_env().await;
        let agent = crate::test_server::MockAgent::respond(
            "reset_session_history",
            r#"{"sessionId":"s1","applied":false,"selectors":{"all":false,"keepFirst":null,"keepLast":2,"drop":["thinking"]},
                "entriesBefore":42,"entriesAfter":12,"turnsBefore":9,"turnsKept":2,
                "estimatedTokensBefore":131072,"estimatedTokensAfter":4096,
                "dropEffects":[{"kind":"thinking","entries":7,"blocks":7}],
                "firstKept":{"id":"info-1","kind":"session_info","role":"system","text":"(session_info entry)"},
                "lastKept":{"id":"a9","kind":"assistant","role":"assistant","text":"latest answer"},
                "notes":[],"backup":null,"dryRun":true}"#,
        );
        let address = crate::test_server::spawn_mock(agent.clone()).await;
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from(address),
        )]);
        let (out, captured) = Output::memory();
        run(
            &args(&["s1", "--keep-last", "2", "--drop", "thinking"]),
            &out,
        )
        .await
        .unwrap();
        let prose = String::from_utf8(captured.out.lock().unwrap().clone()).unwrap();
        assert!(prose.contains("dry run for s1"), "{prose}");
        assert!(prose.contains("--keep-last 2"), "{prose}");
        assert!(prose.contains("Entries    42 → 12"), "{prose}");
        assert!(prose.contains("thinking"), "{prose}");
        assert!(prose.contains("Latest") || prose.contains("a9"), "{prose}");
        assert!(prose.contains("--yes"), "{prose}");

        let seen = agent.seen_of("reset_session_history");
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].session_id, "s1");
        let sent = seen[0].reset_history.as_ref().expect("typed selectors");
        assert!(!sent.apply, "the CLI never applies without --yes");
        assert_eq!(sent.keep_last, Some(2));
        assert_eq!(sent.drop, vec!["thinking".to_string()]);
        assert_eq!(sent.keep_first, None);
        assert!(agent.seen_of("switch_session").is_empty());
    }

    #[tokio::test]
    async fn apply_and_json_pass_through_what_the_agent_reports() {
        let _guard = crate::test_env::lock_env().await;
        let agent = crate::test_server::MockAgent::respond(
            "reset_session_history",
            r#"{"sessionId":"s1","applied":true,"selectors":{"all":true,"keepFirst":null,"keepLast":null,"drop":[]},
                "entriesBefore":42,"entriesAfter":1,"turnsBefore":9,"turnsKept":0,
                "estimatedTokensBefore":131072,"estimatedTokensAfter":64,
                "dropEffects":[],"firstKept":null,"lastKept":null,
                "notes":["compaction checkpoints cover turns this reset drops, so they are removed with them"],
                "backup":"/home/u/.future/agent/session-backups/s1-20261016T120000.000Z.json"}"#,
        );
        let address = crate::test_server::spawn_mock(agent.clone()).await;
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from(address),
        )]);
        let (out, captured) = Output::memory();
        run(&args(&["s1", "--all", "--yes", "--json"]), &out)
            .await
            .unwrap();
        let value: Value =
            serde_json::from_slice(&captured.out.lock().unwrap()).expect("json output");
        assert_eq!(value["applied"], true);
        assert_eq!(value["entriesAfter"], 1);
        assert!(value["backup"]
            .as_str()
            .unwrap()
            .contains("session-backups"));

        let (out, captured) = Output::memory();
        run(&args(&["s1", "--all", "--yes"]), &out).await.unwrap();
        let prose = String::from_utf8(captured.out.lock().unwrap().clone()).unwrap();
        assert!(prose.contains("applied for s1"), "{prose}");
        assert!(prose.contains("Backup"), "{prose}");
        assert!(prose.contains("session-backups"), "{prose}");
        assert!(prose.contains("Note"), "{prose}");
        let sent = agent.seen_of("reset_session_history");
        assert_eq!(sent.len(), 2);
        assert!(sent[1].reset_history.as_ref().unwrap().apply);
    }

    #[tokio::test]
    async fn a_refused_reset_is_an_error_not_a_report() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::default();
        agent.fail_with.insert(
            "reset_session_history".into(),
            "unknown drop kind `thnking`; known kinds: thinking, tool-calls, tool-results, attachments, compaction".into(),
        );
        let address = crate::test_server::spawn_mock(agent).await;
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from(address),
        )]);
        let (out, captured) = Output::memory();
        let error = run(&args(&["s1", "--drop", "thnking", "--yes"]), &out)
            .await
            .unwrap_err();
        assert!(error.contains("unknown drop kind"), "{error}");
        assert!(captured.out.lock().unwrap().is_empty());
    }
}
