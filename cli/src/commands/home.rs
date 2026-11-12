//! `future` with no arguments — the live home view.
//!
//! AXI principle 8 ("content first"): running the binary bare must show live,
//! actionable state rather than a wall of help text, because a help screen tells
//! an agent what could be done and nothing about what *is*. So bare `future`
//! answers the questions an agent actually opens with — is the agent running,
//! which model, how many conversations, is anything scheduled, what is installed
//! — in one round trip, and ends with the concrete next commands.
//!
//! `future --help` (and `-h`, and `future help`) still prints exactly the help it
//! always did, which is the fallback principle 10 asks for.
//!
//! Nothing here is allowed to fail the command. A missing agent, an unreadable
//! settings file or a locked task database is a *state*, reported on its own
//! line; only information that cannot be obtained is omitted, because a home
//! view that exits non-zero is useless to the agent that most needs it.

use crate::axi;
use crate::output::Output;
use future_rpc::home::future_home;
use future_tasks::{Store, TriggerKind};
use std::time::Duration;

/// How long the home view waits for the agent before reporting it as not
/// running. It is a state check, not a query: a slow agent must not turn into a
/// slow `future`.
const AGENT_PROBE: Duration = Duration::from_secs(3);

const DESCRIPTION: &str = "one agent everywhere — terminal (TUI), desktop, mobile, CLI and IM bots";

/// `future` — print the home view.
pub async fn home(out: &Output) {
    out.log(&format!("bin:         {}", axi::bin_path()));
    out.log(&format!("description: {DESCRIPTION}"));
    out.log(&format!(
        "version:     v{} ({}, {})",
        crate::version::VERSION,
        build_platform(),
        build_profile()
    ));
    out.log(&format!("home:        {}", axi::tilde(&future_home())));

    let state = agent_state().await;
    match &state {
        Some((sessions, newest_age)) => {
            out.log(&format!("agent:       running ({})", agent_endpoint()));
            out.log(&format!("sessions:    {sessions}{newest_age}"));
        }
        None => {
            out.log("agent:       not reachable");
            out.log("sessions:    (unknown — the agent is not running)");
        }
    }

    out.log(&format!("model:       {}", model_line()));
    out.log(&format!("tasks:       {}", tasks_line()));
    out.log(&format!("skills:      {}", skills_line()));

    let mut hints = vec![
        "Run `future session list` to see recent conversations.",
        "Run `future task list` to see scheduled tasks.",
    ];
    if state.is_none() {
        // First thing to fix, so it leads.
        hints.insert(0, "Run `future agent` to start the agent.");
    }
    hints.push("Run `future doctor` to check the environment.");
    hints.push("Run `future --help` for every group and app.");
    axi::hints(out, &hints);
}

/// The agent's session count, plus the age of the newest one when there is one.
///
/// One call answers both "is the agent up" and "how much is there": a probe
/// command would cost a round trip to say nothing an agent needs.
async fn agent_state() -> Option<(usize, String)> {
    let client = crate::rpc::RunClient::new(&crate::rpc::grpc_addr());
    let data = tokio::time::timeout(AGENT_PROBE, client.list_sessions())
        .await
        .ok()?
        .ok()?;
    let sessions = data
        .get("sessions")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let newest = sessions
        .iter()
        .filter_map(|s| s["updatedAtMs"].as_i64())
        .max();
    let age = match newest {
        Some(ms) => format!(" total, newest {}", ago(ms)),
        None => " total".to_string(),
    };
    Some((sessions.len(), age))
}

/// The endpoint the home view is talking to, in the form a user would type.
fn agent_endpoint() -> String {
    let addr = crate::rpc::grpc_addr();
    if addr == future_rpc::transport::AUTO_ENDPOINT {
        axi::tilde(std::path::Path::new(
            future_rpc::transport::local_endpoint_label()
                .trim_start_matches("unix://")
                .trim_start_matches("npipe://"),
        ))
    } else {
        addr
    }
}

/// The configured default model, or the fact that none is chosen.
fn model_line() -> String {
    match crate::commands::settings::default_model() {
        Ok(model) if !model.is_empty() => model,
        _ => "(not chosen — Run `future config`) ".trim_end().to_string(),
    }
}

/// `N enabled, next "<name>" in 12m`, or the fact that nothing is scheduled.
fn tasks_line() -> String {
    let Ok(Some(store)) = Store::open_existing_read_only(&future_home()) else {
        return "none — Run `future task add`".to_string();
    };
    let Ok(tasks) = store.list_tasks(false) else {
        return "(the task list could not be read)".to_string();
    };
    let enabled: Vec<_> = tasks.iter().filter(|task| task.enabled).collect();
    if enabled.is_empty() {
        return "none enabled — Run `future task add`".to_string();
    }
    let now = now_ms();
    let next = enabled
        .iter()
        .filter(|task| task.trigger_kind == TriggerKind::Schedule)
        .filter_map(|task| task.next_due_at.map(|due| (due, task.name.clone())))
        .min_by_key(|(due, _)| *due);
    match next {
        Some((due, name)) => format!(
            "{} enabled, next \"{name}\" {}",
            enabled.len(),
            until(due, now)
        ),
        None => format!("{} enabled, none scheduled", enabled.len()),
    }
}

/// `N installed` for the skills directory, counted without reading the
/// catalogue (which lives on the platform and needs the network).
fn skills_line() -> String {
    let dir = crate::commands::skills::skills_dir();
    let installed = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.path().join("SKILL.md").is_file())
                .count()
        })
        .unwrap_or(0);
    match installed {
        0 => format!(
            "0 installed — Run `future skills install-builtin` ({})",
            axi::tilde(&dir)
        ),
        n => format!("{n} installed ({})", axi::tilde(&dir)),
    }
}

fn build_platform() -> String {
    crate::version::build_info()["buildTarget"]
        .as_str()
        .unwrap_or("unknown")
        .to_string()
}

fn build_profile() -> String {
    crate::version::build_info()["buildProfile"]
        .as_str()
        .unwrap_or("unknown")
        .to_string()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `3m ago` — the same wording `future session list` uses, so the two views
/// never disagree about when something last happened.
fn ago(ms: i64) -> String {
    crate::commands::session::relative_age(ms, now_ms())
}

/// `in 12m` for a time still ahead, `overdue by 4m` for one already past (a
/// scheduled task the host has not picked up yet is worth seeing as such).
fn until(due: i64, now: i64) -> String {
    if due <= now {
        return format!("overdue by {}", span(now - due));
    }
    format!("in {}", span(due - now))
}

fn span(ms: i64) -> String {
    let mins = ms.max(0) / 60_000;
    if mins < 1 {
        "<1m".to_string()
    } else if mins < 60 {
        format!("{mins}m")
    } else if mins < 60 * 24 {
        format!("{}h", mins / 60)
    } else {
        format!("{}d", mins / (60 * 24))
    }
}

/// `2026-11-10 14:03` in local time — used by the tests below to keep the
/// relative wording checked against a fixed clock rather than the wall clock.
#[cfg(test)]
fn formatted_local(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env::{lock_env, EnvGuard};

    #[test]
    fn relative_spans_read_the_way_a_person_says_them() {
        let now = 1_700_000_000_000;
        assert_eq!(until(now + 30_000, now), "in <1m");
        assert_eq!(until(now + 12 * 60_000, now), "in 12m");
        assert_eq!(until(now + 3 * 3_600_000, now), "in 3h");
        assert_eq!(until(now + 2 * 86_400_000, now), "in 2d");
        assert_eq!(until(now - 4 * 60_000, now), "overdue by 4m");
        // The formatting helper agrees with the wall clock it stands in for.
        assert!(
            formatted_local(now).starts_with("2023-1"),
            "{}",
            formatted_local(now)
        );
    }

    /// The home view always succeeds: with no agent it says so instead of
    /// failing, which is the whole point of a state view.
    #[tokio::test]
    async fn home_reports_state_without_failing() {
        let _guard = lock_env().await;
        let _home = EnvGuard::temp_home();
        let _addr = EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from("127.0.0.1:1"),
        )]);
        let (out, captured) = Output::memory();
        home(&out).await;
        assert_eq!(out.exit_code(), 0, "a state view never fails");
        let text = String::from_utf8(captured.out.lock().unwrap().clone()).unwrap();
        assert!(text.contains("bin:"), "{text}");
        assert!(text.contains("agent:       not reachable"), "{text}");
        assert!(text.contains("skills:"), "{text}");
        // The next command to fix leads the hints.
        assert!(
            text.contains("help[5]:\n  Run `future agent` to start the agent."),
            "{text}"
        );
        assert!(
            text.contains("Run `future --help` for every group"),
            "{text}"
        );
    }

    /// An empty home is a definitive state, not a blank line: the two lines a
    /// fresh install could otherwise render as empty say what to run.
    #[tokio::test]
    async fn task_and_skill_lines_state_their_empty_case() {
        let _guard = lock_env().await;
        let _home = EnvGuard::temp_home();
        assert!(
            tasks_line().contains("Run `future task add`"),
            "{}",
            tasks_line()
        );
        assert!(
            skills_line().contains("Run `future skills install-builtin`"),
            "{}",
            skills_line()
        );
    }
}
