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
    validate_core(&tx)?;
    tx.pragma_update(None, "user_version", super::CURRENT_SCHEMA_VERSION)?;
    cancellation.check()?;
    tx.commit()?;
    Ok(())
}

/// Preparing the actual read shapes catches missing tables/columns without
/// hydrating or rewriting any historical body. Derived indexes are separate.
pub(super) fn validate_core(connection: &Connection) -> Result<()> {
    for query in [
        "SELECT id,revision,current_metadata_json,created_at_ms,updated_at_ms FROM sessions LIMIT 0",
        "SELECT session_id,position,entry_id,entry_type,role,run_id,timestamp_ms,metadata_json,content_json FROM entries LIMIT 0",
        "SELECT session_id,entry_position,ordinal,kind,text,tool_call_id,tool_name,arguments_json,is_error,metadata_json FROM message_blocks LIMIT 0",
        "SELECT session_id,run_id,status,run_sequence,epoch,started_at_ms,completed_at_ms,error,cache_write_tokens,input_baseline,cache_read_baseline,input_tokens,cache_read_tokens,output_tokens,duration_ms FROM runs LIMIT 0",
        "SELECT sequence,session_id,run_id,epoch,idx,event_id,payload FROM run_events LIMIT 0",
        "SELECT session_id,run_id,version,cursor,payload,pricing_payloads FROM run_snapshots LIMIT 0",
        "SELECT session_id,status,fingerprint,error_file,error_line,error_kind,warnings FROM legacy_imports LIMIT 0",
        "SELECT key,value FROM storage_meta LIMIT 0",
    ] {
        connection.prepare(query).map_err(|error| anyhow::anyhow!("Agent database history structure is incomplete; preserve the database for recovery: {error}"))?;
    }
    Ok(())
}
