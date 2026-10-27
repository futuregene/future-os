//! Canonical tool inspection selects small identities, then pages only the
//! requested fields. Body assembly/JSON decoding never occupy a SQLite worker.
use super::*;
use crate::session::content::reader::{Entry, Snapshot};
use anyhow::ensure;

struct Located {
    entry: Entry,
    ordinal: i64,
    name: Option<String>,
    error: Option<bool>,
    time: Option<i64>,
}

/// A newly reserved identity has no committed tool history yet. This empty
/// result is linearized at the query below; hidden fork/reset operations and
/// deleted/imported identities still require the normal published reader.
fn empty_reservation(store: &SqliteStore, session: &str) -> Result<bool> {
    let session = session.to_owned();
    store.db.read(move |db| {
        Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions s WHERE s.id=?1 AND s.revision=-1
            AND NOT EXISTS(SELECT 1 FROM entries e WHERE e.session_id=s.id)
            AND NOT EXISTS(SELECT 1 FROM content_operations o WHERE o.session_id=s.id)
            AND NOT EXISTS(SELECT 1 FROM fork_operations f WHERE f.child_session_id=s.id)
            AND NOT EXISTS(SELECT 1 FROM session_tombstones d WHERE d.session_id=s.id)
            AND NOT EXISTS(SELECT 1 FROM legacy_imports l WHERE l.session_id=s.id))",
            [session],
            |row| row.get(0),
        )?)
    })
}

fn locate(
    store: &SqliteStore,
    session: &str,
    run: &str,
    call: &str,
    kind: &'static str,
) -> Result<(Snapshot, Option<Located>)> {
    let (session, run, call) = (session.to_owned(), run.to_owned(), call.to_owned());
    store.db.read(move |db| {
        let tx=db.transaction()?;
        let scope=Snapshot::capture_from(&tx,&session)?;
        let run=crate::session::identity::BoundIdentity::run(&tx,&session,&run)?;
        let session=crate::session::identity::BoundIdentity::session(&tx,&session)?;
        let entry=tx.query_row("SELECT e.position,e.entry_id,b.ordinal,b.tool_name,b.is_error,e.timestamp_ms FROM entries e JOIN message_blocks b ON b.session_id=e.session_id AND b.entry_position=e.position WHERE e.session_id=?1 AND e.run_id=?2 AND b.tool_call_id=?3 AND b.kind=?4 ORDER BY e.position DESC,b.ordinal DESC LIMIT 1",
            params![session,run,call,kind],|row|Ok(Located {entry:Entry {position:row.get(0)?,id:row.get(1)?},ordinal:row.get(2)?,name:row.get(3)?,error:row.get(4)?,time:row.get(5)?})).optional()?;
        tx.commit()?;Ok((scope,entry))
    })
}

pub(super) fn arguments(
    store: &SqliteStore,
    session: &str,
    run: &str,
    call: &str,
) -> Result<Value> {
    if empty_reservation(store, session)? {
        return Ok(json!({"toolCallId":call,"name":null,"arguments":null}));
    }
    let (scope, entry) = locate(store, session, run, call, "tool_call")?;
    let (name, args) = if let Some(entry) = entry {
        let args = scope.field(
            store,
            &entry.entry,
            &format!("/content/{}/args", entry.ordinal),
        )?;
        (entry.name, args)
    } else {
        (None, None)
    };
    scope.verify(store)?;
    Ok(json!({"toolCallId":call,"name":name,"arguments":args}))
}

pub(super) fn output(store: &SqliteStore, session: &str, run: &str, call: &str) -> Result<Value> {
    if empty_reservation(store, session)? {
        return Ok(json!({"output":null}));
    }
    let (scope, entry) = locate(store, session, run, call, "tool_result")?;
    let output = if let Some(entry) = entry {
        let text = scope.field(
            store,
            &entry.entry,
            &format!("/content/{}/content", entry.ordinal),
        )?;
        let facts = scope.field(store, &entry.entry, "/meta/shell_result")?;
        let mut value = json!({"toolCallId":call,"runId":run,"text":text,
            "isError":entry.error.unwrap_or(false),"createdAtMs":entry.time});
        if let Some(facts) = facts {
            value["shell_result"] = facts;
        }
        Some(value)
    } else {
        None
    };
    scope.verify(store)?;
    Ok(json!({"output":output}))
}

pub(super) fn page(
    store: &SqliteStore,
    session: &str,
    run: &str,
    offset: i64,
    limit: i64,
) -> Result<Value> {
    if empty_reservation(store, session)? {
        return Ok(json!({"tools":[],"hasMore":false,"nextOffset":offset.max(0)}));
    }
    let (session, run) = (session.to_owned(), run.to_owned());
    let offset = offset.max(0);
    let limit = limit.clamp(1, 64);
    let (scope,mut rows)=store.db.read(move |db| {
        let tx=db.transaction()?;
        let scope=Snapshot::capture_from(&tx,&session)?;
        let run=crate::session::identity::BoundIdentity::run(&tx,&session,&run)?;
        let session=crate::session::identity::BoundIdentity::session(&tx,&session)?;
        let state:Option<String>=tx.query_row("SELECT status FROM runs WHERE session_id=?1 AND run_id=?2",params![session,run],|row|row.get(0)).optional()?;
        let unsettled=match state.as_deref() {Some("running")=>"running",Some("cancelled")=>"cancelled",Some("error"|"failed")=>"failed",Some("completed"|"interrupted_by_restart"|"incomplete"|"interrupted")=>"interrupted",_=>"unknown"};
        let mut query=tx.prepare("SELECT e.position,e.entry_id,b.ordinal,b.tool_call_id,b.tool_name,e.timestamp_ms,
            (SELECT r.is_error FROM message_blocks r JOIN entries re ON re.session_id=r.session_id AND re.position=r.entry_position WHERE r.session_id=e.session_id AND re.run_id=e.run_id AND r.tool_call_id=b.tool_call_id AND r.kind='tool_result' ORDER BY re.position DESC,r.ordinal DESC LIMIT 1),
            (SELECT re.timestamp_ms FROM message_blocks r JOIN entries re ON re.session_id=r.session_id AND re.position=r.entry_position WHERE r.session_id=e.session_id AND re.run_id=e.run_id AND r.tool_call_id=b.tool_call_id AND r.kind='tool_result' ORDER BY re.position DESC,r.ordinal DESC LIMIT 1)
            FROM entries e JOIN message_blocks b ON b.session_id=e.session_id AND b.entry_position=e.position WHERE e.session_id=?1 AND e.run_id=?2 AND b.kind='tool_call' ORDER BY e.position,b.ordinal LIMIT ?3 OFFSET ?4")?;
        let mut cursor=query.query(params![session,run,limit+1,offset])?;
        let mut rows=Vec::new();let mut bytes=0usize;
        while let Some(row)=cursor.next()? {
            let entry=Entry {position:row.get(0)?,id:row.get(1)?};
            let ordinal:i64=row.get(2)?;
            let ended:Option<i64>=row.get(7)?;
            let error:Option<bool>=row.get(6)?;
            let value=json!({"toolCallId":row.get::<_,Option<String>>(3)?,"runId":run,"name":row.get::<_,Option<String>>(4)?,
                "status":if ended.is_some() {if error.unwrap_or(false) {"failed"} else {"completed"}} else {unsettled},
                "startedAtMs":row.get::<_,Option<i64>>(5)?,"completedAtMs":ended});
            bytes+=entry.id.len()+value.to_string().len()+32;
            ensure!(bytes<=crate::session::content::READ_BYTES,"tool identity page exceeds read budget");
            rows.push((entry,ordinal,value));
        }
        drop(cursor);drop(query);tx.commit()?;Ok((scope,rows))
    })?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let mut tools = Vec::new();
    for (entry, ordinal, mut value) in rows {
        value["arguments"] = scope
            .field(store, &entry, &format!("/content/{ordinal}/args"))?
            .unwrap_or(Value::Null);
        tools.push(value);
    }
    scope.verify(store)?;
    Ok(json!({"hasMore":more,"nextOffset":offset+tools.len() as i64,"tools":tools}))
}
