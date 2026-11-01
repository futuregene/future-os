//! Bounded, session-scoped reads of original journal text (not display/summary projections).
use crate::session::identity::BoundIdentity;
use crate::session::Manager;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

pub const HISTORY_DEFAULT_BYTES: i64 = 8_192;
pub const HISTORY_MAX_BYTES: i64 = 32_768;
pub const HISTORY_MAX_MATCHES: i64 = 20;
/// How many of the most recently updated sessions a cross-session search scans
/// when the caller does not ask for a different bound.
pub const HISTORY_DEFAULT_SESSIONS: i64 = 50;
pub const HISTORY_MAX_SESSIONS: i64 = 500;

type EntryMetadata = (
    i64,
    String,
    Option<String>,
    Option<BoundIdentity>,
    Option<i64>,
);

/// SQLite slices the snippet with `substr(CAST(body AS BLOB), ...)`, which is a
/// *byte* window and therefore can begin or end inside a multi-byte character.
/// Decoding such a window lossily renders the halves as U+FFFD (a box in the
/// terminal), so drop the partial character on each edge instead: a snippet may
/// be a few bytes shorter, never a broken character.
fn trim_partial_characters(bytes: &[u8]) -> &[u8] {
    // A window that starts mid-character begins with continuation bytes
    // (0b10xxxxxx); skip them to reach the next character boundary.
    let start = bytes.iter().take_while(|b| *b & 0xC0 == 0x80).count();
    let bytes = &bytes[start..];
    // On the trailing edge `from_utf8` reports the last *complete* character:
    // an incomplete sequence yields `error_len() == None` and `valid_up_to()`
    // at the start of the truncated character. (An actually invalid byte cannot
    // occur in a TEXT column's UTF-8, but cutting there would still be safe.)
    match std::str::from_utf8(bytes) {
        Ok(_) => bytes,
        Err(error) => &bytes[..error.valid_up_to()],
    }
}

fn require_session(db: &Connection, session: &str) -> Result<()> {
    let stored_session = crate::session::identity::session_key(db, session)?;
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1 AND revision>=0) AND NOT EXISTS(SELECT 1 FROM legacy_imports WHERE session_id=?1 AND status='skipped')",
        [&stored_session], |row| row.get(0),
    )?;
    if !exists {
        bail!("session not found or history unavailable");
    }
    Ok(())
}

/// Shared validation for the two search entry points. Rejects the query shapes
/// the CLI documents, so both surfaces fail with the same wording.
fn validate_history_query(query: &str, limit: i64) -> Result<String> {
    let query = query.trim();
    if query.is_empty() {
        bail!("a nonempty query is required");
    }
    if query.chars().count() > 200 || query.contains('\0') {
        bail!("query must be at most 200 characters without NUL");
    }
    if !(1..=HISTORY_MAX_MATCHES).contains(&limit) {
        bail!("search limit must be between 1 and {HISTORY_MAX_MATCHES}");
    }
    Ok(query.to_owned())
}

/// The per-session match query, shared by single-session and cross-session
/// search so both report identical fields, snippets and byte offsets.
///
/// Returns at most `limit + 1` rows: the extra row is how each caller tells
/// "exactly limit matches" from "more exist".
fn search_session_matches(
    db: &Connection,
    session: &str,
    query: &str,
    limit: i64,
) -> Result<Vec<Value>> {
    let session = BoundIdentity::session(db, session)?;
    let mut stmt = db.prepare_cached(
        "WITH hits AS (
            SELECT e.entry_id,e.position,e.role,e.run_id,e.timestamp_ms,b.ordinal,b.kind,b.tool_call_id,b.tool_name,
            coalesce(CASE WHEN b.kind='tool_call' THEN b.arguments_json ELSE b.text END,'') AS body
            FROM entries e JOIN resolved_message_blocks b ON b.session_id=e.session_id AND b.entry_position=e.position
            WHERE e.session_id=?1 AND e.entry_type IN ('user','assistant','tool')
            AND b.kind IN ('text','tool_call','tool_result')
        ), matches AS (
            SELECT *,CASE WHEN tool_call_id=?2 THEN 1 ELSE instr(CAST(lower(body) AS BLOB),CAST(lower(?2) AS BLOB)) END AS hit FROM hits
        )
        SELECT entry_id,position,role,run_id,timestamp_ms,ordinal,kind,tool_call_id,tool_name,
            substr(CAST(body AS BLOB),max(1,hit-120),480),
            (hit-1) +
            COALESCE((SELECT sum(length(CAST(CASE WHEN prior.kind='tool_call' THEN prior.arguments_json ELSE prior.text END AS BLOB)))
                FROM resolved_message_blocks prior WHERE prior.session_id=?1 AND prior.entry_position=matches.position
                AND prior.ordinal<matches.ordinal AND prior.kind IN ('text','tool_call','tool_result')),0)
        FROM matches WHERE hit>0 ORDER BY position DESC,ordinal LIMIT ?3"
    )?;
    let mut rows = stmt.query(params![session, query, limit + 1])?;
    let mut matches = Vec::new();
    while let Some(row) = rows.next()? {
        let snippet = row.get::<_, Option<Vec<u8>>>(9)?.unwrap_or_default();
        matches.push(json!({
            "entryId":row.get::<_,BoundIdentity>(0)?,"entryPosition":row.get::<_,i64>(1)?,
            "role":row.get::<_,Option<String>>(2)?,"runId":row.get::<_,Option<BoundIdentity>>(3)?,
            "timestampMs":row.get::<_,Option<i64>>(4)?,"blockIndex":row.get::<_,i64>(5)?,
            "kind":row.get::<_,String>(6)?,"toolCallId":row.get::<_,Option<String>>(7)?,
            "toolName":row.get::<_,Option<String>>(8)?,
            // Trimmed to character boundaries, so lossy decoding only
            // ever substitutes for genuinely invalid stored bytes.
            "snippet":String::from_utf8_lossy(trim_partial_characters(&snippet)),
            "byteOffset":row.get::<_,i64>(10)?
        }));
    }
    Ok(matches)
}

impl Manager {
    /// Literal substring search, ASCII case insensitive. Scan only one session,
    /// excluding reasoning, provider metadata, checkpoints and lifecycle records.
    pub(crate) fn search_history(&self, session: &str, query: &str, limit: i64) -> Result<Value> {
        if session.is_empty() {
            bail!("sessionId and nonempty query are required");
        }
        let query = validate_history_query(query, limit)?;
        let store = self.storage()?;
        if store.storage_format(session)? == crate::session::content::FORMAT_VERSION {
            let mut matches =
                crate::session::content::reader::Snapshot::search(store, session, &query, limit)?;
            let has_more = matches.len() > limit as usize;
            matches.truncate(limit as usize);
            return Ok(
                json!({"sessionId":session,"query":query,"matches":matches,"hasMore":has_more}),
            );
        }
        let (session, query) = (session.to_owned(), query);
        self.storage()?.db.read(move |db| {
            let tx = db.transaction()?;
            require_session(&tx, &session)?;
            let mut matches = search_session_matches(&tx, &session, &query, limit)?;
            let has_more = matches.len() > limit as usize;
            matches.truncate(limit as usize);
            Ok(json!({"sessionId":session,"query":query,"matches":matches,"hasMore":has_more}))
        })
    }

    /// The same literal search, but across the `max_sessions` most recently
    /// updated sessions instead of one. Sessions are scanned newest-first and
    /// the merged matches are ordered by their own timestamps, so a single call
    /// answers "have we ever talked about X" without knowing which session.
    ///
    /// Search cost is bounded by the session cap, not by the query, so the
    /// response reports `scannedSessions` and `truncated` rather than silently
    /// searching less than the whole history.
    pub(crate) fn search_history_all(
        &self,
        query: &str,
        limit: i64,
        max_sessions: i64,
    ) -> Result<Value> {
        let query = validate_history_query(query, limit)?;
        if !(1..=HISTORY_MAX_SESSIONS).contains(&max_sessions) {
            bail!("session scan bound must be between 1 and {HISTORY_MAX_SESSIONS}");
        }
        let store = self.storage()?;
        let mut sessions: Vec<(String, Option<String>, i64)> = store.db.read(move |db| {
            let mut stmt=db.prepare_cached("SELECT s.id,s.title,coalesce(f.format,1) FROM sessions s
                LEFT JOIN session_formats f ON f.session_id=s.id
                WHERE s.revision>=0 AND NOT EXISTS(SELECT 1 FROM legacy_imports li WHERE li.session_id=s.id AND li.status='skipped')
                ORDER BY s.updated_at_ms DESC,s.id DESC LIMIT ?1")?;
            let rows=stmt.query_map([max_sessions+1],|row|Ok((row.get::<_,BoundIdentity>(0)?.to_string(),row.get(1)?,row.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
            Ok(rows)
        })?;
        let truncated = sessions.len() as i64 > max_sessions;
        sessions.truncate(max_sessions as usize);
        let mut matches = Vec::new();
        let mut any_session_more = false;
        for (session, title, format) in &sessions {
            let mut found = if *format == crate::session::content::FORMAT_VERSION {
                crate::session::content::reader::Snapshot::search(store, session, &query, limit)?
            } else {
                let (session, query) = (session.clone(), query.clone());
                store.db.read(move |db| {
                    let tx = db.transaction()?;
                    require_session(&tx, &session)?;
                    let rows = search_session_matches(&tx, &session, &query, limit)?;
                    tx.commit()?;
                    Ok(rows)
                })?
            };
            if found.len() > limit as usize {
                any_session_more = true;
                found.truncate(limit as usize);
            }
            for entry in &mut found {
                entry["sessionId"] = json!(session);
                if let Some(title) = title {
                    entry["sessionName"] = json!(title);
                }
            }
            matches.append(&mut found);
            // Keep only the best candidates between sessions. Stable ties
            // preserve newest-session/entry order without growing limit*N RAM.
            matches.sort_by(|a, b| {
                b["timestampMs"]
                    .as_i64()
                    .unwrap_or(0)
                    .cmp(&a["timestampMs"].as_i64().unwrap_or(0))
            });
            if matches.len() > limit as usize {
                any_session_more = true;
                matches.truncate(limit as usize);
            }
        }
        Ok(
            json!({"query":query,"matches":matches,"hasMore":any_session_more,
            "scannedSessions":sessions.len(),"truncated":truncated}),
        )
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
        let store = self.storage()?;
        if store.storage_format(session)? == crate::session::content::FORMAT_VERSION {
            return crate::session::content::reader::Snapshot::recall(
                store, session, entry, offset, limit,
            );
        }
        let (session, entry) = (session.to_owned(), entry.to_owned());
        store.db.read(move |db| {
            let tx = db.transaction()?;
            require_session(&tx,&session)?;
            let meta: Option<EntryMetadata> = tx.query_row(
                "SELECT position,entry_type,role,run_id,timestamp_ms FROM entries WHERE session_id=?1 AND entry_id=?2 AND entry_type IN ('user','assistant','tool')",
                params![session,entry], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
            ).optional()?;
            let (position,kind,role,run,time) = meta.context("entry not found in this session's conversation history")?;
            let total: i64 = tx.query_row(
                "SELECT coalesce(sum(length(CAST(CASE WHEN kind='tool_call' THEN arguments_json ELSE text END AS BLOB))),0) FROM resolved_message_blocks WHERE session_id=?1 AND entry_position=?2 AND kind IN ('text','tool_call','tool_result')",
                params![session,position], |r| r.get(0),
            )?;
            if offset > total { bail!("offset is beyond the entry's {total} readable bytes"); }
            let mut stmt = tx.prepare_cached(
                "SELECT ordinal,kind,tool_call_id,tool_name,is_error,length(CAST(CASE WHEN kind='tool_call' THEN arguments_json ELSE text END AS BLOB)) FROM resolved_message_blocks WHERE session_id=?1 AND entry_position=?2 AND kind IN ('text','tool_call','tool_result') ORDER BY ordinal"
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
                    "SELECT substr(CAST(CASE WHEN kind='tool_call' THEN arguments_json ELSE text END AS BLOB),?4+1,?5) FROM resolved_message_blocks WHERE session_id=?1 AND entry_position=?2 AND ordinal=?3",
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
            let omitted: Vec<String> = tx.prepare_cached("SELECT DISTINCT kind FROM resolved_message_blocks WHERE session_id=?1 AND entry_position=?2 AND kind NOT IN ('text','tool_call','tool_result') ORDER BY kind")?
                .query_map(params![session,position],|r|r.get(0))?.collect::<rusqlite::Result<_>>()?;
            Ok(json!({"sessionId":session,"entryId":entry,"entryPosition":position,"entryType":kind,
                "role":role,"runId":run,"timestampMs":time,"offset":offset,"maxBytes":limit,
                "totalBytes":total,"chunks":chunks,"hasMore":next<total,
                "nextOffset":if next<total {Some(next)} else {None},"omittedKinds":omitted}))
        })
    }
}

#[cfg(test)]
#[path = "recall_tests.rs"]
mod tests;
