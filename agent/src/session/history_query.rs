//! Bounded, session-scoped reads of original journal text (not display/summary projections).
use super::Manager;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

pub const HISTORY_DEFAULT_BYTES: i64 = 8_192;
pub const HISTORY_MAX_BYTES: i64 = 32_768;
pub const HISTORY_MAX_MATCHES: i64 = 20;

type EntryMetadata = (i64, String, Option<String>, Option<String>, Option<i64>);

fn require_session(db: &Connection, session: &str) -> Result<()> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1 AND revision>=0) AND NOT EXISTS(SELECT 1 FROM legacy_imports WHERE session_id=?1 AND status='skipped')",
        [session], |row| row.get(0),
    )?;
    if !exists {
        bail!("session not found or history unavailable");
    }
    Ok(())
}

impl Manager {
    /// Literal substring search, ASCII case insensitive. Scan only one session,
    /// excluding reasoning, provider metadata, checkpoints and lifecycle records.
    pub(crate) fn search_history(&self, session: &str, query: &str, limit: i64) -> Result<Value> {
        let query = query.trim();
        if session.is_empty() || query.is_empty() {
            bail!("sessionId and nonempty query are required");
        }
        if query.chars().count() > 200 || query.contains('\0') {
            bail!("query must be at most 200 characters without NUL");
        }
        if !(1..=HISTORY_MAX_MATCHES).contains(&limit) {
            bail!("search limit must be between 1 and {HISTORY_MAX_MATCHES}");
        }
        let (session, query) = (session.to_owned(), query.to_owned());
        self.storage()?.db.call(move |db| {
            let tx = db.transaction()?;
            require_session(&tx, &session)?;
            let mut stmt = tx.prepare(
                "WITH hits AS (
                    SELECT e.entry_id,e.position,e.role,e.run_id,e.timestamp_ms,b.ordinal,b.kind,b.tool_call_id,b.tool_name,
                    coalesce(CASE WHEN b.kind='tool_call' THEN b.arguments_json ELSE b.text END,'') AS body
                    FROM entries e JOIN message_blocks b ON b.session_id=e.session_id AND b.entry_position=e.position
                    WHERE e.session_id=?1 AND e.entry_type IN ('user','assistant','tool')
                    AND b.kind IN ('text','tool_call','tool_result')
                ), matches AS (
                    SELECT *,CASE WHEN tool_call_id=?2 THEN 1 ELSE instr(CAST(lower(body) AS BLOB),CAST(lower(?2) AS BLOB)) END AS hit FROM hits
                )
                SELECT entry_id,position,role,run_id,timestamp_ms,ordinal,kind,tool_call_id,tool_name,
                    substr(CAST(body AS BLOB),max(1,hit-120),480),
                    (hit-1) +
                    COALESCE((SELECT sum(length(CAST(CASE WHEN prior.kind='tool_call' THEN prior.arguments_json ELSE prior.text END AS BLOB)))
                        FROM message_blocks prior WHERE prior.session_id=?1 AND prior.entry_position=matches.position
                        AND prior.ordinal<matches.ordinal AND prior.kind IN ('text','tool_call','tool_result')),0)
                FROM matches WHERE hit>0 ORDER BY position DESC,ordinal LIMIT ?3"
            )?;
            let mut rows = stmt.query(params![session,query,limit+1])?;
            let mut matches = Vec::new();
            while let Some(row) = rows.next()? {
                matches.push(json!({
                    "entryId":row.get::<_,String>(0)?,"entryPosition":row.get::<_,i64>(1)?,
                    "role":row.get::<_,Option<String>>(2)?,"runId":row.get::<_,Option<String>>(3)?,
                    "timestampMs":row.get::<_,Option<i64>>(4)?,"blockIndex":row.get::<_,i64>(5)?,
                    "kind":row.get::<_,String>(6)?,"toolCallId":row.get::<_,Option<String>>(7)?,
                    "toolName":row.get::<_,Option<String>>(8)?,"snippet":String::from_utf8_lossy(&row.get::<_,Vec<u8>>(9)?),
                    "byteOffset":row.get::<_,i64>(10)?
                }));
            }
            let has_more = matches.len() > limit as usize;
            matches.truncate(limit as usize);
            Ok(json!({"sessionId":session,"query":query,"matches":matches,"hasMore":has_more}))
        })
    }

    /// Read an indexed entry without hydrating the session. SQLite slices the
    /// fields before transferring their bytes into Rust/RPC output buffers.
    /// Offset addresses concatenated UTF-8 bytes of visible text/argument fields,
    /// in block order, without separators. Returned chunks preserve block identity.
    pub(crate) fn read_history_entry(
        &self,
        session: &str,
        entry: &str,
        offset: i64,
        limit: i64,
    ) -> Result<Value> {
        if session.is_empty() || entry.is_empty() {
            bail!("sessionId and entryId are required");
        }
        if offset < 0 || !(4..=HISTORY_MAX_BYTES).contains(&limit) {
            bail!("offset must be nonnegative; read limit must be between 4 and {HISTORY_MAX_BYTES} UTF-8 bytes");
        }
        let (session, entry) = (session.to_owned(), entry.to_owned());
        self.storage()?.db.call(move |db| {
            let tx = db.transaction()?;
            require_session(&tx,&session)?;
            let meta: Option<EntryMetadata> = tx.query_row(
                "SELECT position,entry_type,role,run_id,timestamp_ms FROM entries WHERE session_id=?1 AND entry_id=?2 AND entry_type IN ('user','assistant','tool')",
                params![session,entry], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
            ).optional()?;
            let (position,kind,role,run,time) = meta.context("entry not found in this session's conversation history")?;
            let total: i64 = tx.query_row(
                "SELECT coalesce(sum(length(CAST(CASE WHEN kind='tool_call' THEN arguments_json ELSE text END AS BLOB))),0) FROM message_blocks WHERE session_id=?1 AND entry_position=?2 AND kind IN ('text','tool_call','tool_result')",
                params![session,position], |r| r.get(0),
            )?;
            if offset > total { bail!("offset is beyond the entry's {total} readable bytes"); }
            let mut stmt = tx.prepare(
                "SELECT ordinal,kind,tool_call_id,tool_name,is_error,length(CAST(CASE WHEN kind='tool_call' THEN arguments_json ELSE text END AS BLOB)) FROM message_blocks WHERE session_id=?1 AND entry_position=?2 AND kind IN ('text','tool_call','tool_result') ORDER BY ordinal"
            )?;
            let mut rows = stmt.query(params![session,position])?;
            let mut base = 0_i64;
            let mut consumed = 0_i64;
            let mut chunks = Vec::new();
            while let Some(row) = rows.next()? {
                let ordinal: i64 = row.get(0)?;
                let block_kind: String = row.get(1)?;
                let length: i64 = row.get::<_,Option<i64>>(5)?.unwrap_or(0);
                if base+length <= offset { base+=length; continue; }
                let start = (offset-base).max(0);
                let remaining = limit-consumed;
                if remaining == 0 { break; }
                let bytes: Vec<u8> = tx.query_row(
                    "SELECT substr(CAST(CASE WHEN kind='tool_call' THEN arguments_json ELSE text END AS BLOB),?4+1,?5) FROM message_blocks WHERE session_id=?1 AND entry_position=?2 AND ordinal=?3",
                    params![session,position,ordinal,start,remaining], |r| r.get(0),
                )?;
                let text = match std::str::from_utf8(&bytes) {
                    Ok(text) => text,
                    Err(error) if error.error_len().is_none() => std::str::from_utf8(&bytes[..error.valid_up_to()])?,
                    Err(_) => bail!("offset must be on a UTF-8 character boundary"),
                };
                if text.is_empty() { break; }
                consumed += text.len() as i64;
                chunks.push(json!({"blockIndex":ordinal,"kind":block_kind,
                    "field":if block_kind=="tool_call" {"argumentsJson"} else {"text"},
                    "toolCallId":row.get::<_,Option<String>>(2)?,"toolName":row.get::<_,Option<String>>(3)?,
                    "isError":row.get::<_,Option<bool>>(4)?,"blockByteOffset":start,"blockTotalBytes":length,"text":text}));
                if start+(text.len() as i64) < length { break; }
                base+=length;
            }
            let next=offset+consumed;
            if next<total && consumed==0 { bail!("offset must be on a UTF-8 character boundary"); }
            let omitted: Vec<String> = tx.prepare("SELECT DISTINCT kind FROM message_blocks WHERE session_id=?1 AND entry_position=?2 AND kind NOT IN ('text','tool_call','tool_result') ORDER BY kind")?
                .query_map(params![session,position],|r|r.get(0))?.collect::<rusqlite::Result<_>>()?;
            Ok(json!({"sessionId":session,"entryId":entry,"entryPosition":position,"entryType":kind,
                "role":role,"runId":run,"timestampMs":time,"offset":offset,"maxBytes":limit,
                "totalBytes":total,"chunks":chunks,"hasMore":next<total,
                "nextOffset":if next<total {Some(next)} else {None},"omittedKinds":omitted}))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (tempfile::TempDir, Manager) {
        let temp = tempfile::tempdir().unwrap();
        let m = Manager::new(temp.path().to_owned());
        m.storage().unwrap().replace("s",vec![
            json!({"id":"u","type":"user","role":"user","timestamp":"2026-01-01T00:00:00Z","content":[{"type":"text","text":"中文 ExpoSharing literal %_"}]}),
            json!({"id":"a","type":"assistant","role":"assistant","timestamp":"2026-01-01T00:00:01Z","content":[{"type":"reasoning","text":"hidden-only"},{"type":"text","text":"abc"},{"type":"tool_call","id":"call-1","name":"read","args":{"path":"ShareInbox.swift"}}]}),
            json!({"id":"t","type":"tool","role":"tool","timestamp":"2026-01-01T00:00:02Z","content":[{"type":"tool_result","tool_call_id":"call-1","content":"ab中文🙂\u{0000}tail ExpoSharing","is_error":true}]}),
        ]).unwrap();
        m.storage().unwrap().replace("other",vec![json!({"id":"foreign","type":"user","role":"user","timestamp":"2026-01-01T00:00:00Z","content":"OTHER ExpoSharing"})]).unwrap();
        (temp, m)
    }
    #[test]
    fn search_is_scoped_literal_bounded_and_excludes_reasoning() {
        let (_t, m) = setup();
        let revision = m.session_revision("s").unwrap();
        let r = m.search_history("s", "exposharing", 1).unwrap();
        assert_eq!(r["matches"].as_array().unwrap().len(), 1);
        assert_eq!(r["hasMore"], true);
        assert_eq!(r["matches"][0]["entryId"], "t");
        assert!(m.search_history("s", "hidden-only", 5).unwrap()["matches"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(
            m.search_history("s", "%_", 5).unwrap()["matches"][0]["entryId"],
            "u"
        );
        assert_eq!(
            m.search_history("s", "ShareInbox", 5).unwrap()["matches"][0]["kind"],
            "tool_call"
        );
        assert!(m.search_history("missing", "x", 5).is_err());
        assert!(m.search_history("s", " ", 5).is_err());
        assert!(m.search_history("s", "x", 21).is_err());
        assert!(revision == m.session_revision("s").unwrap());
    }
    #[test]
    fn indexed_read_preserves_unicode_nul_and_pages_without_loss() {
        let (_t, m) = setup();
        let mut offset = 0;
        let mut all = String::new();
        loop {
            let r = m.read_history_entry("s", "t", offset, 4).unwrap();
            for c in r["chunks"].as_array().unwrap() {
                all.push_str(c["text"].as_str().unwrap());
                assert_eq!(c["toolCallId"], "call-1");
                assert_eq!(c["isError"], true);
            }
            if !r["hasMore"].as_bool().unwrap() {
                break;
            }
            let next = r["nextOffset"].as_i64().unwrap();
            assert!(next > offset);
            offset = next;
        }
        assert_eq!(all, "ab中文🙂\u{0000}tail ExpoSharing");
        assert!(m.read_history_entry("s", "t", 3, 4).is_err());
        assert!(m.read_history_entry("s", "foreign", 0, 100).is_err());
        assert!(m.read_history_entry("s", "t", 999, 100).is_err());
        assert!(m.read_history_entry("s", "t", -1, 100).is_err());
        assert!(m.read_history_entry("s", "t", 0, 3).is_err());
    }
    #[test]
    fn read_returns_block_ids_arguments_and_omissions() {
        let (_t, m) = setup();
        let r = m.read_history_entry("s", "a", 0, 8192).unwrap();
        let c = r["chunks"].as_array().unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[0]["blockIndex"], 1);
        assert_eq!(c[1]["blockIndex"], 2);
        assert_eq!(c[1]["toolName"], "read");
        assert!(c[1]["text"].as_str().unwrap().contains("ShareInbox.swift"));
        assert_eq!(r["omittedKinds"], json!(["reasoning"]));
        let start = m.search_history("s", "ShareInbox", 5).unwrap()["matches"][0]["byteOffset"]
            .as_i64()
            .unwrap();
        assert!(
            m.read_history_entry("s", "a", start, 8192).unwrap()["chunks"][0]["text"]
                .as_str()
                .unwrap()
                .contains("ShareInbox")
        );
    }
    #[test]
    fn ids_are_searchable_and_entry_lookup_uses_the_identity_index() {
        let (_t, m) = setup();
        let hits = m.search_history("s", "call-1", 5).unwrap();
        assert_eq!(hits["matches"].as_array().unwrap().len(), 2);
        assert_eq!(hits["matches"][0]["entryId"], "t");
        assert!(m.search_history("s", "' OR 1=1 --", 5).unwrap()["matches"]
            .as_array()
            .unwrap()
            .is_empty());
        let plan = m.storage().unwrap().db.call(|db| {
            let mut stmt = db.prepare("EXPLAIN QUERY PLAN SELECT position FROM entries WHERE session_id=?1 AND entry_id=?2")?;
            let rows = stmt.query_map(params!["s", "t"], |row| row.get::<_, String>(3))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.join("\n"))
        }).unwrap();
        assert!(plan.contains("entries_identity_unique"), "{plan}");
    }

    #[test]
    fn very_large_single_line_is_bounded() {
        let (_t, m) = setup();
        m.storage().unwrap().append("s",vec![json!({"id":"large","type":"user","role":"user","timestamp":"2026-01-01T00:00:03Z","content":"z".repeat(2_000_000)})]).unwrap();
        let r = m.read_history_entry("s", "large", 0, 8192).unwrap();
        assert_eq!(r["totalBytes"], 2_000_000);
        assert_eq!(r["nextOffset"], 8192);
        assert!(r.to_string().len() < 10_000);
    }
}
