//! Acceptance probe for an explicitly supplied disposable copy of an Agent DB.
//! Never point this at the live database or the immutable baseline fixture.
use anyhow::{ensure, Context, Result};
use future_agent::session::{sqlite_store::SqliteStore, Manager};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::PathBuf};

fn readonly(path: &std::path::Path) -> Result<Connection> {
    Ok(Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?)
}

fn legacy_ids(db: &Connection) -> Result<Vec<String>> {
    let mut q=db.prepare("SELECT id FROM sessions s WHERE revision>=0 AND NOT EXISTS(SELECT 1 FROM session_formats f WHERE f.session_id=s.id AND f.format=2) ORDER BY id")?;
    let rows = q
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn ledger(db: &Connection) -> Result<BTreeMap<String, String>> {
    let mut q=db.prepare("SELECT session_id,run_id,status,run_sequence,epoch,started_at_ms,completed_at_ms,error,cache_write_tokens,input_baseline,cache_read_baseline,input_tokens,cache_read_tokens,output_tokens,duration_ms FROM runs ORDER BY session_id,run_id")?;
    let count = q.column_count();
    let rows = q
        .query_map([], |row| {
            let values = (0..count)
                .map(|i| row.get::<_, rusqlite::types::Value>(i))
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok((
                format!("{:?}/{:?}", values[0], values[1]),
                format!("{values:?}"),
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

fn main() -> Result<()> {
    let root =
        PathBuf::from(std::env::args().nth(1).context(
            "usage: legacy_migration_probe <disposable-fixture-directory> [delete-only]",
        )?);
    ensure!(
        root.join("DISPOSABLE_FIXTURE").is_file(),
        "refusing a database without DISPOSABLE_FIXTURE marker"
    );
    let path = root.join("agent.db");
    ensure!(path.is_file(), "fixture database is missing");
    let store = SqliteStore::open(&path)?;
    let before_ids = legacy_ids(&readonly(&path)?)?;
    ensure!(
        !before_ids.is_empty(),
        "fixture has no published legacy sessions"
    );
    let mut before = BTreeMap::<String, Vec<Value>>::new();
    for id in &before_ids {
        before.insert(id.clone(), store.entries(id)?);
    }
    let before_ledger = ledger(&readonly(&path)?)?;
    let manager = Manager::new(root.clone());
    manager.initialize()?;
    manager.list_all()?;
    ensure!(
        legacy_ids(&readonly(&path)?)? == before_ids,
        "listing converted legacy conversations"
    );
    let mode = std::env::args().nth(2);
    if matches!(mode.as_deref(), Some("failure-retry" | "failure-delete")) {
        let conn = Connection::open(&path)?;
        conn.execute_batch("CREATE TRIGGER probe_fail_publication BEFORE INSERT ON entries BEGIN SELECT RAISE(ABORT,'injected migration publication failure'); END")?;
        ensure!(
            manager.load(&before_ids[0]).is_err(),
            "publication fault did not fire"
        );
        ensure!(
            store.entries(&before_ids[0])? == before[&before_ids[0]],
            "failed migration changed old entries"
        );
        ensure!(
            legacy_ids(&conn)? == before_ids,
            "failed migration changed session format"
        );
        let pending: i64 =
            conn.query_row("SELECT count(*) FROM legacy_conversions", [], |r| r.get(0))?;
        ensure!(pending == 1, "failed migration lost its recovery receipt");
        // A fresh Manager must still be able to initialize/list/delete while
        // publication is broken, without retrying unrelated conversions.
        let fresh = Manager::new(root.clone());
        fresh.list_all()?;
        if mode.as_deref() == Some("failure-delete") {
            fresh.delete(&before_ids[0])?;
            ensure!(
                !store.contains(&before_ids[0])?,
                "failed migration resurrected after deletion"
            );
            println!(
                "{}",
                json!({"publication_failure_kept_old_entries":true,"pending_conversion_deleted_without_retry":true})
            );
            return Ok(());
        }
        conn.execute_batch("DROP TRIGGER probe_fail_publication")?;
        fresh.load(&before_ids[0])?;
    }
    if mode.as_deref() == Some("delete-only") {
        let conn = Connection::open(&path)?;
        conn.execute_batch("CREATE TRIGGER probe_forbid_conversion BEFORE INSERT ON legacy_conversions BEGIN SELECT RAISE(ABORT,'deletion must not convert'); END")?;
        for id in &before_ids {
            manager.delete(id)?;
        }
        ensure!(
            legacy_ids(&readonly(&path)?)?.is_empty(),
            "legacy sessions survived deletion"
        );
        let pending: i64 =
            conn.query_row("SELECT count(*) FROM legacy_conversions", [], |r| r.get(0))?;
        ensure!(pending == 0, "deletion created migration receipts");
        println!(
            "{}",
            json!({"deleted_without_conversion":before_ids.len(),"integrity":conn.query_row("PRAGMA integrity_check",[],|r|r.get::<_,String>(0))?})
        );
        return Ok(());
    }
    let mut entries = 0;
    for (index, id) in before_ids.iter().enumerate() {
        manager
            .load(id)
            .with_context(|| format!("migration failed for fixture session index {index}"))?;
        let after = store.entries(id)?;
        // Report only indices on failure, never private conversation contents.
        ensure!(
            before[id] == after,
            "entry values changed for fixture session index {index}"
        );
        entries += after.len();
    }
    ensure!(
        legacy_ids(&readonly(&path)?)?.is_empty(),
        "migration left published legacy sessions"
    );
    ensure!(
        ledger(&readonly(&path)?)? == before_ledger,
        "migration changed the run ledger"
    );
    let reopened = Manager::new(root);
    for id in &before_ids {
        reopened.load(id)?;
        ensure!(
            store.entries(id)? == before[id],
            "reopen changed entry values"
        );
    }
    manager.append_entries(
        &before_ids[0],
        &[future_agent::session::SessionEntry::new_user(
            "user",
            json!("disposable migration acceptance continuation"),
        )],
    )?;
    manager.update_session_info(
        &before_ids[0],
        "session_name",
        json!("disposable acceptance"),
    )?;
    ensure!(
        store.entries(&before_ids[0])?.len() > before[&before_ids[0]].len(),
        "continuation was not persisted"
    );
    let db = readonly(&path)?;
    let integrity: String = db.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    let fk: i64 = db.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
        r.get(0)
    })?;
    ensure!(integrity == "ok" && fk == 0, "fixture integrity failed");
    println!(
        "{}",
        json!({"migrated_sessions":before_ids.len(),"preserved_entries":entries,"run_ledger_unchanged":true,"reopen_ok":true,"canonical_continuation_and_metadata_write":true,"failure_retry":mode.as_deref()==Some("failure-retry"),"integrity":integrity,"foreign_key_violations":fk})
    );
    Ok(())
}
