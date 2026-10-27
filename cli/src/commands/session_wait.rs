//! `future session wait <session-id> [--run <run-id>] [--timeout <secs>]` —
//! wait for a run to finish, then print its final answer.
//!
//! This is the retrieval counterpart of `future run --no-wait`. It subscribes
//! to the session's event stream, filters to the requested run (or the
//! session's active/head run when omitted), and exits when that run's
//! `agent_end` arrives. The final text is streamed to stdout just like a
//! normal `future run`.

use crate::output::Output;
use crate::rpc::{grpc_addr, RunClient};
use humantime::format_duration;
use serde_json::{Map, Value};
use std::time::Duration;

const HELP: &str = "future session wait — wait for a run to finish and print its answer

Usage:
  future session wait <session-id> [--run <run-id>] [--timeout <duration>]

Waits for the run to reach `agent_end`, streaming any text to stdout. If the
run is already finished, the command returns immediately with its answer. If
no `--run` is given, the command waits for the session's active or head run.

Options:
  --run <run-id>           Wait for this specific run id.
  --timeout <duration>     Maximum time to wait, e.g. 15m, 30s, 1h
                           (default: 5m). A timeout is an error and the run id
                           is still reported.
  --mode json              Output the final result as JSON (default: text).
  -h, --help               Show this help

Examples:
  future session wait s-123
  future session wait s-123 --run r-456 --timeout 15m";

#[derive(Debug, PartialEq, Eq)]
struct Options {
    session: String,
    run: Option<String>,
    timeout: Duration,
    mode: String,
}

fn parse_duration(raw: &str) -> Result<Duration, String> {
    humantime::parse_duration(raw).map_err(|e| format!("invalid duration: {e}"))
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut session: Option<String> = None;
    let mut run: Option<String> = None;
    let mut timeout = Duration::from_secs(5 * 60);
    let mut mode = "text".to_string();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        if flag == "--help" || flag == "-h" {
            return Err("__help__".to_string());
        }
        if flag == "--mode" {
            let value = args
                .get(index + 1)
                .ok_or("--mode requires a value")?
                .clone();
            if value != "text" && value != "json" {
                return Err("--mode must be text or json".into());
            }
            mode = value;
            index += 2;
            continue;
        }
        if flag == "--run" {
            run = Some(args.get(index + 1).ok_or("--run requires a value")?.clone());
            index += 2;
            continue;
        }
        if flag == "--timeout" {
            let raw = args.get(index + 1).ok_or("--timeout requires a value")?;
            timeout = parse_duration(raw)?;
            index += 2;
            continue;
        }
        if flag.starts_with("--") {
            return Err(format!("unknown option: {flag}"));
        }
        if session.is_some() {
            return Err("too many positional arguments".into());
        }
        session = Some(flag.to_string());
        index += 1;
    }
    Ok(Options {
        session: session.ok_or("a session id is required")?,
        run,
        timeout,
        mode,
    })
}

pub(super) async fn run(args: &[String], out: &Output) -> Result<(), String> {
    let options = match parse(args) {
        Ok(opts) => opts,
        Err(err) if err == "__help__" => {
            out.log(HELP);
            return Ok(());
        }
        Err(err) => return Err(err),
    };

    let client = RunClient::new(&grpc_addr());

    // Resolve the run to wait for when not explicitly provided.
    let target_run = if let Some(run) = &options.run {
        run.clone()
    } else {
        let state = client
            .get_state(Some(&options.session))
            .await
            .map_err(|err| super::session::missing_session_error(err, &options.session))?;
        if let Some(active) = state.get("activeRun").and_then(|v| v.get("runId")) {
            active.as_str().unwrap_or("").to_string()
        } else {
            let queued = state["queuedRuns"].as_array().cloned().unwrap_or_default();
            let first = queued.first().and_then(|v| v.get("runId"));
            first.and_then(Value::as_str).unwrap_or("").to_string()
        }
    };

    if target_run.is_empty() {
        return Err(format!(
            "Session {} has no active or queued run to wait for",
            options.session
        ));
    }

    let result = tokio::time::timeout(
        options.timeout,
        client.stream_events_filtered(&options.session, Some(&target_run), None, false, out),
    )
    .await
    .map_err(|_| {
        format!(
            "Timed out waiting for run {target_run} in session {} after {}",
            options.session,
            format_duration(options.timeout)
        )
    })?
    .map_err(|e| e.to_string())?;

    let (events, text) = result;
    // A wait that ends with nothing to print must not look like a success: an
    // empty answer and a run the Agent cut off are the two ways a caller is
    // left holding an empty string, and exiting 0 on them is how a delegated
    // sub-session reported "done" without ever answering.
    let finish = finish_of(&events, &text);
    if options.mode == "json" {
        let mut value = Map::new();
        value.insert("sessionId".to_string(), Value::String(options.session));
        value.insert("runId".to_string(), Value::String(target_run.clone()));
        value.insert("text".to_string(), Value::String(text));
        value.insert(
            "finishReason".to_string(),
            Value::String(finish.token().to_string()),
        );
        value.insert("messages".to_string(), Value::Array(events));
        out.log(&serde_json::to_string_pretty(&Value::Object(value)).map_err(|e| e.to_string())?);
    } else {
        if !text.is_empty() {
            out.log(&text);
        }
    }
    match finish {
        Finish::Answer => Ok(()),
        other => Err(format!(
            "run {target_run} ended as {}: {}",
            other.token(),
            other.detail()
        )),
    }
}

/// How the waited run ended, from its terminal event as much as its text.
#[derive(Debug, PartialEq, Eq)]
enum Finish {
    /// `agent_end` with a finished state and something to print.
    Answer,
    /// The run finished cleanly but its answer was empty or blank.
    EmptyAnswer,
    /// The Agent named a non-finishing terminal state (`cancelled`,
    /// `interrupted_by_restart`, `incomplete`, …).
    Interrupted,
    /// The run carried an error, or the stream closed with no terminal at all.
    Error,
}

impl Finish {
    fn token(&self) -> &'static str {
        match self {
            Finish::Answer => "answer",
            Finish::EmptyAnswer => "empty_answer",
            Finish::Interrupted => "interrupted",
            Finish::Error => "error",
        }
    }

    fn detail(&self) -> &'static str {
        match self {
            Finish::Answer => "the run answered",
            Finish::EmptyAnswer => {
                "the run finished without any answer; there is nothing to hand back"
            }
            Finish::Interrupted => {
                "the Agent stopped or restarted before the run finished; the answer is partial at best"
            }
            Finish::Error => "the run reported an error or its stream closed early",
        }
    }
}

/// Read the run's terminal event: its `state`, its `error`, and whether one
/// arrived at all. Absent `state` is the older event shape and is read as a
/// normal end — only a named non-finishing state or an error makes it abnormal.
fn finish_of(events: &[Value], text: &str) -> Finish {
    let terminal = events
        .iter()
        .rev()
        .find(|event| event.get("type").and_then(Value::as_str) == Some("agent_end"));
    let Some(terminal) = terminal else {
        // No `agent_end` at all: the stream closed under the run, so whatever
        // text arrived is a prefix, not an answer.
        return Finish::Interrupted;
    };
    if terminal
        .get("error")
        .and_then(Value::as_str)
        .is_some_and(|error| !error.trim().is_empty())
    {
        return Finish::Error;
    }
    match terminal.get("state").and_then(Value::as_str) {
        None | Some("completed") => {
            if text.trim().is_empty() {
                Finish::EmptyAnswer
            } else {
                Finish::Answer
            }
        }
        Some("error" | "failed") => Finish::Error,
        Some(_) => Finish::Interrupted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{spawn_mock, stream_event_with_run, MockAgent};

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    async fn mock_env(agent: MockAgent) -> (MockAgent, crate::test_env::EnvGuard) {
        let addr = spawn_mock(agent.clone()).await;
        let env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from(addr),
        )]);
        (agent, env)
    }

    #[test]
    fn options_require_session() {
        let parsed = parse(&args(&["s1"])).unwrap();
        assert_eq!(parsed.session, "s1");
        assert!(parsed.run.is_none());
        assert!(parse(&args(&[])).is_err());
    }

    #[test]
    fn options_parse_run_and_timeout() {
        let parsed = parse(&args(&[
            "s1",
            "--run",
            "r-1",
            "--timeout",
            "15m",
            "--mode",
            "json",
        ]));
        assert!(parsed.is_ok());
        let parsed = parsed.unwrap();
        assert_eq!(parsed.session, "s1");
        assert_eq!(parsed.run, Some("r-1".to_string()));
        assert_eq!(parsed.timeout, Duration::from_secs(15 * 60));
        assert_eq!(parsed.mode, "json");
    }

    #[test]
    fn options_reject_unknown_flags() {
        assert!(parse(&args(&["s1", "--bogus"])).is_err());
        assert!(parse(&args(&["s1", "--mode", "xml"])).is_err());
    }

    #[tokio::test]
    async fn wait_for_explicit_run_streams_answer() {
        let _guard = crate::test_env::lock_env().await;
        let mut responses = std::collections::HashMap::new();
        responses.insert(
            "get_state".into(),
            serde_json::json!({"activeRun": {"runId": "r-1"}}).to_string(),
        );
        let agent = MockAgent {
            responses,
            events: vec![
                stream_event_with_run("text_chunk", "{\"text\":\"hello\"}", "r-1"),
                stream_event_with_run("agent_end", "{}", "r-1"),
            ],
            ..Default::default()
        };
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        run(&args(&["s1", "--run", "r-1"]), &out)
            .await
            .expect("wait");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("hello"));
    }

    /// A child run that ends with nothing to hand back must not look finished.
    /// `future session wait` used to print nothing and exit 0 in both cases, so
    /// a delegated sub-session that was cut off read exactly like one that
    /// answered — the same silent success the task ledger had.
    #[tokio::test]
    async fn wait_reports_a_run_that_ended_without_an_answer() {
        for (terminal, expected) in [
            (r#"{"state":"completed"}"#, "empty_answer"),
            (r#"{"state":"cancelled"}"#, "interrupted"),
            (r#"{"state":"interrupted_by_restart"}"#, "interrupted"),
        ] {
            let _guard = crate::test_env::lock_env().await;
            let mut responses = std::collections::HashMap::new();
            responses.insert(
                "get_state".into(),
                serde_json::json!({"activeRun": {"runId": "r-1"}}).to_string(),
            );
            let agent = MockAgent {
                responses,
                events: vec![stream_event_with_run("agent_end", terminal, "r-1")],
                ..Default::default()
            };
            let (_agent, _env) = mock_env(agent).await;
            let (out, _cap) = Output::memory();
            let error = run(&args(&["s1", "--run", "r-1"]), &out)
                .await
                .expect_err("an empty or cut-off run is not a success");
            assert!(
                error.contains(expected),
                "terminal {terminal} must report {expected}: {error}"
            );
        }
    }

    /// The JSON form still answers in full — the caller that asked for it can
    /// read `finishReason` instead of inferring from an empty `text`.
    #[tokio::test]
    async fn wait_json_names_the_finish_reason() {
        let _guard = crate::test_env::lock_env().await;
        let mut responses = std::collections::HashMap::new();
        responses.insert(
            "get_state".into(),
            serde_json::json!({"activeRun": {"runId": "r-1"}}).to_string(),
        );
        let agent = MockAgent {
            responses,
            events: vec![stream_event_with_run(
                "agent_end",
                r#"{"state":"completed"}"#,
                "r-1",
            )],
            ..Default::default()
        };
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        let error = run(&args(&["s1", "--run", "r-1", "--mode", "json"]), &out)
            .await
            .expect_err("still a failure");
        assert!(error.contains("empty_answer"), "{error}");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(parsed["finishReason"], "empty_answer");
        assert_eq!(parsed["text"], "");
    }

    #[tokio::test]
    async fn wait_resolves_run_from_state_when_omitted() {
        let _guard = crate::test_env::lock_env().await;
        let mut responses = std::collections::HashMap::new();
        responses.insert(
            "get_state".into(),
            serde_json::json!({"activeRun": {"runId": "r-active"}}).to_string(),
        );
        let agent = MockAgent {
            responses,
            events: vec![
                stream_event_with_run("text_chunk", "{\"text\":\"from active\"}", "r-active"),
                stream_event_with_run("agent_end", "{}", "r-active"),
            ],
            ..Default::default()
        };
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        run(&args(&["s1"]), &out).await.expect("wait");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("from active"));
    }

    #[tokio::test]
    async fn wait_errors_when_no_run_exists() {
        let _guard = crate::test_env::lock_env().await;
        let mut responses = std::collections::HashMap::new();
        responses.insert(
            "get_state".into(),
            serde_json::json!({"activeRun": null, "queuedRuns": []}).to_string(),
        );
        let agent = MockAgent {
            responses,
            ..Default::default()
        };
        let (_agent, _env) = mock_env(agent).await;
        let (out, _cap) = Output::memory();
        let err = run(&args(&["s1"]), &out).await.unwrap_err();
        assert!(err.contains("no active or queued run"));
    }

    #[tokio::test]
    async fn wait_timeout_reports_run_id() {
        let _guard = crate::test_env::lock_env().await;
        let mut responses = std::collections::HashMap::new();
        responses.insert(
            "get_state".into(),
            serde_json::json!({"activeRun": {"runId": "r-1"}}).to_string(),
        );
        let agent = MockAgent {
            responses,
            stream_hang_after: true,
            ..Default::default()
        };
        let (_agent, _env) = mock_env(agent).await;
        let (out, _cap) = Output::memory();
        let err = run(&args(&["s1", "--run", "r-1", "--timeout", "50ms"]), &out)
            .await
            .unwrap_err();
        assert!(err.contains("Timed out waiting for run r-1"));
    }

    /// The parse errors that are not the `--help` sentinel reach the caller:
    /// a second positional is a mistake, not a target.
    #[tokio::test]
    async fn a_second_positional_and_an_unknown_option_are_reported() {
        assert_eq!(
            parse(&args(&["s1", "s2"])).unwrap_err(),
            "too many positional arguments"
        );
        assert_eq!(
            parse(&args(&["s1", "--bogus"])).unwrap_err(),
            "unknown option: --bogus"
        );
        let (out, cap) = Output::memory();
        let error = run(&args(&["s1", "s2"]), &out).await.unwrap_err();
        assert_eq!(error, "too many positional arguments");
        assert!(
            cap.out.lock().unwrap().is_empty(),
            "a usage error goes to stderr, not stdout"
        );
    }

    /// `--mode json` reports the run as a document a pipeline can read, with the
    /// events and the text side by side.
    #[tokio::test]
    async fn json_mode_reports_the_run_as_a_document() {
        let _guard = crate::test_env::lock_env().await;
        let agent = MockAgent {
            events: vec![
                stream_event_with_run("text_chunk", "{\"text\":\"hi\"}", "r-1"),
                stream_event_with_run("agent_end", "{}", "r-1"),
            ],
            ..MockAgent::default()
        };
        let (_agent, _env) = mock_env(agent).await;
        let (out, cap) = Output::memory();
        run(&args(&["s1", "--run", "r-1", "--mode", "json"]), &out)
            .await
            .expect("wait");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let report: Value = serde_json::from_str(&stdout).expect("json document");
        assert_eq!(report["sessionId"], "s1");
        assert_eq!(report["runId"], "r-1");
        assert_eq!(report["text"], "hi");
        assert!(
            report["messages"].as_array().is_some_and(|m| !m.is_empty()),
            "{stdout}"
        );
    }
}
