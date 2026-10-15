//! Schema installation and compatibility indexes. Called once during opening.
use anyhow::Result;
use rusqlite::Connection;

pub(super) fn initialize_schema(
    connection: &mut Connection,
    cancellation: &crate::service::StartupCancellation,
) -> Result<()> {
    cancellation.check()?;
    let tx = connection.transaction()?;
    tx.execute_batch(include_str!("schema.sql"))?;
    tx.execute("INSERT INTO storage_meta(key,value) VALUES ('event_sequence',CAST(coalesce((SELECT max(sequence) FROM run_events),0) AS TEXT))
        ON CONFLICT(key) DO UPDATE SET value=CAST(max(CAST(storage_meta.value AS INTEGER),CAST(excluded.value AS INTEGER)) AS TEXT)", [])?;
    cancellation.check()?;
    tx.execute_batch(crate::skills::registry::SKILLS_TABLE_SQL)?;
    tx.execute_batch(crate::session::entry_records::ENTRY_RECORD_VIEWS_SQL)?;
    tx.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS run_events_custom_id
         ON run_events(session_id,event_id) WHERE event_id IS NOT NULL;",
    )?;
    cancellation.check()?;
    tx.pragma_update(None, "user_version", super::CURRENT_SCHEMA_VERSION)?;
    cancellation.check()?;
    tx.commit()?;
    Ok(())
}
