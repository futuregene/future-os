//! Desktop-owned catalog source. Reads and revisions share one serialization
//! point across handshake, RPC and push; publication time never creates versions.
//!
//! # Two things change in a session list, and they are not the same size
//!
//! A run that is streaming rewrites its thread's `threads.last_message_at`
//! about once a second. That column is the list's *ordering key*, and it is the
//! one field a client can be told about without being handed the list: the value
//! belongs to a row the client already holds, and re-ranking from it is
//! something the client's own sort already does. Everything else — the row set,
//! `title`, `pinned`, `streaming`, `status`, the workspace's name/pin/place, and
//! the array order itself — is *structure*: a client cannot patch it from a
//! value, so a change there is worth a whole snapshot.
//!
//! [`sessions_catalog`] therefore returns two signatures over the same rows
//! *and* the recency vector, so a publisher can send a whole list when the
//! structural signature moves and a few dozen bytes when only recency did. The
//! revision ([`bump`]) is deliberately moved by *either*, because it is the
//! heartbeat's recovery signal: a client that missed a recency packet must be
//! able to notice, exactly as it notices a missed snapshot.
//!
//! The array order is on the structural side rather than the recency side for
//! that reason: the host's own order is what an ordinary single-machine list
//! draws, so a row overtaking another is content, and it is re-published. Only
//! the writes that do *not* reorder anything — the great majority of a run's
//! seconds — cost a packet.
use serde_json::{json, Value};
use std::sync::{LazyLock, Mutex};

struct VersionedCatalog {
    epoch: String,
    sessions: (u64, String),
    workspaces: (u64, String),
}
static CATALOG: LazyLock<Mutex<VersionedCatalog>> = LazyLock::new(|| {
    Mutex::new(VersionedCatalog {
        epoch: nkeys::KeyPair::new_user().public_key(),
        sessions: (0, String::new()),
        workspaces: (0, String::new()),
    })
});
fn version(payload: &mut Value, epoch: &str, previous: &mut (u64, String), key: &str) {
    let signature = payload[key].to_string();
    let revision = bump(previous, &signature);
    payload["version"] = json!({ "epoch": epoch, "revision": revision });
}

/// Advance a domain's revision when its content signature moved.
///
/// Shared by both domains so the rule cannot differ between them: the revision
/// counts *content states*, and any caller that reads the store re-derives the
/// signature from the store rather than from a dirty flag.
fn bump(previous: &mut (u64, String), signature: &str) -> u64 {
    if previous.0 == 0 || previous.1 != signature {
        previous.0 += 1;
        previous.1 = signature.to_string();
    }
    previous.0
}
pub(crate) fn epoch() -> String {
    CATALOG.lock().unwrap().epoch.clone()
}

/// The revisions last observed for each catalog domain, with the epoch they
/// belong to. Deliberately O(1): it reads the revision cached by the last
/// [`sessions`]/[`workspaces`] call instead of re-reading the store, so the
/// presence heartbeat can advertise this recovery signal on every tick for
/// free. A client that sees a revision newer than the one it last applied
/// knows a pushed snapshot was lost and pulls the catalogue itself — which is
/// what lets the desktop stop re-sending unchanged snapshots on a timer.
///
/// `0` means "no snapshot computed in this process yet"; the client's version
/// gate starts below that, so an unread domain never triggers a pull.
pub(crate) fn revisions() -> (String, u64, u64) {
    let state = CATALOG.lock().unwrap();
    (state.epoch.clone(), state.sessions.0, state.workspaces.0)
}
/// How long a pull may reuse the snapshot the publisher's tick just computed.
///
/// The tick reads the whole store every second, and `list_sessions` re-read it for
/// every pull — measured on this machine: 153 pulls in 24 minutes, median 2.6 s
/// and a p90 of 21 s, every one of them queued behind the same `CATALOG` lock and
/// the same read. One second is the tick's own period, so a pull can never be
/// more than one tick stale, and a write inside that window bypasses the cache
/// (`store::catalog_dirty_peek`).
const SNAPSHOT_REUSE: std::time::Duration = std::time::Duration::from_secs(1);

/// The last computed sessions snapshot, and whether it is a pull or a tick.
static SESSIONS_SNAPSHOT: LazyLock<Mutex<Option<(std::time::Instant, String, SessionsCatalog)>>> =
    LazyLock::new(|| Mutex::new(None));

/// A cached workspace list and when it was computed.
type WorkspaceSnapshot = (std::time::Instant, (Value, String));

/// The last computed workspace snapshot, for the same reason.
static WORKSPACES_SNAPSHOT: LazyLock<Mutex<Option<WorkspaceSnapshot>>> =
    LazyLock::new(|| Mutex::new(None));

/// A sessions snapshot a *reader* may be served from the last tick.
///
/// The publisher's own path (`sessions_catalog`) always reads the store: it is
/// the thing that decides whether anything changed. This is the request path,
/// where a client asking for the catalogue a moment after that read would
/// otherwise pay for the whole read again — while holding a lock the publisher
/// needs, which is how a pull storm (client timeouts → reconnect → pull) makes
/// every read in the storm slower than the one before it.
pub(crate) fn sessions_catalog_cached(pair_id: &str) -> Option<SessionsCatalog> {
    if !crate::store::catalog_dirty_peek() {
        let cached = SESSIONS_SNAPSHOT.lock().unwrap();
        if let Some((at, key, value)) = cached.as_ref() {
            if key == pair_id && at.elapsed() < SNAPSHOT_REUSE {
                return Some(value.clone());
            }
        }
    }
    sessions_catalog(pair_id)
}

/// The workspace list a reader may be served from the last tick.
pub(crate) fn workspaces_cached() -> Option<(Value, String)> {
    if !crate::store::catalog_dirty_peek() {
        let cached = WORKSPACES_SNAPSHOT.lock().unwrap();
        if let Some((at, value)) = cached.as_ref() {
            if at.elapsed() < SNAPSHOT_REUSE {
                return Some(value.clone());
            }
        }
    }
    let value = workspaces()?;
    *WORKSPACES_SNAPSHOT.lock().unwrap() = Some((std::time::Instant::now(), value.clone()));
    Some(value)
}

pub(crate) fn sessions(pair_id: &str) -> Option<(Value, String)> {
    let catalog = sessions_catalog(pair_id)?;
    Some((catalog.payload, catalog.signature))
}

/// The one field a recency packet may move, and the only one the structural
/// signature drops.
const RECENCY_FIELD: &str = "lastMessageAt";

pub(crate) use future_remote_server::services::SessionsCatalog;

pub(crate) fn sessions_catalog(pair_id: &str) -> Option<SessionsCatalog> {
    let mut state = CATALOG.lock().unwrap();
    let mut payload = read_sessions(pair_id)?;
    let epoch = state.epoch.clone();
    let (signature, structural) = row_signatures(&payload["sessions"]);
    // The revision is bumped by the *whole* signature, so a recency-only change
    // moves it too: that is what makes a dropped recency packet observable to
    // the heartbeat's revision check.
    let revision = bump(&mut state.sessions, &signature);
    payload["version"] = json!({ "epoch": epoch, "revision": revision });
    let activity = recency_vector(&payload["sessions"]);
    let catalog = SessionsCatalog {
        payload,
        structural,
        signature,
        activity,
    };
    // A pull that arrives between two ticks is answered from this, not by a
    // second read of the same store.
    *SESSIONS_SNAPSHOT.lock().unwrap() = Some((
        std::time::Instant::now(),
        pair_id.to_string(),
        catalog.clone(),
    ));
    Some(catalog)
}

/// The whole-row signature and the structural one, from one walk of the rows.
///
/// The structural form is the rows re-serialized with the recency field
/// stripped. Re-serializing rather than assembling a key by hand is what keeps
/// the two signatures from disagreeing about content they both cover, and it
/// cannot drift from the payload because it *is* the payload minus one field.
/// It costs one JSON serialization of the same rows per read — the publisher
/// already serializes them once to publish, so this is the same order of work,
/// not a new one.
fn row_signatures(sessions: &Value) -> (String, String) {
    let signature = sessions.to_string();
    let Some(rows) = sessions.as_array() else {
        // Not a list: there is nothing to strip, and both signatures describe
        // the same (malformed) shape.
        return (signature.clone(), signature);
    };
    let stripped: Vec<Value> = rows
        .iter()
        .map(|row| {
            let mut row = row.clone();
            if let Some(object) = row.as_object_mut() {
                object.remove(RECENCY_FIELD);
            }
            row
        })
        .collect();
    (signature, Value::Array(stripped).to_string())
}

/// Every row's `(sessionId, lastMessageAt)`, for the recency packets.
fn recency_vector(sessions: &Value) -> Vec<(String, i64)> {
    sessions
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let session_id = row.get("sessionId").and_then(Value::as_str)?;
            let at = row.get(RECENCY_FIELD).and_then(Value::as_i64)?;
            Some((session_id.to_string(), at))
        })
        .collect()
}

pub(crate) fn workspaces() -> Option<(Value, String)> {
    let mut state = CATALOG.lock().unwrap();
    let mut payload = read_workspaces()?;
    let epoch = state.epoch.clone();
    version(&mut payload, &epoch, &mut state.workspaces, "workspaces");
    let signature = state.workspaces.1.clone();
    Some((payload, signature))
}
fn read_sessions(pair_id: &str) -> Option<Value> {
    let active_sessions = crate::store::active_run_sessions().ok()?;
    let threads = crate::store::list_threads().ok()?;
    // A client that groups its merged list by workspace needs the workspace's
    // *name* — an id means nothing to a reader, and the alternative (showing the
    // peer's name for every remote group) would collapse several workspaces into
    // one — its *pin*, because a pinned workspace must lead the client's list
    // exactly as it leads this one, and *when it was last touched*, because that
    // is the key this desktop orders its own workspace list by and the client's
    // groups must hold their place the same way. All three ride on each session
    // row: a client that has to ask for a separate workspace list would be asking
    // on every poll, and a group exists only where a row does. Best-effort: a
    // thread whose workspace row is gone still lists, under its peer's name,
    // un-pinned, and ordered as if its place were unknown.
    // The workspace's archived marker rides here too: a shelved workspace hides
    // its conversations without rewriting their own status (see the archive
    // design), so a session row is archived when either half says so — which is
    // the same rule the desktop's own list queries apply.
    let workspaces: std::collections::HashMap<String, (String, bool, i64, bool)> =
        crate::store::list_workspaces()
            .map(|rows| {
                rows.into_iter()
                    .map(|row| {
                        let ordered_at = row.last_opened_at.unwrap_or(row.updated_at);
                        (
                            row.id,
                            (row.name, row.pinned, ordered_at, row.archived_at.is_some()),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();

    let thread_ids: Vec<String> = threads.iter().map(|t| t.id.clone()).collect();
    let run_infos = crate::store::latest_run_infos(&thread_ids).ok()?;
    let run_status_by_thread: std::collections::HashMap<&str, &str> = run_infos
        .iter()
        .map(|info| (info.thread_id.as_str(), info.status.as_str()))
        .collect();

    let mut sessions: Vec<serde_json::Value> = Vec::new();
    for t in &threads {
        let Some(sid) = t
            .agent_session_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let streaming = active_sessions.iter().any(|active| active == sid);
        let status = run_status_by_thread.get(t.id.as_str()).copied();
        let workspace = workspaces.get(&t.workspace_id);
        sessions.push(json!({
            "sessionId": sid,
            "threadId": t.id,
            "title": t.title,
            "mode": t.mode,
            "workspaceId": t.workspace_id,
            "workspaceName": workspace.map(|(name, _, _, _)| name.clone()),
            // Whether this conversation's workspace is pinned *here*. Optional by
            // contract: a client built before this field simply orders by recency,
            // which is what it did before.
            "workspacePinned": workspace.map(|(_, pinned, _, _)| *pinned),
            // When that workspace itself was last opened or touched, in the same
            // coalesce `future_app_workspaces::list` orders its own rows by. A
            // merged list ranks a group by *its* place rather than by its
            // conversations' timestamps, so this is what keeps a group from
            // moving while the user works in it. Optional by contract: without
            // it a client orders that group last, not as if it were just used.
            "workspaceOrderedAt": workspace.map(|(_, _, ordered_at, _)| *ordered_at),
            "parentSessionId": t.parent_session_id,
            "pinned": t.pinned,
            // Shelved: the conversation itself, or the workspace holding it. The
            // desktop's default lists hide both, and a client must agree or the
            // same conversation sits in two different places depending on which
            // device is looking. Optional by contract: a client that predates the
            // field lists the row as active, which is what it did before.
            "archived": t.status == "archived"
                || workspace.is_some_and(|(_, _, _, archived)| *archived),
            "streaming": streaming,
            "status": status,
            // Last-activity timestamp, in the *same* coalesce the store's own
            // ordering uses (`pinned DESC, COALESCE(last_message_at, updated_at,
            // created_at) DESC`). A client merging several desktops shows this as
            // a row's age, and ranks a row from a host that predates `createdAt`
            // by it. Optional by contract: clients built before this field fall
            // back to the snapshot order, and a row whose thread has no
            // timestamps at all omits it.
            "lastMessageAt": t.last_message_at.unwrap_or(t.updated_at),
            // When the conversation was *started*, which is what a client ranks
            // rows by: an active run rewrites `lastMessageAt` about once a
            // second, and a list ordered by that re-shuffles under the reader's
            // thumb for as long as the run streams. Optional by contract: a
            // client built before this field falls back to the recency key,
            // which is what it ordered by before.
            "createdAt": t.created_at,
        }));
    }

    let payload = json!({
        "pairId": pair_id,
        "sessions": sessions,
    });
    Some(payload)
}

/// Workspaces-only snapshot for `p.{pair}.state.workspaces`.
fn read_workspaces() -> Option<Value> {
    let workspaces = crate::store::list_workspaces().ok()?;
    let mut workspace_values: Vec<serde_json::Value> = Vec::new();
    for w in &workspaces {
        if w.kind != "user" {
            continue;
        }
        if let Ok(mut value) = serde_json::to_value(w) {
            // A plain boolean beside the record's own `archivedAt`: a client
            // should not have to know that "archived" is spelled as a timestamp.
            value["archived"] = serde_json::json!(w.archived_at.is_some());
            workspace_values.push(value);
        }
    }
    let payload = json!({ "workspaces": workspace_values });
    Some(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{create_thread, record_thread_message_activity, CreateThreadInput};

    fn new_thread(session: &str, title: &str) -> crate::store::ThreadRecord {
        create_thread(CreateThreadInput {
            mode: "chat".into(),
            title: Some(title.into()),
            workspace_id: None,
            workspace_path: Some("/tmp/catalog-snapshot".into()),
            workspace_name: Some("catalog".into()),
            agent_session_id: Some(session.into()),
        })
        .expect("create thread under the test HOME")
    }

    /// A conversation filed under a *user* workspace, which is the only kind that
    /// can be shelved (a chat's temporary workspace cannot).
    fn new_workspace_thread(
        session: &str,
        title: &str,
    ) -> (crate::store::WorkspaceRecord, crate::store::ThreadRecord) {
        let thread = create_thread(CreateThreadInput {
            mode: "workspace".into(),
            title: Some(title.into()),
            workspace_id: None,
            workspace_path: Some("/tmp/catalog-workspace".into()),
            workspace_name: Some("Catalog Project".into()),
            agent_session_id: Some(session.into()),
        })
        .expect("create a workspace conversation under the test HOME");
        let workspace = crate::store::get_workspace(&thread.workspace_id)
            .expect("read the workspace")
            .expect("the workspace create_thread made");
        (workspace, thread)
    }

    /// The number behind the cache: how a pull behaves with it and without it.
    ///
    /// Same store, same call count, same machine — only the read path differs.
    /// The live figure this is standing in for was taken from `desktop.log`
    /// during a build window: 153 `list_sessions` in 24 minutes, median 2.6 s,
    /// p90 20.8 s, max 35.9 s, each queued behind the lock the 1 s tick holds.
    ///
    /// `#[ignore]`d: it seeds six hundred conversations and takes seconds, so it
    /// belongs to a deliberate measurement, not to every test run.
    #[test]
    #[ignore = "latency harness: run with --ignored --nocapture"]
    fn a_pull_costs_nothing_when_the_tick_already_read_the_store() {
        let _home = crate::remote::test_support::HomeGuard::new("catalog-pull-latency");
        crate::remote::test_support::init_store();
        for index in 0..600 {
            new_thread(
                &format!("sess_latency_{index}"),
                &format!("Conversation number {index} with a realistic title"),
            );
        }
        crate::store::take_catalog_dirty();

        let report = |label: &str, samples: &mut Vec<u128>, expected_checksum: &str| {
            samples.sort_unstable();
            let at = |p: f64| samples[((samples.len() as f64 * p) as usize).min(samples.len() - 1)];
            println!(
                "{label:<10} n={} median={}us p90={}us max={}us  checksum={expected_checksum}",
                samples.len(),
                at(0.5),
                at(0.9),
                samples[samples.len() - 1],
            );
        };

        // The old path: every pull reads the store (and takes the same lock the
        // tick takes).
        let mut uncached = Vec::new();
        let mut signature = String::new();
        for _ in 0..200 {
            let started = std::time::Instant::now();
            let catalog = sessions("").expect("store readable");
            uncached.push(started.elapsed().as_micros());
            signature = catalog.1;
        }
        let checksum = format!("{:016x}", fxhash(&signature));
        report("uncached", &mut uncached, &checksum);

        // The pull path now: the first call recomputes (as a tick would), the
        // rest reuse that snapshot.
        let mut cached = Vec::new();
        for _ in 0..200 {
            let started = std::time::Instant::now();
            let catalog = sessions_catalog_cached("").expect("store readable");
            cached.push(started.elapsed().as_micros());
            assert_eq!(catalog.signature, signature, "the answer must not change");
        }
        report("cached", &mut cached, &checksum);

        // The same two paths under the concurrency the live case had: the phone's
        // pulls and the tick all wanting the same lock at once.
        let concurrent = |label: &str, pull: fn()| {
            let samples = std::sync::Arc::new(Mutex::new(Vec::<u128>::new()));
            let mut workers = Vec::new();
            for _ in 0..8 {
                let samples = samples.clone();
                workers.push(std::thread::spawn(move || {
                    let mut local = Vec::new();
                    for _ in 0..50 {
                        let started = std::time::Instant::now();
                        pull();
                        local.push(started.elapsed().as_micros());
                    }
                    samples.lock().unwrap().extend(local);
                }));
            }
            for worker in workers {
                worker.join().expect("worker joins");
            }
            let mut samples = samples.lock().unwrap().clone();
            samples.sort_unstable();
            let at = |p: f64| samples[((samples.len() as f64 * p) as usize).min(samples.len() - 1)];
            println!(
                "{label:<18} n={} median={}us p90={}us max={}us",
                samples.len(),
                at(0.5),
                at(0.9),
                samples[samples.len() - 1],
            );
        };
        concurrent("uncached x8", || {
            sessions("");
        });
        concurrent("cached x8", || {
            sessions_catalog_cached("");
        });
    }

    /// A cheap stable hash, only so the two legs can prove they answered with
    /// the same snapshot without printing six hundred titles.
    fn fxhash(value: &str) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for byte in value.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    /// A pull is answered from the snapshot a tick just computed — unless a write
    /// landed in between, which is the one case where that answer would be a lie.
    ///
    /// This is what a pull storm needs: 153 catalogue reads in 24 minutes, each
    /// queued behind the same lock and the same store read the publisher had
    /// already done for that second.
    #[test]
    fn a_pull_reuses_the_tick_it_follows_but_never_a_snapshot_from_before_a_write() {
        let _home = crate::remote::test_support::HomeGuard::new("catalog-pull-cache");
        crate::remote::test_support::init_store();
        new_thread("sess_pull_cache_a", "first");
        // Stand in for the tick having consumed the flag of these writes.
        crate::store::take_catalog_dirty();

        let first = sessions_catalog_cached("pair_cache").expect("store readable");
        assert_eq!(
            first.payload["pairId"], "pair_cache",
            "a cached answer still belongs to the pairing that asked"
        );
        // A second pull within the window is the same snapshot, not a second read.
        let again = sessions_catalog_cached("pair_cache").expect("store readable");
        assert_eq!(again.signature, first.signature);

        // A write in the window must be visible immediately, not one tick later.
        // This is the assertion that goes red if `sessions_catalog_cached` stops
        // asking whether a write is pending: the snapshot above would be handed
        // out again and a rename would be invisible for up to a second.
        new_thread("sess_pull_cache_b", "second");
        assert!(
            crate::store::catalog_dirty_peek(),
            "a write marks the catalogue dirty; the cache must consult this"
        );
        let after = sessions_catalog_cached("pair_cache").expect("store readable");
        assert_ne!(
            after.signature, first.signature,
            "a pull after a write must read the store, not the snapshot taken before it"
        );

        // A pairing this machine does not serve asks its own question: the cache
        // is keyed by pairing, and an answer addressed to another device must
        // never be handed to this one. Last, because it replaces the entry the
        // write check above needs to still be this pairing's.
        let other = sessions_catalog_cached("pair_other").expect("store readable");
        assert_eq!(other.payload["pairId"], "pair_other");
    }

    /// The activity stamp a client shows as a row's age.
    ///
    /// It is the same coalesce the store orders by (`last_message_at`, else
    /// `updated_at`), it has to be present, and it must move when a message
    /// lands. A client no longer ranks rows by it (the creation time below is
    /// the ranking key), but it is the fallback for a client older than that
    /// field — and the age every row draws. Clients older than this field ignore
    /// it.
    #[test]
    fn sessions_snapshot_carries_last_message_at() {
        let _home = crate::remote::test_support::HomeGuard::new("catalog-last-message");
        crate::remote::test_support::init_store();

        let older = new_thread("sess-older", "Older");
        let newer = new_thread("sess-newer", "Newer");
        record_thread_message_activity(&newer.id, 2_000).expect("record activity");

        let (payload, _signature) = sessions("pair_test").expect("snapshot");
        let rows = payload["sessions"].as_array().expect("sessions array");
        let by_id = |session: &str| {
            rows.iter()
                .find(|row| row["sessionId"] == json!(session))
                .expect("session in snapshot")
        };
        assert_eq!(by_id("sess-newer")["lastMessageAt"], json!(2_000));
        // No message yet: the same `updated_at` fallback the store's ORDER BY uses.
        assert_eq!(
            by_id("sess-older")["lastMessageAt"],
            json!(older.updated_at)
        );
    }

    /// When the conversation was started, so a client ranks rows by a key that
    /// does not move.
    ///
    /// `lastMessageAt` is rewritten about once a second while a run streams, and
    /// a list ordered by it re-shuffles rows under the reader's thumb. The
    /// creation time is stable for the life of the thread and is the ranking key
    /// a client uses — every row of the desktop sidebar, and the running rows of
    /// the phone's recent view. Optional by contract: clients that predate this
    /// field keep ordering by recency.
    #[test]
    fn sessions_snapshot_carries_the_creation_time() {
        let _home = crate::remote::test_support::HomeGuard::new("catalog-created-at");
        crate::remote::test_support::init_store();

        let thread = new_thread("sess-started", "Started");
        record_thread_message_activity(&thread.id, 2_000).expect("record activity");

        let (payload, _signature) = sessions("pair_test").expect("snapshot");
        let row = payload["sessions"]
            .as_array()
            .expect("sessions array")
            .iter()
            .find(|row| row["sessionId"] == json!("sess-started"))
            .expect("session in snapshot");
        assert_eq!(row["createdAt"], json!(thread.created_at));
        // It does not move with the run's activity, which is the whole point.
        assert_ne!(row["createdAt"], row["lastMessageAt"]);
    }

    /// The workspace's *name*, not its id: the merged list groups remote sessions
    /// by workspace, and a group labelled with an opaque id tells the reader
    /// nothing while a missing label collapses several workspaces into one.
    #[test]
    fn sessions_snapshot_carries_the_workspace_name() {
        let _home = crate::remote::test_support::HomeGuard::new("catalog-workspace-name");
        crate::remote::test_support::init_store();

        let thread = new_thread("sess-named", "Named");

        let (payload, _signature) = sessions("pair_test").expect("snapshot");
        let row = payload["sessions"]
            .as_array()
            .expect("sessions array")
            .iter()
            .find(|row| row["sessionId"] == json!("sess-named"))
            .expect("session in snapshot");
        assert_eq!(row["workspaceId"], json!(thread.workspace_id));
        // The name the store gave the workspace it created for that path — the
        // row's own name, not its id (see `create_thread`).
        assert_eq!(row["workspaceName"], json!("Named Workspace"));
        assert_eq!(row["workspacePinned"], json!(false));
    }

    /// A workspace group's *own* place, not its conversations'.
    ///
    /// The client orders a workspace group by this field and never by its rows,
    /// so it has to be the key *this* desktop orders its own workspace list by
    /// (`future_app_workspaces::list`: `COALESCE(last_opened_at, updated_at)`) —
    /// and it has to stay put when a conversation inside the workspace gets a
    /// message, which is the whole reason it is not the conversation's timestamp.
    #[test]
    fn sessions_snapshot_carries_the_workspaces_own_place() {
        let _home = crate::remote::test_support::HomeGuard::new("catalog-workspace-order");
        crate::remote::test_support::init_store();

        let thread = new_thread("sess-ordered", "Ordered");
        let workspace = crate::store::get_workspace(&thread.workspace_id)
            .expect("read the workspace")
            .expect("the workspace create_thread made");
        let expected = workspace.last_opened_at.unwrap_or(workspace.updated_at);

        let (payload, _signature) = sessions("pair_test").expect("snapshot");
        let row = payload["sessions"]
            .as_array()
            .expect("sessions array")
            .iter()
            .find(|row| row["sessionId"] == json!("sess-ordered"))
            .expect("session in snapshot");
        assert_eq!(row["workspaceOrderedAt"], json!(expected));

        record_thread_message_activity(&thread.id, 9_000).expect("record activity");
        let (payload, _signature) = sessions("pair_test").expect("snapshot");
        let row = payload["sessions"]
            .as_array()
            .expect("sessions array")
            .iter()
            .find(|row| row["sessionId"] == json!("sess-ordered"))
            .expect("session in snapshot");
        assert_eq!(row["lastMessageAt"], json!(9_000));
        assert_eq!(
            row["workspaceOrderedAt"],
            json!(expected),
            "a message in the workspace must not move the workspace itself"
        );
    }

    /// A pinned workspace says so on its conversations.
    ///
    /// The client orders its merged list by this: a group whose workspace is
    /// pinned leads, and a workspace on *this* machine is pinned here — so the
    /// field is the only way a pin set in this app's own list can move the group
    /// in a paired client's list. Without it the client's "pin" writes its own
    /// store and the group never moves.
    #[test]
    fn sessions_snapshot_marks_a_pinned_workspace() {
        let _home = crate::remote::test_support::HomeGuard::new("catalog-workspace-pinned");
        crate::remote::test_support::init_store();

        let thread = new_thread("sess-pinned-ws", "Pinned workspace");
        let workspace_id = thread.workspace_id.clone();
        crate::store::pin_workspace(crate::store::PinWorkspaceInput {
            workspace_id: workspace_id.clone(),
            pinned: true,
        })
        .expect("pin the workspace");

        let (payload, _signature) = sessions("pair_test").expect("snapshot");
        let row = payload["sessions"]
            .as_array()
            .expect("sessions array")
            .iter()
            .find(|row| row["sessionId"] == json!("sess-pinned-ws"))
            .expect("session in snapshot");
        assert_eq!(row["workspaceId"], json!(workspace_id));
        assert_eq!(row["workspacePinned"], json!(true));
    }

    /// A shelved conversation says so on its own row, and stays in the snapshot.
    ///
    /// The phone draws its archived view from this same catalogue (`list_threads`
    /// returns everything but deleted rows), so archiving must not remove the row
    /// — it marks it. The desktop's default lists hide it; a client that reads
    /// the flag can hide it too, and one that predates the field lists the row as
    /// active, which is what it did before.
    #[test]
    fn sessions_snapshot_marks_an_archived_conversation() {
        let _home = crate::remote::test_support::HomeGuard::new("catalog-thread-archived");
        crate::remote::test_support::init_store();

        let thread = new_thread("sess-archived", "Shelved");
        let archived = |payload: &serde_json::Value| {
            payload["sessions"]
                .as_array()
                .expect("sessions array")
                .iter()
                .find(|row| row["sessionId"] == json!("sess-archived"))
                .map(|row| row["archived"].clone())
                .expect("the row stays in the catalogue after archiving")
        };

        let (payload, _signature) = sessions("pair_test").expect("snapshot");
        assert_eq!(archived(&payload), json!(false));

        crate::store::archive_thread_tree(&thread.id, &[]).expect("archive the conversation");

        let (payload, _signature) = sessions("pair_test").expect("snapshot");
        assert_eq!(archived(&payload), json!(true));

        crate::store::restore_thread(&thread.id).expect("restore the conversation");

        let (payload, _signature) = sessions("pair_test").expect("snapshot");
        assert_eq!(archived(&payload), json!(false));
    }

    /// A shelved workspace's conversations are marked on their rows.
    ///
    /// Archiving a workspace leaves its conversations' own status alone — that is
    /// what makes restoring it exact — so the flag has to come from the workspace
    /// row. Without this a client's default list would show the conversations of
    /// a workspace it had already hidden: rows under no heading, in a list that
    /// no longer draws the heading.
    #[test]
    fn sessions_snapshot_marks_conversations_of_an_archived_workspace() {
        let _home = crate::remote::test_support::HomeGuard::new("catalog-workspace-archived");
        crate::remote::test_support::init_store();

        let (workspace, thread) = new_workspace_thread("sess-in-shelf", "Inside the shelf");
        // Its own status stays active throughout: only the container is shelved.
        assert_ne!(thread.status, "archived");

        crate::store::archive_workspace(&workspace.id, &[]).expect("archive the workspace");

        let (payload, _signature) = sessions("pair_test").expect("snapshot");
        let row = payload["sessions"]
            .as_array()
            .expect("sessions array")
            .iter()
            .find(|row| row["sessionId"] == json!("sess-in-shelf"))
            .expect("session in snapshot");
        assert_eq!(
            row["archived"],
            json!(true),
            "a workspace's shelf covers its conversations"
        );
        assert_eq!(
            crate::store::get_thread(&thread.id)
                .expect("read the thread")
                .expect("the thread")
                .status,
            "active",
            "archiving a workspace must not rewrite its conversations"
        );
    }

    /// The workspace snapshot carries the flag as a boolean beside `archivedAt`.
    ///
    /// A client should not have to know that "archived" is spelled as a
    /// timestamp, and the phone pairs a shelved workspace heading with the
    /// conversations the sessions snapshot marks for it.
    #[test]
    fn workspaces_snapshot_marks_an_archived_workspace() {
        let _home = crate::remote::test_support::HomeGuard::new("catalog-workspace-flag");
        crate::remote::test_support::init_store();

        let (workspace, _thread) = new_workspace_thread("sess-shelf-flag", "Flagged");
        let archived = |payload: &serde_json::Value| {
            payload["workspaces"]
                .as_array()
                .expect("workspaces array")
                .iter()
                .find(|row| row["id"] == json!(workspace.id))
                .map(|row| row["archived"].clone())
                .expect("the workspace in the snapshot")
        };

        let (payload, _signature) = workspaces().expect("snapshot");
        assert_eq!(archived(&payload), json!(false));

        crate::store::archive_workspace(&workspace.id, &[]).expect("archive");

        let (payload, _signature) = workspaces().expect("snapshot");
        assert_eq!(archived(&payload), json!(true));
    }

    /// A thread without an agent session is not a remote session at all, and a
    /// deleted one never reaches the snapshot — `lastMessageAt` must not become
    /// a way to sneak either of them back in.
    #[test]
    fn sessions_snapshot_still_omits_unbound_threads() {
        let _home = crate::remote::test_support::HomeGuard::new("catalog-unbound");
        crate::remote::test_support::init_store();

        create_thread(CreateThreadInput {
            mode: "chat".into(),
            title: Some("No agent session yet".into()),
            workspace_id: None,
            workspace_path: Some("/tmp/catalog-snapshot".into()),
            workspace_name: Some("catalog".into()),
            agent_session_id: None,
        })
        .expect("create unbound thread");

        let (payload, _signature) = sessions("pair_test").expect("snapshot");
        assert!(payload["sessions"]
            .as_array()
            .expect("sessions array")
            .is_empty());
    }

    /// The split the catalog lane is built on: a run's activity timestamp
    /// moving is a change, but it is *not* structure.
    ///
    /// Asserted from both sides because both halves are load-bearing. If
    /// recency leaked into the structural signature, the publisher would send a
    /// whole session list per second of streaming (the measured 94–173 MB this
    /// lane exists to remove). If recency stopped moving the *whole* signature,
    /// the revision would stop advancing and a client that missed a recency
    /// packet would never learn it had — which is the one thing the heartbeat's
    /// revision check exists to notice.
    #[test]
    fn recency_moves_the_revision_but_not_the_structural_signature() {
        let _home = crate::remote::test_support::HomeGuard::new("catalog-signatures");
        crate::remote::test_support::init_store();
        let thread = new_thread("sess-sig", "Sig");
        let revision = |catalog: &SessionsCatalog| {
            catalog.payload["version"]["revision"]
                .as_u64()
                .expect("every snapshot carries a revision")
        };

        let first = sessions_catalog("pair_sig").expect("store readable");
        // An unchanged store is not a change: re-reading it must not move the
        // revision, or the heartbeat would ask every client to pull on every
        // tick.
        let again = sessions_catalog("pair_sig").expect("store readable");
        assert_eq!(first.structural, again.structural);
        assert_eq!(first.signature, again.signature);
        assert_eq!(revision(&first), revision(&again));

        record_thread_message_activity(&thread.id, 5_000).expect("record activity");
        let moved = sessions_catalog("pair_sig").expect("store readable");
        assert_eq!(
            moved.structural, first.structural,
            "a row's activity timestamp is ordering, not structure"
        );
        assert_ne!(
            moved.signature, first.signature,
            "recency is still content the client has to be told about"
        );
        assert!(
            revision(&moved) > revision(&first),
            "the heartbeat's revision must move, or a lost packet is invisible"
        );
        // The row is emitted for the patch, with the value the client orders by
        // — the same one the whole snapshot carries.
        assert_eq!(moved.activity, vec![("sess-sig".to_string(), 5_000)]);
        assert_eq!(moved.payload["sessions"][0]["lastMessageAt"], json!(5_000));
        assert_eq!(
            moved.payload["sessions"][0]["lastMessageAt"],
            json!(moved.activity[0].1)
        );

        // A rename is structure, so it is what a whole snapshot is for.
        crate::store::rename_thread(crate::store::RenameThreadInput {
            thread_id: thread.id.clone(),
            title: "Renamed".to_string(),
        })
        .expect("rename");
        let renamed = sessions_catalog("pair_sig").expect("store readable");
        assert_ne!(renamed.structural, moved.structural);
        assert_ne!(renamed.signature, moved.signature);
    }

    /// A session with no activity timestamp at all (a thread that has never
    /// received a message writes `updated_at`) is still patched: the coalesce
    /// is what the store itself orders by, and a client that keeps a stale
    /// value would rank that row by a clock nobody else uses.
    #[test]
    fn activity_carries_the_same_coalesce_the_store_orders_by() {
        let _home = crate::remote::test_support::HomeGuard::new("catalog-activity-coalesce");
        crate::remote::test_support::init_store();
        let thread = new_thread("sess-coalesce", "Coalesce");

        let catalog = sessions_catalog("pair_act").expect("store readable");
        assert_eq!(
            catalog.activity,
            vec![("sess-coalesce".to_string(), thread.updated_at)]
        );
    }

    #[test]
    fn versions_follow_exact_snapshot_content_not_read_or_publish_time() {
        let mut previous = (0, String::new());
        let mut first = json!({"workspaces":[{"id":"w", "name":"same", "path":"/a"}]});
        version(&mut first, "source", &mut previous, "workspaces");
        let original = first["version"].clone();
        version(&mut first, "source", &mut previous, "workspaces");
        assert_eq!(first["version"], original);
        first["workspaces"][0]["path"] = json!("/b");
        version(&mut first, "source", &mut previous, "workspaces");
        assert_eq!(first["version"]["revision"], 2);
        assert_eq!(first["version"]["epoch"], "source");
    }

    /// A snapshot that is not a list has nothing to strip, so both signatures
    /// describe the same shape: the structural signature must not claim a
    /// change the publisher would then send as "content moved".
    #[test]
    fn a_snapshot_that_is_not_a_list_has_one_signature() {
        let (whole, structural) = row_signatures(&json!({"sessions": "not a list"}));
        assert_eq!(whole, structural);
        assert_eq!(whole, json!({"sessions": "not a list"}).to_string());

        // A list still gets its recency stripped from the structural form, and
        // only from that one.
        let (whole, structural) = row_signatures(&json!([
            {"sessionId": "s1", "lastMessageAt": 7},
        ]));
        assert!(whole.contains("lastMessageAt"));
        assert!(
            !structural.contains("lastMessageAt"),
            "the structural signature drops recency: {structural}"
        );
    }
}
