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

pub const STORE_SCHEMA_VERSION: i64 = 1;
pub const STORE_FILE: &str = "tasks.db";
pub const STORE_DIR: &str = "tasks";

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
        conn.execute_batch(
            "PRAGMA busy_timeout = 5000;
             PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;",
        )?;
        let store = Self { conn };
        store.apply_schema()?;
        Ok(store)
    }

    fn apply_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
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
  thread_id        TEXT,
  trigger_kind     TEXT NOT NULL,
  trigger_json     TEXT NOT NULL,
  dep_join         TEXT NOT NULL DEFAULT 'all',
  next_due_at      INTEGER,
  last_run_at      INTEGER,
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
"#,
        )?;
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)
             ON CONFLICT(key) DO NOTHING",
            params![STORE_SCHEMA_VERSION.to_string()],
        )?;
        Ok(())
    }

    // ─── tasks ────────────────────────────────────────────────────────────

    pub fn insert_task(&self, task: &Task) -> Result<()> {
        self.conn.execute(
            "INSERT INTO tasks (
                id, name, enabled, prompt, prompt_version, cwd, model_id, thinking_level,
                session_policy, thread_id, trigger_kind, trigger_json, dep_join,
                next_due_at, last_run_at, pending_request_at, pending_origin, pending_actor,
                reflection, created_at, updated_at, deleted_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22)",
            params![
                task.id,
                task.name,
                task.enabled as i64,
                task.prompt,
                task.prompt_version,
                task.cwd,
                task.model_id,
                task.thinking_level,
                session_policy_str(task.session_policy),
                task.thread_id,
                trigger_kind_str(task.trigger_kind),
                serde_json::to_string(&task.trigger_json)?,
                dep_join_str(task.dep_join),
                task.next_due_at,
                task.last_run_at,
                task.pending_request_at,
                task.pending_origin.map(run_origin_str),
                task.pending_actor,
                reflection_str(task.reflection),
                task.created_at,
                task.updated_at,
                task.deleted_at,
            ],
        )?;
        Ok(())
    }

    pub fn update_task(&self, task: &Task) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET
                name=?2, enabled=?3, prompt=?4, prompt_version=?5, cwd=?6, model_id=?7,
                thinking_level=?8, session_policy=?9, thread_id=?10, trigger_kind=?11,
                trigger_json=?12, dep_join=?13, next_due_at=?14, last_run_at=?15,
                pending_request_at=?16, pending_origin=?17, pending_actor=?18,
                reflection=?19, updated_at=?20, deleted_at=?21
             WHERE id=?1",
            params![
                task.id,
                task.name,
                task.enabled as i64,
                task.prompt,
                task.prompt_version,
                task.cwd,
                task.model_id,
                task.thinking_level,
                session_policy_str(task.session_policy),
                task.thread_id,
                trigger_kind_str(task.trigger_kind),
                serde_json::to_string(&task.trigger_json)?,
                dep_join_str(task.dep_join),
                task.next_due_at,
                task.last_run_at,
                task.pending_request_at,
                task.pending_origin.map(run_origin_str),
                task.pending_actor,
                reflection_str(task.reflection),
                task.updated_at,
                task.deleted_at,
            ],
        )?;
        Ok(())
    }

    pub fn get_task(&self, id: &str) -> Result<Option<Task>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, enabled, prompt, prompt_version, cwd, model_id, thinking_level,
                    session_policy, thread_id, trigger_kind, trigger_json, dep_join,
                    next_due_at, last_run_at, pending_request_at, pending_origin, pending_actor,
                    reflection, created_at, updated_at, deleted_at
             FROM tasks WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], task_from_row)?;
        Ok(rows.next().transpose()?)
    }

    pub fn find_task_by_name(&self, name: &str) -> Result<Option<Task>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, enabled, prompt, prompt_version, cwd, model_id, thinking_level,
                    session_policy, thread_id, trigger_kind, trigger_json, dep_join,
                    next_due_at, last_run_at, pending_request_at, pending_origin, pending_actor,
                    reflection, created_at, updated_at, deleted_at
             FROM tasks WHERE name = ?1 AND deleted_at IS NULL",
        )?;
        let mut rows = stmt.query_map(params![name], task_from_row)?;
        Ok(rows.next().transpose()?)
    }

    pub fn list_tasks(&self, include_deleted: bool) -> Result<Vec<Task>> {
        let sql = if include_deleted {
            "SELECT id, name, enabled, prompt, prompt_version, cwd, model_id, thinking_level,
                    session_policy, thread_id, trigger_kind, trigger_json, dep_join,
                    next_due_at, last_run_at, pending_request_at, pending_origin, pending_actor,
                    reflection, created_at, updated_at, deleted_at
             FROM tasks ORDER BY created_at"
        } else {
            "SELECT id, name, enabled, prompt, prompt_version, cwd, model_id, thinking_level,
                    session_policy, thread_id, trigger_kind, trigger_json, dep_join,
                    next_due_at, last_run_at, pending_request_at, pending_origin, pending_actor,
                    reflection, created_at, updated_at, deleted_at
             FROM tasks WHERE deleted_at IS NULL ORDER BY created_at"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map([], task_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn list_due_tasks(&self, now_ms: i64) -> Result<Vec<Task>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, enabled, prompt, prompt_version, cwd, model_id, thinking_level,
                    session_policy, thread_id, trigger_kind, trigger_json, dep_join,
                    next_due_at, last_run_at, pending_request_at, pending_origin, pending_actor,
                    reflection, created_at, updated_at, deleted_at
             FROM tasks
             WHERE deleted_at IS NULL AND enabled = 1
               AND (pending_request_at IS NOT NULL
                    OR (trigger_kind = 'schedule' AND next_due_at IS NOT NULL AND next_due_at <= ?1))
             ORDER BY next_due_at, created_at",
        )?;
        let rows = stmt.query_map(params![now_ms], task_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    // ─── deps ─────────────────────────────────────────────────────────────

    pub fn add_dep(&self, dep: &TaskDep) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO task_deps (task_id, upstream_task_id, on_condition) VALUES (?1, ?2, ?3)",
            params![dep.task_id, dep.upstream_task_id, dep_on_str(dep.on)],
        )?;
        Ok(())
    }

    pub fn remove_dep(&self, task_id: &str, upstream_task_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM task_deps WHERE task_id = ?1 AND upstream_task_id = ?2",
            params![task_id, upstream_task_id],
        )?;
        self.conn.execute(
            "DELETE FROM task_dep_state WHERE task_id = ?1 AND upstream_task_id = ?2",
            params![task_id, upstream_task_id],
        )?;
        Ok(())
    }

    pub fn list_deps(&self, task_id: &str) -> Result<Vec<TaskDep>> {
        let mut stmt = self.conn.prepare(
            "SELECT task_id, upstream_task_id, on_condition FROM task_deps WHERE task_id = ?1",
        )?;
        let rows = stmt.query_map(params![task_id], |row| {
            Ok(TaskDep {
                task_id: row.get(0)?,
                upstream_task_id: row.get(1)?,
                on: parse_dep_on(&row.get::<_, String>(2)?).unwrap_or(DepOn::Success),
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn list_all_deps(&self) -> Result<Vec<TaskDep>> {
        let mut stmt = self
            .conn
            .prepare("SELECT task_id, upstream_task_id, on_condition FROM task_deps")?;
        let rows = stmt.query_map([], |row| {
            Ok(TaskDep {
                task_id: row.get(0)?,
                upstream_task_id: row.get(1)?,
                on: parse_dep_on(&row.get::<_, String>(2)?).unwrap_or(DepOn::Success),
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn get_dep_state(
        &self,
        task_id: &str,
        upstream_task_id: &str,
    ) -> Result<Option<TaskDepState>> {
        let mut stmt = self.conn.prepare(
            "SELECT task_id, upstream_task_id, satisfied_run_id, satisfied_at
             FROM task_dep_state WHERE task_id = ?1 AND upstream_task_id = ?2",
        )?;
        let mut rows = stmt.query_map(params![task_id, upstream_task_id], |row| {
            Ok(TaskDepState {
                task_id: row.get(0)?,
                upstream_task_id: row.get(1)?,
                satisfied_run_id: row.get(2)?,
                satisfied_at: row.get(3)?,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    pub fn list_dep_states(&self, task_id: &str) -> Result<Vec<TaskDepState>> {
        let mut stmt = self.conn.prepare(
            "SELECT task_id, upstream_task_id, satisfied_run_id, satisfied_at
             FROM task_dep_state WHERE task_id = ?1",
        )?;
        let rows = stmt.query_map(params![task_id], |row| {
            Ok(TaskDepState {
                task_id: row.get(0)?,
                upstream_task_id: row.get(1)?,
                satisfied_run_id: row.get(2)?,
                satisfied_at: row.get(3)?,
            })
        })?;
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
        self.conn.execute(
            "INSERT INTO task_dep_state (task_id, upstream_task_id, satisfied_run_id, satisfied_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(task_id, upstream_task_id) DO UPDATE SET
               satisfied_run_id = excluded.satisfied_run_id,
               satisfied_at = excluded.satisfied_at",
            params![task_id, upstream_task_id, run_id, at_ms],
        )?;
        Ok(())
    }

    pub fn clear_dep_states(&self, task_id: &str, pairs: &[(String, String)]) -> Result<()> {
        for (upstream, _run_id) in pairs {
            self.conn.execute(
                "UPDATE task_dep_state SET satisfied_run_id = NULL, satisfied_at = NULL
                 WHERE task_id = ?1 AND upstream_task_id = ?2",
                params![task_id, upstream],
            )?;
        }
        Ok(())
    }

    // ─── runs ─────────────────────────────────────────────────────────────

    pub fn insert_run(&self, run: &TaskRun) -> Result<()> {
        self.conn.execute(
            "INSERT INTO task_runs (
                id, task_id, kind, origin, actor, due_at, status, thread_id, session_id,
                run_id, prompt_version, result_summary, feedback, feedback_note,
                started_at, finished_at, error_message
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
            params![
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
            ],
        )?;
        Ok(())
    }

    pub fn update_run(&self, run: &TaskRun) -> Result<()> {
        self.conn.execute(
            "UPDATE task_runs SET
                kind=?2, origin=?3, actor=?4, due_at=?5, status=?6, thread_id=?7, session_id=?8,
                run_id=?9, prompt_version=?10, result_summary=?11, feedback=?12, feedback_note=?13,
                started_at=?14, finished_at=?15, error_message=?16
             WHERE id=?1",
            params![
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
            ],
        )?;
        Ok(())
    }

    pub fn get_run(&self, id: &str) -> Result<Option<TaskRun>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, task_id, kind, origin, actor, due_at, status, thread_id, session_id,
                    run_id, prompt_version, result_summary, feedback, feedback_note,
                    started_at, finished_at, error_message
             FROM task_runs WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], run_from_row)?;
        Ok(rows.next().transpose()?)
    }

    pub fn list_runs_for_task(&self, task_id: &str, limit: i64) -> Result<Vec<TaskRun>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, task_id, kind, origin, actor, due_at, status, thread_id, session_id,
                    run_id, prompt_version, result_summary, feedback, feedback_note,
                    started_at, finished_at, error_message
             FROM task_runs WHERE task_id = ?1 ORDER BY started_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![task_id, limit], run_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn latest_run_for_task(&self, task_id: &str) -> Result<Option<TaskRun>> {
        Ok(self.list_runs_for_task(task_id, 1)?.into_iter().next())
    }

    pub fn has_running_run(&self, task_id: &str) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM task_runs WHERE task_id = ?1 AND status = 'running'",
            params![task_id],
            |row| row.get(0),
        )?;
        Ok(n > 0)
    }

    pub fn mark_interrupted_runs(&self, at_ms: i64) -> Result<usize> {
        let n = self.conn.execute(
            "UPDATE task_runs SET status = 'failed', finished_at = ?1, error_message = 'interrupted'
             WHERE status = 'running'",
            params![at_ms],
        )?;
        Ok(n)
    }

    // ─── prompt revisions ─────────────────────────────────────────────────

    pub fn insert_revision(&self, rev: &PromptRevision) -> Result<()> {
        self.conn.execute(
            "INSERT INTO task_prompt_revisions (
                id, task_id, version, prompt, source, status, reason, confidence, source_run_id, created_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
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
            ],
        )?;
        Ok(())
    }

    pub fn list_revisions(&self, task_id: &str) -> Result<Vec<PromptRevision>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, task_id, version, prompt, source, status, reason, confidence, source_run_id, created_at
             FROM task_prompt_revisions WHERE task_id = ?1 ORDER BY version",
        )?;
        let rows = stmt.query_map(params![task_id], |row| {
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
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn next_revision_version(&self, task_id: &str) -> Result<i64> {
        let v: Option<i64> = self.conn.query_row(
            "SELECT MAX(version) FROM task_prompt_revisions WHERE task_id = ?1",
            params![task_id],
            |row| row.get(0),
        )?;
        Ok(v.unwrap_or(0) + 1)
    }
}

// ─── row mapping ────────────────────────────────────────────────────────────

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
        thread_id: row.get(9)?,
        trigger_kind: parse_trigger_kind(&row.get::<_, String>(10)?).unwrap_or(TriggerKind::Manual),
        trigger_json: serde_json::from_str(&row.get::<_, String>(11)?)
            .unwrap_or(serde_json::Value::Null),
        dep_join: parse_dep_join(&row.get::<_, String>(12)?).unwrap_or(DepJoin::All),
        next_due_at: row.get(13)?,
        last_run_at: row.get(14)?,
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
            thread_id: None,
            trigger_kind: TriggerKind::Schedule,
            trigger_json: serde_json::json!({"mode":"daily","time":"09:00"}),
            dep_join: DepJoin::All,
            next_due_at: Some(1_000),
            last_run_at: None,
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

    #[test]
    fn revisions_are_ordered() {
        let (_dir, s) = store();
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
}
