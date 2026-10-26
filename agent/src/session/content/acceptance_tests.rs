//! Explicit final-pass workloads. Never use a user's conversation/database.
//! Run with `cargo test -p future-agent acceptance_tests -- --ignored --nocapture`.
use super::{journal::Writer, stream_entry::StreamEntry, *};
use crate::{
    session::sqlite_store::SqliteStore,
    types::{AgentMessage, ContentBlock},
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, Write},
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

fn event(idx: i64, kind: &str, data: Value) -> Value {
    json!({"session_id":"s","run_id":"r","epoch":1,"idx":idx,"session_idx":-1,"run_sequence":0,"event_id":format!("s:r:1:{idx}"),"timestamp":"2026-10-26T00:00:00Z","event_type":kind,"data":data.to_string()})
}
fn open(path: &Path, canonical: bool) -> SqliteStore {
    let store = SqliteStore::open(path).unwrap();
    if canonical {
        store.reserve_segmented("s").unwrap();
    } else {
        store.bind_legacy_events("s").unwrap();
    }
    store.replace("s", vec![
        json!({"id":"info","type":"session_info","timestamp":"2026-10-26T00:00:00Z","content":{"cwd":".","model":"synthetic"}}),
        json!({"id":"user","type":"user","role":"user","timestamp":"2026-10-26T00:00:00Z","content":"synthetic prompt"}),
        json!({"id":"start","type":"run_started","timestamp":"2026-10-26T00:00:00Z","content":{"run_id":"r","epoch":1,"run_sequence":0}}),
    ]).unwrap();
    store
}
fn write(store: &SqliteStore, writer: &mut Writer, canonical: bool, events: Vec<Value>) {
    if canonical {
        writer.persist(store, "s", &events).unwrap();
    } else {
        store.append_events("s", events).unwrap();
    }
}
fn counts(store: &SqliteStore) -> Value {
    store
        .db
        .read(|db| {
            let mut result = serde_json::Map::new();
            for table in [
                "run_events",
                "run_snapshots",
                "stream_changes",
                "content_segments",
                "content_objects",
            ] {
                let count: i64 =
                    db.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                        row.get(0)
                    })?;
                result.insert(table.into(), json!(count));
            }
            let bytes: i64 = db.query_row(
                "SELECT coalesce(sum(byte_length),0) FROM content_objects",
                [],
                |row| row.get(0),
            )?;
            result.insert("contentBytes".into(), json!(bytes));
            Ok(Value::Object(result))
        })
        .unwrap()
}

fn workload(canonical: bool, target_bytes: usize) -> Value {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("agent.db");
    let store = open(&path, canonical);
    let mut writer = Writer::default();
    let fragment = "思考🙂x".repeat(5);
    let fragments = target_bytes.div_ceil(fragment.len());
    let body = fragment.repeat(fragments);
    write(
        &store,
        &mut writer,
        canonical,
        vec![
            event(0, "message_start", json!({"entry_id":"m"})),
            event(
                1,
                "thinking_start",
                json!({"entry_id":"m","content_key":"m:thinking:b","content_offset":0,"content_bytes":0}),
            ),
        ],
    );
    let start = Instant::now();
    let mut latencies = Vec::new();
    let mut peak_files = 0;
    for batch_start in (0..fragments).step_by(128) {
        let events = (batch_start..(batch_start + 128).min(fragments)).map(|index| {
            event(index as i64 + 2,"thinking_delta",json!({"entry_id":"m","content_key":"m:thinking:b","content_offset":index*fragment.len(),"content_bytes":fragment.len(),"text":fragment}))
        }).collect();
        let commit = Instant::now();
        write(&store, &mut writer, canonical, events);
        latencies.push(commit.elapsed().as_micros() as u64);
        let files = [&path, &path.with_extension("db-wal")]
            .iter()
            .map(|file| std::fs::metadata(file).map_or(0, |meta| meta.len()))
            .sum::<u64>();
        peak_files = peak_files.max(files);
    }
    let write_micros = start.elapsed().as_micros();
    write(
        &store,
        &mut writer,
        canonical,
        vec![
            event(
                fragments as i64 + 2,
                "thinking_end",
                json!({"entry_id":"m","content_key":"m:thinking:b","content_offset":body.len(),"content_bytes":0,"content_seal":true,"provider_metadata":{}}),
            ),
            event(
                fragments as i64 + 3,
                "message_end",
                json!({"entry_id":"m","complete":true}),
            ),
        ],
    );
    let message = AgentMessage {
        role: "assistant".into(),
        content: vec![ContentBlock::reasoning(&body, serde_json::Map::new())],
        metadata: Some(
            [
                (AgentMessage::JOURNAL_ENTRY_ID_KEY.into(), json!("m")),
                (
                    AgentMessage::STREAM_BLOCKS_KEY.into(),
                    json!(["m:thinking:b"]),
                ),
            ]
            .into_iter()
            .collect(),
        ),
        ..Default::default()
    };
    let before = counts(&store);
    let finish = Instant::now();
    if canonical {
        store
            .append_stream_entry("s", StreamEntry::from_assistant(&message, "r", 1).unwrap())
            .unwrap();
    } else {
        store.append("s", vec![json!({"id":"m","type":"assistant","role":"assistant","timestamp":"2026-10-26T00:00:01Z","meta":{"run_id":"r"},"content":message.content})]).unwrap();
    }
    store.commit_terminal_entries("s", vec![json!({"id":"end","type":"run_terminal","timestamp":"2026-10-26T00:00:02Z","content":{"run_id":"r","state":"completed","run_tokens":17}})]).unwrap();
    let finish_micros = finish.elapsed().as_micros();
    let after = counts(&store);
    if canonical {
        assert_eq!(after["run_events"], 0);
        assert_eq!(after["run_snapshots"], 0);
        assert!(
            after["contentBytes"].as_i64().unwrap() - before["contentBytes"].as_i64().unwrap()
                <= WRITE_BYTES as i64
        );
    }
    let actual = store
        .entries("s")
        .unwrap()
        .into_iter()
        .find(|entry| entry["id"] == "m")
        .unwrap();
    assert_eq!(actual["content"][0]["text"], body);
    latencies.sort_unstable();
    let percentile =
        |percent: usize| latencies[(latencies.len() * percent / 100).min(latencies.len() - 1)];
    let final_files = [&path, &path.with_extension("db-wal")]
        .iter()
        .map(|file| std::fs::metadata(file).map_or(0, |meta| meta.len()))
        .sum::<u64>();
    json!({"format":if canonical {2}else{1},"bodyBytes":body.len(),"fragments":fragments,"writeMicros":write_micros,
        "foregroundBatchMicros":{"p95":percentile(95),"p99":percentile(99),"max":latencies.last()},
        "finishMicros":finish_micros,"finalDatabaseAndWalBytes":final_files,"sampledPeakDatabaseAndWalBytes":peak_files.max(final_files),"rows":after,"io":store.db.metrics()})
}

#[test]
#[ignore = "final acceptance: synthetic 1/10 MiB legacy/canonical storage workload"]
fn storage_space_and_constant_finish_workload() {
    let mut cases = Vec::new();
    for bytes in [1024 * 1024, 10 * 1024 * 1024] {
        for canonical in [false, true] {
            cases.push(workload(canonical, bytes));
        }
    }
    let report = json!({"workload":"synthetic-reasoning-55-byte-fragments-128-event-batches","cases":cases,
        "limits":"foregroundBatch is a direct writer measurement, not live admission backlog, UI latency, CPU time or peak RSS; file peak is sampled after batches; no VACUUM or compaction"});
    println!("STORAGE_ACCEPTANCE_REPORT={report}");
    if let Some(path) = std::env::var_os("FUTURE_STORAGE_ACCEPTANCE_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}

#[test]
#[ignore = "final acceptance: one shared writer with 1/4/16 sessions and bounded readers"]
fn shared_writer_and_history_reader_concurrency() {
    let mut cases = Vec::new();
    for sessions in [1, 4, 16] {
        let root = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&root.path().join("agent.db")).unwrap();
        for index in 0..sessions {
            let session = format!("s{index}");
            store.reserve_segmented(&session).unwrap();
            store.replace(&session,vec![
                json!({"id":"info","type":"session_info","timestamp":"2026-10-26T00:00:00Z","content":{"cwd":".","model":"synthetic"}}),
                json!({"id":"start","type":"run_started","timestamp":"2026-10-26T00:00:00Z","content":{"run_id":"r","epoch":1}}),
            ]).unwrap();
        }
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(sessions + 1));
        let handles: Vec<_> = (0..sessions).map(|index| {
            let store = store.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let session = format!("s{index}");
                let scope = stream::StreamIdentity {session_id:session.clone(),run_id:"r".into(),epoch:1};
                let mut writer = Writer::default();
                let mut initial = vec![event(0,"message_start",json!({"entry_id":"m"})),event(1,"thinking_start",json!({"entry_id":"m","content_key":"m:thinking:b","content_offset":0,"content_bytes":0}))];
                for event in &mut initial { event["session_id"] = json!(session); }
                barrier.wait();
                let start = Instant::now();
                writer.persist(&store,&session,&initial).unwrap();
                let fragment = "思考🙂x".repeat(5);
                let fragments = (1024_usize*1024).div_ceil(fragment.len());
                let mut commits = Vec::new();
                let mut reads = Vec::new();
                for batch in (0..fragments).step_by(128) {
                    let events: Vec<_> = (batch..(batch+128).min(fragments)).map(|part| {
                        let mut event = event(part as i64+2,"thinking_delta",json!({"entry_id":"m","content_key":"m:thinking:b","content_offset":part*fragment.len(),"content_bytes":fragment.len(),"text":fragment}));
                        event["session_id"] = json!(session);
                        event
                    }).collect();
                    let commit = Instant::now();
                    writer.persist(&store,&session,&events).unwrap();
                    commits.push(commit.elapsed().as_micros() as u64);
                    if batch % (8*128) == 0 {
                        let read = Instant::now();
                        assert_eq!(store.segmented_stream_page(&scope,-1,1).unwrap().len(),1);
                        reads.push(read.elapsed().as_micros() as u64);
                    }
                }
                commits.sort_unstable();
                json!({"session":session,"bodyBytes":fragments*fragment.len(),"elapsedMicros":start.elapsed().as_micros(),
                    "batchP99Micros":commits[(commits.len()*99/100).min(commits.len()-1)],"batchMaxMicros":commits.last(),"readerMaxMicros":reads.iter().max()})
            })
        }).collect();
        barrier.wait();
        let workers: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        store.recover_segmented_runs().unwrap();
        for worker in &workers {
            let entries = store.entries(worker["session"].as_str().unwrap()).unwrap();
            let message = entries.iter().find(|entry| entry["id"] == "m").unwrap();
            assert_eq!(
                message["content"][0]["text"].as_str().unwrap().len() as u64,
                worker["bodyBytes"].as_u64().unwrap()
            );
        }
        cases.push(json!({"sessions":sessions,"workers":workers,"io":store.db.metrics()}));
    }
    println!(
        "STORAGE_CONCURRENCY_REPORT={}",
        json!({"cases":cases,"scope":"shared ordered SQLite writer and independent bounded history readers; direct batches, not live admission backlog or UI timing"})
    );
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn ready(label: &str) {
    println!("STORAGE_CRASH_READY:{label}");
    std::io::stdout().flush().unwrap();
    let mut byte = [0];
    std::io::Read::read_exact(&mut std::io::stdin(), &mut byte).unwrap();
}

fn confirmed_prefix(store: &SqliteStore) -> Writer {
    let mut writer = Writer::default();
    writer.persist(store,"s", &[
        event(0,"message_start",json!({"entry_id":"m"})),
        event(1,"thinking_start",json!({"entry_id":"m","content_key":"m:thinking:b","content_offset":0,"content_bytes":0})),
        event(2,"thinking_delta",json!({"entry_id":"m","content_key":"m:thinking:b","content_offset":0,"content_bytes":9,"text":"confirmed"})),
    ]).unwrap();
    writer
}

fn cursor(store: &SqliteStore) -> i64 {
    store
        .db
        .read(|db| {
            Ok(db.query_row(
                "SELECT durable_cursor FROM stream_runs WHERE session_id='s' AND run_id='r'",
                [],
                |row| row.get(0),
            )?)
        })
        .unwrap()
}

#[test]
#[ignore = "final acceptance: SQLITE_FULL injection without filling the actual disk"]
fn full_database_does_not_acknowledge_a_body_and_recovers_after_capacity_returns() {
    let directory = tempfile::tempdir().unwrap();
    let store = open(&directory.path().join("agent.db"), true);
    let mut writer = confirmed_prefix(&store);
    store
        .db
        .call(|db| {
            let pages: i64 = db.query_row("PRAGMA page_count", [], |row| row.get(0))?;
            db.pragma_update(None, "max_page_count", pages)?;
            Ok(())
        })
        .unwrap();
    let delta = event(
        3,
        "thinking_delta",
        json!({"entry_id":"m","content_key":"m:thinking:b","content_offset":9,"content_bytes":WRITE_BYTES,"text":"x".repeat(WRITE_BYTES)}),
    );
    assert!(writer
        .persist(&store, "s", std::slice::from_ref(&delta))
        .is_err());
    assert_eq!(cursor(&store), 2);
    store
        .db
        .call(|db| {
            db.pragma_update(None, "max_page_count", 2_147_483_646_i64)?;
            Ok(())
        })
        .unwrap();
    writer.persist(&store, "s", &[delta]).unwrap();
    assert_eq!(cursor(&store), 3);
    println!(
        "STORAGE_FAULT_ACCEPTANCE={}",
        json!({"fault":"sqlite_full","confirmedPrefixPreserved":true,"sameBodyRetrySucceeded":true,"physicalDiskFilled":false})
    );
}

#[test]
#[ignore = "final acceptance: external write lock and independent committed-prefix read"]
fn write_lock_refuses_new_ack_but_allows_reading_the_confirmed_prefix() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("agent.db");
    let store = open(&path, true);
    let mut writer = confirmed_prefix(&store);
    let lock = rusqlite::Connection::open(&path).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    let delta = event(
        3,
        "thinking_delta",
        json!({"entry_id":"m","content_key":"m:thinking:b","content_offset":9,"content_bytes":4,"text":"tail"}),
    );
    assert!(writer
        .persist(&store, "s", std::slice::from_ref(&delta))
        .is_err());
    assert_eq!(cursor(&store), 2);
    lock.execute_batch("ROLLBACK").unwrap();
    writer.persist(&store, "s", &[delta]).unwrap();
    assert_eq!(cursor(&store), 3);
    println!(
        "STORAGE_FAULT_ACCEPTANCE={}",
        json!({"fault":"external_write_lock","committedReaderAvailable":true,"sameBodyRetrySucceeded":true})
    );
}

#[test]
#[ignore = "final acceptance: real writer-thread unwind with an open SQLite transaction"]
fn writer_panic_rolls_back_and_reopen_recovers_the_confirmed_prefix() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("agent.db");
    let store = open(&path, true);
    let _writer = confirmed_prefix(&store);
    let result = store.db.call::<()>(|db| {
        let tx = crate::session::database::begin_write_transaction(db)?;
        let id = create(&tx)?;
        append(&tx, id, 0, "uncommitted panic body")?;
        tx.execute(
            "UPDATE stream_runs SET durable_cursor=99 WHERE session_id='s'",
            [],
        )?;
        panic!("injected storage writer panic");
    });
    assert!(result.is_err());
    assert_eq!(cursor(&store), 2);
    drop(store);
    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(cursor(&store), 2);
    store.recover_segmented_runs().unwrap();
    assert_eq!(
        store
            .entries("s")
            .unwrap()
            .into_iter()
            .find(|entry| entry["id"] == "m")
            .unwrap()["content"][0]["text"],
        "confirmed"
    );
    println!(
        "STORAGE_FAULT_ACCEPTANCE={}",
        json!({"fault":"writer_panic","openTransactionRolledBack":true,"reopenRecoveredPrefix":true})
    );
}

#[test]
#[ignore = "helper process for the explicit final crash workload"]
fn crash_child() {
    let Some(path) = std::env::var_os("FUTURE_STORAGE_CRASH_DB") else {
        return;
    };
    let store = open(Path::new(&path), true);
    let _writer = confirmed_prefix(&store);
    if std::env::var("FUTURE_STORAGE_CRASH_STAGE").unwrap() == "open-transaction" {
        store
            .db
            .call(|db| {
                let tx = crate::session::database::begin_write_transaction(db)?;
                let id = create(&tx)?;
                append(&tx, id, 0, "uncommitted")?;
                tx.execute(
                    "UPDATE stream_runs SET durable_cursor=99 WHERE session_id='s'",
                    [],
                )?;
                ready("open-transaction");
                tx.commit()?;
                Ok(())
            })
            .unwrap();
    } else {
        ready("confirmed");
    }
}

#[test]
#[ignore = "final acceptance: kill real child processes around commit boundaries"]
fn killed_process_retains_only_the_confirmed_prefix() {
    for stage in ["confirmed", "open-transaction"] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("agent.db");
        let mut child = ChildGuard(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "session::content::acceptance_tests::crash_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env("FUTURE_STORAGE_CRASH_DB", &path)
                .env("FUTURE_STORAGE_CRASH_STAGE", stage)
                .stdout(Stdio::piped())
                .stdin(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let stdout = child.0.stdout.take().unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines() {
                let line = line.unwrap();
                if let Some(at) = line.find("STORAGE_CRASH_READY:") {
                    let _ = send.send(line[at..].to_owned());
                    return;
                }
            }
        });
        assert_eq!(
            receive.recv_timeout(Duration::from_secs(60)).unwrap(),
            format!("STORAGE_CRASH_READY:{stage}")
        );
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        reader.join().unwrap();
        let store = SqliteStore::open(&path).unwrap();
        store
            .db
            .read(|db| {
                let cursor: i64 = db.query_row(
                    "SELECT durable_cursor FROM stream_runs WHERE session_id='s' AND run_id='r'",
                    [],
                    |row| row.get(0),
                )?;
                assert_eq!(cursor, 2);
                Ok(())
            })
            .unwrap();
        store.recover_segmented_runs().unwrap();
        let recovered = store.entries("s").unwrap();
        let message = recovered.iter().find(|entry| entry["id"] == "m").unwrap();
        assert_eq!(message["content"][0]["text"], "confirmed");
        assert!(recovered.iter().any(|entry| entry["type"] == "run_terminal"
            && entry["content"]["state"] == crate::session::RUN_STATE_INTERRUPTED_BY_RESTART));
        println!(
            "STORAGE_CRASH_ACCEPTANCE={}",
            json!({"stage":stage,"committedCursor":2,"recoveredBytes":9,"processKill":true,"powerLossSimulated":false})
        );
    }
}
