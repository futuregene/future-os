//! Schema installation and compatibility indexes. Called once during opening.
use anyhow::Result;
use rusqlite::Connection;

pub(super) fn initialize_schema(
    connection: &mut Connection,
    cancellation: &crate::service::StartupCancellation,
) -> Result<()> {
    cancellation.check()?;
    crate::session::content::install_functions(connection)?;
    let tx = connection.transaction()?;
    let missing_projection: bool = tx.query_row(
        "SELECT count(*)!=2 FROM sqlite_schema WHERE type='table' AND name IN ('history_shapes','history_display')",
        [], |row| row.get(0),
    )?;
    tx.execute_batch(include_str!("schema.sql"))?;
    tx.execute_batch(include_str!("../content/schema.sql"))?;
    tx.execute_batch(&crate::session::identity::inspection_views_sql())?;
    if missing_projection {
        tx.execute("DELETE FROM history_index_state", [])?;
    }
    for key in ["database_id", "restore_generation"] {
        tx.execute(
            "INSERT OR IGNORE INTO storage_meta(key,value) VALUES (?1,?2)",
            rusqlite::params![key, uuid::Uuid::new_v4().to_string()],
        )?;
    }
    tx.execute("INSERT INTO storage_meta(key,value) VALUES ('event_sequence',CAST(coalesce((SELECT max(sequence) FROM run_events),0) AS TEXT))
        ON CONFLICT(key) DO UPDATE SET value=CAST(max(CAST(storage_meta.value AS INTEGER),CAST(excluded.value AS INTEGER)) AS TEXT)", [])?;
    cancellation.check()?;
    tx.execute_batch(crate::skills::registry::SKILLS_TABLE_SQL)?;
    tx.execute_batch(crate::session::content::entries::RESOLVED_BLOCKS_SQL)?;
    tx.execute_batch(&crate::session::entry_records::entry_record_views_sql())?;
    tx.execute_batch(&crate::session::content::entries::header_views_sql())?;
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
pub(super) fn validate_authoritative_history(connection: &Connection, version: i64) -> Result<()> {
    for query in [
        "SELECT id,revision,current_metadata_json,created_at_ms,updated_at_ms FROM sessions LIMIT 0",
        "SELECT session_id,position,entry_id,entry_type,role,run_id,timestamp_ms,metadata_json,content_json FROM entries LIMIT 0",
        "SELECT session_id,entry_position,ordinal,kind,text,tool_call_id,tool_name,arguments_json,is_error,metadata_json FROM message_blocks LIMIT 0",
        "SELECT session_id,run_id,status,run_sequence,epoch,started_at_ms,completed_at_ms,error,cache_write_tokens,input_baseline,cache_read_baseline,input_tokens,cache_read_tokens,output_tokens,duration_ms FROM runs LIMIT 0",
        "SELECT sequence,session_id,run_id,epoch,idx,event_id,payload FROM run_events LIMIT 0",
        "SELECT session_id,status,fingerprint,error_file,error_line,error_kind,warnings FROM legacy_imports LIMIT 0",
        "SELECT key,value FROM storage_meta LIMIT 0",
    ] {
        connection.prepare(query).map_err(|error| anyhow::anyhow!("Agent database history structure is incomplete; preserve the database for recovery: {error}"))?;
    }
    // Snapshots first shipped in schema 5. Earlier databases legitimately
    // lack this table: validate their original history before backing it up,
    // then let initialize_schema install the new empty table. If an older
    // version already has snapshots, still validate them rather than masking
    // a partial or damaged upgrade with CREATE IF NOT EXISTS.
    let has_snapshots: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='run_snapshots')",
        [],
        |row| row.get(0),
    )?;
    if version >= 5 || has_snapshots {
        connection.prepare("SELECT session_id,run_id,version,cursor,payload,pricing_payloads FROM run_snapshots LIMIT 0")
            .map_err(|error| anyhow::anyhow!("Agent database history structure is incomplete; preserve the database for recovery: {error}"))?;
    }
    if version >= 7 {
        for query in [
            "SELECT session_id,deleted_at_ms FROM session_tombstones LIMIT 0",
            "SELECT session_id,run_id,client_request_id,run_sequence,fingerprint,payload_json,user_entry_json,status FROM queued_run_receipts LIMIT 0",
        ] {
            connection.prepare(query).map_err(|error| anyhow::anyhow!("Agent database history structure is incomplete; preserve the database for recovery: {error}"))?;
        }
        let identities: i64 = connection.query_row(
            "SELECT count(*) FROM storage_meta WHERE key IN ('database_id','restore_generation') AND length(value)>0",
            [], |row| row.get(0),
        )?;
        anyhow::ensure!(
            identities == 2,
            "Agent database identity is incomplete; preserve the database for recovery"
        );
    }
    if version >= 8 {
        crate::session::content::compact::validate_layout(connection)?;
        for query in [
            "SELECT session_id,format FROM session_formats LIMIT 0",
            "SELECT session_id,generation,metadata_generation FROM content_history_versions LIMIT 0",
            "SELECT id,byte_length,sealed,sha256,json_sha256 FROM content_objects LIMIT 0",
            "SELECT content_id,byte_offset,body FROM content_segments LIMIT 0",
            "SELECT session_id,owner_kind,owner_id,field,encoding,content_id,byte_offset,byte_length FROM content_references LIMIT 0",
            "SELECT id,session_id,run_id,epoch,durable_cursor FROM stream_runs LIMIT 0",
            "SELECT id,stream_id,context_id,first_cursor,last_cursor,event_kind,unknown_kind,event_type,flags,metadata_json,content_id,byte_offset,byte_length FROM stream_changes LIMIT 0",
            "SELECT id,stream_id,value FROM stream_symbols LIMIT 0",
            "SELECT stream_id,key_id,content_id,byte_offset,byte_length,encoding FROM stream_body_owners LIMIT 0",
            "SELECT id,stream_id,digest,descriptor_json,symbol_bytes,event_kind,entry_id,tool_id,content_key,block_key,tool_name_key,tool_type_key,tc_index,direct_field,direct_encoding FROM stream_contexts LIMIT 0",
            "SELECT change_id,field,encoding,content_id,byte_offset,byte_length FROM stream_change_content LIMIT 0",
            "SELECT content_id FROM content_collection LIMIT 0",
            "SELECT session_id,entry_id,digest FROM entry_fingerprints LIMIT 0",
            "SELECT operation_id,session_id,owner_prefix,expected_revision,kind FROM content_operations LIMIT 0",
            "SELECT operation_id,session_id,entry_id,run_id FROM content_entry_stages LIMIT 0",
            "SELECT stream_id FROM retired_streams LIMIT 0",
            "SELECT session_id,input_key,history_generation FROM checkpoint_generations LIMIT 0",
            "SELECT session_id,through_generation FROM retired_checkpoint_generations LIMIT 0",
        ] {
            connection.prepare(query).map_err(|error| anyhow::anyhow!("Agent database history structure is incomplete; preserve the database for recovery: {error}"))?;
        }
    }
    Ok(())
}

pub(super) fn validate_core(connection: &Connection) -> Result<()> {
    validate_authoritative_history(connection, super::CURRENT_SCHEMA_VERSION)
}
