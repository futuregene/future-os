//! Lossless preservation of pre-Agent GUI history. Archives are read only and
//! never used as LLM context or as an excuse to replace a missing Agent session.
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use super::db::connect;

const VERSION: &str = "v1.2.4-history-preservation";
const TABLES: &[&str] = &["messages", "run_events", "tool_calls", "tool_outputs"];

fn table_exists(conn: &Connection, name: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
        [name],
        |row| row.get(0),
    )
}

pub(super) fn preserve_legacy_history(conn: &Connection) -> Result<(), crate::AppError> {
    let applied: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=?1)",
        [VERSION],
        |row| row.get(0),
    )?;
    if applied {
        return Ok(());
    }
    // The oldest GUI schemas reference the retired tables from runs,
    // approvals and reviews. Rebuild those referencing tables from today's
    // schema in the same transaction; leaving dangling FKs breaks future sends.
    let fresh = Connection::open_in_memory()?;
    fresh.execute_batch(super::schema::SCHEMA)?;
    let foreign_keys: bool = conn.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
    conn.pragma_update(None, "foreign_keys", false)?;
    let result = (|| -> Result<(), crate::AppError> {
        let tx = conn.unchecked_transaction()?;
        for table in TABLES {
            if table_exists(&tx, table)? {
                // Preserve every column, including legacy payloads we no longer
                // interpret. No foreign keys: deleting an old projection must not
                // cause an implicit cascade through its archived history.
                tx.execute_batch(&format!(
                    "CREATE TABLE legacy_{table} AS SELECT * FROM {table}"
                ))?;
                if *table == "messages" {
                    tx.execute_batch("CREATE INDEX legacy_messages_thread ON legacy_messages(thread_id,created_at,id)")?;
                }
            }
        }
        // Save duplicate read bindings BEFORE the released uniqueness migration
        // detaches them. Writes continue to use only threads.agent_session_id.
        tx.execute(
            "INSERT INTO legacy_thread_bindings(thread_id,session_id)
         SELECT id,TRIM(agent_session_id) FROM threads
         WHERE agent_session_id IS NOT NULL AND TRIM(agent_session_id)!=''
           AND EXISTS(SELECT 1 FROM threads other WHERE other.id!=threads.id
                      AND TRIM(other.agent_session_id)=TRIM(threads.agent_session_id))",
            [],
        )?;
        // Only rebuild tables with obsolete FK targets, preserving all shared
        // columns. The source history copies above retain their original shape.
        for table in ["runs", "approval_requests", "review_changesets"] {
            let obsolete: bool = tx.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_list('{table}') WHERE \"table\" IN ('messages','tool_calls'))"),
            [], |row| row.get(0))?;
            if !obsolete {
                continue;
            }
            let sql: String = fresh.query_row(
                "SELECT sql FROM sqlite_schema WHERE type='table' AND name=?1",
                [table],
                |row| row.get(0),
            )?;
            tx.execute_batch(&format!(
                "CREATE TABLE legacy_{table} AS SELECT * FROM {table}"
            ))?;
            let rebuilt = format!("recovery_{table}");
            tx.execute_batch(&sql.replacen(
                &format!("CREATE TABLE {table}"),
                &format!("CREATE TABLE {rebuilt}"),
                1,
            ))?;
            let mut columns = tx.prepare(&format!("SELECT name FROM pragma_table_info('{table}') WHERE name IN (SELECT name FROM pragma_table_info('{rebuilt}'))"))?;
            let names = columns
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let names = names
                .iter()
                .map(|name| format!("\"{name}\""))
                .collect::<Vec<_>>()
                .join(",");
            tx.execute_batch(&format!("INSERT INTO {rebuilt} ({names}) SELECT {names} FROM {table}; DROP TABLE {table}; ALTER TABLE {rebuilt} RENAME TO {table};"))?;
        }
        tx.execute(
            "INSERT INTO schema_migrations(version,applied_at) VALUES (?1,?2)",
            params![VERSION, super::now_millis()],
        )?;
        tx.commit()?;
        Ok(())
    })();
    conn.pragma_update(None, "foreign_keys", foreign_keys)?;
    result?;
    // Reinstall indexes belonging to a rebuilt table. Other released
    // migrations still run normally after this preservation boundary.
    conn.execute_batch(super::schema::SCHEMA)?;
    Ok(())
}

pub fn legacy_session_id(thread_id: &str) -> Result<Option<String>, crate::AppError> {
    let conn = connect()?;
    Ok(conn
        .query_row(
            "SELECT session_id FROM legacy_thread_bindings WHERE thread_id=?1",
            [thread_id],
            |row| row.get(0),
        )
        .optional()?)
}

pub fn legacy_history_entries(thread_id: &str) -> Result<Vec<Value>, crate::AppError> {
    let conn = connect()?;
    read_entries(&conn, thread_id).map_err(Into::into)
}

fn read_entries(conn: &Connection, thread_id: &str) -> rusqlite::Result<Vec<Value>> {
    if !table_exists(conn, "legacy_messages")? {
        return Ok(Vec::new());
    }
    let has_content_type:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('legacy_messages') WHERE name='content_type')",[],|row| row.get(0))?;
    let content_type = if has_content_type {
        "m.content_type"
    } else {
        "'markdown'"
    };
    let mut statement = conn.prepare(&format!(
        "SELECT m.id,m.role,m.content,m.created_at,
           COALESCE(m.run_id,(SELECT id FROM runs WHERE trigger_message_id=m.id AND thread_id=m.thread_id LIMIT 1)),{content_type}
         FROM legacy_messages m WHERE m.thread_id=?1 ORDER BY m.created_at,m.id"
    ))?;
    let entries = statement
        .query_map([thread_id], |row| {
            let role: String = row.get(1)?;
            let content: String = row.get(2)?;
            let mixed = if row.get::<_, String>(5)? == "mixed" {
                serde_json::from_str::<Value>(&content)
                    .ok()
                    .filter(|value| value["type"] == "user_message")
            } else {
                None
            };
            let text = mixed.as_ref()
                .map(|value| value["text"].as_str().unwrap_or_default())
                .unwrap_or(&content);
            Ok(json!({
                "id":row.get::<_,String>(0)?, "kind":role, "role":role,
                "blocks":[{"kind":"text","text":text}],
                "createdAtMs":row.get::<_,i64>(3)?, "runId":row.get::<_,Option<String>>(4)?,
                "metadata":{"attachments":mixed.as_ref().and_then(|value| value.get("attachments"))},
            }))
        })?
        .collect();
    entries
}

/// Explicit user deletion removes recovery copies too, before its run ids go.
pub(super) fn delete_history_in(conn: &Connection, thread_id: &str) -> rusqlite::Result<()> {
    for table in [
        "tool_outputs",
        "tool_calls",
        "run_events",
        "messages",
        "runs",
        "approval_requests",
        "review_changesets",
    ] {
        let archive = format!("legacy_{table}");
        if !table_exists(conn, &archive)? {
            continue;
        }
        let predicate = match table {
            "messages" | "runs" | "approval_requests" | "review_changesets" => "thread_id=?1",
            "tool_outputs" => "tool_call_id IN (SELECT id FROM legacy_tool_calls WHERE run_id IN (SELECT id FROM runs WHERE thread_id=?1))",
            _ => "run_id IN (SELECT id FROM runs WHERE thread_id=?1)",
        };
        conn.execute(
            &format!("DELETE FROM {archive} WHERE {predicate}"),
            [thread_id],
        )?;
    }
    conn.execute(
        "DELETE FROM legacy_thread_bindings WHERE thread_id=?1",
        [thread_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db::apply_schema;

    fn seed(conn: &Connection) {
        conn.execute_batch("INSERT INTO workspaces(id,name,kind,path,created_at,updated_at) VALUES('w','W','user','.',1,1);
            INSERT INTO threads(id,workspace_id,mode,title,created_at,updated_at) VALUES('t','w','chat','History',1,1);
            INSERT INTO runs(id,thread_id,status,created_at,updated_at) VALUES('r','t','completed',1,2);").unwrap();
    }

    #[test]
    fn upgrades_released_and_fresh_databases_without_changing_history_identity() {
        for schema in ["", include_str!("fixtures/v1.2.1.sql")] {
            let conn = Connection::open_in_memory().unwrap();
            if !schema.is_empty() {
                conn.execute_batch(schema).unwrap();
                seed(&conn);
            }
            apply_schema(&conn).unwrap();
            apply_schema(&conn).unwrap();
            let markers: i64 = conn
                .query_row(
                    "SELECT count(*) FROM schema_migrations WHERE version=?1",
                    [VERSION],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(markers, 1);
            if !schema.is_empty() {
                let identity: (String, String) = conn
                    .query_row("SELECT id,thread_id FROM runs WHERE id='r'", [], |row| {
                        Ok((row.get(0)?, row.get(1)?))
                    })
                    .unwrap();
                assert_eq!(identity, ("r".into(), "t".into()));
            }
        }
    }

    #[test]
    fn preserves_pre_agent_payloads_and_removes_obsolete_foreign_keys() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("fixtures/pre-agent.sql"))
            .unwrap();
        seed(&conn);
        conn.execute_batch("INSERT INTO messages(id,thread_id,run_id,role,content,created_at,updated_at) VALUES('u','t','r','user','old question',1,1),('a','t','r','assistant','old answer',2,2);
            INSERT INTO tool_calls(id,run_id,name,kind,input,status,created_at) VALUES('tool','r','read','read','original args','completed',1);
            INSERT INTO tool_outputs(id,tool_call_id,kind,content,created_at) VALUES('out','tool','text','original output',1);
            INSERT INTO run_events(id,run_id,event_type,payload,sequence,created_at) VALUES('event','r','text','original event',1,1);
            UPDATE runs SET trigger_message_id='u' WHERE id='r';
            PRAGMA foreign_keys=ON;").unwrap();
        let mixed = json!({"type":"user_message","text":"visible","inlineContext":"model-only","attachments":[{"name":"old.png","path":"old.png","kind":"image"}]}).to_string();
        conn.execute("INSERT INTO messages(id,thread_id,role,content_type,content,created_at,updated_at) VALUES('mixed','t','user','mixed',?1,3,3)",[&mixed]).unwrap();
        apply_schema(&conn).unwrap();
        apply_schema(&conn).unwrap();
        assert!(!table_exists(&conn, "messages").unwrap());
        assert_eq!(
            read_entries(&conn, "t").unwrap()[1]["blocks"][0]["text"],
            "old answer"
        );
        for (table, column, value) in [
            ("legacy_tool_calls", "input", "original args"),
            ("legacy_tool_outputs", "content", "original output"),
            ("legacy_run_events", "payload", "original event"),
        ] {
            let actual: String = conn
                .query_row(&format!("SELECT {column} FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(actual, value);
        }
        let restored = read_entries(&conn, "t").unwrap();
        assert_eq!(restored[2]["blocks"][0]["text"], "visible");
        assert_eq!(restored[2]["metadata"]["attachments"][0]["name"], "old.png");
        assert_eq!(
            conn.query_row(
                "SELECT content FROM legacy_messages WHERE id='mixed'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            mixed
        );
        // New run/approval tool identifiers need no retired GUI message rows.
        conn.execute("INSERT INTO runs(id,thread_id,trigger_message_id,status,created_at,updated_at) VALUES('new','t','new-user','running',3,3)",[]).unwrap();
        let broken: i64 = conn
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(broken, 0);
        delete_history_in(&conn, "t").unwrap();
        assert!(read_entries(&conn, "t").unwrap().is_empty());
        assert_eq!(
            conn.query_row("SELECT count(*) FROM legacy_tool_outputs", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn failed_preservation_rolls_back_and_keeps_the_original_for_retry() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(super::super::schema::SCHEMA).unwrap();
        seed(&conn);
        conn.execute_batch("CREATE TABLE messages(id TEXT,thread_id TEXT,run_id TEXT,role TEXT,content TEXT,created_at INTEGER);
          INSERT INTO messages VALUES('u','t','r','user','only copy',1);
          CREATE TRIGGER reject_history_marker BEFORE INSERT ON schema_migrations
          WHEN NEW.version='v1.2.4-history-preservation' BEGIN SELECT RAISE(ABORT,'disk failure'); END;
          PRAGMA foreign_keys=ON;").unwrap();
        assert!(apply_schema(&conn).is_err());
        assert!(table_exists(&conn, "messages").unwrap());
        assert!(!table_exists(&conn, "legacy_messages").unwrap());
        assert!(conn
            .pragma_query_value(None, "foreign_keys", |row| row.get::<_, bool>(0))
            .unwrap());
        conn.execute_batch("DROP TRIGGER reject_history_marker")
            .unwrap();
        apply_schema(&conn).unwrap();
        assert_eq!(
            read_entries(&conn, "t").unwrap()[0]["blocks"][0]["text"],
            "only copy"
        );
    }

    #[test]
    fn preserves_duplicate_read_bindings_before_the_released_migration_detaches_them() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("fixtures/v1.2.1.sql"))
            .unwrap();
        seed(&conn);
        conn.execute_batch("UPDATE threads SET agent_session_id='shared' WHERE id='t';
          INSERT INTO threads(id,workspace_id,mode,title,agent_session_id,created_at,updated_at) VALUES('newer','w','chat','Newer','shared',2,2);").unwrap();
        apply_schema(&conn).unwrap();
        let old: Option<String> = conn
            .query_row(
                "SELECT agent_session_id FROM threads WHERE id='t'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(old.is_none());
        let preserved: String = conn
            .query_row(
                "SELECT session_id FROM legacy_thread_bindings WHERE thread_id='t'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(preserved, "shared");
    }
}
