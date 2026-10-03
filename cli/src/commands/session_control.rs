//! `future session abort|cancel|approvals|approve|reject` — acting on a session
//! that is running, queued, or parked on an approval.
//!
//! Every other session command reads the journal. These act on the *live*
//! agent: they are the CLI counterpart of the TUI's `/stop`, `/cancel`,
//! `/approve` and `/reject`, and of the desktop's approval card. Without them a
//! headless caller could start a run (`future run`) and then have no way to
//! stop it, see what it was waiting on, or answer the question.
//!
//! They are deliberately explicit about scope: `abort` targets the whole
//! session (active run plus queue), `cancel` targets exactly one queued run by
//! id, and `approve`/`reject` name one approval request.

use crate::output::Output;
use crate::rpc::{grpc_addr, RunClient};
use serde_json::{json, Value};
use std::collections::HashSet;

pub(super) const HELP: &str = "future session — stop work and answer approvals

Usage:
  future session abort <session-id> [--json]
  future session cancel <session-id> --run <run-id> [--json]
  future session approvals <session-id> [--json]
  future session approve <session-id> <request-id> [--note <text>] [--json]
      [--allow <glob> [--access read|write]]
  future session reject <session-id> <request-id> [--note <text>] [--json]

  abort        Stop the session's active run and cancel everything queued
               behind it.
  cancel       Cancel one run that has not started yet.
  approvals    List the approval requests the session is parked on.
  approve      Allow the named request, letting its tool run.
  reject       Deny the named request, failing that tool call.

Options:
  --note <text>        Decision note recorded with an approve/reject.
  --allow <glob>       Also allow this path glob for the rest of the session
                       (the \"always allow\" choice), before approving.
  --access read|write  Access level for --allow (default read).
  --json               Agent acknowledgement as JSON.
  -h, --help           Show this help

abort and cancel reject nothing and change no settings; approvals lists what
`future session status` also shows. An approval request id comes from
`future session approvals`.";

/// Which of the five actions to take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    Abort,
    Cancel,
    Approvals,
    Approve,
    Reject,
}

#[derive(Debug, PartialEq, Eq)]
struct Options {
    action: Action,
    session: String,
    /// The approval request id (approve/reject) or the run id (cancel).
    target: Option<String>,
    note: String,
    allow: Option<String>,
    access: String,
    json: bool,
}

/// The usage line for an action, used both in errors and in `--help` order.
fn usage(action: Action) -> &'static str {
    match action {
        Action::Abort => "future session abort <session-id>",
        Action::Cancel => "future session cancel <session-id> --run <run-id>",
        Action::Approvals => "future session approvals <session-id>",
        Action::Approve => "future session approve <session-id> <request-id>",
        Action::Reject => "future session reject <session-id> <request-id>",
    }
}

fn parse(action: Action, args: &[String]) -> Result<Options, String> {
    let mut positional: Vec<String> = Vec::new();
    let mut run = None;
    let mut note = String::new();
    let mut allow = None;
    let mut access = "read".to_string();
    let mut json = false;
    let mut seen = HashSet::new();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        if flag == "--json" {
            json = true;
            index += 1;
            continue;
        }
        if !matches!(flag, "--run" | "--note" | "--allow" | "--access") {
            if flag.starts_with("--") {
                return Err(format!("unknown option: {flag}; see {}.", usage(action)));
            }
            positional.push(flag.to_string());
            index += 1;
            continue;
        }
        if !seen.insert(flag) {
            return Err(format!("duplicate option: {flag}"));
        }
        let raw = args
            .get(index + 1)
            .ok_or_else(|| format!("missing value for {flag}"))?;
        if raw.is_empty() || raw.starts_with("--") {
            return Err(format!("missing value for {flag}"));
        }
        match flag {
            "--run" => run = Some(raw.clone()),
            "--note" => note = raw.clone(),
            "--allow" => allow = Some(raw.clone()),
            _ => {
                if !matches!(raw.as_str(), "read" | "write") {
                    return Err("--access must be read or write".into());
                }
                access = raw.clone();
            }
        }
        index += 2;
    }

    if run.is_some() && action != Action::Cancel {
        return Err("--run is only valid for cancel".into());
    }
    if allow.is_some() && action != Action::Approve {
        return Err("--allow is only valid for approve".into());
    }

    let mut positional = positional.into_iter();
    let session = positional.next().ok_or("a session id is required")?;
    let mut target = positional.next();
    if positional.next().is_some() {
        return Err(format!("too many arguments; see {}.", usage(action)));
    }
    // `cancel <run-id>` and `cancel --run <run-id>` are the same request, and
    // asking for both is a mistake rather than a preference.
    if let Some(run) = run {
        if target.is_some() {
            return Err(format!(
                "cancel takes one run id, not both --run and a positional one; see {}.",
                usage(action)
            ));
        }
        target = Some(run);
    }

    let arity_ok = match action {
        Action::Abort | Action::Approvals => target.is_none(),
        Action::Cancel | Action::Approve | Action::Reject => target.is_some(),
    };
    if !arity_ok {
        return Err(format!("usage: {}", usage(action)));
    }
    Ok(Options {
        action,
        session,
        target,
        note,
        allow,
        access,
        json,
    })
}

fn print_json(out: &Output, value: Value) -> Result<(), String> {
    out.log(&serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?);
    Ok(())
}

async fn abort(options: &Options, out: &Output) -> Result<(), String> {
    let result = RunClient::new(&grpc_addr())
        .abort_session(&options.session)
        .await?;
    if options.json {
        return print_json(out, result);
    }
    let active = result["active_run_id"].as_str().unwrap_or("");
    let queued = result["queued_cancelled"].as_i64().unwrap_or(0);
    if active.is_empty() {
        out.log(&format!("Session {} has no active run.", options.session));
    } else {
        out.log(&format!(
            "Aborting session {}: run {active} is cancelling.",
            options.session
        ));
    }
    if queued > 0 {
        out.log(&format!("Cancelled {queued} queued run(s)."));
    }
    Ok(())
}

async fn cancel(options: &Options, out: &Output) -> Result<(), String> {
    let run_id = options.target.as_deref().unwrap_or_default();
    let result = RunClient::new(&grpc_addr())
        .cancel_queued_run(run_id, &options.session)
        .await?;
    if options.json {
        return print_json(out, result);
    }
    out.log(&format!("Cancelled queued run {run_id}."));
    Ok(())
}

async fn approvals(options: &Options, out: &Output) -> Result<(), String> {
    let state = RunClient::new(&grpc_addr())
        .get_state(Some(&options.session))
        .await
        .map_err(|err| super::session::missing_session_error(err, &options.session))?;
    let pending = state["pendingApprovals"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if options.json {
        return print_json(
            out,
            json!({"sessionId": options.session, "pendingApprovals": pending}),
        );
    }
    if pending.is_empty() {
        out.log(&format!(
            "Session {} is not waiting on any approval.",
            options.session
        ));
        return Ok(());
    }
    out.log(&format!(
        "Session {} has {} pending approval(s):",
        options.session,
        pending.len()
    ));
    for approval in &pending {
        // The card's own field names, with the camelCase spelling as a
        // fallback for a relay that renames them.
        let field = |snake: &str, camel: &str| -> String {
            approval[snake]
                .as_str()
                .or_else(|| approval[camel].as_str())
                .unwrap_or("")
                .to_string()
        };
        let id = field("approval_request_id", "approvalRequestId");
        let tool = field("tool_name", "toolName");
        let risk = field("risk_level", "riskLevel");
        let title = field("title", "title");
        out.log(&format!("\n  {id}"));
        out.log(&format!("    tool:   {tool}"));
        if !risk.is_empty() {
            out.log(&format!("    risk:   {risk}"));
        }
        if !title.is_empty() {
            out.log(&format!("    title:  {title}"));
        }
        let summary = field("summary", "summary");
        if !summary.is_empty() {
            out.log(&format!("    detail: {summary}"));
        }
        let action = &approval["requested_action"];
        if !action.is_null() {
            out.log(&format!("    action: {action}"));
        }
        out.log(&format!(
            "    decide: future session approve {} {id}",
            options.session
        ));
    }
    Ok(())
}

async fn decide(options: &Options, approved: bool, out: &Output) -> Result<(), String> {
    let request_id = options.target.as_deref().unwrap_or_default();
    let client = RunClient::new(&grpc_addr());
    // The "always allow" rule is installed first: it is the same-run
    // counterpart of the decision, so the tool that runs after approval must
    // already see it. A rule failure aborts the decision rather than approving
    // a call whose standing permission the caller asked to change.
    if let Some(glob) = &options.allow {
        client
            .add_session_rule(glob, &options.access, &options.session)
            .await?;
    }
    let mode = if approved { "approved" } else { "rejected" };
    let result = client
        .approval_decision(request_id, mode, &options.note, &options.session)
        .await;
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            // The rule was already installed (it has to precede the decision so
            // the approved tool sees it). Say so rather than letting a failure
            // hide a permission that is now standing.
            if let Some(glob) = &options.allow {
                out.log_err(&format!(
                    "The approval was not decided ({error}), but {} access to {glob} \
                     was already allowed for this session.",
                    options.access
                ));
                return Err(crate::HANDLED_EXIT.to_string());
            }
            return Err(error);
        }
    };
    if options.json {
        let mut value = result;
        if let Some(glob) = &options.allow {
            value["rule"] = json!({"pattern": glob, "access": options.access});
        }
        return print_json(out, value);
    }
    out.log(&format!(
        "{} approval request {request_id}.",
        if approved { "Approved" } else { "Rejected" }
    ));
    if let Some(glob) = &options.allow {
        out.log(&format!(
            "Allowed {} access to {glob} for this session.",
            options.access
        ));
    }
    if !options.note.is_empty() {
        out.log(&format!("Note: {}", options.note));
    }
    Ok(())
}

pub(super) async fn run(action: Action, args: &[String], out: &Output) -> Result<(), String> {
    let options = parse(action, args)?;
    match options.action {
        Action::Abort => abort(&options, out).await,
        Action::Cancel => cancel(&options, out).await,
        Action::Approvals => approvals(&options, out).await,
        Action::Approve => decide(&options, true, out).await,
        Action::Reject => decide(&options, false, out).await,
    }
}

/// The subcommands this module owns, so `session.rs` dispatches one list.
pub(super) const SUBCOMMANDS: &[(&str, Action)] = &[
    ("abort", Action::Abort),
    ("cancel", Action::Cancel),
    ("approvals", Action::Approvals),
    ("approve", Action::Approve),
    ("reject", Action::Reject),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::MockAgent;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    async fn spawn(agent: MockAgent) -> (MockAgent, crate::test_env::EnvGuard) {
        let addr = crate::test_server::spawn_mock(agent.clone()).await;
        let env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from(addr),
        )]);
        (agent, env)
    }

    #[test]
    fn abort_and_approvals_take_only_a_session() {
        let parsed = parse(Action::Abort, &args(&["s1"])).unwrap();
        assert_eq!(parsed.session, "s1");
        assert!(parsed.target.is_none());
        assert!(parse(Action::Abort, &args(&[])).is_err());
        assert!(
            parse(Action::Abort, &args(&["s1", "extra"])).is_err(),
            "abort takes no extra positional"
        );
        assert!(
            parse(Action::Approvals, &args(&["s1", "--json"]))
                .unwrap()
                .json
        );
    }

    #[test]
    fn cancel_accepts_a_positional_or_a_run_flag() {
        let positional = parse(Action::Cancel, &args(&["s1", "run-1"])).unwrap();
        assert_eq!(positional.target.as_deref(), Some("run-1"));
        let flagged = parse(Action::Cancel, &args(&["s1", "--run", "run-2"])).unwrap();
        assert_eq!(flagged.target.as_deref(), Some("run-2"));
        assert!(
            parse(Action::Cancel, &args(&["s1", "run-1", "--run", "run-2"])).is_err(),
            "one run id, not two"
        );
        assert!(parse(Action::Cancel, &args(&["s1"])).is_err());
    }

    #[test]
    fn approve_and_reject_require_a_request_id() {
        let parsed = parse(
            Action::Approve,
            &args(&[
                "s1", "ap-1", "--note", "ok", "--allow", "/tmp/**", "--access", "write",
            ]),
        )
        .unwrap();
        assert_eq!(parsed.target.as_deref(), Some("ap-1"));
        assert_eq!(parsed.note, "ok");
        assert_eq!(parsed.allow.as_deref(), Some("/tmp/**"));
        assert_eq!(parsed.access, "write");
        assert!(parse(Action::Approve, &args(&["s1"])).is_err());
        assert!(parse(Action::Reject, &args(&["s1"])).is_err());
        assert!(parse(Action::Reject, &args(&["s1", "ap-1", "extra"])).is_err());
    }

    #[test]
    fn option_guards_reject_misuse() {
        assert!(parse(Action::Reject, &args(&["s1", "ap-1", "--allow", "/x"])).is_err());
        assert!(parse(Action::Abort, &args(&["s1", "--run", "r"])).is_err());
        assert!(parse(Action::Approve, &args(&["s1", "ap-1", "--access", "rw"])).is_err());
        assert!(parse(Action::Approve, &args(&["s1", "ap-1", "--bogus"])).is_err());
        assert!(parse(Action::Approve, &args(&["s1", "ap-1", "--note"])).is_err());
        assert!(
            parse(
                Action::Approve,
                &args(&["s1", "ap-1", "--note", "a", "--note", "b"])
            )
            .is_err(),
            "duplicate options are rejected, not last-wins"
        );
    }

    #[tokio::test]
    async fn abort_reports_the_active_run_and_queue() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = MockAgent::default();
        agent.responses.insert(
            "abort_session".into(),
            json!({"active_run_id": "run-9", "queued_cancelled": 2, "state": "cancelling"})
                .to_string(),
        );
        let (agent, _env) = spawn(agent).await;
        let (out, cap) = Output::memory();
        run(Action::Abort, &args(&["s1"]), &out)
            .await
            .expect("abort");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("run run-9 is cancelling"), "{stdout}");
        assert!(stdout.contains("Cancelled 2 queued run(s)."), "{stdout}");
        assert_eq!(agent.seen_of("abort_session")[0].session_id, "s1");
    }

    #[tokio::test]
    async fn abort_with_nothing_running_says_so() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = MockAgent::default();
        agent.responses.insert(
            "abort_session".into(),
            json!({"active_run_id": null, "queued_cancelled": 0}).to_string(),
        );
        let (_agent, _env) = spawn(agent).await;
        let (out, cap) = Output::memory();
        run(Action::Abort, &args(&["s1"]), &out)
            .await
            .expect("abort");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("has no active run"), "{stdout}");
        assert!(!stdout.contains("Cancelled"), "{stdout}");
    }

    #[tokio::test]
    async fn cancel_sends_the_run_id() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = MockAgent::default();
        agent.responses.insert(
            "cancel_queued_run".into(),
            json!({"run_id": "run-3", "state": "cancelled"}).to_string(),
        );
        let (agent, _env) = spawn(agent).await;
        let (out, cap) = Output::memory();
        run(Action::Cancel, &args(&["s1", "--run", "run-3"]), &out)
            .await
            .expect("cancel");
        let sent = agent.seen_of("cancel_queued_run");
        assert_eq!(sent[0].run_id, "run-3");
        assert_eq!(sent[0].session_id, "s1");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("Cancelled queued run run-3"));
    }

    #[tokio::test]
    async fn cancel_surfaces_the_agents_refusal() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = MockAgent::default();
        agent
            .fail_with
            .insert("cancel_queued_run".into(), "run is not queued".into());
        let (_agent, _env) = spawn(agent).await;
        let (out, _cap) = Output::memory();
        let error = run(Action::Cancel, &args(&["s1", "--run", "run-x"]), &out)
            .await
            .unwrap_err();
        assert!(error.contains("not queued"), "{error}");
    }

    fn pending_state() -> Value {
        json!({
            "sessionId": "s1",
            "pendingApprovals": [{
                "approval_request_id": "ap-1",
                "tool_name": "shell",
                "kind": "tool",
                "risk_level": "high",
                "title": "Run a shell command",
                "summary": "rm -rf build",
                "requested_action": {"command": "rm -rf build"}
            }]
        })
    }

    #[tokio::test]
    async fn approvals_lists_the_pending_card() {
        let _guard = crate::test_env::lock_env().await;
        let agent = MockAgent::respond("get_state", &pending_state().to_string());
        let (_agent, _env) = spawn(agent).await;
        let (out, cap) = Output::memory();
        run(Action::Approvals, &args(&["s1"]), &out)
            .await
            .expect("approvals");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("1 pending approval(s)"), "{stdout}");
        assert!(stdout.contains("ap-1"), "{stdout}");
        assert!(stdout.contains("tool:   shell"), "{stdout}");
        assert!(stdout.contains("risk:   high"), "{stdout}");
        assert!(stdout.contains("rm -rf build"), "{stdout}");
        assert!(
            stdout.contains("future session approve s1 ap-1"),
            "the next command must be copy-pasteable: {stdout}"
        );
    }

    #[tokio::test]
    async fn approvals_says_so_when_there_is_nothing_pending() {
        let _guard = crate::test_env::lock_env().await;
        let agent = MockAgent::respond(
            "get_state",
            &json!({"sessionId": "s1", "pendingApprovals": []}).to_string(),
        );
        let (_agent, _env) = spawn(agent).await;
        let (out, cap) = Output::memory();
        run(Action::Approvals, &args(&["s1"]), &out)
            .await
            .expect("approvals");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("not waiting on any approval"), "{stdout}");
    }

    #[tokio::test]
    async fn approvals_json_is_machine_readable() {
        let _guard = crate::test_env::lock_env().await;
        let agent = MockAgent::respond("get_state", &pending_state().to_string());
        let (_agent, _env) = spawn(agent).await;
        let (out, cap) = Output::memory();
        run(Action::Approvals, &args(&["s1", "--json"]), &out)
            .await
            .expect("approvals");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(parsed["sessionId"], "s1");
        assert_eq!(parsed["pendingApprovals"][0]["approval_request_id"], "ap-1");
    }

    #[tokio::test]
    async fn approve_decides_with_the_requested_mode() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = MockAgent::default();
        agent.responses.insert(
            "approval_decision".into(),
            json!({"approvalRequestId": "ap-1", "status": "approved"}).to_string(),
        );
        let (agent, _env) = spawn(agent).await;
        let (out, cap) = Output::memory();
        run(
            Action::Approve,
            &args(&["s1", "ap-1", "--note", "looks fine"]),
            &out,
        )
        .await
        .expect("approve");
        let sent = agent.seen_of("approval_decision");
        assert_eq!(sent[0].entry_id, "ap-1");
        assert_eq!(sent[0].mode, "approved");
        assert_eq!(sent[0].message, "looks fine");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(
            stdout.contains("Approved approval request ap-1."),
            "{stdout}"
        );
        assert!(stdout.contains("Note: looks fine"));
        // No rule was requested, so none may be installed.
        assert!(agent.seen_of("add_session_rule").is_empty());
    }

    #[tokio::test]
    async fn reject_sends_rejected() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = MockAgent::default();
        agent
            .responses
            .insert("approval_decision".into(), json!({}).to_string());
        let (agent, _env) = spawn(agent).await;
        let (out, cap) = Output::memory();
        run(Action::Reject, &args(&["s1", "ap-2"]), &out)
            .await
            .expect("reject");
        assert_eq!(agent.seen_of("approval_decision")[0].mode, "rejected");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("Rejected approval request ap-2."));
    }

    #[tokio::test]
    async fn approve_with_allow_installs_the_rule_first() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = MockAgent::default();
        agent
            .responses
            .insert("approval_decision".into(), json!({}).to_string());
        agent
            .responses
            .insert("add_session_rule".into(), json!({}).to_string());
        let (agent, _env) = spawn(agent).await;
        let (out, cap) = Output::memory();
        run(
            Action::Approve,
            &args(&["s1", "ap-1", "--allow", "/work/**", "--access", "write"]),
            &out,
        )
        .await
        .expect("approve");
        let rule = agent.seen_of("add_session_rule");
        assert_eq!(rule[0].message, "/work/**");
        assert_eq!(rule[0].mode, "write");
        // Order matters: the rule must precede the decision so the approved
        // tool already sees it.
        let seen = agent.seen.lock().unwrap();
        let rule_at = seen
            .iter()
            .position(|c| c.r#type == "add_session_rule")
            .expect("rule sent");
        let decide_at = seen
            .iter()
            .position(|c| c.r#type == "approval_decision")
            .expect("decision sent");
        assert!(
            rule_at < decide_at,
            "rule must be installed before the decision"
        );
        drop(seen);
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("Allowed write access to /work/** for this session."));
    }

    /// The rule is installed before the decision, so a decision that then fails
    /// has already broadened the session's permissions. That must be reported
    /// rather than hidden behind the error.
    #[tokio::test]
    async fn a_failed_decision_reports_the_rule_it_left_behind() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = MockAgent::default();
        agent
            .responses
            .insert("add_session_rule".into(), json!({}).to_string());
        agent.fail_with.insert(
            "approval_decision".into(),
            "approval request `ap-9` is not pending".into(),
        );
        let (_agent, _env) = spawn(agent).await;
        let (out, cap) = Output::memory();
        let result = run(
            Action::Approve,
            &args(&["s1", "ap-9", "--allow", "/work/**"]),
            &out,
        )
        .await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.contains("is not pending"), "{stderr}");
        assert!(
            stderr.contains("was already allowed for this session"),
            "the standing permission must be reported: {stderr}"
        );

        // Without --allow there is no side effect to report, so the agent's
        // own error passes through untouched.
        let mut agent = MockAgent::default();
        agent.fail_with.insert(
            "approval_decision".into(),
            "approval request `ap-9` is not pending".into(),
        );
        let (_agent, _env) = spawn(agent).await;
        let (out, _cap) = Output::memory();
        let error = run(Action::Approve, &args(&["s1", "ap-9"]), &out)
            .await
            .unwrap_err();
        assert_eq!(error, "approval request `ap-9` is not pending");
    }

    #[tokio::test]
    async fn a_failed_rule_aborts_the_decision() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = MockAgent::default();
        agent
            .fail_with
            .insert("add_session_rule".into(), "bad glob".into());
        agent
            .responses
            .insert("approval_decision".into(), json!({}).to_string());
        let (agent, _env) = spawn(agent).await;
        let (out, _cap) = Output::memory();
        let error = run(
            Action::Approve,
            &args(&["s1", "ap-1", "--allow", "["]),
            &out,
        )
        .await
        .unwrap_err();
        assert!(error.contains("bad glob"), "{error}");
        assert!(
            agent.seen_of("approval_decision").is_empty(),
            "a request whose standing rule failed must not be approved"
        );
    }

    #[tokio::test]
    async fn approve_json_reports_the_rule_it_installed() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = MockAgent::default();
        agent
            .responses
            .insert("approval_decision".into(), json!({}).to_string());
        agent
            .responses
            .insert("add_session_rule".into(), json!({}).to_string());
        let (_agent, _env) = spawn(agent).await;
        let (out, cap) = Output::memory();
        run(
            Action::Approve,
            &args(&["s1", "ap-1", "--allow", "/work/**", "--json"]),
            &out,
        )
        .await
        .expect("approve");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(parsed["rule"]["pattern"], "/work/**");
        assert_eq!(parsed["rule"]["access"], "read");
    }

    #[tokio::test]
    async fn decide_surfaces_a_stale_request() {
        let _guard = crate::test_env::lock_env().await;
        let mut agent = MockAgent::default();
        agent.fail_with.insert(
            "approval_decision".into(),
            "approval request ap-9 is not pending".into(),
        );
        let (_agent, _env) = spawn(agent).await;
        let (out, _cap) = Output::memory();
        let error = run(Action::Approve, &args(&["s1", "ap-9"]), &out)
            .await
            .unwrap_err();
        assert!(error.contains("not pending"), "{error}");
    }
}
