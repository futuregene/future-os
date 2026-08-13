//! Store CRUD for the shadow review pipeline (see desktop/ER.md §4.10):
//! before/after snapshots, the per-Run `run_snapshot` changeset, its file
//! rows, the "latest ended Run" lookup, and concurrency overlap marking.

use rusqlite::{params, OptionalExtension};
use serde::Serialize;

use super::db::*;
use super::records::*;
use super::util::*;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewChangesetRecord {
    pub id: String,
    pub thread_id: String,
    pub run_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub title: String,
    pub summary: Option<String>,
    pub status: String,
    pub files_changed: i64,
    pub additions: i64,
    pub deletions: i64,
    pub source_kind: String,
    pub workspace_id: Option<String>,
    pub before_snapshot_id: Option<String>,
    pub after_snapshot_id: Option<String>,
    pub binary_files: i64,
    pub omitted_files: i64,
    pub completeness: String,
    pub confidence: String,
    pub overlapped: bool,
    pub error_message: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFileChangeRecord {
    pub id: String,
    pub changeset_id: String,
    pub target_type: String,
    pub target_id: Option<String>,
    pub path: Option<String>,
    pub change_type: String,
    pub before_ref: Option<String>,
    pub after_ref: Option<String>,
    pub diff: Option<String>,
    pub summary: Option<String>,
    pub additions: i64,
    pub deletions: i64,
    pub previous_path: Option<String>,
    pub binary: bool,
    pub before_size: Option<i64>,
    pub after_size: Option<i64>,
    pub mime: Option<String>,
    pub diff_truncated: bool,
    pub omission_reason: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSnapshotRecord {
    pub id: String,
    pub workspace_id: String,
    pub thread_id: String,
    pub run_id: String,
    pub phase: String,
    pub commit_id: Option<String>,
    pub tree_id: Option<String>,
    pub status: String,
    pub file_count: i64,
    pub total_bytes: i64,
    pub ignored_count: i64,
    pub omitted_count: i64,
    pub error_message: Option<String>,
    pub created_at: i64,
}

sql_record!(pub(super) REVIEW_CHANGESET_COLUMNS, review_changeset_from_row -> ReviewChangesetRecord {
    id, thread_id, run_id, tool_call_id, title, summary, status,
    files_changed, additions, deletions, source_kind, workspace_id,
    before_snapshot_id, after_snapshot_id, binary_files, omitted_files,
    completeness, confidence, overlapped, error_message, created_at, updated_at,
});

sql_record!(pub(super) REVIEW_FILE_CHANGE_COLUMNS, review_file_change_from_row -> ReviewFileChangeRecord {
    id, changeset_id, target_type, target_id, path, change_type,
    before_ref, after_ref, diff, summary, additions, deletions,
    previous_path, binary, before_size, after_size, mime, diff_truncated,
    omission_reason, created_at, updated_at,
});

sql_record!(pub(super) REVIEW_SNAPSHOT_COLUMNS, review_snapshot_from_row -> ReviewSnapshotRecord {
    id, workspace_id, thread_id, run_id, phase, commit_id, tree_id, status,
    file_count, total_bytes, ignored_count, omitted_count, error_message, created_at,
});

/// `review_changesets.status` is `NOT NULL` and only meaningful for the legacy
/// apply/discard flow. `run_snapshot` changesets do not use it, so they store
/// this sentinel (§8.2).
const RUN_SNAPSHOT_STATUS: &str = "n/a";

pub fn create_review_snapshot(
    input: CreateReviewSnapshotInput,
) -> Result<ReviewSnapshotRecord, crate::AppError> {
    let conn = connect()?;
    let now = now_millis();
    let id = create_id("rsnap");
    conn.execute(
        "INSERT INTO review_snapshots (
             id, workspace_id, thread_id, run_id, phase, commit_id, tree_id, status,
             file_count, total_bytes, ignored_count, omitted_count, error_message, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
         ON CONFLICT(run_id, phase) DO UPDATE SET
             commit_id = excluded.commit_id,
             tree_id = excluded.tree_id,
             status = excluded.status,
             file_count = excluded.file_count,
             total_bytes = excluded.total_bytes,
             ignored_count = excluded.ignored_count,
             omitted_count = excluded.omitted_count,
             error_message = excluded.error_message,
             created_at = excluded.created_at",
        params![
            id,
            input.workspace_id,
            input.thread_id,
            input.run_id,
            input.phase,
            input.commit_id,
            input.tree_id,
            input.status,
            input.file_count,
            input.total_bytes,
            input.ignored_count,
            input.omitted_count,
            input.error_message,
            now,
        ],
    )?;

    loaded(
        get_review_snapshot(&input.run_id, &input.phase)?,
        "Review snapshot",
    )
}

pub fn get_review_snapshot(
    run_id: &str,
    phase: &str,
) -> Result<Option<ReviewSnapshotRecord>, crate::AppError> {
    let conn = connect()?;
    conn.query_row(
        &format!(
            "SELECT {REVIEW_SNAPSHOT_COLUMNS} FROM review_snapshots WHERE run_id = ?1 AND phase = ?2"
        ),
        params![run_id, phase],
        review_snapshot_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}

/// Create-or-replace the single `run_snapshot` changeset for a Run, along with
/// its file rows. Replacing keeps retries (§10.4) idempotent.
pub fn upsert_run_changeset(
    input: UpsertRunChangesetInput,
) -> Result<ReviewChangesetRecord, crate::AppError> {
    let mut conn = connect()?;
    let now = now_millis();
    let tx = conn.transaction()?;

    // Drop any prior run_snapshot changeset (and its file rows) for this Run.
    let existing: Option<String> = tx
        .query_row(
            "SELECT id FROM review_changesets
             WHERE run_id = ?1 AND source_kind = 'run_snapshot'
             LIMIT 1",
            params![input.run_id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(old_id) = existing {
        tx.execute(
            "DELETE FROM review_file_changes WHERE changeset_id = ?1",
            params![old_id],
        )?;
        tx.execute(
            "DELETE FROM review_changesets WHERE id = ?1",
            params![old_id],
        )?;
    }

    let changeset_id = create_id("review");
    tx.execute(
        "INSERT INTO review_changesets (
             id, thread_id, run_id, tool_call_id, title, summary, status,
             files_changed, additions, deletions, source_kind, workspace_id,
             before_snapshot_id, after_snapshot_id, binary_files, omitted_files,
             completeness, confidence, overlapped, error_message, created_at, updated_at
         ) VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, ?8, ?9, 'run_snapshot', ?10,
                   ?11, ?12, ?13, ?14, ?15, ?16, 0, ?17, ?18, ?18)",
        params![
            changeset_id,
            input.thread_id,
            input.run_id,
            input.title,
            input.summary,
            RUN_SNAPSHOT_STATUS,
            input.files_changed,
            input.additions,
            input.deletions,
            input.workspace_id,
            input.before_snapshot_id,
            input.after_snapshot_id,
            input.binary_files,
            input.omitted_files,
            input.completeness,
            input.confidence,
            input.error_message,
            now,
        ],
    )?;

    for file in &input.files {
        tx.execute(
            "INSERT INTO review_file_changes (
                 id, changeset_id, target_type, target_id, path, change_type,
                 before_ref, after_ref, diff, summary, additions, deletions,
                 previous_path, binary, before_size, after_size, mime,
                 diff_truncated, omission_reason, created_at, updated_at
             ) VALUES (?1, ?2, 'file', NULL, ?3, ?4, NULL, NULL, ?5, ?6, ?7, ?8,
                       ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?16)",
            params![
                create_id("review_file"),
                changeset_id,
                file.path,
                file.change_type,
                file.diff,
                file.summary,
                file.additions,
                file.deletions,
                file.previous_path,
                file.binary as i64,
                file.before_size,
                file.after_size,
                file.mime,
                file.diff_truncated as i64,
                file.omission_reason,
                now,
            ],
        )?;
    }

    tx.commit()?;

    loaded(get_run_changeset(&input.run_id)?, "Run changeset")
}

pub fn get_run_changeset(run_id: &str) -> Result<Option<ReviewChangesetRecord>, crate::AppError> {
    let conn = connect()?;
    conn.query_row(
        &format!(
            "SELECT {REVIEW_CHANGESET_COLUMNS} FROM review_changesets
             WHERE run_id = ?1 AND source_kind = 'run_snapshot' LIMIT 1"
        ),
        params![run_id],
        review_changeset_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}

/// The `run_snapshot` changeset of the Thread's latest *ended* Run (§2.2):
/// strictly the most recent ended Run, never skipping a no-change Run.
pub fn get_last_run_changeset(
    thread_id: &str,
) -> Result<Option<ReviewChangesetRecord>, crate::AppError> {
    // Columns qualified with `c.` because the JOIN onto `runs` makes several
    // names (id, thread_id, status, created_at, updated_at) ambiguous.
    let cols = qualify_columns("c", REVIEW_CHANGESET_COLUMNS);
    let conn = connect()?;
    conn.query_row(
        &format!(
            "SELECT {cols} FROM review_changesets c
             JOIN runs r ON r.id = c.run_id
             WHERE c.thread_id = ?1 AND c.source_kind = 'run_snapshot'
             ORDER BY COALESCE(r.ended_at, r.updated_at) DESC, c.created_at DESC
             LIMIT 1"
        ),
        params![thread_id],
        review_changeset_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}

/// Mark a Run's changeset (and any concurrently-overlapping peer Runs in the
/// same Workspace) as `overlapped` (§12.5). Overlap is derived purely from the
/// snapshot time windows; no extra in-memory state.
pub fn mark_run_overlapped(workspace_id: &str, run_id: &str) -> Result<(), crate::AppError> {
    let mut conn = connect()?;
    let now = now_millis();
    // BEGIN IMMEDIATE so the window reads, the peer scan, and the overlap updates
    // all run against one consistent snapshot under the write lock — a concurrent
    // Run finalizing its snapshot between the scan and the updates can't slip
    // through unmarked (§12.5).
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

    let before_ts: Option<i64> = tx
        .query_row(
            "SELECT created_at FROM review_snapshots WHERE run_id = ?1 AND phase = 'before'",
            params![run_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(before_ts) = before_ts else {
        return Ok(());
    };
    let after_ts: i64 = tx
        .query_row(
            "SELECT created_at FROM review_snapshots WHERE run_id = ?1 AND phase = 'after'",
            params![run_id],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(now);

    // Peers in the same Workspace whose [before, after|now] window intersects
    // this Run's [before_ts, after_ts]. Scoped so the statement is dropped before
    // the updates/commit borrow the transaction.
    let peers: Vec<String> = {
        let mut stmt = tx.prepare(
            "SELECT DISTINCT b.run_id
             FROM review_snapshots b
             LEFT JOIN review_snapshots a ON a.run_id = b.run_id AND a.phase = 'after'
             WHERE b.phase = 'before'
               AND b.workspace_id = ?1
               AND b.run_id != ?2
               AND b.created_at <= ?3
               AND COALESCE(a.created_at, ?4) >= ?5",
        )?;
        let args = params![workspace_id, run_id, after_ts, now, before_ts];
        let rows = stmt.query_map(args, |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    if peers.is_empty() {
        return Ok(());
    }

    set_overlapped(&tx, run_id, now)?;
    for peer in &peers {
        set_overlapped(&tx, peer, now)?;
    }
    tx.commit()?;
    Ok(())
}

fn set_overlapped(conn: &rusqlite::Connection, run_id: &str, now: i64) -> rusqlite::Result<()> {
    const SQL: &str = "UPDATE review_changesets
         SET overlapped = 1, updated_at = ?2
         WHERE run_id = ?1 AND source_kind = 'run_snapshot'";
    conn.execute(SQL, params![run_id, now])?;
    Ok(())
}

// ── retention / recovery / consistency (Phase 2) ────────────────────────────

/// Delete all review rows for a single Run (file changes, the `run_snapshot`
/// changeset, and snapshots), in FK-safe order. Transaction-injecting: the
/// three DELETEs must land atomically — a partial delete either leaves a
/// changeset with no file rows (renders as an empty review) or orphaned
/// snapshots that `list_unmaterialized_runs` would misread as crash-recovery
/// candidates and re-materialize against pruned shadow refs.
pub(super) fn delete_run_review_in(
    conn: &rusqlite::Connection,
    run_id: &str,
) -> rusqlite::Result<()> {
    const DELETE_FILES_SQL: &str = "DELETE FROM review_file_changes WHERE changeset_id IN (
             SELECT id FROM review_changesets WHERE run_id = ?1 AND source_kind = 'run_snapshot'
         )";
    conn.execute(DELETE_FILES_SQL, params![run_id])?;
    const DELETE_CHANGESET_SQL: &str =
        "DELETE FROM review_changesets WHERE run_id = ?1 AND source_kind = 'run_snapshot'";
    conn.execute(DELETE_CHANGESET_SQL, params![run_id])?;
    const DELETE_SNAPSHOTS_SQL: &str = "DELETE FROM review_snapshots WHERE run_id = ?1";
    conn.execute(DELETE_SNAPSHOTS_SQL, params![run_id])?;
    Ok(())
}

/// Prune a Thread's `run_snapshot` changesets to the newest `keep`, deleting the
/// older ones' review data. Returns `(workspace_id, run_id)` for each pruned Run
/// so the caller can delete its shadow refs (§12.3).
pub fn prune_thread_changesets(
    thread_id: &str,
    keep: usize,
) -> Result<Vec<(String, String)>, crate::AppError> {
    // One transaction end-to-end: the ordering read and every per-run cascade
    // see a single consistent snapshot, and an interrupted prune can't leave a
    // half-deleted run behind.
    let mut conn = connect()?;
    let tx = conn.transaction()?;
    let ordered: Vec<(String, Option<String>)> = {
        let mut stmt = tx.prepare(
            "SELECT c.run_id, c.workspace_id
             FROM review_changesets c
             JOIN runs r ON r.id = c.run_id
             WHERE c.thread_id = ?1 AND c.source_kind = 'run_snapshot'
             ORDER BY COALESCE(r.ended_at, r.updated_at) DESC, c.created_at DESC",
        )?;
        let rows = stmt.query_map(params![thread_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let mut pruned = Vec::new();
    for (run_id, workspace_id) in ordered.into_iter().skip(keep) {
        delete_run_review_in(&tx, &run_id)?;
        if let Some(workspace_id) = workspace_id {
            pruned.push((workspace_id, run_id));
        }
    }
    tx.commit()?;
    Ok(pruned)
}

/// Runs interrupted by a crash: a `before` snapshot exists but there is no
/// `after` snapshot and no changeset. Returns `(run_id, thread_id, workspace_id)`
/// (§6.6).
/// Runs that have a usable `before` snapshot but no materialized `run_snapshot`
/// changeset yet. Covers two restart-recovery shapes (§6.6, B-6): an
/// interrupted Run with no `after` snapshot, and a finished Run whose deferred
/// materialize never ran before exit (its `after` snapshot is present). The
/// `after`-present case is deliberately *not* excluded — the old query's
/// `NOT EXISTS(after)` dropped it, leaving such Runs permanently
/// changeset-less.
pub fn list_unmaterialized_runs() -> Result<Vec<(String, String, String)>, crate::AppError> {
    let conn = connect()?;
    list_unmaterialized_runs_in(&conn)
}

fn list_unmaterialized_runs_in(
    conn: &rusqlite::Connection,
) -> Result<Vec<(String, String, String)>, crate::AppError> {
    const SQL: &str = "SELECT s.run_id, s.thread_id, s.workspace_id
         FROM review_snapshots s
         WHERE s.phase = 'before' AND s.status != 'failed'
           AND NOT EXISTS (
             SELECT 1 FROM review_changesets c
             WHERE c.run_id = s.run_id AND c.source_kind = 'run_snapshot'
           )";
    let mut stmt = conn.prepare(SQL)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::AppError::from)
}

/// All non-failed snapshots that pin a commit, for the startup consistency check.
/// Returns `(snapshot_id, workspace_id, commit_id)` (§8.4).
pub fn list_snapshots_with_commits() -> Result<Vec<(String, String, String)>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn.prepare(
        "SELECT id, workspace_id, commit_id
         FROM review_snapshots
         WHERE status != 'failed' AND commit_id IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::AppError::from)
}

/// Mark a snapshot failed (e.g. its commit went missing), so the derived
/// `snapshotStatus` becomes `unavailable` (§8.4).
pub fn mark_snapshot_failed(snapshot_id: &str, reason: &str) -> Result<(), crate::AppError> {
    let conn = connect()?;
    conn.execute(
        "UPDATE review_snapshots SET status = 'failed', error_message = ?2 WHERE id = ?1",
        params![snapshot_id, reason],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::{params, Connection};

    use super::*;
    use crate::store::schema::SCHEMA;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory database");
        conn.execute_batch(SCHEMA).expect("initialize test schema");
        // The recovery query is exercised in isolation; insert snapshot/changeset
        // rows directly without their workspace/thread/run parents.
        conn.execute_batch("PRAGMA foreign_keys = OFF;")
            .expect("disable foreign keys");
        conn
    }

    fn insert_snapshot(conn: &Connection, run_id: &str, phase: &str) {
        conn.execute(
            "INSERT INTO review_snapshots (id, workspace_id, thread_id, run_id, phase, status, created_at)
             VALUES (?1, 'ws', 'thread', ?2, ?3, 'complete', 1)",
            params![format!("{run_id}_{phase}"), run_id, phase],
        )
        .expect("insert snapshot");
    }

    fn insert_changeset(conn: &Connection, run_id: &str) {
        conn.execute(
            "INSERT INTO review_changesets (id, thread_id, run_id, title, status, source_kind, created_at, updated_at)
             VALUES (?1, 'thread', ?2, 't', 'ready', 'run_snapshot', 1, 1)",
            params![format!("cs_{run_id}"), run_id],
        )
        .expect("insert changeset");
    }

    /// B-6: a Run with before+after but no changeset (deferred materialize never
    /// ran before exit) must be recoverable — the old `NOT EXISTS(after)` query
    /// wrongly excluded it.
    #[test]
    fn lists_finished_but_unmaterialized_run() {
        let conn = test_conn();
        insert_snapshot(&conn, "run_b6", "before");
        insert_snapshot(&conn, "run_b6", "after");
        let runs = list_unmaterialized_runs_in(&conn).unwrap();
        assert!(runs.iter().any(|(run_id, ..)| run_id == "run_b6"));
    }

    /// The interrupted shape (before only, no after) is still listed.
    #[test]
    fn lists_interrupted_run() {
        let conn = test_conn();
        insert_snapshot(&conn, "run_int", "before");
        let runs = list_unmaterialized_runs_in(&conn).unwrap();
        assert!(runs.iter().any(|(run_id, ..)| run_id == "run_int"));
    }

    /// A Run that already has a materialized changeset is excluded.
    #[test]
    fn excludes_already_materialized_run() {
        let conn = test_conn();
        insert_snapshot(&conn, "run_done", "before");
        insert_snapshot(&conn, "run_done", "after");
        insert_changeset(&conn, "run_done");
        let runs = list_unmaterialized_runs_in(&conn).unwrap();
        assert!(!runs.iter().any(|(run_id, ..)| run_id == "run_done"));
    }

    // ── connect()-backed API surface (fake HOME) ────────────────────────────

    use crate::store::db::test_support::guarded_conn;

    /// Fake-HOME database with the schema applied; the guard keeps HOME pinned
    /// for the rest of the test (later `connect()` calls reuse it).
    fn fresh_db(label: &str) -> crate::auth_store::test_support::HomeGuard {
        let home = crate::auth_store::test_support::HomeGuard::new(label);
        let conn = connect().expect("connect");
        apply_schema(&conn).expect("apply schema");
        drop(conn);
        home
    }

    /// ws1/t1 plus one completed run per id (`ended_at` increasing).
    fn seed_graph(conn: &Connection, runs: &[&str]) {
        conn.execute_batch(
            "INSERT INTO workspaces (
                 id, name, kind, path, cleanup_status, created_at, updated_at
             ) VALUES ('ws1', 'WS', 'temporary', '/tmp/ws1', 'active', 1, 1);
             INSERT INTO threads (
                 id, workspace_id, mode, title, created_at, updated_at
             ) VALUES ('t1', 'ws1', 'chat', 'T', 1, 1);",
        )
        .expect("seed workspace/thread");
        for (index, run_id) in runs.iter().enumerate() {
            conn.execute(
                "INSERT INTO runs (
                     id, thread_id, status, ended_at, created_at, updated_at
                 ) VALUES (?1, 't1', 'completed', ?2, ?2, ?2)",
                params![run_id, (index as i64 + 1) * 100],
            )
            .expect("seed run");
        }
    }

    fn snapshot_input(
        run_id: &str,
        phase: &str,
        commit: Option<&str>,
    ) -> CreateReviewSnapshotInput {
        CreateReviewSnapshotInput {
            workspace_id: "ws1".to_string(),
            thread_id: "t1".to_string(),
            run_id: run_id.to_string(),
            phase: phase.to_string(),
            commit_id: commit.map(str::to_string),
            tree_id: Some("tree".to_string()),
            status: "complete".to_string(),
            file_count: 3,
            total_bytes: 100,
            ignored_count: 1,
            omitted_count: 0,
            error_message: None,
        }
    }

    fn changeset_input(run_id: &str, with_file: bool) -> UpsertRunChangesetInput {
        UpsertRunChangesetInput {
            run_id: run_id.to_string(),
            thread_id: "t1".to_string(),
            workspace_id: Some("ws1".to_string()),
            title: format!("Changes {run_id}"),
            summary: None,
            before_snapshot_id: None,
            after_snapshot_id: None,
            files_changed: 1,
            additions: 5,
            deletions: 2,
            binary_files: 0,
            omitted_files: 0,
            completeness: "complete".to_string(),
            confidence: "normal".to_string(),
            error_message: None,
            files: if with_file {
                vec![InsertReviewFileChangeInput {
                    path: Some("src/main.rs".to_string()),
                    change_type: "edit".to_string(),
                    diff: Some("@@".to_string()),
                    additions: 5,
                    deletions: 2,
                    ..Default::default()
                }]
            } else {
                Vec::new()
            },
        }
    }

    #[test]
    fn snapshot_create_upserts_on_conflict_and_reads_back() {
        let (_home, conn) = guarded_conn("rsnap_create");
        seed_graph(&conn, &["r1"]);
        drop(conn);

        let created =
            create_review_snapshot(snapshot_input("r1", "before", Some("c1"))).expect("create");
        assert_eq!(created.commit_id.as_deref(), Some("c1"));
        assert_eq!(created.phase, "before");

        // Same (run, phase) with a new commit replaces the row, not duplicates.
        let replaced =
            create_review_snapshot(snapshot_input("r1", "before", Some("c2"))).expect("replace");
        assert_eq!(replaced.commit_id.as_deref(), Some("c2"));

        let loaded = get_review_snapshot("r1", "before")
            .expect("get")
            .expect("some");
        assert_eq!(loaded.commit_id.as_deref(), Some("c2"));
        assert!(get_review_snapshot("r1", "after").expect("get").is_none());
    }

    #[test]
    fn upsert_run_changeset_replaces_prior_rows_and_files() {
        let _home = fresh_db("rsnap_upsert");
        seed_graph(&connect().expect("connect"), &["r1"]);

        let first = upsert_run_changeset(changeset_input("r1", true)).expect("first upsert");
        assert_eq!(first.status, RUN_SNAPSHOT_STATUS);
        assert_eq!(first.files_changed, 1);

        // A retry replaces the changeset and drops the old file rows.
        let second = upsert_run_changeset(changeset_input("r1", false)).expect("replace");
        assert_eq!(second.title, "Changes r1");
        assert!(get_run_changeset("r1").expect("get").is_some());

        let conn = connect().expect("reconnect");
        let changeset_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM review_changesets WHERE run_id = 'r1'",
                [],
                |row| row.get(0),
            )
            .expect("count changesets");
        let file_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM review_file_changes", [], |row| {
                row.get(0)
            })
            .expect("count files");
        assert_eq!(changeset_count, 1);
        assert_eq!(file_count, 0, "stale file rows were deleted");
    }

    #[test]
    fn last_run_changeset_is_the_latest_ended_run() {
        let (_home, conn) = guarded_conn("rsnap_last");
        seed_graph(&conn, &["r1", "r2"]);
        drop(conn);
        upsert_run_changeset(changeset_input("r1", false)).expect("upsert r1");
        upsert_run_changeset(changeset_input("r2", false)).expect("upsert r2");

        let latest = get_last_run_changeset("t1").expect("last").expect("some");
        assert_eq!(latest.run_id.as_deref(), Some("r2"));
        assert!(get_last_run_changeset("t_ghost").expect("last").is_none());
    }

    #[test]
    fn mark_run_overlapped_covers_windows_and_peers() {
        let _home = fresh_db("rsnap_overlap");
        seed_graph(&connect().expect("connect"), &["r1", "r2"]);

        // No `before` snapshot → nothing to do.
        mark_run_overlapped("ws1", "r1").expect("no snapshots is a no-op");

        // r1's window [100, 200]; r2 starts inside it and never ends (its
        // missing `after` defaults to now) → mutual overlap.
        create_review_snapshot(snapshot_input("r1", "before", None)).expect("before1");
        create_review_snapshot(snapshot_input("r1", "after", None)).expect("after1");
        create_review_snapshot(snapshot_input("r2", "before", None)).expect("before2");
        connect()
            .expect("reconnect")
            .execute_batch(
                "UPDATE review_snapshots SET created_at = 100 WHERE run_id = 'r1' AND phase = 'before';
                 UPDATE review_snapshots SET created_at = 200 WHERE run_id = 'r1' AND phase = 'after';
                 UPDATE review_snapshots SET created_at = 150 WHERE run_id = 'r2' AND phase = 'before';",
            )
            .expect("pin snapshot timestamps");
        upsert_run_changeset(changeset_input("r1", false)).expect("changeset r1");
        upsert_run_changeset(changeset_input("r2", false)).expect("changeset r2");

        mark_run_overlapped("ws1", "r1").expect("mark");

        let r1 = get_run_changeset("r1").expect("get").expect("some");
        let r2 = get_run_changeset("r2").expect("get").expect("some");
        assert!(r1.overlapped, "the target run is marked");
        assert!(r2.overlapped, "the overlapping peer is marked");

        // No peers in a fresh workspace → early return on the empty scan.
        mark_run_overlapped("ws_empty", "r1").expect("no peers");
    }

    #[test]
    fn prune_keeps_the_newest_and_reports_shadow_refs() {
        let _home = fresh_db("rsnap_prune");
        seed_graph(&connect().expect("connect"), &["r1", "r2", "r3"]);
        for run in ["r1", "r2", "r3"] {
            upsert_run_changeset(changeset_input(run, true)).expect("upsert");
            create_review_snapshot(snapshot_input(run, "before", None)).expect("snapshot");
        }
        // A workspace-less changeset is pruned without a shadow-ref report.
        connect()
            .expect("reconnect")
            .execute(
                "UPDATE review_changesets SET workspace_id = NULL WHERE run_id = 'r2'",
                [],
            )
            .expect("null workspace");

        let pruned = prune_thread_changesets("t1", 1).expect("prune");
        assert_eq!(pruned, vec![("ws1".to_string(), "r1".to_string())]);

        assert!(
            get_run_changeset("r3").expect("get").is_some(),
            "newest kept"
        );
        assert!(get_run_changeset("r1").expect("get").is_none());
        assert!(get_run_changeset("r2").expect("get").is_none());
        // Snapshots of pruned runs are gone too (delete_run_review_in cascade).
        assert!(get_review_snapshot("r1", "before").expect("get").is_none());
    }

    #[test]
    fn unmaterialized_and_commit_pinning_queries() {
        let (_home, conn) = guarded_conn("rsnap_queries");
        seed_graph(&conn, &["r1", "r2"]);
        drop(conn);

        create_review_snapshot(snapshot_input("r1", "before", Some("c1"))).expect("snap r1");
        create_review_snapshot(snapshot_input("r2", "before", None)).expect("snap r2");
        upsert_run_changeset(changeset_input("r1", false)).expect("changeset r1");

        // r1 has a changeset; only r2 is unmaterialized.
        let unmaterialized = list_unmaterialized_runs().expect("unmaterialized");
        assert_eq!(
            unmaterialized,
            vec![("r2".to_string(), "t1".to_string(), "ws1".to_string())]
        );

        // Only r1 pins a commit.
        let pinned = list_snapshots_with_commits().expect("pinned");
        assert_eq!(pinned.len(), 1);
        assert_eq!(pinned[0].1, "ws1");
        assert_eq!(pinned[0].2, "c1");

        // Failing the snapshot removes it from both queries.
        let snapshot_id = pinned[0].0.clone();
        mark_snapshot_failed(&snapshot_id, "commit gone").expect("mark failed");
        assert!(list_snapshots_with_commits().expect("pinned").is_empty());
        let unmaterialized = list_unmaterialized_runs().expect("unmaterialized");
        assert_eq!(unmaterialized.len(), 1, "failed snapshots drop out too");
    }
}
