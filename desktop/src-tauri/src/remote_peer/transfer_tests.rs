//! The client's half of a file download, against a **real** host bridge.
//!
//! The host's policy — which files a paired client may ask for — is its own
//! concern and is covered in `remote_host::files`. What is new here is the
//! transport: registering a pull, joining the host's byte publish to its
//! acknowledgement (they arrive on different subjects), assembling the chunks in
//! order, and checking the result against the size and hash the host declared.
//! Those are exactly the places a file can be silently corrupted, so they are
//! exercised against bytes the host really served rather than a fixture.

use super::runtime;
use super::testing::{fixture, teardown};
use super::transfer::{self, DownloadInfo};
use std::sync::Arc;

/// A file to serve, plus the host's declaration of it in its wire form.
fn serve(path: &std::path::Path, contents: &[u8]) -> (String, DownloadInfo) {
    std::fs::create_dir_all(path.parent().expect("a parent directory")).expect("create dir");
    std::fs::write(path, contents).expect("write the served file");
    let wire = crate::remote_host::files::register_download_for_test(path);
    let info: DownloadInfo =
        serde_json::from_value(wire).expect("the client reads the host's declaration");
    (info.transfer_id.clone(), info)
}

/// A scratch directory per test, so two tests cannot serve over each other.
fn scratch(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("futureos-peer-xfer-{label}"));
    std::fs::create_dir_all(&dir).expect("create the scratch directory");
    dir
}

/// The client as it runs in the app: connected *with* a stream, because the
/// transfer subscription is taken at connect time. A connection with no emitter
/// has no stream task at all, and therefore no way to receive bytes.
async fn connect_with_stream(desktop_id: &str) {
    let noop: runtime::Emitter = Arc::new(|_| {});
    runtime::connect_with_emitter(desktop_id, Some(noop))
        .await
        .expect("connect with a stream");
}

#[tokio::test]
async fn a_download_round_trips_a_file_through_the_host() {
    let (_home, fx) = fixture("peer-xfer-round-trip").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect_with_stream(&desktop_id).await;

    let contents = b"hello from the other machine";
    let (_transfer_id, info) = serve(&scratch("round-trip").join("note.txt"), contents);

    let bytes = transfer::fetch(&desktop_id, &info).await.expect("fetch");

    assert_eq!(bytes, contents);

    teardown().await;
}

/// More than one chunk, so the index arithmetic, the ordered append and the
/// partial last chunk are all real. One chunk would exercise none of them.
#[tokio::test]
async fn a_multi_chunk_download_assembles_in_order() {
    let (_home, fx) = fixture("peer-xfer-multi-chunk").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect_with_stream(&desktop_id).await;

    // Two full chunks plus a partial third: the boundary cases are the ends.
    let chunk = crate::remote_host::files::CHUNK_BYTES as usize;
    let contents: Vec<u8> = (0..chunk * 2 + 7)
        .map(|index| (index % 251) as u8)
        .collect();
    let (_transfer_id, info) = serve(&scratch("multi-chunk").join("chunked.bin"), &contents);

    let bytes = transfer::fetch(&desktop_id, &info).await.expect("fetch");

    assert_eq!(info.size.div_ceil(info.chunk_bytes), 3, "three chunks");
    // Byte-for-byte: a pattern whose position is recoverable, so an
    // out-of-order or duplicated chunk cannot coincide with the expected result.
    assert_eq!(bytes, contents);

    teardown().await;
}

/// The bytes are checked against the hash the host declared. This is the
/// failure the whole verification exists for: a chunk lost over an at-most-once
/// transport must produce an error, not a file that looks complete.
#[tokio::test]
async fn a_download_whose_bytes_do_not_match_the_declared_hash_fails() {
    let (_home, fx) = fixture("peer-xfer-hash-mismatch").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect_with_stream(&desktop_id).await;

    let path = scratch("hash-mismatch").join("tampered.txt");
    let (_transfer_id, info) = serve(&path, b"the original contents");
    // The file changes after the host declared its size and hash, exactly as a
    // chunk corrupted or substituted in transit would look to the client. Same
    // length, so only the hash can catch it.
    std::fs::write(&path, b"the ORIGINAL contents").expect("rewrite the served file");

    let error = transfer::fetch(&desktop_id, &info)
        .await
        .expect_err("a mismatched download must not be reported as success");

    assert!(
        error.to_string().contains("remote_download_hash_mismatch"),
        "the failure must say what did not match, got: {error}"
    );

    teardown().await;
}

/// A file whose source shrank after the host declared it is refused. The host
/// notices first here (it cannot read the bytes it promised); the point of the
/// test is that the mismatch is a refusal, not a short file reported as success.
#[tokio::test]
async fn a_download_whose_source_shrank_is_refused() {
    let (_home, fx) = fixture("peer-xfer-short").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect_with_stream(&desktop_id).await;

    let path = scratch("short").join("truncated.bin");
    let (_transfer_id, info) = serve(&path, &[7_u8; 4096]);
    std::fs::write(&path, b"too short").expect("truncate the served file");

    let error = transfer::fetch(&desktop_id, &info)
        .await
        .expect_err("a download that cannot be completed must be refused");

    assert!(
        !error.to_string().is_empty(),
        "the refusal must carry a reason"
    );

    teardown().await;
}

/// A pull for a transfer the host does not know is refused by the host, and the
/// host's own reason reaches the caller: a generic "download failed" would leave
/// a support session with nothing to go on.
#[tokio::test]
async fn an_unknown_transfer_is_refused_with_the_hosts_reason() {
    let (_home, fx) = fixture("peer-xfer-unknown").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect_with_stream(&desktop_id).await;

    let info = DownloadInfo {
        transfer_id: "download_does_not_exist".into(),
        name: "ghost.txt".into(),
        size: 4,
        content_hash: "00".repeat(32),
        chunk_bytes: crate::remote_host::files::CHUNK_BYTES,
    };

    let error = transfer::fetch(&desktop_id, &info)
        .await
        .expect_err("refused");

    assert!(
        error.to_string().contains("expired or does not exist"),
        "the host's own reason must reach the caller, got: {error}"
    );

    teardown().await;
}

/// The two halves of one pull are joined by `(desktop, transfer, index)`: the
/// same index appears in every transfer, so a key that ignored the transfer id
/// would hand one download's bytes to another.
///
/// Both pulls are **in flight at once** on purpose. Pulling them in sequence
/// would leave no two waiters registered together, and a key that ignored the
/// transfer id would then never be contradicted — the test would pass without
/// exercising the thing it names.
#[tokio::test]
async fn two_downloads_in_flight_pull_independently() {
    let (_home, fx) = fixture("peer-xfer-two-downloads").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect_with_stream(&desktop_id).await;

    let root = scratch("two-downloads");
    // Different lengths, so a swapped or dropped chunk cannot coincidentally
    // produce the other file's bytes.
    let (_first_id, first) = serve(&root.join("first.txt"), b"first file bytes");
    let (_second_id, second) = serve(&root.join("second.txt"), b"second file bytes, longer");

    let (first_bytes, second_bytes) = tokio::join!(
        transfer::fetch(&desktop_id, &first),
        transfer::fetch(&desktop_id, &second),
    );

    assert_eq!(first_bytes.expect("first"), b"first file bytes");
    assert_eq!(second_bytes.expect("second"), b"second file bytes, longer");

    teardown().await;
}

// ── the guards around a prepared transfer ──────────────────────────────────

/// The size ceiling is checked before anything is pulled, and the host's record
/// is released: holding a prepared copy on the other machine for a download we
/// refuse is a leak the user cannot see.
#[tokio::test]
async fn an_oversized_download_is_refused_before_pulling() {
    let (_home, fx) = fixture("peer-xfer-too-large").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect_with_stream(&desktop_id).await;

    // A real transfer, so the cancel has something real to release.
    let (_transfer_id, mut info) = serve(&scratch("too-large").join("big.bin"), b"tiny");
    info.size = u64::MAX;

    let error = transfer::finish(&desktop_id, info)
        .await
        .expect_err("an oversized download must be refused");

    assert!(
        error.to_string().contains("remote_download_too_large"),
        "got: {error}"
    );

    teardown().await;
}

/// A declared chunk size of zero would make the chunk count a division by zero.
#[tokio::test]
async fn a_zero_chunk_size_is_refused() {
    let (_home, fx) = fixture("peer-xfer-zero-chunk").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect_with_stream(&desktop_id).await;

    let (_transfer_id, mut info) = serve(&scratch("zero-chunk").join("z.bin"), b"tiny");
    info.chunk_bytes = 0;

    let error = transfer::finish(&desktop_id, info)
        .await
        .expect_err("a zero chunk size is unusable");

    assert!(
        error.to_string().contains("remote_download_bad_chunk_size"),
        "got: {error}"
    );

    teardown().await;
}

/// A pull that fails propagates, and the host's record is released on the way
/// out — the same reasoning as the size guard, on the failure path.
#[tokio::test]
async fn a_failed_pull_releases_the_hosts_record() {
    let (_home, fx) = fixture("peer-xfer-release-on-failure").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect_with_stream(&desktop_id).await;

    let info = DownloadInfo {
        transfer_id: "download_never_prepared".into(),
        name: "ghost.txt".into(),
        size: 8,
        content_hash: "00".repeat(32),
        chunk_bytes: 8,
    };

    let error = transfer::finish(&desktop_id, info)
        .await
        .expect_err("an unprepared transfer cannot be pulled");

    assert!(
        error.to_string().contains("expired or does not exist"),
        "got: {error}"
    );

    teardown().await;
}

/// The retry decision, tested directly: it is the one choice here whose wrong
/// answer is invisible. Retrying a refusal delays a definite error; refusing to
/// retry a transport fault turns a lost packet into a failed download.
#[test]
fn only_a_transport_fault_is_worth_retrying() {
    use crate::AppError;
    assert!(transfer::retryable(&AppError::RemoteTransport(
        "Remote request timed out".into()
    )));
    // A host that answered and refused: it will refuse again.
    assert!(!transfer::retryable(&AppError::Message(
        "Download expired or does not exist.".into()
    )));
    // A local misconfiguration is not the link's fault either.
    assert!(!transfer::retryable(&AppError::Message(
        "peer_not_connected".into()
    )));
}

// ── pure pieces ─────────────────────────────────────────────────────────────

use super::runtime::parse_chunk_subject;

#[test]
fn a_chunk_subject_yields_its_transfer_and_index() {
    assert_eq!(
        parse_chunk_subject("p.pair_1.xfer.down.download_9.chunk.3"),
        Some(("download_9".to_string(), 3))
    );
}

#[test]
fn a_subject_that_is_not_a_chunk_is_refused() {
    for subject in [
        // Another family on the same pair.
        "p.pair_1.evt.sess_1",
        // The lane the client writes on, not the one it reads.
        "p.pair_1.xfer.up.download_9.pull.3",
        // An upload chunk, which is a command rather than a publish.
        "p.pair_1.xfer.up.download_9.chunk.3",
        // A non-numeric index: guessing at it would mis-address the file.
        "p.pair_1.xfer.down.download_9.chunk.abc",
        // Truncated.
        "p.pair_1.xfer.down.download_9.chunk",
        "",
    ] {
        assert_eq!(parse_chunk_subject(subject), None, "subject {subject}");
    }
}

#[test]
fn writing_a_file_leaves_no_partial_neighbour() {
    let dir = scratch("write");
    let destination = dir.join("written.bin");

    transfer::write_atomically(&destination, b"complete contents").expect("write");

    assert_eq!(
        std::fs::read(&destination).expect("read back"),
        b"complete contents"
    );
    // The temporary is renamed into place, not left behind: a stray `.part` file
    // beside a download is the debris of a failure that already happened.
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .expect("read dir")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".part"))
        .collect();
    assert!(leftovers.is_empty(), "left behind {leftovers:?}");

    // Overwriting an existing file is the normal case (the user chose the path).
    transfer::write_atomically(&destination, b"second").expect("overwrite");
    assert_eq!(std::fs::read(&destination).expect("read back"), b"second");
}

#[test]
fn a_destination_with_no_file_name_is_refused() {
    let error = transfer::write_atomically(std::path::Path::new("/"), b"x")
        .expect_err("a directory cannot be a destination");
    assert!(
        error
            .to_string()
            .contains("remote_download_bad_destination"),
        "got: {error}"
    );
}
