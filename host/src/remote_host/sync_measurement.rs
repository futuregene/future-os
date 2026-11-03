//! Opt-in browser probe around the real DesktopHost and an isolated real Agent.
//! Replaces NATS/E2EE with loopback HTTP. Not a phone/network/UI benchmark.
use super::*;
use crate::console;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone, Deserialize, Serialize)]
struct Sample {
    label: String,
    session: String,
    run: String,
    expected_events: usize,
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
            *self.0.lock().unwrap() = Some(json!({"success":success,"data":data,"error":error}));
        })
    }
}

async fn handle(
    mut socket: tokio::net::TcpStream,
    origin: String,
    samples: Arc<Vec<Sample>>,
    root: std::path::PathBuf,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut bytes = Vec::new();
    let header_end;
    loop {
        let mut part = [0; 4096];
        let n = socket.read(&mut part).await?;
        if n == 0 {
            return Ok(());
        }
        bytes.extend_from_slice(&part[..n]);
        if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
            header_end = end + 4;
            break;
        }
        if bytes.len() > 16384 {
            return Err("headers too large".into());
        }
    }
    let headers = String::from_utf8(bytes[..header_end].to_vec())?;
    let mut lines = headers.lines();
    let mut start = lines
        .next()
        .ok_or("missing request line")?
        .split_whitespace();
    let method = start.next().unwrap_or("");
    let path = start.next().unwrap_or("");
    let headers: std::collections::HashMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    if headers.get("host").map(String::as_str) != origin.strip_prefix("http://") {
        return Err("invalid host".into());
    }
    let size = headers
        .get("content-length")
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(0);
    // Bounds any request this loopback probe accepts. The scenario result
    // document is the largest one it sees, and the plain/gzip passes are
    // reported together so one snapshot is one result — ~40 KB for six samples.
    if size > 262_144 {
        return Err("body too large".into());
    }
    while bytes.len() < header_end + size {
        let mut part = [0; 4096];
        let n = socket.read(&mut part).await?;
        if n == 0 {
            return Err("incomplete body".into());
        }
        bytes.extend_from_slice(&part[..n]);
    }
    let started = Instant::now();
    let (mime, body) = match (method, path) {
        ("GET", "/") => ("text/html", std::fs::read(root.join("index.html"))?),
        ("GET", "/bundle.js") => ("text/javascript", std::fs::read(root.join("bundle.js"))?),
        ("GET", "/samples") => ("application/json", serde_json::to_vec(&*samples)?),
        // Measurement clients POST their own result document here instead of
        // relying on a human clicking a button and copying console output. Same
        // origin + probe header are enforced by the shared check below.
        ("POST", "/result") => {
            if headers.get("origin") != Some(&origin)
                || headers.get("x-sync-measurement").map(String::as_str) != Some("1")
            {
                return Err("same-origin probe header required".into());
            }
            let path = root.join("results.json");
            std::fs::write(&path, &bytes[header_end..header_end + size])?;
            console::println!("measurement results written to {}", path.display());
            ("application/json", b"{\"stored\":true}".to_vec())
        }
        ("POST", "/rpc") => {
            if headers.get("origin") != Some(&origin)
                || headers.get("x-sync-measurement").map(String::as_str) != Some("1")
            {
                return Err("same-origin probe header required".into());
            }
            let command: future_remote_protocol::IncomingCmd =
                serde_json::from_slice(&bytes[header_end..header_end + size])?;
            if !matches!(
                command.cmd_type.as_str(),
                "get_state" | "get_session_entries" | "get_events_since" | "get_read_chunk"
            ) {
                return Err("read-only command required".into());
            }
            if !samples.iter().any(|sample| {
                sample.session == command.session_id
                    && (command.run_id.is_empty() || sample.run == command.run_id)
            }) {
                return Err("command outside sample scope".into());
            }
            let sink = Capture::default();
            host().execute(command, &sink).await;
            let response = sink.0.into_inner().unwrap().ok_or("missing response")?;
            // Route through the production reply encoder so the measurement
            // reflects the real wire bytes, including the gzip decision. The
            // per-request header models the per-connection capability the
            // desktop negotiates in production; the env var is the operator
            // override, kept for a whole-run sweep.
            let mut gzip = std::env::var("SYNC_MEASURE_GZIP").is_ok_and(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            });
            if let Some(header) = headers.get("x-sync-measure-gzip") {
                gzip = header == "1";
            }
            (
                "application/octet-stream",
                future_remote_server::reply::encode_reply_payload_with_gzip(&response, gzip),
            )
        }
        _ => ("text/plain", b"not found".to_vec()),
    };
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.;
    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nX-Backend-Ms: {elapsed_ms:.3}\r\nConnection: close\r\n\r\n", body.len());
    socket.write_all(response.as_bytes()).await?;
    socket.write_all(&body).await?;
    socket.shutdown().await?;
    Ok(())
}

/// The probe handler is the only code path this harness ships, and its guards
/// (same-origin, size bounds, read-only commands) are what keep a loopback
/// listener from becoming a way to read or write outside the sample scope.
/// Driven over a real socket: the guards live in the header/body parsing, so a
/// test that called a helper instead would not exercise them at all.
#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use tokio::io::AsyncWriteExt;

    #[derive(Debug)]
    struct RoundTrip {
        raw: Vec<u8>,
        result: Result<(), Box<dyn std::error::Error + Send + Sync>>,
    }

    impl RoundTrip {
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.raw).into_owned()
        }

        /// The error the handler rejected the request with. Panics with the
        /// response when the handler answered instead — a guard that stopped
        /// guarding must fail loudly, not silently pass.
        fn rejection(&self) -> String {
            self.result
                .as_ref()
                .err()
                .map(|error| error.to_string())
                .unwrap_or_else(|| panic!("expected a rejection, got: {}", self.text()))
        }

        fn status_line(&self) -> String {
            self.text()
                .lines()
                .next()
                .unwrap_or_default()
                .trim_end()
                .to_string()
        }

        /// Assert the handler answered (rather than rejecting) and hand back the
        /// answer's text.
        ///
        /// The rejection is read **eagerly**, into a local the assertion then
        /// quotes. An `assert!` message argument is evaluated only when the
        /// assertion fails, so an expression left inside the message is a line
        /// llvm-cov reports as uncovered however green the suite is — the
        /// measurement would carry a phantom gap on every assertion that names
        /// the value it checked.
        fn answered(&self) -> String {
            let rejection = self.result.as_ref().err().map(ToString::to_string);
            assert!(rejection.is_none(), "the handler rejected: {rejection:?}");
            self.text()
        }
    }

    /// One raw request against a fresh `handle` on a real loopback socket.
    ///
    /// `{PORT}` in the request text is substituted with the bound port so the
    /// `host:`/`origin:` guards see the address the listener actually owns.
    /// The write half is closed after the request: that is what a client does,
    /// and it is what makes a truncated body observable as EOF.
    async fn round_trip(root: &std::path::Path, samples: Vec<Sample>, request: &str) -> RoundTrip {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind loopback");
        let port = listener.local_addr().expect("bound addr").port();
        let origin = format!("http://127.0.0.1:{port}");
        let root = root.to_path_buf();
        let samples = Arc::new(samples);
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("accept");
            handle(socket, origin, samples, root).await
        });

        let mut client = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        let text = request.replace("{PORT}", &port.to_string());
        client
            .write_all(text.as_bytes())
            .await
            .expect("write request");
        client.shutdown().await.expect("half-close");

        let mut raw = Vec::new();
        client.read_to_end(&mut raw).await.expect("read response");
        let result = tokio::time::timeout(Duration::from_secs(10), server)
            .await
            .expect("the handler must not hang")
            .expect("join handle");
        RoundTrip { raw, result }
    }

    /// The same exchange, but the request arrives in two writes with a pause
    /// between them: the header block first, then the body. A browser does this
    /// whenever a body crosses a packet boundary, and the handler has to keep
    /// reading to the declared length instead of treating the first short read
    /// as the whole body.
    async fn round_trip_split_body(
        root: &std::path::Path,
        samples: Vec<Sample>,
        headers: &str,
        body: &str,
    ) -> RoundTrip {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind loopback");
        let port = listener.local_addr().expect("bound addr").port();
        let origin = format!("http://127.0.0.1:{port}");
        let root = root.to_path_buf();
        let samples = Arc::new(samples);
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("accept");
            handle(socket, origin, samples, root).await
        });

        let mut client = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        let head = format!(
            "{}\r\ncontent-length: {}\r\n\r\n",
            headers.replace("{PORT}", &port.to_string()),
            body.len()
        );
        client
            .write_all(head.as_bytes())
            .await
            .expect("write the header block");
        client.flush().await.expect("flush");
        // Long enough that the handler is parked in its body read when the rest
        // of the body arrives.
        tokio::time::sleep(Duration::from_millis(80)).await;
        client
            .write_all(body.as_bytes())
            .await
            .expect("write the body");
        client.shutdown().await.expect("half-close");

        let mut raw = Vec::new();
        client.read_to_end(&mut raw).await.expect("read response");
        let result = tokio::time::timeout(Duration::from_secs(10), server)
            .await
            .expect("the handler must not hang")
            .expect("join handle");
        RoundTrip { raw, result }
    }

    fn sample(label: &str, session: &str, run: &str) -> Sample {
        Sample {
            label: label.to_string(),
            session: session.to_string(),
            run: run.to_string(),
            expected_events: 3,
        }
    }

    /// The three GET routes: the probe shell, its bundle, and the sample list.
    #[tokio::test]
    async fn serves_the_probe_shell_bundle_and_samples() {
        let root = tempfile::tempdir().expect("web root");
        std::fs::write(root.path().join("index.html"), b"<html>probe</html>").expect("index");
        std::fs::write(root.path().join("bundle.js"), b"console.log(1)").expect("bundle");

        let samples = vec![sample("cold", "sess-1", "run-1")];
        // The sample list must survive the round trip intact — the browser
        // half of the harness reads it back as JSON.
        let list = round_trip(
            root.path(),
            samples.clone(),
            "GET /samples HTTP/1.1\r\nhost: 127.0.0.1:{PORT}\r\n\r\n",
        )
        .await;
        let listing = list.answered();
        assert_eq!(list.status_line(), "HTTP/1.1 200 OK");
        assert!(
            listing.contains("application/json"),
            "the sample list is JSON: {listing}"
        );
        let body = list
            .text()
            .split_once("\r\n\r\n")
            .map(|(_, body)| body.to_string())
            .expect("a body follows the headers");
        let decoded: Vec<Sample> = serde_json::from_str(&body).expect("samples decode");
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].session, "sess-1");
        assert_eq!(decoded[0].expected_events, 3);

        for (path, mime, expected) in [
            ("/", "text/html", "<html>probe</html>"),
            ("/bundle.js", "text/javascript", "console.log(1)"),
        ] {
            let response = round_trip(
                root.path(),
                samples.clone(),
                &format!("GET {path} HTTP/1.1\r\nhost: 127.0.0.1:{{PORT}}\r\n\r\n"),
            )
            .await;
            let text = response.answered();
            assert_eq!(response.status_line(), "HTTP/1.1 200 OK");
            assert!(text.contains(mime), "{path} is served as {mime}: {text}");
            assert!(text.ends_with(expected), "{path} body: {text}");
            // The probe page is a local measurement surface: no caching, and
            // a browser must not sniff a different type out of it.
            assert!(text.contains("Cache-Control: no-store"), "{text}");
            assert!(text.contains("X-Content-Type-Options: nosniff"), "{text}");
        }
    }

    /// An unknown path answers with the probe's own 404 body rather than
    /// falling through to a filesystem read.
    #[tokio::test]
    async fn an_unknown_route_is_plain_not_found() {
        let root = tempfile::tempdir().expect("web root");
        let response = round_trip(
            root.path(),
            Vec::new(),
            "GET /../secrets HTTP/1.1\r\nhost: 127.0.0.1:{PORT}\r\n\r\n",
        )
        .await;
        let text = response.answered();
        assert!(text.ends_with("not found"), "{text}");
        assert!(
            text.contains("text/plain"),
            "the fallback is plain text: {text}"
        );
    }

    /// A request whose `host` is not the bound origin is refused: that check is
    /// what keeps a page from pointing the probe at a foreign name.
    #[tokio::test]
    async fn a_foreign_host_header_is_rejected() {
        let root = tempfile::tempdir().expect("web root");
        let response = round_trip(
            root.path(),
            Vec::new(),
            "GET /samples HTTP/1.1\r\nhost: evil.example\r\n\r\n",
        )
        .await;
        assert_eq!(response.rejection(), "invalid host");
        assert!(response.raw.is_empty(), "a rejected request gets no reply");
    }

    /// A client that disconnects before sending anything is an ordinary end of
    /// stream, not a failure — the browser closes speculative connections.
    #[tokio::test]
    async fn a_client_that_disconnects_early_is_not_an_error() {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind loopback");
        let port = listener.local_addr().expect("addr").port();
        let root = tempfile::tempdir().expect("web root");
        let server = tokio::spawn({
            let root = root.path().to_path_buf();
            async move {
                let (socket, _) = listener.accept().await.expect("accept");
                handle(
                    socket,
                    format!("http://127.0.0.1:{port}"),
                    Arc::new(vec![]),
                    root,
                )
                .await
            }
        });
        let client = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        drop(client);
        let result = tokio::time::timeout(Duration::from_secs(10), server)
            .await
            .expect("must not hang")
            .expect("join");
        assert!(
            result.is_ok(),
            "EOF before headers is not an error: {result:?}"
        );
    }

    /// Unbounded headers are refused before the body is even considered.
    #[tokio::test]
    async fn an_oversized_header_block_is_rejected() {
        let root = tempfile::tempdir().expect("web root");
        let padding = "x-pad: 0123456789abcdef\r\n".repeat(700);
        let response = round_trip(
            root.path(),
            Vec::new(),
            &format!("GET /samples HTTP/1.1\r\nhost: 127.0.0.1:{{PORT}}\r\n{padding}"),
        )
        .await;
        assert_eq!(response.rejection(), "headers too large");
    }

    /// The declared body length is bounded before a single body byte is read,
    /// so a huge `content-length` cannot make the probe buffer memory.
    #[tokio::test]
    async fn a_declared_body_over_the_bound_is_rejected() {
        let root = tempfile::tempdir().expect("web root");
        let response = round_trip(
            root.path(),
            Vec::new(),
            "POST /result HTTP/1.1\r\nhost: 127.0.0.1:{PORT}\r\n\
             origin: http://127.0.0.1:{PORT}\r\nx-sync-measurement: 1\r\n\
             content-length: 262145\r\n\r\n",
        )
        .await;
        assert_eq!(response.rejection(), "body too large");
    }

    /// A body shorter than its declared length ends in EOF, which must be an
    /// error rather than a silent partial write of `results.json`.
    #[tokio::test]
    async fn a_truncated_body_is_rejected() {
        let root = tempfile::tempdir().expect("web root");
        let response = round_trip(
            root.path(),
            Vec::new(),
            "POST /result HTTP/1.1\r\nhost: 127.0.0.1:{PORT}\r\n\
             origin: http://127.0.0.1:{PORT}\r\nx-sync-measurement: 1\r\n\
             content-length: 40\r\n\r\n{\"short\":",
        )
        .await;
        assert_eq!(response.rejection(), "incomplete body");
    }

    /// Writing a result needs the same-origin probe header: without it a
    /// cross-site page could plant a document the operator then reads.
    #[tokio::test]
    async fn storing_a_result_requires_the_probe_header() {
        let root = tempfile::tempdir().expect("web root");
        let body = "{\"results\":[]}";
        let response = round_trip(
            root.path(),
            Vec::new(),
            &format!(
                "POST /result HTTP/1.1\r\nhost: 127.0.0.1:{{PORT}}\r\n\
                 origin: http://127.0.0.1:{{PORT}}\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            ),
        )
        .await;
        assert_eq!(response.rejection(), "same-origin probe header required");
        assert!(
            !root.path().join("results.json").exists(),
            "a rejected write must leave no file behind"
        );

        // …and a foreign origin is refused even with the header present.
        let foreign = round_trip(
            root.path(),
            Vec::new(),
            &format!(
                "POST /result HTTP/1.1\r\nhost: 127.0.0.1:{{PORT}}\r\n\
                 origin: https://evil.example\r\nx-sync-measurement: 1\r\n\
                 content-length: {}\r\n\r\n{body}",
                body.len()
            ),
        )
        .await;
        assert_eq!(foreign.rejection(), "same-origin probe header required");
    }

    /// With both headers the document is stored verbatim for the operator.
    #[tokio::test]
    async fn a_probe_result_is_stored_verbatim() {
        let root = tempfile::tempdir().expect("web root");
        let body = r#"{"results":[{"label":"cold"}]}"#;
        let response = round_trip(
            root.path(),
            Vec::new(),
            &format!(
                "POST /result HTTP/1.1\r\nhost: 127.0.0.1:{{PORT}}\r\n\
                 origin: http://127.0.0.1:{{PORT}}\r\nx-sync-measurement: 1\r\n\
                 content-length: {}\r\n\r\n{body}",
                body.len()
            ),
        )
        .await;
        let text = response.answered();
        assert!(text.ends_with("{\"stored\":true}"), "{text}");
        let stored = std::fs::read_to_string(root.path().join("results.json")).expect("stored");
        assert_eq!(
            stored, body,
            "the operator's own document is stored as sent"
        );
    }

    /// The RPC route is read-only on purpose: a measurement run must not be
    /// able to drive the app's write commands.
    #[tokio::test]
    async fn rpc_rejects_anything_but_the_read_commands() {
        let root = tempfile::tempdir().expect("web root");
        let body = r#"{"type":"send_message","sessionId":"sess-1"}"#;
        let response = round_trip(
            root.path(),
            vec![sample("cold", "sess-1", "run-1")],
            &format!(
                "POST /rpc HTTP/1.1\r\nhost: 127.0.0.1:{{PORT}}\r\n\
                 origin: http://127.0.0.1:{{PORT}}\r\nx-sync-measurement: 1\r\n\
                 content-length: {}\r\n\r\n{body}",
                body.len()
            ),
        )
        .await;
        assert_eq!(response.rejection(), "read-only command required");
    }

    /// …and even a read command only reaches sessions the run declared.
    #[tokio::test]
    async fn rpc_rejects_a_command_outside_the_sample_scope() {
        let root = tempfile::tempdir().expect("web root");
        let body = r#"{"type":"get_state","sessionId":"sess-other"}"#;
        let response = round_trip(
            root.path(),
            vec![sample("cold", "sess-1", "run-1")],
            &format!(
                "POST /rpc HTTP/1.1\r\nhost: 127.0.0.1:{{PORT}}\r\n\
                 origin: http://127.0.0.1:{{PORT}}\r\nx-sync-measurement: 1\r\n\
                 content-length: {}\r\n\r\n{body}",
                body.len()
            ),
        )
        .await;
        assert_eq!(response.rejection(), "command outside sample scope");

        // A run id is only a narrowing filter: an empty one accepts any run of
        // the sampled session, which is how the browser asks its first question.
        // The command gets past the guard (it is then the store, not this
        // handler, that decides the answer), so the scope error must not recur.
        let in_scope = r#"{"type":"get_state","sessionId":"sess-1"}"#;
        let unscoped = round_trip(
            root.path(),
            vec![sample("cold", "sess-1", "run-1")],
            &format!(
                "POST /rpc HTTP/1.1\r\nhost: 127.0.0.1:{{PORT}}\r\n\
                 origin: http://127.0.0.1:{{PORT}}\r\nx-sync-measurement: 1\r\n\
                 content-length: {}\r\n\r\n{in_scope}",
                in_scope.len()
            ),
        )
        .await;
        assert_ne!(
            unscoped
                .result
                .as_ref()
                .err()
                .map(|error| error.to_string()),
            Some("command outside sample scope".to_string()),
            "an in-scope read must get past the scope guard: {unscoped:?}",
        );
    }

    /// A malformed RPC body is a JSON error, not a panic.
    #[tokio::test]
    async fn rpc_rejects_a_malformed_body() {
        let root = tempfile::tempdir().expect("web root");
        let body = "{not json";
        let response = round_trip(
            root.path(),
            vec![sample("cold", "sess-1", "run-1")],
            &format!(
                "POST /rpc HTTP/1.1\r\nhost: 127.0.0.1:{{PORT}}\r\n\
                 origin: http://127.0.0.1:{{PORT}}\r\nx-sync-measurement: 1\r\n\
                 content-length: {}\r\n\r\n{body}",
                body.len()
            ),
        )
        .await;
        let rejection = response.rejection();
        // serde's own message, propagated as-is: the point is that a malformed
        // body is a clean JSON error naming the position, never a panic and
        // never a silent empty command.
        assert!(
            rejection.contains("at line 1 column 2"),
            "a malformed body surfaces as a positioned JSON error: {rejection}"
        );
    }

    /// The reply sink records the whole envelope the handler later encodes.
    #[tokio::test]
    async fn the_capture_sink_records_the_reply_envelope() {
        let sink = Capture::default();
        sink.send(true, json!({"a": 1}), Some("ignored".to_string()))
            .await;
        let recorded = sink.0.lock().expect("lock").clone().expect("captured");
        assert_eq!(recorded["success"], true);
        assert_eq!(recorded["data"]["a"], 1);
        assert_eq!(recorded["error"], "ignored");

        // A failure reply keeps its error text and a null payload.
        let sink = Capture::default();
        sink.send(false, Value::Null, Some("boom".to_string()))
            .await;
        let recorded = sink.0.lock().expect("lock").clone().expect("captured");
        assert_eq!(recorded["success"], false);
        assert!(recorded["data"].is_null());
        assert_eq!(recorded["error"], "boom");
    }

    /// A body that arrives in two writes is reassembled: the declared length,
    /// not the first short read, is what ends it. (A single-read handler would
    /// store a truncated document — the measurement's own results — and the
    /// operator would read a JSON file that stops mid-object.)
    #[tokio::test]
    async fn a_body_split_across_two_writes_is_reassembled() {
        let root = tempfile::tempdir().expect("web root");
        let body = r#"{"results":[{"label":"cold","events":3}]}"#;
        let response = round_trip_split_body(
            root.path(),
            Vec::new(),
            "POST /result HTTP/1.1\r\nhost: 127.0.0.1:{PORT}\r\n\
             origin: http://127.0.0.1:{PORT}\r\nx-sync-measurement: 1",
            body,
        )
        .await;
        response.answered();
        let stored = std::fs::read_to_string(root.path().join("results.json")).expect("stored");
        assert_eq!(
            stored, body,
            "the whole body is stored, not just its first packet"
        );
    }

    /// The RPC route needs the same-origin probe header too: an in-scope read
    /// command from a foreign origin is refused before it reaches the app.
    #[tokio::test]
    async fn an_rpc_request_without_the_probe_header_is_rejected() {
        let root = tempfile::tempdir().expect("web root");
        let body = r#"{"type":"get_state","sessionId":"sess-1"}"#;
        let response = round_trip(
            root.path(),
            vec![sample("cold", "sess-1", "run-1")],
            &format!(
                "POST /rpc HTTP/1.1\r\nhost: 127.0.0.1:{{PORT}}\r\n\
                 origin: https://evil.example\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            ),
        )
        .await;
        assert_eq!(
            response.rejection(),
            "same-origin probe header required",
            "the read route is gated on the same header as the write route"
        );
    }

    /// The per-request gzip declaration, and the whole-run env override behind
    /// it, both reach the production reply encoder — which is the only place
    /// the measured byte count and the shipping encoding can disagree. A page
    /// past the encoder's threshold is asked for twice, once per request flag
    /// and once through the env var, and the wire really does carry a gzip
    /// stream for the declared request.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // mock_agent_lock serializes this family
    async fn the_gzip_declaration_is_honoured_on_the_measurement_lane() {
        use crate::remote::test_support::{ensure_mock_agent, init_store, mock_agent_lock};
        use flate2::read::GzDecoder;
        use std::io::Read;

        const RPC_BODY: &str = r#"{"id":"m1","type":"get_session_entries","sessionId":"sess-1"}"#;
        let request = |extra: &str| {
            format!(
                "POST /rpc HTTP/1.1\r\nhost: 127.0.0.1:{{PORT}}\r\n\
                 origin: http://127.0.0.1:{{PORT}}\r\nx-sync-measurement: 1\r\n{extra}\
                 content-length: {}\r\n\r\n{RPC_BODY}",
                RPC_BODY.len()
            )
        };

        let _lock = mock_agent_lock();
        let _home = crate::remote::test_support::HomeGuard::new("sync-measure-gzip");
        let root = tempfile::tempdir().expect("web root");
        init_store();
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        // Past `REMOTE_JSON_GZIP_THRESHOLD_BYTES` (32 KiB), which is where the
        // encoder's choice starts to be visible on the wire.
        let entries: Vec<Value> = (0..120)
            .map(|index| {
                json!({
                    "id": format!("e{index}"), "kind": "assistant", "role": "assistant",
                    "createdAtMs": 1_000 + index, "runId": "run-1",
                    "blocks": [{"kind": "text", "text": "measured page body ".repeat(20)}],
                })
            })
            .collect();
        agent.set_session_entries("sess-1", json!({ "entries": entries }));

        let plain = round_trip(
            root.path(),
            vec![sample("cold", "sess-1", "run-1")],
            &request(""),
        )
        .await;
        assert!(plain.result.is_ok(), "rejection: {}", plain.rejection());
        assert!(
            !plain.raw.windows(2).any(|pair| pair == [0x1f, 0x8b]),
            "without the declaration the reply rides plain"
        );
        let plain_json = plain
            .text()
            .split_once("\r\n\r\n")
            .map(|(_, body)| body.to_string())
            .expect("a body follows the headers");
        let described = plain_json[..plain_json.len().min(200)].to_string();
        assert!(
            plain_json.contains("entries"),
            "the page is the reply the measurement reads: {described}"
        );

        // Per-request declaration: the same page, gzipped on the wire.
        let gzipped = round_trip(
            root.path(),
            vec![sample("cold", "sess-1", "run-1")],
            &request("x-sync-measure-gzip: 1\r\n"),
        )
        .await;
        gzipped.answered();
        let gz_body = gzipped
            .raw
            .windows(2)
            .position(|pair| pair == [0x1f, 0x8b])
            .map(|at| gzipped.raw[at..].to_vec())
            .expect("the declared request carries a gzip stream");
        let mut decoded = Vec::new();
        GzDecoder::new(gz_body.as_slice())
            .read_to_end(&mut decoded)
            .expect("the stream decompresses");
        assert_eq!(
            String::from_utf8_lossy(&decoded),
            plain_json,
            "gzip is an encoding, not a different answer"
        );

        // …and the whole-run override does the same for a request that does not
        // ask: the operator sweeps a run with one env var.
        std::env::set_var("SYNC_MEASURE_GZIP", "true");
        let from_env = round_trip(
            root.path(),
            vec![sample("cold", "sess-1", "run-1")],
            &request(""),
        )
        .await;
        std::env::remove_var("SYNC_MEASURE_GZIP");
        from_env.answered();
        assert!(
            from_env.raw.windows(2).any(|pair| pair == [0x1f, 0x8b]),
            "the env override reaches the encoder too"
        );

        agent.clear_scripts();
    }
}

#[tokio::test]
#[ignore = "requires isolated DB snapshot; run scripts/measure/measure-sync-browser.py"]
async fn serve_real_snapshot() {
    let endpoint = std::env::var("FUTURE_AGENT_GRPC_ADDR").unwrap();
    assert!(endpoint.starts_with("http://127.0.0.1:"));
    let home = std::env::var("HOME").unwrap();
    assert!(std::path::Path::new(&home)
        .join("MEASUREMENT_ISOLATED_HOME")
        .exists());
    let samples: Vec<Sample> =
        serde_json::from_str(&std::env::var("SYNC_MEASURE_SAMPLES").unwrap()).unwrap();
    let samples = Arc::new(samples);
    let root = std::path::PathBuf::from(std::env::var("SYNC_MEASURE_WEB_ROOT").unwrap());
    let port: u16 = std::env::var("SYNC_MEASURE_WEB_PORT")
        .unwrap()
        .parse()
        .unwrap();
    let origin = format!("http://127.0.0.1:{port}");
    crate::install_rustls_provider();
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    console::println!("SYNC_BROWSER_READY {origin}");
    loop {
        let (socket, _) = listener.accept().await.unwrap();
        let samples = samples.clone();
        let root = root.clone();
        let origin = origin.clone();
        tokio::spawn(async move {
            match tokio::time::timeout(
                std::time::Duration::from_secs(120),
                handle(socket, origin, samples, root),
            )
            .await
            {
                Ok(Ok(())) => (),
                Ok(Err(error)) => console::eprintln!("probe request failed: {error}"),
                Err(_) => console::eprintln!("probe request timed out"),
            }
        });
    }
}
