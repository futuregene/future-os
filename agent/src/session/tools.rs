//! Tool inspection reads the canonical message blocks, not an event replay.
use super::{sqlite_store::SqliteStore, Manager};
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

impl Manager {
    pub(crate) fn tool_page(
        &self,
        session: &str,
        run: &str,
        offset: i64,
        limit: i64,
    ) -> Result<Value> {
        self.storage()?.tool_page(session, run, offset, limit)
    }
    pub(crate) fn tool_output(&self, session: &str, run: &str, call: &str) -> Result<Value> {
        self.storage()?.tool_output(session, run, call)
    }
}

impl SqliteStore {
    fn tool_page(&self, session: &str, run: &str, offset: i64, limit: i64) -> Result<Value> {
        let (session, run) = (session.to_owned(), run.to_owned());
        let offset = offset.max(0);
        let limit = limit.clamp(1, 200);
        self.db.call(move |db| {
            let state: Option<String> = db.query_row("SELECT status FROM runs WHERE session_id=?1 AND run_id=?2",params![session,run],|r|r.get(0)).optional()?;
            let unsettled_status = match state.as_deref() { Some("running") => "running", Some("cancelled") => "cancelled", Some("error" | "failed") => "failed", Some("completed" | "interrupted_by_restart" | "incomplete" | "interrupted") => "interrupted", _ => "unknown" };
            let mut stmt=db.prepare("SELECT b.tool_call_id,b.tool_name,b.arguments_json,e.timestamp_ms,
                (SELECT r.is_error FROM message_blocks r JOIN entries re ON re.session_id=r.session_id AND re.position=r.entry_position WHERE r.session_id=e.session_id AND re.run_id=e.run_id AND r.tool_call_id=b.tool_call_id AND r.kind='tool_result' ORDER BY re.position DESC LIMIT 1),
                (SELECT re.timestamp_ms FROM message_blocks r JOIN entries re ON re.session_id=r.session_id AND re.position=r.entry_position WHERE r.session_id=e.session_id AND re.run_id=e.run_id AND r.tool_call_id=b.tool_call_id AND r.kind='tool_result' ORDER BY re.position DESC LIMIT 1)
                FROM entries e JOIN message_blocks b ON b.session_id=e.session_id AND b.entry_position=e.position
                WHERE e.session_id=?1 AND e.run_id=?2 AND b.kind='tool_call' ORDER BY e.position,b.ordinal LIMIT ?3 OFFSET ?4")?;
            let mut rows=stmt.query(params![session,run,limit+1,offset])?;
            let mut tools=Vec::new();
            while let Some(row)=rows.next()? {
                let arguments:Option<String>=row.get(2)?;
                let error:Option<bool>=row.get(4)?;
                let ended:Option<i64>=row.get(5)?;
                tools.push(json!({"toolCallId":row.get::<_,Option<String>>(0)?,"runId":run,"name":row.get::<_,Option<String>>(1)?,
                    "arguments":arguments.map(|s|serde_json::from_str::<Value>(&s)).transpose()?,
                    "status":if ended.is_some() { if error.unwrap_or(false) {"failed"} else {"completed"} } else {unsettled_status},
                    "startedAtMs":row.get::<_,Option<i64>>(3)?,"completedAtMs":ended}));
            }
            let more=tools.len()>limit as usize;tools.truncate(limit as usize);
            Ok(json!({"tools":tools,"hasMore":more,"nextOffset":offset+tools.len() as i64}))
        })
    }

    fn tool_output(&self, session: &str, run: &str, call: &str) -> Result<Value> {
        let (session, run, call) = (session.to_owned(), run.to_owned(), call.to_owned());
        self.db.call(move |db| {
            let mut stmt=db.prepare("SELECT b.text,b.is_error,e.timestamp_ms FROM message_blocks b JOIN entries e ON e.session_id=b.session_id AND e.position=b.entry_position WHERE b.session_id=?1 AND e.run_id=?2 AND b.tool_call_id=?3 AND b.kind='tool_result' ORDER BY e.position DESC LIMIT 1")?;
            let mut rows=stmt.query(params![session,run,call])?;
            let output=if let Some(row)=rows.next()? {Some(json!({"toolCallId":call,"runId":run,"text":row.get::<_,Option<String>>(0)?,"isError":row.get::<_,Option<bool>>(1)?.unwrap_or(false),"createdAtMs":row.get::<_,Option<i64>>(2)?}))}else{None};
            Ok(json!({"output":output}))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tool_queries_are_run_scoped_paged_and_preserve_null_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
        let mut entries = Vec::new();
        for (run, count) in [("one", 3), ("two", 1)] {
            for index in 0..count {
                entries.push(json!({"id":format!("{run}-{index}"),"type":"assistant","role":"assistant","timestamp":"2026-01-01T00:00:00Z","meta":{"run_id":run},"content":[{"type":"tool_call","id":format!("call-{index}"),"name":"read","args":null}]}));
            }
        }
        entries.push(json!({"id":"result","type":"tool","role":"tool","timestamp":"2026-01-01T00:00:01Z","meta":{"run_id":"two"},"content":[{"type":"tool_result","tool_call_id":"call-0","content":"synthetic result"}]}));
        entries.push(json!({"id":"terminal","type":"run_terminal","timestamp":"2026-01-01T00:00:02Z","content":{"run_id":"one","state":"completed","run_tokens":1,"run_duration_ms":2000}}));
        store.replace("s", entries).unwrap();
        let page = store.tool_page("s", "one", 0, 2).unwrap();
        assert_eq!(page["tools"].as_array().unwrap().len(), 2);
        assert_eq!(page["hasMore"], true);
        assert_eq!(page["tools"][0]["arguments"], Value::Null);
        assert_eq!(page["tools"][0]["status"], "interrupted");
        assert_eq!(store.tool_page("s", "one", 2, 2).unwrap()["hasMore"], false);
        assert!(store.tool_output("s", "one", "call-0").unwrap()["output"].is_null());
        assert_eq!(
            store.tool_output("s", "two", "call-0").unwrap()["output"]["text"],
            "synthetic result"
        );
        assert_eq!(
            store.tool_page("s", "two", 0, 2).unwrap()["tools"][0]["status"],
            "completed"
        );
    }
}
