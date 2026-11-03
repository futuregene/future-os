//! Review recovery is a read of one conversation's recorded history. It never
//! captures today's workspace as an old Run's missing end, or settles Agent runs.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, Weak};

use crate::store;
use crate::Error;

use super::{build_last_run_review, LastRunReviewData};

static RUN_LOCKS: LazyLock<Mutex<HashMap<String, Weak<Mutex<()>>>>> =
    LazyLock::new(Default::default);

/// Serialize lazy reads, explicit retries and normal finalization for a Run.
/// Waiting is interruptible so a queued review cannot hold shutdown open.
pub(crate) fn with_run_review_lock<T>(
    run_id: &str,
    job: impl FnOnce() -> Result<T, Error>,
) -> Result<T, Error> {
    let lock = {
        let mut locks = RUN_LOCKS.lock().unwrap_or_else(|p| p.into_inner());
        locks.retain(|_, lock| lock.strong_count() > 0);
        locks
            .entry(run_id.to_string())
            .or_default()
            .upgrade()
            .unwrap_or_else(|| {
                let lock = Arc::new(Mutex::new(()));
                locks.insert(run_id.to_string(), Arc::downgrade(&lock));
                lock
            })
    };
    super::process::with_lock(&lock, job)
}

pub fn load_last_run_review(thread_id: &str) -> Result<Option<LastRunReviewData>, Error> {
    let Some(run_id) = store::latest_review_run(thread_id)? else {
        // A finalized projection can precede the local terminal-status update.
        return store::get_last_run_changeset(thread_id)?
            .map(build_last_run_review)
            .transpose();
    };
    with_run_review_lock(&run_id, || {
        let cached = store::get_run_changeset(&run_id)?;
        let before = store::get_review_snapshot(&run_id, "before")?;
        let after = store::get_review_snapshot(&run_id, "after")?;
        // Persisted file diffs are self-contained. An incomplete projection can
        // be rebuilt only if a genuine saved boundary arrived since that read.
        if let Some(changeset) = cached {
            let boundaries_ready = before
                .as_ref()
                .is_some_and(|s| s.status != "failed" && s.commit_id.is_some())
                && after
                    .as_ref()
                    .is_some_and(|s| s.status != "failed" && s.commit_id.is_some());
            let needs_projection = (changeset.completeness == "incomplete"
                || changeset.after_snapshot_id.is_none())
                && boundaries_ready;
            if !needs_projection {
                return Ok(Some(build_last_run_review(changeset)?));
            }
        }
        if before.is_none() {
            return Ok(None);
        }
        super::materialize_recorded(thread_id, &run_id, Vec::new())?;
        store::get_run_changeset(&run_id)?
            .map(build_last_run_review)
            .transpose()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::test_support::HomeGuard;
    use crate::store::{
        CreateRunInput, CreateThreadInput, CreateWorkspaceInput, UpdateRunStatusInput,
    };

    struct Fixture {
        _home: HomeGuard,
        dir: tempfile::TempDir,
        thread: store::ThreadRecord,
        run: store::RunRecord,
    }
    fn fixture(label: &str) -> Fixture {
        let home = HomeGuard::new(label);
        store::initialize_app_store().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let workspace = store::create_workspace(CreateWorkspaceInput {
            path: dir.path().to_string_lossy().into_owned(),
            name: Some(label.into()),
            description: None,
            create_directory: Some(false),
        })
        .unwrap();
        let thread = store::create_thread(CreateThreadInput {
            mode: "workspace".into(),
            title: Some(label.into()),
            workspace_id: Some(workspace.id),
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .unwrap();
        let run = store::create_run(CreateRunInput {
            id: None,
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        Fixture {
            _home: home,
            dir,
            thread,
            run,
        }
    }
    fn end(run: &str) {
        store::update_run_status_if_active(UpdateRunStatusInput {
            run_id: run.into(),
            status: "completed".into(),
            error_message: None,
            error_type: None,
        })
        .unwrap();
    }
    fn boundaries(f: &Fixture) {
        std::fs::write(f.dir.path().join("a.txt"), "before\n").unwrap();
        crate::agent_bridge::capture_before(&f.thread.id, &f.run.id);
        std::fs::write(f.dir.path().join("a.txt"), "before\nafter\n").unwrap();
        crate::agent_bridge::review::capture_after(&f.thread.id, &f.run.id);
    }

    #[test]
    fn opening_one_conversation_recovers_only_its_recorded_review() {
        let f = fixture("review-lazy-recorded");
        boundaries(&f);
        end(&f.run.id);
        let other = store::create_run(CreateRunInput {
            id: None,
            thread_id: f.thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        // A live newer Run must not be settled or given a synthetic end.
        crate::agent_bridge::capture_before(&f.thread.id, &other.id);
        let result =
            serde_json::to_value(load_last_run_review(&f.thread.id).unwrap().unwrap()).unwrap();
        assert_eq!(result["changeset"]["runId"], f.run.id);
        assert_eq!(result["changeset"]["filesChanged"], 1);
        assert!(!store::is_terminal_run_status(
            &store::get_run(&other.id).unwrap().unwrap().status
        ));
        assert!(store::get_run_changeset(&other.id).unwrap().is_none());
        assert!(store::get_review_snapshot(&other.id, "after")
            .unwrap()
            .is_none());
    }

    #[test]
    fn missing_after_is_incomplete_and_never_captures_todays_files() {
        let f = fixture("review-lazy-no-after");
        std::fs::write(f.dir.path().join("a.txt"), "before\n").unwrap();
        crate::agent_bridge::capture_before(&f.thread.id, &f.run.id);
        end(&f.run.id);
        std::fs::write(f.dir.path().join("a.txt"), "unrelated later edits\n").unwrap();
        let result =
            serde_json::to_value(load_last_run_review(&f.thread.id).unwrap().unwrap()).unwrap();
        assert_eq!(result["snapshotStatus"], "incomplete");
        assert_eq!(result["changeset"]["completeness"], "incomplete");
        assert!(store::get_review_snapshot(&f.run.id, "after")
            .unwrap()
            .is_none());
        assert_eq!(
            store::get_run(&f.run.id).unwrap().unwrap().status,
            "completed"
        );
        assert_eq!(
            std::fs::read_to_string(f.dir.path().join("a.txt")).unwrap(),
            "unrelated later edits\n"
        );
    }

    #[test]
    fn cached_review_survives_missing_workspace_and_shadow_objects() {
        let f = fixture("review-lazy-cached");
        boundaries(&f);
        end(&f.run.id);
        let first = serde_json::to_value(load_last_run_review(&f.thread.id).unwrap()).unwrap();
        let before = store::get_review_snapshot(&f.run.id, "before")
            .unwrap()
            .unwrap();
        let root = std::path::PathBuf::from(store::app_data_path().unwrap().app_dir)
            .join("review")
            .join(&before.workspace_id);
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(f.dir.path()).unwrap();
        let again = serde_json::to_value(load_last_run_review(&f.thread.id).unwrap()).unwrap();
        assert_eq!(first, again);
    }

    #[test]
    fn simultaneous_open_requests_share_one_projection() {
        let f = fixture("review-lazy-concurrent");
        boundaries(&f);
        end(&f.run.id);
        std::thread::scope(|scope| {
            let requests: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        serde_json::to_value(load_last_run_review(&f.thread.id).unwrap()).unwrap()
                    })
                })
                .collect();
            let results: Vec<_> = requests.into_iter().map(|r| r.join().unwrap()).collect();
            assert!(results.windows(2).all(|pair| pair[0] == pair[1]));
        });
    }

    #[test]
    fn latest_ended_run_without_a_snapshot_does_not_show_an_older_diff() {
        let f = fixture("review-lazy-latest");
        boundaries(&f);
        end(&f.run.id);
        load_last_run_review(&f.thread.id).unwrap();
        let next = store::create_run(CreateRunInput {
            id: None,
            thread_id: f.thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        end(&next.id);
        // End ordering is explicit so this does not depend on millisecond ticks.
        let conn = rusqlite::Connection::open(store::app_data_path().unwrap().db_path).unwrap();
        conn.execute("UPDATE runs SET ended_at = 1 WHERE id = ?1", [&f.run.id])
            .unwrap();
        conn.execute("UPDATE runs SET ended_at = 2 WHERE id = ?1", [&next.id])
            .unwrap();
        assert!(load_last_run_review(&f.thread.id).unwrap().is_none());
    }
}
