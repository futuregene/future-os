//! `future session unblock <session-id> [--why <text>]` — release a
//! restart-recovery fence.
//!
//! When the agent restarts while a mutating tool call is in flight, that call's
//! effect is unknown and the Agent stops trusting *every* mutating tool in the
//! session — `write`, `edit` and `shell` are all refused, `read` still works.
//! The fence is there for a reason (nobody watched that shell finish, and
//! silently replaying it is how a half-applied change gets applied twice), so
//! it is never released automatically. This command is the human decision: it
//! records, in the session's own journal, which calls were released and by
//! whom.
//!
//! What it does *not* do: it does not replay the interrupted call and it does
//! not claim the call succeeded. That one operation stays questioned once per
//! run — the model is told to verify it read-only first — and the rest of the
//! session's tools work again. Running it twice is a no-op, so a retried
//! command cannot double-record anything.

use super::session::missing_session_error;
use crate::output::Output;
use crate::rpc::{grpc_addr, RunClient};
use serde_json::Value;

const HELP: &str = "future session unblock — release a session fenced by an interrupted operation

Usage:
  future session unblock <session-id> [--why <text>] [--json]

Why a session is fenced: the Agent was restarted (or killed) while a tool call
was in flight. Its result never reached the journal, so whether the operation
took effect is unknown. Rather than risk replaying it, the Agent refuses every
mutating tool for the rest of that session — only `read` is allowed — and every
attempt returns the same [RECOVERY_UNCERTAIN] message.

  unblock       Release the fence for exactly the interrupted calls. They stay
                questioned: the next attempt at one of them warns the model to
                check the result read-only first, then allows it. Every other
                tool works again from the next run.

Options:
  --why <text>  Optional reason recorded with the acknowledgement.
  --json        Agent acknowledgement as JSON.
  -h, --help    Show this help

Nothing is replayed and nothing is marked verified: the interrupted operation
still may or may not have happened, and the model is told so. Nothing already
in the session history is rewritten — the acknowledgement is one appended
entry. Running unblock on an unfenced (or already released) session reports
that there was nothing to release.
";

/// `custom` entry name the Agent records the acknowledgement under. Quoted in
/// the output so a reader can find the entry in the transcript.
const ENTRY_NAME: &str = "recovery_acknowledged";

pub(super) async fn run(args: &[String], out: &Output) -> Result<(), String> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        out.log(HELP);
        return Ok(());
    }
    let mut json_flag = false;
    let mut why = String::new();
    let mut target: Option<String> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                json_flag = true;
                index += 1;
            }
            "--why" => {
                let Some(value) = args.get(index + 1) else {
                    out.log_err("Missing value for --why");
                    return Err(crate::HANDLED_EXIT.to_string());
                };
                why = value.clone();
                index += 2;
            }
            other if other.starts_with("--") => {
                out.log_err(&format!("Unknown option: {other}"));
                return Err(crate::HANDLED_EXIT.to_string());
            }
            other => {
                if target.is_some() {
                    out.log_err("unblock takes exactly one session id");
                    return Err(crate::HANDLED_EXIT.to_string());
                }
                target = Some(other.to_string());
                index += 1;
            }
        }
    }
    let Some(session_id) = target else {
        out.log_err("Usage: future session unblock <session-id> [--why <text>]");
        return Err(crate::HANDLED_EXIT.to_string());
    };

    let client = RunClient::new(&grpc_addr());
    let data = client
        .acknowledge_recovery(&session_id, &why)
        .await
        .map_err(|err| missing_session_error(err, &session_id))?;
    if json_flag {
        out.log(&serde_json::to_string_pretty(&data).unwrap_or_default());
        return Ok(());
    }
    out.log(&report(&session_id, &data));
    Ok(())
}

fn report(session_id: &str, data: &Value) -> String {
    let released = data["released"].as_array().cloned().unwrap_or_default();
    if released.is_empty() {
        return format!(
            "Session {session_id} is not fenced: nothing to release.\n\
             (A session that has already been released reports the same thing — this is a no-op.)"
        );
    }
    let mut lines = vec![format!(
        "Released the recovery fence on session {session_id} for {} operation(s):",
        released.len()
    )];
    for call in &released {
        let tool = call["tool"].as_str().unwrap_or("operation");
        let call_id = call["call_id"].as_str().unwrap_or("");
        let run_id = call["run_id"].as_str().unwrap_or("");
        lines.push(format!("  {tool} ({call_id}) — run {run_id}"));
    }
    lines.push(String::new());
    lines.push(
        "Each released operation is still questioned once: the next attempt warns the model to \
         verify it read-only, then allows it. Every other tool works again on the next run."
            .to_string(),
    );
    lines.push(format!(
        "Nothing was replayed and nothing was marked verified. Recorded as a `{ENTRY_NAME}` \
         journal entry; remove nothing from the session — it was appended, not rewritten."
    ));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::MockAgent;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn report_names_every_released_operation_with_its_call_id() {
        // The call id must survive into the output: it is what a later
        // transcript reading needs to find the interrupted call.
        let data = serde_json::json!({
            "released": [
                {"run_id":"run-1","call_id":"call-00","tool":"shell"},
                {"run_id":"run-1","call_id":"call-01","tool":"write"},
            ],
            "already_released": false,
            "replayed": false,
        });
        let text = report("s1", &data);
        assert!(text.contains("2 operation(s)"), "{text}");
        assert!(text.contains("shell (call-00)"), "{text}");
        assert!(text.contains("write (call-01)"), "{text}");
        assert!(text.contains("run-1"), "{text}");
        assert!(text.contains("Nothing was replayed"), "{text}");
    }

    #[test]
    fn an_unfenced_session_says_there_was_nothing_to_release() {
        let data = serde_json::json!({"released": [], "already_released": true});
        let text = report("s1", &data);
        assert!(text.contains("nothing to release"), "{text}");
        assert!(!text.contains("Released the recovery fence"), "{text}");
    }

    #[tokio::test]
    async fn unblock_sends_the_reason_and_reports_the_release() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = MockAgent::default();
        agent.responses.insert(
            "acknowledge_recovery".into(),
            serde_json::json!({
                "released": [{"run_id":"run-1","call_id":"call-9","tool":"shell"}],
                "already_released": false,
                "replayed": false,
            })
            .to_string(),
        );
        let addr = crate::test_server::spawn_mock(agent.clone()).await;
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from(addr),
        )]);
        let (out, cap) = Output::memory();
        run(&args(&["s1", "--why", "checked the build dir"]), &out)
            .await
            .expect("unblock");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("shell (call-9)"), "{stdout}");
        let seen = agent.seen_of("acknowledge_recovery");
        assert_eq!(seen[0].session_id, "s1");
        assert_eq!(seen[0].message, "checked the build dir");
    }

    #[tokio::test]
    async fn unblock_requires_exactly_one_session_id() {
        let (out, cap) = Output::memory();
        assert!(run(&args(&[]), &out).await.is_err());
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.contains("Usage: future session unblock"), "{stderr}");

        let (out, cap) = Output::memory();
        assert!(run(&args(&["s1", "s2"]), &out).await.is_err());
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.contains("exactly one session id"), "{stderr}");

        let (out, cap) = Output::memory();
        assert!(run(&args(&["s1", "--why"]), &out).await.is_err());
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.contains("Missing value for --why"), "{stderr}");
    }

    #[test]
    fn help_is_reachable() {
        assert!(HELP.contains("future session unblock <session-id>"));
        assert!(HELP.contains("--why"));
        // The help must both name the command and say it is not a replay.
        assert!(HELP.contains("Nothing is replayed"), "{HELP}");
    }
}
