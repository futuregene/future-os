//! Synthetic public-lifecycle workloads shaped by aggregate historical counts.
//! Contains no user prompts, generated text, provider identifiers or arguments.
use super::*;
use crate::console;

fn id(n: usize) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}
fn stats(mut values: Vec<u64>) -> Value {
    values.sort_unstable();
    json!({"count":values.len(),"p50":values[values.len()/2],"p95":values[(values.len()*95/100).min(values.len()-1)],"p99":values[(values.len()*99/100).min(values.len()-1)],"max":values.last()})
}
struct Scenario {
    store: SqliteStore,
    writer: Writer,
    session: String,
    run: String,
    cursor: i64,
    latencies: Vec<u64>,
    fragments: usize,
}
impl Scenario {
    fn persist(&mut self, events: Vec<(&str, Value)>) {
        let events: Vec<Value> = events.into_iter().map(|(kind,data)| {
            let cursor = self.cursor;
            self.cursor += 1;
            json!({"session_id":self.session,"run_id":self.run,"epoch":1,"idx":cursor,"session_idx":-1,"event_id":format!("{}:{}:1:{cursor}",self.session,self.run),"run_sequence":0,"timestamp":"2026-10-27T00:00:01Z","event_type":kind,"data":data.to_string()})
        }).collect();
        let start = Instant::now();
        self.writer
            .persist(&self.store, &self.session, &events)
            .unwrap();
        self.latencies.push(start.elapsed().as_micros() as u64);
    }
    fn body(&mut self, entry: &str, key: &str, text: &str, kind: &'static str, tool: Option<&str>) {
        let mut offset = 0;
        while offset < text.len() {
            let mut batch = Vec::new();
            for _ in 0..8 {
                if offset == text.len() {
                    break;
                }
                let mut end = (offset + 4).min(text.len());
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                let mut data = json!({"entry_id":entry,"content_key":key,"content_offset":offset,"content_bytes":end-offset,"text":&text[offset..end]});
                if let Some(tool) = tool {
                    data["tool_id"] = json!(tool);
                    data["snapshot"] = json!(false);
                }
                batch.push((kind, data));
                self.fragments += 1;
                offset = end;
            }
            self.persist(batch);
        }
    }
}
fn run(tool_calls: usize) -> Value {
    run_sized(tool_calls, None)
}
fn tool_output(index: usize, bytes: Option<usize>) -> String {
    bytes.map_or_else(
        || "synthetic output\n".repeat(if index.is_multiple_of(10) { 1024 } else { 4 }),
        |bytes| "x".repeat(bytes),
    )
}
fn run_sized(tool_calls: usize, large_tool_bytes: Option<usize>) -> Value {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("agent.db");
    let store = SqliteStore::open(&path).unwrap();
    let session = id(1);
    let run = id(2);
    store.reserve_segmented(&session).unwrap();
    store.replace(&session,vec![
        json!({"id":id(3),"type":"session_info","timestamp":"2026-10-27T00:00:00Z","content":{"cwd":".","model":"synthetic"}}),
        json!({"id":id(4),"type":"user","role":"user","timestamp":"2026-10-27T00:00:00Z","content":"Synthetic historical-distribution workload"}),
        json!({"id":id(5),"type":"run_started","timestamp":"2026-10-27T00:00:00Z","content":{"run_id":run,"epoch":1,"run_sequence":0}}),
    ]).unwrap();
    let before = WalWrites::capture(&store);
    let mut scenario = Scenario {
        store,
        writer: Writer::default(),
        session: session.clone(),
        run: run.clone(),
        cursor: 0,
        latencies: vec![],
        fragments: 0,
    };
    let mut finalizations = Vec::new();
    let mut tool_finalizations = Vec::new();
    let mut publication_added_bytes = 0;
    let mut body_bytes = 0usize;
    let start = Instant::now();
    scenario.persist(vec![("agent_start", json!({}))]);
    for i in 0..tool_calls {
        let entry = id(100 + i * 2);
        let tool = format!("provider-tool-{i:04}");
        let thinking = "思考".repeat([3, 47, 91, 212, 909][i % 5]);
        let args =
            json!({"command":format!("synthetic-{}","x".repeat([8,180,330,1500,7500][i%5]))});
        let raw_args = args.to_string();
        let thinking_key = format!("{entry}:thinking:0");
        let tool_key = format!("{entry}:1:tool:0");
        scenario.persist(vec![("message_start",json!({"entry_id":entry})),("thinking_start",json!({"entry_id":entry,"content_key":thinking_key,"content_offset":0,"content_bytes":0,"block_id":"0"}))]);
        scenario.body(&entry, &thinking_key, &thinking, "thinking_delta", None);
        scenario.persist(vec![
            ("thinking_end",json!({"entry_id":entry,"content_key":thinking_key,"content_offset":thinking.len(),"content_bytes":0,"content_seal":true,"provider_metadata":{}})),
            ("tool_start",json!({"entry_id":entry,"phase":"input","tool_id":tool,"tool_name":"shell","tc_index":0,"content_key":tool_key,"content_offset":0,"content_bytes":0,"tool_args":""})),
        ]);
        scenario.body(&entry, &tool_key, &raw_args, "tool_delta", Some(&tool));
        scenario.persist(vec![
            ("tool_input_end",json!({"entry_id":entry,"tool_id":tool,"tool_name":"shell","tc_index":0,"content_key":tool_key,"content_offset":raw_args.len(),"content_bytes":0,"content_seal":true,"tool_args":args,"provider_metadata":{}})),
            ("message_end",json!({"entry_id":entry,"complete":true})),
        ]);
        let message = AgentMessage {
            role: "assistant".into(),
            content: vec![
                ContentBlock::reasoning(&thinking, Default::default()),
                ContentBlock::tool_call(&tool, "shell", args.clone(), Default::default()),
            ],
            metadata: Some(
                [
                    (AgentMessage::JOURNAL_ENTRY_ID_KEY.into(), json!(entry)),
                    (
                        AgentMessage::STREAM_BLOCKS_KEY.into(),
                        json!([thinking_key, null]),
                    ),
                ]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        };
        let finish = Instant::now();
        scenario
            .store
            .append_stream_entry(
                &session,
                StreamEntry::from_assistant(&message, &run, 1).unwrap(),
            )
            .unwrap();
        finalizations.push(finish.elapsed().as_micros() as u64);
        let output = tool_output(i, large_tool_bytes);
        scenario.persist(vec![(
            "tool_end",
            json!({"tool_id":tool,"tool_name":"shell","text":output,"is_error":false}),
        )]);
        let before_publication = if large_tool_bytes.is_some() {
            scenario
                .store
                .db
                .read(|db| {
                    Ok(db.query_row(
                        "SELECT coalesce(sum(byte_length),0) FROM content_objects",
                        [],
                        |r| r.get::<_, i64>(0),
                    )?)
                })
                .unwrap()
        } else {
            0
        };
        let tool_finish = Instant::now();
        scenario.store.append(&session,vec![json!({"id":id(101+i*2),"type":"tool","role":"tool","timestamp":"2026-10-27T00:00:02Z","meta":{"run_id":run},"content":[{"type":"tool_result","tool_call_id":tool,"content":output,"is_error":false}]})]).unwrap();
        tool_finalizations.push(tool_finish.elapsed().as_micros() as u64);
        if large_tool_bytes.is_some() {
            let after = scenario
                .store
                .db
                .read(|db| {
                    Ok(db.query_row(
                        "SELECT coalesce(sum(byte_length),0) FROM content_objects",
                        [],
                        |r| r.get::<_, i64>(0),
                    )?)
                })
                .unwrap();
            assert_eq!(
                after, before_publication,
                "final tool callback copied an already durable body"
            );
            publication_added_bytes += after - before_publication;
        }
        body_bytes += thinking.len() + raw_args.len() + output.len();
    }
    let finish = Instant::now();
    scenario.store.commit_terminal_entries(&session,vec![json!({"id":id(10000),"type":"run_terminal","timestamp":"2026-10-27T00:00:03Z","content":{"run_id":run,"state":"completed","run_tokens":0}})]).unwrap();
    let terminal_micros = finish.elapsed().as_micros();
    let elapsed = start.elapsed().as_micros();
    let writes = WalWrites::capture(&scenario.store).since(before);
    let allocation = scenario
        .store
        .db
        .read(|db| {
            let mut query = db.prepare(
                "SELECT name,sum(pgsize),sum(payload) FROM dbstat GROUP BY name ORDER BY name",
            )?;
            let rows = query.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    json!({"allocatedBytes":r.get::<_,i64>(1)?,"payloadBytes":r.get::<_,i64>(2)?}),
                ))
            })?;
            Ok(Value::Object(
                rows.collect::<rusqlite::Result<serde_json::Map<_, _>>>()?,
            ))
        })
        .unwrap();
    let io = scenario.store.db.metrics();
    let files = [&path, &path.with_extension("db-wal")]
        .iter()
        .map(|p| std::fs::metadata(p).map_or(0, |m| m.len()))
        .sum::<u64>();
    let batch = stats(scenario.latencies);
    let fragments = scenario.fragments;
    drop(scenario.store);
    let reopen = Instant::now();
    let store = SqliteStore::open(&path).unwrap();
    let reopen_micros = reopen.elapsed().as_micros();
    let first = Instant::now();
    let snapshot = reader::Snapshot::capture(&store, &session).unwrap();
    let page = snapshot.page(&store, -1).unwrap();
    let first_entry = page.iter().find(|e| e.id == id(100)).unwrap();
    let first_body = snapshot.entry(&store, first_entry).unwrap();
    let first_micros = first.elapsed().as_micros();
    assert_eq!(first_body["content"][0]["text"], "思考".repeat(3));
    let mut reads = Vec::new();
    let mut returned = 0;
    let mut result_bytes = 0;
    let mut logical_digest = String::new();
    for _ in 0..if large_tool_bytes.is_some() { 1 } else { 5 } {
        let start = Instant::now();
        let entries = store.entries(&session).unwrap();
        reads.push(start.elapsed().as_micros() as u64);
        assert_eq!(
            entries.iter().filter(|e| e["type"] == "assistant").count(),
            tool_calls
        );
        assert_eq!(
            entries.iter().filter(|e| e["type"] == "tool").count(),
            tool_calls
        );
        for i in 0..tool_calls {
            let entry = entries.iter().find(|e| e["id"] == id(100 + i * 2)).unwrap();
            assert_eq!(
                entry["content"][0]["text"],
                "思考".repeat([3, 47, 91, 212, 909][i % 5])
            );
            assert_eq!(
                entry["content"][1]["args"]["command"],
                format!("synthetic-{}", "x".repeat([8, 180, 330, 1500, 7500][i % 5]))
            );
            let tool = entries.iter().find(|e| e["id"] == id(101 + i * 2)).unwrap();
            assert_eq!(
                tool["content"][0]["content"],
                tool_output(i, large_tool_bytes)
            );
        }
        returned = entries.len();
        result_bytes = serde_json::to_vec(&entries).unwrap().len();
        use sha2::Digest;
        let logical: Vec<Value> = entries
            .iter()
            .map(|e| json!([e["id"], e["type"], e["role"], e["content"]]))
            .collect();
        let digest = format!(
            "{:x}",
            sha2::Sha256::digest(serde_json::to_vec(&logical).unwrap())
        );
        if !logical_digest.is_empty() {
            assert_eq!(logical_digest, digest);
        }
        logical_digest = digest;
    }
    json!({"largeToolOutputBytes":large_tool_bytes,"toolOutputPublicationMicros":stats(tool_finalizations),"toolPublicationAddedBodyBytes":publication_added_bytes,"toolCalls":tool_calls,"assistantMessages":tool_calls,"bodyBytes":body_bytes,"providerFragments":fragments,"fragmentBatchLimit":8,"writeAndPublicationMicros":elapsed,"batchMicros":batch,"assistantPublicationMicros":stats(finalizations),"terminalMicros":terminal_micros,"walWrites":writes,"databaseAndWalBytesBeforeClose":files,"structures":allocation,"io":io,"reopenMicros":reopen_micros,"firstPageAndFirstAssistantBodyMicros":first_micros,"firstIdentityPageEntries":page.len(),"completeTranscriptReadMicros":stats(reads),"completeTranscriptEntries":returned,"completeTranscriptSerializedBytes":result_bytes,"logicalTranscriptSha256":logical_digest})
}

#[test]
fn historical_distribution_valid_transcripts() {
    let report = json!({"workload":"historical-distribution-valid-UUID-transcripts","cases":[run(20),run(150)],"limitations":"Synthetic aggregate-shaped inputs, not replay of private history. Direct admitted batches, not timer/admission/UI latency. First read uses a reopened store with warm OS file cache. Complete transcript reads return the same logical messages/bodies; serialized envelope byte sizes may differ. No compression, VACUUM or history rewrite."});
    console::println!("REALISTIC_STORAGE_REPORT={report}");
    if let Some(path) = std::env::var_os("FUTURE_REALISTIC_STORAGE_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}

#[test]
fn large_tool_outputs_reuse_durable_bodies() {
    let report = json!({"workload":"valid-UUID-large-tool-outputs","cases":[run_sized(1,Some(1024*1024)),run_sized(1,Some(10*1024*1024))],"limitations":"Direct synthetic tool_end facts; explicit full transcript export reads, not inline UI transport. One sample per size; no latency percentile claim. Body hash verification is off the ordered database writer; no body compression or completion-time copying."});
    console::println!("LARGE_TOOL_STORAGE_REPORT={report}");
    if let Some(path) = std::env::var_os("FUTURE_LARGE_TOOL_STORAGE_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
