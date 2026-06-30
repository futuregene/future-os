//! Store CRUD for the shadow review pipeline (see gui/ER.md §4.10):
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

/// Column list for `review_changeset_from_row`, in struct order. Reuse this in
/// every `SELECT` that maps into `ReviewChangesetRecord`.
pub(super) const REVIEW_CHANGESET_COLUMNS: &str =
    "id, thread_id, run_id, tool_call_id, title, summary, status, \
     files_changed, additions, deletions, source_kind, workspace_id, \
     before_snapshot_id, after_snapshot_id, binary_files, omitted_files, \
     completeness, confidence, overlapped, error_message, created_at, updated_at";

pub(super) fn review_changeset_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ReviewChangesetRecord> {
    Ok(ReviewChangesetRecord {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        run_id: row.get(2)?,
        tool_call_id: row.get(3)?,
        title: row.get(4)?,
        summary: row.get(5)?,
        status: row.get(6)?,
        files_changed: row.get(7)?,
        additions: row.get(8)?,
        deletions: row.get(9)?,
        source_kind: row.get(10)?,
        workspace_id: row.get(11)?,
        before_snapshot_id: row.get(12)?,
        after_snapshot_id: row.get(13)?,
        binary_files: row.get(14)?,
        omitted_files: row.get(15)?,
        completeness: row.get(16)?,
        confidence: row.get(17)?,
        overlapped: row.get(18)?,
        error_message: row.get(19)?,
        created_at: row.get(20)?,
        updated_at: row.get(21)?,
    })
}

/// Column list for `review_file_change_from_row`, in struct order.
pub(super) const REVIEW_FILE_CHANGE_COLUMNS: &str =
    "id, changeset_id, target_type, target_id, path, change_type, \
     before_ref, after_ref, diff, summary, additions, deletions, \
     previous_path, binary, before_size, after_size, mime, diff_truncated, \
     omission_reason, created_at, updated_at";

pub(super) fn review_file_change_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ReviewFileChangeRecord> {
    Ok(ReviewFileChangeRecord {
        id: row.get(0)?,
        changeset_id: row.get(1)?,
        target_type: row.get(2)?,
        target_id: row.get(3)?,
        path: row.get(4)?,
        change_type: row.get(5)?,
        before_ref: row.get(6)?,
        after_ref: row.get(7)?,
        diff: row.get(8)?,
        summary: row.get(9)?,
        additions: row.get(10)?,
        deletions: row.get(11)?,
        previous_path: row.get(12)?,
        binary: row.get(13)?,
        before_size: row.get(14)?,
        after_size: row.get(15)?,
        mime: row.get(16)?,
        diff_truncated: row.get(17)?,
        omission_reason: row.get(18)?,
        created_at: row.get(19)?,
        updated_at: row.get(20)?,
    })
}

/// Column list for `review_snapshot_from_row`, in struct order.
pub(super) const REVIEW_SNAPSHOT_COLUMNS: &str =
    "id, workspace_id, thread_id, run_id, phase, commit_id, tree_id, status, \
     file_count, total_bytes, ignored_count, omitted_count, error_message, created_at";

pub(super) fn review_snapshot_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ReviewSnapshotRecord> {
    Ok(ReviewSnapshotRecord {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        thread_id: row.get(2)?,
        run_id: row.get(3)?,
        phase: row.get(4)?,
        commit_id: row.get(5)?,
        tree_id: row.get(6)?,
        status: row.get(7)?,
        file_count: row.get(8)?,
        total_bytes: row.get(9)?,
        ignored_count: row.get(10)?,
        omitted_count: row.get(11)?,
        error_message: row.get(12)?,
        created_at: row.get(13)?,
    })
}

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
    let cols = REVIEW_CHANGESET_COLUMNS
        .split(", ")
        .map(|c| format!("c.{}", c.trim()))
        .collect::<Vec<_>>()
        .join(", ");
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
        let rows = stmt
            .query_map(
                params![workspace_id, run_id, after_ts, now, before_ts],
                |row| row.get(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
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
    conn.execute(
        "UPDATE review_changesets
         SET overlapped = 1, updated_at = ?2
         WHERE run_id = ?1 AND source_kind = 'run_snapshot'",
        params![run_id, now],
    )?;
    Ok(())
}

// ── retention / recovery / consistency (Phase 2) ────────────────────────────

/// Delete all review rows for a single Run (file changes, the `run_snapshot`
/// changeset, and snapshots), in FK-safe order.
pub fn delete_run_review(run_id: &str) -> Result<(), crate::AppError> {
    let conn = connect()?;
    conn.execute(
        "DELETE FROM review_file_changes WHERE changeset_id IN (
             SELECT id FROM review_changesets WHERE run_id = ?1 AND source_kind = 'run_snapshot'
         )",
        params![run_id],
    )?;
    conn.execute(
        "DELETE FROM review_changesets WHERE run_id = ?1 AND source_kind = 'run_snapshot'",
        params![run_id],
    )?;
    conn.execute(
        "DELETE FROM review_snapshots WHERE run_id = ?1",
        params![run_id],
    )?;
    Ok(())
}

/// Prune a Thread's `run_snapshot` changesets to the newest `keep`, deleting the
/// older ones' review data. Returns `(workspace_id, run_id)` for each pruned Run
/// so the caller can delete its shadow refs (§12.3).
pub fn prune_thread_changesets(
    thread_id: &str,
    keep: usize,
) -> Result<Vec<(String, String)>, crate::AppError> {
    let ordered: Vec<(String, Option<String>)> = {
        let conn = connect()?;
        let mut stmt = conn.prepare(
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
        delete_run_review(&run_id)?;
        if let Some(workspace_id) = workspace_id {
            pruned.push((workspace_id, run_id));
        }
    }
    Ok(pruned)
}

/// Runs interrupted by a crash: a `before` snapshot exists but there is no
/// `after` snapshot and no changeset. Returns `(run_id, thread_id, workspace_id)`
/// (§6.6).
pub fn list_interrupted_runs() -> Result<Vec<(String, String, String)>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn.prepare(
        "SELECT s.run_id, s.thread_id, s.workspace_id
         FROM review_snapshots s
         WHERE s.phase = 'before' AND s.status != 'failed'
           AND NOT EXISTS (
             SELECT 1 FROM review_snapshots a WHERE a.run_id = s.run_id AND a.phase = 'after'
           )
           AND NOT EXISTS (
             SELECT 1 FROM review_changesets c
             WHERE c.run_id = s.run_id AND c.source_kind = 'run_snapshot'
           )",
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
