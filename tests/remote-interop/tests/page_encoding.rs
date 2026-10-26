//! What a paged read costs on the wire and in time, with and without the
//! negotiated binary carrier (`read_binary_v1`).
//!
//! The two arms differ in exactly one decision — whether a chunk's bytes are
//! base64 inside the reply's JSON or appended behind it — so the page, the chunk
//! size, the client, the broker and the sealed channel are all the same. Two
//! measurements are reported separately because they answer different questions:
//!
//! * **bytes** are exact and deterministic: the encoded reply length from the
//!   production encoders (`future_remote_server::reply`) plus the fixed per
//!   message seal overhead (`HEADER_LEN + TAG_LEN`). This is the part the change
//!   can move, and it does not depend on this machine.
//! * **elapsed** is a loopback run: two arms, same page, real encrypted NATS. A
//!   loopback link has effectively no RTT and no loss, so the difference it
//!   shows is the encode/decode work, not a network property. It is reported as
//!   what it is rather than as a promise about a phone on a slow link.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use future_remote_interop::{Broker, Fixture, MemoryClientHost, PageEncoding, DESKTOP};
use future_remote_protocol::json::{encode_binary_frame, split_binary_frame};
use future_remote_server::reply::{
    encode_binary_reply_payload_with_gzip, encode_reply_payload_with_gzip, with_base64_page,
};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The desktop's chunk size (`remote_host::read_pages::CHUNK_BYTES`).
const CHUNK_BYTES: usize = 192 * 1024;
/// One sealed record's fixed overhead: header + AEAD tag.
const SEAL_OVERHEAD: usize = future_remote_crypto::HEADER_LEN + future_remote_crypto::TAG_LEN;

/// A page of session entries in the shape the desktop pages, of roughly `target`
/// JSON bytes.
///
/// Real long-form text rather than one repeated character or paragraph: a page of
/// `'x'` compresses ~1000×, and even a repeated paragraph compresses far better
/// than a transcript, which would flatter both arms — and would make the base64
/// arm look *worse* than it is, since base64 of already-incompressible bytes is
/// the expensive case. The word stream below is deterministic and gzips at
/// roughly what real assistant prose does (about 2.5–3×).
fn history_page(target: usize) -> Vec<u8> {
    history_page_of(target, &|index| {
        prose(index as u64, 40 + (index * 17) % 220)
    })
}

/// The same page shape over another kind of body text, so a number can be quoted
/// for more than one corpus. Base64's cost depends on what the compressor can do
/// with the expanded alphabet, which is a property of the *content*.
fn history_page_of(target: usize, body: &dyn Fn(usize) -> String) -> Vec<u8> {
    let mut entries = Vec::new();
    let mut size = 0usize;
    while size < target {
        let id = format!("entry_{:04}", entries.len());
        let text = body(entries.len());
        let entry = json!({
            "id": id,
            "kind": if entries.len() % 2 == 0 { "user" } else { "assistant" },
            "role": if entries.len() % 2 == 0 { "user" } else { "assistant" },
            "runId": "run_page",
            "blocks": [{ "kind": "text", "text": text }],
        });
        size += serde_json::to_vec(&entry).unwrap().len();
        entries.push(entry);
    }
    serde_json::to_vec(&json!({ "entries": entries, "version": 1 })).unwrap()
}

/// `words` words of English-looking prose, reproducibly.
///
/// A linear congruential generator rather than the `rand` crate so the corpus is
/// the same bytes on every run and every machine: a number that moves because the
/// input changed is not a measurement.
fn prose(seed: u64, words: usize) -> String {
    const VOCABULARY: [&str; 32] = [
        "remote",
        "chunk",
        "snapshot",
        "offset",
        "reader",
        "writer",
        "carrier",
        "encoding",
        "session",
        "transport",
        "payload",
        "compress",
        "handshake",
        "capability",
        "negotiate",
        "bounded",
        "immutable",
        "conversation",
        "projection",
        "timeline",
        "cursor",
        "reply",
        "command",
        "approval",
        "workspace",
        "provider",
        "settings",
        "attachment",
        "transfer",
        "directory",
        "history",
        "replay",
    ];
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut text = String::with_capacity(words * 7);
    for index in 0..words {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        text.push_str(VOCABULARY[(state % VOCABULARY.len() as u64) as usize]);
        // Sentence-ish structure: a full stop every so often, a newline rarely,
        // so the compressor sees the same short-range redundancy real text has.
        match (state >> 8) % 23 {
            0 => text.push_str(". "),
            5 => text.push_str(", "),
            17 => text.push_str(".\n"),
            _ => {
                if index + 1 == words {
                    text.push('.');
                } else {
                    text.push(' ');
                }
            }
        }
    }
    text
}

/// A shell/tool transcript: dense with punctuation, hex and paths, which is what
/// a page carrying command output looks like.
fn tool_output(index: usize) -> String {
    let mut text = String::new();
    for line in 0..(20 + index % 40) {
        text.push_str(&format!(
            "[exit: 0] ./scripts/run.mjs --step {line} --dir src/features/remote-{index}/k{line}\n     \
             chunk {line} of {:x} bytes at 0x{:x}\n",
            0x4000 + line * 31,
            0x1000_0000u64 + (line as u64) * 4093,
        ));
    }
    text
}

/// A page of identifiers and numbers: text gzip cannot model well, which is the
/// shape where base64's expansion costs the most and where the frame's saving is
/// largest for a page that is still plain JSON.
fn high_entropy(index: usize) -> String {
    let mut state = 0x2545_F491_4F6C_DD1Du64 ^ (index as u64);
    let mut text = String::new();
    for _ in 0..(120 + index % 80) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        text.push_str(&format!("{:016x}-{:08x} ", state, (state >> 32) as u32));
    }
    text
}

/// One page chunk's address, exactly as `remote_host::read_pages` writes it.
fn chunk_header(id: &str, page: &[u8], offset: usize) -> (Value, usize) {
    let end = (offset + CHUNK_BYTES).min(page.len());
    (
        json!({ "readChunk": {
            "id": id, "offset": offset, "nextOffset": end, "totalBytes": page.len(),
        }}),
        end,
    )
}

/// The encoded reply bytes the two carriers put on the wire for one page.
///
/// `gzip` is the negotiation this client makes (`reply_gzip_v1`), so it is on in
/// both arms: the change is what the compressor is fed, not whether it runs.
fn wire_bytes(page: &[u8], encoding: PageEncoding) -> (usize, usize) {
    let (mut messages, mut bytes, mut offset) = (0, 0, 0);
    while offset < page.len() {
        let (header, end) = chunk_header("read_fixture", page, offset);
        let encoded = match encoding {
            PageEncoding::Base64 => encode_reply_payload_with_gzip(
                &json!({
                    "type": "response", "success": true,
                    "data": with_base64_page(header, &page[offset..end]), "error": Value::Null,
                }),
                true,
            ),
            PageEncoding::Binary => {
                let document = serde_json::to_vec(&json!({
                    "type": "response", "success": true, "data": header, "error": Value::Null,
                }))
                .unwrap();
                encode_binary_reply_payload_with_gzip(
                    encode_binary_frame(&document, &page[offset..end]).unwrap(),
                    true,
                )
            }
        };
        // One sealed record per chunk, so the fixed overhead is added once per
        // message rather than estimated at the page level.
        messages += 1;
        bytes += encoded.len() + SEAL_OVERHEAD;
        offset = end;
    }
    (messages, bytes)
}

/// Raw bytes seen on one direction of the client↔broker leg.
///
/// The measurement the encoder figures are *checked against*: a counting TCP
/// proxy between the client and the broker sees exactly what NATS carries, so a
/// number here cannot be an artefact of what this test believes the host sends.
#[derive(Default)]
struct WireCount {
    to_broker: AtomicUsize,
    to_client: AtomicUsize,
}

impl WireCount {
    fn snapshot(&self) -> (usize, usize) {
        (
            self.to_broker.load(Ordering::SeqCst),
            self.to_client.load(Ordering::SeqCst),
        )
    }
}

async fn copy_counted(
    read: &mut (impl AsyncRead + Unpin),
    write: &mut (impl AsyncWrite + Unpin),
    counter: &AtomicUsize,
) {
    let mut buffer = [0; 16 * 1024];
    while let Ok(read) = read.read(&mut buffer).await {
        if read == 0 {
            break;
        }
        counter.fetch_add(read, Ordering::SeqCst);
        if write.write_all(&buffer[..read]).await.is_err() {
            break;
        }
    }
    let _ = write.shutdown().await;
}

/// A loopback TCP proxy that counts both directions, and the URL to reach it.
async fn counting_proxy(
    upstream: std::net::SocketAddr,
) -> anyhow::Result<(String, Arc<WireCount>, tokio::task::JoinHandle<()>)> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let address = listener.local_addr()?;
    let count = Arc::new(WireCount::default());
    let proxy = {
        let count = count.clone();
        tokio::spawn(async move {
            loop {
                let Ok((client, _)) = listener.accept().await else {
                    break;
                };
                let count = count.clone();
                tokio::spawn(async move {
                    let Ok(server) = TcpStream::connect(upstream).await else {
                        return;
                    };
                    let (mut client_read, mut client_write) = client.into_split();
                    let (mut server_read, mut server_write) = server.into_split();
                    tokio::join!(
                        copy_counted(&mut client_read, &mut server_write, &count.to_broker),
                        copy_counted(&mut server_read, &mut client_write, &count.to_client)
                    );
                });
            }
        })
    };
    Ok((format!("nats://{address}"), count, proxy))
}

/// The page's chunk sizes as a reader sees them, so the numbers can be checked
/// without trusting the encoder.
fn chunk_offsets(page: &[u8]) -> Vec<(usize, usize)> {
    let mut offsets = Vec::new();
    let mut offset = 0;
    while offset < page.len() {
        let end = (offset + CHUNK_BYTES).min(page.len());
        offsets.push((offset, end));
        offset = end;
    }
    offsets
}

#[test]
fn a_binary_page_is_smaller_than_its_base64_twin_chunk_by_chunk() {
    for (corpus, build) in [
        ("prose", history_page as fn(usize) -> Vec<u8>),
        ("tool-output", |target: usize| {
            history_page_of(target, &|index| tool_output(index))
        }),
        ("hex-identifiers", |target: usize| {
            history_page_of(target, &|index| high_entropy(index))
        }),
    ] {
        for target in [512 * 1024, 4 * 1024 * 1024] {
            let page = build(target);
            let (messages, base64) = wire_bytes(&page, PageEncoding::Base64);
            let (binary_messages, binary) = wire_bytes(&page, PageEncoding::Binary);
            assert_eq!(messages, binary_messages, "the same page, the same chunks");
            assert_eq!(messages, chunk_offsets(&page).len());
            assert!(
                binary < base64,
                "the frame must be smaller than base64 inside JSON: {binary} vs {base64}"
            );
            println!(
                "RESULT {}",
                json!({
                    "benchmark": "page-encoding-bytes",
                    "corpus": corpus,
                    "pageBytes": page.len(),
                    "chunks": messages,
                    "base64WireBytes": base64,
                    "binaryWireBytes": binary,
                    "savedBytes": base64 - binary,
                    "savedPercent": ((base64 - binary) as f64) * 100.0 / (base64 as f64),
                })
            );
        }
    }
}

/// The reader's own cost: base64-decoding a chunk versus splitting a frame.
///
/// This is CPU, measured on the same bytes the wire measurement uses. It is the
/// half of the change that a phone pays on every chunk it pulls.
#[test]
fn decoding_a_frame_is_cheaper_than_decoding_base64() {
    let page = history_page(4 * 1024 * 1024);
    let (offset, end) = chunk_offsets(&page)[3];
    let body = page[offset..end].to_vec();
    let (header, _) = chunk_header("read_fixture", &page, offset);
    let encoded = with_base64_page(header.clone(), &body);
    let base64_chunk = serde_json::to_vec(&json!({ "readChunk": encoded["readChunk"] })).unwrap();
    let document = serde_json::to_vec(&json!({
        "type": "response", "success": true, "data": header, "error": Value::Null,
    }))
    .unwrap();
    let frame = encode_binary_frame(&document, &body).unwrap();

    const ROUNDS: usize = 64;
    let started = std::time::Instant::now();
    for _ in 0..ROUNDS {
        let envelope: Value = serde_json::from_slice(&base64_chunk).unwrap();
        let data = envelope["readChunk"]["data"].as_str().unwrap();
        let bytes = URL_SAFE_NO_PAD.decode(data).unwrap();
        assert_eq!(bytes.len(), body.len());
    }
    let base64_nanos = started.elapsed().as_nanos() / ROUNDS as u128;

    let started = std::time::Instant::now();
    for _ in 0..ROUNDS {
        let split = split_binary_frame(&frame).unwrap().unwrap();
        let _envelope: Value = serde_json::from_slice(split.header).unwrap();
        assert_eq!(split.body.len(), body.len());
    }
    let frame_nanos = started.elapsed().as_nanos() / ROUNDS as u128;

    // The host pays the mirror image of this on every chunk it serves, so the
    // same comparison is made on the encoding side.
    let header = chunk_header("read_fixture", &page, offset).0;
    let started = std::time::Instant::now();
    for _ in 0..ROUNDS {
        let data = with_base64_page(header.clone(), &body);
        let encoded = serde_json::to_vec(&json!({
            "type": "response", "success": true, "data": data, "error": Value::Null,
        }))
        .unwrap();
        assert!(encoded.len() > body.len());
    }
    let base64_encode_nanos = started.elapsed().as_nanos() / ROUNDS as u128;

    let document = serde_json::to_vec(&json!({
        "type": "response", "success": true, "data": header, "error": Value::Null,
    }))
    .unwrap();
    let started = std::time::Instant::now();
    for _ in 0..ROUNDS {
        let frame = encode_binary_frame(&document, &body).unwrap();
        assert!(frame.len() > body.len());
    }
    let frame_encode_nanos = started.elapsed().as_nanos() / ROUNDS as u128;

    println!(
        "RESULT {}",
        json!({
            "benchmark": "page-encoding-cpu",
            "chunkBytes": body.len(),
            "base64DecodeNanos": base64_nanos,
            "frameDecodeNanos": frame_nanos,
            "base64EncodeNanos": base64_encode_nanos,
            "frameEncodeNanos": frame_encode_nanos,
        })
    );
}

/// Both carriers over the real encrypted transport, on the same page.
///
/// Correctness first — the reassembled page must be byte-identical to the
/// snapshot in both arms — then the loopback timing, reported as RESULT lines.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "benchmark; real sockets and measured wall time"]
async fn page_read_carrier_benchmark() -> anyhow::Result<()> {
    let page = history_page(4 * 1024 * 1024);
    for encoding in [PageEncoding::Base64, PageEncoding::Binary] {
        let broker = Broker::start().await?;
        let fixture = Fixture::start(&broker.nats_url, &broker.websocket_url).await?;
        fixture.host.install_page(page.clone());
        fixture.host.set_page_encoding(encoding);
        let host = Arc::new(MemoryClientHost::new(fixture.client_credentials.clone()));
        let client = future_remote_client::Client::new(host);
        client
            .connect_with_emitter(DESKTOP, Some(Arc::new(|_| {})))
            .await?;
        for round in 0..3 {
            let started = std::time::Instant::now();
            let read = client
                .request(
                    DESKTOP,
                    json!({ "type": "get_session_entries", "sessionId": "s", "chunkedRead": true }),
                    "s",
                )
                .await?;
            let elapsed = started.elapsed();
            assert_eq!(
                serde_json::to_vec(&read).unwrap(),
                page,
                "{encoding:?} must reassemble the snapshot byte for byte"
            );
            println!(
                "RESULT {}",
                json!({
                    "benchmark": "page-read-loopback",
                    "encoding": format!("{encoding:?}"),
                    "round": round,
                    "pageBytes": page.len(),
                    "chunks": chunk_offsets(&page).len(),
                    "elapsedMs": elapsed.as_secs_f64() * 1000.0,
                })
            );
        }
        client.shutdown(std::time::Duration::from_secs(5)).await?;
        fixture.shutdown().await?;
    }
    Ok(())
}

/// The capability is per connection: a client that never declares the frame gets
/// the base64 page even from a host that would serve one.
///
/// This is the fallback in the direction the wire decides, and it is not
/// hypothetical: every shipped desktop until this change sends the base64 page,
/// and every shipped phone until this change reads only that.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_undeclared_connection_receives_the_base64_page() -> anyhow::Result<()> {
    let broker = Broker::start().await?;
    let fixture = Fixture::start(&broker.nats_url, &broker.websocket_url).await?;
    let page = history_page(600 * 1024);
    fixture.host.install_page(page.clone());
    // The host serves the frame carrier; the client under test must still be
    // served what it can decode, which for an undeclared connection is base64.
    fixture.host.set_page_encoding(PageEncoding::Binary);
    let host = Arc::new(MemoryClientHost::new(fixture.client_credentials.clone()));
    let client = future_remote_client::Client::new(host);
    client
        .connect_with_emitter(DESKTOP, Some(Arc::new(|_| {})))
        .await?;
    let read = client
        .request(
            DESKTOP,
            json!({ "type": "get_session_entries", "sessionId": "s", "chunkedRead": true }),
            "s",
        )
        .await?;
    assert_eq!(serde_json::to_vec(&read).unwrap(), page);
    client.shutdown(std::time::Duration::from_secs(5)).await?;
    fixture.shutdown().await?;
    Ok(())
}

/// The fixture's page is a real snapshot: a chunk asked for at an offset that is
/// not a chunk boundary, or under another id, is refused rather than served.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_page_chunk_is_addressed_by_offset_and_snapshot() -> anyhow::Result<()> {
    let broker = Broker::start().await?;
    let fixture = Fixture::start(&broker.nats_url, &broker.websocket_url).await?;
    fixture.host.install_page(history_page(600 * 1024));
    let host = Arc::new(MemoryClientHost::new(fixture.client_credentials.clone()));
    let client = future_remote_client::Client::new(host);
    client
        .connect_with_emitter(DESKTOP, Some(Arc::new(|_| {})))
        .await?;
    let expired = client
        .request(
            DESKTOP,
            json!({ "type": "get_read_chunk", "sessionId": "s", "replyId": "read_other", "offset": CHUNK_BYTES }),
            "s",
        )
        .await;
    assert_eq!(
        expired.err().map(|error| error.to_string()),
        Some("remote_read_expired".to_string())
    );
    client.shutdown(std::time::Duration::from_secs(5)).await?;
    fixture.shutdown().await?;
    Ok(())
}

/// A chunk the fixture would answer is exactly the chunk the reader expects: the
/// header's own numbers describe the bytes that arrive beside it.
#[test]
fn a_chunk_header_describes_the_bytes_that_travel_with_it() {
    let page = history_page(600 * 1024);
    let (header, end) = chunk_header("read_fixture", &page, CHUNK_BYTES);
    assert_eq!(header["readChunk"]["offset"], json!(CHUNK_BYTES));
    assert_eq!(header["readChunk"]["nextOffset"], json!(end));
    assert_eq!(header["readChunk"]["totalBytes"], json!(page.len()));
    assert_eq!(end - CHUNK_BYTES, CHUNK_BYTES, "a middle chunk is full");
}

/// The two carriers, measured on the socket rather than on the encoder.
///
/// Same page, same broker, same sealed channel — a counting TCP proxy sits
/// between the client and the broker, so the numbers are the bytes NATS
/// actually carried and cannot be an artefact of what this test believes was
/// sent. Only the read window is counted: the proxy is sampled immediately
/// before the command and immediately after its reply, so handshake traffic and
/// the supervisor's own publishing stay out of it.
///
/// Both directions are reported because they answer different questions: the
/// broker→client leg is what the change is for, and the client→broker leg is the
/// requests, which the change deliberately does not touch (same commands, same
/// count).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_page_carrier_changes_the_bytes_on_the_socket() -> anyhow::Result<()> {
    let page = history_page(4 * 1024 * 1024);
    let mut measured = Vec::new();
    for encoding in [PageEncoding::Base64, PageEncoding::Binary] {
        let broker = Broker::start().await?;
        let upstream: std::net::SocketAddr =
            broker.nats_url.strip_prefix("nats://").unwrap().parse()?;
        let (url, count, proxy) = counting_proxy(upstream).await?;
        // The *host* keeps its direct connection; only the client is routed
        // through the proxy. Otherwise the host's own socket would carry the same
        // page bytes the other way and both directions would report the whole
        // conversation, which is what made the first attempt at this measurement
        // read ~5.4 MB in each direction for a 4.2 MB page.
        let fixture = Fixture::start(&broker.nats_url, &broker.websocket_url).await?;
        fixture.host.install_page(page.clone());
        fixture.host.set_page_encoding(encoding);
        let mut creds = fixture.client_credentials.clone();
        creds.nats_url = url;
        let host = Arc::new(MemoryClientHost::new(creds));
        let client = future_remote_client::Client::new(host);
        client
            .connect_with_emitter(DESKTOP, Some(Arc::new(|_| {})))
            .await?;
        // The presence beat is every 15 s and a page read takes tens of
        // milliseconds, so a beat landing inside the window is the only traffic
        // that can leak in.
        let (before_to_broker, before_to_client) = count.snapshot();
        let read = client
            .request(
                DESKTOP,
                json!({ "type": "get_session_entries", "sessionId": "s", "chunkedRead": true }),
                "s",
            )
            .await?;
        let (to_broker, to_client) = count.snapshot();
        assert_eq!(serde_json::to_vec(&read).unwrap(), page, "{encoding:?}");
        measured.push((
            encoding,
            to_client - before_to_client,
            to_broker - before_to_broker,
        ));
        client.shutdown(std::time::Duration::from_secs(5)).await?;
        fixture.shutdown().await?;
        proxy.abort();
    }
    let base64 = measured[0];
    let binary = measured[1];
    assert!(
        binary.1 < base64.1,
        "the frame must put fewer bytes on the socket: {} vs {}",
        binary.1,
        base64.1
    );
    for (encoding, to_client, to_broker) in measured {
        println!(
            "RESULT {}",
            json!({
                "benchmark": "page-encoding-socket",
                "encoding": format!("{encoding:?}"),
                "pageBytes": page.len(),
                "chunks": chunk_offsets(&page).len(),
                "brokerToClientBytes": to_client,
                "clientToBrokerBytes": to_broker,
                "savedPercent": ((base64.1 - binary.1) as f64) * 100.0 / (base64.1 as f64),
            })
        );
    }
    Ok(())
}
