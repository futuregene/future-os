//! Browser endpoint targets — a CDP endpoint is either an `http(s)` URL or a
//! local socket.
//!
//! A socket target exists because some environments expose Chrome's DevTools
//! endpoint *only* on a Unix socket. Android is the motivating case: Chrome
//! listens on the abstract socket `@chrome_devtools_remote`, and nothing can
//! reach it over TCP without a third-party relay process bridging the two.
//! Accepting the socket directly removes that process.
//!
//! Two spellings, both parsed from the same `endpoint` string the tool already
//! stores in `config.json` (so a saved socket endpoint persists like any other):
//!
//! ```text
//! unix:/data/local/tmp/chrome.sock     filesystem socket
//! abstract:chrome_devtools_remote      Linux/Android abstract socket
//! ```
//!
//! The abstract form is Linux-only, so that variant is compiled conditionally.

use std::fmt;

/// Where a CDP endpoint lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointTarget {
    /// `http(s)://host:port` — the ordinary case (localhost over TCP).
    Http(String),
    /// A local socket holding the DevTools endpoint.
    Socket(SocketSpec),
}

/// A Unix socket address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocketSpec {
    /// A filesystem path, e.g. `/data/local/tmp/chrome.sock`.
    Path(String),
    /// A Linux abstract socket (no filesystem entry), e.g. Chrome on Android.
    Abstract(String),
}

impl EndpointTarget {
    /// Parse an `endpoint` string.
    ///
    /// Anything `http`/`https` is an HTTP target; `unix:` and `abstract:` are
    /// socket targets. Unknown schemes are rejected rather than treated as a
    /// URL, so a typo fails loudly instead of producing a confusing connection
    /// error later.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let value = raw.trim();
        if value.is_empty() {
            return Err("browser endpoint is empty".to_string());
        }
        if let Some(rest) = value.strip_prefix("unix:") {
            let path = rest.trim();
            if path.is_empty() {
                return Err("browser endpoint \"unix:\" needs a socket path".to_string());
            }
            return Ok(EndpointTarget::Socket(SocketSpec::Path(path.to_string())));
        }
        if let Some(rest) = value.strip_prefix("abstract:") {
            let name = rest.trim();
            if name.is_empty() {
                return Err("browser endpoint \"abstract:\" needs a socket name".to_string());
            }
            return Ok(EndpointTarget::Socket(SocketSpec::Abstract(
                name.to_string(),
            )));
        }
        if value.starts_with("http://") || value.starts_with("https://") {
            return Ok(EndpointTarget::Http(value.to_string()));
        }
        Err(format!(
            "unsupported browser endpoint {value:?}: expected an http(s) URL, \"unix:<path>\", or \"abstract:<name>\""
        ))
    }

    /// The socket this endpoint names, if it is a socket endpoint.
    pub fn socket(&self) -> Option<&SocketSpec> {
        match self {
            EndpointTarget::Socket(spec) => Some(spec),
            EndpointTarget::Http(_) => None,
        }
    }

    /// The endpoint exactly as given, for messages and config round-trips.
    pub fn as_str(&self) -> &str {
        match self {
            EndpointTarget::Http(url) => url,
            EndpointTarget::Socket(SocketSpec::Path(path)) => path,
            EndpointTarget::Socket(SocketSpec::Abstract(name)) => name,
        }
    }
}

impl SocketSpec {
    /// Connect to the socket.
    ///
    /// Unix-only: other platforms have no `UnixStream` to return, so the
    /// function does not exist there. Callers live behind the same gate (see
    /// `chromium::socket_http` and `WebSocketTransport::connect_over_socket`),
    /// and a socket endpoint on such a platform is reported as unsupported
    /// rather than as a connection failure.
    #[cfg(unix)]
    pub async fn connect(&self) -> Result<tokio::net::UnixStream, String> {
        match self {
            SocketSpec::Path(path) => tokio::net::UnixStream::connect(path)
                .await
                .map_err(|e| format!("cannot connect to browser socket {path}: {e}")),
            SocketSpec::Abstract(name) => connect_abstract(name).await,
        }
    }
}

#[cfg(all(unix, any(target_os = "linux", target_os = "android")))]
async fn connect_abstract(name: &str) -> Result<tokio::net::UnixStream, String> {
    use std::os::linux::net::SocketAddrExt;
    let addr = std::os::linux::net::SocketAddr::from_abstract_name(name.as_bytes())
        .map_err(|e| format!("invalid abstract socket name {name:?}: {e}"))?;
    tokio::net::UnixStream::connect_addr(&addr)
        .await
        .map_err(|e| format!("cannot connect to abstract browser socket @{name}: {e}"))
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
async fn connect_abstract(name: &str) -> Result<tokio::net::UnixStream, String> {
    Err(format!(
        "abstract browser sockets (@{name}) only exist on Linux/Android; this platform cannot connect to one"
    ))
}

impl fmt::Display for SocketSpec {
    /// The canonical `endpoint` spelling, so a message can be pasted back into
    /// `--endpoint`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SocketSpec::Path(path) => write!(f, "unix:{path}"),
            SocketSpec::Abstract(name) => write!(f, "abstract:{name}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_urls_are_http_targets() {
        for url in ["http://127.0.0.1:9222", "https://example.com:443"] {
            let target = EndpointTarget::parse(url).expect("parse");
            assert!(matches!(target, EndpointTarget::Http(_)), "{url}");
            assert!(target.socket().is_none(), "{url}");
            assert_eq!(target.as_str(), url);
        }
    }

    #[test]
    fn socket_spellings_are_parsed() {
        let path = EndpointTarget::parse("unix:/data/local/tmp/chrome.sock").expect("parse");
        assert_eq!(
            path.socket(),
            Some(&SocketSpec::Path("/data/local/tmp/chrome.sock".to_string()))
        );
        // Round-trips: the Display form is what `--endpoint` accepts.
        assert_eq!(
            path.socket().unwrap().to_string(),
            "unix:/data/local/tmp/chrome.sock"
        );

        let abstracted = EndpointTarget::parse("abstract:chrome_devtools_remote").expect("parse");
        assert_eq!(
            abstracted.socket(),
            Some(&SocketSpec::Abstract("chrome_devtools_remote".to_string()))
        );
        assert_eq!(
            abstracted.socket().unwrap().to_string(),
            "abstract:chrome_devtools_remote"
        );

        // Surrounding whitespace is tolerated (the value often comes from a shell).
        let padded = EndpointTarget::parse("  unix:/tmp/a.sock  ").expect("parse");
        assert_eq!(
            padded.socket(),
            Some(&SocketSpec::Path("/tmp/a.sock".to_string()))
        );
    }

    #[test]
    fn a_socket_endpoint_never_carries_a_host_and_port() {
        // The distinction matters downstream: a socket target has no
        // http base, so callers must not build a URL from it.
        let target = EndpointTarget::parse("abstract:chrome_devtools_remote").expect("parse");
        assert!(target.socket().is_some());
        assert!(!matches!(target, EndpointTarget::Http(_)));
    }

    #[test]
    fn malformed_endpoints_are_rejected_with_the_expected_forms() {
        for raw in ["", "   "] {
            let err = EndpointTarget::parse(raw).unwrap_err();
            assert!(err.contains("empty"), "{raw:?}: {err}");
        }
        for raw in ["unix:", "unix:   ", "abstract:", "abstract:  "] {
            let err = EndpointTarget::parse(raw).unwrap_err();
            assert!(err.contains("needs a socket"), "{raw:?}: {err}");
        }
        // A scheme we do not know is an error, not a URL: silently treating it
        // as one would fail later with a misleading connection error.
        let err = EndpointTarget::parse("wss://example.com").unwrap_err();
        assert!(err.contains("unsupported browser endpoint"), "{err}");
        assert!(
            err.contains("unix:<path>"),
            "the error names the valid forms: {err}"
        );
    }

    /// An abstract socket is reachable by name — the Android case.
    ///
    /// Gated to Linux/Android because abstract sockets only exist there; the
    /// syscall is the same one Android Chrome's `@chrome_devtools_remote` needs,
    /// so a Linux CI run is the closest this repository gets to exercising it.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[tokio::test]
    async fn an_abstract_socket_is_reachable_by_name() {
        use std::os::linux::net::SocketAddrExt;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let name = format!("future-cdp-test-{}", std::process::id());
        let addr = std::os::linux::net::SocketAddr::from_abstract_name(name.as_bytes())
            .expect("abstract name");
        let listener = tokio::net::UnixListener::bind_addr(&addr).expect("bind abstract");

        let mut stream = SocketSpec::Abstract(name)
            .connect()
            .await
            .expect("connect by abstract name");
        let (mut server_side, _) = listener.accept().await.expect("accept");

        // Real traffic in both directions: the same socket, not just a
        // successful connect.
        stream.write_all(b"ping").await.expect("write");
        let mut buf = [0u8; 4];
        server_side.read_exact(&mut buf).await.expect("read");
        assert_eq!(&buf, b"ping");
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[tokio::test]
    async fn a_missing_abstract_socket_names_it_in_the_error() {
        let err = SocketSpec::Abstract("future-no-such-socket".to_string())
            .connect()
            .await
            .expect_err("no listener");
        assert!(err.contains("future-no-such-socket"), "{err}");
        assert!(err.contains('@'), "names the abstract form: {err}");
    }
}
