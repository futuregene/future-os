//! Transfer measurements over real encrypted Core NATS on loopback.
//! `chunk_delay` is explicit host-side work per 512-KiB chunk, not an estimate
//! of public-relay RTT. Wall time is informational; overlap is asserted.
use future_remote_client::Client;
use future_remote_interop::{Broker, Fixture, MemoryClientHost, DESKTOP};
use std::{sync::Arc, time::Duration};

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
    client.shutdown(Duration::from_secs(5)).await?;
    fixture.shutdown().await?;
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
