//! Time zones for a task's schedule.
//!
//! A schedule stores wall-clock times (`daily 09:00`, `weekly mon 10:00`). Which
//! instant those are depends on a time zone, and the task stores it as an IANA
//! name (`Asia/Shanghai`) so a schedule keeps its meaning when the machine that
//! runs the tick sits in a different zone — or when the machine's own zone
//! changes.
//!
//! A task with no stored zone (a row written before the setting existed) runs on
//! the host's zone, and a host whose zone cannot be read falls back to Beijing.
//! That fallback is deliberate: an unreadable zone must not silently mean UTC,
//! which would move a Chinese user's morning task to the afternoon.

use chrono_tz::Tz;

use crate::types::Task;

/// The zone used when the host's own zone cannot be read.
pub const FALLBACK_TIMEZONE: &str = "Asia/Shanghai";

/// Parse an IANA name (`Asia/Shanghai`, `UTC`, `America/New_York`).
///
/// The canonical spelling is tried first; the crate's `case-insensitive`
/// feature then accepts `asia/shanghai` too, so a CLI argument typed in lower
/// case still lands on the real zone rather than on the fallback.
pub fn parse_timezone(name: &str) -> Option<Tz> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    name.parse::<Tz>()
        .ok()
        .or_else(|| Tz::from_str_insensitive(name).ok())
}

/// The host's own zone, if the platform can tell us.
///
/// `iana_time_zone` reads the OS setting (the `/etc/localtime` link and `TZ` on
/// Unix, the registry on Windows). Its answer is validated against the zone
/// database in this crate: a name the database does not know is no more usable
/// than an unreadable one, and would otherwise turn every schedule into "no
/// candidate at all" instead of falling back.
pub fn system_timezone() -> Option<Tz> {
    let name = iana_time_zone::get_timezone().ok()?;
    parse_timezone(&name)
}

/// The zone a task has when it does not name one: the host's, else Beijing.
pub fn default_timezone() -> Tz {
    system_timezone().unwrap_or(Tz::Asia__Shanghai)
}

/// The zone a task's schedule is computed in.
///
/// A stored name wins; an empty, unknown or absent one falls through to
/// [`default_timezone`], so a hand-edited database or a zone this build's
/// database dropped degrades to the host's zone instead of scheduling nothing.
pub fn task_timezone(task: &Task) -> Tz {
    task.timezone
        .as_deref()
        .and_then(parse_timezone)
        .unwrap_or_else(default_timezone)
}

/// The concrete zone name to **store** for a task, given what a client sent.
///
/// `None`, an empty string or an unparsable name means "not chosen": the host's
/// zone, else Beijing — the same rule [`default_timezone`] applies, resolved
/// once at write time so the task's record says what its schedule means.
/// Storage keeps the canonical spelling (`Asia/Shanghai`, not `asia/shanghai`).
pub fn resolve_timezone(raw: Option<&str>) -> String {
    raw.and_then(parse_timezone)
        .unwrap_or_else(default_timezone)
        .name()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{SessionPolicy, SessionRetention, TriggerKind};

    fn task(timezone: Option<&str>) -> Task {
        Task {
            id: "tsk_tz".into(),
            name: "tz".into(),
            enabled: true,
            prompt: "p".into(),
            prompt_version: 1,
            cwd: "/tmp".into(),
            model_id: None,
            thinking_level: None,
            timezone: timezone.map(str::to_string),
            session_policy: SessionPolicy::New,
            conversation_mode: crate::types::ConversationMode::Workspace,
            thread_id: None,
            trigger_kind: TriggerKind::Manual,
            trigger_json: serde_json::json!({}),
            next_due_at: None,
            pending_request_at: None,
            pending_origin: None,
            pending_actor: None,
            session_retention: SessionRetention::Keep,
            created_at: 0,
            updated_at: 0,
            deleted_at: None,
        }
    }

    #[test]
    fn iana_names_parse_and_are_case_insensitive() {
        assert_eq!(parse_timezone("Asia/Shanghai"), Some(Tz::Asia__Shanghai));
        assert_eq!(parse_timezone(" asia/shanghai "), Some(Tz::Asia__Shanghai));
        assert_eq!(parse_timezone("UTC"), Some(Tz::UTC));
        assert_eq!(parse_timezone(""), None);
        assert_eq!(parse_timezone("Mars/Olympus_Mons"), None);
    }

    /// A stored, valid name is what the schedule uses; anything else falls back
    /// to the host's zone rather than failing to schedule at all.
    #[test]
    fn a_task_uses_its_stored_zone_and_falls_back_otherwise() {
        assert_eq!(
            task_timezone(&task(Some("America/New_York"))),
            Tz::America__New_York
        );
        assert_eq!(task_timezone(&task(Some("Not/AZone"))), default_timezone());
        assert_eq!(task_timezone(&task(None)), default_timezone());
    }

    /// Writing a task always stores a concrete, canonical zone: what a client
    /// typed (or did not type) becomes the record's answer.
    #[test]
    fn resolve_names_a_zone_for_every_input() {
        assert_eq!(resolve_timezone(Some("asia/shanghai")), "Asia/Shanghai");
        assert_eq!(resolve_timezone(Some("Europe/Berlin")), "Europe/Berlin");
        let fallback = resolve_timezone(None);
        assert_eq!(fallback, default_timezone().name());
        assert_eq!(resolve_timezone(Some("")), fallback);
        assert_eq!(resolve_timezone(Some("Not/AZone")), fallback);
        // Whether the host's zone is readable or not, a resolved name is always
        // one the database can parse back.
        assert!(parse_timezone(&fallback).is_some());
    }
}
