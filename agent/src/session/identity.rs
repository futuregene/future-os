//! Business identity at the storage boundary. New UUIDs use their full 16-byte
//! value; historical text remains opaque and is never normalized on read.
//! Public APIs render UUIDs in canonical 36-character form. Internal row IDs
//! and provider-owned call IDs are different types and do not use this codec.
use rusqlite::{
    types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef},
    ToSql,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StoredIdentity {
    Uuid(uuid::Uuid),
    Legacy(String),
}

impl StoredIdentity {
    /// Only a new application-generated identity is admitted here. Loading an
    /// old UUID-looking TEXT value must still use FromSql's Legacy branch.
    pub(crate) fn generated(wire: &str) -> anyhow::Result<Self> {
        let id = uuid::Uuid::parse_str(wire)?;
        anyhow::ensure!(
            id.to_string() == wire,
            "new identity must be a canonical UUID"
        );
        Ok(Self::Uuid(id))
    }

    pub(crate) fn wire(&self) -> String {
        match self {
            Self::Uuid(id) => id.to_string(),
            Self::Legacy(value) => value.clone(),
        }
    }
}

impl FromSql for StoredIdentity {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value {
            ValueRef::Blob(bytes) => uuid::Uuid::from_slice(bytes)
                .map(Self::Uuid)
                .map_err(|error| FromSqlError::Other(Box::new(error))),
            ValueRef::Text(bytes) => std::str::from_utf8(bytes)
                .map(|text| Self::Legacy(text.into()))
                .map_err(|error| FromSqlError::Other(Box::new(error))),
            _ => Err(FromSqlError::InvalidType),
        }
    }
}

impl ToSql for StoredIdentity {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Borrowed(match self {
            Self::Uuid(id) => ValueRef::Blob(id.as_bytes()),
            Self::Legacy(text) => ValueRef::Text(text.as_bytes()),
        }))
    }
}

/// Built-in SQL only, so inspection views work in independent SQLite tools.
/// `column` is a trusted schema expression, never a user-supplied identifier.
pub(crate) fn inspection_expression(column: &str) -> String {
    let hex = format!("hex({column})");
    format!("CASE WHEN typeof({column})='blob' AND length({column})=16 THEN lower(substr({hex},1,8)||'-'||substr({hex},9,4)||'-'||substr({hex},13,4)||'-'||substr({hex},17,4)||'-'||substr({hex},21,12)) ELSE {column} END")
}

/// Convert a new canonical wire UUID in a SQL staging expression without
/// changing arbitrary opaque IDs. This is used only by new-format writers.
/// The expression is trusted schema SQL; no user input is interpolated.
pub(crate) fn storage_expression(expression: &str) -> String {
    let raw = format!("unhex(replace({expression},'-',''))");
    format!("CASE WHEN typeof({expression})='text' AND length({expression})=36 AND {expression} GLOB '????????-????-????-????-????????????' AND lower({expression})={expression} AND length({raw})=16 THEN {raw} ELSE {expression} END")
}

/// Read-only, portable SQL inspection of UUIDs. No duplicate text is stored
/// and no extension has to be loaded in a separate database viewer.
pub(crate) fn inspection_views_sql() -> String {
    let session = inspection_expression("session_id");
    let entry = inspection_expression("entry_id");
    let run = inspection_expression("run_id");
    let id = inspection_expression("id");
    let value = inspection_expression("value");
    format!("CREATE VIEW IF NOT EXISTS sessions_inspect AS SELECT {id} AS id,revision,created_at_ms,updated_at_ms,typeof(id) AS id_storage_type,length(CAST(id AS BLOB)) AS id_bytes FROM sessions;
CREATE VIEW IF NOT EXISTS entries_inspect AS SELECT {session} AS session_id,position,{entry} AS entry_id,entry_type,role,{run} AS run_id,timestamp_ms,typeof(entry_id) AS id_storage_type,length(CAST(entry_id AS BLOB)) AS id_bytes FROM entries;
CREATE VIEW IF NOT EXISTS runs_inspect AS SELECT {session} AS session_id,{run} AS run_id,status,run_sequence,epoch,started_at_ms,completed_at_ms,typeof(run_id) AS id_storage_type,length(CAST(run_id AS BLOB)) AS id_bytes FROM runs;
CREATE VIEW IF NOT EXISTS stream_symbols_inspect AS SELECT id,stream_id,{value} AS value,typeof(value) AS storage_type,length(CAST(value AS BLOB)) AS stored_bytes FROM stream_symbols;")
}

/// Resolve an existing session's actual storage representation with two indexed
/// candidates. Historical UUID-looking TEXT is not implicitly reclassified.
/// Unknown identities remain text so read-only "not found" behavior is stable.
pub(crate) fn session_key(db: &rusqlite::Connection, wire: &str) -> anyhow::Result<StoredIdentity> {
    // A valid native key can only render as a UUID. Opaque historical strings
    // have exactly one representation and need no extra database lookup.
    let Ok(binary) = uuid::Uuid::parse_str(wire) else {
        return Ok(StoredIdentity::Legacy(wire.into()));
    };
    // Tombstones outlive their session rows; pending/skipped imports can also
    // reserve an identity before a transcript exists. Include all authorities
    // so deletion or an unavailable import cannot resurrect a UUID as a new
    // physical representation. Every arm uses its primary identity index.
    let mut query = db.prepare_cached(
        "SELECT id FROM sessions WHERE id IN (?1,?2)
        UNION SELECT session_id FROM session_tombstones WHERE session_id IN (?1,?2)
        UNION SELECT session_id FROM legacy_imports WHERE session_id IN (?1,?2) LIMIT 2",
    )?;
    let rows = query
        .query_map(
            rusqlite::params![wire, binary.as_bytes().as_slice()],
            |row| row.get::<_, StoredIdentity>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    anyhow::ensure!(
        rows.len() < 2,
        "ambiguous session identity representations; preserve database for recovery"
    );
    Ok(rows
        .into_iter()
        .next()
        .unwrap_or_else(|| StoredIdentity::Legacy(wire.into())))
}

/// An identity carried through a SQL read/write scope. The wire spelling is
/// available to JSON/UI consumers; SQL parameters retain the actual stored
/// representation rather than reparsing an opaque historical identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BoundIdentity {
    stored: StoredIdentity,
    wire: String,
}
impl BoundIdentity {
    pub(crate) fn session(db: &rusqlite::Connection, wire: &str) -> anyhow::Result<Self> {
        Ok(Self::from_stored(session_key(db, wire)?))
    }
    /// Only an explicit new-format creation may select a native representation.
    /// Existing history/imports/tombstones reserve their original public ID.
    pub(crate) fn fresh_session(db: &rusqlite::Connection, wire: &str) -> anyhow::Result<Self> {
        let existing = Self::session(db, wire)?;
        let known:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1) OR EXISTS(SELECT 1 FROM session_tombstones WHERE session_id=?1) OR EXISTS(SELECT 1 FROM legacy_imports WHERE session_id=?1)",[&existing],|row|row.get(0))?;
        anyhow::ensure!(!known, "new conversation identity already exists");
        Ok(Self::from_stored(
            StoredIdentity::generated(wire).unwrap_or_else(|_| StoredIdentity::Legacy(wire.into())),
        ))
    }
    pub(crate) fn entry(
        db: &rusqlite::Connection,
        session: &str,
        wire: &str,
    ) -> anyhow::Result<Self> {
        Self::scoped(
            db,
            session,
            wire,
            "SELECT entry_id FROM entries WHERE session_id=?1 AND entry_id IN (?2,?3) LIMIT 2",
        )
    }

    pub(crate) fn run(
        db: &rusqlite::Connection,
        session: &str,
        wire: &str,
    ) -> anyhow::Result<Self> {
        // A stream or durable acceptance may precede its transcript header.
        // Each arm uses its owning table's session/run index. UNION preserves
        // the distinction between TEXT and BLOB while folding repeated keys.
        Self::scoped(
            db,
            session,
            wire,
            "SELECT run_id FROM runs WHERE session_id=?1 AND run_id IN (?2,?3)
             UNION SELECT run_id FROM stream_runs WHERE session_id=?1 AND run_id IN (?2,?3)
             UNION SELECT run_id FROM queued_run_receipts WHERE session_id=?1 AND run_id IN (?2,?3) LIMIT 2",
        )
    }

    fn scoped(
        db: &rusqlite::Connection,
        session: &str,
        wire: &str,
        sql: &'static str,
    ) -> anyhow::Result<Self> {
        use rusqlite::OptionalExtension as _;
        let Ok(binary) = uuid::Uuid::parse_str(wire) else {
            return Ok(Self::from_stored(StoredIdentity::Legacy(wire.into())));
        };
        let parent = session_key(db, session)?;
        let mut query = db.prepare_cached(sql)?;
        let rows = query
            .query_map(
                rusqlite::params![&parent, wire, binary.as_bytes().as_slice()],
                |r| r.get::<_, StoredIdentity>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        anyhow::ensure!(
            rows.len() < 2,
            "ambiguous scoped identity representations; preserve database for recovery"
        );
        if let Some(stored) = rows.into_iter().next() {
            return Ok(Self::from_stored(stored));
        }
        let format: i64 = db
            .query_row(
                "SELECT format FROM session_formats WHERE session_id=?1",
                [&parent],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(1);
        let stored = if format == 2 {
            StoredIdentity::generated(wire).unwrap_or_else(|_| StoredIdentity::Legacy(wire.into()))
        } else {
            StoredIdentity::Legacy(wire.into())
        };
        Ok(Self::from_stored(stored))
    }
    pub(crate) fn from_stored(stored: StoredIdentity) -> Self {
        let wire = stored.wire();
        Self { stored, wire }
    }
    pub(crate) fn as_str(&self) -> &str {
        &self.wire
    }
}
impl std::ops::Deref for BoundIdentity {
    type Target = str;
    fn deref(&self) -> &str {
        &self.wire
    }
}
impl PartialEq<&str> for BoundIdentity {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}
impl PartialEq<String> for BoundIdentity {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other
    }
}
impl std::fmt::Display for BoundIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.wire)
    }
}
impl serde::Serialize for BoundIdentity {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.wire)
    }
}
impl FromSql for BoundIdentity {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        StoredIdentity::column_result(value).map(Self::from_stored)
    }
}
impl ToSql for BoundIdentity {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.stored.to_sql()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_uses_sixteen_bytes_and_inspection_matches_the_public_value() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE ids(id BLOB PRIMARY KEY CHECK(typeof(id)='blob' AND length(id)=16));",
        )
        .unwrap();
        let wire = uuid::Uuid::new_v4().to_string();
        let id = StoredIdentity::generated(&wire).unwrap();
        db.execute("INSERT INTO ids VALUES (?1)", [&id]).unwrap();
        let (kind, bytes, decoded): (String, i64, StoredIdentity) = db
            .query_row("SELECT typeof(id),length(id),id FROM ids", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        assert_eq!((kind.as_str(), bytes), ("blob", 16));
        assert_eq!(decoded, id);
        assert_eq!(decoded.wire(), wire);
        let sql = format!("SELECT {} FROM ids", inspection_expression("id"));
        assert_eq!(
            db.query_row(&sql, [], |r| r.get::<_, String>(0)).unwrap(),
            wire
        );
    }

    #[test]
    fn historical_text_including_uuid_looking_values_remains_exact_text() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE ids(id);").unwrap();
        for raw in [
            "20261027-000000-00112233445566778899aabbccddeeff",
            "550e8400-e29b-41d4-a716-446655440000",
            "历史\0opaque",
        ] {
            db.execute("DELETE FROM ids", []).unwrap();
            db.execute("INSERT INTO ids VALUES (?1)", [raw]).unwrap();
            let stored: StoredIdentity = db
                .query_row("SELECT id FROM ids", [], |r| r.get(0))
                .unwrap();
            assert_eq!(stored, StoredIdentity::Legacy(raw.into()));
            assert_eq!(stored.wire(), raw);
            db.execute("UPDATE ids SET id=?1", [stored]).unwrap();
            assert_eq!(
                db.query_row("SELECT typeof(id) FROM ids", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "text"
            );
        }
    }

    #[test]
    fn session_lookup_preserves_legacy_uuid_text_and_resolves_native_keys() {
        let mut db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE sessions(id TEXT PRIMARY KEY,revision INTEGER NOT NULL DEFAULT 0); CREATE TABLE entries(session_id TEXT,entry_id TEXT,UNIQUE(session_id,entry_id)); CREATE TABLE compaction_operations(session_id TEXT,input_key TEXT,PRIMARY KEY(session_id,input_key)); CREATE TABLE session_tombstones(session_id TEXT PRIMARY KEY); CREATE TABLE legacy_imports(session_id TEXT PRIMARY KEY);").unwrap();
        db.execute_batch(include_str!("content/schema.sql"))
            .unwrap();
        let old = uuid::Uuid::new_v4().to_string();
        let new = uuid::Uuid::new_v4().to_string();
        db.execute("INSERT INTO sessions(id) VALUES (?1)", [&old])
            .unwrap();
        db.execute(
            "INSERT INTO sessions(id) VALUES (?1)",
            [StoredIdentity::generated(&new).unwrap()],
        )
        .unwrap();
        assert_eq!(
            session_key(&db, &old).unwrap(),
            StoredIdentity::Legacy(old.clone())
        );
        assert_eq!(
            session_key(&db, &new).unwrap(),
            StoredIdentity::generated(&new).unwrap()
        );
        let tx = db.transaction().unwrap();
        crate::session::content::enable(&tx, &old).unwrap();
        crate::session::content::enable(&tx, &new).unwrap();
        tx.commit().unwrap();
        assert_eq!(crate::session::content::format(&db, &old).unwrap(), 2);
        assert_eq!(crate::session::content::format(&db, &new).unwrap(), 2);
        let types: Vec<String> = db
            .prepare("SELECT typeof(session_id) FROM session_formats ORDER BY typeof(session_id)")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(types, vec!["blob", "text"]);
        db.execute("INSERT INTO sessions(id) VALUES (?1)", [&new])
            .unwrap();
        assert!(session_key(&db, &new).is_err());
    }

    #[test]
    fn malformed_binary_and_non_id_values_are_rejected() {
        for value in [
            ValueRef::Blob(&[0; 15]),
            ValueRef::Blob(&[0; 17]),
            ValueRef::Integer(7),
            ValueRef::Null,
        ] {
            assert!(StoredIdentity::column_result(value).is_err());
        }
    }
}
