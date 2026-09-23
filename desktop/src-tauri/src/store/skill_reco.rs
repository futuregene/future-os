//! Daily skill-recommendation bookkeeping.
//!
//! The agent only answers `suggest_skill`; the rules about *when* to call it
//! live in the client, so their durable state lives here. One row is written
//! for every recommendation actually shown to the user.
//!
//! Two deliberate consequences of that shape:
//!
//! - **Calls are not recorded.** The daily budget counts *recommendations*, not
//!   calls: a call that produces no recommendation (the model judged nothing
//!   suitable, or its confidence was too low) leaves no row and does not consume
//!   the budget. That is why the budget can outlive three calls.
//! - **The same row answers all three questions.** A row carries the day, the
//!   skill and a hash of the message that produced it, so "how many
//!   recommendations today", "has this skill been recommended today" and "has
//!   this message already produced a recommendation" are one query each, and
//!   `clear_all_data` resets all of it with the other tables.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::db::*;
use super::util::*;

/// Rows are only ever read for the current day, so older ones are pruned on
/// write to keep the table bounded (a few rows per user per day).
const RETENTION_DAYS: i64 = 30;

/// One recommendation shown today.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRecoEvent {
    pub skill_id: String,
    pub message_hash: String,
}

/// Today's recommendation state, as the client's trigger rules need it.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRecoToday {
    /// Recommendations shown today. The daily budget counts these, not calls.
    pub count: u32,
    /// Skill ids already recommended today (same skill is never shown twice).
    pub skill_ids: Vec<String>,
    /// Messages that already produced a recommendation today.
    pub message_hashes: Vec<String>,
}

/// Local calendar day as `YYYY-MM-DD` (the day boundary is the user's, not UTC).
fn local_day() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

pub fn skill_reco_today() -> Result<SkillRecoToday, crate::AppError> {
    let conn = connect()?;
    let day = local_day();
    let mut statement =
        conn.prepare("SELECT skill_id, message_hash FROM skill_reco_events WHERE day = ?1")?;
    let rows = statement.query_map([&day], |row| {
        Ok(SkillRecoEvent {
            skill_id: row.get(0)?,
            message_hash: row.get(1)?,
        })
    })?;
    let mut today = SkillRecoToday::default();
    for row in rows {
        let event = row?;
        today.count += 1;
        today.skill_ids.push(event.skill_id);
        today.message_hashes.push(event.message_hash);
    }
    Ok(today)
}

/// Record one shown recommendation. Called only when a card is actually
/// displayed — a call that recommended nothing must not consume the budget.
pub fn record_skill_reco(skill_id: &str, message_hash: &str) -> Result<(), crate::AppError> {
    let mut conn = connect()?;
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO skill_reco_events (day, skill_id, message_hash, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![local_day(), skill_id, message_hash, now_millis()],
    )?;
    tx.execute(
        "DELETE FROM skill_reco_events WHERE day < ?1",
        [local_day_before(RETENTION_DAYS)],
    )?;
    tx.commit()?;
    Ok(())
}

/// `YYYY-MM-DD` for `days` ago in local time, used as the prune cut-off.
fn local_day_before(days: i64) -> String {
    (chrono::Local::now() - chrono::Duration::days(days))
        .format("%Y-%m-%d")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db::test_support::guarded_conn;
    use rusqlite::params;

    #[test]
    fn empty_state_reports_zero_recommendations() {
        let _home = guarded_conn("skill_reco_empty").0;
        let today = skill_reco_today().expect("read today");
        assert_eq!(today.count, 0);
        assert!(today.skill_ids.is_empty());
        assert!(today.message_hashes.is_empty());
    }

    #[test]
    fn recorded_recommendations_are_read_back_with_skill_and_message() {
        let _home = guarded_conn("skill_reco_record").0;
        record_skill_reco("future-web", "hash-a").expect("record first");
        record_skill_reco("future-paper", "hash-b").expect("record second");

        let today = skill_reco_today().expect("read today");
        assert_eq!(today.count, 2);
        assert!(today.skill_ids.contains(&"future-web".to_string()));
        assert!(today.skill_ids.contains(&"future-paper".to_string()));
        assert!(today.message_hashes.contains(&"hash-a".to_string()));
        assert!(today.message_hashes.contains(&"hash-b".to_string()));
    }

    /// The store records rows; de-duplicating a repeated skill is the client's
    /// rule, so a second row for the same skill must still be counted (otherwise
    /// the daily budget could be evaded by re-recommending one skill).
    #[test]
    fn the_same_skill_recorded_twice_counts_twice() {
        let _home = guarded_conn("skill_reco_repeat").0;
        record_skill_reco("future-web", "hash-a").expect("record first");
        record_skill_reco("future-web", "hash-b").expect("record second");
        assert_eq!(skill_reco_today().expect("read today").count, 2);
    }

    #[test]
    fn older_days_are_pruned_but_today_survives() {
        let (_home, conn) = guarded_conn("skill_reco_prune");
        conn.execute(
            "INSERT INTO skill_reco_events (day, skill_id, message_hash, created_at)
             VALUES (?1, 'stale-skill', 'stale-hash', 0)",
            params![local_day_before(RETENTION_DAYS + 1)],
        )
        .expect("insert stale row");

        record_skill_reco("future-web", "hash-a").expect("record today");

        let today = skill_reco_today().expect("read today");
        assert_eq!(today.count, 1);
        assert!(!today.skill_ids.contains(&"stale-skill".to_string()));
        let stale = conn
            .query_row(
                "SELECT COUNT(*) FROM skill_reco_events WHERE skill_id = 'stale-skill'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("count stale rows");
        assert_eq!(
            stale, 0,
            "rows past the retention window are pruned on write"
        );
    }

    /// A row from a recent day is still inside the retention window, so it must
    /// not be pruned — and must not leak into today's counts.
    #[test]
    fn yesterdays_rows_are_kept_and_excluded_from_today() {
        let (_home, conn) = guarded_conn("skill_reco_yesterday");
        conn.execute(
            "INSERT INTO skill_reco_events (day, skill_id, message_hash, created_at)
             VALUES (?1, 'yesterday-skill', 'yesterday-hash', 0)",
            params![local_day_before(1)],
        )
        .expect("insert yesterday row");

        let today = skill_reco_today().expect("read today");
        assert_eq!(today.count, 0, "yesterday's rows are not today's budget");
        assert!(!today.skill_ids.contains(&"yesterday-skill".to_string()));

        record_skill_reco("future-web", "hash-a").expect("record today");
        let kept = conn
            .query_row(
                "SELECT COUNT(*) FROM skill_reco_events WHERE skill_id = 'yesterday-skill'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("count yesterday rows");
        assert_eq!(kept, 1, "rows inside the retention window are kept");
    }

    /// Schema application is idempotent on an existing database: reconnecting
    /// (which re-runs SCHEMA) must not disturb recorded rows or fail on the
    /// already-created table.
    #[test]
    fn schema_reapplication_keeps_recorded_rows() {
        let (_home, conn) = guarded_conn("skill_reco_reapply");
        record_skill_reco("future-web", "hash-a").expect("record");
        crate::store::db::apply_schema(&conn).expect("re-apply schema");
        assert_eq!(skill_reco_today().expect("read today").count, 1);
    }
}
