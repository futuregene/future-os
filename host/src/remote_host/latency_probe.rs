//! Opt-in open-latency probe: every read command a phone sends, run through the
//! real business dispatcher against an isolated snapshot of the real databases.
//!
//! What it measures, and why that is the number the audit needs:
//!
//! - **`handler_ms`** is the wall time of `business::execute` for one command.
//!   That is the desktop's half of "opening the conversation": the app-store
//!   read, the Agent round trip (which is where `agent.db` is read), the page
//!   trim, and the JSON the handler builds. It does **not** include the reply's
//!   publish/flush (transport) or the client's own render.
//! - **`encode_ms` / `reply_bytes`** are the production reply encoder
//!   (`reply::encode_reply_payload_with_gzip`) applied to the captured reply, so
//!   the byte column is the number a phone actually has to receive, not the
//!   handler's in-memory JSON.
//!
//! It runs *only* when explicitly named (`--ignored`) and only against a home
//! that declares itself a throwaway copy, so a mistaken `cargo test` can never
//! point it at the live databases. The runner is
//! `scripts/perf/remote-commands/measure-remote-latency.py`.
//!
//! The command list is data, not code: the runner derives the ids (heaviest
//! session, deepest run, median session, …) from the snapshot with SQL and
//! writes one spec per command per scale. Two phases are separate processes, so
//! every command gets a genuine first call:
//!
//! - `cold` — one iteration each, on an Agent that was started seconds ago;
//! - `warm` — warmup + N iterations each, on the same Agent.
use crate::console;
use future_remote_protocol::IncomingCmd;
use future_remote_server::services::ReplySink;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

/// One command to measure, plus what its numbers are to be read against.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProbeSpec {
    /// Stable name in the report, e.g. `get_session_entries:first_page:largest`.
    label: String,
    /// Which slice of real data this command touches. Free-form on purpose: the
    /// runner fills it from the snapshot (session id, entry count, event count,
    /// task id …) and the report quotes it verbatim, so a suspicious number can
    /// be traced back to the rows that produced it.
    #[serde(default)]
    scale: Value,
    /// Iterations after the first call. `1` is the cold phase.
    #[serde(default = "default_iterations")]
    iterations: usize,
    /// Iterations discarded before the measured ones.
    #[serde(default)]
    warmup: usize,
    /// Pause between iterations. Non-zero for the commands whose desktop-side
    /// answer is cached for a second (`list_sessions`, `list_workspaces`): the
    /// cold number is the store read, and a back-to-back warm loop would only
    /// measure the snapshot cache.
    #[serde(default)]
    spacing_ms: u64,
    /// Fill in `contentRequest` from a real page before measuring.
    ///
    /// A `get_event_content_range` request is not data the snapshot query can
    /// write: `databaseId`, `restoreGeneration`, `cursor` and `field` come from
    /// the `content_refs` the Agent attaches to an event whose body it did not
    /// inline, and only a real reply carries them. The probe therefore reads one
    /// page for the spec's `sessionId`/`runId` and lifts the reference out of it —
    /// the same field, read the same way a client builds the request. That read
    /// is *not* timed; it is the fixture, not the measurement.
    #[serde(default)]
    derive_content_request: bool,
    /// The wire command, exactly as `serde_json::from_slice` would parse it from
    /// a relayed message — `type` is the phone's own spelling.
    ///
    /// Kept as JSON rather than a parsed `IncomingCmd` because a command is not
    /// `Clone` (the bridge parses it out of the bytes it received, once per
    /// command): each iteration re-parses, so the parse is timed separately
    /// instead of being charged to the handler or quietly skipped.
    command: Value,
}

fn default_iterations() -> usize {
    8
}

#[derive(Default)]
struct Capture(Mutex<Option<Value>>);
impl ReplySink for Capture {
    fn send<'a>(
        &'a self,
        success: bool,
        data: Value,
        error: Option<String>,
    ) -> futures::future::BoxFuture<'a, ()> {
        Box::pin(async move {
            *self.0.lock().unwrap() = Some(json!({
                "success": success,
                "data": data,
                "error": error,
            }));
        })
    }
}

impl Capture {
    fn take(&self) -> Option<Value> {
        self.0.lock().unwrap().take()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Sample {
    ms: f64,
    /// `serde_json` parse of the command itself. Separated because a large
    /// `prompt`/`task` body would otherwise look like a slow store read.
    parse_ms: f64,
    encode_ms: f64,
    /// The reply as the encoder produced it — gzip applied, as a connection that
    /// declared `reply_gzip` receives it.
    reply_bytes: usize,
    /// The same reply before encoding, so an "is this a payload or a query
    /// problem" question has its answer next to the timing.
    json_bytes: usize,
    success: bool,
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SpecResult {
    label: String,
    command: String,
    /// `None` when the command JSON does not parse at all — that spec measured
    /// nothing and the report must not print a timing for it.
    parse_error: Option<String>,
    scale: Value,
    iterations: usize,
    spacing_ms: u64,
    /// The first call in this process. For the cold phase that is the only one.
    cold: Option<Sample>,
    /// The measured (post-warmup) series, sorted by nothing — kept in order so a
    /// trend inside one run is still visible.
    samples: Vec<Sample>,
    p50_ms: f64,
    p95_ms: f64,
    max_ms: f64,
    min_ms: f64,
    encode_p50_ms: f64,
    reply_bytes: usize,
    json_bytes: usize,
    /// `0` means no `data` field came back and no error either: a sink that was
    /// never called (a handler that returned without answering) would otherwise
    /// look like a very fast command.
    replies: usize,
}

/// Nearest-rank percentile over an ascending slice. With a handful of samples an
/// interpolated percentile invents a number no run produced; a measured one is
/// what a follow-up can reproduce.
fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (fraction * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

/// The handler half of one command: dispatch under the same per-request scopes a
/// production connection installs, then stop at the captured reply.
///
/// The two scopes are not decoration. `content_capability::in_request(true)` is
/// what makes a command that reads a session's stored format take the modern
/// path instead of the upgrade refusal, and `lean::in_request(true)` is what
/// makes `get_tool_call_args` reachable at all and gives history pages the trim
/// a current phone gets — without them the probe would measure a shape no
/// client ships.
async fn measure_once(spec: &ProbeSpec) -> (Option<Value>, Duration, Duration) {
    let sink = Capture::default();
    let parse_started = Instant::now();
    let command: IncomingCmd = match serde_json::from_value(spec.command.clone()) {
        Ok(command) => command,
        Err(error) => {
            return (
                Some(json!({"success": false, "data": Value::Null,
                            "error": format!("unparseable command: {error}")})),
                Duration::ZERO,
                parse_started.elapsed(),
            )
        }
    };
    let parse = parse_started.elapsed();
    let started = Instant::now();
    future_remote_server::content_capability::in_request(
        true,
        future_remote_server::lean::in_request(true, super::business::execute(command, &sink)),
    )
    .await;
    let elapsed = started.elapsed();
    (sink.take(), elapsed, parse)
}

/// Encode one captured reply the way a phone's connection would receive it.
fn encode(reply: &Value) -> (usize, usize, Duration) {
    let started = Instant::now();
    let json_bytes = serde_json::to_vec(reply)
        .map(|bytes| bytes.len())
        .unwrap_or(0);
    let encoded = future_remote_server::reply::encode_reply_payload_with_gzip(reply, true);
    (json_bytes, encoded.len(), started.elapsed())
}

fn sample_of(reply: Option<Value>, elapsed: Duration, parse: Duration) -> Sample {
    let (success, error) = match reply.as_ref() {
        Some(reply) => (
            reply["success"].as_bool().unwrap_or(false),
            reply["error"].as_str().map(str::to_owned),
        ),
        None => (false, Some("the handler never answered".to_string())),
    };
    let (json_bytes, reply_bytes, encode_elapsed) =
        reply.as_ref().map(encode).unwrap_or((0, 0, Duration::ZERO));
    Sample {
        ms: round3(elapsed.as_secs_f64() * 1000.0),
        parse_ms: round3(parse.as_secs_f64() * 1000.0),
        encode_ms: round3(encode_elapsed.as_secs_f64() * 1000.0),
        reply_bytes,
        json_bytes,
        success,
        error,
    }
}

/// Lift a `contentRequest` out of a real page for the spec's session/run.
///
/// The Agent only writes `content_refs` for a body it chose not to inline, so
/// this asks for the page with the flag that keeps those references (a plain
/// `get_events_since`, not the snapshot shortcut) and takes the largest one.
/// `content_refs` is already spelled as a `ContentRequest` (`databaseId`,
/// `restoreGeneration`, `epoch`, `cursor`, `field`, `offset`, `maxBytes`), which
/// is why no field is invented here.
async fn derive_content_request(command: &Value) -> Result<Value, String> {
    let discovery = json!({
        "type": "get_events_since",
        "sessionId": command["sessionId"],
        "runId": command["runId"],
        "sinceIdx": -1,
        "limit": 200,
        "chunkedRead": false,
        "preferSnapshot": false,
    });
    let probe = ProbeSpec {
        label: "content-request-discovery".into(),
        scale: Value::Null,
        iterations: 0,
        warmup: 0,
        spacing_ms: 0,
        derive_content_request: false,
        command: discovery,
    };
    let (reply, _, _) = measure_once(&probe).await;
    let reply = reply.ok_or("the discovery page never answered")?;
    if reply["success"].as_bool() != Some(true) {
        return Err(reply["error"]
            .as_str()
            .unwrap_or("the discovery page failed")
            .to_string());
    }
    let events = reply["data"]["events"]
        .as_array()
        .ok_or("the discovery page has no events")?;
    let mut best: Option<(i64, Value)> = None;
    for event in events {
        // An event's semantic body travels as a JSON *string*; the references
        // live inside it, next to the fields they stand in for.
        let Some(text) = event["data"].as_str() else {
            continue;
        };
        let Ok(payload) = serde_json::from_str::<Value>(text) else {
            continue;
        };
        let Some(references) = payload["content_refs"].as_object() else {
            continue;
        };
        for reference in references.values() {
            let max_bytes = reference["maxBytes"].as_i64().unwrap_or(0);
            if reference["databaseId"].as_str().is_some_and(str::is_empty)
                || reference["field"].as_str().is_none_or(str::is_empty)
            {
                continue;
            }
            if best
                .as_ref()
                .is_none_or(|(largest, _)| max_bytes > *largest)
            {
                best = Some((max_bytes, reference.clone()));
            }
        }
    }
    best.map(|(_, reference)| reference)
        .ok_or_else(|| "no event in the page carried a content_refs body".to_string())
}

async fn run_spec(mut spec: ProbeSpec, verbose: bool) -> SpecResult {
    let spacing = Duration::from_millis(spec.spacing_ms);
    let mut resolve_error = None;
    if spec.derive_content_request {
        match derive_content_request(&spec.command).await {
            Ok(request) => spec.command["contentRequest"] = request,
            Err(error) => resolve_error = Some(format!("content request not derivable: {error}")),
        }
    }
    let command_name = spec.command["type"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    if !spacing.is_zero() {
        tokio::time::sleep(spacing).await;
    }
    let (reply, elapsed, parse) = measure_once(&spec).await;
    let cold = sample_of(reply, elapsed, parse);
    if verbose {
        console::println!(
            "  {} cold {:.3} ms {} B{}",
            spec.label,
            cold.ms,
            cold.reply_bytes,
            cold.error
                .as_deref()
                .map(|error| format!(" error={error}"))
                .unwrap_or_default()
        );
    }
    for _ in 0..spec.warmup {
        if !spacing.is_zero() {
            tokio::time::sleep(spacing).await;
        }
        let (reply, elapsed, parse) = measure_once(&spec).await;
        let sample = sample_of(reply, elapsed, parse);
        if verbose {
            console::println!("  {} warmup {:.3} ms", spec.label, sample.ms);
        }
    }
    let mut samples = Vec::with_capacity(spec.iterations);
    for _ in 0..spec.iterations {
        if !spacing.is_zero() {
            tokio::time::sleep(spacing).await;
        }
        let (reply, elapsed, parse) = measure_once(&spec).await;
        let sample = sample_of(reply, elapsed, parse);
        if verbose {
            console::println!(
                "  {} {:.3} ms {} B{}",
                spec.label,
                sample.ms,
                sample.reply_bytes,
                sample
                    .error
                    .as_deref()
                    .map(|error| format!(" error={error}"))
                    .unwrap_or_default()
            );
        }
        samples.push(sample);
    }
    let mut times: Vec<f64> = samples.iter().map(|sample| sample.ms).collect();
    times.sort_by(|a, b| a.partial_cmp(b).expect("no NaN timings"));
    let mut encodes: Vec<f64> = samples.iter().map(|sample| sample.encode_ms).collect();
    encodes.sort_by(|a, b| a.partial_cmp(b).expect("no NaN timings"));
    let failed_parse = resolve_error.or_else(|| {
        cold.error
            .as_deref()
            .filter(|error| error.starts_with("unparseable command:"))
            .map(str::to_owned)
    });
    SpecResult {
        label: spec.label,
        command: command_name,
        parse_error: failed_parse,
        scale: spec.scale,
        iterations: spec.iterations,
        spacing_ms: spec.spacing_ms,
        cold: Some(cold),
        p50_ms: percentile(&times, 0.50),
        p95_ms: percentile(&times, 0.95),
        max_ms: times.last().copied().unwrap_or(0.0),
        min_ms: times.first().copied().unwrap_or(0.0),
        encode_p50_ms: percentile(&encodes, 0.50),
        reply_bytes: samples
            .first()
            .map(|sample| sample.reply_bytes)
            .unwrap_or(0),
        json_bytes: samples.first().map(|sample| sample.json_bytes).unwrap_or(0),
        replies: samples.iter().filter(|sample| sample.success).count(),
        samples,
    }
}

fn specs(path: &str) -> Vec<ProbeSpec> {
    let text = std::fs::read_to_string(path).expect("read the command list");
    serde_json::from_str(&text).expect("parse the command list")
}

/// The probe is the only code path this module ships (the task is `#[ignore]`d),
/// and what it must never do is run against the live databases: the home marker
/// is the runner's declaration that this is a throwaway copy.
#[test]
fn a_spec_parses_the_same_spelling_the_wire_uses() {
    let spec: ProbeSpec = serde_json::from_value(json!({
        "label": "get_session_entries:first_page:largest",
        "scale": {"sessionId": "s1", "entries": 14531},
        "iterations": 3,
        "command": {"type": "get_session_entries", "sessionId": "s1",
                    "before": 9007199254740991_i64, "limit": 40, "chunkedRead": true}
    }))
    .expect("a spec parses");
    let command: IncomingCmd =
        serde_json::from_value(spec.command.clone()).expect("the command parses");
    assert_eq!(command.cmd_type, "get_session_entries");
    assert_eq!(command.session_id, "s1");
    assert_eq!(command.limit, 40);
    assert!(command.chunked_read);
    assert_eq!(spec.iterations, 3);
    assert_eq!(spec.warmup, 0);
    assert_eq!(spec.spacing_ms, 0);
    assert_eq!(spec.scale["entries"], json!(14531));
}

/// Run a test body on a thread with a big stack.
///
/// `run_spec` awaits `measure_once` at three call sites (the cold call, the
/// warmup loop, the measured loop), and a debug build lays a full copy of that
/// ~140 KB state machine into the frame at each one: the chain asks for more
/// than 1.75 MB of the 2 MB a `cargo test` thread gets. Linux and macOS fit in
/// that margin, a Windows debug frame does not, and the abort
/// (`STATUS_STACK_OVERFLOW`) takes the whole test binary with it instead of
/// failing one test. A release build's frames are a fraction of that, which is
/// why the measured path never notices. Depth is bounded, so this is headroom
/// rather than a mask: nothing about what the test asserts changes.
fn with_big_stack<T: Send + 'static>(body: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(body)
        .expect("a test thread")
        .join()
        .expect("the test body did not panic")
}

/// The runner derives ids from the snapshot, so a spec that lost a field is a
/// possibility worth failing loudly on: a command missing its session would
/// measure an error reply and report it as a fast open.
#[test]
fn an_unparseable_command_is_reported_not_timed_as_a_read() {
    with_big_stack(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a test runtime")
            .block_on(async {
                let spec: ProbeSpec = serde_json::from_value(json!({
                    "label": "broken",
                    "iterations": 1,
                    "command": {"type": "list_tasks", "limit": "not a number"}
                }))
                .expect("the spec still parses");
                let result = run_spec(spec, false).await;
                assert!(
                    result
                        .parse_error
                        .as_deref()
                        .is_some_and(|error| error.starts_with("unparseable command:")),
                    "a spec whose command does not parse is named, not timed: {:?}",
                    result.parse_error
                );
                assert_eq!(
                    result.samples.len(),
                    1,
                    "the failure is still a recorded run"
                );
            })
    });
}

/// Nearest rank, not interpolation: the p95 of eight samples is the largest of
/// the eight, which is a number the run actually produced.
#[test]
fn percentiles_are_measured_values_not_interpolations() {
    let sorted = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 800.0];
    assert_eq!(percentile(&sorted, 0.50), 4.0);
    assert_eq!(percentile(&sorted, 0.95), 800.0);
    assert_eq!(percentile(&[3.0], 0.95), 3.0);
    assert_eq!(percentile(&[], 0.5), 0.0);
}

/// A handler that answers nothing must not read as a fast command.
#[tokio::test]
async fn a_silent_handler_is_recorded_as_a_failure() {
    let sample = sample_of(None, Duration::from_millis(1), Duration::ZERO);
    assert!(!sample.success);
    assert_eq!(sample.reply_bytes, 0);
    assert_eq!(
        sample.error.as_deref(),
        Some("the handler never answered"),
        "the missing reply is named, not silently zero"
    );
}

/// The whole probe: read the command list, measure it against the isolated
/// snapshot, write the raw samples. Never modifies the snapshot's data — every
/// command in the list is a read (the runner refuses anything else).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires an isolated DB snapshot; run scripts/perf/remote-commands/measure-remote-latency.py"]
async fn measure_real_snapshot() {
    let endpoint =
        std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("the runner sets the Agent address");
    assert!(
        endpoint.starts_with("http://127.0.0.1:"),
        "the probe measures a loopback Agent, not {endpoint}"
    );
    let home = std::env::var("HOME").expect("HOME is set");
    assert!(
        std::path::Path::new(&home)
            .join("MEASUREMENT_ISOLATED_HOME")
            .exists(),
        "refusing to measure {home}: it is not a declared throwaway snapshot"
    );
    let phase = std::env::var("REMOTE_LATENCY_PHASE").unwrap_or_else(|_| "warm".into());
    let list = specs(&std::env::var("REMOTE_LATENCY_COMMANDS").expect("REMOTE_LATENCY_COMMANDS"));
    let out = std::env::var("REMOTE_LATENCY_OUT").expect("REMOTE_LATENCY_OUT");
    let verbose = std::env::var("REMOTE_LATENCY_VERBOSE").is_ok();

    crate::install_rustls_provider();
    // Entry-owned preconditions, exactly as the sync probe documents them: the
    // Agent channel lives on a process-lifetime runtime, and the app store is
    // migrated by the entry rather than by the library.
    let _ = crate::agent_client::provide_channel_runtime("remote-latency-agent-channel", || {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .thread_name("remote-latency-agent-channel")
            .build()
            .expect("build the probe's Agent channel runtime")
    });
    crate::store::initialize_app_store().expect("initialize the isolated app store");

    console::println!("REMOTE_LATENCY_PHASE {phase} commands={}", list.len());
    let mut results = Vec::with_capacity(list.len());
    for spec in list {
        let label = spec.label.clone();
        let result = run_spec(spec, verbose).await;
        console::println!(
            "{phase} {label} cold={:.1} p50={:.1} p95={:.1} max={:.1} ms bytes={}",
            result.cold.as_ref().map(|cold| cold.ms).unwrap_or(0.0),
            result.p50_ms,
            result.p95_ms,
            result.max_ms,
            result.reply_bytes
        );
        results.push(result);
    }
    let document = json!({
        "phase": phase,
        "agentAddress": endpoint,
        "home": home,
        "machine": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "cpus": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
        },
        "cargoProfile": std::env::var("REMOTE_LATENCY_PROFILE").unwrap_or_else(|_| "unknown".into()),
        "builtAt": std::env::var("REMOTE_LATENCY_BUILT_AT").unwrap_or_else(|_| "unknown".into()),
        "results": results,
    });
    if let Some(parent) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(parent).expect("create the raw directory");
    }
    std::fs::write(
        &out,
        serde_json::to_vec_pretty(&document).expect("serialize"),
    )
    .expect("write the raw samples");
    console::println!("REMOTE_LATENCY_WRITTEN {out}");
}
