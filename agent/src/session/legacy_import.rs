//! Read-only legacy importer. Each transcript and its event files either
//! commits completely or is recorded as skipped; storage failures propagate.

use super::sqlite_store::{insert_event_record, insert_transcript_entries, SqliteStore};
use anyhow::{bail, Context, Result};
use chrono::TimeZone;
use rusqlite::{params, OptionalExtension};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

struct Source {
    path: PathBuf,
    bytes: Vec<u8>,
    problem: Option<&'static str>,
}

fn source_unreadable(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::NotFound
            | std::io::ErrorKind::PermissionDenied
            | std::io::ErrorKind::InvalidData
            | std::io::ErrorKind::IsADirectory
            | std::io::ErrorKind::NotADirectory
    )
}

#[cfg(test)]
fn read_source(path: PathBuf) -> Result<Source> {
    read_source_cancellable(path, &crate::service::StartupCancellation::default())
}

fn read_bytes(path: &Path, cancellation: &crate::service::StartupCancellation) -> Result<Vec<u8>> {
    use std::io::Read;
    cancellation.check()?;
    let mut file = std::fs::File::open(path)?;
    let mut result = Vec::new();
    let mut buffer = [0; 65536];
    loop {
        cancellation.check()?;
        let count = file.read(&mut buffer)?;
        if count == 0 {
            return Ok(result);
        }
        result.extend_from_slice(&buffer[..count]);
    }
}

fn read_source_cancellable(
    path: PathBuf,
    cancellation: &crate::service::StartupCancellation,
) -> Result<Source> {
    match read_bytes(&path, cancellation) {
        Ok(bytes) => Ok(Source {
            path,
            bytes,
            problem: None,
        }),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(source_unreadable) =>
        {
            Ok(Source {
                path,
                bytes: Vec::new(),
                problem: Some("source_unreadable"),
            })
        }
        Err(error) => Err(error).context("read legacy source"),
    }
}

#[derive(Debug)]
struct InvalidSource {
    file: String,
    line: usize,
    kind: &'static str,
}

#[cfg(test)]
fn revalidate_sources(sources: &[Source]) -> std::result::Result<(), InvalidSource> {
    match revalidate_sources_cancellable(sources, &crate::service::StartupCancellation::default()) {
        Ok(()) => Ok(()),
        Err(ParseFailure::Invalid(error)) => Err(error),
        Err(ParseFailure::Cancelled) => unreachable!(),
    }
}

enum ParseFailure {
    Invalid(InvalidSource),
    Cancelled,
}
impl From<InvalidSource> for ParseFailure {
    fn from(error: InvalidSource) -> Self {
        Self::Invalid(error)
    }
}

fn revalidate_sources_cancellable(
    sources: &[Source],
    cancellation: &crate::service::StartupCancellation,
) -> std::result::Result<(), ParseFailure> {
    for source in sources.iter().filter(|source| source.problem.is_none()) {
        let kind = match read_bytes(&source.path, cancellation) {
            Ok(bytes) if bytes == source.bytes => continue,
            Ok(_) => "source_changed",
            Err(error) if error.is::<crate::service::StartupCancelled>() => {
                return Err(ParseFailure::Cancelled)
            }
            Err(_) => "source_unreadable",
        };
        return Err(InvalidSource {
            file: source
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            line: 0,
            kind,
        }
        .into());
    }
    Ok(())
}

#[cfg(test)]
#[test]
fn second_read_source_changes_are_classified_without_importing_stale_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.jsonl");
    std::fs::write(&path, "initial").unwrap();
    let source = read_source(path.clone()).unwrap();
    std::fs::write(&path, "changed").unwrap();
    assert_eq!(
        revalidate_sources(std::slice::from_ref(&source))
            .unwrap_err()
            .kind,
        "source_changed"
    );
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        revalidate_sources(&[source]).unwrap_err().kind,
        "source_unreadable"
    );
    assert_eq!(
        read_source(path).unwrap().problem,
        Some("source_unreadable")
    );
}

struct Import {
    entries: Vec<Value>,
    events: Vec<Value>,
    warnings: usize,
}

/// Migration diagnostics contain identifiers and controlled error categories,
/// never transcript text or raw decoder errors.
#[derive(Debug, serde::Serialize)]
pub struct ImportRecord {
    pub session_id: String,
    pub status: String,
    pub error_file: Option<String>,
    pub error_line: Option<i64>,
    pub error_kind: Option<String>,
    pub warnings: i64,
}

impl SqliteStore {
    pub fn import_records(&self) -> Result<Vec<ImportRecord>> {
        self.db.call(|db| {
            let mut statement = db.prepare("SELECT session_id,status,error_file,error_line,error_kind,warnings FROM legacy_imports ORDER BY session_id")?;
            let records = statement.query_map([], |row| Ok(ImportRecord {
                session_id: row.get(0)?, status: row.get(1)?, error_file: row.get(2)?,
                error_line: row.get(3)?, error_kind: row.get(4)?, warnings: row.get(5)?,
            }))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(records)
        })
    }
    /// Import a legacy directory once, or explicitly retry one skipped session.
    /// The caller must exclude concurrent legacy writers for the entire call.
    pub fn import_legacy(&self, directory: &Path, retry: Option<&str>) -> Result<()> {
        self.import_legacy_cancellable(
            directory,
            retry,
            &crate::service::StartupCancellation::default(),
        )
    }

    pub(crate) fn import_legacy_cancellable(
        &self,
        directory: &Path,
        retry: Option<&str>,
        cancellation: &crate::service::StartupCancellation,
    ) -> Result<()> {
        cancellation.check()?;
        let retry = retry.map(str::to_owned);
        if let Some(id) = retry.clone() {
            let status = self.db.call(move |db| {
                Ok(db
                    .query_row(
                        "SELECT status FROM legacy_imports WHERE session_id=?1",
                        [id],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()?)
            })?;
            if status.as_deref() != Some("skipped") {
                bail!("only a skipped legacy session can be retried");
            }
        }
        // Discover newly restored files on every startup. Per-session import
        // records below still fence successful, skipped and deleted sessions;
        // a global completion bit must not hide a backup restored later.
        let mut paths = Vec::new();
        match std::fs::read_dir(directory) {
            Ok(files) => {
                for file in files {
                    cancellation.check()?;
                    let path = file?.path();
                    if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                        paths.push(path);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("enumerate legacy sessions"),
        }
        paths.sort();
        if let Some(id) = retry.as_ref() {
            if !paths
                .iter()
                .any(|path| path.file_stem().and_then(|s| s.to_str()) == Some(id))
            {
                bail!("legacy retry source is missing");
            }
        }
        let candidates = paths.len();
        if candidates > 0 {
            tracing::info!(
                candidates,
                "Agent startup: scanning for legacy sessions to import"
            );
        }
        for (index, path) in paths.into_iter().enumerate() {
            cancellation.check()?;
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .context("invalid legacy session filename")?
                .to_owned();
            if retry.as_ref().is_some_and(|wanted| wanted != &id) {
                continue;
            }
            let key = id.clone();
            let status = self.db.call(move |db| {
                Ok(db
                    .query_row(
                        "SELECT status FROM legacy_imports WHERE session_id=?1",
                        [key],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()?)
            })?;
            if status
                .as_deref()
                .is_some_and(|s| s != "skipped" || retry.is_none())
            {
                continue;
            }
            // One line per session: the only visible signal while a large
            // history is read from slow storage, so it is deliberately
            // per-session and not a periodic summary.
            tracing::info!(
                session = %id,
                index = index + 1,
                candidates,
                "Agent startup: importing legacy session"
            );
            let mut sources = vec![read_source_cancellable(path, cancellation)?];
            let events_root = if directory.file_name().and_then(|s| s.to_str()) == Some("sessions")
            {
                directory.parent().unwrap_or(directory).join("run-events")
            } else {
                directory.join(".run-events")
            };
            match std::fs::read_dir(events_root.join(&id)) {
                Ok(files) => {
                    let mut events = Vec::new();
                    for file in files {
                        cancellation.check()?;
                        let path = file?.path();
                        if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                            events.push(path);
                        }
                    }
                    events.sort();
                    for path in events {
                        sources.push(read_source_cancellable(path, cancellation)?);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) if source_unreadable(&error) => sources.push(Source {
                    path: events_root.join(&id),
                    bytes: Vec::new(),
                    problem: Some("events_directory_unreadable"),
                }),
                Err(error) => return Err(error).context("enumerate legacy events"),
            }
            let mut hash = Sha256::new();
            for source in &sources {
                cancellation.check()?;
                hash.update(
                    source
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .as_encoded_bytes(),
                );
                hash.update(source.problem.unwrap_or_default().as_bytes());
                hash.update((source.bytes.len() as u64).to_le_bytes());
                for chunk in source.bytes.chunks(65536) {
                    cancellation.check()?;
                    hash.update(chunk);
                }
            }
            let fingerprint = format!("{:x}", hash.finalize());
            // Source races are per-session failures, like parse failures; they
            // must neither import an inconsistent snapshot nor prevent all
            // unrelated sessions from starting. Destination storage errors still
            // propagate from the transaction below.
            let parsed = match revalidate_sources_cancellable(&sources, cancellation)
                .and_then(|()| parse_sources(&sources, &id, cancellation))
            {
                Ok(import) => Ok(import),
                Err(ParseFailure::Invalid(error)) => Err(error),
                Err(ParseFailure::Cancelled) => return Err(crate::service::StartupCancelled.into()),
            };
            cancellation.check()?;
            let cancellation = cancellation.clone();
            self.db.call(move |db| {
                // Reads the destination before it writes: take the write lock at
                // BEGIN, or a concurrent connection's commit invalidates this
                // snapshot and the upgrade fails instantly (SQLITE_BUSY_SNAPSHOT).
                cancellation.check()?;
                let tx = crate::session::database::begin_write_transaction(db)?;
                cancellation.check()?;
                let parsed = if tx.query_row("SELECT id FROM sessions WHERE id=?1", [&id], |r| r.get::<_, String>(0)).optional()?.is_some() {
                    Err(InvalidSource { file: "transcript".into(), line: 0, kind: "existing_sqlite_session" })
                } else { parsed };
                match parsed {
                    Ok(import) => {
                        tx.execute("INSERT INTO sessions(id) VALUES (?1)", [&id])?;
                        insert_transcript_entries(&tx, &id, import.entries)?;
                        for event in import.events { cancellation.check()?; insert_event_record(&tx, &id, event)?; }
                        tx.execute("UPDATE runs SET status='interrupted' WHERE session_id=?1 AND status='running'", [&id])?;
                        tx.execute("INSERT INTO legacy_imports(session_id,status,fingerprint,warnings) VALUES (?1,'imported',?2,?3)
                            ON CONFLICT(session_id) DO UPDATE SET status='imported',fingerprint=excluded.fingerprint,warnings=excluded.warnings,error_file=NULL,error_line=NULL,error_kind=NULL",
                            params![id,fingerprint,import.warnings as i64])?;
                    },
                    Err(error) => {
                        tx.execute("INSERT INTO legacy_imports(session_id,status,fingerprint,error_file,error_line,error_kind) VALUES (?1,'skipped',?2,?3,?4,?5)
                            ON CONFLICT(session_id) DO UPDATE SET status='skipped',fingerprint=excluded.fingerprint,error_file=excluded.error_file,error_line=excluded.error_line,error_kind=excluded.error_kind",
                            params![id,fingerprint,error.file,error.line as i64,error.kind])?;
                        tracing::warn!(session_id=%id, line=error.line, kind=error.kind, "legacy session skipped");
                    }
                }
                cancellation.check()?;
                tx.commit()?;
                Ok(())
            })?;
        }
        if candidates > 0 {
            tracing::info!(candidates, "Agent startup: legacy session scan complete");
        }
        cancellation.check()?;
        let cancellation = cancellation.clone();
        self.db.call(move |db| {
            cancellation.check()?;
            db.execute(
                "INSERT OR IGNORE INTO storage_meta VALUES ('legacy_complete','1')",
                [],
            )?;
            // A first import can temporarily grow the WAL to the size of the
            // complete legacy event journal. The import is fully committed at
            // this point, so reset and truncate that high-water allocation
            // before the Agent begins serving requests.
            db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
            Ok(())
        })
    }
}

fn parse_sources(
    sources: &[Source],
    session: &str,
    cancellation: &crate::service::StartupCancellation,
) -> std::result::Result<Import, ParseFailure> {
    let mut result = Import {
        entries: Vec::new(),
        events: Vec::new(),
        warnings: 0,
    };
    let mut entry_ids = HashMap::<String, Value>::new();
    let mut event_ids = HashMap::<String, Value>::new();
    let mut wire_ids = HashMap::<String, String>::new();
    for (source_index, source) in sources.iter().enumerate() {
        cancellation.check().map_err(|_| ParseFailure::Cancelled)?;
        let file = source
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if let Some(kind) = source.problem {
            return Err(InvalidSource {
                file,
                line: 0,
                kind,
            }
            .into());
        }
        let lines: Vec<_> = source.bytes.split(|byte| *byte == b'\n').collect();
        for (index, bytes) in lines.iter().enumerate() {
            cancellation.check().map_err(|_| ParseFailure::Cancelled)?;
            let invalid = |kind| InvalidSource {
                file: file.clone(),
                line: index + 1,
                kind,
            };
            if bytes.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let mut value: Value = match serde_json::from_slice(bytes) {
                Ok(value) => value,
                Err(error)
                    if error.is_eof()
                        && index + 1 == lines.len()
                        && !source.bytes.ends_with(b"\n") =>
                {
                    result.warnings += 1;
                    continue;
                }
                Err(_) => return Err(invalid("invalid_json").into()),
            };
            if source_index == 0 {
                let id = value
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| invalid("missing_entry_id"))?
                    .to_owned();
                // Validate without the lenient timestamp decoder (which logs
                // source data and substitutes wall-clock time).
                if value.get("timestamp").is_none() {
                    let modified = source
                        .path
                        .metadata()
                        .and_then(|m| m.modified())
                        .map_err(|_| invalid("missing_timestamp"))?;
                    value["timestamp"] =
                        Value::String(chrono::DateTime::<chrono::Utc>::from(modified).to_rfc3339());
                    result.warnings += 1;
                }
                let timestamp = value
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("invalid_timestamp"))?;
                if chrono::DateTime::parse_from_rfc3339(timestamp).is_err() {
                    let naive =
                        chrono::NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%dT%H:%M:%S%.f")
                            .or_else(|_| {
                                chrono::NaiveDateTime::parse_from_str(
                                    timestamp,
                                    "%Y-%m-%d %H:%M:%S%.f",
                                )
                            })
                            .map_err(|_| invalid("invalid_timestamp"))?;
                    let local = chrono::Local
                        .from_local_datetime(&naive)
                        .single()
                        .ok_or_else(|| invalid("ambiguous_timestamp"))?;
                    value["timestamp"] = Value::String(local.to_rfc3339());
                    result.warnings += 1;
                }
                serde_json::from_value::<super::SessionEntry>(value.clone())
                    .map_err(|_| invalid("invalid_entry"))?;
                if value
                    .get("meta")
                    .is_some_and(|meta| !meta.is_null() && !meta.is_object())
                    || value
                        .get("meta")
                        .and_then(|meta| meta.get("run_id"))
                        .is_some_and(|run| !run.is_null() && !run.is_string())
                {
                    return Err(invalid("invalid_entry_metadata").into());
                }
                if matches!(value["type"].as_str(), Some("run_started" | "run_terminal")) {
                    if value["content"]["run_id"]
                        .as_str()
                        .is_none_or(str::is_empty)
                    {
                        return Err(invalid("missing_run_identity").into());
                    }
                    if value["type"] == "run_terminal"
                        && value["content"]["state"].as_str().is_none()
                    {
                        return Err(invalid("missing_terminal_state").into());
                    }
                }
                if let Some(previous) = entry_ids.insert(id, value.clone()) {
                    if previous != value {
                        return Err(invalid("conflicting_entry_id").into());
                    }
                    result.warnings += 1;
                    continue;
                }
                result.entries.push(value);
            } else {
                let event: crate::rpc::SseEvent =
                    serde_json::from_value(value.clone()).map_err(|_| invalid("invalid_event"))?;
                let expected_run = source
                    .path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default();
                let expected_run = if expected_run == "_session" {
                    ""
                } else {
                    expected_run
                };
                if event.run_id != expected_run
                    || (!event.session_id.is_empty() && event.session_id != session)
                {
                    return Err(invalid("event_identity_mismatch").into());
                }
                let session_scoped = event.run_id.is_empty() && event.session_idx >= 0;
                let idx = if session_scoped {
                    event.session_idx
                } else {
                    event.idx
                };
                if idx < 0 {
                    return Err(invalid("invalid_event_sequence").into());
                }
                let identity = if session_scoped {
                    format!("{session}:session:{idx}")
                } else {
                    format!("{session}:{}:{}:{idx}", event.run_id, event.epoch)
                };
                value["session_id"] = Value::String(session.to_owned());
                if event.event_id.is_empty() {
                    value["event_id"] = Value::String(identity.clone());
                }
                let wire_id = value["event_id"]
                    .as_str()
                    .ok_or_else(|| invalid("invalid_event_id"))?
                    .to_owned();
                if wire_ids
                    .insert(wire_id, identity.clone())
                    .is_some_and(|previous| previous != identity)
                {
                    return Err(invalid("conflicting_event_id").into());
                }
                if let Some(previous) = event_ids.insert(identity, value.clone()) {
                    if previous != value {
                        return Err(invalid("conflicting_event_identity").into());
                    }
                    result.warnings += 1;
                    continue;
                }
                result.events.push(value);
            }
        }
    }
    if result.entries.is_empty() {
        return Err(InvalidSource {
            file: "transcript".into(),
            line: 0,
            kind: "empty_session",
        }
        .into());
    }
    let positions: HashMap<_, _> = result
        .entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| entry["id"].as_str().map(|id| (id, index)))
        .collect();
    for (index, entry) in result.entries.iter().enumerate() {
        cancellation.check().map_err(|_| ParseFailure::Cancelled)?;
        // Only the current schema is validated. A row from a retired schema is imported as
        // an inert journal entry: nothing reads it, so a dangling range in it cannot affect
        // the child session's prompt.
        if entry["type"] == "compaction" && entry["content"]["schema_version"].as_u64() == Some(3) {
            let content = &entry["content"];
            let start = content["covered_from_entry_id"]
                .as_str()
                .and_then(|id| positions.get(id));
            let end = content["cutoff_entry_id"]
                .as_str()
                .and_then(|id| positions.get(id));
            let protected_valid = content
                .get("protected_entry_ids")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|ids| {
                    ids.iter().all(|id| {
                        id.as_str()
                            .and_then(|id| positions.get(id))
                            .is_some_and(|position| {
                                end.is_some_and(|end| position <= end)
                                    && matches!(
                                        result.entries[*position]["type"].as_str(),
                                        Some("user" | "assistant")
                                    )
                            })
                    })
                });
            if !protected_valid
                || !matches!((start, end), (Some(start), Some(end)) if start <= end && *end < index)
            {
                return Err(InvalidSource {
                    file: "transcript".into(),
                    line: 0,
                    kind: "dangling_checkpoint",
                }
                .into());
            }
        }
    }
    Ok(result)
}

/// Every import-level failure mode that a legacy directory can present, one
/// case per rejection kind. The legacy reader is the only path that ingests
/// foreign bytes, so its error classification is what keeps a malformed old
/// session from blocking a whole migration.
#[cfg(test)]
mod import_paths {
    use super::*;

    fn store_in(dir: &Path) -> SqliteStore {
        SqliteStore::open(&dir.join("agent.db")).unwrap()
    }

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn discovers_backups_restored_after_initial_start_without_resurrecting_deleted_sessions() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        let store = store_in(temp.path());
        store.import_legacy(&sessions, None).unwrap();
        write(&sessions, "old.jsonl", &entry_line("old-user"));
        store.import_legacy(&sessions, None).unwrap();
        assert!(store.contains("old").unwrap());
        store.delete("old").unwrap();
        write(&sessions, "later.jsonl", &entry_line("later-user"));
        store.import_legacy(&sessions, None).unwrap();
        assert!(!store.contains("old").unwrap());
        assert!(store.contains("later").unwrap());
        assert_eq!(store.import_records().unwrap().len(), 2);
    }

    #[test]
    fn cancelled_initialization_does_not_create_a_database_or_mark_sources_invalid() {
        let root = tempfile::tempdir().unwrap();
        let cancellation = crate::service::StartupCancellation::default();
        cancellation.cancel();
        let database = root.path().join("cancelled.db");
        let error = SqliteStore::open_cancellable(&database, cancellation.clone())
            .err()
            .unwrap();
        assert!(error.is::<crate::service::StartupCancelled>());
        assert!(!database.exists());
        let source = Source {
            path: root.path().join("bad.jsonl"),
            bytes: b"invalid".to_vec(),
            problem: None,
        };
        assert!(matches!(
            parse_sources(&[source], "session", &cancellation),
            Err(ParseFailure::Cancelled)
        ));
    }

    /// A path that cannot be addressed at all (an interior NUL) fails with an
    /// io error that is neither missing nor unreadable. It is an operator
    /// error about the source list, not a damaged session, so it must
    /// propagate with its own context rather than become `source_unreadable`.
    #[test]
    fn an_unaddressable_source_path_reports_the_read_failure() {
        let error = read_source(PathBuf::from("bad\0name"))
            .map_err(|error| format!("{error:#}"))
            .err()
            .unwrap();
        assert!(
            format!("{error:#}").contains("read legacy source"),
            "{error:#}"
        );
        assert!(
            !format!("{error:#}").contains('\0'),
            "the path must not be echoed raw: {error:#}"
        );
    }

    /// Revalidating nothing is a clean no-op, which is the state of a home with
    /// no legacy sessions to check.
    #[test]
    fn an_empty_source_list_revalidates_cleanly() {
        assert!(revalidate_sources(&[]).is_ok());
    }

    /// A cancelled startup must be reported as cancellation, not mistaken for a
    /// source that changed while it was being read.
    #[test]
    fn a_cancelled_revalidation_reports_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jsonl");
        std::fs::write(&path, "initial").unwrap();
        let source = read_source(path).unwrap();
        let cancellation = crate::service::StartupCancellation::default();
        cancellation.cancel();
        assert!(matches!(
            revalidate_sources_cancellable(std::slice::from_ref(&source), &cancellation),
            Err(ParseFailure::Cancelled)
        ));
    }

    /// A run-event directory that exists but cannot be enumerated (a
    /// self-referential link yields ELOOP, which is neither "absent" nor an
    /// unreadable-source category) is an operator error: it must surface with
    /// its context instead of being swallowed as a session with no events.
    #[cfg(unix)]
    #[test]
    fn a_run_event_directory_that_cannot_be_enumerated_reports_its_context() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        write(&sessions, "s.jsonl", &entry_line("s"));
        let events = temp.path().join("run-events");
        std::fs::create_dir_all(&events).unwrap();
        std::os::unix::fs::symlink("s", events.join("s")).unwrap();
        let store = store_in(temp.path());
        let error = store
            .import_legacy(&sessions, None)
            .map_err(|error| format!("{error:#}"))
            .err()
            .unwrap();
        assert!(error.contains("enumerate legacy events"), "{error}");
    }

    fn entry_line(id: &str) -> String {
        format!(
            "{{\"id\":\"{id}\",\"type\":\"user\",\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"question\"}}],\"timestamp\":\"2026-01-01T00:00:00Z\"}}\n"
        )
    }

    fn event_line(run: &str, session_idx: i64, idx: i64) -> String {
        format!(
            "{{\"event_type\":\"text_chunk\",\"data\":\"{{\\\"text\\\":\\\"synthetic\\\"}}\",\"session_id\":\"\",\"run_id\":\"{run}\",\"epoch\":1,\"idx\":{idx},\"session_idx\":{session_idx},\"run_sequence\":1,\"timestamp\":\"2026-01-01T00:00:00Z\"}}\n"
        )
    }

    fn checkpoint_line(id: &str, from: &str, cutoff: &str, protected: &str) -> String {
        format!(
            "{{\"id\":\"{id}\",\"type\":\"compaction\",\"content\":{{\"schema_version\":3,\"covered_from_entry_id\":\"{from}\",\"cutoff_entry_id\":\"{cutoff}\",\"protected_entry_ids\":{protected}}},\"timestamp\":\"2026-01-01T00:00:00Z\"}}\n"
        )
    }

    /// Import one synthetic legacy session (and optional run-event file) into a
    /// fresh store. The store is returned even when the session was skipped, so
    /// the caller can assert the recorded rejection kind.
    fn import_case(
        entries: &str,
        events: Option<(&str, &str)>,
    ) -> (tempfile::TempDir, SqliteStore) {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        write(&sessions, "s.jsonl", entries);
        if let Some((run, body)) = events {
            write(
                &temp.path().join("run-events/s"),
                &format!("{run}.jsonl"),
                body,
            );
        }
        let store = store_in(temp.path());
        store.import_legacy(&sessions, None).unwrap();
        (temp, store)
    }

    fn kind_of(store: &SqliteStore, id: &str) -> Option<String> {
        store
            .import_records()
            .unwrap()
            .into_iter()
            .find(|record| record.session_id == id)
            .and_then(|record| record.error_kind)
    }

    #[test]
    fn unreadable_sources_and_missing_retry_targets_are_reported_not_imported() {
        let temp = tempfile::tempdir().unwrap();
        let store = store_in(temp.path());
        // A file where the legacy root should be: read_dir fails with something
        // other than NotFound, which is an operator error, not "no sessions yet".
        let not_a_directory = temp.path().join("sessions.jsonl");
        std::fs::write(&not_a_directory, entry_line("e")).unwrap();
        let error = store
            .import_legacy(&not_a_directory, None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("enumerate legacy sessions"), "{error}");

        // A directory named like a transcript is unreadable as bytes, so the
        // session is skipped with that category instead of aborting the run.
        let legacy = temp.path().join("legacy");
        write(&legacy, "good.jsonl", &entry_line("good-entry"));
        std::fs::create_dir_all(legacy.join("bad.jsonl")).unwrap();
        store.import_legacy(&legacy, None).unwrap();
        assert_eq!(store.ids(false).unwrap(), ["good"]);
        assert_eq!(kind_of(&store, "bad").as_deref(), Some("source_unreadable"));

        // Retrying a session whose source file disappeared is rejected. A
        // fresh store is needed: the import above already marked this home
        // complete, and a complete home short-circuits an automatic re-import.
        let retry_store = store_in(&temp.path().join("retry-home"));
        let retry_dir = temp.path().join("retry");
        write(&retry_dir, "s.jsonl", "{broken}\n");
        retry_store.import_legacy(&retry_dir, None).unwrap();
        assert_eq!(kind_of(&retry_store, "s").as_deref(), Some("invalid_json"));
        std::fs::remove_file(retry_dir.join("s.jsonl")).unwrap();
        let error = retry_store
            .import_legacy(&retry_dir, Some("s"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("legacy retry source is missing"), "{error}");
    }

    #[test]
    fn a_second_import_skips_committed_sessions_and_finishes_the_rest() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("sessions");
        write(&legacy, "a.jsonl", &entry_line("a"));
        write(&legacy, "z.jsonl", &entry_line("z"));
        let path = temp.path().join("agent.db");
        let store = SqliteStore::open(&path).unwrap();
        let trigger = rusqlite::Connection::open(&path).unwrap();
        trigger
            .execute_batch(
                "CREATE TRIGGER fail_z BEFORE INSERT ON sessions WHEN NEW.id='z' \
                 BEGIN SELECT RAISE(ABORT,'synthetic storage failure'); END;",
            )
            .unwrap();
        assert!(store.import_legacy(&legacy, None).is_err());
        assert_eq!(
            store.ids(false).unwrap(),
            ["a"],
            "a committed, z rolled back"
        );
        assert_eq!(kind_of(&store, "z"), None, "the failed row is not recorded");

        trigger.execute_batch("DROP TRIGGER fail_z;").unwrap();
        // The retry re-walks the directory: the already-imported session is
        // skipped in place, and the run reaches its completion marker.
        store.import_legacy(&legacy, None).unwrap();
        assert_eq!(store.ids(false).unwrap(), ["a", "z"]);
    }

    #[test]
    fn a_legacy_file_never_overwrites_an_existing_sqlite_session() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("sessions");
        write(&legacy, "s.jsonl", &entry_line("legacy-entry"));
        let store = store_in(temp.path());
        store
            .replace(
                "s",
                vec![serde_json::json!({
                    "id":"sqlite-entry","type":"user","role":"user",
                    "content":[{"type":"text","text":"kept"}],
                    "timestamp":"2026-01-01T00:00:00Z"
                })],
            )
            .unwrap();
        store.import_legacy(&legacy, None).unwrap();
        assert_eq!(
            kind_of(&store, "s").as_deref(),
            Some("existing_sqlite_session")
        );
        assert_eq!(store.entries("s").unwrap()[0]["id"], "sqlite-entry");
    }

    #[test]
    fn entry_shapes_are_rejected_with_their_reported_kind() {
        let duplicate_id = format!("{}{}", entry_line("e"), entry_line("e"));
        let conflicting_id = format!(
            "{}{}",
            "{\"id\":\"e\",\"type\":\"user\",\"content\":\"one\",\"timestamp\":\"2026-01-01T00:00:00Z\"}\n",
            "{\"id\":\"e\",\"type\":\"user\",\"content\":\"two\",\"timestamp\":\"2026-01-01T00:00:00Z\"}\n"
        );
        let cases = [
            (
                "missing_entry_id",
                "{\"type\":\"user\",\"timestamp\":\"2026-01-01T00:00:00Z\"}\n".to_string(),
            ),
            (
                "invalid_entry_metadata",
                "{\"id\":\"e\",\"type\":\"user\",\"meta\":7,\"timestamp\":\"2026-01-01T00:00:00Z\"}\n"
                    .to_string(),
            ),
            (
                "missing_run_identity",
                "{\"id\":\"e\",\"type\":\"run_started\",\"content\":{},\"timestamp\":\"2026-01-01T00:00:00Z\"}\n"
                    .to_string(),
            ),
            (
                "missing_terminal_state",
                "{\"id\":\"e\",\"type\":\"run_terminal\",\"content\":{\"run_id\":\"r\"},\"timestamp\":\"2026-01-01T00:00:00Z\"}\n"
                    .to_string(),
            ),
            ("conflicting_entry_id", conflicting_id),
            ("empty_session", "\n   \n".to_string()),
        ];
        for (expected, body) in cases {
            let (_temp, store) = import_case(&body, None);
            assert_eq!(
                kind_of(&store, "s").as_deref(),
                Some(expected),
                "body: {body}"
            );
        }

        // An identical repeated line is only a warning: the session imports.
        let (_temp, store) = import_case(&duplicate_id, None);
        assert_eq!(store.entries("s").unwrap().len(), 1);
        assert_eq!(store.import_records().unwrap()[0].warnings, 1);
    }

    #[test]
    fn events_are_validated_for_identity_sequence_and_duplicates() {
        let session_scoped = event_line("", 3, 0);
        let (_temp, store) = import_case(&entry_line("e"), Some(("_session", &session_scoped)));
        assert_eq!(store.import_records().unwrap()[0].status, "imported");
        let stored = store.events("s", "").unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0]["event_id"], "s:session:3");

        let (_temp, store) = import_case(
            &entry_line("e"),
            Some(("_session", &event_line("", -1, -2))),
        );
        assert_eq!(
            kind_of(&store, "s").as_deref(),
            Some("invalid_event_sequence")
        );

        let (_temp, store) =
            import_case(&entry_line("e"), Some(("r", &event_line("other", -1, 0))));
        assert_eq!(
            kind_of(&store, "s").as_deref(),
            Some("event_identity_mismatch")
        );

        // Two events that claim the same wire id for different identities.
        let mut first = serde_json::from_str::<Value>(&session_scoped).unwrap();
        let mut second = serde_json::from_str::<Value>(&event_line("", 4, 0)).unwrap();
        first["event_id"] = Value::String("same".into());
        second["event_id"] = Value::String("same".into());
        let body = format!("{first}\n{second}\n");
        let (_temp, store) = import_case(&entry_line("e"), Some(("_session", &body)));
        assert_eq!(
            kind_of(&store, "s").as_deref(),
            Some("conflicting_event_id")
        );

        // Same identity, different payload.
        let mut conflicting = serde_json::from_str::<Value>(&event_line("r", -1, 0)).unwrap();
        conflicting["data"] = Value::String("different".into());
        let body = format!("{}{}\n", event_line("r", -1, 0), conflicting);
        let (_temp, store) = import_case(&entry_line("e"), Some(("r", &body)));
        assert_eq!(
            kind_of(&store, "s").as_deref(),
            Some("conflicting_event_identity")
        );

        // An identical repeated event is only a warning.
        let body = format!("{}{}", event_line("r", -1, 0), event_line("r", -1, 0));
        let (_temp, store) = import_case(&entry_line("e"), Some(("r", &body)));
        assert_eq!(store.events("s", "r").unwrap().len(), 1);
        assert_eq!(store.import_records().unwrap()[0].warnings, 1);
    }

    #[test]
    fn timestamp_repairs_are_warnings_and_unparsable_time_is_a_skip() {
        // Space-separated local time: repaired, and recorded as a warning.
        let body =
            "{\"id\":\"e\",\"type\":\"user\",\"content\":\"q\",\"timestamp\":\"2026-06-15 12:00:00\"}\n";
        let (_temp, store) = import_case(body, None);
        assert_eq!(store.entries("s").unwrap().len(), 1);
        assert_eq!(store.import_records().unwrap()[0].warnings, 1);

        // Missing timestamp: taken from the transcript's mtime.
        let body = "{\"id\":\"e\",\"type\":\"user\",\"content\":\"q\"}\n";
        let (_temp, store) = import_case(body, None);
        assert_eq!(store.entries("s").unwrap().len(), 1);
        assert_eq!(store.import_records().unwrap()[0].warnings, 1);

        // An unparsable timestamp cannot be repaired.
        let body =
            "{\"id\":\"e\",\"type\":\"user\",\"content\":\"q\",\"timestamp\":\"yesterday\"}\n";
        let (_temp, store) = import_case(body, None);
        assert_eq!(kind_of(&store, "s").as_deref(), Some("invalid_timestamp"));
    }

    #[test]
    fn checkpoint_ranges_must_resolve_to_real_user_and_assistant_entries() {
        // A protected id that resolves to a run marker is not a protected message.
        let body = format!(
            "{}{}{}",
            entry_line("e"),
            "{\"id\":\"started\",\"type\":\"run_started\",\"content\":{\"run_id\":\"r\"},\"timestamp\":\"2026-01-01T00:00:00Z\"}\n",
            checkpoint_line("cp", "e", "started", "[\"started\"]"),
        );
        let (_temp, store) = import_case(&body, None);
        assert_eq!(kind_of(&store, "s").as_deref(), Some("dangling_checkpoint"));

        // A fully resolvable range imports, checkpoint included.
        let body = format!(
            "{}{}",
            entry_line("e"),
            checkpoint_line("cp", "e", "e", "[\"e\"]")
        );
        let (_temp, store) = import_case(&body, None);
        assert_eq!(store.ids(false).unwrap(), ["s"]);
        assert_eq!(store.entries("s").unwrap().len(), 2);
    }
}
