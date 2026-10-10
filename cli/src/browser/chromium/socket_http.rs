//! One HTTP request over a browser's Unix socket.
//!
//! Chrome exposes its DevTools endpoint as `GET /json/version`; when that
//! endpoint is a socket (Android's `@chrome_devtools_remote`, or a socket path
//! on any Unix host) this module speaks the request by hand. It exists as its
//! own module because the whole thing is Unix-only: [`SocketSpec::connect`]
//! yields a `UnixStream`, which other platforms do not have. Confining that to
//! one `cfg` keeps the callers platform-neutral.
//!
//! Nothing else here is generic: `/json/version` is the only request the CDP
//! HTTP surface is used for. Commands like `tabs` go over the WebSocket
//! (`Target.getTargets`), not `/json/list`.

use crate::browser::target::SocketSpec;

#[cfg(unix)]
pub async fn get(spec: &SocketSpec, path: &str) -> Result<(u16, Vec<u8>), String> {
    imp::get(spec, path).await
}

#[cfg(not(unix))]
pub async fn get(spec: &SocketSpec, _path: &str) -> Result<(u16, Vec<u8>), String> {
    Err(format!(
        "browser endpoint {spec} is a socket, which needs a Unix-like platform; this build cannot connect to it"
    ))
}

#[cfg(unix)]
mod imp {
    use super::SocketSpec;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Issue `GET <path>` and read the response incrementally.
    ///
    /// The response is parsed as soon as it is complete. Reading to EOF would be
    /// simpler and wrong: Chrome's DevTools HTTP server keeps the connection
    /// alive regardless of `Connection: close`, so `read_to_end` blocks until
    /// the caller's timeout even though the reply already arrived.
    pub async fn get(spec: &SocketSpec, path: &str) -> Result<(u16, Vec<u8>), String> {
        let mut stream = spec.connect().await?;
        let request =
            format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|e| format!("CDP request over socket {spec} failed: {e}"))?;
        stream
            .flush()
            .await
            .map_err(|e| format!("CDP request over socket {spec} failed: {e}"))?;

        let mut raw = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            if response_is_complete(&raw) {
                return parse_http_response(&raw).ok_or_else(|| {
                    format!("CDP over socket {spec} returned a malformed HTTP response")
                });
            }
            let read = stream
                .read(&mut chunk)
                .await
                .map_err(|e| format!("CDP response over socket {spec} could not be read: {e}"))?;
            if read == 0 {
                // Peer closed: a body framed `UntilEof` is only complete now,
                // and an under-run is reported as such rather than truncated.
                if raw.is_empty() {
                    return Err(format!("CDP over socket {spec} closed without a response"));
                }
                return parse_http_response(&raw).ok_or_else(|| {
                    format!(
                        "CDP over socket {spec} returned an incomplete HTTP response ({} bytes)",
                        raw.len()
                    )
                });
            }
            raw.extend_from_slice(&chunk[..read]);
        }
    }

    /// Body framing declared by the response headers.
    #[derive(Debug, PartialEq, Eq)]
    enum Framing {
        Length(usize),
        Chunked,
        /// No framing headers: the body ends when the peer closes.
        UntilEof,
    }

    /// Offset just past the `\r\n\r\n` that ends the head.
    fn header_end(raw: &[u8]) -> Option<usize> {
        raw.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
    }

    fn framing_of(head: &str) -> Framing {
        let mut length = None;
        let mut chunked = false;
        for line in head.split("\r\n").skip(1) {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim();
            if name.eq_ignore_ascii_case("content-length") {
                length = value.parse::<usize>().ok();
            } else if name.eq_ignore_ascii_case("transfer-encoding")
                && value.to_ascii_lowercase().contains("chunked")
            {
                chunked = true;
            }
        }
        if chunked {
            Framing::Chunked
        } else {
            match length {
                Some(n) => Framing::Length(n),
                None => Framing::UntilEof,
            }
        }
    }

    /// Whether `raw` holds a whole HTTP response.
    fn response_is_complete(raw: &[u8]) -> bool {
        let Some(end) = header_end(raw) else {
            return false;
        };
        let head = String::from_utf8_lossy(&raw[..end]);
        let body = &raw[end..];
        match framing_of(&head) {
            Framing::Length(n) => body.len() >= n,
            Framing::Chunked => matches!(dechunk_prefix(body), Ok(Some(_))),
            Framing::UntilEof => false,
        }
    }

    /// Parse an HTTP/1.1 response: status code plus body.
    ///
    /// Handles both framings Chrome may use — `Content-Length` and chunked —
    /// because guessing wrong surfaces as a JSON parse error rather than as the
    /// framing problem it is.
    fn parse_http_response(raw: &[u8]) -> Option<(u16, Vec<u8>)> {
        let end = header_end(raw)?;
        let head = String::from_utf8_lossy(&raw[..end]);
        let body = &raw[end..];
        let status_line = head.split("\r\n").next()?;
        let status: u16 = status_line.split_whitespace().nth(1)?.parse().ok()?;

        let body = match framing_of(&head) {
            Framing::Chunked => dechunk_prefix(body).ok().flatten()?.0,
            Framing::Length(length) => body.get(..length)?.to_vec(),
            Framing::UntilEof => body.to_vec(),
        };
        Some((status, body))
    }

    /// Decode a chunked body, or report that more bytes are needed.
    ///
    /// `Ok(None)` is the caller's signal to keep reading; `Err(())` means the
    /// framing is malformed and no extra input will fix it.
    fn dechunk_prefix(body: &[u8]) -> Result<Option<(Vec<u8>, usize)>, ()> {
        let mut out = Vec::new();
        let mut cursor = 0usize;
        loop {
            let Some(line_end) = body[cursor..]
                .windows(2)
                .position(|w| w == b"\r\n")
                .map(|i| i + cursor)
            else {
                return Ok(None); // size line not fully arrived
            };
            let size_line = String::from_utf8_lossy(&body[cursor..line_end]);
            // A chunk may carry extensions after `;`.
            let size = usize::from_str_radix(size_line.split(';').next().ok_or(())?.trim(), 16)
                .map_err(|_| ())?;
            cursor = line_end + 2;
            if size == 0 {
                return Ok(Some((out, cursor)));
            }
            let Some(data) = body.get(cursor..cursor + size) else {
                return Ok(None); // chunk data not fully arrived
            };
            out.extend_from_slice(data);
            cursor += size;
            // The CRLF that terminates the chunk data.
            match body.get(cursor..cursor + 2) {
                Some(b"\r\n") => cursor += 2,
                Some(_) => return Err(()),
                None => return Ok(None),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn http_response_framing_is_parsed_both_ways() {
            let with_length = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi";
            assert_eq!(
                parse_http_response(with_length),
                Some((200, b"hi".to_vec()))
            );

            let chunked =
                b"HTTP/1.1 404 Not Found\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nhi\r\n0\r\n\r\n";
            assert_eq!(parse_http_response(chunked), Some((404, b"hi".to_vec())));

            // No framing headers: read to the end of what arrived.
            let bare = b"HTTP/1.1 200 OK\r\n\r\nplain";
            assert_eq!(parse_http_response(bare), Some((200, b"plain".to_vec())));

            // Garbage is rejected rather than mis-parsed.
            assert_eq!(parse_http_response(b"not http at all"), None);
        }

        #[test]
        fn chunk_extensions_and_upper_case_hex_are_accepted() {
            let body =
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: CHUNKED\r\n\r\nA;ext=1\r\n0123456789\r\n0\r\n\r\n";
            assert_eq!(
                parse_http_response(body),
                Some((200, b"0123456789".to_vec()))
            );
            // An unfinished chunk asks for more input rather than truncating;
            // malformed framing is not retried.
            assert_eq!(dechunk_prefix(b"5\r\nab"), Ok(None));
            assert_eq!(dechunk_prefix(b"zz\r\n"), Err(()));
            assert_eq!(dechunk_prefix(b"5\r\nabcdeXX"), Err(()));
        }

        /// A complete response is recognised without waiting for the peer to
        /// close.
        ///
        /// Chrome's DevTools HTTP server keeps the connection alive even when
        /// asked to close, so "read until EOF" hangs until the timeout. This is
        /// the case that made the real socket path report `reachable: false`
        /// while every mock-based test passed (the mocks closed the socket).
        #[test]
        fn a_keep_alive_response_is_complete_without_eof() {
            let complete = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi";
            assert!(response_is_complete(complete));
            // Headers only, body still arriving.
            assert!(!response_is_complete(
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nh"
            ));
            assert!(!response_is_complete(b"HTTP/1.1 200 O"));

            let chunked_done =
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nhi\r\n0\r\n\r\n";
            assert!(response_is_complete(chunked_done));
            assert!(!response_is_complete(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nhi\r\n"
            ));

            // No framing headers: only EOF can end the body, so it is never
            // complete on its own.
            assert!(!response_is_complete(b"HTTP/1.1 200 OK\r\n\r\nbody"));
        }
    }
}
