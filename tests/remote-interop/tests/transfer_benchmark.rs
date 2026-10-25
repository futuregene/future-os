//! Transfer measurements over real encrypted Core NATS on loopback.
//! `chunk_delay` is explicit host-side work per 512-KiB chunk, not an estimate
//! of public-relay RTT. Wall time is informational; overlap is asserted.
use future_remote_client::Client;
use future_remote_interop::{Broker, Fixture, MemoryClientHost, DESKTOP};
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Default)]
struct WireCount {
    publish: std::sync::atomic::AtomicUsize,
    request: std::sync::atomic::AtomicUsize,
    bytes: std::sync::atomic::AtomicUsize,
}
async fn copy_counted(
    read: &mut (impl AsyncRead + Unpin),
    write: &mut (impl AsyncWrite + Unpin),
    count: Arc<WireCount>,
) {
    let mut bytes = Vec::new();
    let mut buffer = [0; 16 * 1024];
    while let Ok(read) = read.read(&mut buffer).await {
        if read == 0 {
            break;
        }
        count
            .bytes
            .fetch_add(read, std::sync::atomic::Ordering::SeqCst);
        bytes.extend_from_slice(&buffer[..read]);
        while let Some(newline) = bytes.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = bytes.drain(..=newline).collect();
            let text = String::from_utf8_lossy(&line);
            if text.starts_with("PUB ") {
                count
                    .publish
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            if text.starts_with("PUB ") && text.contains(".xfer.up.") {
                count
                    .request
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
        if write.write_all(&buffer[..read]).await.is_err() {
            break;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn indexed_download_reads_overlap_without_unbounded_fanout() -> anyhow::Result<()> {
    let broker = Broker::start().await?;
    let bytes = vec![42_u8; 4 * 1024 * 1024];
    let fixture = Fixture::start_transfer(
        &broker.nats_url,
        &broker.websocket_url,
        bytes.clone(),
        512 * 1024,
        Duration::from_millis(50),
    )
    .await?;
    let host = Arc::new(MemoryClientHost::new(fixture.client_credentials.clone()));
    let client = Client::new(host);
    client
        .connect_with_emitter(DESKTOP, Some(Arc::new(|_| {})))
        .await?;
    let downloaded = client
        .download(DESKTOP, "interop_session", "fixture.bin", None, "original")
        .await?;
    assert_eq!(downloaded.bytes, bytes);
    assert_eq!(fixture.metrics()["chunkMaxActive"], 4);
    assert_eq!(fixture.metrics()["chunkReads"], 8);
    assert_eq!(fixture.metrics()["uploadChunks"], 0);
    client.shutdown(Duration::from_secs(5)).await?;
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn download_wire_message_count() -> anyhow::Result<()> {
    let broker = Broker::start().await?;
    let upstream: std::net::SocketAddr =
        broker.nats_url.strip_prefix("nats://").unwrap().parse()?;
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
                        copy_counted(&mut client_read, &mut server_write, count.clone()),
                        copy_counted(
                            &mut server_read,
                            &mut client_write,
                            Arc::new(WireCount::default())
                        )
                    );
                });
            }
        })
    };
    let url = format!("nats://{address}");
    let bytes = vec![42_u8; 4 * 1024 * 1024];
    let fixture =
        Fixture::start_transfer(&url, &url, bytes.clone(), 512 * 1024, Duration::ZERO).await?;
    let host = Arc::new(MemoryClientHost::new(fixture.client_credentials.clone()));
    let client = Client::new(host);
    client
        .connect_with_emitter(DESKTOP, Some(Arc::new(|_| {})))
        .await?;
    assert_eq!(
        client
            .download(DESKTOP, "interop_session", "fixture.bin", None, "original")
            .await?
            .bytes,
        bytes
    );
    assert_eq!(count.request.load(std::sync::atomic::Ordering::SeqCst), 8);
    let transfer_bytes = count.bytes.load(std::sync::atomic::Ordering::SeqCst);
    println!(
        "RESULT {}",
        serde_json::json!({"fileBytes":bytes.len(), "downloadReads":fixture.metrics()["chunkReads"], "clientToBrokerPub":count.publish.load(std::sync::atomic::Ordering::SeqCst), "transferClientToBrokerBytes":transfer_bytes, "chunkRequests":count.request.load(std::sync::atomic::Ordering::SeqCst)})
    );
    client.request(DESKTOP, serde_json::json!({"type":"download_cancel", "transferId":fixture.host.download_ids()[0]}), "transfer").await?;
    client.shutdown(Duration::from_secs(5)).await?;
    fixture.shutdown().await?;
    proxy.abort();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "benchmark; real sockets and deliberate host-side latency"]
async fn transfer_latency_benchmark() -> anyhow::Result<()> {
    for chunk_delay_ms in [0, 50] {
        let broker = Broker::start().await?;
        let bytes = vec![42_u8; 4 * 1024 * 1024];
        let fixture = Fixture::start_transfer(
            &broker.nats_url,
            &broker.websocket_url,
            bytes.clone(),
            512 * 1024,
            Duration::from_millis(chunk_delay_ms),
        )
        .await?;
        let host = Arc::new(MemoryClientHost::new(fixture.client_credentials.clone()));
        let client = Client::new(host);
        client
            .connect_with_emitter(DESKTOP, Some(Arc::new(|_| {})))
            .await?;
        for _ in 0..3 {
            for direction in ["upload", "download"] {
                let started = std::time::Instant::now();
                if direction == "upload" {
                    assert!(!client
                        .upload(DESKTOP, "fixture.bin", &bytes)
                        .await?
                        .content_hash
                        .is_empty());
                } else {
                    assert_eq!(
                        client
                            .download(DESKTOP, "interop_session", "fixture.bin", None, "original")
                            .await?
                            .bytes,
                        bytes
                    );
                }
                println!(
                    "RESULT {}",
                    serde_json::json!({ "direction": direction, "fileBytes": bytes.len(),
                        "chunkBytes": 512 * 1024, "chunkDelayMs": chunk_delay_ms,
                        "elapsedMs": started.elapsed().as_millis() })
                );
            }
        }
        client.shutdown(Duration::from_secs(5)).await?;
        fixture.shutdown().await?;
    }
    Ok(())
}
