//! Per-command evidence for a remote conversation that will not load.
//!
//! A phone's cold open is a chain of business commands (`get_state`,
//! `get_session_entries`, `get_read_chunk`, `get_events_since`) over one relay
//! link. When the phone reports a load timeout, the question that decides the
//! fix is *which side of that link was slow*: the desktop executing the
//! command, or the transport carrying it. Without a record of the desktop's
//! own execution time, that question is unanswerable and the cause is guessed
//! at — twice on 2026-10-28, each time confidently and wrongly.
//!
//! This wrapper measures every business command the desktop executes and
//! reports the slow or failed ones into `desktop.log` (through
//! [`crate::environment::report`], which writes the file even when there is no
//! terminal). A phone timeout with no slow command logged here is a transport
//! problem; a slow command here names exactly which read was expensive.
use std::sync::Mutex;
use std::time::Duration;

use future_remote_server::services::ReplySink;
use futures::future::BoxFuture;
use serde_json::Value;

/// Below this a command is ordinary remote traffic and is not worth a log row.
const SLOW_COMMAND_MS: u128 = 1_000;

/// Commands that must never appear in the report: liveness traffic, which is
/// emitted on a timer and would drown the commands a reader is actually waiting
/// for. A *slow* one of these is a transport symptom, not an execution one, and
/// the transport is exactly what this log cannot measure anyway.
const HEARTBEAT_COMMANDS: [&str; 4] = [
    "client_presence",
    "client_disconnect",
    "get_presence",
    "secure_ready",
];

#[derive(Default, Debug)]
pub(super) struct CommandOutcome {
    /// The handler's verdict. `None` means the command was dropped without a
    /// reply (an access/handshake fence), which is itself a fact worth naming.
    success: Option<bool>,
    error: Option<String>,
    /// The chunk payload of a binary read page. A JSON reply's size is not
    /// measured: computing it would serialize every reply just for telemetry.
    bytes: Option<usize>,
}

/// A sink that forwards every reply and records what it carried.
///
/// Only `send` is implemented: the trait's default `send_binary` funnels
/// through it, so one implementation observes both reply shapes, and the
/// chunk's byte length comes along for free.
pub(super) struct ObservedSink<'a> {
    inner: &'a dyn ReplySink,
    outcome: &'a Mutex<CommandOutcome>,
}

impl<'a> ObservedSink<'a> {
    pub(super) fn new(inner: &'a dyn ReplySink, outcome: &'a Mutex<CommandOutcome>) -> Self {
        Self { inner, outcome }
    }
}

impl ReplySink for ObservedSink<'_> {
    fn send<'a>(&'a self, success: bool, data: Value, error: Option<String>) -> BoxFuture<'a, ()> {
        {
            let mut outcome = self.outcome.lock().unwrap();
            outcome.success = Some(success);
            outcome.error = error.clone();
            if let Some(bytes) = read_chunk_bytes(&data) {
                outcome.bytes = Some(bytes);
            }
        }
        self.inner.send(success, data, error)
    }

    /// Forward the binary carrier as well. Relying on the trait default here
    /// would downgrade a negotiated binary page to base64-in-JSON, which is a
    /// different wire shape than the connection asked for.
    fn send_binary<'a>(&'a self, header: Value, body: Vec<u8>) -> BoxFuture<'a, ()> {
        {
            let mut outcome = self.outcome.lock().unwrap();
            outcome.success = Some(true);
            outcome.error = None;
            outcome.bytes = Some(body.len());
        }
        self.inner.send_binary(header, body)
    }
}

/// The chunk length of a JSON-carried read page, when this reply is one. A
/// binary-carried page reports its length directly from the body in
/// `send_binary` instead.
fn read_chunk_bytes(data: &Value) -> Option<usize> {
    let chunk = data.get("readChunk")?;
    let next = chunk.get("nextOffset")?.as_u64()?;
    let offset = chunk.get("offset")?.as_u64()?;
    usize::try_from(next.checked_sub(offset)?).ok()
}

/// The log row for one executed business command, or `None` when this command
/// is ordinary traffic.
pub(super) fn report_line(
    cmd_type: &str,
    session_id: &str,
    elapsed: Duration,
    outcome: &Mutex<CommandOutcome>,
) -> Option<String> {
    if HEARTBEAT_COMMANDS.contains(&cmd_type) {
        return None;
    }
    let outcome = outcome.lock().unwrap();
    let slow = elapsed.as_millis() >= SLOW_COMMAND_MS;
    if !slow && outcome.error.is_none() {
        return None;
    }
    // Session and run identify the read, never its content: what a diagnostic
    // needs is *which* command was expensive, not what it returned.
    Some(format!(
        "remote command {}: type={} session={} elapsed={}ms{}{}{}",
        if slow { "slow" } else { "failed" },
        cmd_type,
        if session_id.is_empty() {
            "-"
        } else {
            session_id
        },
        elapsed.as_millis(),
        match outcome.bytes {
            Some(bytes) => format!(" bytes={bytes}"),
            None => String::new(),
        },
        match outcome.success {
            Some(false) => " replied=error".to_owned(),
            Some(true) => String::new(),
            None => " replied=none".to_owned(),
        },
        match &outcome.error {
            Some(error) => format!(" error={error}"),
            None => String::new(),
        },
    ))
}

/// Write that row into `desktop.log`, when there is one.
pub(super) fn report(
    cmd_type: &str,
    session_id: &str,
    elapsed: Duration,
    outcome: &Mutex<CommandOutcome>,
) {
    if let Some(line) = report_line(cmd_type, session_id, elapsed, outcome) {
        crate::environment::report(&line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn outcome(success: Option<bool>, error: Option<&str>, bytes: Option<usize>) -> CommandOutcome {
        CommandOutcome {
            success,
            error: error.map(str::to_owned),
            bytes,
        }
    }

    fn decide(cmd_type: &str, elapsed_ms: u128, outcome: &CommandOutcome) -> bool {
        let outcome = Mutex::new(CommandOutcome {
            success: outcome.success,
            error: outcome.error.clone(),
            bytes: outcome.bytes,
        });
        report_line(
            cmd_type,
            "s1",
            Duration::from_millis(elapsed_ms as u64),
            &outcome,
        )
        .is_some()
    }

    #[test]
    fn only_slow_or_failed_business_commands_are_reported() {
        let kind = "get_session_entries";
        assert!(!decide(kind, 20, &outcome(Some(true), None, None)));
        assert!(!decide(
            kind,
            SLOW_COMMAND_MS - 1,
            &outcome(Some(true), None, None)
        ));
        assert!(decide(
            kind,
            SLOW_COMMAND_MS,
            &outcome(Some(true), None, None)
        ));
        assert!(decide(
            kind,
            20,
            &outcome(Some(false), Some("agent offline"), None)
        ));
        assert!(decide(
            kind,
            20,
            &outcome(Some(false), Some("remote_read_expired"), None)
        ));
    }

    #[test]
    fn heartbeat_commands_never_report() {
        for kind in HEARTBEAT_COMMANDS {
            assert!(!decide(kind, 60_000, &outcome(Some(true), None, None)));
        }
    }

    #[test]
    fn chunk_length_is_the_payload_span_on_either_carrier() {
        let data = serde_json::json!({"readChunk": {
            "id": "read_a", "offset": 192 * 1024, "nextOffset": 192 * 1024 + 4096,
            "totalBytes": 900_000,
        }});
        assert_eq!(read_chunk_bytes(&data), Some(4096));
        assert_eq!(read_chunk_bytes(&serde_json::json!({"entries": []})), None);
    }

    #[test]
    fn the_row_names_the_command_its_cost_and_its_error() {
        let outcome = Mutex::new(CommandOutcome {
            success: Some(false),
            error: Some("agent offline".into()),
            bytes: None,
        });
        let line = report_line(
            "get_session_entries",
            "s1",
            Duration::from_millis(42),
            &outcome,
        )
        .expect("a failed command is reported");
        assert!(line.contains("type=get_session_entries"), "{line}");
        assert!(line.contains("session=s1"), "{line}");
        assert!(line.contains("elapsed=42ms"), "{line}");
        assert!(line.contains("error=agent offline"), "{line}");

        let outcome = Mutex::new(CommandOutcome {
            success: Some(true),
            error: None,
            bytes: Some(4096),
        });
        let line = report_line(
            "get_read_chunk",
            "s1",
            Duration::from_millis(2500),
            &outcome,
        )
        .expect("a slow command is reported");
        assert!(line.starts_with("remote command slow:"), "{line}");
        assert!(line.contains("bytes=4096"), "{line}");
    }

    #[test]
    fn the_sink_forwards_and_records_verdict_error_and_chunk_bytes() {
        #[derive(Default)]
        struct Recorder {
            calls: Mutex<Vec<(bool, Value, Option<String>)>>,
            frames: Mutex<Vec<(Value, Vec<u8>)>>,
        }
        impl ReplySink for Recorder {
            fn send<'a>(
                &'a self,
                success: bool,
                data: Value,
                error: Option<String>,
            ) -> BoxFuture<'a, ()> {
                self.calls
                    .lock()
                    .unwrap()
                    .push((success, data.clone(), error));
                Box::pin(async {})
            }
            fn send_binary<'a>(&'a self, header: Value, body: Vec<u8>) -> BoxFuture<'a, ()> {
                self.frames.lock().unwrap().push((header, body));
                Box::pin(async {})
            }
        }

        let recorder = Recorder::default();
        let outcome = Arc::new(Mutex::new(CommandOutcome::default()));
        let sink = ObservedSink::new(&recorder, &outcome);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            sink.send(true, serde_json::json!({"entries": []}), None)
                .await;
            sink.send(false, Value::Null, Some("agent offline".into()))
                .await;
            sink.send_binary(
                serde_json::json!({"readChunk": {
                    "id": "read_a", "offset": 0, "nextOffset": 4096, "totalBytes": 4096,
                }}),
                vec![7u8; 4096],
            )
            .await;
            sink.send_binary(
                serde_json::json!({"readChunk": {
                    "id": "read_b", "offset": 0, "nextOffset": 128, "totalBytes": 128,
                }}),
                vec![9u8; 128],
            )
            .await;
        });

        assert_eq!(recorder.calls.lock().unwrap().len(), 2);
        assert_eq!(
            recorder.frames.lock().unwrap().len(),
            2,
            "binary pages stay binary"
        );
        let outcome = outcome.lock().unwrap();
        assert_eq!(outcome.success, Some(true), "the last reply wins");
        assert_eq!(outcome.error, None, "the last reply wins");
        assert_eq!(outcome.bytes, Some(128), "the binary body, not the header");
    }
}
