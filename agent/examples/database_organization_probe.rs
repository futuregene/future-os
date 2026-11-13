//! Runs the same per-conversation maintenance used by the developer UI, only on
//! an explicitly marked disposable copy. Reports aggregates, never chat text.
use anyhow::{ensure, Context, Result};
use future_agent::session::{sqlite_store::SqliteStore, Manager};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::PathBuf};

fn ledger(db: &Connection) -> Result<Vec<Vec<rusqlite::types::Value>>> {
    let mut query = db.prepare("SELECT * FROM runs ORDER BY session_id,run_id")?;
    let columns = query.column_count();
    let rows = query
        .query_map([], |r| (0..columns).map(|i| r.get(i)).collect())?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

fn main() -> Result<()> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .context("usage: database_organization_probe <disposable-fixture-directory>")?,
    );
    ensure!(
        root.join("DISPOSABLE_FIXTURE").is_file(),
        "refusing an unmarked database"
    );
    let path = root.join("agent.db");
    ensure!(path.is_file(), "fixture database missing");
    let manager = Manager::new(root.clone());
    manager.initialize()?;
    let store = SqliteStore::open(&path)?;
    let inject = std::env::args().nth(2).as_deref() == Some("--inject-failures");
    if inject {
        for id in ["probe-busy", "probe-broken", "probe-failed"] {
            ensure!(!store.contains(id)?, "fault fixtures already exist");
            store.replace(
                id,
                vec![json!({
                    "id":format!("entry-{id}"),"type":"user","role":"user",
                    "content":[{"type":"text","text":format!("unique fixture {id} ").repeat(200)}],
                    "meta":{"run_id":format!("run-{id}")},
                    "timestamp":"2026-01-01T00:00:00Z"
                })],
            )?;
        }
        let faults = Connection::open(&path)?;
        faults.execute(
            "UPDATE runs SET status='completed' WHERE session_id LIKE 'probe-%'",
            [],
        )?;
        manager.load("probe-broken")?;
        manager.load("probe-failed")?;
        faults.execute(
            "UPDATE runs SET status='running' WHERE session_id='probe-busy'",
            [],
        )?;
        faults.execute(
            "UPDATE entries SET metadata_json='[]' WHERE session_id='probe-broken'",
            [],
        )?;
        faults.execute_batch("CREATE TRIGGER injected_organization_failure BEFORE INSERT ON content_compressed WHEN NEW.content_id IN (SELECT content_id FROM content_references WHERE session_id='probe-failed') BEGIN SELECT RAISE(ABORT,'injected compression failure'); END")?;
    }
    let plan = manager.database_organization_plan()?;
    let ids: Vec<String> = serde_json::from_value(plan["sessionIds"].clone())?;
    let db = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let before_ledger = ledger(&db)?;
    let mut before = BTreeMap::<String, Vec<Value>>::new();
    let mut initially_unreadable = 0;
    for id in &ids {
        match store.entries(id) {
            Ok(entries) => {
                before.insert(id.clone(), entries);
            }
            Err(_) => initially_unreadable += 1,
        }
    }
    let (mut migrated, mut done, mut skipped, mut failed) = (0, 0, 0, 0);
    let mut failures = Vec::new();
    for (index, id) in ids.iter().enumerate() {
        let mut slices = 0;
        loop {
            slices += 1;
            ensure!(slices <= 100, "maintenance failed to make bounded progress");
            match manager.organize_database_session(id) {
                Ok(result) => {
                    if result["migrated"] == true {
                        migrated += 1;
                    }
                    if result["hasMore"] == true {
                        continue;
                    }
                    if result["status"] == "skipped" {
                        skipped += 1;
                    } else {
                        done += 1;
                    }
                }
                Err(error) => {
                    failed += 1;
                    failures.push(json!({"index":index,"error":format!("{error:#}")}));
                }
            }
            break;
        }
    }
    let mut finish;
    let mut slices = 0;
    loop {
        slices += 1;
        ensure!(slices <= 100, "collection failed to make bounded progress");
        finish = manager.finish_database_organization()?;
        if finish["hasMore"] != true {
            break;
        }
    }
    let mut entry_count = 0;
    for (id, entries) in &before {
        ensure!(
            store.entries(id)? == *entries,
            "maintenance changed conversation values"
        );
        entry_count += entries.len();
    }
    ensure!(
        ledger(&db)? == before_ledger,
        "maintenance changed run ledger"
    );
    let integrity: String = db.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    let foreign_keys: i64 =
        db.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })?;
    ensure!(
        integrity == "ok" && foreign_keys == 0,
        "fixture integrity failed"
    );
    if inject {
        ensure!(
            skipped == 1 && failed == 2 && done > 0,
            "fault isolation did not skip the expected conversations"
        );
        ensure!(
            finish["compacted"] == false && finish["deferredReason"] == "busy",
            "busy run did not defer final file compaction"
        );
        let leases: i64 =
            db.query_row("SELECT count(*) FROM content_operations", [], |r| r.get(0))?;
        ensure!(
            leases == 0,
            "a maintenance failure leaked its admission lease"
        );
        let busy_format: i64 = db.query_row(
            "SELECT coalesce((SELECT format FROM session_formats WHERE session_id='probe-busy'),1)",
            [],
            |r| r.get(0),
        )?;
        ensure!(busy_format == 1, "busy conversation was converted");
    }
    let remaining_legacy: i64 = db.query_row("SELECT count(*) FROM sessions s WHERE revision>=0 AND NOT EXISTS(SELECT 1 FROM session_formats f WHERE f.session_id=s.id AND f.format=2)", [], |r| r.get(0))?;
    println!(
        "{}",
        json!({"sessions":ids.len(),"migrated":migrated,"done":done,"skipped":skipped,"failed":failed,"failures":failures,"initiallyUnreadable":initially_unreadable,"preservedEntries":entry_count,"ledgerUnchanged":true,"remainingLegacy":remaining_legacy,"before":plan["storage"],"finish":finish,"integrity":integrity,"foreignKeyViolations":foreign_keys})
    );
    Ok(())
}
