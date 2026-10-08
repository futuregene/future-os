//! Durable store for `future-tasks`.
//!
//! Single SQLite database at `<home>/.future/tasks/tasks.db`, WAL +
//! `busy_timeout=5000`. One host (the tick loop) is the writer; other hosts
//! (CLI, remote bridge) open the same file read/write through the same
//! `Store` — WAL serializes them. Schema lives here and is applied
//! idempotently; `meta.schema_version` guards future migrations.

use crate::types::*;
use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

pub const STORE_SCHEMA_VERSION: i64 = 4;
pub const STORE_FILE: &str = "tasks.db";
pub const STORE_DIR: &str = "tasks";

/// The `tasks` column list, once — every task query selects exactly these, in
/// this order, and `task_from_row` reads them positionally.
const TASK_COLUMNS: &str = "
    id, name, enabled, prompt, prompt_version, cwd, model_id, thinking_level,
    session_policy, conversation_mode, thread_id, trigger_kind, trigger_json, dep_join,
    next_due_at, pending_request_at, pending_origin, pending_actor,
    reflection, created_at, updated_at, deleted_at";

const SQL_INSERT_TASK: &str = "
    INSERT INTO tasks (
        id, name, enabled, prompt, prompt_version, cwd, model_id, thinking_level,
        session_policy, conversation_mode, thread_id, trigger_kind, trigger_json, dep_join,
        next_due_at, pending_request_at, pending_origin, pending_actor,
        reflection, created_at, updated_at, deleted_at
    ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22)";

const SQL_UPDATE_TASK: &str = "
    UPDATE tasks SET
        name=?2, enabled=?3, prompt=?4, prompt_version=?5, cwd=?6, model_id=?7,
        thinking_level=?8, session_policy=?9, conversation_mode=?10, thread_id=?11,
        trigger_kind=?12, trigger_json=?13, dep_join=?14, next_due_at=?15,
        pending_request_at=?16, pending_origin=?17, pending_actor=?18,
        reflection=?19, updated_at=?20, deleted_at=?21
    WHERE id=?1";

/// `(enabled = 1)` plus "due now or explicitly requested" — the tick's query.
const SQL_DUE_TASKS: &str = "
    SELECT %COLUMNS% FROM tasks
    WHERE deleted_at IS NULL AND enabled = 1
      AND (pending_request_at IS NOT NULL
           OR (trigger_kind = 'schedule' AND next_due_at IS NOT NULL AND next_due_at <= ?1))
    ORDER BY next_due_at, created_at";

const SQL_ADD_DEP: &str =
    "INSERT OR REPLACE INTO task_deps (task_id, upstream_task_id, on_condition) VALUES (?1, ?2, ?3)";
const SQL_DROP_DEP: &str = "DELETE FROM task_deps WHERE task_id = ?1 AND upstream_task_id = ?2";
const SQL_DROP_DEP_STATE: &str =
    "DELETE FROM task_dep_state WHERE task_id = ?1 AND upstream_task_id = ?2";
/// Edges that point *at* a task, from whatever downstream. Used when a task is
/// deleted: the downstream must not keep waiting on something that is gone.
const SQL_DROP_DEPS_POINTING_AT: &str = "DELETE FROM task_deps WHERE upstream_task_id = ?1";
const SQL_DROP_DEP_STATES_POINTING_AT: &str =
    "DELETE FROM task_dep_state WHERE upstream_task_id = ?1";
const SQL_LIST_DEPS: &str =
    "SELECT task_id, upstream_task_id, on_condition FROM task_deps WHERE task_id = ?1";
const SQL_LIST_ALL_DEPS: &str = "SELECT task_id, upstream_task_id, on_condition FROM task_deps";
const SQL_GET_DEP_STATE: &str = "
    SELECT task_id, upstream_task_id, satisfied_run_id, satisfied_at
    FROM task_dep_state WHERE task_id = ?1 AND upstream_task_id = ?2";
const SQL_LIST_DEP_STATES: &str = "
    SELECT task_id, upstream_task_id, satisfied_run_id, satisfied_at
    FROM task_dep_state WHERE task_id = ?1";
const SQL_MARK_DEP_SATISFIED: &str = "
    INSERT INTO task_dep_state (task_id, upstream_task_id, satisfied_run_id, satisfied_at)
    VALUES (?1, ?2, ?3, ?4)
    ON CONFLICT(task_id, upstream_task_id) DO UPDATE SET
      satisfied_run_id = excluded.satisfied_run_id,
      satisfied_at = excluded.satisfied_at";
const SQL_CLEAR_DEP_STATE: &str = "
    UPDATE task_dep_state SET satisfied_run_id = NULL, satisfied_at = NULL
    WHERE task_id = ?1 AND upstream_task_id = ?2";

/// The `task_runs` column list (see `TASK_COLUMNS`).
const RUN_COLUMNS: &str = "
    id, task_id, kind, origin, actor, due_at, status, thread_id, session_id,
    run_id, prompt_version, result_summary, feedback, feedback_note,
    started_at, finished_at, error_message";

const SQL_INSERT_RUN: &str = "
    INSERT INTO task_runs (
        id, task_id, kind, origin, actor, due_at, status, thread_id, session_id,
        run_id, prompt_version, result_summary, feedback, feedback_note,
        started_at, finished_at, error_message
    ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)";

const SQL_UPDATE_RUN: &str = "
    UPDATE task_runs SET
        kind=?2, origin=?3, actor=?4, due_at=?5, status=?6, thread_id=?7, session_id=?8,
        run_id=?9, prompt_version=?10, result_summary=?11,
        feedback=?12, feedback_note=?13, started_at=?14, finished_at=?15, error_message=?16
    WHERE id=?1";

const SQL_RUNS_FOR_TASK: &str = "
    SELECT %COLUMNS% FROM task_runs WHERE task_id = ?1 ORDER BY started_at DESC LIMIT ?2";
/// Runs that are still in flight *for the task itself*. A reflection pass is a
/// side activity over a finished run, so it must not hold the task's next
/// scheduled slot back (a slow suggestion would silently skip a run).
const SQL_COUNT_RUNNING: &str =
    "SELECT COUNT(*) FROM task_runs WHERE task_id = ?1 AND status = 'running' AND kind != 'reflection'";
const SQL_INTERRUPT_RUNNING: &str = "
    UPDATE task_runs SET status = 'failed', finished_at = ?1, error_message = 'interrupted'
    WHERE status = 'running'";

const SQL_INSERT_REVISION: &str = "
    INSERT INTO task_prompt_revisions (
        id, task_id, version, prompt, source, status, reason, confidence, source_run_id, created_at
    ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)";
const SQL_REVISIONS_FOR_TASK: &str = "
    SELECT id, task_id, version, prompt, source, status, reason, confidence, source_run_id, created_at
    FROM task_prompt_revisions WHERE task_id = ?1 ORDER BY version";
const SQL_NEXT_REVISION_VERSION: &str =
    "SELECT MAX(version) FROM task_prompt_revisions WHERE task_id = ?1";

/// Expand a `%COLUMNS%` template. Kept out of the query strings themselves so
/// the column list has exactly one definition.
fn with_columns(template: &str, columns: &str) -> String {
    template.replace("%COLUMNS%", columns)
}

const PRAGMAS: &str = "PRAGMA busy_timeout = 5000;
             PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;";

const SQL_SET_SCHEMA_VERSION: &str = "
    INSERT INTO meta (key, value) VALUES ('schema_version', ?1)
    ON CONFLICT(key) DO NOTHING";

/// One schema step for a database written by an older build.
enum Migration {
    /// A column that database predates.
    Add(&'static str, &'static str, &'static str),
    /// A column the code stopped reading.
    Drop(&'static str, &'static str),
}

use Migration::{Add, Drop};

/// Applied in order on every open, each guarded by the column's own presence so
/// the list is idempotent and a database settles on the schema in one pass.
/// (`meta.schema_version` records when a database was born; it does not gate
/// these — a database written by any older build is brought forward by content,
/// not by version number.)
const MIGRATIONS: &[Migration] = &[
    Add(
        "tasks",
        "conversation_mode",
        "ALTER TABLE tasks ADD COLUMN conversation_mode TEXT NOT NULL DEFAULT 'workspace'",
    ),
    Drop("task_runs", "source_entry_id"),
    Drop("tasks", "last_run_at"),
];

/// The full schema. Single source of truth for a fresh database (see the
/// module doc: `meta.schema_version` guards later migrations).
const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS tasks (
  id               TEXT PRIMARY KEY,
  name             TEXT NOT NULL,
  enabled          INTEGER NOT NULL DEFAULT 1,
  prompt           TEXT NOT NULL,
  prompt_version   INTEGER NOT NULL DEFAULT 1,
  cwd              TEXT NOT NULL,
  model_id         TEXT,
  thinking_level   TEXT,
  session_policy   TEXT NOT NULL DEFAULT 'new',
  conversation_mode TEXT NOT NULL DEFAULT 'workspace',
  thread_id        TEXT,
  trigger_kind     TEXT NOT NULL,
  trigger_json     TEXT NOT NULL,
  dep_join         TEXT NOT NULL DEFAULT 'all',
  next_due_at      INTEGER,
  pending_request_at INTEGER,
  pending_origin   TEXT,
  pending_actor    TEXT,
  reflection       TEXT NOT NULL DEFAULT 'ask',
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  deleted_at INTEGER
);
CREATE UNIQUE INDEX IF NOT EXISTS tasks_active_name ON tasks(name) WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS task_deps (
  task_id          TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  upstream_task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  on_condition     TEXT NOT NULL,
  PRIMARY KEY (task_id, upstream_task_id)
);

CREATE TABLE IF NOT EXISTS task_dep_state (
  task_id          TEXT NOT NULL,
  upstream_task_id TEXT NOT NULL,
  satisfied_run_id TEXT,
  satisfied_at     INTEGER,
  PRIMARY KEY (task_id, upstream_task_id),
  FOREIGN KEY (task_id) REFERENCES tasks(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS task_runs (
  id             TEXT PRIMARY KEY,
  task_id        TEXT NOT NULL REFERENCES tasks(id),
  kind           TEXT NOT NULL,
  origin         TEXT NOT NULL,
  actor          TEXT,
  due_at         INTEGER,
  status         TEXT NOT NULL,
  thread_id      TEXT,
  session_id     TEXT,
  run_id         TEXT,
  prompt_version INTEGER,
  result_summary TEXT,
  feedback       TEXT,
  feedback_note  TEXT,
  started_at     INTEGER,
  finished_at    INTEGER,
  error_message  TEXT,
  UNIQUE(task_id, due_at)
);

CREATE TABLE IF NOT EXISTS task_prompt_revisions (
  id         TEXT PRIMARY KEY,
  task_id    TEXT NOT NULL REFERENCES tasks(id),
  version    INTEGER NOT NULL,
  prompt     TEXT NOT NULL,
  source     TEXT NOT NULL,
  status     TEXT NOT NULL,
  reason     TEXT,
  confidence REAL,
  source_run_id TEXT,
  created_at INTEGER NOT NULL,
  UNIQUE(task_id, version)
);
"#;

pub fn store_dir(home: &Path) -> PathBuf {
    home.join(STORE_DIR)
}

pub fn store_path(home: &Path) -> PathBuf {
    store_dir(home).join(STORE_FILE)
}

/// Open (creating if needed) the tasks store under `home`.
pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(home: &Path) -> Result<Self> {
        let dir = store_dir(home);
        std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        let path = store_path(home);
        let conn = Connection::open(&path).with_context(|| format!("open {}", path.display()))?;
        conn.execute_batch(PRAGMAS)?;
        let store = Self { conn };
        store.apply_schema()?;
        Ok(store)
    }

    fn apply_schema(&self) -> Result<()> {
        self.conn.execute_batch(SCHEMA_SQL)?;
        for step in MIGRATIONS {
            match step {
                // `CREATE TABLE IF NOT EXISTS` leaves an existing table alone, so
                // a database written before a column needs it added explicitly;
                // a fresh one already has it.
                Migration::Add(table, column, sql) => {
                    if !self.has_column(table, column)? {
                        self.conn.execute_batch(sql)?;
                    }
                }
                // SQLite has no `DROP COLUMN IF EXISTS`. The guard is what makes
                // this re-runnable, and it is why a dropped column never has to
                // be named in a query again — a database that still carries it is
                // repaired on the next open rather than on the next query.
                Migration::Drop(table, column) => {
                    if self.has_column(table, column)? {
                        self.conn
                            .execute_batch(&format!("ALTER TABLE {table} DROP COLUMN {column}"))?;
                    }
                }
            }
        }
        let values = params![STORE_SCHEMA_VERSION.to_string()];
        self.conn.execute(SQL_SET_SCHEMA_VERSION, values)?;
        Ok(())
    }

    fn has_column(&self, table: &str, column: &str) -> Result<bool> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1"
        ))?;
        let mut rows = stmt.query(params![column])?;
        Ok(rows.next()?.is_some())
    }

    // ─── tasks ────────────────────────────────────────────────────────────

    pub fn insert_task(&self, task: &Task) -> Result<()> {
        let values = params![
            task.id,
            task.name,
            task.enabled as i64,
            task.prompt,
            task.prompt_version,
            task.cwd,
            task.model_id,
            task.thinking_level,
            session_policy_str(task.session_policy),
            conversation_mode_str(task.conversation_mode),
            task.thread_id,
            trigger_kind_str(task.trigger_kind),
            serde_json::to_string(&task.trigger_json)?,
            dep_join_str(task.dep_join),
            task.next_due_at,
            task.pending_request_at,
            task.pending_origin.map(run_origin_str),
            task.pending_actor,
            reflection_str(task.reflection),
            task.created_at,
            task.updated_at,
            task.deleted_at,
        ];
        self.conn.execute(SQL_INSERT_TASK, values)?;
        Ok(())
    }

    pub fn update_task(&self, task: &Task) -> Result<()> {
        let values = params![
            task.id,
            task.name,
            task.enabled as i64,
            task.prompt,
            task.prompt_version,
            task.cwd,
            task.model_id,
            task.thinking_level,
            session_policy_str(task.session_policy),
            conversation_mode_str(task.conversation_mode),
            task.thread_id,
            trigger_kind_str(task.trigger_kind),
            serde_json::to_string(&task.trigger_json)?,
            dep_join_str(task.dep_join),
            task.next_due_at,
            task.pending_request_at,
            task.pending_origin.map(run_origin_str),
            task.pending_actor,
            reflection_str(task.reflection),
            task.updated_at,
            task.deleted_at,
        ];
        self.conn.execute(SQL_UPDATE_TASK, values)?;
        Ok(())
    }

    pub fn get_task(&self, id: &str) -> Result<Option<Task>> {
        let sql = format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?1");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query_map(params![id], task_from_row)?;
        Ok(rows.next().transpose()?)
    }

    pub fn find_task_by_name(&self, name: &str) -> Result<Option<Task>> {
        let sql =
            format!("SELECT {TASK_COLUMNS} FROM tasks WHERE name = ?1 AND deleted_at IS NULL");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query_map(params![name], task_from_row)?;
        Ok(rows.next().transpose()?)
    }

    pub fn list_tasks(&self, include_deleted: bool) -> Result<Vec<Task>> {
        let filter = if include_deleted {
            ""
        } else {
            "WHERE deleted_at IS NULL"
        };
        let sql = format!("SELECT {TASK_COLUMNS} FROM tasks {filter} ORDER BY created_at");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], task_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn list_due_tasks(&self, now_ms: i64) -> Result<Vec<Task>> {
        let sql = with_columns(SQL_DUE_TASKS, TASK_COLUMNS);
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![now_ms], task_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    // ─── deps ─────────────────────────────────────────────────────────────

    pub fn add_dep(&self, dep: &TaskDep) -> Result<()> {
        let values = params![dep.task_id, dep.upstream_task_id, dep_on_str(dep.on)];
        self.conn.execute(SQL_ADD_DEP, values)?;
        Ok(())
    }

    pub fn remove_dep(&self, task_id: &str, upstream_task_id: &str) -> Result<()> {
        let values = params![task_id, upstream_task_id];
        self.conn.execute(SQL_DROP_DEP, values)?;
        self.conn.execute(SQL_DROP_DEP_STATE, values)?;
        Ok(())
    }

    /// Drop every edge that points at `upstream_task_id`, whichever task holds
    /// it. Deleting a task has to do this: an edge is a promise that the
    /// upstream will report a run, and a deleted task never will — the
    /// downstream would wait forever on a name that resolves to nothing.
    pub fn remove_deps_pointing_at(&self, upstream_task_id: &str) -> Result<()> {
        let values = params![upstream_task_id];
        self.conn.execute(SQL_DROP_DEPS_POINTING_AT, values)?;
        self.conn.execute(SQL_DROP_DEP_STATES_POINTING_AT, values)?;
        Ok(())
    }

    pub fn list_deps(&self, task_id: &str) -> Result<Vec<TaskDep>> {
        let mut stmt = self.conn.prepare(SQL_LIST_DEPS)?;
        let rows = stmt.query_map(params![task_id], dep_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn list_all_deps(&self) -> Result<Vec<TaskDep>> {
        let mut stmt = self.conn.prepare(SQL_LIST_ALL_DEPS)?;
        let rows = stmt.query_map([], dep_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn get_dep_state(
        &self,
        task_id: &str,
        upstream_task_id: &str,
    ) -> Result<Option<TaskDepState>> {
        let mut stmt = self.conn.prepare(SQL_GET_DEP_STATE)?;
        let mut rows = stmt.query_map(params![task_id, upstream_task_id], dep_state_from_row)?;
        Ok(rows.next().transpose()?)
    }

    pub fn list_dep_states(&self, task_id: &str) -> Result<Vec<TaskDepState>> {
        let mut stmt = self.conn.prepare(SQL_LIST_DEP_STATES)?;
        let rows = stmt.query_map(params![task_id], dep_state_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn mark_dep_satisfied(
        &self,
        task_id: &str,
        upstream_task_id: &str,
        run_id: &str,
        at_ms: i64,
    ) -> Result<()> {
        let values = params![task_id, upstream_task_id, run_id, at_ms];
        self.conn.execute(SQL_MARK_DEP_SATISFIED, values)?;
        Ok(())
    }

    pub fn clear_dep_states(&self, task_id: &str, pairs: &[(String, String)]) -> Result<()> {
        for (upstream, _run_id) in pairs {
            let values = params![task_id, upstream];
            self.conn.execute(SQL_CLEAR_DEP_STATE, values)?;
        }
        Ok(())
    }

    // ─── runs ─────────────────────────────────────────────────────────────

    pub fn insert_run(&self, run: &TaskRun) -> Result<()> {
        let values = params![
            run.id,
            run.task_id,
            run_kind_str(run.kind),
            run_origin_str(run.origin),
            run.actor,
            run.due_at,
            run_status_str(run.status),
            run.thread_id,
            run.session_id,
            run.run_id,
            run.prompt_version,
            run.result_summary,
            run.feedback,
            run.feedback_note,
            run.started_at,
            run.finished_at,
            run.error_message,
        ];
        self.conn.execute(SQL_INSERT_RUN, values)?;
        Ok(())
    }

    pub fn update_run(&self, run: &TaskRun) -> Result<()> {
        let values = params![
            run.id,
            run_kind_str(run.kind),
            run_origin_str(run.origin),
            run.actor,
            run.due_at,
            run_status_str(run.status),
            run.thread_id,
            run.session_id,
            run.run_id,
            run.prompt_version,
            run.result_summary,
            run.feedback,
            run.feedback_note,
            run.started_at,
            run.finished_at,
            run.error_message,
        ];
        self.conn.execute(SQL_UPDATE_RUN, values)?;
        Ok(())
    }

    pub fn get_run(&self, id: &str) -> Result<Option<TaskRun>> {
        let sql = format!("SELECT {RUN_COLUMNS} FROM task_runs WHERE id = ?1");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query_map(params![id], run_from_row)?;
        Ok(rows.next().transpose()?)
    }

    pub fn list_runs_for_task(&self, task_id: &str, limit: i64) -> Result<Vec<TaskRun>> {
        let sql = with_columns(SQL_RUNS_FOR_TASK, RUN_COLUMNS);
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![task_id, limit], run_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn latest_run_for_task(&self, task_id: &str) -> Result<Option<TaskRun>> {
        Ok(self.list_runs_for_task(task_id, 1)?.into_iter().next())
    }

    pub fn has_running_run(&self, task_id: &str) -> Result<bool> {
        let n: i64 = self
            .conn
            .query_row(SQL_COUNT_RUNNING, params![task_id], |row| row.get(0))?;
        Ok(n > 0)
    }

    pub fn mark_interrupted_runs(&self, at_ms: i64) -> Result<usize> {
        let values = params![at_ms];
        let n = self.conn.execute(SQL_INTERRUPT_RUNNING, values)?;
        Ok(n)
    }

    // ─── prompt revisions ─────────────────────────────────────────────────

    pub fn insert_revision(&self, rev: &PromptRevision) -> Result<()> {
        let values = params![
            rev.id,
            rev.task_id,
            rev.version,
            rev.prompt,
            rev.source,
            rev.status,
            rev.reason,
            rev.confidence,
            rev.source_run_id,
            rev.created_at,
        ];
        self.conn.execute(SQL_INSERT_REVISION, values)?;
        Ok(())
    }

    pub fn list_revisions(&self, task_id: &str) -> Result<Vec<PromptRevision>> {
        let mut stmt = self.conn.prepare(SQL_REVISIONS_FOR_TASK)?;
        let rows = stmt.query_map(params![task_id], revision_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Move one revision through its lifecycle (`proposed` → `applied`,
    /// `proposed` → `superseded`). Rows are otherwise immutable: a version that
    /// was in force is history and must not be rewritten.
    pub fn set_revision_status(
        &self,
        task_id: &str,
        revision_id: &str,
        status: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE task_prompt_revisions SET status = ?3 WHERE task_id = ?1 AND id = ?2",
            params![task_id, revision_id, status],
        )?;
        Ok(())
    }

    pub fn next_revision_version(&self, task_id: &str) -> Result<i64> {
        let v: Option<i64> =
            self.conn
                .query_row(SQL_NEXT_REVISION_VERSION, params![task_id], |row| {
                    row.get(0)
                })?;
        Ok(v.unwrap_or(0) + 1)
    }
}

// ─── row mapping ────────────────────────────────────────────────────────────

fn dep_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskDep> {
    Ok(TaskDep {
        task_id: row.get(0)?,
        upstream_task_id: row.get(1)?,
        on: parse_dep_on(&row.get::<_, String>(2)?).unwrap_or(DepOn::Success),
    })
}

fn dep_state_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskDepState> {
    Ok(TaskDepState {
        task_id: row.get(0)?,
        upstream_task_id: row.get(1)?,
        satisfied_run_id: row.get(2)?,
        satisfied_at: row.get(3)?,
    })
}

fn revision_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PromptRevision> {
    Ok(PromptRevision {
        id: row.get(0)?,
        task_id: row.get(1)?,
        version: row.get(2)?,
        prompt: row.get(3)?,
        source: row.get(4)?,
        status: row.get(5)?,
        reason: row.get(6)?,
        confidence: row.get(7)?,
        source_run_id: row.get(8)?,
        created_at: row.get(9)?,
    })
}

fn task_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    Ok(Task {
        id: row.get(0)?,
        name: row.get(1)?,
        enabled: row.get::<_, i64>(2)? != 0,
        prompt: row.get(3)?,
        prompt_version: row.get(4)?,
        cwd: row.get(5)?,
        model_id: row.get(6)?,
        thinking_level: row.get(7)?,
        session_policy: parse_session_policy(&row.get::<_, String>(8)?)
            .unwrap_or(SessionPolicy::New),
        conversation_mode: parse_conversation_mode(&row.get::<_, String>(9)?)
            .unwrap_or(ConversationMode::Workspace),
        thread_id: row.get(10)?,
        trigger_kind: parse_trigger_kind(&row.get::<_, String>(11)?).unwrap_or(TriggerKind::Manual),
        trigger_json: serde_json::from_str(&row.get::<_, String>(12)?)
            .unwrap_or(serde_json::Value::Null),
        dep_join: parse_dep_join(&row.get::<_, String>(13)?).unwrap_or(DepJoin::All),
        next_due_at: row.get(14)?,
        pending_request_at: row.get(15)?,
        pending_origin: row
            .get::<_, Option<String>>(16)?
            .and_then(|s| parse_run_origin(&s)),
        pending_actor: row.get(17)?,
        reflection: parse_reflection(&row.get::<_, String>(18)?).unwrap_or(Reflection::Ask),
        created_at: row.get(19)?,
        updated_at: row.get(20)?,
        deleted_at: row.get(21)?,
    })
}

fn run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRun> {
    Ok(TaskRun {
        id: row.get(0)?,
        task_id: row.get(1)?,
        kind: parse_run_kind(&row.get::<_, String>(2)?).unwrap_or(RunKind::Main),
        origin: parse_run_origin(&row.get::<_, String>(3)?).unwrap_or(RunOrigin::Schedule),
        actor: row.get(4)?,
        due_at: row.get(5)?,
        status: parse_run_status(&row.get::<_, String>(6)?).unwrap_or(RunStatus::Failed),
        thread_id: row.get(7)?,
        session_id: row.get(8)?,
        run_id: row.get(9)?,
        prompt_version: row.get(10)?,
        result_summary: row.get(11)?,
        feedback: row.get(12)?,
        feedback_note: row.get(13)?,
        started_at: row.get(14)?,
        finished_at: row.get(15)?,
        error_message: row.get(16)?,
    })
}

// ─── enum <-> str ───────────────────────────────────────────────────────────

fn trigger_kind_str(k: TriggerKind) -> &'static str {
    match k {
        TriggerKind::Manual => "manual",
        TriggerKind::Schedule => "schedule",
    }
}
fn parse_trigger_kind(s: &str) -> Option<TriggerKind> {
    match s {
        "manual" => Some(TriggerKind::Manual),
        "schedule" => Some(TriggerKind::Schedule),
        _ => None,
    }
}

fn dep_join_str(j: DepJoin) -> &'static str {
    match j {
        DepJoin::All => "all",
        DepJoin::Any => "any",
    }
}
fn parse_dep_join(s: &str) -> Option<DepJoin> {
    match s {
        "all" => Some(DepJoin::All),
        "any" => Some(DepJoin::Any),
        _ => None,
    }
}

fn dep_on_str(o: DepOn) -> &'static str {
    match o {
        DepOn::Success => "success",
        DepOn::Failure => "failure",
        DepOn::Completed => "completed",
    }
}
fn parse_dep_on(s: &str) -> Option<DepOn> {
    match s {
        "success" => Some(DepOn::Success),
        "failure" => Some(DepOn::Failure),
        "completed" => Some(DepOn::Completed),
        _ => None,
    }
}

fn conversation_mode_str(mode: ConversationMode) -> &'static str {
    match mode {
        ConversationMode::Chat => "chat",
        ConversationMode::Workspace => "workspace",
    }
}
fn parse_conversation_mode(s: &str) -> Option<ConversationMode> {
    match s {
        "chat" => Some(ConversationMode::Chat),
        "workspace" => Some(ConversationMode::Workspace),
        _ => None,
    }
}

fn session_policy_str(p: SessionPolicy) -> &'static str {
    match p {
        SessionPolicy::New => "new",
        SessionPolicy::Existing => "existing",
    }
}
fn parse_session_policy(s: &str) -> Option<SessionPolicy> {
    match s {
        "new" => Some(SessionPolicy::New),
        "existing" => Some(SessionPolicy::Existing),
        _ => None,
    }
}

fn reflection_str(r: Reflection) -> &'static str {
    match r {
        Reflection::Off => "off",
        Reflection::Ask => "ask",
        Reflection::Auto => "auto",
    }
}
fn parse_reflection(s: &str) -> Option<Reflection> {
    match s {
        "off" => Some(Reflection::Off),
        "ask" => Some(Reflection::Ask),
        "auto" => Some(Reflection::Auto),
        _ => None,
    }
}

fn run_kind_str(k: RunKind) -> &'static str {
    match k {
        RunKind::Main => "main",
        RunKind::Manual => "manual",
        RunKind::Chain => "chain",
        RunKind::Reflection => "reflection",
    }
}
fn parse_run_kind(s: &str) -> Option<RunKind> {
    match s {
        "main" => Some(RunKind::Main),
        "manual" => Some(RunKind::Manual),
        "chain" => Some(RunKind::Chain),
        "reflection" => Some(RunKind::Reflection),
        _ => None,
    }
}

fn run_origin_str(o: RunOrigin) -> &'static str {
    match o {
        RunOrigin::Schedule => "schedule",
        RunOrigin::Ui => "ui",
        RunOrigin::Cli => "cli",
        RunOrigin::Chain => "chain",
        RunOrigin::Reflection => "reflection",
    }
}
fn parse_run_origin(s: &str) -> Option<RunOrigin> {
    match s {
        "schedule" => Some(RunOrigin::Schedule),
        "ui" => Some(RunOrigin::Ui),
        "cli" => Some(RunOrigin::Cli),
        "chain" => Some(RunOrigin::Chain),
        "reflection" => Some(RunOrigin::Reflection),
        _ => None,
    }
}

fn run_status_str(s: RunStatus) -> &'static str {
    match s {
        RunStatus::Running => "running",
        RunStatus::Completed => "completed",
        RunStatus::Failed => "failed",
        RunStatus::Skipped => "skipped",
    }
}
fn parse_run_status(s: &str) -> Option<RunStatus> {
    match s {
        "running" => Some(RunStatus::Running),
        "completed" => Some(RunStatus::Completed),
        "failed" => Some(RunStatus::Failed),
        "skipped" => Some(RunStatus::Skipped),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn store() -> (TempDir, Store) {
        let dir = TempDir::new().unwrap();
        let s = Store::open(dir.path()).unwrap();
        (dir, s)
    }

    fn task(name: &str) -> Task {
        Task {
            id: new_task_id(),
            name: name.into(),
            enabled: true,
            prompt: "p".into(),
            prompt_version: 1,
            cwd: "/tmp".into(),
            model_id: None,
            thinking_level: None,
            session_policy: SessionPolicy::New,
            conversation_mode: ConversationMode::Workspace,
            thread_id: None,
            trigger_kind: TriggerKind::Schedule,
            trigger_json: serde_json::json!({"mode":"daily","time":"09:00"}),
            dep_join: DepJoin::All,
            next_due_at: Some(1_000),
            pending_request_at: None,
            pending_origin: None,
            pending_actor: None,
            reflection: Reflection::Ask,
            created_at: 1,
            updated_at: 1,
            deleted_at: None,
        }
    }

    #[test]
    fn insert_get_round_trip() {
        let (_dir, s) = store();
        let t = task("a");
        s.insert_task(&t).unwrap();
        let got = s.get_task(&t.id).unwrap().unwrap();
        assert_eq!(got.name, "a");
        assert_eq!(got.trigger_kind, TriggerKind::Schedule);
        assert_eq!(got.dep_join, DepJoin::All);
        assert_eq!(got.reflection, Reflection::Ask);
    }

    #[test]
    fn active_name_is_unique() {
        let (_dir, s) = store();
        s.insert_task(&task("dup")).unwrap();
        let err = s.insert_task(&task("dup")).unwrap_err();
        assert!(err.to_string().contains("UNIQUE") || err.to_string().contains("unique"));
        // Soft-delete frees the name.
        let mut t = s.find_task_by_name("dup").unwrap().unwrap();
        t.deleted_at = Some(1);
        s.update_task(&t).unwrap();
        s.insert_task(&task("dup")).unwrap();
    }

    #[test]
    fn due_tasks_picks_pending_and_scheduled() {
        let (_dir, s) = store();
        let mut t1 = task("due");
        t1.next_due_at = Some(500);
        s.insert_task(&t1).unwrap();
        let mut t2 = task("pending");
        t2.next_due_at = None;
        t2.pending_request_at = Some(400);
        s.insert_task(&t2).unwrap();
        let mut t3 = task("future");
        t3.next_due_at = Some(10_000);
        s.insert_task(&t3).unwrap();
        let due = s.list_due_tasks(1_000).unwrap();
        let names: Vec<_> = due.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"due"));
        assert!(names.contains(&"pending"));
        assert!(!names.contains(&"future"));
    }

    #[test]
    fn dep_state_mark_and_clear() {
        let (_dir, s) = store();
        let a = task("a");
        let b = task("b");
        s.insert_task(&a).unwrap();
        s.insert_task(&b).unwrap();
        s.add_dep(&TaskDep {
            task_id: b.id.clone(),
            upstream_task_id: a.id.clone(),
            on: DepOn::Success,
        })
        .unwrap();
        s.mark_dep_satisfied(&b.id, &a.id, "trn_1", 1_000).unwrap();
        let st = s.get_dep_state(&b.id, &a.id).unwrap().unwrap();
        assert_eq!(st.satisfied_run_id.as_deref(), Some("trn_1"));
        s.clear_dep_states(&b.id, &[(a.id.clone(), "trn_1".to_string())])
            .unwrap();
        let st = s.get_dep_state(&b.id, &a.id).unwrap().unwrap();
        assert!(st.satisfied_run_id.is_none());
    }

    #[test]
    fn run_insert_update_and_interrupted() {
        let (_dir, s) = store();
        let t = task("r");
        s.insert_task(&t).unwrap();
        let run = TaskRun {
            id: new_run_id(),
            task_id: t.id.clone(),
            kind: RunKind::Main,
            origin: RunOrigin::Schedule,
            actor: None,
            due_at: Some(1_000),
            status: RunStatus::Running,
            thread_id: None,
            session_id: None,
            run_id: None,
            prompt_version: Some(1),
            result_summary: None,
            feedback: None,
            feedback_note: None,
            started_at: Some(1_000),
            finished_at: None,
            error_message: None,
        };
        s.insert_run(&run).unwrap();
        assert!(s.has_running_run(&t.id).unwrap());
        let n = s.mark_interrupted_runs(2_000).unwrap();
        assert_eq!(n, 1);
        let got = s.get_run(&run.id).unwrap().unwrap();
        assert_eq!(got.status, RunStatus::Failed);
        assert_eq!(got.error_message.as_deref(), Some("interrupted"));
    }

    /// A reflection pass is a side activity over a finished run: while one is
    /// in flight the task's own work must not be held back (a slow suggestion
    /// would silently skip a scheduled slot).
    #[test]
    fn a_running_reflection_does_not_hold_the_task_back() {
        let (_dir, s) = store();
        let t = task("suggesting");
        s.insert_task(&t).unwrap();
        s.insert_run(&TaskRun {
            id: new_run_id(),
            task_id: t.id.clone(),
            kind: RunKind::Reflection,
            origin: RunOrigin::Reflection,
            actor: None,
            due_at: None,
            status: RunStatus::Running,
            thread_id: None,
            session_id: None,
            run_id: None,
            prompt_version: Some(1),
            result_summary: None,
            feedback: None,
            feedback_note: None,
            started_at: Some(1),
            finished_at: None,
            error_message: None,
        })
        .unwrap();
        assert!(!s.has_running_run(&t.id).unwrap());
    }

    /// A suggestion moves through its own lifecycle, and nothing else rewrites
    /// a row: a version that was in force is history.
    #[test]
    fn a_suggestion_can_be_marked_applied() {
        let (_dir, s) = store();
        let t = task("suggested");
        s.insert_task(&t).unwrap();
        let row = PromptRevision {
            id: new_revision_id(),
            task_id: t.id.clone(),
            version: PROPOSAL_VERSION,
            prompt: "a better prompt".into(),
            source: REVISION_SOURCE_REFLECTION.into(),
            status: REVISION_STATUS_PROPOSED.into(),
            reason: Some("clearer".into()),
            confidence: Some(0.8),
            source_run_id: Some("trn_1".into()),
            created_at: 5,
        };
        s.insert_revision(&row).unwrap();
        assert_eq!(s.list_revisions(&t.id).unwrap()[0].prompt, "a better prompt");
        s.set_revision_status(&t.id, &row.id, REVISION_STATUS_APPLIED)
            .unwrap();
        let stored = s.list_revisions(&t.id).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].status, REVISION_STATUS_APPLIED);
        assert_eq!(
            stored[0].prompt, "a better prompt",
            "only the status moves"
        );
        assert_eq!(stored[0].confidence, Some(0.8));
        // A suggestion is outside the version sequence, so the task's own next
        // version is unaffected by it.
        assert_eq!(s.next_revision_version(&t.id).unwrap(), 1);
    }

    #[test]
    fn revisions_are_ordered() {        let (_dir, s) = store();
        let t = task("rev");
        s.insert_task(&t).unwrap();
        for v in 1..=3 {
            s.insert_revision(&PromptRevision {
                id: new_revision_id(),
                task_id: t.id.clone(),
                version: v,
                prompt: format!("p{v}"),
                source: "user".into(),
                status: if v == 3 { "active" } else { "superseded" }.into(),
                reason: None,
                confidence: None,
                source_run_id: None,
                created_at: v,
            })
            .unwrap();
        }
        let revs = s.list_revisions(&t.id).unwrap();
        assert_eq!(revs.len(), 3);
        assert_eq!(revs[2].version, 3);
        assert_eq!(s.next_revision_version(&t.id).unwrap(), 4);
    }

    // ─── paths and opening ────────────────────────────────────────────────

    #[test]
    fn the_store_lives_under_the_home_it_was_opened_with() {
        let dir = TempDir::new().unwrap();
        assert_eq!(store_dir(dir.path()), dir.path().join("tasks"));
        assert_eq!(
            store_path(dir.path()),
            dir.path().join("tasks").join("tasks.db")
        );
        // Opening creates the directory (a fresh home has no `.future` yet), is
        // idempotent, and records the schema version once.
        let first = Store::open(dir.path()).unwrap();
        assert!(store_path(dir.path()).exists());
        drop(first);
        let second = Store::open(dir.path()).unwrap();
        let version: String = second
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(version, STORE_SCHEMA_VERSION.to_string());
        let rows: i64 = second
            .conn
            .query_row("SELECT COUNT(*) FROM meta", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1, "re-opening must not duplicate the meta row");
    }

    /// A database written before a column was dropped carries it until the next
    /// open, and the repair must not disturb the rows around it.
    #[test]
    fn a_dropped_column_is_removed_from_a_database_that_still_carries_it() {
        let dir = TempDir::new().unwrap();
        let t = task("legacy");
        {
            let s = Store::open(dir.path()).unwrap();
            s.insert_task(&t).unwrap();
            s.insert_run(&run_row(&t.id, RunStatus::Completed, 1))
                .unwrap();
            // An older build's `task_runs` had this column.
            s.conn
                .execute_batch("ALTER TABLE task_runs ADD COLUMN source_entry_id TEXT")
                .unwrap();
            assert!(s.has_column("task_runs", "source_entry_id").unwrap());
        }

        let reopened = Store::open(dir.path()).unwrap();
        assert!(
            !reopened.has_column("task_runs", "source_entry_id").unwrap(),
            "opening must bring the schema forward, not just tolerate the column"
        );
        // The rows are untouched, and reads work through the new column list.
        let runs = reopened.list_runs_for_task(&t.id, 10).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, RunStatus::Completed);
        assert_eq!(reopened.get_task(&t.id).unwrap().unwrap().name, "legacy");
    }

    /// The other half of the migration list: a database written before a column
    /// existed gets it added, and the guard makes a second open a no-op.
    #[test]
    fn a_missing_column_is_added_to_a_database_that_predates_it() {
        let dir = TempDir::new().unwrap();
        {
            let s = Store::open(dir.path()).unwrap();
            s.insert_task(&task("old")).unwrap();
            // Mimic a database from before `conversation_mode` was introduced.
            s.conn
                .execute_batch("ALTER TABLE tasks DROP COLUMN conversation_mode")
                .unwrap();
            assert!(!s.has_column("tasks", "conversation_mode").unwrap());
        }

        let reopened = Store::open(dir.path()).unwrap();
        assert!(
            reopened.has_column("tasks", "conversation_mode").unwrap(),
            "opening adds the column rather than failing on the missing one"
        );
        // Rows written against the old schema read back on the default.
        let t = reopened.find_task_by_name("old").unwrap().unwrap();
        assert_eq!(t.conversation_mode, ConversationMode::Workspace);

        // Re-running the list is idempotent: the column is there, so the step
        // does nothing and the open still succeeds.
        drop(reopened);
        let third = Store::open(dir.path()).unwrap();
        assert_eq!(third.find_task_by_name("old").unwrap().unwrap().name, "old");
    }

    /// Deleting a task has to take the edges pointing at it: an edge is a
    /// promise the upstream will report again, and a deleted one never will.
    #[test]
    fn edges_pointing_at_a_task_are_dropped_together() {
        let (_dir, s) = store();
        let upstream = task("upstream");
        let a = task("a");
        let b = task("b");
        for t in [&upstream, &a, &b] {
            s.insert_task(t).unwrap();
        }
        for downstream in [&a, &b] {
            s.add_dep(&TaskDep {
                task_id: downstream.id.clone(),
                upstream_task_id: upstream.id.clone(),
                on: DepOn::Success,
            })
            .unwrap();
            s.mark_dep_satisfied(&downstream.id, &upstream.id, "trn_1", 1)
                .unwrap();
        }
        // An unrelated edge survives.
        s.add_dep(&TaskDep {
            task_id: b.id.clone(),
            upstream_task_id: a.id.clone(),
            on: DepOn::Success,
        })
        .unwrap();

        s.remove_deps_pointing_at(&upstream.id).unwrap();

        assert!(s.list_deps(&a.id).unwrap().is_empty());
        assert!(s.get_dep_state(&a.id, &upstream.id).unwrap().is_none());
        assert!(s.get_dep_state(&b.id, &upstream.id).unwrap().is_none());
        assert_eq!(
            s.list_deps(&b.id).unwrap().len(),
            1,
            "an edge that does not point at the removed task is left alone"
        );
        assert_eq!(s.list_all_deps().unwrap().len(), 1);

        // Idempotent: nothing left to remove is not an error.
        s.remove_deps_pointing_at(&upstream.id).unwrap();
    }

    /// Both conversation modes round-trip, and an unparseable value falls back
    /// to the behaviour every task had before the setting existed.
    #[test]
    fn conversation_mode_round_trips_and_defaults_to_workspace() {
        let (_dir, s) = store();
        for mode in [ConversationMode::Chat, ConversationMode::Workspace] {
            let mut t = task("m");
            t.name = format!("{mode:?}");
            t.conversation_mode = mode;
            s.insert_task(&t).unwrap();
            let read = s.find_task_by_name(&t.name).unwrap().unwrap();
            assert_eq!(read.conversation_mode, mode);
        }

        // A row written by something that did not know the column: blank it out
        // and read it back.
        let mut t = task("blank");
        s.insert_task(&t).unwrap();
        s.conn
            .execute(
                "UPDATE tasks SET conversation_mode = 'nonsense' WHERE id = ?1",
                params![t.id],
            )
            .unwrap();
        let read = s.find_task_by_name("blank").unwrap().unwrap();
        assert_eq!(read.conversation_mode, ConversationMode::Workspace);

        // A `Task` serialized by a build that did not have the field deserializes
        // onto the mode every task had before the setting existed.
        t.conversation_mode = ConversationMode::Chat;
        assert_eq!(
            serde_json::to_string(&t.conversation_mode).unwrap(),
            "\"chat\""
        );
        let mut json = serde_json::to_value(&t).unwrap();
        json.as_object_mut().unwrap().remove("conversation_mode");
        let restored: Task = serde_json::from_value(json).unwrap();
        assert_eq!(restored.conversation_mode, ConversationMode::Workspace);
    }

    // ─── tasks ────────────────────────────────────────────────────────────

    #[test]
    fn every_optional_task_field_round_trips() {
        let (_dir, s) = store();
        let mut t = task("full");
        t.enabled = false;
        t.model_id = Some("future/model".into());
        t.thinking_level = Some("high".into());
        t.session_policy = SessionPolicy::Existing;
        t.thread_id = Some("thr_1".into());
        t.trigger_kind = TriggerKind::Manual;
        t.trigger_json = serde_json::json!({"mode":"interval","every_minutes":5,"anchor":7});
        t.dep_join = DepJoin::Any;
        t.pending_request_at = Some(42);
        t.pending_origin = Some(RunOrigin::Chain);
        t.pending_actor = Some("task:tsk_up".into());
        t.reflection = Reflection::Auto;
        t.deleted_at = Some(9);
        s.insert_task(&t).unwrap();

        let got = s.get_task(&t.id).unwrap().unwrap();
        assert_eq!(got, t, "insert/get must be lossless");
        // A write through `update_task` reaches every column.
        let mut edited = got.clone();
        edited.enabled = true;
        edited.session_policy = SessionPolicy::New;
        edited.pending_origin = Some(RunOrigin::Ui);
        edited.reflection = Reflection::Off;
        edited.deleted_at = None;
        s.update_task(&edited).unwrap();
        assert_eq!(s.get_task(&t.id).unwrap().unwrap(), edited);
    }

    #[test]
    fn find_by_name_skips_soft_deleted_tasks() {
        let (_dir, s) = store();
        let mut t = task("gone");
        t.deleted_at = Some(1);
        s.insert_task(&t).unwrap();
        assert!(s.find_task_by_name("gone").unwrap().is_none());
        assert!(s.find_task_by_name("never-existed").unwrap().is_none());
        assert!(s.get_task("never-existed").unwrap().is_none());
        assert!(s.get_run("never-existed").unwrap().is_none());
    }

    #[test]
    fn list_tasks_honours_the_include_deleted_flag() {
        let (_dir, s) = store();
        s.insert_task(&task("live")).unwrap();
        let mut dead = task("dead");
        dead.deleted_at = Some(1);
        s.insert_task(&dead).unwrap();

        let live = s.list_tasks(false).unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].name, "live");
        let all = s.list_tasks(true).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn due_tasks_exclude_disabled_deleted_and_future_rows() {
        let (_dir, s) = store();
        let mut disabled = task("disabled");
        disabled.enabled = false;
        disabled.next_due_at = Some(1);
        s.insert_task(&disabled).unwrap();

        let mut deleted = task("deleted");
        deleted.deleted_at = Some(1);
        deleted.next_due_at = Some(1);
        s.insert_task(&deleted).unwrap();

        let mut manual_due_by_time = task("manual-future");
        manual_due_by_time.trigger_kind = TriggerKind::Manual;
        manual_due_by_time.next_due_at = Some(1);
        s.insert_task(&manual_due_by_time).unwrap();

        let mut scheduled = task("scheduled");
        scheduled.next_due_at = Some(500);
        s.insert_task(&scheduled).unwrap();

        let due = s.list_due_tasks(1_000).unwrap();
        let names: Vec<&str> = due.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["scheduled"],
            "only an enabled, live schedule fires"
        );

        // A manual task is still claimable through its pending request.
        let mut manual = s.find_task_by_name("manual-future").unwrap().unwrap();
        manual.pending_request_at = Some(600);
        s.update_task(&manual).unwrap();
        let due = s.list_due_tasks(1_000).unwrap();
        let names: Vec<&str> = due.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"manual-future"), "{names:?}");
    }

    // ─── dependencies ─────────────────────────────────────────────────────

    #[test]
    fn dependency_edges_and_their_progress_are_stored_per_task() {
        let (_dir, s) = store();
        let a = task("a");
        let b = task("b");
        let c = task("c");
        for t in [&a, &b, &c] {
            s.insert_task(t).unwrap();
        }
        s.add_dep(&TaskDep {
            task_id: c.id.clone(),
            upstream_task_id: a.id.clone(),
            on: DepOn::Success,
        })
        .unwrap();
        s.add_dep(&TaskDep {
            task_id: c.id.clone(),
            upstream_task_id: b.id.clone(),
            on: DepOn::Completed,
        })
        .unwrap();

        let deps = s.list_deps(&c.id).unwrap();
        assert_eq!(deps.len(), 2);
        assert!(deps
            .iter()
            .any(|d| d.upstream_task_id == a.id && d.on == DepOn::Success));
        assert!(deps
            .iter()
            .any(|d| d.upstream_task_id == b.id && d.on == DepOn::Completed));
        assert!(s.list_deps(&a.id).unwrap().is_empty());
        assert_eq!(s.list_all_deps().unwrap().len(), 2);

        // Progress is per (downstream, upstream) pair and only the latest mark
        // survives.
        assert!(s.list_dep_states(&c.id).unwrap().is_empty());
        assert!(s.get_dep_state(&c.id, &a.id).unwrap().is_none());
        s.mark_dep_satisfied(&c.id, &a.id, "trn_1", 10).unwrap();
        s.mark_dep_satisfied(&c.id, &a.id, "trn_2", 20).unwrap();
        let states = s.list_dep_states(&c.id).unwrap();
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].satisfied_run_id.as_deref(), Some("trn_2"));
        assert_eq!(states[0].satisfied_at, Some(20));

        s.clear_dep_states(&c.id, &[(a.id.clone(), "trn_2".into())])
            .unwrap();
        let st = s.get_dep_state(&c.id, &a.id).unwrap().unwrap();
        assert!(st.satisfied_run_id.is_none());

        // Removing an edge drops its progress row too.
        s.mark_dep_satisfied(&c.id, &b.id, "trn_3", 30).unwrap();
        s.remove_dep(&c.id, &a.id).unwrap();
        assert_eq!(s.list_deps(&c.id).unwrap().len(), 1);
        assert!(s.get_dep_state(&c.id, &a.id).unwrap().is_none());
        assert_eq!(s.list_dep_states(&c.id).unwrap().len(), 1);
    }

    // ─── runs ─────────────────────────────────────────────────────────────

    fn run_row(task_id: &str, status: RunStatus, started: i64) -> TaskRun {
        TaskRun {
            id: new_run_id(),
            task_id: task_id.into(),
            kind: RunKind::Main,
            origin: RunOrigin::Schedule,
            actor: None,
            due_at: Some(started),
            status,
            thread_id: None,
            session_id: None,
            run_id: None,
            prompt_version: Some(1),
            result_summary: None,
            feedback: None,
            feedback_note: None,
            started_at: Some(started),
            finished_at: None,
            error_message: None,
        }
    }

    #[test]
    fn runs_are_listed_newest_first_and_updates_are_lossless() {
        let (_dir, s) = store();
        let t = task("runs");
        s.insert_task(&t).unwrap();
        let older = run_row(&t.id, RunStatus::Completed, 100);
        let newer = run_row(&t.id, RunStatus::Running, 200);
        s.insert_run(&older).unwrap();
        s.insert_run(&newer).unwrap();

        let runs = s.list_runs_for_task(&t.id, 10).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].id, newer.id, "newest first");
        assert_eq!(s.latest_run_for_task(&t.id).unwrap().unwrap().id, newer.id);
        assert!(s.has_running_run(&t.id).unwrap());
        assert!(s.list_runs_for_task("other", 10).unwrap().is_empty());
        assert!(s.latest_run_for_task("other").unwrap().is_none());

        // Every mutable field survives a full update.
        let mut done = newer.clone();
        done.kind = RunKind::Chain;
        done.origin = RunOrigin::Chain;
        done.actor = Some("task:tsk_up".into());
        done.due_at = None;
        done.status = RunStatus::Failed;
        done.thread_id = Some("thr_1".into());
        done.session_id = Some("sess_1".into());
        done.run_id = Some("run_1".into());
        done.prompt_version = Some(3);
        done.result_summary = Some("summary".into());
        done.feedback = Some("bad".into());
        done.feedback_note = Some("too terse".into());
        done.finished_at = Some(300);
        done.error_message = Some("boom".into());
        s.update_run(&done).unwrap();
        assert_eq!(s.get_run(&newer.id).unwrap().unwrap(), done);
        assert!(!s.has_running_run(&t.id).unwrap());
    }

    #[test]
    fn interrupted_runs_are_settled_as_failed() {
        let (_dir, s) = store();
        let t = task("crash");
        s.insert_task(&t).unwrap();
        let live = run_row(&t.id, RunStatus::Running, 100);
        let settled = run_row(&t.id, RunStatus::Completed, 50);
        s.insert_run(&live).unwrap();
        s.insert_run(&settled).unwrap();

        assert_eq!(s.mark_interrupted_runs(999).unwrap(), 1);
        let got = s.get_run(&live.id).unwrap().unwrap();
        assert_eq!(got.status, RunStatus::Failed);
        assert_eq!(got.error_message.as_deref(), Some("interrupted"));
        assert_eq!(got.finished_at, Some(999));
        // A settled run is untouched, and a second pass finds nothing.
        assert_eq!(
            s.get_run(&settled.id).unwrap().unwrap().status,
            RunStatus::Completed
        );
        assert_eq!(s.mark_interrupted_runs(1_000).unwrap(), 0);
    }

    // ─── enum decoding ────────────────────────────────────────────────────

    /// Rows written by a newer binary (or by hand) must decode to the safest
    /// default rather than failing the whole read.
    #[test]
    fn unknown_enum_strings_decode_to_defaults() {
        let dir = TempDir::new().unwrap();
        let s = Store::open(dir.path()).unwrap();
        let t = task("legacy");
        s.insert_task(&t).unwrap();
        let raw = Connection::open(store_path(dir.path())).unwrap();
        raw.execute(
            "UPDATE tasks SET session_policy='bogus', trigger_kind='bogus', trigger_json='not json',
                              dep_join='bogus', reflection='bogus', pending_origin='bogus'
             WHERE id = ?1",
            params![t.id],
        )
        .unwrap();
        let got = s.get_task(&t.id).unwrap().unwrap();
        assert_eq!(got.session_policy, SessionPolicy::New);
        assert_eq!(got.trigger_kind, TriggerKind::Manual);
        assert_eq!(got.trigger_json, serde_json::Value::Null);
        assert_eq!(got.dep_join, DepJoin::All);
        assert_eq!(got.reflection, Reflection::Ask);
        assert!(got.pending_origin.is_none());

        // A real upstream row: the schema enforces the foreign key.
        let upstream = task("upstream");
        s.insert_task(&upstream).unwrap();
        raw.execute(
            "INSERT INTO task_deps (task_id, upstream_task_id, on_condition) VALUES (?1, ?2, 'bogus')",
            params![t.id, upstream.id],
        )
        .unwrap();
        assert_eq!(s.list_deps(&t.id).unwrap()[0].on, DepOn::Success);
        assert_eq!(s.list_all_deps().unwrap()[0].on, DepOn::Success);

        s.insert_run(&run_row(&t.id, RunStatus::Running, 1))
            .unwrap();
        raw.execute(
            "UPDATE task_runs SET kind='bogus', origin='bogus', status='bogus'",
            [],
        )
        .unwrap();
        let run = s.list_runs_for_task(&t.id, 1).unwrap().remove(0);
        assert_eq!(run.kind, RunKind::Main);
        assert_eq!(run.origin, RunOrigin::Schedule);
        assert_eq!(
            run.status,
            RunStatus::Failed,
            "an unreadable status is not running"
        );
        assert_eq!(s.get_run(&run.id).unwrap().unwrap().kind, RunKind::Main);
    }

    /// Every enum value must round-trip through its stored spelling: these are
    /// the strings the desktop, the CLI and the phone all read back.
    #[test]
    fn every_enum_value_round_trips_through_its_stored_spelling() {
        let (_dir, s) = store();
        for (policy, join, reflection) in [
            (SessionPolicy::New, DepJoin::All, Reflection::Off),
            (SessionPolicy::Existing, DepJoin::Any, Reflection::Ask),
            (SessionPolicy::Existing, DepJoin::All, Reflection::Auto),
        ] {
            let mut t = task(&format!("{policy:?}-{join:?}-{reflection:?}"));
            t.session_policy = policy;
            t.dep_join = join;
            t.reflection = reflection;
            t.trigger_kind = TriggerKind::Manual;
            t.pending_origin = Some(RunOrigin::Reflection);
            s.insert_task(&t).unwrap();
            let got = s.get_task(&t.id).unwrap().unwrap();
            assert_eq!(got.session_policy, policy);
            assert_eq!(got.dep_join, join);
            assert_eq!(got.reflection, reflection);
            assert_eq!(got.trigger_kind, TriggerKind::Manual);
            assert_eq!(got.pending_origin, Some(RunOrigin::Reflection));
        }

        for on in [DepOn::Success, DepOn::Failure, DepOn::Completed] {
            let mut upstream = task(&format!("up-{on:?}"));
            upstream.trigger_kind = TriggerKind::Manual;
            let downstream = task(&format!("down-{on:?}"));
            s.insert_task(&upstream).unwrap();
            s.insert_task(&downstream).unwrap();
            s.add_dep(&TaskDep {
                task_id: downstream.id.clone(),
                upstream_task_id: upstream.id.clone(),
                on,
            })
            .unwrap();
            assert_eq!(s.list_deps(&downstream.id).unwrap()[0].on, on);
        }

        for kind in [
            RunKind::Main,
            RunKind::Manual,
            RunKind::Chain,
            RunKind::Reflection,
        ] {
            for origin in [
                RunOrigin::Schedule,
                RunOrigin::Ui,
                RunOrigin::Cli,
                RunOrigin::Chain,
                RunOrigin::Reflection,
            ] {
                for status in [
                    RunStatus::Running,
                    RunStatus::Completed,
                    RunStatus::Failed,
                    RunStatus::Skipped,
                ] {
                    let t = task(&format!("runs-{kind:?}-{origin:?}-{status:?}"));
                    s.insert_task(&t).unwrap();
                    let mut run = run_row(&t.id, status, 1);
                    run.kind = kind;
                    run.origin = origin;
                    s.insert_run(&run).unwrap();
                    let got = s.get_run(&run.id).unwrap().unwrap();
                    assert_eq!(got.kind, kind);
                    assert_eq!(got.origin, origin);
                    assert_eq!(got.status, status);
                }
            }
        }
    }
}
