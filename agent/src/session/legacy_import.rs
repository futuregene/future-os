//! Read-only legacy importer. Each transcript and its event files either
//! commits completely or is recorded as skipped; storage failures propagate.

use super::sqlite_store::{insert_entries, insert_event, SqliteStore};
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

fn read_source(path: PathBuf) -> Result<Source> {
    match std::fs::read(&path) {
        Ok(bytes) => Ok(Source {
            path,
            bytes,
            problem: None,
        }),
        Err(error) if source_unreadable(&error) => Ok(Source {
            path,
            bytes: Vec::new(),
            problem: Some("source_unreadable"),
        }),
        Err(error) => Err(error).context("read legacy source"),
    }
}

#[derive(Debug)]
struct InvalidSource {
    file: String,
    line: usize,
    kind: &'static str,
}

fn revalidate_sources(sources: &[Source]) -> std::result::Result<(), InvalidSource> {
    for source in sources.iter().filter(|source| source.problem.is_none()) {
        let kind = match std::fs::read(&source.path) {
            Ok(bytes) if bytes == source.bytes => continue,
            Ok(_) => "source_changed",
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
        });
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
        let complete = self.db.call(|db| {
            Ok(db
                .query_row(
                    "SELECT value FROM storage_meta WHERE key='legacy_complete'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
                .is_some())
        })?;
        if complete && retry.is_none() {
            return Ok(());
        }
        let mut paths = Vec::new();
        match std::fs::read_dir(directory) {
            Ok(files) => {
                for file in files {
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
        for path in paths {
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
            let mut sources = vec![read_source(path)?];
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
                        let path = file?.path();
                        if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                            events.push(path);
                        }
                    }
                    events.sort();
                    for path in events {
                        sources.push(read_source(path)?);
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
                hash.update(
                    source
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .as_encoded_bytes(),
                );
                hash.update(source.problem.unwrap_or_default().as_bytes());
                hash.update((source.bytes.len() as u64).to_le_bytes());
                hash.update(&source.bytes);
            }
            let fingerprint = format!("{:x}", hash.finalize());
            // Source races are per-session failures, like parse failures; they
            // must neither import an inconsistent snapshot nor prevent all
            // unrelated sessions from starting. Destination storage errors still
            // propagate from the transaction below.
            let parsed = revalidate_sources(&sources).and_then(|()| parse_sources(&sources, &id));
            self.db.call(move |db| {
                let tx = db.transaction()?;
                let parsed = if tx.query_row("SELECT id FROM sessions WHERE id=?1", [&id], |r| r.get::<_, String>(0)).optional()?.is_some() {
                    Err(InvalidSource { file: "transcript".into(), line: 0, kind: "existing_sqlite_session" })
                } else { parsed };
                match parsed {
                    Ok(import) => {
                        tx.execute("INSERT INTO sessions(id) VALUES (?1)", [&id])?;
                        insert_entries(&tx, &id, import.entries)?;
                        for event in import.events { insert_event(&tx, &id, event)?; }
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
                tx.commit()?;
                Ok(())
            })?;
        }
        self.db.call(|db| {
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

fn parse_sources(sources: &[Source], session: &str) -> std::result::Result<Import, InvalidSource> {
    let mut result = Import {
        entries: Vec::new(),
        events: Vec::new(),
        warnings: 0,
    };
    let mut entry_ids = HashMap::<String, Value>::new();
    let mut event_ids = HashMap::<String, Value>::new();
    let mut wire_ids = HashMap::<String, String>::new();
    for (source_index, source) in sources.iter().enumerate() {
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
            });
        }
        let lines: Vec<_> = source.bytes.split(|byte| *byte == b'\n').collect();
        for (index, bytes) in lines.iter().enumerate() {
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
                Err(_) => return Err(invalid("invalid_json")),
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
                    return Err(invalid("invalid_entry_metadata"));
                }
                if matches!(value["type"].as_str(), Some("run_started" | "run_terminal")) {
                    if value["content"]["run_id"]
                        .as_str()
                        .is_none_or(str::is_empty)
                    {
                        return Err(invalid("missing_run_identity"));
                    }
                    if value["type"] == "run_terminal"
                        && value["content"]["state"].as_str().is_none()
                    {
                        return Err(invalid("missing_terminal_state"));
                    }
                }
                if let Some(previous) = entry_ids.insert(id, value.clone()) {
                    if previous != value {
                        return Err(invalid("conflicting_entry_id"));
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
                    return Err(invalid("event_identity_mismatch"));
                }
                let session_scoped = event.run_id.is_empty() && event.session_idx >= 0;
                let idx = if session_scoped {
                    event.session_idx
                } else {
                    event.idx
                };
                if idx < 0 {
                    return Err(invalid("invalid_event_sequence"));
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
                    return Err(invalid("conflicting_event_id"));
                }
                if let Some(previous) = event_ids.insert(identity, value.clone()) {
                    if previous != value {
                        return Err(invalid("conflicting_event_identity"));
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
        });
    }
    let positions: HashMap<_, _> = result
        .entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| entry["id"].as_str().map(|id| (id, index)))
        .collect();
    for (index, entry) in result.entries.iter().enumerate() {
        if entry["type"] == "compaction" && entry["content"]["schema_version"] == 2 {
            let content = &entry["content"];
            let start = content["covered_from_entry_id"]
                .as_str()
                .and_then(|id| positions.get(id));
            let end = content["cutoff_entry_id"]
                .as_str()
                .and_then(|id| positions.get(id));
            if !matches!((start, end), (Some(start), Some(end)) if start <= end && *end < index) {
                return Err(InvalidSource {
                    file: "transcript".into(),
                    line: 0,
                    kind: "dangling_checkpoint",
                });
            }
        }
    }
    Ok(result)
}
