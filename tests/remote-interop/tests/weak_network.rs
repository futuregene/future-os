//! What a 512-KiB-chunk download costs when the client's link has delay and loss.
//!
//! The question this answers is not "how fast is the transfer" but "how much of
//! the transfer's time is the control plane": the client pulls each chunk with
//! its own request and gets its own acknowledgement, so a 4 MiB page is eight
//! pulls and eight acknowledgements, and the open proposal is to replace them
//! with one pull per group of four (a range batch). That change touches
//! request/ACK semantics, so it has to be justified by a measurement of round
//! trips under latency — not by a byte count, which it barely moves.
//!
//! Everything measured here is real: real encrypted Core NATS, a real broker,
//! the real client and server, the real 512-KiB chunking. The only fixture is the
//! link (`support/weak_link.rs`), which delays bytes in both directions and
//! charges a retransmission for lost packets. The host reaches the broker
//! directly; only the client's leg is weak.
//!
//! Two things this deliberately does not do, because they are the boundary of
//! what a number here means:
//!
//! * it does not limit bandwidth. Delay is the only propagation cost, so a
//!   transfer's time is round trips plus CPU and nothing else. A real link also
//!   has to move 4 MiB or 10 MiB, and on any link where that takes longer than
//!   the round trips saved, the saving shrinks in proportion.
//! * it does not model congestion-window collapse, a dropped connection, or a
//!   NATS message that never arrives. Those are the failures the protocol's retry
//!   and hash checks exist for, and 1% packet loss on a TCP stream produces none
//!   of them — it produces stalls.
use future_remote_client::Client;
use future_remote_interop::{Broker, Fixture, MemoryClientHost, DESKTOP};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use weak_link::{LinkProfile, WeakLink};

#[path = "support/weak_link.rs"]
mod weak_link;

/// The transfer chunk size every existing benchmark uses, and the protocol's
/// default.
const CHUNK_BYTES: usize = 512 * 1024;

/// One page, at one link profile, measured `reps` times on one warm connection.
///
/// Returns once the page's numbers are printed; nothing is asserted about wall
/// time beyond the correctness of the bytes, because a wall clock is a
/// measurement, not a contract.
async fn measure(
    page: &str,
    page_bytes: usize,
    profile: LinkProfile,
    reps: usize,
) -> anyhow::Result<()> {
    let broker = Broker::start().await?;
    let upstream: SocketAddr = broker
        .nats_url
        .strip_prefix("nats://")
        .expect("fixture broker url is nats://")
        .parse()?;
    let link = WeakLink::start(upstream, profile).await?;
    let bytes = vec![42_u8; page_bytes];
    let fixture = Fixture::start_transfer_routed(
        &broker.nats_url,
        &link.url(),
        &broker.websocket_url,
        bytes.clone(),
        CHUNK_BYTES,
        Duration::ZERO,
    )
    .await?;
    let host = Arc::new(MemoryClientHost::new(fixture.client_credentials.clone()));
    let client = Client::new(host);
    // Connect once, then measure downloads on the warm connection: the handshake
    // is not what the range-batch proposal changes, and folding it in would blunt
    // every comparison by the same constant.
    let connecting = Instant::now();
    client
        .connect_with_emitter(DESKTOP, Some(Arc::new(|_| {})))
        .await?;
    let connect_ms = ms(connecting.elapsed());

    let chunks = page_bytes.div_ceil(CHUNK_BYTES) as u64;
    let mut measured = Vec::new();
    for _ in 0..reps {
        let before = link.counters();
        let reads_before = fixture.metrics()["chunkReads"].as_u64().unwrap_or(0);
        let started = Instant::now();
        let downloaded = client
            .download(DESKTOP, "interop_session", "fixture.bin", None, "original")
            .await?;
        let elapsed = ms(started.elapsed());
        let counts = link.counters().since(before);
        let reads = fixture.metrics()["chunkReads"].as_u64().unwrap_or(0) - reads_before;
        assert_eq!(
            downloaded.bytes, bytes,
            "the weak link must not change the bytes it carries"
        );
        // A loss here costs time, never a message: TCP redelivers, so the chunk
        // still arrives on its first pull and the protocol's retry path is not
        // entered. If that ever stops holding, the numbers mean something else.
        assert_eq!(
            counts.client_pulls, chunks,
            "one pull per chunk, even on a lossy link"
        );
        assert_eq!(counts.broker_chunks, chunks, "one chunk message per chunk");
        assert_eq!(reads, chunks, "one host-side read per chunk");
        measured.push(json!({
            "elapsedMs": elapsed,
            "chunkReads": reads,
            "wire": counts.snapshot_json(),
        }));
    }
    let mut sorted: Vec<f64> = measured
        .iter()
        .map(|rep| rep["elapsedMs"].as_f64().unwrap_or(f64::NAN))
        .collect();
    sorted.sort_by(f64::total_cmp);
    println!(
        "CELL {}",
        json!({
            "page": page,
            "pageBytes": page_bytes,
            "chunkBytes": CHUNK_BYTES,
            "chunks": chunks,
            "oneWayMs": ms(profile.one_way),
            "rttMs": ms(profile.rtt()),
            "lossPerMille": profile.loss_per_mille,
            "retransmitMs": ms(profile.retransmit),
            "seed": profile.seed,
            "reps": reps,
            "connectMs": connect_ms,
            "medianMs": sorted[sorted.len() / 2],
            "minMs": sorted[0],
            "maxMs": sorted[sorted.len() - 1],
            "runs": measured,
        })
    );
    client.shutdown(Duration::from_secs(5)).await?;
    fixture.shutdown().await?;
    Ok(())
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// The correctness half: a weak link changes the clock, not the protocol.
///
/// Kept out of `#[ignore]` because it is cheap and it is the check that would
/// catch a fixture that silently stopped delaying, stopped counting, or started
/// corrupting the stream.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_weak_link_delays_a_download_without_changing_it() -> anyhow::Result<()> {
    let broker = Broker::start().await?;
    let upstream: SocketAddr = broker
        .nats_url
        .strip_prefix("nats://")
        .expect("fixture broker url is nats://")
        .parse()?;
    // 40 ms round trip, no loss: enough delay that a missing one is visible.
    let profile = LinkProfile::clean(Duration::from_millis(20));
    let link = WeakLink::start(upstream, profile).await?;
    let bytes = vec![7_u8; 4 * 1024 * 1024];
    let fixture = Fixture::start_transfer_routed(
        &broker.nats_url,
        &link.url(),
        &broker.websocket_url,
        bytes.clone(),
        CHUNK_BYTES,
        Duration::ZERO,
    )
    .await?;
    let host = Arc::new(MemoryClientHost::new(fixture.client_credentials.clone()));
    let client = Client::new(host);
    client
        .connect_with_emitter(DESKTOP, Some(Arc::new(|_| {})))
        .await?;
    let before = link.counters();
    let started = Instant::now();
    let downloaded = client
        .download(DESKTOP, "interop_session", "fixture.bin", None, "original")
        .await?;
    let elapsed = started.elapsed();
    let counts = link.counters().since(before);
    assert_eq!(downloaded.bytes, bytes);
    assert_eq!(counts.connections, 0, "one connection, reused");
    assert_eq!(counts.client_pulls, 8, "4 MiB at 512 KiB is eight pulls");
    assert_eq!(counts.broker_chunks, 8);
    assert_eq!(counts.pull_acks, 8, "one acknowledgement per pull");
    assert_eq!(counts.control_messages(), 16);
    assert!(
        counts.client_bytes < 16 * 1024,
        "the uplink carries control only, not the page: {} bytes",
        counts.client_bytes
    );
    assert!(
        counts.broker_bytes >= bytes.len() as u64,
        "the downlink carries the page itself: {} bytes",
        counts.broker_bytes
    );
    assert_eq!(counts.lost_packets_client + counts.lost_packets_broker, 0);
    // Eight pulls in waves of four, plus prepare: a round trip each, so 40 ms of
    // link alone puts the floor at 120 ms. Deliberately loose — this asserts that
    // the delay is in the path at all, not what the wall clock is.
    assert!(
        elapsed >= Duration::from_millis(120),
        "the injected round trip must be on the path: {elapsed:?}"
    );
    client.shutdown(Duration::from_secs(5)).await?;
    fixture.shutdown().await?;
    Ok(())
}

/// The matrix. Slow by construction — every cell starts its own broker, client and
/// host on a real socket with real delay — so it is `#[ignore]`d like the other
/// transfer benchmarks. Run it with:
///
/// ```text
/// cargo test -p future-remote-interop --features network-tests --test weak_network \
///     -- --ignored --nocapture
/// ```
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "measured benchmark; real sockets, deliberate latency and loss"]
async fn weak_link_download_matrix() -> anyhow::Result<()> {
    for (page, page_bytes) in [("4MiB", 4 * 1024 * 1024), ("10MiB", 10 * 1024 * 1024)] {
        for rtt_ms in [2_u64, 40, 120, 300] {
            let one_way = Duration::from_millis(rtt_ms / 2);
            // Three repetitions where the link is clean, so a median means
            // something; two where it is lossy, because each repetition carries
            // tens of retransmission stalls and the second already shows whether
            // the number reproduces.
            measure(page, page_bytes, LinkProfile::clean(one_way), 3).await?;
            measure(page, page_bytes, LinkProfile::lossy(one_way, 10), 2).await?;
        }
    }
    Ok(())
}

/// The same matrix with the delay removed, to separate the link's cost from the
/// loopback floor. Also `#[ignore]`d: it is still a real broker and sockets.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "measured benchmark; real sockets"]
async fn weak_link_zero_delay_floor() -> anyhow::Result<()> {
    for (page, page_bytes) in [("4MiB", 4 * 1024 * 1024), ("10MiB", 10 * 1024 * 1024)] {
        measure(page, page_bytes, LinkProfile::clean(Duration::ZERO), 3).await?;
    }
    Ok(())
}
