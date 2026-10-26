//! WAL/commit observations, authored for the final validation phase only.
use super::*;
use crate::session::database::{begin_write_transaction, Database};

#[test]
fn a_long_reader_can_delay_checkpointing_without_joining_the_ordered_commit_queue() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("agent.db");
    let db = Database::open(&path).unwrap();
    db.call(|connection| {
        let tx = begin_write_transaction(connection)?;
        tx.execute_batch(
            "CREATE TABLE synthetic_checkpoint_input(id INTEGER PRIMARY KEY, body BLOB)",
        )?;
        tx.commit()?;
        Ok(())
    })
    .unwrap();
    let reader = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    reader.execute_batch("BEGIN").unwrap();
    assert_eq!(
        reader
            .query_row(
                "SELECT count(*) FROM synthetic_checkpoint_input",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    db.call(|connection| {
        let tx = begin_write_transaction(connection)?;
        for _ in 0..128 {
            tx.execute(
                "INSERT INTO synthetic_checkpoint_input(body) VALUES(zeroblob(8183))",
                [],
            )?;
        }
        tx.commit()?;
        Ok(())
    })
    .unwrap();
    let (busy, frames, copied) = db.checkpoint_for_test().unwrap();
    assert_eq!(
        busy, 0,
        "PASSIVE returns promptly even when a reader pins newer frames"
    );
    assert!(
        frames > copied,
        "the reader must actually hold back checkpoint progress"
    );
    db.call(|connection| {
        assert_eq!(
            connection.pragma_query_value(None, "synchronous", |row| row.get::<_, i64>(0))?,
            2
        );
        assert_eq!(
            connection
                .pragma_query_value(None, "wal_autocheckpoint", |row| row.get::<_, i64>(0))?,
            0
        );
        let tx = begin_write_transaction(connection)?;
        tx.execute(
            "INSERT INTO synthetic_checkpoint_input(body) VALUES('still writable')",
            [],
        )?;
        tx.commit()?;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        db.read(|connection| Ok(connection.query_row(
            "SELECT count(*) FROM synthetic_checkpoint_input",
            [],
            |row| row.get::<_, i64>(0)
        )?))
        .unwrap(),
        129
    );
    reader.execute_batch("ROLLBACK").unwrap();
    let (_, frames, copied) = db.checkpoint_for_test().unwrap();
    assert_eq!(frames, copied);
    let observed = db.metrics();
    assert!(observed["foregroundCommit"]["count"].as_u64().unwrap() >= 3);
    assert!(observed["checkpoint"]["count"].as_u64().unwrap() >= 2);
    assert_eq!(observed["foregroundCommit"]["failures"], 0);
    assert!(observed["databaseBytes"].as_u64().unwrap() > 0);
    assert!(observed["maxWalBytes"].as_u64().unwrap() > 0);
    assert!(observed["writerJob"]["count"].as_u64().unwrap() >= 3);
    assert!(observed["readerJob"]["count"].as_u64().unwrap() >= 1);
    db.shutdown().unwrap();
    assert!(db.checkpoint_for_test().is_err());
}
