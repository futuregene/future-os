//! Foreign keys must be checked through an index, never a table scan.
//!
//! SQLite performs an FK action by looking the child rows up by their key
//! columns — for `ON DELETE CASCADE` it deletes them, for `NO ACTION`/`RESTRICT`
//! it has to prove there are none. Without an index whose *leading* columns are
//! those key columns the lookup is a scan of the whole child table, repeated once
//! per deleted parent row, so tearing down one conversation costs
//! `symbols × child rows`.
//!
//! That is not theoretical: on 2026-10-29 deleting a 30k-change conversation
//! spent 113 s of CPU in these scans inside the store's single ordered writer,
//! and every other session's writes (including a user's stop) queued behind it
//! for 71 s. The seven indexes these tests pin are in
//! `agent/src/session/content/schema.sql`.
//!
//! The check is a query plan rather than an index inventory on purpose: partial
//! indexes only help when the planner can prove their predicate, which is
//! exactly why `stream_contexts`' scoped indexes (`stream_id` first, three of
//! them partial on `event_kind`) never served these lookups.
use super::*;

/// The symbol-reference indexes a stream teardown needs, named so a failure says
/// which one is missing rather than only that some query scans.
const TEARDOWN_INDEXES: [&str; 7] = [
    "stream_body_owners_key",
    "stream_contexts_entry_key",
    "stream_contexts_tool_key",
    "stream_contexts_content_key",
    "stream_contexts_block_key",
    "stream_contexts_tool_name_key",
    "stream_contexts_tool_type_key",
];

fn plan_steps(connection: &Connection, sql: &str) -> Vec<String> {
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap_or_else(|error| panic!("cannot plan `{sql}`: {error}"));
    let columns = statement.column_count();
    statement
        .query_map([], |row| {
            // `detail` is the last column on every supported SQLite.
            row.get::<_, String>(columns - 1)
        })
        .unwrap_or_else(|error| panic!("cannot run `{sql}`: {error}"))
        .map(|row| row.expect("plan row"))
        .collect()
}

/// The lookup SQLite itself performs for one FK action: `WHERE <key columns> = ?`.
/// `NULL` stands in for the bound value — the plan is chosen at prepare time, so
/// the operand only has to be a plain comparison, never a real key.
fn lookup_sql(table: &str, columns: &[String]) -> String {
    let predicate = columns
        .iter()
        .map(|column| format!("\"{column}\" = NULL"))
        .collect::<Vec<_>>()
        .join(" AND ");
    format!("SELECT 1 FROM \"{table}\" WHERE {predicate} LIMIT 1")
}

fn foreign_keys(connection: &Connection) -> Vec<(String, String, Vec<String>)> {
    let tables = connection
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .expect("table list")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("table list")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("table list");

    let mut keys = Vec::new();
    for table in tables {
        // One row per *column* of a composite key; `id` groups them and `seq`
        // orders them, so a composite parent is not mistaken for two singles.
        let mut rows = connection
            .prepare("SELECT id,seq,\"from\",\"table\" FROM pragma_foreign_key_list(?1)")
            .expect("fk list")
            .query_map([&table], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .expect("fk list")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("fk list");
        rows.sort_by_key(|(id, seq, _, _)| (*id, *seq));
        let mut grouped: std::collections::BTreeMap<i64, (String, Vec<String>)> =
            std::collections::BTreeMap::new();
        for (id, _, child, parent) in rows {
            grouped
                .entry(id)
                .or_insert((parent, Vec::new()))
                .1
                .push(child);
        }
        for (parent, columns) in grouped.into_values() {
            keys.push((table.clone(), parent, columns));
        }
    }
    keys
}

/// Every FK lookup that would scan a child table, as `child(columns) -> parent`
/// plus the plan that gave it away.
fn scanned_foreign_keys(connection: &Connection) -> Vec<String> {
    let mut scans = Vec::new();
    for (child, parent, columns) in foreign_keys(connection) {
        let steps = plan_steps(connection, &lookup_sql(&child, &columns));
        assert!(
            !steps.is_empty(),
            "{child}({}) -> {parent} produced no plan",
            columns.join(",")
        );
        if steps.iter().any(|step| step.starts_with("SCAN")) {
            scans.push(format!(
                "{child}({}) -> {parent}: {}",
                columns.join(","),
                steps.join(" | ")
            ));
        }
    }
    scans
}

fn installed(connection: &Connection, name: &str) -> bool {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1)",
            [name],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn every_foreign_key_lookup_uses_an_index() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("agent.db")).unwrap();
    let scans = db
        .read(|connection| Ok(scanned_foreign_keys(connection)))
        .unwrap();
    assert!(
        scans.is_empty(),
        "a foreign key is checked with a full table scan, which turns every \
         parent-row delete into a scan of the whole child table:\n{}",
        scans.join("\n")
    );
}

/// The same property on a database that predates the indexes: they are a plain
/// `CREATE INDEX IF NOT EXISTS` in the schema batch, so opening the store adds
/// them — no version bump and no client handshake change.
#[test]
fn an_existing_database_gains_the_teardown_indexes_when_it_is_opened() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    {
        let db = Database::open(&path).unwrap();
        db.call(|connection| {
            for name in TEARDOWN_INDEXES {
                connection.execute_batch(&format!("DROP INDEX IF EXISTS {name}"))?;
            }
            Ok(())
        })
        .unwrap();
        // The teardown scans are back, which is the state a schema-8 store is in.
        let scans = db
            .read(|connection| Ok(scanned_foreign_keys(connection)))
            .unwrap();
        assert_eq!(
            scans.len(),
            TEARDOWN_INDEXES.len(),
            "expected exactly the teardown lookups to scan:\n{}",
            scans.join("\n")
        );
    }

    let db = Database::open(&path).unwrap();
    for name in TEARDOWN_INDEXES {
        assert!(db.read(|c| Ok(installed(c, name))).unwrap(), "{name}");
    }
    let scans = db
        .read(|connection| Ok(scanned_foreign_keys(connection)))
        .unwrap();
    assert!(
        scans.is_empty(),
        "reopened store still scans:\n{}",
        scans.join("\n")
    );
}
