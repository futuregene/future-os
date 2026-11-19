//! Detection of catalogue changes made *outside* this process.
//!
//! Every presented list — the sidebar, the recent view, the archived view — is
//! drawn from `app.db`, and any process sharing this home can write that file:
//! `future session archive` in a terminal, a monitor script, another client.
//! Those writes publish nothing here, because every event this host emits comes
//! from its own code paths. A window left open therefore keeps drawing the list
//! it read at startup until some unrelated action happens to refresh it, which
//! is exactly how conversations archived from the CLI days earlier stayed on
//! screen.
//!
//! A change reported here is answered by re-reading *two* streams: the live
//! conversations and the archived shelf, which a `threads-updated` re-list does
//! not cover (see `Event::ArchiveChanged`).
//!
//! The watcher answers one question cheaply and often: *is the list-drawing
//! part of the catalogue still the same?* It deliberately does **not** compare
//! `PRAGMA data_version`. That value moves on every commit, including the ones
//! this process makes for a running reply's activity stamp about once a second,
//! so it would ask the window to re-list every conversation continuously — the
//! rebuild the activity path exists to avoid (see `emit_thread_activity`'s note
//! in `desktop/src-tauri`).
//!
//! What the fingerprint covers: how many conversations exist and how many sit
//! in each lifecycle state, how many are pinned, and the same counts for
//! workspaces. That is precisely the surface a list is drawn from, and it moves
//! only when a row appears, disappears, changes state or changes pinning.
//!
//! What it does not cover: a rename that changes nothing else, and a
//! conversation moved to another workspace. Both change a row a list already
//! holds, and when another process is the author they arrive together with a
//! lifecycle change anyway (a monitor script retires a conversation by renaming
//! *and* archiving it), so neither is worth a re-list on its own.

use rusqlite::Connection;

use super::db::connect;

/// The list-drawing shape of the catalogue, as one comparable value.
///
/// Fields are counts rather than a hash so a mismatch is legible in a failure
/// message, and counts rather than `MAX(updated_at)` so a reply's activity
/// stamp cannot trigger a re-list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogueFingerprint {
    active_threads: i64,
    archived_threads: i64,
    pinned_threads: i64,
    deleted_threads: i64,
    other_threads: i64,
    workspaces: i64,
    archived_workspaces: i64,
    deleted_workspaces: i64,
}

/// One scan of `threads`. Every state a list distinguishes is counted
/// separately, so any transition between them moves the fingerprint: a status
/// that is neither active nor archived (`shelved`) lands in `other_threads`,
/// which is what makes `shelved → active` visible. Pin counting is limited to
/// live rows, matching what the pinned block draws.
const THREAD_COUNTS_SQL: &str = "
SELECT
    COALESCE(SUM(status = 'active' AND deleted_at IS NULL), 0),
    COALESCE(SUM(status = 'archived' AND deleted_at IS NULL), 0),
    COALESCE(SUM(pinned <> 0 AND deleted_at IS NULL), 0),
    COALESCE(SUM(deleted_at IS NOT NULL), 0),
    COALESCE(SUM(status NOT IN ('active', 'archived') AND deleted_at IS NULL), 0)
FROM threads";

/// One scan of `workspaces`: a workspace that appears, disappears or is shelved
/// changes every group drawn from it.
const WORKSPACE_COUNTS_SQL: &str = "
SELECT
    COALESCE(SUM(deleted_at IS NULL), 0),
    COALESCE(SUM(archived_at IS NOT NULL AND deleted_at IS NULL), 0),
    COALESCE(SUM(deleted_at IS NOT NULL), 0)
FROM workspaces";

fn fingerprint_on(conn: &Connection) -> Result<CatalogueFingerprint, crate::Error> {
    let (active_threads, archived_threads, pinned_threads, deleted_threads, other_threads) =
        conn.query_row(THREAD_COUNTS_SQL, [], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?;
    let (workspaces, archived_workspaces, deleted_workspaces) =
        conn.query_row(WORKSPACE_COUNTS_SQL, [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
    Ok(CatalogueFingerprint {
        active_threads,
        archived_threads,
        pinned_threads,
        deleted_threads,
        other_threads,
        workspaces,
        archived_workspaces,
        deleted_workspaces,
    })
}

/// Whether the catalogue changed since `last`, and remember this reading.
///
/// `last` is the caller's memory of the previous poll: `None` primes it (the
/// first poll reports nothing, so starting the watcher does not ask the window
/// to re-list everything it just loaded), and a `Some` that differs from
/// today's reading is a change some other writer made since — which the caller
/// answers by re-listing.
///
/// Keeping the memory in the caller's own value is what makes the job testable:
/// a test drives this with a local `Option` instead of reaching into the
/// process-wide baseline the scheduled pass holds.
pub fn poll_catalogue_change(
    last: &mut Option<CatalogueFingerprint>,
) -> Result<bool, crate::Error> {
    let conn = connect()?;
    let current = fingerprint_on(&conn)?;
    let changed = match *last {
        Some(previous) => previous != current,
        None => false,
    };
    *last = Some(current);
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::test_support::HomeGuard;
    use crate::store::test_support::{seed_thread, seed_workspace};

    /// A fixture home holding one conversation, plus the guard that owns it.
    ///
    /// The guard must live for the whole test: it takes the process-wide `HOME`
    /// lock (a second guard in the same test would deadlock) and its `Drop`
    /// deletes the database the assertions just read.
    fn fixture(label: &str) -> (HomeGuard, String) {
        let home = HomeGuard::new(label);
        // The schema lives on disk once applied; the connection itself goes
        // back to the pool here, so every later write opens its own.
        drop(crate::store::test_support::open_schema_connection());
        let workspace = seed_workspace(&home.path().join("workspaces"), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-watch"));
        (home, thread.id)
    }

    /// Prime the fingerprint, run one store write, then poll: the shape every
    /// assertion below uses.
    ///
    /// Each poll opens its own connection through the pool, exactly as the
    /// scheduled pass does. The write in between stands in for another process:
    /// a change this poll's connection did not make is what the two look like
    /// to a fingerprint, which is the property being tested.
    fn reports_after(write: impl FnOnce()) -> bool {
        let mut last = None;
        assert!(
            !poll_catalogue_change(&mut last).expect("prime the fingerprint"),
            "the priming poll must never report a change"
        );
        write();
        poll_catalogue_change(&mut last).expect("poll after the write")
    }

    /// The case this watcher exists for: a conversation archived by another
    /// writer leaves the recent list, and the window has to hear about it.
    #[test]
    fn reports_a_conversation_archived_by_another_writer() {
        let (_home, thread_id) = fixture("store-watch-archive");
        assert!(
            reports_after(|| {
                crate::store::archive_thread(&thread_id).expect("archive");
            }),
            "an archive made outside the host must be reported"
        );
    }

    /// The reverse transition, so the fingerprint cannot be satisfied by "the
    /// archived count rose" alone.
    #[test]
    fn reports_a_conversation_restored_by_another_writer() {
        let (_home, thread_id) = fixture("store-watch-restore");
        crate::store::archive_thread(&thread_id).expect("seed the archive");
        assert!(
            reports_after(|| {
                crate::store::restore_thread(&thread_id).expect("restore");
            }),
            "a restore made outside the host must be reported"
        );
    }

    /// A list's membership is not only its status: a pin moves a row into the
    /// pinned block, and a delete removes it from every block.
    #[test]
    fn reports_a_pin_and_a_delete_made_by_another_writer() {
        let (_home, thread_id) = fixture("store-watch-pin");
        assert!(
            reports_after(|| {
                crate::store::pin_thread(crate::store::PinThreadInput {
                    thread_id: thread_id.clone(),
                    pinned: true,
                })
                .expect("pin");
            }),
            "a pin made outside the host must be reported"
        );
        assert!(
            reports_after(|| {
                crate::store::delete_thread(&thread_id).expect("delete");
            }),
            "a delete made outside the host must be reported"
        );
    }

    /// A workspace leaving the list takes its whole group with it, and no
    /// member row reports that on its own.
    #[test]
    fn reports_a_workspace_archived_by_another_writer() {
        let (_home, thread_id) = fixture("store-watch-workspace");
        let workspace_id = crate::store::get_thread(&thread_id)
            .expect("read thread")
            .expect("thread exists")
            .workspace_id;
        assert!(
            reports_after(|| {
                crate::store::archive_workspace(&workspace_id, &[]).expect("archive workspace");
            }),
            "a workspace archived outside the host must be reported"
        );
    }

    /// Nothing written: no re-list. This is what keeps the watcher from
    /// becoming a spinner that rebuilds the sidebar for no reason.
    #[test]
    fn reports_nothing_when_nothing_changed() {
        let (_home, _thread_id) = fixture("store-watch-quiet");
        assert!(!reports_after(|| {}), "an idle store must stay quiet");
    }

    /// A reply's activity stamp updates its row about once a second and must
    /// not re-list the conversation. Pinning the property here is what
    /// justifies counts instead of `PRAGMA data_version` (see the module docs).
    #[test]
    fn does_not_report_an_activity_stamp() {
        let (_home, thread_id) = fixture("store-watch-activity");
        assert!(
            !reports_after(|| {
                crate::store::record_thread_message_activity(&thread_id, 1_700_000_000_000)
                    .expect("record activity");
            }),
            "activity must patch its row, not re-list the catalogue"
        );
    }
}
