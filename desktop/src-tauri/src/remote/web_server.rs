use super::*;

/// Cap on concurrent accepted web-client connections. Acquired BEFORE `accept`
/// so a flood of idle sockets can't exhaust file descriptors (the accept loop
/// blocks at capacity instead of parking unbounded tasks).
pub(super) const WEB_MAX_CONNECTIONS: usize = 32;

/// A client that connects and never sends a request can't hold a task + fd
/// open indefinitely; its read times out and the connection is dropped. Tests
/// shrink the timeout so the silent-client path runs fast.
pub(super) fn web_read_timeout() -> std::time::Duration {
    #[cfg(test)]
    const TIMEOUT: std::time::Duration = std::time::Duration::from_millis(50);
    #[cfg(not(test))]
    const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
    TIMEOUT
}

/// `desktop/web/` on disk — one level up from CARGO_MANIFEST_DIR (desktop/src-tauri/).
pub(super) fn web_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../web")
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../web"))
}

/// The web remote client is intentionally available only against the test
/// platform. Production and custom platform URLs retain mobile remote control
/// but never bind the local HTTP listener.
pub(super) fn web_client_enabled_for_platform(platform_url: &str) -> bool {
    platform_url == crate::future_platform::TEST_PLATFORM_URL
}

pub(super) fn web_client_enabled() -> bool {
    // Remote integration tests use local mock platform URLs while exercising
    // the listener; production code always checks the actual environment.
    #[cfg(test)]
    {
        true
    }
    #[cfg(not(test))]
    {
        web_client_enabled_for_platform(&crate::future_platform::current_platform_url())
    }
}

/// Bind the web-client listener up front (in `start()`) so a busy port surfaces
/// in the returned status instead of a silent web_url that goes nowhere.
pub(super) async fn bind_web_listener() -> Result<tokio::net::TcpListener, crate::AppError> {
    tokio::net::TcpListener::bind(("0.0.0.0", WEB_PORT))
        .await
        .map_err(|error| {
            crate::AppError::Message(format!(
                "web server bind on port {WEB_PORT} failed: {error}"
            ))
        })
}

/// Best-effort LAN IPv4 address so a phone on the same network can reach the
/// `0.0.0.0` web client (the GUI only knows `localhost`). Uses the classic
/// "connect a UDP socket and read the local endpoint" trick, which selects a
/// default route without sending any packets; `None` when there's no route.
/// The probe target is an IPv4 literal, so the selected source address is
/// always IPv4 — no address-family arm is needed.
pub(super) fn lan_ip() -> Option<String> {
    use std::net::UdpSocket;
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}

/// Serve the web client from `desktop/web/` on the already-bound listener.
/// Reads each file per request so edits are picked up on browser refresh
/// without rebuilding. Aborts on `stop()`.
pub(super) fn spawn_web_server(listener: tokio::net::TcpListener) -> tokio::task::JoinHandle<()> {
    let web_dir = web_dir();
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(WEB_MAX_CONNECTIONS));
    tokio::spawn(async move {
        eprintln!(
            "remote: web client at http://localhost:{WEB_PORT} (serving {})",
            web_dir.display()
        );
        loop {
            // Acquire the permit BEFORE accepting: at capacity the loop blocks
            // here instead of accepting sockets it can't serve. The semaphore
            // is never closed, so acquisition cannot fail.
            let permit = semaphore
                .clone()
                .acquire_owned()
                .await
                .expect("web connection semaphore is never closed");
            // An accept failure drops the permit with the temporary and tries
            // the next connection.
            if let Ok((stream, _)) = listener.accept().await {
                spawn_web_connection(permit, stream, web_dir.clone());
            }
        }
    })
}

pub(super) fn spawn_web_connection(
    permit: tokio::sync::OwnedSemaphorePermit,
    mut stream: tokio::net::TcpStream,
    web_dir: std::path::PathBuf,
) {
    tokio::spawn(async move {
        let _permit = permit; // held until the handler returns
        handle_web_request(&mut stream, &web_dir).await;
    });
}

pub(super) async fn handle_web_request(
    stream: &mut tokio::net::TcpStream,
    web_dir: &std::path::Path,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut buf = vec![0u8; 8192];
    let n = match tokio::time::timeout(web_read_timeout(), stream.read(&mut buf)).await {
        Ok(Ok(n)) => n,
        _ => return, // read error or a client that never sent a request
    };
    let request = String::from_utf8_lossy(&buf[..n]);
    // Parse path from "GET /path HTTP/1.1" — default to index.html.
    let path = request
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    // URL paths never need Windows separators or drive/ADS prefixes. Reject
    // them on every host before Path::join can replace the serving root.
    if path.contains("..") || path.contains(['\\', ':']) {
        let resp = "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let _ = stream.write_all(resp.as_bytes()).await;
        return;
    }
    let file_path = web_dir.join(path);
    // In development, serve the on-disk file so browser refreshes pick up web
    // client edits. Release bundles do not contain the source checkout, so
    // fall back to the copy embedded at compile time for the entry page.
    let content = match tokio::fs::read(&file_path).await {
        Ok(content) => Some(content),
        Err(_) if path == "index.html" => Some(EMBEDDED_WEB_INDEX.to_vec()),
        Err(_) => None,
    };
    match content {
        Some(content) => {
            let content_type = if path.ends_with(".html") {
                "text/html; charset=utf-8"
            } else if path.ends_with(".js") {
                "application/javascript"
            } else if path.ends_with(".css") {
                "text/css"
            } else {
                "application/octet-stream"
            };
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                content.len()
            );
            let _ = stream.write_all(header.as_bytes()).await;
            let _ = stream.write_all(&content).await;
        }
        None => {
            let body = "Not Found";
            let resp = format!(
                "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes()).await;
        }
    }
}
