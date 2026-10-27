//! What one picture costs on the remote link, segment by segment.
//!
//! The question is "how long until the picture is on screen", and the honest
//! answer has three measurable pieces plus parts this machine cannot see:
//!
//! * **host-side preview encode** — `host/src/remote_host/files.rs` decodes the
//!   attachment, resizes it to 1600 px with Lanczos3 when it is bigger, and
//!   re-encodes it (JPEG q65 or PNG). The probe below repeats exactly that
//!   sequence in `--release`; it is a copy of the production steps in a test, not
//!   a call into them, because this crate depends on the remote protocol crates
//!   and not on `future-host`.
//! * **transfer** — the real client, the real host, real 512-KiB chunking, real
//!   seal, real Core NATS, with `support/weak_link.rs` injecting delay and loss
//!   on the client's leg only. This is the part that scales with the network.
//! * **client-side decode** — the reader has to turn the received bytes back into
//!   pixels; measured here with the same `image` crate version the host uses.
//!
//! Not measured here, and named rather than guessed: WebView/GPU upload, frame
//! composition, display scan-out, and the desktop's own event-publish window
//! between the bytes landing and the timeline row being painted. Those need a
//! running UI and a camera.
//!
//! The payloads are the *encoded preview bytes this run actually produced*, not
//! a round number, so the transfer arms carry the size the encode arms decided.
use future_remote_interop::{Broker, Fixture, MemoryClientHost, DESKTOP};
use image::{ImageEncoder, ImageFormat};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use weak_link::{LinkProfile, WeakLink};

#[path = "support/weak_link.rs"]
mod weak_link;

/// The desktop's download chunk size (`remote_host::files::CHUNK_BYTES`).
const CHUNK_BYTES: usize = 512 * 1024;
/// The host's phone-preview edge cap (`MOBILE_PREVIEW_MAX_EDGE`).
const PREVIEW_MAX_EDGE: u32 = 1600;
/// The host's JPEG preview quality (`JpegEncoder::new_with_quality(.., 65)`).
const PREVIEW_JPEG_QUALITY: u8 = 65;

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// Nearest-rank percentile on a sorted slice.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = (p / 100.0 * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

fn spread(label: &str, mut runs: Vec<f64>) -> serde_json::Value {
    runs.sort_by(f64::total_cmp);
    json!({
        "label": label,
        "reps": runs.len(),
        "min": runs.first().copied().unwrap_or(f64::NAN),
        "median": percentile(&runs, 50.0),
        "p95": percentile(&runs, 95.0),
        "p99": percentile(&runs, 99.0),
        "max": runs.last().copied().unwrap_or(f64::NAN),
        "runs": runs,
    })
}

/// A 3024×1964 "screenshot": smooth gradients (a photo-ish backdrop), flat UI
/// panels, glyph-like marks and a little noise. Compressibility matters — pure
/// noise would inflate both the encode time and the byte count of every arm.
fn screenshot() -> image::RgbImage {
    let (w, h) = (3024_u32, 1964_u32);
    let mut image = image::RgbImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let f = (x * 7 + y * 3) % 97;
            let mut pixel = [
                (40 + y * 90 / h) as u8,
                (60 + x * 40 / w) as u8,
                (110 - (f / 3) % 40) as u8,
            ];
            // Panels and rules: large flat areas, the cheap case for JPEG.
            if (x / 240) % 3 == 0 && (y / 180) % 4 == 0 {
                pixel = [236, 238, 242];
            }
            if y % 96 < 3 {
                pixel = [90, 94, 104];
            }
            // Glyph-like marks so the encoder cannot coast on flat blocks.
            if (x / 6) % 17 == 0 && (y / 14) % 9 == 0 {
                pixel = [26, 28, 32];
            }
            let noise = ((x * 131 + y * 71) % 13) as i16 - 6;
            for channel in pixel.iter_mut() {
                *channel = (*channel as i16 + noise).clamp(0, 255) as u8;
            }
            image.put_pixel(x, y, image::Rgb(pixel));
        }
    }
    image
}

/// A 1600-wide JPEG, the shape a phone uploads (mobile downsamples to 1600 px
/// before sending), so the preview path decodes and re-encodes without resizing.
fn photo_jpeg() -> Vec<u8> {
    let mut base = screenshot();
    base = image::imageops::resize(
        &base,
        PREVIEW_MAX_EDGE,
        PREVIEW_MAX_EDGE * 1964 / 3024,
        image::imageops::FilterType::Triangle,
    );
    let mut bytes = Vec::new();
    let encoder =
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, PREVIEW_JPEG_QUALITY);
    encoder
        .write_image(
            base.as_raw(),
            base.width(),
            base.height(),
            image::ExtendedColorType::Rgb8,
        )
        .expect("encode source photo");
    bytes
}

/// The host's preview sequence (`prepare_preview_image` in `remote_host/files.rs`):
/// decode → resize to 1600 px with Lanczos3 when needed → JPEG q65 or PNG.
fn preview_encode(source: &[u8], png: bool) -> (Vec<u8>, Duration, Duration) {
    let decode_started = Instant::now();
    let mut reader = image::ImageReader::new(std::io::Cursor::new(source))
        .with_guessed_format()
        .expect("guess source format");
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().expect("decode source");
    let decode_ms = decode_started.elapsed();

    let encode_started = Instant::now();
    let resized = if decoded.width().max(decoded.height()) > PREVIEW_MAX_EDGE {
        decoded.resize(
            PREVIEW_MAX_EDGE,
            PREVIEW_MAX_EDGE,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        decoded
    };
    let mut out = Vec::new();
    if png {
        resized
            .write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Png)
            .expect("PNG preview encode");
    } else {
        let encoder =
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, PREVIEW_JPEG_QUALITY);
        encoder
            .write_image(
                resized.to_rgb8().as_raw(),
                resized.width(),
                resized.height(),
                image::ExtendedColorType::Rgb8,
            )
            .expect("JPEG preview encode");
    }
    (out, decode_ms, encode_started.elapsed())
}

/// Decode received preview bytes the way a reader has to before it can draw them.
fn client_decode(bytes: &[u8]) -> Result<Duration, image::ImageError> {
    let started = Instant::now();
    let decoded = image::load_from_memory(bytes)?;
    std::hint::black_box(decoded);
    Ok(started.elapsed())
}

/// One transfer cell: real broker, real client, real host, weak client leg.
async fn download_cell(
    label: &str,
    bytes: &[u8],
    profile: LinkProfile,
    reps: usize,
) -> anyhow::Result<serde_json::Value> {
    let broker = Broker::start().await?;
    let upstream: SocketAddr = broker
        .nats_url
        .strip_prefix("nats://")
        .expect("fixture broker url is nats://")
        .parse()?;
    let link = WeakLink::start(upstream, profile).await?;
    let fixture = Fixture::start_transfer_routed(
        &broker.nats_url,
        &link.url(),
        &broker.websocket_url,
        bytes.to_vec(),
        CHUNK_BYTES,
        Duration::ZERO,
    )
    .await?;
    let host = Arc::new(MemoryClientHost::new(fixture.client_credentials.clone()));
    let client = future_remote_client::Client::new(host);
    client
        .connect_with_emitter(DESKTOP, Some(Arc::new(|_| {})))
        .await?;

    let chunks = bytes.len().div_ceil(CHUNK_BYTES) as u64;
    let mut runs = Vec::with_capacity(reps);
    let mut wire = Vec::with_capacity(reps);
    for _ in 0..reps {
        let before = link.counters();
        let started = Instant::now();
        let downloaded = client
            .download(DESKTOP, "interop_session", "fixture.bin", None, "original")
            .await?;
        runs.push(ms(started.elapsed()));
        assert_eq!(
            downloaded.bytes, bytes,
            "the weak link must not change the bytes"
        );
        let counts = link.counters().since(before);
        assert_eq!(counts.client_pulls, chunks, "one pull per chunk");
        assert_eq!(
            counts.snapshot_json()["chunkMessages"],
            json!(chunks),
            "one chunk message per chunk"
        );
        wire.push(counts.wire_bytes());
    }
    client.shutdown(Duration::from_secs(5)).await?;
    fixture.shutdown().await?;

    let mut cell = spread(label, runs);
    cell["wireBytes"] = json!(wire[0]);
    cell["oneWayMs"] = json!(ms(profile.one_way));
    cell["rttMs"] = json!(ms(profile.rtt()));
    cell["lossPerMille"] = json!(profile.loss_per_mille);
    cell["bytes"] = json!(bytes.len());
    cell["chunks"] = json!(chunks);
    Ok(cell)
}

/// Encode arms plus the decode of their own output. Deterministic CPU work; the
/// only thing that varies is the machine.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "measured benchmark; builds two 6-megapixel sources in release"]
async fn image_segments_encode_and_decode() {
    let source = screenshot();
    let mut png_source = Vec::new();
    source
        .write_to(&mut std::io::Cursor::new(&mut png_source), ImageFormat::Png)
        .expect("encode screenshot source");
    let photo = photo_jpeg();

    for (label, input, png) in [
        (
            "screenshot_png_3024->preview_jpeg",
            png_source.as_slice(),
            false,
        ),
        (
            "screenshot_png_3024->preview_png",
            png_source.as_slice(),
            true,
        ),
        ("photo_jpeg_1600->preview_jpeg", photo.as_slice(), false),
    ] {
        let mut encodes = Vec::new();
        let mut decodes = Vec::new();
        let mut decode_source = Vec::new();
        let mut encoded_bytes = 0;
        for _ in 0..5 {
            let (out, decode_ms, encode_ms) = preview_encode(input, png);
            decode_source.push(ms(decode_ms));
            encodes.push(ms(encode_ms));
            encoded_bytes = out.len();
            let mut client_runs = Vec::new();
            for _ in 0..5 {
                client_runs.push(ms(client_decode(&out).expect("client decode")));
            }
            decodes.push(client_runs);
        }
        let client: Vec<f64> = decodes.iter().map(|runs| runs[runs.len() / 2]).collect();
        println!(
            "IMAGE {}",
            json!({
                "segment": "host_preview_encode",
                "arm": label,
                "sourceBytes": input.len(),
                "previewBytes": encoded_bytes,
                "sourceDecodeMs": spread("source_decode", decode_source),
                "resizeAndEncodeMs": spread("resize_encode", encodes),
                "clientDecodeMs": spread("client_decode", client),
            })
        );
    }
}

/// The transfer arms. Payloads are the real preview outputs, so the byte counts
/// here are the ones the encoder above produced.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "measured benchmark; real sockets, deliberate latency and loss"]
async fn image_segments_transfer() -> anyhow::Result<()> {
    let source = screenshot();
    let mut png_source = Vec::new();
    source
        .write_to(&mut std::io::Cursor::new(&mut png_source), ImageFormat::Png)
        .expect("encode screenshot source");
    let (jpeg_preview, _, _) = preview_encode(&png_source, false);
    let (png_preview, _, _) = preview_encode(&png_source, true);

    let arms: [(&str, &[u8]); 2] = [
        ("preview_jpeg", jpeg_preview.as_slice()),
        ("preview_png", png_preview.as_slice()),
    ];
    for (label, bytes) in arms {
        for rtt in [2_u64, 40, 120] {
            let profile = LinkProfile::clean(Duration::from_millis(rtt / 2));
            let cell = download_cell(label, bytes, profile, 20).await?;
            println!(
                "IMAGE {}",
                json!({"segment": "transfer", "link": "clean", "cell": cell})
            );
        }
        // Lossy: the tail is the point of this arm, so more reps.
        let profile = LinkProfile::lossy(Duration::from_millis(20), 10);
        let cell = download_cell(label, bytes, profile, 40).await?;
        println!(
            "IMAGE {}",
            json!({"segment": "transfer", "link": "lossy_1pct_40ms_rtt", "cell": cell})
        );
    }
    // One long arm, so a p99 is a percentile rather than the second-worst of 20.
    for (label, bytes) in arms {
        let profile = LinkProfile::clean(Duration::from_millis(20));
        let cell = download_cell(label, bytes, profile, 100).await?;
        println!(
            "IMAGE {}",
            json!({"segment": "transfer", "link": "clean_40ms_rtt_n100", "cell": cell})
        );
    }
    Ok(())
}
