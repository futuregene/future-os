//! A weak link on one leg of the fixture: one-way delay, the stall a lost packet
//! costs, and exact NATS protocol counting.
//!
//! This is a measurement instrument, not a simulator. It sits between the
//! fixture's **client** and the fixture's broker (the host connects directly, so
//! exactly one leg is weak, like a phone on a bad link talking to a host on a
//! good one). Three properties are deliberate:
//!
//! * **Bandwidth is unlimited.** A byte read at `t` is offered to the peer at
//!   `t + one_way`, whatever the read boundaries were: reads are queued with a
//!   release instant and a writer task honours it, so a 512-KiB message is not
//!   serialized into one delay per 16-KiB read. A "delayed read" proxy cannot
//!   measure this — it turns each read into a link slot, so a message's wall time
//!   becomes a function of the proxy's buffer size instead of the network's.
//!   Everything measured here is therefore a *round-trip* cost; the report says
//!   so: on a link where bytes also cost time these savings are an upper bound.
//! * **Loss is a stall, never a deletion.** TCP does not lose bytes; it delivers
//!   every byte once, in order, after a retransmission. A proxy that deletes read
//!   bytes corrupts the NATS frame stream (`MSG`/`PUB` payload lengths are
//!   explicit), so it measures a protocol error and a reconnect, not recovery.
//!   So a lost packet here costs `retransmit` (one RTT by default: the optimistic
//!   fast-retransmit case — a real RTO is 200 ms–1 s) and everything after it in
//!   the stream waits behind it, which is the head-of-line blocking a lost
//!   segment actually causes. Losses that land while the stream is already
//!   waiting share one recovery window rather than adding up, because one flight
//!   is recovered by one retransmission episode. Congestion-window collapse after
//!   loss is *not* modelled, so lossy numbers are a lower bound on the cost.
//! * **Loss is deterministic in byte space.** The decision for a virtual packet
//!   is a hash of the seed and the packet's index, not a running PRNG, so the
//!   same seed hits the same byte offsets no matter how reads happen to split.
//!
//! The counting is exact rather than estimated: headers are parsed
//! (`PUB`/`HPUB`/`MSG`/`HMSG`) and payloads are skipped by their declared
//! length, so a control-message count is a count of protocol messages.
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{tcp, TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Bytes per virtual packet for loss accounting: an Ethernet-sized segment, so
/// "1%" means 1% of MTU-sized packets rather than 1% of whatever buffer a read
/// happened to produce.
pub const SEGMENT_BYTES: u64 = 1_400;
/// Read size. Deliberately unrelated to [`SEGMENT_BYTES`]: the release time must
/// not depend on it (see the module comment).
const READ_BYTES: usize = 16 * 1024;
/// One fixed seed for every measurement, so two runs of the same profile lose the
/// same packets.
pub const LOSS_SEED: u64 = 0x5EED_0000_0000_0001;

/// The link's parameters. Only ever constructed by tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkProfile {
    /// Propagation delay in one direction; a round trip is twice this.
    pub one_way: Duration,
    /// Lost-packet probability in per-mille (10 = 1%).
    pub loss_per_mille: u32,
    /// What one lost packet costs the stream: a retransmission.
    pub retransmit: Duration,
    pub seed: u64,
}

impl LinkProfile {
    /// A link with delay and no loss.
    pub fn clean(one_way: Duration) -> Self {
        Self {
            one_way,
            loss_per_mille: 0,
            retransmit: Duration::ZERO,
            seed: LOSS_SEED,
        }
    }
    /// A link that drops `loss_per_mille` of its virtual packets, each costing
    /// one round trip — the optimistic fast-retransmit case.
    pub fn lossy(one_way: Duration, loss_per_mille: u32) -> Self {
        Self {
            one_way,
            loss_per_mille,
            retransmit: one_way.saturating_mul(2),
            seed: LOSS_SEED,
        }
    }
    pub fn rtt(&self) -> Duration {
        self.one_way.saturating_mul(2)
    }
}

/// Which way a byte is going.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// Client to broker: requests, pulls, upload chunks.
    Client,
    /// Broker to client: replies, events, download chunks.
    Broker,
}

/// Exact counters for one proxied leg, cumulative for the link's lifetime.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LinkCounts {
    pub client_bytes: u64,
    pub broker_bytes: u64,
    /// `PUB`/`HPUB` headers the client sent.
    pub client_publish: u64,
    /// Those of them that are chunk pulls (`...xfer.up.<t>.pull.<i>`).
    pub client_pulls: u64,
    /// `MSG`/`HMSG` headers the broker sent.
    pub broker_messages: u64,
    /// Those of them that carry a download chunk (`...xfer.down.<t>.chunk.<i>`).
    pub broker_chunks: u64,
    /// Those of them addressed to the reply subject of a pull this proxy saw the
    /// client send: exactly one acknowledgement per pull, counted rather than
    /// guessed from the shape of the subject.
    pub pull_acks: u64,
    pub lost_packets_client: u64,
    pub lost_packets_broker: u64,
    /// Milliseconds of retransmission stall injected, per direction.
    pub stall_ms_client: u64,
    pub stall_ms_broker: u64,
    pub connections: u64,
}

impl LinkCounts {
    /// What happened between two snapshots.
    pub fn since(self, before: Self) -> Self {
        Self {
            client_bytes: self.client_bytes - before.client_bytes,
            broker_bytes: self.broker_bytes - before.broker_bytes,
            client_publish: self.client_publish - before.client_publish,
            client_pulls: self.client_pulls - before.client_pulls,
            broker_messages: self.broker_messages - before.broker_messages,
            broker_chunks: self.broker_chunks - before.broker_chunks,
            pull_acks: self.pull_acks - before.pull_acks,
            lost_packets_client: self.lost_packets_client - before.lost_packets_client,
            lost_packets_broker: self.lost_packets_broker - before.lost_packets_broker,
            stall_ms_client: self.stall_ms_client - before.stall_ms_client,
            stall_ms_broker: self.stall_ms_broker - before.stall_ms_broker,
            connections: self.connections - before.connections,
        }
    }
    /// Control messages, counted rather than estimated: every pull the client
    /// sent plus every reply the host's acknowledgement arrived in.
    pub fn control_messages(&self) -> u64 {
        self.client_pulls + self.pull_acks
    }
    pub fn wire_bytes(&self) -> u64 {
        self.client_bytes + self.broker_bytes
    }
    pub fn snapshot_json(&self) -> Value {
        json!({
            "clientBytes": self.client_bytes,
            "brokerBytes": self.broker_bytes,
            "wireBytes": self.wire_bytes(),
            "clientPublish": self.client_publish,
            "pulls": self.client_pulls,
            "brokerMessages": self.broker_messages,
            "chunkMessages": self.broker_chunks,
            "pullAcks": self.pull_acks,
            "controlMessages": self.control_messages(),
            "lostPacketsClient": self.lost_packets_client,
            "lostPacketsBroker": self.lost_packets_broker,
            "stallMsClient": self.stall_ms_client,
            "stallMsBroker": self.stall_ms_broker,
            "connections": self.connections,
        })
    }
}

#[derive(Default)]
struct Counters {
    values: Mutex<LinkCounts>,
    /// Reply subjects of the pulls the client sent, so the acknowledgement that
    /// answers one is counted as an acknowledgement rather than as traffic.
    pull_replies: Mutex<std::collections::HashSet<String>>,
}

impl Counters {
    fn update(&self, change: impl FnOnce(&mut LinkCounts)) {
        let mut values = self.values.lock().expect("link counters poisoned");
        change(&mut values);
    }
    fn snapshot(&self) -> LinkCounts {
        *self.values.lock().expect("link counters poisoned")
    }
    fn expect_pull_reply(&self, subject: &str) {
        self.pull_replies
            .lock()
            .expect("link counters poisoned")
            .insert(subject.to_string());
    }
    fn is_pull_reply(&self, subject: &str) -> bool {
        self.pull_replies
            .lock()
            .expect("link counters poisoned")
            .contains(subject)
    }
}

/// The proxy itself. Dropping it stops accepting connections.
pub struct WeakLink {
    address: SocketAddr,
    counters: Arc<Counters>,
    accept: JoinHandle<()>,
}

impl WeakLink {
    /// Listen on a fresh loopback port and forward to `upstream` over a link with
    /// `profile`.
    pub async fn start(upstream: SocketAddr, profile: LinkProfile) -> anyhow::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
        let counters = Arc::new(Counters::default());
        let accept = {
            let counters = counters.clone();
            tokio::spawn(async move {
                loop {
                    let Ok((client, _)) = listener.accept().await else {
                        break;
                    };
                    let counters = counters.clone();
                    tokio::spawn(async move {
                        let Ok(server) = TcpStream::connect(upstream).await else {
                            return;
                        };
                        counters.update(|values| values.connections += 1);
                        client.set_nodelay(true).ok();
                        server.set_nodelay(true).ok();
                        let (client_read, client_write) = client.into_split();
                        let (server_read, server_write) = server.into_split();
                        tokio::join!(
                            pump(
                                client_read,
                                server_write,
                                profile,
                                Side::Client,
                                counters.clone()
                            ),
                            pump(server_read, client_write, profile, Side::Broker, counters),
                        );
                    });
                }
            })
        };
        Ok(Self {
            address,
            counters,
            accept,
        })
    }
    /// The endpoint the client should be pointed at.
    pub fn url(&self) -> String {
        format!("nats://{0}", self.address)
    }
    pub fn counters(&self) -> LinkCounts {
        self.counters.snapshot()
    }
}

impl Drop for WeakLink {
    fn drop(&mut self) {
        // A test that stops mid-measurement must not leave the accept loop or its
        // per-connection pumps alive to perturb the next one.
        self.accept.abort();
    }
}

/// Forward one direction: read as fast as the peer sends (never blocking on the
/// link), then let a writer task release each read at its scheduled instant.
async fn pump(
    mut read: tcp::OwnedReadHalf,
    mut write: tcp::OwnedWriteHalf,
    profile: LinkProfile,
    side: Side,
    counters: Arc<Counters>,
) {
    let (send, mut receive) = mpsc::unbounded_channel::<(tokio::time::Instant, Vec<u8>)>();
    let writer = tokio::spawn(async move {
        while let Some((release, bytes)) = receive.recv().await {
            tokio::time::sleep_until(release).await;
            if write.write_all(&bytes).await.is_err() {
                break;
            }
        }
        let _ = write.shutdown().await;
    });
    let mut schedule = LossSchedule::new(profile, side);
    let mut scan = NatsScan::default();
    let mut buffer = vec![0u8; READ_BYTES];
    loop {
        let read = match read.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        counters.update(|values| match side {
            Side::Client => values.client_bytes += read as u64,
            Side::Broker => values.broker_bytes += read as u64,
        });
        scan.feed(&buffer[..read], side, &counters);
        let release = schedule.release(read, &counters);
        if send.send((release, buffer[..read].to_vec())).is_err() {
            break;
        }
    }
    drop(send);
    let _ = writer.await;
}

/// When the next bytes may be released, and what the loss model charged for them.
///
/// Release instants are monotonic, so a stalled packet also holds back the bytes
/// behind it — the head-of-line blocking of one TCP stream — while bytes that
/// arrive when nothing is stalled are released one propagation delay after they
/// were read, however many reads they took.
struct LossSchedule {
    profile: LinkProfile,
    side: Side,
    /// Absolute byte offset of the next undecided virtual packet.
    offset: u64,
    next_release: tokio::time::Instant,
}

impl LossSchedule {
    fn new(profile: LinkProfile, side: Side) -> Self {
        Self {
            profile,
            side,
            offset: 0,
            next_release: tokio::time::Instant::now(),
        }
    }
    /// The instant these bytes may leave, charging any lost packets among them.
    fn release(&mut self, bytes: usize, counters: &Counters) -> tokio::time::Instant {
        let start = self.offset;
        let end = start + bytes as u64;
        self.offset = end;
        let mut lost = 0u64;
        if self.profile.loss_per_mille > 0 {
            // Each virtual packet is decided by the read that carries its first
            // byte, so a packet spanning two reads is still one decision.
            let mut packet = start / SEGMENT_BYTES;
            while packet * SEGMENT_BYTES < end {
                if packet * SEGMENT_BYTES >= start && self.drops(packet) {
                    lost += 1;
                }
                packet += 1;
            }
        }
        if lost > 0 {
            // Losses in one flight are recovered by one retransmission episode, not
            // one per packet: the stream is already waiting, and a second missing
            // packet in the same window costs the same wait, not twice the wait.
            // Charging them additively would make a 10 MiB page on a 1%-loss,
            // 300-ms-RTT link look like tens of seconds when the real cost is one
            // recovery window per burst of losses.
            let now = tokio::time::Instant::now();
            let clean = now + self.profile.one_way;
            let already_waiting = self.next_release > clean;
            let stall = if already_waiting {
                Duration::ZERO
            } else {
                self.profile.retransmit
            };
            let side = self.side;
            counters.update(|values| match side {
                Side::Client => {
                    values.lost_packets_client += lost;
                    values.stall_ms_client += stall.as_millis() as u64;
                }
                Side::Broker => {
                    values.lost_packets_broker += lost;
                    values.stall_ms_broker += stall.as_millis() as u64;
                }
            });
            // A clean link would have delivered these bytes at `clean`; a lost one
            // has them wait for a retransmission on top of that.
            self.next_release = self.next_release.max(clean + stall);
        }
        let release = (tokio::time::Instant::now() + self.profile.one_way).max(self.next_release);
        self.next_release = release;
        release
    }
    fn drops(&self, packet: u64) -> bool {
        let roll = mix(self.profile.seed ^ packet.wrapping_mul(0x9E37_79B9_7F4A_7C15)) % 1000;
        roll < u64::from(self.profile.loss_per_mille)
    }
}

/// A deterministic hash: the same `(seed, packet)` is lost on every run and every
/// machine, which is what makes two runs of the same profile comparable.
fn mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

/// Streaming NATS header parser: counts protocol messages and skips payloads by
/// their declared length, so a payload byte that happens to look like `PUB` is not
/// counted as one.
#[derive(Default)]
struct NatsScan {
    /// The header line being assembled across reads. Only ever holds a line.
    head: Vec<u8>,
    /// Payload bytes still to skip before the next header starts.
    skip: u64,
}

impl NatsScan {
    fn feed(&mut self, mut bytes: &[u8], side: Side, counters: &Counters) {
        while !bytes.is_empty() {
            if self.skip > 0 {
                let take = bytes.len().min(self.skip as usize);
                bytes = &bytes[take..];
                self.skip -= take as u64;
                continue;
            }
            let Some(position) = bytes.iter().position(|byte| *byte == b'\n') else {
                // A header line longer than the buffer is not a protocol line we
                // understand; dropping it is safer than growing without bound.
                if self.head.len() + bytes.len() <= READ_BYTES {
                    self.head.extend_from_slice(bytes);
                } else {
                    self.head.clear();
                }
                return;
            };
            self.head.extend_from_slice(&bytes[..position]);
            bytes = &bytes[position + 1..];
            let line = std::mem::take(&mut self.head);
            let line = line.strip_suffix(b"\r").unwrap_or(&line);
            if let Some(skip) = self.header(line, side, counters) {
                self.skip = skip;
            }
        }
    }
    /// One protocol line; returns how many payload bytes follow it.
    fn header(&mut self, line: &[u8], side: Side, counters: &Counters) -> Option<u64> {
        let text = std::str::from_utf8(line).ok()?;
        let mut fields = text.split(' ');
        let verb = fields.next()?;
        let rest: Vec<&str> = fields.collect();
        let (subject, payload) = match (verb, side) {
            // `PUB <subject> [reply] <size>`
            ("PUB", Side::Client) => (rest.first()?, rest.last()?.parse::<u64>().ok()?),
            // `HPUB <subject> [reply] <header> <total>`
            ("HPUB", Side::Client) => (rest.first()?, rest.last()?.parse::<u64>().ok()?),
            // `MSG <subject> <sid> [reply] <size>`
            ("MSG", Side::Broker) => (rest.first()?, rest.last()?.parse::<u64>().ok()?),
            // `HMSG <subject> <sid> [reply] <header> <total>`
            ("HMSG", Side::Broker) => (rest.first()?, rest.last()?.parse::<u64>().ok()?),
            _ => return None,
        };
        let chunk = subject.contains(".xfer.down.") && subject.contains(".chunk.");
        let pull = subject.contains(".xfer.up.") && subject.contains(".pull.");
        if pull {
            // `PUB <subject> <reply> <size>`: the acknowledgement of this pull
            // arrives on `rest[1]`.
            if let Some(reply) = rest.get(1) {
                counters.expect_pull_reply(reply);
            }
        }
        let acknowledgement = side == Side::Broker && counters.is_pull_reply(subject);
        counters.update(|values| match side {
            Side::Client => {
                values.client_publish += 1;
                if pull {
                    values.client_pulls += 1;
                }
            }
            Side::Broker => {
                values.broker_messages += 1;
                if chunk {
                    values.broker_chunks += 1;
                }
                if acknowledgement {
                    values.pull_acks += 1;
                }
            }
        });
        // The declared payload, and the CRLF that terminates it.
        Some(payload + 2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Splits `total` bytes into reads of `size`, with the remainder last.
    fn splits_of(total: usize, size: usize) -> Vec<usize> {
        let mut splits = vec![size; total / size];
        if !total.is_multiple_of(size) {
            splits.push(total % size);
        }
        splits
    }

    /// The same byte offsets are lost however the reads are split: this is what
    /// makes two runs of one profile comparable at all.
    fn losses_for(profile: LinkProfile, splits: &[usize]) -> (u64, u64) {
        let counters = Counters::default();
        let mut schedule = LossSchedule::new(profile, Side::Broker);
        for split in splits {
            schedule.release(*split, &counters);
        }
        let counts = counters.snapshot();
        (counts.lost_packets_broker, schedule.offset)
    }

    #[test]
    fn loss_decisions_do_not_depend_on_read_boundaries() {
        let profile = LinkProfile::lossy(Duration::from_millis(60), 10);
        let total = 512 * 1024;
        let uniform = losses_for(profile, &splits_of(total, 16 * 1024));
        let ragged = losses_for(profile, &splits_of(total, 997));
        assert_eq!(uniform.1 as usize, total);
        assert_eq!(ragged.1 as usize, total);
        assert_eq!(uniform.0, ragged.0);
    }

    #[test]
    fn a_clean_link_never_charges_a_retransmission() {
        let profile = LinkProfile::clean(Duration::from_millis(20));
        let (lost, _) = losses_for(profile, &splits_of(1024 * 1024, 4096));
        assert_eq!(lost, 0);
    }

    #[test]
    fn one_percent_loses_about_three_packets_per_half_megabyte() {
        let profile = LinkProfile::lossy(Duration::from_millis(60), 10);
        // 512 KiB is ~375 MTU-sized packets; 1% of that is ~3.75.
        let (lost, _) = losses_for(profile, &splits_of(512 * 1024, 16 * 1024));
        assert!((1..=10).contains(&lost), "lost {lost} packets in 512 KiB");
    }

    /// A loss stalls the stream instead of deleting bytes, and every byte is
    /// still delivered.
    #[test]
    fn a_lost_packet_stalls_the_bytes_behind_it() {
        let profile = LinkProfile::lossy(Duration::from_millis(60), 10);
        let counters = Counters::default();
        let mut schedule = LossSchedule::new(profile, Side::Broker);
        let mut stall = None;
        let mut delivered = 0u64;
        for _ in 0..400 {
            let before = schedule.next_release;
            let lost_before = counters.snapshot().lost_packets_broker;
            let release = schedule.release(SEGMENT_BYTES as usize, &counters);
            delivered += SEGMENT_BYTES;
            if counters.snapshot().lost_packets_broker > lost_before {
                stall = Some(release - before);
                break;
            }
        }
        let stall = stall.expect("a 1% link loses a packet within 400 packets");
        assert!(
            stall >= profile.retransmit,
            "the stall behind a lost packet is a retransmission: {stall:?}"
        );
        // Nothing was dropped: the schedule consumed exactly the bytes it was
        // given, so every lost packet is only late.
        assert_eq!(schedule.offset, delivered);
    }

    /// Frame lengths, not text: a payload byte that spells `PUB ` is not a publish.
    #[test]
    fn payload_bytes_are_skipped_by_their_declared_length() {
        let counters = Counters::default();
        let mut scan = NatsScan::default();
        let payload = "PUB p.pair.xfer.up.t.pull.3 p.pair.rep.device.a 2\r\n";
        let frame = format!("MSG real.subject 7 {}\r\n{}\r\n", payload.len(), payload);
        // One byte at a time: the parser has to hold a header across reads.
        for byte in frame.as_bytes() {
            scan.feed(std::slice::from_ref(byte), Side::Broker, &counters);
        }
        let counts = counters.snapshot();
        assert_eq!(counts.broker_messages, 1);
        assert_eq!(counts.client_pulls, 0);
        assert_eq!(counts.broker_chunks, 0);
    }

    #[test]
    fn client_pulls_and_broker_chunks_are_counted_by_subject() {
        let counters = Counters::default();
        let mut scan = NatsScan::default();
        scan.feed(
            b"PUB p.pair.xfer.up.download_1.pull.3 p.pair.rep.device.x1 2\r\n{}\r\n",
            Side::Client,
            &counters,
        );
        scan.feed(
            b"MSG p.pair.xfer.down.download_1.chunk.3 4 5\r\nhello\r\n",
            Side::Broker,
            &counters,
        );
        scan.feed(
            b"MSG p.pair.rep.device.x1 2 16\r\n{\"success\":true}",
            Side::Broker,
            &counters,
        );
        let counts = counters.snapshot();
        assert_eq!(counts.client_pulls, 1);
        assert_eq!(counts.client_publish, 1);
        assert_eq!(counts.broker_chunks, 1);
        assert_eq!(counts.pull_acks, 1);
        assert_eq!(counts.control_messages(), 2);
    }
}
