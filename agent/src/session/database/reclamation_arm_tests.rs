use super::*;

/// Evidence for `reclaim_space_when_idle`'s "no further progress" branch
/// (`reclamation.rs` 45-46) and its twin in the test-only `Database::reclaim`
/// helper (79-80). Both wait for `PRAGMA incremental_vacuum` to leave
/// `freelist_count` **unchanged while returning Ok**.
///
/// Ten candidate states were probed for that outcome — a deleted filler table,
/// free pages in the middle with a live tail, an open cursor on the same
/// connection, a foreign read transaction, the connection's own transaction,
/// free pages created inside the same write transaction, a `DROP TABLE`,
/// `auto_vacuum=2` requested on a populated database (and after reopening it),
/// and `auto_vacuum=1`. **None produced a silent no-op**: this build frees
/// exactly one page per call, and returns `database is locked` when it cannot
/// write. That is the whole reason the branch is carried as
/// `unreachable-in-this-environment` rather than as coverable work.
///
/// This test is the falsifier, not a decoration: if a future SQLite build
/// starts reporting no progress, the invariant below fails — which is exactly
/// the moment the branch becomes reachable and the ledger row must change.
#[test]
fn incremental_vacuum_never_reports_no_progress_without_an_error() {
    /// Run one `incremental_vacuum(1)` and report (before, after, result).
    fn vacuum(connection: &Connection) -> anyhow::Result<(i64, i64, Result<(), rusqlite::Error>)> {
        let before: i64 = connection.pragma_query_value(None, "freelist_count", |r| r.get(0))?;
        let outcome = connection.execute_batch("PRAGMA incremental_vacuum(1)");
        let after: i64 = connection.pragma_query_value(None, "freelist_count", |r| r.get(0))?;
        Ok((before, after, outcome))
    }

    /// The invariant under test: Ok means the freelist went DOWN.
    fn assert_progress_or_error(
        tag: &str,
        before: i64,
        after: i64,
        outcome: &Result<(), rusqlite::Error>,
    ) {
        match outcome {
            Ok(()) => assert!(
                after < before,
                "{tag}: incremental_vacuum returned Ok without freeing a page \
                 (free {before} -> {after}) — the no-progress branch is reachable \
                 in this build and the ledger row must be reclassified"
            ),
            Err(error) => assert!(
                error.to_string().contains("locked"),
                "{tag}: unexpected failure {error}"
            ),
        }
    }

    fn filler(connection: &Connection, table: &str, rows: usize) -> anyhow::Result<()> {
        connection.execute_batch(&format!(
            "CREATE TABLE {table}(id INTEGER PRIMARY KEY, body BLOB);"
        ))?;
        for _ in 0..rows {
            connection.execute(
                &format!("INSERT INTO {table}(body) VALUES (zeroblob(8183))"),
                [],
            )?;
        }
        connection.execute_batch(&format!("DELETE FROM {table}"))?;
        Ok(())
    }

    // 1. The plain case, and the exact size of one slice's progress.
    let dir = tempfile::tempdir().unwrap();
    let store = Database::open(&dir.path().join("a.db")).unwrap();
    store.db_call_filler("f", 40).unwrap();
    let (before, after, outcome) = store.call(|c| vacuum(c)).unwrap();
    assert!(outcome.is_ok(), "the plain case must run");
    assert_eq!(
        before - after,
        1,
        "one incremental_vacuum(1) slice frees exactly one page"
    );

    // 2. A foreign writer holds the lock and the connection will not wait: the
    //    vacuum must report the failure rather than quietly doing nothing.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b.db");
    let store = Database::open(&path).unwrap();
    store.db_call_filler("f", 40).unwrap();
    let writer = Connection::open(&path).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let (before, after, outcome) = store
        .call(|c| {
            c.busy_timeout(std::time::Duration::ZERO)?;
            vacuum(c)
        })
        .unwrap();
    assert_progress_or_error("foreign write lock", before, after, &outcome);
    assert!(
        outcome.is_err(),
        "a held write lock must surface as an error"
    );
    writer.execute_batch("ROLLBACK").unwrap();

    // 3. An open cursor on the same connection.
    let dir = tempfile::tempdir().unwrap();
    let store = Database::open(&dir.path().join("c.db")).unwrap();
    let (before, after, outcome) = store
        .call(|c| {
            filler(c, "f", 40)?;
            let mut statement = c.prepare("SELECT id, body FROM f")?;
            let mut rows = statement.query([])?;
            let _ = rows.next()?;
            let out = vacuum(c)?;
            drop(rows);
            Ok(out)
        })
        .unwrap();
    assert_progress_or_error("same-connection cursor", before, after, &outcome);

    // 4. A foreign connection holding an open read transaction.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("d.db");
    let store = Database::open(&path).unwrap();
    store.db_call_filler("f", 40).unwrap();
    let reader = Connection::open(&path).unwrap();
    reader
        .execute_batch("BEGIN; SELECT count(*) FROM f;")
        .unwrap();
    let (before, after, outcome) = store.call(|c| vacuum(c)).unwrap();
    assert_progress_or_error("foreign read transaction", before, after, &outcome);
    reader.execute_batch("ROLLBACK").unwrap();

    // 5. Inside the connection's own transaction, with the free pages created
    //    in that same transaction (so nothing is committed yet).
    let dir = tempfile::tempdir().unwrap();
    let store = Database::open(&dir.path().join("e.db")).unwrap();
    store
        .call(|c| {
            c.execute_batch("CREATE TABLE f(id INTEGER PRIMARY KEY, body BLOB);")?;
            for _ in 0..40 {
                c.execute("INSERT INTO f(body) VALUES (zeroblob(8183))", [])?;
            }
            Ok(())
        })
        .unwrap();
    let (before, after, outcome) = store
        .call(|c| {
            c.execute_batch("BEGIN IMMEDIATE")?;
            c.execute_batch("DELETE FROM f")?;
            let out = vacuum(c)?;
            c.execute_batch("ROLLBACK")?;
            Ok(out)
        })
        .unwrap();
    assert_progress_or_error("free pages in own write txn", before, after, &outcome);

    // 6. Free pages in the middle, with a live table owning the tail — the case
    //    that has to relocate a page rather than truncate.
    let dir = tempfile::tempdir().unwrap();
    let store = Database::open(&dir.path().join("f.db")).unwrap();
    store
        .call(|c| {
            c.execute_batch("CREATE TABLE mid(id INTEGER PRIMARY KEY, body BLOB);")?;
            for _ in 0..40 {
                c.execute("INSERT INTO mid(body) VALUES (zeroblob(8183))", [])?;
            }
            c.execute_batch("CREATE TABLE tail(id INTEGER PRIMARY KEY, body BLOB);")?;
            for _ in 0..40 {
                c.execute("INSERT INTO tail(body) VALUES (zeroblob(8183))", [])?;
            }
            c.execute_batch("DELETE FROM mid")?;
            Ok(())
        })
        .unwrap();
    let (before, after, outcome) = store.call(|c| vacuum(c)).unwrap();
    assert_progress_or_error("free pages in the middle", before, after, &outcome);

    // 7. Free pages from a DROP TABLE, which also frees the root page.
    let dir = tempfile::tempdir().unwrap();
    let store = Database::open(&dir.path().join("g.db")).unwrap();
    store
        .call(|c| {
            c.execute_batch("CREATE TABLE f(id INTEGER PRIMARY KEY, body BLOB);")?;
            for _ in 0..40 {
                c.execute("INSERT INTO f(body) VALUES (zeroblob(8183))", [])?;
            }
            Ok(())
        })
        .unwrap();
    let (before, after, outcome) = store
        .call(|c| {
            c.execute_batch("DROP TABLE f")?;
            vacuum(c)
        })
        .unwrap();
    assert_progress_or_error("after drop table", before, after, &outcome);

    // 8. `auto_vacuum=2` requested on a POPULATED database (the mode switch is
    //    deferred to VACUUM, so the pointer map may be missing).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("h.db");
    let store = Database::open(&path).unwrap();
    store
        .call(|c| {
            c.execute_batch("PRAGMA auto_vacuum=0;")?;
            filler(c, "f", 40)?;
            c.execute_batch("PRAGMA auto_vacuum=2;")?;
            Ok(())
        })
        .unwrap();
    let (before, after, outcome) = store.call(|c| vacuum(c)).unwrap();
    assert_progress_or_error("mode 2 requested on populated db", before, after, &outcome);

    // 9. The same file reopened, so the header setting is the one in force.
    let store = Database::open(&path).unwrap();
    let (before, after, outcome) = store.call(|c| vacuum(c)).unwrap();
    assert_progress_or_error("reopened after mode switch", before, after, &outcome);
}

/// The idle-reclamation **failure** arm (`database/mod.rs` 79-82): the worker's
/// periodic reclaim finds the database's write lock held elsewhere, so
/// `PRAGMA incremental_vacuum(1)` returns `database is locked` under the
/// `busy_timeout(0)` that `reclaim_space_when_idle` sets — and the worker reports
/// it once, stays alive, and resumes when the lock is released.
///
/// This exists because the arm spent ten segments carried as
/// `unreachable-in-this-environment` on the strength of a probe that had only
/// refuted *file damage* (F-54). It then ran once under machine load, which is
/// what a racy arm does; this fixture makes it run **every** time.
///
/// Two things have to hold, and both are asserted before the wait so a future
/// change cannot turn this into a test of nothing:
/// * the database is in `auto_vacuum = 2` (incremental) — otherwise
///   `remaining` is 0 and the reclaim loop breaks before the vacuum;
/// * the freelist is **non-empty** — same reason. Both are properties of a
///   fresh store plus a deleted filler table, which is why the fixture builds one.
///
/// The lock is a foreign `BEGIN IMMEDIATE` on the store's own file: a write lock
/// is what makes the vacuum's first page move return SQLITE_BUSY, and the worker's
/// own connection cannot wait it out because its busy timeout is zero.
#[test]
fn the_idle_reclaim_reports_a_lock_it_cannot_wait_for_and_then_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("worker.db");
    let store = Database::open(&path).unwrap();
    store.db_call_filler("f", 40).unwrap();

    // Preconditions (see the note above): with either of these false the reclaim
    // loop returns before the padding vacuum and the arm cannot be reached.
    let (mode, free) = store
        .call(|c| {
            Ok((
                c.pragma_query_value(None, "auto_vacuum", |r| r.get::<_, i64>(0))?,
                c.pragma_query_value(None, "freelist_count", |r| r.get::<_, i64>(0))?,
            ))
        })
        .unwrap();
    assert_eq!(mode, 2, "the reclaim loop needs incremental auto-vacuum");
    assert!(free > 0, "the reclaim loop needs free pages to move");

    // The foreign writer holds the lock while the worker's next 50 ms idle slice
    // fires; the reclaim cannot wait it out, so the worker takes the `Err` arm.
    let writer = Connection::open(&path).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    // Long enough for several idle slices, so "the slice happened to land after
    // the release" cannot explain a pass.
    std::thread::sleep(Duration::from_millis(250));
    writer.execute_batch("ROLLBACK").unwrap();

    // The observable the arm is for: the worker survived the failure. A job after
    // it still runs (the loop did not break, and the connection is not poisoned).
    let survived =
        store.call(|c| Ok(c.pragma_query_value(None, "freelist_count", |r| r.get::<_, i64>(0))?));
    assert!(
        survived.is_ok(),
        "the worker must survive a reclaim error: {survived:?}"
    );

    // And it recovers: once the lock is gone, reclamation drains the freelist.
    // The reclaim hint is left armed by the failed slice (`reclaim_requested`
    // stays true because the error path never stores `!done`), so nothing has to
    // re-arm it — the worker does this by itself.
    //
    // The wait must leave the worker **idle**: its reclaim runs only on a
    // `recv_timeout` expiry, so polling `freelist_count` every few milliseconds
    // would starve it forever — the check itself would be the reason the drain
    // never happens. Each probe is a single fast call, separated by longer quiet
    // stretches in which several 50 ms slices fire.
    let mut drained = false;
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(150));
        let free = store
            .call(|c| Ok(c.pragma_query_value(None, "freelist_count", |r| r.get::<_, i64>(0))?))
            .unwrap();
        if free == 0 {
            drained = true;
            break;
        }
    }
    assert!(
        drained,
        "reclamation must resume once the foreign lock is released"
    );
}

/// Small helper so the test above can build its freelist without repeating the
/// `call` wrapper at every site.
trait DatabaseFiller {
    fn db_call_filler(&self, table: &str, rows: usize) -> anyhow::Result<()>;
}

impl DatabaseFiller for Database {
    fn db_call_filler(&self, table: &str, rows: usize) -> anyhow::Result<()> {
        let table = table.to_string();
        self.call(move |c| {
            c.execute_batch(&format!(
                "CREATE TABLE {table}(id INTEGER PRIMARY KEY, body BLOB);"
            ))?;
            for _ in 0..rows {
                c.execute(
                    &format!("INSERT INTO {table}(body) VALUES (zeroblob(8183))"),
                    [],
                )?;
            }
            c.execute_batch(&format!("DELETE FROM {table}"))?;
            Ok(())
        })
    }
}
