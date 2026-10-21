//! `future session prune` — delete every session idle longer than an age.
//!
//! `future session delete` takes one id and `future session list` only shows
//! what exists now, so cleaning up conversations abandoned months ago meant
//! scraping the list and looping over it by hand. This is that loop with the
//! two things a destructive one-shot needs: an explicit age threshold and a
//! dry run by default. The Agent stays the authority on what may be deleted —
//! a session with an active run is refused there, not here — and this command
//! reports that refusal per session instead of treating the batch as one
//! transaction.
//!
//! The age comes from the session's own history, not from the summary's
//! `updatedAtMs`: that value moves on any write, so a rename — or one storage
//! upgrade rewriting every session's metadata snapshot — makes a dormant
//! conversation read as freshly used. The summary is still the cheap filter
//! (its value is never older than the conversation), and only the sessions it
//! reports inside the window need one bounded backward history page each.

use super::session::{now_ms, pad_end, relative_age, truncate};
use crate::output::Output;
use crate::rpc::{grpc_addr, RunClient};
use future_rpc::session_deletion::MAX_DELETE_SESSIONS;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::HashMap;

const HELP: &str = "future session prune — delete sessions that have been idle for a while

Usage:
  future session prune --older-than <age> [--dry-run | --yes] [--keep <id>] [--json]

Options:
  --older-than <age>  How long a session must have been idle to qualify, as
                      <number><unit>: m (minutes), h (hours), d (days), w
                      (weeks) — e.g. 90m, 12h, 3d, 2w.
                      Idle is measured from the session's newest history entry
                      (the last user turn and what came with it), not from the
                      file's mtime and not from `session list`'s UPDATED: a
                      metadata write — a rename, or a storage upgrade that
                      rewrites every session's snapshot — moves that value
                      without anybody using the conversation. The two only
                      differ for sessions the Agent has saved since they were
                      last used.
  --keep <id>         Never delete this session. Repeatable.
  --dry-run           Print what would be deleted; change nothing (this is
                      the default).
  --yes               Delete the matching sessions.
  --json              Print the report as JSON (preview and apply alike).
  -h, --help          Show this help

Safety: a session with an active run is skipped, never interrupted, and the
Agent also refuses one that starts a run or a compaction between the listing
and the deletion — that refusal is reported per session, so the rest of the
batch still goes. A session whose age cannot be established (no history and no
timestamp) is left alone.

Examples:
  future session prune --older-than 3d             # preview
  future session prune --older-than 3d --yes       # apply
  future session prune --older-than 12h --keep 20261020-abc --yes";

#[derive(Debug, PartialEq, Eq)]
struct Options {
    /// `--older-than` as written, echoed back in the report.
    older_than: String,
    older_than_ms: i64,
    keep: Vec<String>,
    apply: bool,
    json: bool,
}

/// Milliseconds in one `<age>` unit. A closed set on purpose: an age decides
/// whether something is deleted, so an unrecognised unit has to be an error
/// rather than a silent default.
fn unit_ms(unit: &str) -> Option<i64> {
    match unit {
        "m" | "min" | "mins" | "minute" | "minutes" => Some(60_000),
        "h" | "hr" | "hrs" | "hour" | "hours" => Some(3_600_000),
        "d" | "day" | "days" => Some(86_400_000),
        "w" | "week" | "weeks" => Some(604_800_000),
        _ => None,
    }
}

/// `<number><unit>` → milliseconds. The unit is required: a bare number would
/// have to mean something (days? seconds?) and either guess is a surprise on a
/// command that deletes.
fn parse_age_ms(raw: &str) -> Result<i64, String> {
    let text = raw.trim();
    let digits_end = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let (digits, unit) = text.split_at(digits_end);
    if digits.is_empty() {
        return Err(format!(
            "--older-than expects <number><unit> (e.g. 3d), got {raw:?}"
        ));
    }
    let count: i64 = digits
        .parse()
        .map_err(|_| format!("--older-than {raw:?} is not a whole number of units"))?;
    let unit = unit.trim().to_ascii_lowercase();
    let scale = unit_ms(&unit).ok_or_else(|| {
        format!("unknown unit {unit:?} in --older-than {raw:?}; use m, h, d or w")
    })?;
    let ms = count
        .checked_mul(scale)
        .ok_or_else(|| format!("--older-than {raw:?} is too large"))?;
    if ms <= 0 {
        return Err("--older-than must be greater than zero".into());
    }
    Ok(ms)
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut older_than: Option<(String, i64)> = None;
    let mut keep: Vec<String> = Vec::new();
    let mut apply = false;
    let mut dry_run = false;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        match flag {
            "--json" => {
                json = true;
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
            "--older-than" | "--keep" => {
                let value = args
                    .get(index + 1)
                    .filter(|value| !value.starts_with("--"))
                    .ok_or_else(|| format!("missing value for {flag}"))?;
                if flag == "--older-than" {
                    if older_than.is_some() {
                        return Err("duplicate option: --older-than".into());
                    }
                    older_than = Some((value.clone(), parse_age_ms(value)?));
                } else {
                    keep.push(value.clone());
                }
                index += 2;
            }
            _ if flag.starts_with("--") => {
                return Err(format!(
                    "unknown option: {flag}; see future session prune --help"
                ))
            }
            _ => return Err(format!("unexpected argument: {flag}")),
        }
    }
    if apply && dry_run {
        return Err("--yes and --dry-run are mutually exclusive".into());
    }
    let (label, ms) = older_than.ok_or(
        "Usage: future session prune --older-than <age> [--dry-run | --yes]\n\
         e.g. future session prune --older-than 3d",
    )?;
    Ok(Options {
        older_than: label,
        older_than_ms: ms,
        keep,
        apply,
        json,
    })
}

/// `ms` as local wall-clock time. The threshold is what a person compares
/// against their own memory of when they last used a conversation, so it is
/// printed in local time (and only in the human report; JSON carries `cutoffMs`).
fn format_local(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|utc| {
            utc.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn title_of(session: &Value) -> String {
    let name = session
        .get("sessionName")
        .and_then(Value::as_str)
        .unwrap_or("");
    let first = session
        .get("firstMessage")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !name.is_empty() {
        truncate(name, 48)
    } else if !first.is_empty() {
        truncate(first, 48)
    } else {
        "(untitled)".to_string()
    }
}

fn session_json(session: &Value, activity_ms: i64, now: i64) -> Value {
    json!({
        "id": session["id"].as_str().unwrap_or_default(),
        // The number the decision was made on, and the one `session list`
        // reports: they differ when the Agent's recency came from a metadata
        // write rather than from the conversation.
        "updatedAtMs": activity_ms,
        "listedUpdatedAtMs": session["updatedAtMs"].as_i64().unwrap_or(0),
        "age": relative_age(activity_ms, now),
        "title": title_of(session),
    })
}

fn noun(count: usize) -> &'static str {
    if count == 1 {
        "session"
    } else {
        "sessions"
    }
}

/// One matched session and the activity time the decision was made on.
struct Match {
    session: Value,
    activity_ms: i64,
}

/// Everything one prune run needs to report, gathered before anything is
/// printed so the JSON and the text form cannot disagree.
struct Report {
    dry_run: bool,
    scanned: usize,
    matched: Vec<Match>,
    skipped: Vec<(String, &'static str)>,
    kept: Vec<String>,
    deleted: Vec<String>,
    failed: Vec<(String, String)>,
}

/// How many sessions are probed at once. One bounded backward page per
/// session, so this is a handful of round trips rather than a batch read.
const ACTIVITY_READ_CONCURRENCY: usize = 8;

/// Below this many sessions the scan is over before a notice is worth printing.
const SCAN_NOTICE_THRESHOLD: usize = 20;

/// Newest timestamp in one bounded backward page of a session's display
/// history. The `session_info` snapshot is skipped: it is metadata, rewritten
/// without the conversation moving, so it says when the Agent last *saved* the
/// session, not when somebody last used it.
fn newest_activity(page: &Value) -> Option<i64> {
    page.get("entries")?
        .as_array()?
        .iter()
        .filter(|entry| entry.get("kind").and_then(Value::as_str) != Some("session_info"))
        .filter_map(|entry| entry.get("createdAtMs").and_then(Value::as_i64))
        .filter(|ms| *ms > 0)
        .max()
}

/// The last user exchange of a session, as a single entry timestamp. `None`
/// means the history could not be read — never a reason to delete by itself.
async fn last_activity_ms(client: &RunClient, session_id: &str) -> Option<i64> {
    match client.get_session_entries_tail(session_id, 1).await {
        Ok(page) => newest_activity(&page),
        Err(_) => None,
    }
}

pub(super) async fn run(args: &[String], out: &Output) -> Result<(), String> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        out.log(HELP);
        return Ok(());
    }
    let options = parse(args)?;
    let client = RunClient::new(&grpc_addr());
    // Enumeration errors propagate: "the agent could not list sessions" must
    // never read as "there is nothing to delete".
    let data = client.list_sessions().await?;
    let sessions: Vec<Value> = data
        .get("sessions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let now = now_ms();
    let cutoff = now - options.older_than_ms;

    let mut matched: Vec<Match> = Vec::new();
    let mut skipped: Vec<(String, &'static str)> = Vec::new();
    let mut kept: Vec<String> = Vec::new();
    // Sessions whose reported recency is inside the window: their real age has
    // to be read from the history, because the Agent's `updatedAtMs` also
    // moves when only the session's metadata is written.
    let mut pending: Vec<Value> = Vec::new();
    for session in &sessions {
        let id = session
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if id.is_empty() {
            continue;
        }
        if options.keep.iter().any(|keep| keep == &id) {
            kept.push(id);
            continue;
        }
        let stored = session
            .get("updatedAtMs")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if session
            .get("isStreaming")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            // An active run is never interrupted. It is only worth reporting
            // when the session would otherwise have matched.
            if stored > 0 && stored <= cutoff {
                skipped.push((id, "active run"));
            }
            continue;
        }
        // The stored value is the newest entry timestamp, so it is never older
        // than the conversation: before the cutoff means the session is idle
        // however the metadata moved, and no history read is needed.
        if stored > 0 && stored <= cutoff {
            matched.push(Match {
                session: session.clone(),
                activity_ms: stored,
            });
            continue;
        }
        pending.push(session.clone());
    }

    if !options.json && pending.len() >= SCAN_NOTICE_THRESHOLD {
        out.log_err(&format!(
            "Reading the last activity of {} sessions…",
            pending.len()
        ));
    }
    let client_ref = &client;
    let mut activities: HashMap<String, Option<i64>> = futures_util::stream::iter(
        pending
            .iter()
            .map(|session| {
                let id = session["id"].as_str().unwrap_or_default().to_string();
                let client = client_ref;
                async move {
                    let activity = last_activity_ms(client, &id).await;
                    (id, activity)
                }
            })
            .collect::<Vec<_>>(),
    )
    .buffer_unordered(ACTIVITY_READ_CONCURRENCY)
    .collect()
    .await;

    for session in pending {
        let id = session["id"].as_str().unwrap_or_default().to_string();
        let stored = session["updatedAtMs"].as_i64().unwrap_or(0);
        // No readable history and no timestamp means the age cannot be
        // established — never a reason to delete.
        let activity = activities
            .remove(&id)
            .flatten()
            .or(Some(stored).filter(|ms| *ms > 0));
        let Some(activity) = activity else { continue };
        if activity > cutoff {
            continue;
        }
        matched.push(Match {
            session,
            activity_ms: activity,
        });
    }
    matched.sort_by_key(|matched| matched.activity_ms);

    let ids: Vec<String> = matched
        .iter()
        .filter_map(|matched| matched.session["id"].as_str())
        .map(str::to_string)
        .collect();

    let mut deleted = Vec::new();
    let mut failed = Vec::new();
    if options.apply && !ids.is_empty() {
        for chunk in ids.chunks(MAX_DELETE_SESSIONS) {
            let response = match client.delete_sessions(chunk).await {
                Ok(response) => response,
                // The Agent answers per session, so a whole-batch failure is
                // the transport or a rejected request: no session in this
                // chunk was deleted, and the ones already reported as deleted
                // stay reported.
                Err(error) => {
                    failed.extend(chunk.iter().map(|id| (id.clone(), error.clone())));
                    break;
                }
            };
            for result in response
                .get("results")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
            {
                let id = result["sessionId"].as_str().unwrap_or_default().to_string();
                if result["deleted"].as_bool().unwrap_or(false) {
                    deleted.push(id);
                } else {
                    let message = result["error"]
                        .as_str()
                        .filter(|message| !message.is_empty())
                        .unwrap_or("deletion failed")
                        .to_string();
                    failed.push((id, message));
                }
            }
        }
    }

    let report = Report {
        dry_run: !options.apply,
        scanned: sessions.len(),
        matched,
        skipped,
        kept,
        deleted,
        failed,
    };

    if options.json {
        let body = json!({
            "dryRun": report.dry_run,
            "olderThan": options.older_than,
            "olderThanMs": options.older_than_ms,
            "cutoffMs": cutoff,
            "scanned": report.scanned,
            "matched": report
                .matched
                .iter()
                .map(|matched| session_json(&matched.session, matched.activity_ms, now))
                .collect::<Vec<_>>(),
            "skipped": report
                .skipped
                .iter()
                .map(|(id, reason)| json!({"id": id, "reason": reason}))
                .collect::<Vec<_>>(),
            "kept": report.kept,
            "deleted": report.deleted,
            "failed": report
                .failed
                .iter()
                .map(|(id, error)| json!({"id": id, "error": error}))
                .collect::<Vec<_>>(),
        });
        out.log(&serde_json::to_string_pretty(&body).map_err(|error| error.to_string())?);
    } else {
        out.log(&format!(
            "future session prune — {}\n",
            if report.dry_run {
                format!(
                    "dry run, {} {} would be deleted",
                    report.matched.len(),
                    noun(report.matched.len())
                )
            } else {
                format!(
                    "{} of {} {} deleted",
                    report.deleted.len(),
                    report.matched.len(),
                    noun(report.matched.len())
                )
            }
        ));
        out.log(&format!(
            "Older than  {}  (last activity before {})",
            options.older_than,
            format_local(cutoff)
        ));
        out.log(&format!(
            "Matched     {} of {} {}\n",
            report.matched.len(),
            report.scanned,
            noun(report.scanned)
        ));
        if report.matched.is_empty() {
            out.log(&format!(
                "No session has been idle longer than {}; nothing to delete.",
                options.older_than
            ));
        } else {
            out.log(&format!(
                "  {} {} TITLE",
                pad_end("SESSION ID", 46),
                pad_end("AGE", 9)
            ));
            for matched in &report.matched {
                let session = &matched.session;
                let id = session["id"].as_str().unwrap_or_default();
                out.log(&format!(
                    "  {} {} {}",
                    pad_end(id, 46),
                    pad_end(&relative_age(matched.activity_ms, now), 9),
                    title_of(session)
                ));
            }
            out.log("");
        }
        if !report.skipped.is_empty() {
            out.log(&format!(
                "Skipped {} {} with an active run (never interrupted): {}",
                report.skipped.len(),
                noun(report.skipped.len()),
                report
                    .skipped
                    .iter()
                    .map(|(id, _)| id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !report.kept.is_empty() {
            out.log(&format!(
                "Kept {} {} named with --keep: {}",
                report.kept.len(),
                noun(report.kept.len()),
                report.kept.join(", ")
            ));
        }
        if report.dry_run && !report.matched.is_empty() {
            out.log(&format!(
                "\nDry run: nothing was changed. Re-run the same command with --yes to delete\nthese {} {}.",
                report.matched.len(),
                noun(report.matched.len())
            ));
        }
        for (id, error) in &report.failed {
            out.log_err(&format!("Failed to delete {id}: {error}"));
        }
    }

    if report.failed.is_empty() {
        Ok(())
    } else {
        Err(crate::HANDLED_EXIT.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::Output;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn stdout_of(captured: &crate::output::Captured) -> String {
        String::from_utf8(captured.out.lock().unwrap().clone()).unwrap()
    }

    fn stderr_of(captured: &crate::output::Captured) -> String {
        String::from_utf8(captured.err.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn ages_are_number_plus_unit() {
        assert_eq!(parse_age_ms("3d"), Ok(3 * 86_400_000));
        assert_eq!(parse_age_ms("72h"), Ok(72 * 3_600_000));
        assert_eq!(parse_age_ms("90m"), Ok(90 * 60_000));
        assert_eq!(parse_age_ms("2w"), Ok(2 * 604_800_000));
        assert_eq!(parse_age_ms("3 days"), Ok(3 * 86_400_000));
        assert_eq!(parse_age_ms("1D"), Ok(86_400_000));
    }

    /// An age decides whether a session is deleted, so every malformed form is
    /// a usage error rather than a guess.
    #[test]
    fn malformed_ages_are_usage_errors() {
        for (input, expected) in [
            ("3", "unknown unit"),
            ("d", "expects <number><unit>"),
            ("-3d", "expects <number><unit>"),
            ("3y", "unknown unit"),
            ("0d", "greater than zero"),
            ("999999999999999d", "too large"),
        ] {
            let error = parse_age_ms(input).unwrap_err();
            assert!(error.contains(expected), "{input} → {error}");
        }
    }

    #[test]
    fn options_parse_and_validate() {
        assert_eq!(
            parse(&args(&[
                "--older-than",
                "3d",
                "--keep",
                "a",
                "--keep",
                "b",
                "--json"
            ]))
            .unwrap(),
            Options {
                older_than: "3d".into(),
                older_than_ms: 3 * 86_400_000,
                keep: vec!["a".into(), "b".into()],
                apply: false,
                json: true,
            },
            "a preview is the default"
        );
        for (input, expected) in [
            (
                vec!["--older-than", "3d", "--yes", "--dry-run"],
                "mutually exclusive",
            ),
            (vec!["--older-than"], "missing value for --older-than"),
            (vec!["--keep"], "missing value for --keep"),
            (
                vec!["--older-than", "3d", "--older-than", "4d"],
                "duplicate option",
            ),
            (
                vec!["--older-than", "3d", "--force"],
                "unknown option: --force",
            ),
            (vec!["--older-than", "3d", "extra"], "unexpected argument"),
            (vec![], "Usage: future session prune"),
        ] {
            let error = parse(&args(&input)).unwrap_err();
            assert!(error.contains(expected), "{input:?} → {error}");
        }
    }

    #[tokio::test]
    async fn help_needs_no_server() {
        let (out, captured) = Output::memory();
        run(&args(&["--help"]), &out).await.unwrap();
        let help = stdout_of(&captured);
        assert!(help.contains("--older-than"));
        assert!(help.contains("--dry-run"), "{help}");
    }

    /// The history decides the age, not the metadata: a session the Agent has
    /// saved recently (a rename, or a storage upgrade rewriting every
    /// snapshot) must still be recognised as idle when nobody used it.
    #[tokio::test]
    async fn preview_lists_only_sessions_older_than_the_threshold() {
        let _guard = crate::test_env::lock_env().await;
        let now = now_ms();
        let day = 86_400_000i64;
        let body = json!({"sessions": [
            {"id": "stale", "sessionName": "old work", "updatedAtMs": now - 5 * day},
            {"id": "stale-untitled", "firstMessage": "a question I asked", "updatedAtMs": now},
            {"id": "fresh", "sessionName": "today", "updatedAtMs": now},
            {"id": "busy-even-though-old", "updatedAtMs": now - 9 * day, "isStreaming": true},
            {"id": "kept", "updatedAtMs": now - 9 * day},
            {"id": "no-clock"}
        ]})
        .to_string();
        let mut agent = crate::test_server::MockAgent::default();
        agent.responses.insert("list_sessions".into(), body);
        for (id, age_days) in [("stale-untitled", 4), ("fresh", 1)] {
            agent.per_session.insert(
                ("get_session_entries".into(), id.into()),
                json!({"entries": [
                    {"kind": "session_info", "createdAtMs": now},
                    {"kind": "assistant", "createdAtMs": now - age_days * day}
                ]})
                .to_string(),
            );
        }
        let seen = agent.seen.clone();
        let agent = crate::test_server::spawn_mock(agent).await;
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from(agent),
        )])
        .with_temp_home();

        let (out, captured) = Output::memory();
        run(&args(&["--older-than", "3d", "--keep", "kept"]), &out)
            .await
            .expect("preview");
        let text = stdout_of(&captured);
        assert!(text.contains("stale"), "{text}");
        assert!(text.contains("stale-untitled"), "{text}");
        // Its listed recency is "now" and its history is four days old: the
        // preview must report the history, and its own `session_info` snapshot
        // must not count as activity.
        assert!(text.contains("4d ago"), "{text}");
        assert!(!text.contains("today"), "{text}");
        assert!(text.contains("Matched     2 of 6 sessions"), "{text}");
        assert!(
            text.contains("Kept 1 session named with --keep: kept"),
            "{text}"
        );
        assert!(
            text.contains("Skipped 1 session with an active run"),
            "{text}"
        );
        assert!(text.contains("nothing was changed"), "{text}");
        assert!(!text.contains("no-clock"), "{text}");

        // Only sessions whose listed recency is inside the window need their
        // history read; the fast path is what keeps this one round trip each.
        let commands = seen.lock().unwrap();
        let mut probed: Vec<String> = commands
            .iter()
            .filter(|command| command.r#type == "get_session_entries")
            .map(|command| command.session_id.clone())
            .collect();
        probed.sort();
        assert_eq!(probed, vec!["fresh", "no-clock", "stale-untitled"]);
        assert_eq!(
            commands
                .iter()
                .filter(|c| c.r#type == "delete_sessions")
                .count(),
            0,
            "a preview must not delete"
        );
    }

    /// `--yes` deletes exactly the matched ids, in one batched call, and the
    /// per-session refusal the Agent answers with is reported instead of
    /// failing the whole batch.
    #[tokio::test]
    async fn apply_deletes_the_matched_ids_and_reports_refusals() {
        let _guard = crate::test_env::lock_env().await;
        let now = now_ms();
        let day = 86_400_000i64;
        let mut agent = crate::test_server::MockAgent::default();
        agent.responses.insert(
            "list_sessions".into(),
            json!({"sessions": [
                // Listed as just saved; its history is ten days old.
                {"id": "gone-1", "updatedAtMs": now},
                {"id": "gone-2", "updatedAtMs": now - 8 * day},
                {"id": "refused", "updatedAtMs": now},
                {"id": "recent", "updatedAtMs": now}
            ]})
            .to_string(),
        );
        for (id, age_days) in [("gone-1", 10), ("refused", 7), ("recent", 1)] {
            agent.per_session.insert(
                ("get_session_entries".into(), id.into()),
                json!({"entries": [{"kind": "assistant", "createdAtMs": now - age_days * day}]})
                    .to_string(),
            );
        }
        agent.responses.insert(
            "delete_sessions".into(),
            json!({"results": [
                {"sessionId": "gone-1", "deleted": true},
                {"sessionId": "gone-2", "deleted": true},
                {"sessionId": "refused", "deleted": false, "errorCode": "deleting",
                 "error": "session deletion is waiting for the active run to stop; retry delete_session"}
            ]})
            .to_string(),
        );
        let seen = agent.seen.clone();
        let addr = crate::test_server::spawn_mock(agent).await;
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from(addr),
        )])
        .with_temp_home();

        let (out, captured) = Output::memory();
        let error = run(&args(&["--older-than", "3d", "--yes"]), &out)
            .await
            .expect_err("a refused deletion exits non-zero");
        assert_eq!(error, crate::HANDLED_EXIT);
        let text = stdout_of(&captured);
        assert!(text.contains("2 of 3 sessions deleted"), "{text}");
        assert!(!text.contains("recent"), "{text}");
        // The refusal names the session and the Agent's reason on stderr, so a
        // script's stdout stays a report and a person still sees what failed.
        let errors = stderr_of(&captured);
        assert!(errors.contains("Failed to delete refused"), "{errors}");
        assert!(errors.contains("waiting for the active run"), "{errors}");

        let commands = seen.lock().unwrap();
        let deletes: Vec<_> = commands
            .iter()
            .filter(|command| command.r#type == "delete_sessions")
            .collect();
        assert_eq!(deletes.len(), 1, "one batched call");
        assert_eq!(deletes[0].session_ids, vec!["gone-1", "gone-2", "refused"]);
        assert_eq!(
            commands
                .iter()
                .filter(|c| c.r#type == "delete_session")
                .count(),
            0
        );
        // The fast path skipped the session whose listed time was already old.
        assert_eq!(
            commands
                .iter()
                .filter(|c| c.r#type == "get_session_entries")
                .count(),
            3
        );
    }

    #[test]
    fn only_conversation_entries_are_activity() {
        let page = json!({"entries": [
            {"kind": "session_info", "createdAtMs": 3_000},
            {"kind": "user", "createdAtMs": 1_000},
            {"kind": "tool", "createdAtMs": 2_000}
        ]});
        assert_eq!(newest_activity(&page), Some(2_000));
        // A page that only carries metadata (or nothing at all) establishes no
        // activity, so the caller falls back to the listed timestamp.
        assert_eq!(
            newest_activity(&json!({"entries": [{"kind": "session_info", "createdAtMs": 5}]})),
            None
        );
        assert_eq!(newest_activity(&json!({})), None);
    }
}
