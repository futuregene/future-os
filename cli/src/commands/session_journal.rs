//! `future session journal` — what the raw event journal costs, and the
//! explicit sweep that reclaims it.
//!
//! The journal is a *rebuildable* record: a settled run's raw events exist so a
//! client can replay the run, and once a snapshot replaces them they are dead
//! weight. This command exists because two things were invisible:
//!
//! * a run the compactor refuses keeps its raw journal forever, and that used
//!   to be a silent no-op — here it is a line of counters;
//! * a database that already grew has no automatic way back, because the only
//!   automatic trigger is a completion notification in the running process.
//!
//! Reporting is read-only and always available. Reclaiming is off unless
//! `--reclaim --yes` is given, and it is resumable: each call applies one
//! bounded slice and the command repeats until there is nothing left.

use crate::output::Output;
use crate::rpc::{grpc_addr, RunClient};
use serde_json::Value;
use std::collections::HashSet;

const HELP: &str = "future session journal — run-journal cost and reclamation

Usage:
  future session journal [--json]
  future session journal --reclaim [--yes] [--json]

Reports the agent database's raw run journal: how many rows and bytes
`run_events` holds, the largest single run, how many settled runs still keep a
raw journal, and the durable reclamation evidence (last reclaim, how many runs
and events, and why journals were kept).

Options:
  --reclaim   Reclaim settled raw run journals. Default is report-only, and
              even this flag only plans unless --yes is also given.
  --yes       Confirm the reclamation. Without it nothing is deleted.
  --json      Print the agent's report verbatim
  -h, --help  Show this help

What is deleted: only the raw event stream of runs that settled (completed,
failed, cancelled or interrupted) and whose folded snapshot is published in the
same transaction. Session messages (`entries`), message blocks, pricing and
running journals are never touched. The sweep is idempotent and resumable: an
interruption leaves published snapshots intact and the next run continues.
The raw journal is folded, not truncated — replay of a reclaimed run still
produces the same messages. Any command may report `quickCheck`, which must be
`ok` for the sweep to be considered complete.";

#[derive(Debug, PartialEq, Eq)]
struct Options {
    reclaim: bool,
    yes: bool,
    json: bool,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        reclaim: false,
        yes: false,
        json: false,
    };
    let mut seen = HashSet::new();
    for flag in args {
        if !seen.insert(flag.as_str()) {
            return Err(format!("duplicate option: {flag}"));
        }
        match flag.as_str() {
            "--reclaim" => options.reclaim = true,
            "--yes" => options.yes = true,
            "--json" => options.json = true,
            other => {
                return Err(format!(
                    "unknown option: {other}; see future session journal --help"
                ))
            }
        }
    }
    if options.yes && !options.reclaim {
        return Err("--yes only confirms --reclaim".into());
    }
    Ok(options)
}

/// Byte counts are the whole point of this command, so they are never printed
/// as an unreadable run of digits.
fn bytes(value: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = value as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit + 1 < UNITS.len() {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

/// Digit grouping, so a six-figure row count is readable at a glance.
fn count(value: i64) -> String {
    let digits = value.abs().to_string();
    let mut grouped = String::new();
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(character);
    }
    if value < 0 {
        format!("-{grouped}")
    } else {
        grouped
    }
}

fn number(value: &Value, key: &str) -> i64 {
    match &value[key] {
        Value::Number(number) => number.as_i64().unwrap_or(0),
        Value::String(text) => text.parse().unwrap_or(0),
        _ => 0,
    }
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_string()
}

fn short(id: &str) -> String {
    if id.chars().count() <= 22 {
        id.to_string()
    } else {
        format!("{}…", id.chars().take(21).collect::<String>())
    }
}

fn local_time(ms: i64) -> String {
    if ms <= 0 {
        return "never".into();
    }
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|stamp| {
            stamp
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| format!("{ms} ms"))
}

/// The read-only report, as `session info`-style aligned rows.
fn report(stats: &Value, out: &Output) {
    let database = &stats["database"];
    let raw = &stats["runEvents"];
    let snapshots = &stats["runSnapshots"];
    let pending = &stats["settledUncompacted"];
    let leftovers = &stats["publishedWithRawLeftovers"];
    let reclaimed = &stats["reclaimed"];
    let row = |label: &str, value: String| out.log(&format!("{label:<18}{value}"));
    row(
        "Database",
        format!(
            "{} ({} free pages)",
            bytes(number(database, "bytes")),
            bytes(number(database, "freeBytes"))
        ),
    );
    row(
        "run_events",
        format!(
            "{} rows · {} payload",
            count(number(raw, "rows")),
            bytes(number(raw, "bytes"))
        ),
    );
    row(
        "run_snapshots",
        format!(
            "{} runs · {} payload",
            count(number(snapshots, "rows")),
            bytes(number(snapshots, "bytes"))
        ),
    );
    let largest = &stats["largestRun"];
    if largest.is_object() {
        row(
            "Largest run",
            format!(
                "{} events · {}",
                count(number(largest, "events")),
                bytes(number(largest, "bytes"))
            ),
        );
        out.log(&format!(
            "{:<18}session {} run {}",
            "",
            short(&text(largest, "sessionId")),
            short(&text(largest, "runId"))
        ));
    }
    row(
        "Reclaimable",
        format!(
            "{} settled runs · {} events · {}",
            count(number(pending, "runs")),
            count(number(pending, "events")),
            bytes(number(pending, "bytes"))
        ),
    );
    row(
        "Leftover rows",
        format!(
            "{} rows in {} published runs",
            count(number(leftovers, "events")),
            count(number(leftovers, "runs"))
        ),
    );
    row(
        "Last reclamation",
        match number(reclaimed, "atMs") {
            0 => "never".into(),
            at => format!(
                "{} ({} runs, {} events)",
                local_time(at),
                count(number(reclaimed, "runs")),
                count(number(reclaimed, "events"))
            ),
        },
    );
    row(
        "Over budget",
        format!(
            "{} snapshot(s) published above the 4 MiB target",
            count(number(stats, "oversizePublishes"))
        ),
    );
    row(
        "Truncated",
        format!(
            "{} snapshot(s) folded from a run that never sent agent_end",
            count(number(stats, "truncatedPublishes"))
        ),
    );
    let skipped = &stats["skipped"];
    let kept: Vec<String> = skipped
        .as_object()
        .map(|reasons| {
            reasons
                .iter()
                .map(|(reason, value)| {
                    format!("{reason}={}", count(value.as_i64().unwrap_or_default()))
                })
                .collect()
        })
        .unwrap_or_default();
    row(
        "Kept journals",
        if kept.is_empty() {
            "none recorded".into()
        } else {
            kept.join(", ")
        },
    );
}

/// One line per run the sweep looked at: what it held, and either what was
/// deleted or why it was kept. This is the per-run evidence a report needs.
fn print_entries(report: &Value, dry_run: bool, out: &Output) {
    let entries = report["entries"].as_array().cloned().unwrap_or_default();
    for entry in &entries {
        let detail = match entry["skip"].as_str() {
            Some(reason) => format!("kept ({reason})"),
            None if dry_run => bytes(number(entry, "rawBytes")),
            None => format!(
                "deleted {} · snapshot {}",
                count(number(entry, "deleted")),
                bytes(number(entry, "snapshotBytes"))
            ),
        };
        out.log(&format!(
            "  {:<8} {} · {} events · {detail}",
            text(entry, "kind"),
            short(&text(entry, "runId")),
            count(number(entry, "rawEvents")),
        ));
    }
}

pub(super) async fn run(args: &[String], out: &Output) -> Result<(), String> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        out.log(HELP);
        return Ok(());
    }
    let options = parse(args)?;
    let client = RunClient::new(&grpc_addr());
    if !options.reclaim {
        let stats = client.journal_stats().await?;
        if options.json {
            out.log(&serde_json::to_string_pretty(&stats).map_err(|e| e.to_string())?);
        } else {
            report(&stats, out);
            out.log("");
            out.log("Nothing was deleted. `future session journal --reclaim` shows a plan;");
            out.log("add --yes to apply it.");
        }
        return Ok(());
    }

    let plan = client.reclaim_run_journals(true).await?;
    out.log("Plan (nothing deleted yet):");
    out.log(&format!(
        "  {} settled runs · {} raw events · {}",
        count(number(&plan, "reclaimedRuns")),
        count(number(&plan, "reclaimedEvents")),
        bytes(
            plan["entries"]
                .as_array()
                .map(|entries| entries.iter().map(|entry| number(entry, "rawBytes")).sum())
                .unwrap_or(0)
        )
    ));
    out.log(&format!(
        "  {} leftover rows from earlier interruptions",
        count(number(&plan, "leftoverEvents"))
    ));
    print_entries(&plan, true, out);
    if options.json {
        out.log(&serde_json::to_string_pretty(&plan).map_err(|e| e.to_string())?);
    }
    if !options.yes {
        out.log("");
        out.log("Dry run: nothing was deleted. Re-run with --reclaim --yes to apply it.");
        return Ok(());
    }

    let mut applied_runs = 0i64;
    let mut applied_events = 0i64;
    let bytes_before = number(&plan["database"], "bytesBefore");
    let mut bytes_after;
    let quick_check;
    // Each call is bounded, so the loop is the sweep. The cap is a guard against
    // a server that keeps answering `hasMore` without retiring anything (it would
    // otherwise spin forever); a real sweep needs one call per 20s slice.
    const MAX_SLICES: usize = 256;
    let mut slices = 0usize;
    loop {
        slices += 1;
        if slices > MAX_SLICES {
            return Err(format!(
                "reclamation stopped after {MAX_SLICES} slices without finishing; \
                 re-run `future session journal --reclaim --yes` to continue"
            ));
        }
        let report = client.reclaim_run_journals(false).await?;
        out.log("Reclaiming:");
        print_entries(&report, false, out);
        applied_runs += number(&report, "reclaimedRuns");
        applied_events += number(&report, "reclaimedEvents");
        bytes_after = number(&report["database"], "bytesAfter");
        if report["hasMore"].as_bool() != Some(true) {
            quick_check = report["quickCheck"].clone();
            if options.json {
                out.log(&serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?);
            }
            break;
        }
    }
    out.log("");
    out.log(&format!(
        "Reclaimed {} events from {} runs.",
        count(applied_events),
        count(applied_runs)
    ));
    out.log(&format!(
        "Database {} → {}.",
        bytes(bytes_before),
        bytes(bytes_after)
    ));
    match quick_check.as_str() {
        Some("ok") => {
            out.log("quick_check: ok");
            Ok(())
        }
        other => Err(format!(
            "reclamation finished but the integrity check did not pass: {}",
            other.unwrap_or("missing quick_check result")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn reclaim_needs_both_an_explicit_trigger_and_confirmation() {
        assert_eq!(
            parse(&args(&[])).unwrap(),
            Options {
                reclaim: false,
                yes: false,
                json: false
            }
        );
        assert_eq!(
            parse(&args(&["--reclaim"])).unwrap(),
            Options {
                reclaim: true,
                yes: false,
                json: false
            }
        );
        assert!(parse(&args(&["--reclaim", "--yes", "--json"])).unwrap().yes);
        for input in [
            vec!["--yes"],
            vec!["--force"],
            vec!["--reclaim", "--reclaim"],
        ] {
            assert!(parse(&args(&input)).is_err(), "{input:?}");
        }
    }

    #[test]
    fn sizes_and_counts_stay_readable() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1023), "1023 B");
        assert_eq!(bytes(1024), "1.0 KiB");
        assert_eq!(bytes(356_355_038), "339.8 MiB");
        assert_eq!(count(0), "0");
        assert_eq!(count(999), "999");
        assert_eq!(count(1_213_282), "1,213,282");
        assert_eq!(count(-1_000), "-1,000");
    }

    #[tokio::test]
    async fn help_needs_no_server() {
        let (out, cap) = Output::memory();
        run(&args(&["--help"]), &out).await.unwrap();
        let printed = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(printed.contains("--reclaim"));
        assert!(printed.contains("never touched"));
    }

    fn stats_report() -> &'static str {
        r#"{"runEvents":{"rows":1455957,"bytes":356355038},
            "runSnapshots":{"rows":382,"bytes":113772416},
            "largestRun":{"sessionId":"20261026-082104-5e89","runId":"run_0cadceb2c0ff",
                           "events":136298,"bytes":32800000},
            "settledUncompacted":{"runs":22,"events":1304373,"bytes":307500000},
            "publishedWithRawLeftovers":{"runs":0,"events":0},
            "database":{"bytes":1008119808,"freeBytes":0},
            "reclaimed":{"runs":15,"events":1213282,"atMs":1761461892000},
            "oversizePublishes":1,
            "truncatedPublishes":3,
            "skipped":{"not_terminal":2,"oversized_event":1}}"#
    }

    fn reclaim_report() -> &'static str {
        r#"{"dryRun":false,
            "entries":[{"sessionId":"s","runId":"run_big","rawEvents":136298,
                        "rawBytes":32800000,"kind":"journal","deleted":136298,
                        "snapshotBytes":1940000,"skip":null}],
            "reclaimedRuns":1,"reclaimedEvents":136298,
            "leftoverRuns":0,"leftoverEvents":0,
            "hasMore":false,"quickCheck":"ok",
            "database":{"bytesBefore":1008119808,"bytesAfter":680000000,"freeBytesAfter":0}}"#
    }

    async fn dispatch_report(
        reclaim: &str,
        arguments: Vec<String>,
    ) -> (Result<(), String>, String, crate::test_server::MockAgent) {
        let _guard = crate::test_env::lock_env().await;
        let agent = crate::test_server::MockAgent::respond("get_journal_stats", stats_report());
        let agent = {
            let mut agent = agent;
            agent
                .responses
                .insert("reclaim_run_journals".into(), reclaim.into());
            agent
        };
        let seen = agent.clone();
        let address = crate::test_server::spawn_mock(agent).await;
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from(address),
        )]);
        let (out, cap) = Output::memory();
        let result = crate::commands::session::session(Some("journal"), &arguments, &out).await;
        let printed = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        (result, printed, seen)
    }

    async fn dispatch(arguments: Vec<String>) -> (Result<(), String>, String) {
        let (result, printed, _) = dispatch_report(reclaim_report(), arguments).await;
        (result, printed)
    }

    #[tokio::test]
    async fn the_report_names_the_rows_and_the_unreclaimed_work() {
        let (result, printed) = dispatch(args(&[])).await;
        result.unwrap();
        assert!(printed.contains("run_events"), "{printed}");
        assert!(printed.contains("1,455,957 rows"), "{printed}");
        assert!(printed.contains("Largest run"), "{printed}");
        assert!(printed.contains("22 settled runs"), "{printed}");
        assert!(printed.contains("not_terminal=2"), "{printed}");
        assert!(
            printed.contains("3 snapshot(s) folded from a run that never sent agent_end"),
            "{printed}"
        );
        assert!(printed.contains("Nothing was deleted"), "{printed}");
    }

    #[tokio::test]
    async fn reclaim_without_confirmation_only_plans() {
        let (result, printed) = dispatch(args(&["--reclaim"])).await;
        result.unwrap();
        assert!(printed.contains("Plan (nothing deleted yet)"), "{printed}");
        assert!(printed.contains("run_big"), "{printed}");
        assert!(
            printed.contains("Dry run: nothing was deleted"),
            "{printed}"
        );
        assert!(!printed.contains("quick_check"), "{printed}");
    }

    #[tokio::test]
    async fn an_applied_sweep_prints_the_integrity_result() {
        let (result, printed) = dispatch(args(&["--reclaim", "--yes"])).await;
        result.unwrap();
        assert!(
            printed.contains("Reclaimed 136,298 events from 1 runs"),
            "{printed}"
        );
        assert!(printed.contains("quick_check: ok"), "{printed}");
    }

    #[tokio::test]
    async fn a_sweep_whose_integrity_check_fails_is_an_error() {
        let _guard = crate::test_env::lock_env().await;
        let agent = crate::test_server::MockAgent::respond(
            "reclaim_run_journals",
            &reclaim_report().replace("\"quickCheck\":\"ok\"", "\"quickCheck\":\"malformed\""),
        );
        let address = crate::test_server::spawn_mock(agent).await;
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from(address),
        )]);
        let (out, _) = Output::memory();
        let error = run(&args(&["--reclaim", "--yes"]), &out).await.unwrap_err();
        assert!(error.contains("integrity check"), "{error}");
    }

    /// A machine that has never reclaimed and has never skipped a journal: the
    /// two rows must read "never" and "none recorded" rather than a blank or a
    /// zero that looks like a measurement. Both arms are what a fresh install
    /// prints, and both are pure functions of the report — no Agent is needed.
    #[test]
    fn a_report_with_no_history_says_never_and_none_recorded() {
        let (out, cap) = Output::memory();
        report(&serde_json::json!({}), &out);
        let printed = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let row = |label: &str| {
            printed
                .lines()
                .find(|line| line.starts_with(label))
                .map(|line| line.trim_end().to_string())
                .unwrap_or_else(|| panic!("no {label} row in:\n{printed}"))
        };
        // One call per row, into a binding: a `row(..)` inside an assert's
        // message argument is only evaluated when the assertion fails, and would
        // sit in this lane's count as an uncovered line forever.
        let reclaimed = row("Last reclamation");
        assert!(reclaimed.ends_with("never"), "{reclaimed}");
        let kept = row("Kept journals");
        assert!(kept.ends_with("none recorded"), "{kept}");
    }

    /// A kept journal is printed with the reason it was kept: the sweep's whole
    /// point is that "not deleted" is evidence, not silence.
    #[test]
    fn a_kept_journal_prints_the_reason_it_was_kept() {
        let (out, cap) = Output::memory();
        let plan = serde_json::json!({
            "entries": [{
                "sessionId": "s",
                "runId": "run_big",
                "rawEvents": 3,
                "rawBytes": 10,
                "kind": "journal",
                "skip": "not_terminal",
            }],
        });
        print_entries(&plan, true, &out);
        let printed = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(printed.contains("kept (not_terminal)"), "{printed}");
    }

    /// The report's numbers arrive as JSON and are not always typed: a counter
    /// can be a string after an Agent round trip, and an unexpected shape must
    /// degrade to zero instead of inventing a figure.
    #[test]
    fn counters_read_a_string_and_degrade_an_unknown_shape_to_zero() {
        assert_eq!(
            number(&serde_json::json!({ "at": "1761461892000" }), "at"),
            1_761_461_892_000
        );
        assert_eq!(number(&serde_json::json!({ "at": true }), "at"), 0);
        assert_eq!(number(&serde_json::json!({ "at": null }), "at"), 0);
        assert_eq!(number(&serde_json::json!({}), "at"), 0);
        // An unparseable string is not a number either.
        assert_eq!(number(&serde_json::json!({ "at": "later" }), "at"), 0);
    }

    /// Run ids shorter than the column are printed whole; longer ones are cut to
    /// 21 characters plus one ellipsis, so every row keeps its alignment.
    #[test]
    fn long_run_ids_are_shortened_and_short_ones_are_not() {
        let short_id = "a".repeat(22);
        assert_eq!(short(&short_id), short_id);
        let long = "b".repeat(30);
        let printed = short(&long);
        assert_eq!(printed.chars().count(), 22);
        assert!(printed.ends_with('…'), "{printed}");
        assert_eq!(printed.trim_end_matches('…'), "b".repeat(21));
    }

    /// "Never" is a state, not a timestamp: a missing or non-positive instant is
    /// said in words rather than rendered as 1970 or as a bare number.
    #[test]
    fn an_absent_instant_reads_as_never() {
        assert_eq!(local_time(0), "never");
        assert_eq!(local_time(-1), "never");
    }

    /// `--json` is the machine-readable route: the Agent's own payload goes out
    /// whole, not the human report rendered beside it. The human labels are
    /// asserted absent, so a run that printed only the table cannot pass.
    #[tokio::test]
    async fn a_report_as_json_is_the_agents_payload_verbatim() {
        let (result, printed) = dispatch(args(&["--json"])).await;
        result.unwrap();
        // Parsed first, asserted second: a message handed to `unwrap_or_else`
        // or to a multi-line `assert!` is evaluated only on failure, and would
        // sit in this lane's count as an uncovered line for ever.
        let parsed = serde_json::from_str::<Value>(printed.trim());
        assert!(parsed.is_ok(), "not one JSON document:\n{printed}");
        let payload = parsed.unwrap();
        assert!(payload["runEvents"]["rows"].is_number(), "{printed}");
        assert!(!printed.contains("Kept journals"), "{printed}");
        assert!(!printed.contains("Nothing was deleted"), "{printed}");
    }

    /// The dry run has its own JSON branch above the confirmation check: a plan
    /// is exactly what a script wants to read. The document comes first and the
    /// "add --yes" advice after it, so a caller can take the leading JSON and
    /// ignore the trailing sentence.
    #[tokio::test]
    async fn a_dry_run_as_json_prints_the_plan_then_the_advice() {
        let (result, printed) = dispatch(args(&["--reclaim", "--json"])).await;
        result.unwrap();
        assert!(printed.contains("\"hasMore\": false"), "{printed}");
        assert!(printed.contains("\"entries\""), "{printed}");
        let document = printed.find("\"hasMore\": false").unwrap();
        let advice = printed
            .find("Dry run: nothing was deleted")
            .unwrap_or_else(|| panic!("no advice after the plan:\n{printed}"));
        assert!(
            document < advice,
            "the plan must precede the advice:\n{printed}"
        );
        // The plan's own JSON is not the human report: no bytes column.
        assert!(!printed.contains("32.8 MiB"), "{printed}");
    }

    /// The applied sweep's JSON is the *last* slice — the one that carried
    /// `hasMore: false` and the integrity result — because that is the report
    /// the caller's numbers come from. The mock answers the plan and the slice
    /// with *different* counts, so a run that printed only the plan's document
    /// cannot pass: the plan's `reclaimedEvents` is 0 and the slice's is the
    /// number that was actually reclaimed.
    #[tokio::test]
    async fn an_applied_sweep_as_json_prints_the_final_slice() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = crate::test_server::MockAgent::respond("get_journal_stats", stats_report());
        agent.sequences.insert(
            "reclaim_run_journals".into(),
            vec![
                reclaim_report().replace("\"reclaimedEvents\":136298", "\"reclaimedEvents\":0"),
                reclaim_report().into(),
            ],
        );
        let address = crate::test_server::spawn_mock(agent).await;
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from(address),
        )]);
        let (out, cap) = Output::memory();
        run(&args(&["--reclaim", "--yes", "--json"]), &out)
            .await
            .unwrap();
        let printed = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        // Two documents — the plan's and the final slice's — and the slice's
        // count is the one that says the sweep ran.
        assert_eq!(
            printed.matches("\"reclaimedEvents\": ").count(),
            2,
            "{printed}"
        );
        assert!(printed.contains("\"reclaimedEvents\": 136298"), "{printed}");
        assert!(printed.contains("\"quickCheck\": \"ok\""), "{printed}");
        assert!(printed.contains("quick_check: ok"), "{printed}");
    }

    /// A server that keeps answering `hasMore` without retiring anything is the
    /// one way this loop can fail to end, so the cap is load-bearing and its
    /// refusal has to say how to resume. The mock answers `hasMore: true` for
    /// ever, which is that server.
    #[tokio::test]
    async fn a_sweep_that_never_finishes_stops_at_the_cap_and_says_how_to_resume() {
        let (result, printed, agent) = dispatch_report(
            &reclaim_report().replace("\"hasMore\":false", "\"hasMore\":true"),
            args(&["--reclaim", "--yes"]),
        )
        .await;
        let error = result.unwrap_err();
        assert!(error.contains("stopped after 256 slices"), "{error}");
        assert!(error.contains("to continue"), "{error}");
        // One plan call plus exactly MAX_SLICES slices, and no "a real sweep
        // finished" line: the cap bounds the loop, it does not end it.
        assert_eq!(agent.seen_of("reclaim_run_journals").len(), 1 + 256);
        assert!(!printed.contains("Reclaimed "), "{printed}");
    }
}
