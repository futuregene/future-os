//! Opt-in browser probe around the real DesktopHost and an isolated real Agent.
//! Replaces NATS/E2EE with loopback HTTP. Not a phone/network/UI benchmark.
use super::*;
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
    if size > 32768 {
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
        ("POST", "/rpc") => {
            if headers.get("origin") != Some(&origin)
                || headers.get("x-sync-measurement").map(String::as_str) != Some("1")
            {
                return Err("same-origin probe header required".into());
            }
            let command: crate::remote::protocol::IncomingCmd =
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
            ("application/json", serde_json::to_vec(&response)?)
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

#[tokio::test]
#[ignore = "requires isolated DB snapshot; run scripts/measure-sync-browser.py"]
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
    println!("SYNC_BROWSER_READY {origin}");
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
                Ok(Err(error)) => eprintln!("probe request failed: {error}"),
                Err(_) => eprintln!("probe request timed out"),
            }
        });
    }
}
