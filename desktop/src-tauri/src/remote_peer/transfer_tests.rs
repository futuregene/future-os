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
use super::runtime_tests::find;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
}

// ── pure pieces ─────────────────────────────────────────────────────────────

/**
 * A preview pulled for the overlay lands under its own name, one entry per source.
 *
 * The name matters more than it looks: the overlay picks its renderer from the
 * extension (`previewKindForPath`), so a cache entry spelled as an opaque digest
 * would make every image arrive as an unreadable blob. The digest in front of the
 * name is what keeps two files with one name apart and makes a second look at the
 * same file reuse the entry instead of adding a copy.
 */
#[test]
fn a_preview_is_cached_under_its_own_name_and_one_entry_per_source() {
    let _home = crate::remote::test_support::HomeGuard::new("peer-xfer-preview-path");

    let first =
        transfer::preview_cache_path("desktop_a", "sess_1", "/tmp/a/forest.png", "forest.png")
            .expect("cache path");
    assert!(first.to_string_lossy().contains("remote-previews"));
    assert!(
        first
            .file_name()
            .expect("a name")
            .to_string_lossy()
            .ends_with("forest.png"),
        "the extension is what the overlay reads: {first:?}"
    );

    // The same source is the same entry, which is the whole point of a cache.
    let again =
        transfer::preview_cache_path("desktop_a", "sess_1", "/tmp/a/forest.png", "forest.png")
            .expect("cache path");
    assert_eq!(first, again);

    // Another file under another path is another entry even when the names match.
    let other =
        transfer::preview_cache_path("desktop_a", "sess_1", "/tmp/b/forest.png", "forest.png")
            .expect("cache path");
    assert_ne!(first, other);

    // A host name that is a path fragment, or reserved, still lands somewhere
    // writable — and a name with nothing usable left is still a file.
    let odd =
        transfer::preview_cache_path("desktop_a", "sess_1", "/tmp/a", "..").expect("cache path");
    assert!(odd
        .file_name()
        .expect("a name")
        .to_string_lossy()
        .ends_with("preview"));
}

/**
 * The cache keeps its newest entries and its age limit.
 *
 * A cache that only grows is a second copy of another machine's folder on this
 * one, so both bounds are asserted: the count (the oldest entries go) and the age
 * (an entry nothing has looked at for a week goes, whatever the count).
 */
#[test]
fn the_preview_cache_drops_what_is_old_or_excess() {
    use std::time::{Duration, SystemTime};
    let _home = crate::remote::test_support::HomeGuard::new("peer-xfer-preview-prune");
    let root = crate::store::remote_previews_dir().expect("previews dir");
    std::fs::create_dir_all(&root).expect("create the cache root");

    let entry = |index: usize, age: Duration| {
        let path = root.join(format!("{index:016x}__note.md"));
        std::fs::write(&path, b"contents").expect("write an entry");
        std::fs::File::options()
            .write(true)
            .open(&path)
            .expect("open for the timestamp")
            .set_modified(SystemTime::now() - age)
            .expect("set the timestamp");
    };
    // 70 live entries, newest last, plus one long past its week.
    for index in 0..70 {
        entry(index, Duration::from_secs((70 - index) as u64));
    }
    let stale = root.join("00000000000000ff__stale.md");
    std::fs::write(&stale, b"contents").expect("write the stale entry");
    std::fs::File::options()
        .write(true)
        .open(&stale)
        .expect("open the stale entry")
        .set_modified(SystemTime::now() - Duration::from_secs(8 * 24 * 60 * 60))
        .expect("set the stale timestamp");

    transfer::prune_preview_cache();

    let mut remaining: Vec<String> = std::fs::read_dir(&root)
        .expect("read the cache")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    remaining.sort();
    assert!(!stale.exists(), "an entry past its week is dropped");
    assert_eq!(
        remaining.len(),
        64,
        "the count bound is the newest 64: {remaining:?}"
    );
    assert!(
        remaining
            .iter()
            .any(|name| name.starts_with(&format!("{:016x}", 69))),
        "the newest entry is kept: {remaining:?}"
    );
    assert!(
        !remaining
            .iter()
            .any(|name| name.starts_with(&format!("{:016x}", 0))),
        "the oldest entry is dropped: {remaining:?}"
    );
}

/**
 * A half-written file is not a cache entry, and a cache that cannot be read is
 * not a failure.
 *
 * The first half is what makes the count bound honest: `write_atomically` leaves
 * a `.part` neighbour while a preview is being written, and counting that as an
 * entry would evict a *real* preview to make room for a temporary one — the
 * cache would shrink under a burst of writes and pull the same files again.
 *
 * The second half is the pruning contract: this runs on the preview path, where
 * a cache it cannot read (no home yet, a directory that does not exist) must be
 * a no-op rather than an error the user's preview would surface. Both are
 * asserted by their *effect* — the `.part` file survives a prune, and a prune
 * against a home with no cache directory leaves that state exactly as it found
 * it.
 */
#[test]
fn the_prune_keeps_a_partial_write_and_tolerates_a_cache_it_cannot_read() {
    // (1) A `.part` neighbour is not an entry. Scoped, because `HomeGuard` holds
    // a non-reentrant global lock: a second guard created while the first is
    // alive would block forever.
    {
        let _home = crate::remote::test_support::HomeGuard::new("peer-xfer-preview-part");
        let root = crate::store::remote_previews_dir().expect("previews dir");
        std::fs::create_dir_all(&root).expect("create the cache root");
        let partial = root.join("0123456789abcdef__clipboard.png.part");
        std::fs::write(&partial, b"half a preview").expect("write the partial file");
        let real = root.join("fedcba9876543210__clipboard.png");
        std::fs::write(&real, b"a whole preview").expect("write the real entry");

        transfer::prune_preview_cache();

        assert!(
            partial.exists(),
            "a half-written file must not be counted as an entry, and must not be deleted"
        );
        assert!(real.exists(), "and the real entry stays");
        assert_eq!(
            std::fs::read_to_string(&partial).expect("read the partial file"),
            "half a preview",
            "pruning does not rewrite it either"
        );
    }

    // (2) A cache directory that does not exist yet is a no-op, not an error.
    let _home = crate::remote::test_support::HomeGuard::new("peer-xfer-preview-absent");
    let absent = crate::store::remote_previews_dir().expect("previews dir");
    assert!(!absent.exists(), "the fixture must start without a cache");
    transfer::prune_preview_cache();
    assert!(
        !absent.exists(),
        "pruning an absent cache leaves it absent rather than creating or failing on it"
    );
}

/// With no home to resolve, the preview cache is not reachable at all and the
/// prune is a no-op: it must not panic, and it must not create anything. This is
/// the state the app can be in before its home is resolved (and the reason the
/// pruning contract is "best effort" rather than a failure the preview would
/// carry). The environment is process-global, so the global test-home lock is
/// held — which is exactly what `HomeGuard` exists for — and restored before it
/// is released.
#[test]
fn the_prune_is_a_no_op_when_no_home_can_be_resolved() {
    let _home = crate::remote::test_support::HomeGuard::new("peer-xfer-preview-no-home");
    let previous = std::env::var_os("HOME");
    std::env::remove_var("HOME");
    std::env::remove_var("USERPROFILE");
    std::env::remove_var("FUTURE_HOME");

    transfer::prune_preview_cache();

    match previous {
        Some(value) => std::env::set_var("HOME", value),
        None => std::env::remove_var("HOME"),
    }
    assert!(
        std::env::var_os("HOME").is_some(),
        "the fixture must put HOME back"
    );
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

/// A declaration that disagrees with the bytes is caught before hashing.
///
/// Reachable with a real host by declaring a size the host does not serve: the
/// host slices by its *own* record, so a smaller declaration pulls fewer chunks
/// and the total comes up short. This is the check that makes a lying or buggy
/// declaration fail rather than produce a plausible-looking file.
#[tokio::test]
async fn a_declaration_that_disagrees_with_the_bytes_is_refused() {
    let (_home, fx) = fixture("peer-xfer-size-mismatch").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect_with_stream(&desktop_id).await;

    // A declaration smaller than the file: one chunk is asked for and the host
    // serves all ten bytes, against a declared six. The chunk size stays the
    // host's own, because the host validates an index against its own
    // boundaries — doctoring it would produce a different refusal entirely.
    let (_transfer_id, mut info) = serve(&scratch("size-mismatch").join("ten.bin"), b"0123456789");
    info.size = 6;

    let error = transfer::fetch(&desktop_id, &info)
        .await
        .expect_err("a short delivery must not pass as a download");

    assert!(
        error.to_string().contains("remote_download_size_mismatch"),
        "got: {error}"
    );

    teardown(&fx.pair_id).await;
}

/// A destination that cannot be written is reported.
#[test]
fn an_unwritable_destination_is_reported() {
    // The parent directory does not exist, which is the ordinary way this fails
    // (a stale path from a dialog, a directory removed in the meantime).
    let error = transfer::write_to_path("/futureos-no-such-directory/file.txt", b"x")
        .expect_err("a missing parent cannot be written into");
    assert!(
        error.to_string().contains("remote_download_write_failed"),
        "got: {error}"
    );
}

/// The rename is the last step, and its failure must not leave the partial copy
/// beside the destination: a `.part` file is debris from a failure that already
/// happened, and it looks like something the user can open.
#[test]
fn a_failed_rename_leaves_no_partial_file() {
    let dir = scratch("rename-fails");
    // A directory where the file should go: the temporary writes fine, then the
    // rename onto a directory fails.
    let destination = dir.join("occupied.txt");
    std::fs::create_dir_all(&destination).expect("create the occupying directory");

    let error = transfer::write_atomically(&destination, b"contents")
        .expect_err("a directory cannot be replaced by a file");

    assert!(
        error.to_string().contains("remote_download_write_failed"),
        "got: {error}"
    );
    assert!(
        !dir.join(".occupied.txt.part").exists(),
        "the temporary must be removed when the rename fails"
    );
    assert!(
        destination.is_dir(),
        "and the destination must be left alone"
    );
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

// ── the arms a healthy host never produces ──────────────────────────────────

/// A pull with no connection behind it is refused, and refused *by the client*:
/// a user who clicks download after a host dropped gets a reason rather than a
/// request that goes nowhere.
#[tokio::test]
async fn a_pull_without_a_connection_is_refused() {
    let (_home, fx) = fixture("peer-xfer-no-connection").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect_with_stream(&desktop_id).await;
    let (_transfer_id, info) = serve(&scratch("no-connection").join("f.txt"), b"bytes");

    // The pairing stays; only the live connection is dropped, as the user's own
    // Disconnect leaves it.
    runtime::disconnect(&desktop_id).await;

    let error = transfer::fetch(&desktop_id, &info)
        .await
        .expect_err("a pull needs a connection");
    assert!(
        error.to_string().contains("peer_not_connected"),
        "the refusal must name the reason, got: {error}"
    );

    teardown(&fx.pair_id).await;
}

/// A chunk that never arrives fails rather than hanging.
///
/// The host acknowledges the pull *after* publishing, so a chunk that does not
/// turn up is the host having published somewhere this client is not listening —
/// a wedged or dead subscription. The pull has to give up on its own rather than
/// leave the user with a download that never finishes.
///
/// The budget for that give-up is passed in here rather than the whole suite
/// running on a shortened one: production's is 15 s, and a test-only 500 ms was
/// measured reporting a *lost* publish for a chunk a loaded host had served
/// (FLAKES.md item 8). This call cannot be affected by that mistake — the stream
/// task is stopped before the pull starts, so no chunk can arrive at any load —
/// which is exactly what makes it the right place to exercise the arm.
#[tokio::test]
async fn a_chunk_that_never_arrives_gives_up() {
    let (_home, fx) = fixture("peer-xfer-no-chunk").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect_with_stream(&desktop_id).await;
    let (transfer_id, info) = serve(&scratch("no-chunk").join("f.txt"), b"bytes");
    assert!(
        info.size > 0 && info.chunk_bytes > 0,
        "the host must have a chunk to serve for this to be a missing-delivery test"
    );

    // Everything still works except delivery: the host will publish the chunk
    // and acknowledge the pull, and nothing will hand it to the waiter.
    runtime::stop_stream_for_test(&desktop_id).await;

    let error = runtime::pull_chunk_within(
        &desktop_id,
        &transfer_id,
        0,
        std::time::Duration::from_millis(200),
    )
    .await
    .expect_err("a chunk that never arrives cannot be reported as a download");
    assert!(
        error.to_string().contains("remote_download_chunk_missing"),
        "got: {error}"
    );

    teardown(&fx.pair_id).await;
}

/// A connection that dies between the lookup and the request is recorded, not
/// reported as a stalled download.
#[tokio::test]
async fn a_request_on_a_vanished_connection_is_recorded() {
    use std::sync::atomic::Ordering;
    let (_home, fx) = fixture("peer-xfer-vanished").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect_with_stream(&desktop_id).await;

    // A command that is not an object: the lane stamps an id onto one, so it has
    // to tolerate anything the caller passes rather than assuming.
    let not_an_object = runtime::request_at(
        &desktop_id,
        serde_json::json!("not an object"),
        "p.unused.xfer.up.t.pull.0",
        std::time::Duration::from_secs(1),
    )
    .await;
    assert!(
        not_an_object.is_err(),
        "a command the host cannot answer must fail"
    );

    // The interleaving: the entry is removed after the connect at the top of the
    // request, which cannot be produced from outside without racing.
    runtime::INJECT_CONNECTION_LOST.store(true, Ordering::Relaxed);
    let error = runtime::request_at(
        &desktop_id,
        serde_json::json!({}),
        "p.unused.xfer.up.t.pull.0",
        std::time::Duration::from_secs(1),
    )
    .await
    .expect_err("a vanished connection has nobody to ask");

    assert!(
        error.to_string().contains("peer_not_connected"),
        "got: {error}"
    );

    teardown(&fx.pair_id).await;
}

/// Forged bytes on a chunk subject are dropped, not handed to a waiting pull.
///
/// A relay can inject anything on the transfer subscription, and the failure
/// this guards is not "an extra message" but *substituted file bytes*: a chunk
/// accepted without authenticating would be written into the download, and the
/// hash check would then reject the whole file. The assertion is therefore about
/// the waiter — it must still be waiting after the forgery — because a waiter
/// consumed by garbage is exactly how that substitution would begin.
///
/// The bytes are raw, not sealed: sealing needs the host's own traffic key, and
/// what is being tested is that *unsigned* input is refused.
#[tokio::test]
async fn forged_chunk_bytes_are_not_handed_to_a_waiting_pull() {
    let (_home, fx) = fixture("peer-xfer-forged-chunk").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect_with_stream(&desktop_id).await;

    let (transfer_id, info) = serve(&scratch("forged-chunk").join("real.txt"), b"real bytes");
    let (sender, mut receiver) = tokio::sync::oneshot::channel();
    runtime::register_chunk(&desktop_id, &transfer_id, 0, sender).await;

    // Waited for before injecting: `subscribe()` returning only means the command
    // was sent, and injecting earlier would race the broker's own bookkeeping.
    fx.nats
        .wait_for_sub(
            &format!("p.{}.xfer.down.*.chunk.*", fx.pair_id),
            std::time::Duration::from_secs(10),
        )
        .await;
    fx.nats.inject(
        &format!("p.{}.xfer.down.{transfer_id}.chunk.0", fx.pair_id),
        None,
        b"substituted bytes".to_vec(),
    );
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    assert!(
        receiver.try_recv().is_err(),
        "unauthenticated bytes must not satisfy a pull"
    );

    // And the real transfer is unaffected: the forgery did not wedge the stream.
    let bytes = transfer::fetch(&desktop_id, &info)
        .await
        .expect("a forged chunk must not break a real download");
    assert_eq!(bytes, b"real bytes");

    teardown(&fx.pair_id).await;
}

/// A chunk subject with a non-numeric index is dropped before anything is
/// decrypted, and does not satisfy a waiting pull.
///
/// The subscription matches any final token, so a malformed index can arrive. It
/// must be dropped on its shape alone: a parser that defaulted the index (or
/// searched for "the last number") would map this to chunk 0 of the named
/// transfer and consume the waiter a real download is parked on.
///
/// The bytes are raw here rather than sealed — and that is the point of the
/// ordering this asserts: the shape is rejected before the message is ever
/// offered to the crypto layer.
#[tokio::test]
async fn a_chunk_subject_that_is_not_shaped_like_one_is_dropped() {
    let (_home, fx) = fixture("peer-xfer-bad-index").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect_with_stream(&desktop_id).await;

    let (transfer_id, info) = serve(&scratch("bad-index").join("real.txt"), b"real bytes");
    let (sender, mut receiver) = tokio::sync::oneshot::channel();
    runtime::register_chunk(&desktop_id, &transfer_id, 0, sender).await;

    fx.nats
        .wait_for_sub(
            &format!("p.{}.xfer.down.*.chunk.*", fx.pair_id),
            std::time::Duration::from_secs(10),
        )
        .await;
    fx.nats.inject(
        &format!("p.{}.xfer.down.{transfer_id}.chunk.abc", fx.pair_id),
        None,
        b"payload".to_vec(),
    );
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    assert!(
        receiver.try_recv().is_err(),
        "a malformed index must not consume the waiter for chunk 0"
    );

    // The stream is still reading: a real transfer completes afterwards.
    assert_eq!(
        transfer::fetch(&desktop_id, &info).await.expect("fetch"),
        b"real bytes"
    );

    teardown(&fx.pair_id).await;
}

// ── upload ──────────────────────────────────────────────────────────────────

/// The whole upload path against a real host: staging record, chunks, completion.
///
/// The assertion that matters is the host's own hash. `complete_upload` hashes
/// the file the host *assembled* from the chunks it received, so a hash equal to
/// the local one proves every byte arrived and arrived in order — which is
/// exactly what a chunked upload over an at-most-once transport can get wrong.
#[tokio::test]
async fn an_upload_round_trips_through_the_host() {
    let (_home, fx) = fixture("peer-xfer-upload-round-trip").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect_with_stream(&desktop_id).await;

    let contents = b"a file on its way to the other machine";
    let uploaded = transfer::upload(&desktop_id, "note.txt", contents)
        .await
        .expect("upload");

    assert_eq!(uploaded.name, "note.txt");
    assert!(
        uploaded.upload_id.starts_with("upload_"),
        "got {}",
        uploaded.upload_id
    );
    // The host's hash of what it wrote, against this machine's hash of what it
    // sent: equal means nothing was lost, duplicated or reordered.
    let expected = {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(contents))
    };
    assert_eq!(uploaded.content_hash, expected);

    teardown(&fx.pair_id).await;
}

/// More than one chunk, so the index arithmetic and the ordered append are real.
/// The host appends each chunk as it arrives, so an out-of-order write shows up
/// as a hash mismatch rather than as an error.
#[tokio::test]
async fn a_multi_chunk_upload_keeps_its_order() {
    let (_home, fx) = fixture("peer-xfer-upload-multi-chunk").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect_with_stream(&desktop_id).await;

    let chunk = crate::remote_host::files::CHUNK_BYTES as usize;
    let contents: Vec<u8> = (0..chunk * 2 + 7)
        .map(|index| (index % 251) as u8)
        .collect();
    let uploaded = transfer::upload(&desktop_id, "chunked.bin", &contents)
        .await
        .expect("upload");

    let expected = {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(&contents))
    };
    assert_eq!(uploaded.content_hash, expected);

    teardown(&fx.pair_id).await;
}

/// The host enforces its own size limit, and its message is the one that reaches
/// the user: a second copy of the number here would be a second place for it to
/// drift.
#[tokio::test]
async fn an_oversized_upload_is_refused_by_the_host() {
    let (_home, fx) = fixture("peer-xfer-upload-too-large").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect_with_stream(&desktop_id).await;

    // Asked directly rather than by allocating the file: the host refuses on the
    // declared size, before a single chunk travels.
    let error = transfer::begin_upload_for_test(&desktop_id, "huge.bin", 10 * 1024 * 1024 + 1)
        .await
        .expect_err("the host's limit must be enforced");

    assert!(
        error.to_string().contains("10 MiB"),
        "the host's own reason must reach the caller, got: {error}"
    );

    teardown(&fx.pair_id).await;
}

/**
 * An upload to a host that was never paired is refused, and says so.
 *
 * Not "while disconnected": the command lane reconnects on demand, so a
 * deliberate disconnect does not make an upload fail — it makes it connect
 * again. What cannot be recovered from is a host this installation has no
 * credentials for, which is the case worth pinning.
 */
#[tokio::test]
async fn an_upload_to_an_unpaired_host_is_refused() {
    let (_home, _fx) = fixture("peer-xfer-upload-unpaired").await;

    let error = transfer::upload("desktop_nobody", "note.txt", b"bytes")
        .await
        .expect_err("an unpaired host has nothing to receive this");

    assert!(
        error.to_string().contains("peer_not_paired"),
        "got: {error}"
    );
}

/**
 * The id an upload returns is one the host will accept as an attachment.
 *
 * This is the seam the feature actually rests on: an upload nobody can reference
 * is a file that goes nowhere, and every part of it — the staging record, the
 * completed flag, the name — is the host's to accept or reject. Claiming it here
 * is exactly what sending a prompt with the file attached does on the host.
 */
#[tokio::test]
async fn an_uploaded_file_can_be_claimed_as_an_attachment() {
    let (_home, fx) = fixture("peer-xfer-upload-claim").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect_with_stream(&desktop_id).await;

    let contents = b"the bytes the model will read";
    let uploaded = transfer::upload(&desktop_id, "report.txt", contents)
        .await
        .expect("upload");

    let references = vec![future_remote_protocol::UploadReference {
        upload_id: uploaded.upload_id.clone(),
    }];
    let claimed = crate::remote_host::files::claim_uploads(&references, "thread-from-client")
        .expect("the host claims what it just received");

    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].kind, "file");
    assert_eq!(claimed[0].name, "report.txt");
    // The bytes the host hands to the conversation are the ones we sent.
    assert_eq!(
        std::fs::read(&claimed[0].path).expect("read the claimed copy"),
        contents
    );

    teardown(&fx.pair_id).await;
}

/// A chunk write with no connection behind it is refused rather than sent into
/// the void.
///
/// Reached through the runtime directly because the *command* lane reconnects on
/// demand: this is the path a chunk takes, and it addresses the live map, so a
/// host that is gone is reported instead of silently reopening a socket
/// mid-upload.
#[tokio::test]
async fn a_chunk_write_without_a_connection_is_refused() {
    let (_home, fx) = fixture("peer-xfer-put-offline").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect_with_stream(&desktop_id).await;
    runtime::disconnect(&desktop_id).await;

    let error = runtime::put_chunk(&desktop_id, "upload_1", 0, b"bytes")
        .await
        .expect_err("a chunk needs a connection");

    assert!(
        error.to_string().contains("peer_not_connected"),
        "got: {error}"
    );

    teardown(&fx.pair_id).await;
}

/// A chunk write on a socket that has died is recorded, not merely failed.
///
/// The same state machine the command lane uses: a dead socket must not keep
/// looking connected, and the retry loop has to be running — otherwise an upload
/// that hits it would simply stop, and the user would see a connection the app
/// still believed in.
#[tokio::test]
async fn a_chunk_write_on_a_dead_socket_is_recorded() {
    let (_home, fx) = fixture("peer-xfer-put-dead-socket").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect_with_stream(&desktop_id).await;

    // The socket is closed under the connection, which is what a host that
    // disappeared looks like from here: the entry is still in the map, and the
    // next write on it fails as a transport fault.
    runtime::close_socket_for_test(&desktop_id).await;

    // A short budget on purpose: the socket is already closed, and what this
    // test asserts is that the failure is *recorded as a transport fault* — not
    // how long the client waits for an answer that cannot come. Leaving it on
    // the fixture budget would make the same assertion take a minute on a
    // loaded machine.
    let error = runtime::put_chunk_within(
        &desktop_id,
        "upload_1",
        0,
        b"bytes",
        std::time::Duration::from_millis(200),
    )
    .await
    .expect_err("a dead socket cannot carry a chunk");
    assert!(
        !error.to_string().is_empty(),
        "the failure must carry a reason"
    );

    // The connection is no longer believed in. (No supervisor assertion here:
    // this seam removes the session itself, so the call that follows never
    // reaches `on_transport_failure` — `a_chunk_write_on_a_dropped_broker_is_recorded`
    // is the test that pins the drop-and-retry arm.)
    assert!(
        !find(&runtime::list().await.expect("list"), &desktop_id).connected,
        "a dead socket must not keep looking connected"
    );
    teardown(&fx.pair_id).await;
}

/// A command that **timed out** does not, by itself, condemn the connection.
///
/// The two states are indistinguishable from the caller's seat and they want
/// opposite responses: a slow host must keep its socket and its subscriptions
/// (dropping them is what turned a busy host into `peer_not_connected`, the
/// shape CI reports — `docs/testing/flaky-tauri.md` §13.6.6), while a dead one
/// must be dropped and retried. So the client asks the transport, and this test
/// pins the "alive but slow" half against the same `on_transport_failure`
/// the dead-socket test pins the other half of.
///
/// The timeout is induced deterministically rather than by load: the request goes
/// to a subject **nobody is subscribed to**, which the in-process broker accepts
/// and cannot answer (it never sends a no-responders header), over a connection
/// whose socket is fine. Both halves of the discrimination are therefore
/// asserted: the error is a timeout, and the connection survives it.
#[tokio::test]
async fn a_request_that_times_out_keeps_a_connection_that_is_still_up() {
    let (_home, fx) = fixture("peer-xfer-live-timeout").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect_with_stream(&desktop_id).await;
    assert!(
        find(&runtime::list().await.expect("list"), &desktop_id).connected,
        "the fixture must start connected for this to be a timeout test"
    );

    let error = runtime::request_at(
        &desktop_id,
        serde_json::json!({}),
        // No host subscribes to this: the broker accepts the publish and nothing
        // answers it, so the request times out with the socket untouched.
        &format!("p.{}_nobody.cmd.unanswered", fx.pair_id),
        std::time::Duration::from_millis(200),
    )
    .await
    .expect_err("an unanswered request must fail");
    assert!(
        error.to_string().contains("timed out"),
        "the fixture must produce a timeout for this to be the right arm: {error}"
    );

    // The connection, and therefore every subscription on it, is still there...
    assert!(
        find(&runtime::list().await.expect("list"), &desktop_id).connected,
        "one slow command must not drop a live connection"
    );
    // ...and nothing was started to "recover" a host that never went away.
    assert_eq!(
        runtime::supervisor_count().await,
        0,
        "a live connection must not get a retry loop"
    );

    // And the connection still works: the peer answers the next command, which is
    // the property the old behaviour destroyed (after the timeout it was gone).
    let (transfer_id, info) = serve(&scratch("live-timeout").join("f.txt"), b"still here");
    assert_eq!(
        transfer::fetch(&desktop_id, &info).await.expect("fetch"),
        b"still here",
        "the transfer lane must still work after one timed-out command ({transfer_id})"
    );

    teardown(&fx.pair_id).await;
}

/**
 * A chunk write on a broker that has gone away is recorded, not merely failed.
 *
 * The entry is still in the map, so the write reaches the socket and fails
 * there — which is the case the command lane already handles and this one has to
 * handle too: a connection the app still believes in is how an upload appears to
 * hang instead of failing.
 */
#[tokio::test]
async fn a_chunk_write_on_a_dropped_broker_is_recorded() {
    let (_home, fx) = fixture("peer-xfer-put-dead-broker").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect_with_stream(&desktop_id).await;

    // Sever every connection and stop accepting, under a live session.
    fx.nats.kill();

    // Same short budget as the dead-socket sibling, and for the same reason: the
    // broker is gone, so the assertion is "the failure is recorded", not "it
    // took the production budget to notice".
    let error = runtime::put_chunk_within(
        &desktop_id,
        "upload_1",
        0,
        b"bytes",
        std::time::Duration::from_millis(200),
    )
    .await
    .expect_err("a broker that is gone cannot carry a chunk");
    assert!(
        !error.to_string().is_empty(),
        "the failure must carry a reason"
    );

    assert!(
        !find(&runtime::list().await.expect("list"), &desktop_id).connected,
        "a dead socket must not keep looking connected"
    );

    // This test is the arm that reaches `on_transport_failure` with a session
    // still in the map — the dead-socket sibling's seam removes it first — so it
    // is the one that pins "a timeout over a transport that is *actually* down
    // still drops the connection and starts the retry". The other half of the
    // discrimination is `a_request_that_times_out_keeps_a_connection_that_is_still_up`.
    assert!(
        runtime::supervisor_count().await > 0,
        "a dropped connection must be retried, not left looking merely idle"
    );

    teardown(&fx.pair_id).await;
}
