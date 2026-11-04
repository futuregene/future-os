//! P0-1 harness: the cheap reads must answer exactly what the whole-transcript
//! reads answered, and must stop growing with the transcript.
//!
//! Both tests are `#[ignore]`d because neither belongs in a normal run: one needs
//! a copy of a real database, the other builds a 50,000-entry conversation.
//!
//! Real snapshot (never open the live file read-write, never touch the running
//! agent):
//!
//! ```text
//! mkdir -p /tmp/p0-equiv
//! sqlite3 "file:$HOME/.future/agent/agent.db?mode=ro" ".backup '/tmp/p0-equiv/agent.db'"
//! FUTURE_EQUIV_DB=/tmp/p0-equiv/agent.db \
//!   cargo test -p future-agent --lib legacy_reads_match -- --ignored --nocapture
//! ```
//!
//! The 50,000-entry scale point from the audit:
//!
//! ```text
//! cargo test -p future-agent --lib legacy_reads_do_not_scale -- --ignored --nocapture
//! ```
use super::*;
use rusqlite::OptionalExtension;
use std::time::Instant;

/// Every run marker of the session, assembled by the `entry_records` view — what
/// `read_run_marker_headers` returned for a legacy conversation before the
/// indexed read.
fn whole_history_run_markers(store: &SqliteStore, id: &str) -> Vec<serde_json::Value> {
    let id = id.to_owned();
    store
        .db
        .read(move |db| {
            let key = crate::session::identity::session_key(db, &id)?;
            let mut statement = db.prepare_cached(
                "SELECT payload FROM entry_records WHERE session_id=?1 AND entry_type IN \
                 ('run_started','run_terminal','run_stop_requested') ORDER BY position",
            )?;
            let rows = statement
                .query_map([&key], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows
                .into_iter()
                .map(|raw| serde_json::from_str::<serde_json::Value>(&raw))
                .collect::<std::result::Result<Vec<_>, _>>()?)
        })
        .unwrap()
}

/// The cheap marker read, exactly as `run_candidate` asks for it.
fn cheap_run_markers(store: &SqliteStore, id: &str) -> Vec<serde_json::Value> {
    let id = id.to_owned();
    store
        .db
        .read(move |db| crate::session::sqlite_store::read_run_marker_headers(db, &id))
        .unwrap()
}

fn as_entries(markers: &[serde_json::Value]) -> Vec<SessionEntry> {
    markers
        .iter()
        .cloned()
        .map(|marker| serde_json::from_value(marker).unwrap())
        .collect()
}

/// The answer the data itself gives, independent of any read path.
///
/// This used to call `load_metadata` and count `role == "user"` over its
/// entries, because that is what `state_metadata` did before the indexed read.
/// `load_metadata` is no longer a transcript read — a later change made it
/// fetch the three metadata rows (first entry, latest `session_info`, latest
/// `model_change`) and nothing else — so counting over its output now answers
/// zero for every legacy conversation and would compare the cheap read against
/// an empty list rather than against the truth.
///
/// So the baseline reads the bodies: one row per stored entry in
/// `history_shapes`, whose `role` is what the transcript always said the role
/// was. `entry_type` and that `role` agree on every row of a real database
/// (232,312 rows on this machine, 0 disagreements), which is exactly why
/// counting `entry_type='user'` is a faithful replacement — and why this test
/// can hold it to that.
fn whole_history_state_metadata(store: &SqliteStore, id: &str) -> (usize, String) {
    let id = id.to_owned();
    store
        .db
        .read(move |db| {
            let count: i64 = db.query_row(
                "SELECT count(*) FROM history_shapes
                  WHERE session_id=?1 AND json_extract(payload,'$.role')='user'",
                [&id],
                |row| row.get(0),
            )?;
            let parent: Option<String> = db
                .query_row(
                    "SELECT CASE WHEN json_type(current_metadata_json,'$.parent_session_id')='text'
                                  THEN json_extract(current_metadata_json,'$.parent_session_id') END
                       FROM sessions WHERE id=?1 AND revision>=0",
                    [&id],
                    |row| row.get(0),
                )
                .optional()?;
            Ok((usize::try_from(count)?, parent.unwrap_or_default()))
        })
        .unwrap_or((0, String::new()))
}

fn effective(metadata: anyhow::Result<(usize, String)>) -> (usize, String) {
    metadata.unwrap_or((0, String::new()))
}

#[test]
#[ignore = "needs FUTURE_EQUIV_DB: a .backup copy of a real agent.db"]
fn legacy_reads_match_the_whole_transcript_reads() {
    let path = std::env::var("FUTURE_EQUIV_DB")
        .expect("set FUTURE_EQUIV_DB to a copy of a real agent.db (see this module's docs)");
    let path = std::path::PathBuf::from(path);
    assert!(path.exists(), "{} is not readable", path.display());
    let manager = Manager::new(path.parent().expect("a file path").to_path_buf());
    let store = manager.storage().unwrap();

    // Legacy sessions are stored under their TEXT identity; a canonical
    // conversation's native identity is bytes, and its branches are untouched.
    let ids: Vec<String> = store
        .db
        .read(|db| {
            let mut statement =
                db.prepare("SELECT id FROM sessions WHERE typeof(id)='text' ORDER BY id")?;
            let ids = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(ids)
        })
        .unwrap();

    // Canonical conversations keep their native (bytes) identity and their own
    // reads, which this change does not touch.
    let native: i64 = store
        .db
        .read(|db| {
            Ok(db.query_row(
                "SELECT count(*) FROM sessions WHERE typeof(id)!='text'",
                [],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    let (mut legacy, mut with_entries) = (0usize, 0usize);
    let mut differences: Vec<String> = Vec::new();
    let started = Instant::now();
    for id in &ids {
        if store.storage_format(id).unwrap() == 2 {
            // A text-identity canonical conversation: its reads are unchanged,
            // so there is nothing to compare here.
            continue;
        }
        legacy += 1;
        let session = id.clone();
        let has_entries: bool = store
            .db
            .read(move |db| {
                let key = crate::session::identity::session_key(db, &session)?;
                Ok(db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM entries WHERE session_id=?1)",
                    [&key],
                    |row| row.get(0),
                )?)
            })
            .unwrap();
        if has_entries {
            with_entries += 1;
        }
        let new = effective(manager.state_metadata(id));
        let old = whole_history_state_metadata(store, id);
        if new != old {
            differences.push(format!("state_metadata {id}: cheap={new:?} whole={old:?}"));
        }
        let whole = whole_history_run_markers(store, id);
        let entries = as_entries(&whole);
        let old_run = (
            crate::session::find_unterminated_run(&entries),
            crate::session::find_resumable_run(&entries),
        );
        let new_run = (
            manager.unterminated_run_id(id).unwrap(),
            manager.resumable_run_id(id).unwrap(),
        );
        if new_run != old_run {
            differences.push(format!(
                "run markers {id}: cheap={new_run:?} whole={old_run:?}"
            ));
        }
        // Every marker the cheap read returns must be one the view returned,
        // with the content the recovery folds read. The cheap read drops rows
        // and carries only `type` + `content`; it never reshapes what it keeps.
        for marker in cheap_run_markers(store, id) {
            let matched = whole.iter().any(|whole| {
                whole["type"] == marker["type"] && whole.get("content") == marker.get("content")
            });
            if !matched {
                differences.push(format!(
                    "marker payload {id}: cheap={} has no whole-transcript twin",
                    marker
                ));
            }
        }
    }

    println!(
        "compared {legacy} legacy conversations ({with_entries} with entries), \
         {native} canonical skipped, in {:.1?}: state_metadata + late-run answers \
         against the whole-transcript reads",
        started.elapsed()
    );
    for difference in differences.iter().take(20) {
        println!("  {difference}");
    }
    assert_eq!(
        differences.len(),
        0,
        "{} session(s) answered differently",
        differences.len()
    );
    assert!(
        legacy > 0,
        "the snapshot had no legacy conversation to check"
    );
}

/// Insert `count` entries straight into the tables, bypassing the write path's
/// per-entry indexing: the fixture is read-only test data, and the index is
/// built once afterwards the way a real conversation's would already be.
fn build_legacy_fixture(store: &SqliteStore, id: &str, count: usize, user_every: i64) {
    store.bind_legacy_events(id).unwrap();
    let id = id.to_owned();
    let index_id = id.clone();
    store
        .db
        .call(move |db| {
            let tx = db.transaction()?;
            {
                let mut statement = tx.prepare(
                    "INSERT INTO entries(session_id,position,entry_id,entry_type,role,run_id,\
                     timestamp_ms,metadata_json,content_json) VALUES (?1,?2,?3,?4,?5,?6,?7,'{}',?8)",
                )?;
                for position in 0..count as i64 {
                    let (kind, role, run, content) = if position == 0 {
                        (
                            "session_info",
                            None,
                            None,
                            r#"{"cwd":"/a","model":"m","parent_session_id":"parent-1"}"#
                                .to_string(),
                        )
                    } else if position % 10 == 0 {
                        let run = format!("run-{}", position / 10);
                        (
                            "run_started",
                            Some("system"),
                            Some(run.clone()),
                            format!(r#"{{"run_id":"{run}","epoch":{}}}"#, position / 10),
                        )
                    } else if position % 10 == 5 {
                        let run = format!("run-{}", position / 10);
                        (
                            "run_terminal",
                            Some("system"),
                            Some(run.clone()),
                            format!(r#"{{"run_id":"{run}","state":"completed"}}"#),
                        )
                    } else if position % user_every == 1 {
                        ("user", Some("user"), None, r#""question""#.to_string())
                    } else {
                        ("assistant", Some("assistant"), None, r#""answer""#.to_string())
                    };
                    statement.execute(rusqlite::params![
                        &id,
                        position,
                        format!("e{position}"),
                        kind,
                        role,
                        run,
                        position,
                        content
                    ])?;
                }
            }
            // The write path copies a `session_info` snapshot into the session
            // row; the direct insert has to do it itself or the fixture would
            // have no metadata for either read to find.
            tx.execute(
                "UPDATE sessions SET current_metadata_json=?2 WHERE id=?1",
                rusqlite::params![
                    &id,
                    r#"{"cwd":"/a","model":"m","parent_session_id":"parent-1"}"#
                ],
            )?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
    store
        .db
        .call(move |db| {
            let tx = db.transaction()?;
            crate::session::history::ensure_history_index(&tx, index_id.as_str())?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
}

fn median_seconds(runs: usize, mut measure: impl FnMut()) -> f64 {
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let started = Instant::now();
        measure();
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    samples[samples.len() / 2]
}

/// The audit's judgement for P0-1 is "flat in transcript length", measured on a
/// 50,000-entry legacy conversation. These are the two reads themselves —
/// `state_metadata` and the unterminated-run answer — against the reads they
/// replaced, on a short conversation, on a 50,000-entry one, and on a
/// 50,000-entry one whose entries are mostly assistant/tool rows (a real
/// conversation carries a user message every few dozen entries, so the indexed
/// count visits far fewer rows than the dense fixture does).
#[test]
#[ignore = "builds 50,000-entry legacy conversations"]
fn legacy_reads_do_not_scale_with_the_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(dir.path().join("sessions"));
    let store = manager.storage().unwrap();
    let cases: [(&str, usize, i64); 3] = [
        ("legacy-small", 46, 2),
        ("legacy-dense", 50_000, 2),
        ("legacy-sparse", 50_000, 25),
    ];
    for (id, count, user_every) in cases {
        build_legacy_fixture(store, id, count, user_every);
    }
    let ids: Vec<&str> = cases.iter().map(|case| case.0).collect();

    println!(
        "{:<28} {:>11} {:>11} {:>11}",
        "read", "46", "50k dense", "50k sparse"
    );
    let row = |label: &str, values: [f64; 3]| {
        println!(
            "{label:<28} {:>10.3}ms {:>10.3}ms {:>10.3}ms",
            values[0], values[1], values[2]
        );
    };

    // A write's only effect on the cache key, measured on its own so the miss
    // row below can be read net of it.
    let bump = |id: &str| {
        let id = id.to_owned();
        store
            .db
            .call(move |db| {
                db.execute(
                    "UPDATE sessions SET revision=revision+1 WHERE id=?1",
                    [id.as_str()],
                )?;
                Ok(())
            })
            .unwrap();
    };
    row(
        "revision bump (write)",
        [
            median_seconds(5, || bump(ids[0])),
            median_seconds(5, || bump(ids[1])),
            median_seconds(5, || bump(ids[2])),
        ],
    );

    // Before: one `history_shapes` decode per entry of the transcript.
    row(
        "state_metadata (whole)",
        [
            median_seconds(5, || {
                let _ = whole_history_state_metadata(store, ids[0]);
            }),
            median_seconds(5, || {
                let _ = whole_history_state_metadata(store, ids[1]);
            }),
            median_seconds(5, || {
                let _ = whole_history_state_metadata(store, ids[2]);
            }),
        ],
    );

    // After: the indexed answer, on a cache miss and on a repeat poll.
    row(
        "state_metadata (cheap miss)",
        [
            median_seconds(5, || {
                bump(ids[0]);
                let _ = manager.state_metadata(ids[0]).unwrap();
            }),
            median_seconds(5, || {
                bump(ids[1]);
                let _ = manager.state_metadata(ids[1]).unwrap();
            }),
            median_seconds(5, || {
                bump(ids[2]);
                let _ = manager.state_metadata(ids[2]).unwrap();
            }),
        ],
    );
    row(
        "state_metadata (cheap cached)",
        [
            median_seconds(5, || {
                let _ = manager.state_metadata(ids[0]).unwrap();
            }),
            median_seconds(5, || {
                let _ = manager.state_metadata(ids[1]).unwrap();
            }),
            median_seconds(5, || {
                let _ = manager.state_metadata(ids[2]).unwrap();
            }),
        ],
    );

    // Before: the `entry_records` view assembles the session to find a handful of
    // run markers.
    row(
        "run markers (whole view)",
        [
            median_seconds(5, || {
                let _ = whole_history_run_markers(store, ids[0]);
            }),
            median_seconds(5, || {
                let _ = whole_history_run_markers(store, ids[1]);
            }),
            median_seconds(5, || {
                let _ = whole_history_run_markers(store, ids[2]);
            }),
        ],
    );

    // After: the latest run's markers, straight from the index.
    row(
        "unterminated_run_id (cheap)",
        [
            median_seconds(5, || {
                let _ = manager.unterminated_run_id(ids[0]).unwrap();
            }),
            median_seconds(5, || {
                let _ = manager.unterminated_run_id(ids[1]).unwrap();
            }),
            median_seconds(5, || {
                let _ = manager.unterminated_run_id(ids[2]).unwrap();
            }),
        ],
    );

    // Answers, not just timings: every fixture's last run is terminated, and
    // the parent id comes from the session row.
    for (id, count, user_every) in cases {
        assert_eq!(manager.unterminated_run_id(id).unwrap(), None);
        assert_eq!(
            manager.state_metadata(id).unwrap().1,
            "parent-1",
            "{id} parent"
        );
        let users = (0..count as i64)
            .filter(|position| {
                *position != 0
                    && position % 10 != 0
                    && position % 10 != 5
                    && position % user_every == 1
            })
            .count();
        assert_eq!(manager.state_metadata(id).unwrap().0, users, "{id} users");
    }
}
