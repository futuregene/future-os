//! User-facing async compact command. It does not wait inside an active model run.
use crate::output::Output;
use crate::rpc::{grpc_addr, RunClient};
use std::collections::HashSet;

const HELP: &str = "future session compact — request manual session compaction

Usage:
  future session compact --session <id> [--instructions <text>] [--json]

Options:
  --session <id>          Required, explicit session to compact
  --instructions <text>  Verbatim continuation note (C does not interpret it)
  --json                 Print the Agent acknowledgement as JSON
  -h, --help             Show this help

This command returns an asynchronous acknowledgement, NOT a completed summary.
The Agent reports completion/reuse/failure through its compaction events.
Compaction keeps every protected original, adds a fixed-budget evidence index, and
asks the session model for a handoff summary that carries the previous summary
forward. If the model is unreachable the evidence index alone is committed.
Active runs are rejected. Identical persisted history and parameters reuse the
Agent's durable receipt; this command does not replay tools or bypass budgets.
An unknown transport outcome must not be interpreted as success or cancellation.";

#[derive(Debug, PartialEq, Eq)]
struct Options {
    session: String,
    instructions: String,
    json: bool,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut session = None;
    let mut instructions = String::new();
    let mut json = false;
    let mut seen = HashSet::new();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        if !seen.insert(flag) {
            return Err(format!("duplicate option: {flag}"));
        }
        match flag {
            "--json" => {
                json = true;
                index += 1;
            }
            "--session" | "--instructions" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| format!("missing value for {flag}"))?;
                if value.starts_with("--") {
                    return Err(format!("missing value for {flag}"));
                }
                if flag == "--session" {
                    if value.trim().is_empty() {
                        return Err("--session must not be empty".into());
                    }
                    session = Some(value.clone());
                } else {
                    instructions = value.clone();
                }
                index += 2;
            }
            _ => {
                return Err(format!(
                    "unknown option: {flag}; see future session compact --help"
                ))
            }
        }
    }
    Ok(Options {
        session: session.ok_or("--session is required")?,
        instructions,
        json,
    })
}

pub(super) async fn run(args: &[String], out: &Output) -> Result<(), String> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        out.log(HELP);
        return Ok(());
    }
    let options = parse(args)?;
    let result = RunClient::new(&grpc_addr())
        .compact_session(&options.session, &options.instructions)
        .await?;
    let operation = result
        .get("operationId")
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty());
    if result.get("accepted").and_then(|v| v.as_bool()) != Some(true) || operation.is_none() {
        return Err(
            "invalid compact acknowledgement: expected accepted=true and operationId".into(),
        );
    }
    if options.json {
        out.log(&serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?);
    } else {
        out.log(&format!("Compaction request accepted.\nSession:   {}\nOperation: {}\nThis is an acknowledgement, not completion. Follow the Agent's compaction events for the final result.",options.session,operation.unwrap()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn options_are_explicit_and_do_not_offer_model_force_or_wait() {
        assert_eq!(
            parse(&args(&[
                "--session",
                "s",
                "--instructions",
                "保留约束与错误",
                "--json"
            ]))
            .unwrap(),
            Options {
                session: "s".into(),
                instructions: "保留约束与错误".into(),
                json: true
            }
        );
        for input in [
            vec![],
            vec!["s"],
            vec!["--session"],
            vec!["--session", " "],
            vec!["--session", "a", "--session", "b"],
            vec!["--session", "s", "--instructions"],
            vec!["--session", "s", "--force"],
            vec!["--session", "s", "--wait"],
            vec!["request", "--session", "s"],
        ] {
            assert!(parse(&args(&input)).is_err(), "{input:?}");
        }
    }

    #[tokio::test]
    async fn help_needs_no_server() {
        let (out, cap) = Output::memory();
        run(&args(&["--help"]), &out).await.unwrap();
        assert!(String::from_utf8(cap.out.lock().unwrap().clone())
            .unwrap()
            .contains("NOT a completed summary"));
    }

    #[tokio::test]
    async fn dispatch_sends_scoped_instructions_and_prints_ack_only() {
        let _guard = crate::test_env::lock_env().await;
        let agent = crate::test_server::MockAgent::respond(
            "compact",
            r#"{"accepted":true,"operationId":"cmp-one"}"#,
        );
        let address = crate::test_server::spawn_mock(agent.clone()).await;
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from(address),
        )]);
        let (out, cap) = Output::memory();
        crate::commands::session::session(
            Some("compact"),
            &args(&[
                "--session",
                "s",
                "--instructions",
                "原样保留 128MiB",
                "--json",
            ]),
            &out,
        )
        .await
        .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&cap.out.lock().unwrap()).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"accepted":true,"operationId":"cmp-one"})
        );
        let first = agent.seen_of("compact");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].session_id, "s");
        assert_eq!(first[0].custom_instructions, "原样保留 128MiB");
        let (out, cap) = Output::memory();
        run(&args(&["--session", "s"]), &out).await.unwrap();
        assert!(String::from_utf8(cap.out.lock().unwrap().clone())
            .unwrap()
            .contains("not completion"));
        let seen = agent.seen_of("compact");
        assert_ne!(seen[0].id, seen[1].id);
        assert!(agent.seen_of("switch_session").is_empty());
    }

    #[tokio::test]
    async fn busy_and_transport_errors_are_not_success() {
        let _guard = crate::test_env::lock_env().await;
        for (response, error) in [
            (r#"{}"#, None),
            (r#"{"accepted":true}"#, None),
            (
                r#"{}"#,
                Some("finish or stop the active run before manual compaction"),
            ),
        ] {
            let mut agent = crate::test_server::MockAgent::respond("compact", response);
            if let Some(error) = error {
                agent.fail_with.insert("compact".into(), error.into());
            }
            let address = crate::test_server::spawn_mock(agent).await;
            let _env = crate::test_env::EnvGuard::set(&[(
                "FUTURE_AGENT_GRPC_ADDR",
                std::ffi::OsString::from(address),
            )]);
            let (out, cap) = Output::memory();
            let error = run(&args(&["--session", "s"]), &out).await.unwrap_err();
            assert!(!error.is_empty());
            assert!(cap.out.lock().unwrap().is_empty());
        }
        let mut agent = crate::test_server::MockAgent::default();
        agent.status_message_types.insert("compact".into());
        let address = crate::test_server::spawn_mock(agent).await;
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from(address),
        )]);
        let (out, _) = Output::memory();
        assert!(run(&args(&["--session", "s"]), &out)
            .await
            .unwrap_err()
            .contains("transport down"));
    }
}
