use super::*;
use std::sync::{mpsc, Arc};
use std::time::Duration;

#[test]
fn historical_and_current_disk_replay_release_broadcast_locks_while_waiting_for_flush() {
    for mode in ["historical", "current", "attach"] {
        let directory = tempfile::tempdir().unwrap();
        let manager = crate::session::Manager::new(directory.path().to_path_buf());
        let broadcaster = Arc::new(SseBroadcaster::new());
        broadcaster
            .configure_legacy_journal("session-read-lock", &manager)
            .unwrap();
        broadcaster.start_run("live".into(), 0);
        broadcaster.broadcast(SseEvent::new("agent_start", serde_json::json!({})));
        broadcaster.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text":"before"}),
        ));
        manager
            .storage()
            .unwrap()
            .append_event(
                "session-read-lock",
                serde_json::to_value(SseEvent {
                    run_id: "old".into(),
                    idx: 0,
                    ..SseEvent::new("agent_end", serde_json::json!({}))
                })
                .unwrap(),
            )
            .unwrap();
        if mode != "historical" {
            // Force a real current-run backfill without filling the entire ring.
            broadcaster.run.lock().events.remove(0);
        }
        let (queued, waiting) = mpsc::channel();
        let gate = {
            let mut journal = broadcaster.journal.lock();
            journal.read_flush_queued = Some(queued);
            journal.writer.as_ref().unwrap().hold_worker_for_test()
        };
        gate.wait();
        let reader = broadcaster.clone();
        let read = std::thread::spawn(move || {
            if mode == "attach" {
                let attached = reader.attach("live", -1)?;
                Ok::<_, anyhow::Error>((
                    "live".to_owned(),
                    attached.events,
                    Some(attached.receiver),
                ))
            } else {
                let page = reader.events_page(
                    if mode == "historical" { "old" } else { "live" },
                    -1,
                    Some(10),
                )?;
                Ok((page.0, page.1, None))
            }
        });
        let queued = waiting.recv_timeout(Duration::from_secs(5));
        let run_free = broadcaster.run.try_lock().is_some();
        let journal_free = broadcaster.journal.try_lock().is_some();
        let mut receiver = broadcaster.subscribe();
        let mut published = false;
        if queued.is_ok() && run_free && journal_free {
            // Buffered appends can publish while a prior read's flush waits.
            broadcaster.broadcast(SseEvent::new(
                "text_chunk",
                serde_json::json!({"text":"during read"}),
            ));
            published = receiver.try_recv().is_ok();
        }
        // Release before assertions so a regression cannot strand the writer.
        gate.wait();
        let mut result = read.join().unwrap().unwrap();
        assert!(queued.is_ok(), "reader must reach the flush boundary");
        assert!(run_free, "disk replay must release the current run lock");
        assert!(
            journal_free,
            "flush waits must release the journal-state lock"
        );
        assert!(
            published,
            "a pending replay must allow another chunk to publish"
        );
        assert_eq!(result.0, if mode == "historical" { "old" } else { "live" });
        assert!(result.1.iter().any(|event| event.idx == 0));
        if mode == "attach" {
            assert!(
                result.1.iter().all(|event| event.idx <= 1),
                "backfill must stop at the subscription boundary"
            );
            assert_eq!(
                result.2.as_mut().unwrap().try_recv().unwrap().idx,
                2,
                "the concurrent chunk must arrive on the registered receiver"
            );
        }
    }
}
