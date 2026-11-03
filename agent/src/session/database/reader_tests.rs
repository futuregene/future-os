//! Run with the complete local suite after the storage refactor is integrated.
use super::*;

#[test]
fn an_open_writer_does_not_serialize_independent_history_reads() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("agent.db")).unwrap();
    db.call(|c| {
        c.execute("INSERT INTO sessions(id) VALUES('committed')", [])?;
        Ok(())
    })
    .unwrap();
    let (arrived, started) = mpsc::sync_channel(1);
    let (release, resume) = mpsc::sync_channel(1);
    let writer = db.clone();
    let handle = std::thread::spawn(move || {
        writer.call(move |c| {
            let tx = c.transaction()?;
            tx.execute("INSERT INTO sessions(id) VALUES('pending')", [])?;
            arrived.send(()).unwrap();
            resume.recv().unwrap();
            tx.commit()?;
            Ok(())
        })
    });
    started.recv().unwrap();
    // Release the writer even if an assertion fails, so failure cannot deadlock
    // cleanup. This select must execute on a different read connection.
    let reader = db.clone();
    let (read_done, answer) = mpsc::sync_channel(1);
    let reading = std::thread::spawn(move || {
        let _ = read_done.send(reader.read(|c| {
            Ok(c.query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))?)
        }));
    });
    let count = answer.recv_timeout(std::time::Duration::from_secs(2));
    release.send(()).unwrap();
    handle.join().unwrap().unwrap();
    reading.join().unwrap();
    assert_eq!(count.unwrap().unwrap(), 1);
    assert_eq!(
        db.read(|c| Ok(c.query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))?))
            .unwrap(),
        2
    );
}

#[test]
fn readers_cannot_mutate_and_shutdown_closes_both_paths() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("agent.db")).unwrap();
    assert!(db
        .read(|c| {
            c.execute("INSERT INTO sessions(id) VALUES('forbidden')", [])?;
            Ok(())
        })
        .is_err());
    db.shutdown().unwrap();
    assert!(db.read(|_| Ok(())).is_err());
    assert!(db.call(|_| Ok(())).is_err());
}
