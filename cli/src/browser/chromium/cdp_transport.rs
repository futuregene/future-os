//! CDP transport — port of `cli/src/browser/chromium/cdp-transport.ts`.
//!
//! `WebSocketTransport` wraps a tokio-tungstenite WebSocket. A single
//! reader task fans inbound text frames and close events out to a broadcast
//! channel (multiple subscribers = the CdpConnection dispatch loop and any
//! test observers). `close()` is bounded so a Chrome that never completes
//! the close handshake cannot hang the CLI.

use futures_util::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// Transport event — an inbound text frame or a close notification.
#[derive(Debug, Clone)]
pub enum TransportEvent {
    /// Inbound CDP message (JSON text).
    Message(String),
    /// Connection closed (Some(reason) on close/error).
    Close(Option<String>),
}

/// `WebSocketTransport` — production transport over tokio-tungstenite.
pub struct WebSocketTransport {
    sink: mpsc::UnboundedSender<Message>,
    closed: Arc<AtomicBool>,
    events: broadcast::Sender<TransportEvent>,
    /// Kept alive so the runtime keeps the reader/writer tasks running while
    /// the transport lives.
    _tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl WebSocketTransport {
    /// Connect to a `ws(s)://` CDP endpoint. Resolves once the WebSocket
    /// handshake completes; bounded by `timeout_ms`.
    pub async fn connect(url: &str, timeout_ms: u64) -> Result<Self, String> {
        let ws = tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            tokio_tungstenite::connect_async(url),
        )
        .await
        .map_err(|_| format!("WebSocket connection timeout: {url}"))?
        .map_err(|e| format!("WebSocket connection failed: {e}"))?
        .0;
        Ok(Self::from_stream(ws))
    }

    /// Wrap an already-connected WebSocket stream.
    pub fn from_stream(ws: WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>) -> Self {
        let (mut sink, mut stream) = ws.split();
        let (sink_tx, mut sink_rx) = mpsc::unbounded_channel::<Message>();
        let (events_tx, _) = broadcast::channel::<TransportEvent>(1024);
        let closed = Arc::new(AtomicBool::new(false));

        // Writer task: forwards queued frames to the socket.
        let writer = tokio::spawn(async move {
            while let Some(msg) = sink_rx.recv().await {
                if sink.send(msg).await.is_err() {
                    break;
                }
            }
        });

        // Reader task: forwards text frames, notifies close.
        let reader_closed = closed.clone();
        let reader_events = events_tx.clone();
        let reader = tokio::spawn(async move {
            loop {
                match stream.next().await {
                    Some(Ok(Message::Text(text))) => {
                        if reader_events
                            .send(TransportEvent::Message(text.to_string()))
                            .is_err()
                        {
                            break; // no subscribers left
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        let _ =
                            reader_events.send(TransportEvent::Close(Some("closed".to_string())));
                        break;
                    }
                    Some(Ok(_)) => continue, // binary/ping/pong — ignore
                    Some(Err(e)) => {
                        let _ = reader_events
                            .send(TransportEvent::Close(Some(format!("WebSocket error: {e}"))));
                        break;
                    }
                }
            }
            reader_closed.store(true, Ordering::SeqCst);
        });

        WebSocketTransport {
            sink: sink_tx,
            closed,
            events: events_tx,
            _tasks: vec![writer, reader],
        }
    }

    /// `send(message)` — fire-and-forget; no-op once closed.
    pub fn send(&self, message: &str) {
        if self.closed.load(Ordering::SeqCst) {
            return;
        }
        let _ = self.sink.send(Message::Text(message.to_string()));
    }

    /// Subscribe to inbound events. Dropping the receiver unsubscribes.
    pub fn subscribe(&self) -> broadcast::Receiver<TransportEvent> {
        self.events.subscribe()
    }

    /// `close()` — bounded: give a normal close a brief chance (500 ms),
    /// then return so the short-lived CLI can finish instead of hanging.
    /// Idempotent: a second call returns immediately.
    pub async fn close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        // Subscribe BEFORE sending the close frame so a fast reply can never
        // be missed between the send and the subscription.
        let mut rx = self.events.subscribe();
        let _ = self.sink.send(Message::Close(None));
        // Wait for the peer's close frame (or the 500 ms bound).
        let _ = tokio::time::timeout(std::time::Duration::from_millis(500), async {
            loop {
                match rx.recv().await {
                    Ok(TransportEvent::Close(_)) => break,
                    Ok(_) => continue,
                    Err(_) => break,
                }
            }
        })
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_async;

    type ServerWs = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

    /// Read frames until the client's Close frame arrives (returns true) or
    /// the peer goes away (false). Shared so the stream-end exit is covered
    /// once by `peer_drop_without_close_ends_reader` instead of needing a
    /// drop-early variant per server.
    async fn next_close_frame(ws: &mut ServerWs) -> bool {
        while let Some(frame) = ws.next().await {
            if matches!(frame, Ok(Message::Close(_))) {
                return true;
            }
        }
        false
    }

    /// Read until the client goes away, so the server task completes
    /// promptly once the test drops its transport (its closing line must
    /// execute inside the test).
    async fn hold_until_peer_leaves(mut ws: ServerWs) {
        while ws.next().await.is_some() {}
    }

    /// Extract the text of a Message event (panic arm covered below).
    #[track_caller]
    fn expect_message(event: TransportEvent) -> String {
        match event {
            TransportEvent::Message(text) => text,
            other => panic!("expected message, got {other:?}"),
        }
    }

    #[test]
    fn expect_message_panics_on_close() {
        assert!(std::panic::catch_unwind(|| expect_message(TransportEvent::Close(None))).is_err());
    }

    /// A client that connects and drops WITHOUT a close frame: the shared
    /// close-reader returns false and the server task ends. Uses a raw WS
    /// client (dropping it closes the socket outright — the transport's
    /// background tasks would keep the connection alive).
    #[tokio::test(flavor = "multi_thread")]
    async fn peer_drop_without_close_ends_reader() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(stream).await.unwrap();
            let saw_close = next_close_frame(&mut ws).await;
            assert!(!saw_close);
        });
        let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
            .await
            .unwrap();
        drop(ws);
        tokio::time::timeout(std::time::Duration::from_secs(2), server)
            .await
            .expect("server finishes")
            .unwrap();
    }

    /// A server that accepts the handshake and then deliberately NEVER sends
    /// a close frame — matching the Windows CDP hang the TS test simulates
    /// with `WebSocketWithoutCloseFrame`.
    #[tokio::test(flavor = "multi_thread")]
    async fn close_is_bounded_when_peer_never_sends_close_frame() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(stream).await.unwrap();
            let mut close_frames = 0u32;
            if next_close_frame(&mut ws).await {
                close_frames += 1;
            }
            // HOLD the socket without answering (and without reading — that
            // would flush tungstenite's automatic Close reply).
            let _keep = ws;
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            close_frames
        });

        let transport = WebSocketTransport::connect(&format!("ws://{addr}"), 5000)
            .await
            .expect("connect");

        let started = std::time::Instant::now();
        transport.close().await;
        let first_close_ms = started.elapsed().as_millis();

        let started2 = std::time::Instant::now();
        transport.close().await;
        let second_close_ms = started2.elapsed().as_millis();

        drop(transport);
        let close_frames = tokio::time::timeout(std::time::Duration::from_secs(2), server)
            .await
            .expect("server task completes")
            .unwrap();
        assert_eq!(close_frames, 1);
        assert!(first_close_ms >= 400, "first close {first_close_ms}ms");
        assert!(first_close_ms < 1_500, "first close {first_close_ms}ms");
        assert!(second_close_ms < 50, "second close {second_close_ms}ms");
    }

    /// A server performing the NORMAL close handshake (replies to the
    /// client's close frame after a short delay).
    #[tokio::test(flavor = "multi_thread")]
    async fn close_returns_fast_on_normal_handshake() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(stream).await.unwrap();
            // Send one text frame on a schedule (BEFORE any close frame
            // arrives — tungstenite refuses data frames after that). It
            // lands while the client's close() is waiting → continue arm.
            let tickler = tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(40)).await;
                let _ = ws.send(Message::Text("late".to_string())).await;
                ws
            });
            let mut ws = tickler.await.unwrap();
            if next_close_frame(&mut ws).await {
                let _ = ws.send(Message::Close(None)).await;
            }
            hold_until_peer_leaves(ws).await;
        });

        let transport = WebSocketTransport::connect(&format!("ws://{addr}"), 5_000)
            .await
            .unwrap();
        // A spare subscriber keeps the reader task alive even if the text
        // arrives before close() subscribes.
        let _keep = transport.subscribe();
        transport.close().await;

        // send() after close is a no-op (closed flag set).
        transport.send("{\"id\":1}");
        drop(transport);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), server).await;
    }

    /// The close-wait loop breaks on a channel error when the event stream
    /// lags. The server floods from the moment the handshake lands, so by
    /// the time close() subscribes the reader has a permanent multi-thousand
    /// event backlog — the new receiver starts lagged.
    #[tokio::test(flavor = "multi_thread")]
    async fn close_wait_breaks_on_lagged_receiver() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(stream).await.unwrap();
            // Flood without bound; the loop ends when the dropped client
            // makes a send fail.
            tokio::spawn(async move {
                loop {
                    if ws.send(Message::Text("x".to_string())).await.is_err() {
                        break;
                    }
                }
                hold_until_peer_leaves(ws).await;
            });
        });
        let transport = WebSocketTransport::connect(&format!("ws://{addr}"), 5_000)
            .await
            .unwrap();
        // A spare subscriber keeps the reader task forwarding the flood even
        // before close() subscribes (a failed send would kill the reader).
        let _keep = transport.subscribe();
        // Let the flood build up, then close. The close-wait receiver lags
        // behind the multi-thousand-event backlog → channel error → break.
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        tokio::time::timeout(std::time::Duration::from_secs(10), transport.close())
            .await
            .expect("close returns");
        drop(transport);
        crate::test_env::wait_for(|| server.is_finished()).await;
    }

    /// Server-initiated close and abrupt TCP drop both notify subscribers.
    #[tokio::test(flavor = "multi_thread")]
    async fn inbound_close_frame_and_tcp_drop_notify_close() {
        // Variant A: server sends a Close frame.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(stream).await.unwrap();
            let _ = ws.send(Message::Close(None)).await;
            hold_until_peer_leaves(ws).await;
        });
        let transport = WebSocketTransport::connect(&format!("ws://{addr}"), 5_000)
            .await
            .unwrap();
        let mut rx = transport.subscribe();
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(event, TransportEvent::Close(Some(_))));
        drop(transport);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), server).await;

        // Variant B: server drops the TCP socket without a close frame.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let _ws = accept_async(stream).await.unwrap();
            drop(_ws); // abrupt drop
        });
        let transport = WebSocketTransport::connect(&format!("ws://{addr}"), 5_000)
            .await
            .unwrap();
        let mut rx = transport.subscribe();
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(event, TransportEvent::Close(_)));
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), server).await;
    }

    /// After the peer vanishes, queued writes start failing and the writer
    /// task exits (the `sink.send(...) .is_err() → break` arm). The burst is
    /// one tight loop so many sends reach the writer before the reader's
    /// close flag makes `send()` a no-op.
    #[tokio::test(flavor = "multi_thread")]
    async fn writer_breaks_when_peer_vanishes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let ws = accept_async(stream).await.unwrap();
            drop(ws);
        });
        let transport = WebSocketTransport::connect(&format!("ws://{addr}"), 5_000)
            .await
            .unwrap();
        for _ in 0..20_000 {
            transport.send("{\"id\":1}");
        }
        transport.close().await;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), server).await;
    }

    /// Binary and ping frames are ignored; text keeps flowing.
    #[tokio::test(flavor = "multi_thread")]
    async fn binary_and_ping_frames_are_ignored() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(stream).await.unwrap();
            let _ = ws.send(Message::Binary(vec![1, 2, 3])).await;
            let _ = ws.send(Message::Ping(vec![9])).await;
            let _ = ws.send(Message::Text("{\"hello\":true}".to_string())).await;
            hold_until_peer_leaves(ws).await;
        });
        let transport = WebSocketTransport::connect(&format!("ws://{addr}"), 5_000)
            .await
            .unwrap();
        let mut rx = transport.subscribe();
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(expect_message(event).contains("hello"));
        drop(transport);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), server).await;
    }

    /// With no subscribers left the reader task exits and flags closed.
    #[tokio::test(flavor = "multi_thread")]
    async fn reader_breaks_when_no_subscribers_remain() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(stream).await.unwrap();
            let _ = ws.send(Message::Text("first".to_string())).await;
            // Just hold the socket; the assertions are client-side.
            hold_until_peer_leaves(ws).await;
        });
        let transport = WebSocketTransport::connect(&format!("ws://{addr}"), 5_000)
            .await
            .unwrap();
        {
            // Take a receiver and drop it immediately: after that the
            // broadcast channel has no receivers, so the next inbound text
            // makes events.send fail → reader breaks → closed flag set.
            let _rx = transport.subscribe();
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        // close() must not hang regardless of the reader state.
        transport.close().await;
        drop(transport);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), server).await;
    }

    /// Connect error paths.
    #[tokio::test(flavor = "multi_thread")]
    async fn connect_failures() {
        // Refused → "connection failed".
        let err = WebSocketTransport::connect("ws://127.0.0.1:1", 500)
            .await
            .err()
            .map(|e| e.to_string());
        assert!(err.unwrap().contains("WebSocket connection failed"));

        // TCP open but handshake never completes → timeout.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hold = tokio::spawn(async move {
            let s = listener.accept().await;
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            drop(s);
        });
        let err = WebSocketTransport::connect(&format!("ws://{addr}"), 120)
            .await
            .err()
            .map(|e| e.to_string());
        assert!(err.unwrap().contains("WebSocket connection timeout"));
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), hold).await;
    }
}
