//! Explicit reference-host acceptance for production admission, timer batching
//! and the independent control lane. Never opens a user's data directory.
use super::*;
use crate::session::sqlite_store::SqliteStore;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

fn stats(mut samples: Vec<u64>) -> Value {
    samples.sort_unstable();
    json!({"count":samples.len(),"p50":samples[samples.len()/2],"p95":samples[(samples.len()*95/100).min(samples.len()-1)],"p99":samples[(samples.len()*99/100).min(samples.len()-1)],"max":samples.last()})
}

#[test]
#[ignore = "reference-host acceptance: native UUID event admission, 25ms window and control response at 1/4/16 sessions"]
fn paced_native_admission_and_control_latency() {
    let mut cases = Vec::new();
    for sessions in [1, 4, 16] {
        let root = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&root.path().join("agent.db")).unwrap();
        let ids: Vec<_> = (0..sessions)
            .map(|_| {
                (
                    uuid::Uuid::new_v4().to_string(),
                    uuid::Uuid::new_v4().to_string(),
                    uuid::Uuid::new_v4().to_string(),
                )
            })
            .collect();
        for (session, run, _) in &ids {
            store.reserve_segmented(session).unwrap();
            store.replace(session,vec![json!({"id":uuid::Uuid::new_v4().to_string(),"type":"session_info","timestamp":"2026-10-27T00:00:00Z","content":{"model":"synthetic","cwd":"."}}),json!({"id":uuid::Uuid::new_v4().to_string(),"type":"run_started","timestamp":"2026-10-27T00:00:00Z","content":{"run_id":run,"epoch":1,"run_sequence":0}})]).unwrap();
        }
        let barrier = Arc::new(std::sync::Barrier::new(sessions));
        let workers:Vec<_>=ids.into_iter().map(|(session,run,entry)| {
            let store=store.clone();let barrier=barrier.clone();
            std::thread::spawn(move || {
                let health=Arc::new(EventJournalHealth::default());
                let mut writer=EventBatchWriter::new(store.clone(),session.clone(),health.clone()).unwrap();
                let observations=Arc::new(parking_lot::Mutex::new(VecDeque::<(i64,Instant)>::new()));
                let done=Arc::new(AtomicBool::new(false));
                let (pending,finished,read_store,read_session,read_run)=(observations.clone(),done.clone(),store.clone(),session.clone(),run.clone());
                let monitor=std::thread::spawn(move || {
                    let start=Instant::now();let mut lags=Vec::new();let mut reads=Vec::new();
                    loop {
                        assert!(start.elapsed()<Duration::from_secs(15),"event persistence monitor did not drain");
                        let read=Instant::now();let cursor=read_store.run_replay_watermark(&read_session,&read_run).unwrap().unwrap_or(-1);reads.push(read.elapsed().as_micros() as u64);
                        let mut queue=pending.lock();
                        while queue.front().is_some_and(|(idx,_)|*idx<=cursor) {let (_,admitted)=queue.pop_front().unwrap();lags.push(admitted.elapsed().as_micros() as u64);}
                        if finished.load(Ordering::Acquire)&&queue.is_empty(){break;}
                        drop(queue);std::thread::sleep(Duration::from_millis(2));
                    }
                    (lags,reads)
                });
                let key=format!("{entry}:thinking:0");let mut cursor=0i64;let mut enqueues=Vec::new();let mut peak_bytes=0u64;let mut peak_age=0u64;
                let mut enqueue=|kind:&str,data:Value,durable:bool| {
                    let idx=cursor;cursor+=1;
                    let event=json!({"session_id":session,"run_id":run,"epoch":1,"idx":idx,"session_idx":-1,"run_sequence":0,"event_id":format!("{session}:{run}:1:{idx}"),"timestamp":"2026-10-27T00:00:01Z","event_type":kind,"data":data.to_string()});
                    let bytes=event.to_string().len();let admitted=Instant::now();
                    let receipt=writer.enqueue(event,durable,bytes).unwrap();
                    enqueues.push(admitted.elapsed().as_micros() as u64);observations.lock().push_back((idx,admitted));
                    if let Some(receipt)=receipt {receipt.recv().unwrap().unwrap();}
                    let state=writer.admission.snapshot();peak_bytes=peak_bytes.max(state["pendingBytes"].as_u64().unwrap());peak_age=peak_age.max(state["oldestPendingMicros"].as_u64().unwrap());
                };
                barrier.wait();
                enqueue("message_start",json!({"entry_id":entry}),false);
                enqueue("thinking_start",json!({"entry_id":entry,"content_key":key,"content_offset":0,"content_bytes":0}),false);
                let fragment="思考";let mut offset=0usize;
                for _ in 0..64 {
                    for _ in 0..8 {enqueue("thinking_delta",json!({"entry_id":entry,"content_key":key,"content_offset":offset,"content_bytes":fragment.len(),"text":fragment}),false);offset+=fragment.len();}
                    std::thread::sleep(Duration::from_millis(4));
                }
                for _ in 0..64 {enqueue("thinking_delta",json!({"entry_id":entry,"content_key":key,"content_offset":offset,"content_bytes":fragment.len(),"text":fragment}),false);offset+=fragment.len();}
                let control=Instant::now();writer.control.queue_flush().unwrap().recv().unwrap().unwrap();let control_micros=control.elapsed().as_micros();
                enqueue("thinking_end",json!({"entry_id":entry,"content_key":key,"content_offset":offset,"content_bytes":0,"content_seal":true,"provider_metadata":{}}),true);
                enqueue("message_end",json!({"entry_id":entry,"complete":true}),true);
                drop(enqueue);
                writer.shutdown().unwrap();done.store(true,Ordering::Release);
                let (lags,reads)=monitor.join().unwrap();
                assert!(health.error().is_none());assert_eq!(health.metrics.snapshot()["batchFailures"],0);assert_eq!(writer.admission.snapshot()["pendingEvents"],0);
                assert_eq!(store.run_replay_watermark(&session,&run).unwrap(),Some(cursor-1));
                let message=crate::types::AgentMessage {role:"assistant".into(),content:vec![crate::types::ContentBlock::reasoning(fragment.repeat(576),Default::default())],metadata:Some([(crate::types::AgentMessage::JOURNAL_ENTRY_ID_KEY.into(),json!(entry)),(crate::types::AgentMessage::STREAM_BLOCKS_KEY.into(),json!([key]))].into_iter().collect()),..Default::default()};
                store.append_stream_entry(&session,crate::session::content::stream_entry::StreamEntry::from_assistant(&message,&run,1).unwrap()).unwrap();
                let actual=store.entries(&session).unwrap();assert_eq!(actual.iter().find(|e|e["id"]==entry).unwrap()["content"][0]["text"],fragment.repeat(576));
                json!({"enqueueMicros":stats(enqueues),"observedAdmissionToDurableMicros":stats(lags),"controlFlushMicros":control_micros,"readerMicros":stats(reads),"peakPendingBytes":peak_bytes,"peakOldestPendingMicros":peak_age,"health":health.metrics.snapshot()})
            })
        }).collect();
        let results: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        cases.push(json!({"sessions":sessions,"workers":results,"databaseIo":store.db.metrics()}));
    }
    let report = json!({"workload":"production-event-admission-25ms-native-UUID","cases":cases,"scope":"Real EventBatchWriter enqueue/admission, natural 25ms timer and durable control fence. Eight fragments every 4ms for 64 ticks plus a 64-fragment burst per session; shared ordered SQLite writer and independent watermark observers. Observed durable lag includes up to one 2ms poll interval plus read scheduling. Does not include broadcaster projection, gRPC, Tauri IPC or UI frames. One reference-host run per session count."});
    println!("PACED_ADMISSION_REPORT={report}");
    if let Some(path) = std::env::var_os("FUTURE_PACED_ADMISSION_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
