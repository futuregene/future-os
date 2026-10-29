use crate::{
    now_ms, Conclusion, Consultation, ConsultationInput, Discussion, Event, Parent, Step,
    MAX_CALLS_PER_PARENT_RUN,
};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn open(home: &Path) -> Result<Self> {
        let store = Self {
            root: home.join("fusion"),
        };
        std::fs::create_dir_all(&store.root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&store.root, std::fs::Permissions::from_mode(0o700))?;
        }
        let db = store.connect()?;
        let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(
            version <= 1,
            "Unsupported Fusion database version {version}"
        );
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
            BEGIN IMMEDIATE;
            CREATE TABLE IF NOT EXISTS fusion_meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS fusion_runs(id TEXT PRIMARY KEY,parent_session TEXT NOT NULL,parent_run TEXT NOT NULL,tool_call TEXT NOT NULL,status TEXT NOT NULL,owner TEXT,data TEXT NOT NULL CHECK(json_valid(data)),UNIQUE(parent_session,parent_run,tool_call));
            CREATE INDEX IF NOT EXISTS fusion_queue ON fusion_runs(status);
            CREATE TABLE IF NOT EXISTS fusion_steps(id TEXT PRIMARY KEY,run_id TEXT NOT NULL REFERENCES fusion_runs(id),data TEXT NOT NULL CHECK(json_valid(data)));
            CREATE INDEX IF NOT EXISTS fusion_step_run ON fusion_steps(run_id);
            CREATE TABLE IF NOT EXISTS fusion_events(seq INTEGER PRIMARY KEY AUTOINCREMENT,run_id TEXT NOT NULL REFERENCES fusion_runs(id),kind TEXT NOT NULL,data TEXT NOT NULL CHECK(json_valid(data)),timestamp INTEGER NOT NULL);
            CREATE INDEX IF NOT EXISTS fusion_event_run ON fusion_events(run_id,seq);
            CREATE TABLE IF NOT EXISTS fusion_inputs(run_id TEXT PRIMARY KEY REFERENCES fusion_runs(id),data TEXT NOT NULL CHECK(json_valid(data)));
            CREATE TABLE IF NOT EXISTS fusion_participants(run_id TEXT NOT NULL REFERENCES fusion_runs(id),participant TEXT NOT NULL,model TEXT NOT NULL,PRIMARY KEY(run_id,participant));
            CREATE TABLE IF NOT EXISTS fusion_messages(step_id TEXT PRIMARY KEY REFERENCES fusion_steps(id),run_id TEXT NOT NULL REFERENCES fusion_runs(id),session_id TEXT NOT NULL,agent_run_id TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS fusion_message_session ON fusion_messages(session_id);
            CREATE TABLE IF NOT EXISTS fusion_issues(run_id TEXT NOT NULL REFERENCES fusion_runs(id),id TEXT NOT NULL,data TEXT NOT NULL CHECK(json_valid(data)),PRIMARY KEY(run_id,id));
            CREATE TABLE IF NOT EXISTS fusion_positions(run_id TEXT NOT NULL REFERENCES fusion_runs(id),issue_id TEXT NOT NULL,participant TEXT NOT NULL,data TEXT NOT NULL CHECK(json_valid(data)),PRIMARY KEY(run_id,issue_id,participant));
            INSERT OR IGNORE INTO fusion_meta VALUES('layout','fusion_v1');
            PRAGMA user_version=1;
            COMMIT;")?;
        let layout: String = db.query_row(
            "SELECT value FROM fusion_meta WHERE key='layout'",
            [],
            |r| r.get(0),
        )?;
        ensure!(layout == "fusion_v1", "Unrecognized Fusion database layout");
        Ok(store)
    }

    pub fn existing(home: &Path) -> Result<Self> {
        let store = Self {
            root: home.join("fusion"),
        };
        ensure!(
            store.root.join("fusion.db").is_file(),
            "Fusion is unavailable: start the FutureOS host"
        );
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    fn connect(&self) -> Result<Connection> {
        let db = Connection::open(self.root.join("fusion.db"))?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.pragma_update(None, "foreign_keys", "ON")?;
        Ok(db)
    }

    pub fn heartbeat(&self, owner: &str) -> Result<()> {
        self.connect()?.execute("INSERT INTO fusion_meta(key,value) VALUES('host',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [json!({"owner":owner,"at":now_ms()}).to_string()])?;
        Ok(())
    }

    pub fn host_available(&self) -> bool {
        (|| -> Result<bool> {
            let raw: Option<String> = self
                .connect()?
                .query_row("SELECT value FROM fusion_meta WHERE key='host'", [], |r| {
                    r.get(0)
                })
                .optional()?;
            let value: Value = serde_json::from_str(raw.as_deref().unwrap_or("null"))?;
            let age = now_ms().saturating_sub(value["at"].as_i64().unwrap_or(0));
            Ok((0..5000).contains(&age))
        })()
        .unwrap_or(false)
    }

    pub fn get(&self, id: &str) -> Result<Consultation> {
        let raw: String = self
            .connect()?
            .query_row("SELECT data FROM fusion_runs WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .context("Fusion run not found")?;
        Ok(serde_json::from_str(&raw)?)
    }

    pub fn for_tool(&self, session: &str, run: &str, tool: &str) -> Result<Option<Consultation>> {
        let raw: Option<String> = self.connect()?.query_row("SELECT data FROM fusion_runs WHERE parent_session=?1 AND parent_run=?2 AND tool_call=?3", params![session,run,tool], |r|r.get(0)).optional()?;
        raw.map(|s| serde_json::from_str(&s).map_err(Into::into))
            .transpose()
    }

    /// Latest consultation within this parent session, used by terminal history.
    pub fn latest_for_session(&self, session: &str) -> Result<Option<Consultation>> {
        let raw: Option<String> = self
            .connect()?
            .query_row(
                "SELECT data FROM fusion_runs WHERE parent_session=?1 ORDER BY rowid DESC LIMIT 1",
                [session],
                |r| r.get(0),
            )
            .optional()?;
        raw.map(|s| serde_json::from_str(&s).map_err(Into::into))
            .transpose()
    }

    pub fn prepare(
        &self,
        parent: Parent,
        input: ConsultationInput,
        workspace: &Path,
    ) -> Result<(Consultation, bool)> {
        input.validate()?;
        let mut db = self.connect()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = parent.consultation_id();
        let existing: Option<String> = tx
            .query_row("SELECT data FROM fusion_runs WHERE id=?1", [&id], |r| {
                r.get(0)
            })
            .optional()?;
        if let Some(raw) = existing {
            let old: Consultation = serde_json::from_str(&raw)?;
            ensure!(
                serde_json::to_value(&old.input)? == serde_json::to_value(&input)?
                    && old.parent == parent,
                "Fusion invocation key reused with a different input"
            );
            return Ok((old, false));
        }
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM fusion_runs WHERE parent_session=?1 AND parent_run=?2",
            params![parent.session_id, parent.run_id],
            |r| r.get(0),
        )?;
        ensure!(count < MAX_CALLS_PER_PARENT_RUN as i64, "Fusion consultation limit reached; continue independently or reuse the existing conclusion");
        let root = self.root.join("snapshots").join(&id);
        let run = Consultation {
            id: id.clone(),
            parent,
            input,
            workspace: workspace
                .to_str()
                .context("Fusion requires a Unicode workspace path")?
                .into(),
            snapshot_root: root
                .to_str()
                .context("Fusion requires a Unicode snapshot path")?
                .into(),
            status: "preparing".into(),
            phase: "preparing".into(),
            completeness: "full".into(),
            result: None,
            error: None,
            created_at: now_ms(),
            updated_at: now_ms(),
        };
        tx.execute("INSERT INTO fusion_runs(id,parent_session,parent_run,tool_call,status,data) VALUES(?1,?2,?3,?4,?5,?6)",params![run.id,run.parent.session_id,run.parent.run_id,run.parent.tool_call_id,run.status,serde_json::to_string(&run)?])?;
        tx.execute(
            "INSERT INTO fusion_inputs VALUES(?1,?2)",
            params![run.id, serde_json::to_string(&run.input)?],
        )?;
        append(
            &tx,
            &run.id,
            "created",
            json!({"question":run.input.question,"models":crate::PANEL_MODELS,"moderator":crate::MODERATOR_MODEL}),
        )?;
        tx.commit()?;
        Ok((run, true))
    }

    fn change(
        &self,
        id: &str,
        owner: Option<&str>,
        kind: &str,
        f: impl FnOnce(&mut Consultation),
    ) -> Result<bool> {
        let mut db = self.connect()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (raw, recorded_owner): (String, Option<String>) = tx.query_row(
            "SELECT data,owner FROM fusion_runs WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let mut run: Consultation = serde_json::from_str(&raw)?;
        if run.terminal() || owner.is_some_and(|o| recorded_owner.as_deref() != Some(o)) {
            return Ok(false);
        }
        f(&mut run);
        run.updated_at = now_ms();
        tx.execute(
            "UPDATE fusion_runs SET status=?2,data=?3 WHERE id=?1",
            params![id, run.status, serde_json::to_string(&run)?],
        )?;
        append(
            &tx,
            id,
            kind,
            json!({"status":run.status,"phase":run.phase,"error":run.error}),
        )?;
        tx.commit()?;
        Ok(true)
    }

    pub fn ready(&self, id: &str) -> Result<()> {
        self.change(id, None, "queued", |r| r.status = "queued".into())?;
        Ok(())
    }
    pub fn phase(&self, id: &str, owner: &str, phase: &str) -> Result<bool> {
        self.change(id, Some(owner), "phase", |r| r.phase = phase.into())
    }
    pub fn fail(&self, id: &str, owner: Option<&str>, error: &str) -> Result<()> {
        self.change(id, owner, "failed", |r| {
            r.status = "failed".into();
            r.error = Some(error.into());
        })?;
        Ok(())
    }
    pub fn cancel(&self, id: &str) -> Result<()> {
        self.change(id, None, "cancelled", |r| r.status = "cancelled".into())?;
        Ok(())
    }
    pub fn finish(
        &self,
        id: &str,
        owner: &str,
        mut result: Conclusion,
        completeness: &str,
    ) -> Result<()> {
        result.fusion_run_id = id.into();
        result.completeness = completeness.into();
        self.change(id, Some(owner), "result_ready", |r| {
            r.status = "delivering".into();
            r.completeness = completeness.into();
            r.result = Some(result);
        })?;
        Ok(())
    }
    pub fn delivered(&self, id: &str) -> Result<()> {
        self.change(id, None, "delivered", |r| r.status = "completed".into())?;
        Ok(())
    }

    pub fn claim(&self, owner: &str) -> Result<Option<Consultation>> {
        let mut db = self.connect()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let raw: Option<String> = tx
            .query_row(
                "SELECT data FROM fusion_runs WHERE status='queued' ORDER BY rowid LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let Some(raw) = raw else { return Ok(None) };
        let mut run: Consultation = serde_json::from_str(&raw)?;
        run.status = "running".into();
        run.updated_at = now_ms();
        tx.execute(
            "UPDATE fusion_runs SET status='running',owner=?2,data=?3 WHERE id=?1",
            params![run.id, owner, serde_json::to_string(&run)?],
        )?;
        append(&tx, &run.id, "claimed", json!({"owner":owner}))?;
        tx.commit()?;
        Ok(Some(run))
    }

    pub fn reconcile(&self) -> Result<()> {
        let db = self.connect()?;
        let ids=db.prepare("SELECT id FROM fusion_runs WHERE status IN ('running','preparing','queued','delivering')")?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for id in ids {
            self.change(&id,None,"interrupted",|r|{r.status="interrupted".into();r.error=Some("Host restarted; original consultation was interrupted. Do not silently repeat it.".into());})?;
        }
        Ok(())
    }

    pub fn record_step(&self, step: &Step) -> Result<()> {
        let mut db = self.connect()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("INSERT INTO fusion_steps VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET data=excluded.data",params![step.id,step.fusion_run_id,serde_json::to_string(step)?])?;
        tx.execute(
            "INSERT OR IGNORE INTO fusion_participants VALUES(?1,?2,?3)",
            params![step.fusion_run_id, step.participant, step.model],
        )?;
        tx.execute("INSERT INTO fusion_messages VALUES(?1,?2,?3,?4) ON CONFLICT(step_id) DO UPDATE SET session_id=excluded.session_id,agent_run_id=excluded.agent_run_id",params![step.id,step.fusion_run_id,step.session_id,step.agent_run_id])?;
        append(
            &tx,
            &step.fusion_run_id,
            "step",
            json!({"step_id":step.id,"phase":step.phase,"participant":step.participant,"status":step.status}),
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn record_issues(&self, id: &str, issues: &Value) -> Result<()> {
        let mut db = self.connect()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(items) = issues.get("issues").and_then(Value::as_array) {
            for (n, item) in items.iter().enumerate() {
                let issue_id = format!("I{}", n + 1);
                tx.execute(
                    "INSERT INTO fusion_issues VALUES(?1,?2,?3)",
                    params![id, issue_id, item.to_string()],
                )?;
                for stance in item["stances"].as_array().into_iter().flatten() {
                    tx.execute(
                        "INSERT OR REPLACE INTO fusion_positions VALUES(?1,?2,?3,?4)",
                        params![
                            id,
                            issue_id,
                            stance["participant"]
                                .as_str()
                                .context("Missing participant")?,
                            stance.to_string()
                        ],
                    )?;
                }
            }
        }
        append(&tx, id, "issues", issues.clone())?;
        tx.commit()?;
        Ok(())
    }

    pub fn unfinished_steps(&self) -> Result<Vec<Step>> {
        let db = self.connect()?;
        let rows=db.prepare("SELECT data FROM fusion_steps WHERE json_extract(data,'$.status') IN ('starting','running')")?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|raw| Ok(serde_json::from_str(&raw)?))
            .collect()
    }
    pub fn child_sessions(&self) -> Result<Vec<String>> {
        let db = self.connect()?;
        let rows = db
            .prepare("SELECT DISTINCT session_id FROM fusion_messages WHERE session_id<>''")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(rows)
    }
    pub fn runs_for_parent(&self, session: &str) -> Result<Vec<Consultation>> {
        let db = self.connect()?;
        let rows = db
            .prepare("SELECT data FROM fusion_runs WHERE parent_session=?1")?
            .query_map([session], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|raw| Ok(serde_json::from_str(&raw)?))
            .collect()
    }
    /// Purge only after the parent and linked child transcripts are deleted.
    pub fn purge_parent(&self, session: &str) -> Result<()> {
        for run in self.runs_for_parent(session)? {
            let mut db = self.connect()?;
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            for table in [
                "fusion_positions",
                "fusion_issues",
                "fusion_messages",
                "fusion_participants",
                "fusion_inputs",
                "fusion_events",
                "fusion_steps",
            ] {
                tx.execute(&format!("DELETE FROM {table} WHERE run_id=?1"), [&run.id])?;
            }
            tx.execute("DELETE FROM fusion_runs WHERE id=?1", [&run.id])?;
            tx.commit()?;
            let expected = self.root.join("snapshots").join(&run.id);
            if expected.exists() {
                std::fs::remove_dir_all(expected)?;
            }
        }
        Ok(())
    }

    pub fn steps(&self, id: &str) -> Result<Vec<Step>> {
        let db = self.connect()?;
        let rows = db
            .prepare("SELECT data FROM fusion_steps WHERE run_id=?1 ORDER BY rowid")?
            .query_map([id], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|s| Ok(serde_json::from_str(&s)?))
            .collect()
    }
    pub fn discussion(&self, id: &str) -> Result<Discussion> {
        let db = self.connect()?;
        let events=db.prepare("SELECT seq,kind,data,timestamp FROM fusion_events WHERE run_id=?1 ORDER BY seq LIMIT 256")?.query_map([id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Discussion {
            run: self.get(id)?,
            steps: self.steps(id)?,
            events: events
                .into_iter()
                .map(|(seq, kind, data, timestamp)| {
                    Ok(Event {
                        seq,
                        fusion_run_id: id.into(),
                        kind,
                        data: serde_json::from_str(&data)?,
                        timestamp,
                    })
                })
                .collect::<Result<_>>()?,
        })
    }
}

fn append(db: &Connection, id: &str, kind: &str, data: Value) -> Result<()> {
    db.execute(
        "INSERT INTO fusion_events(run_id,kind,data,timestamp) VALUES(?1,?2,?3,?4)",
        params![id, kind, data.to_string(), now_ms()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn claims_are_exclusive_and_terminal_result_cannot_win_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let parent = Parent {
            database_id: "db:0".into(),
            session_id: "parent".into(),
            run_id: "r".into(),
            tool_call_id: "t".into(),
        };
        let input = ConsultationInput {
            question: "A or B?".into(),
            goal: "choose".into(),
            background: "facts".into(),
            constraints: vec![],
            options: vec![],
            evidence: vec![],
            read_scope: vec![],
        };
        let (run, _) = store.prepare(parent, input, dir.path()).unwrap();
        store.ready(&run.id).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles = (0..2)
            .map(|n| {
                let store = store.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store.claim(&format!("owner{n}")).unwrap()
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            handles
                .into_iter()
                .filter_map(|h| h.join().unwrap())
                .count(),
            1
        );
        assert!(store.latest_for_session("other").unwrap().is_none());
        assert_eq!(
            store.latest_for_session("parent").unwrap().unwrap().id,
            run.id
        );
        store.cancel(&run.id).unwrap();
        let result = Conclusion {
            fusion_run_id: String::new(),
            conclusion: "late".into(),
            rationale: vec![],
            conditions: vec![],
            unresolved: vec![],
            next_steps: vec![],
            completeness: String::new(),
        };
        store.finish(&run.id, "owner0", result, "full").unwrap();
        assert_eq!(store.get(&run.id).unwrap().status, "cancelled");
        assert!(store.get(&run.id).unwrap().result.is_none());
        store.purge_parent("parent").unwrap();
        assert!(store.latest_for_session("parent").unwrap().is_none());
    }
    #[test]
    fn idempotency_claim_and_cancel_fence() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let parent = Parent {
            database_id: "db".into(),
            session_id: "s".into(),
            run_id: "r".into(),
            tool_call_id: "t".into(),
        };
        let input = ConsultationInput {
            question: "A or B?".into(),
            goal: "choose".into(),
            background: "facts".into(),
            constraints: vec![],
            options: vec![],
            evidence: vec![],
            read_scope: vec![],
        };
        let (run, created) = store
            .prepare(parent.clone(), input.clone(), dir.path())
            .unwrap();
        assert!(created);
        assert!(
            !store
                .prepare(parent.clone(), input.clone(), dir.path())
                .unwrap()
                .1
        );
        let mut conflict = input;
        conflict.goal = "different".into();
        assert!(store.prepare(parent, conflict, dir.path()).is_err());
        store.ready(&run.id).unwrap();
        assert_eq!(store.claim("owner").unwrap().unwrap().id, run.id);
        assert!(store.claim("owner2").unwrap().is_none());
        store.cancel(&run.id).unwrap();
        assert!(!store.phase(&run.id, "owner", "synthesizing").unwrap());
        assert_eq!(store.get(&run.id).unwrap().status, "cancelled");
    }
}
