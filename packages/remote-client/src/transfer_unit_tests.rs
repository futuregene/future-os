use super::*;
use crate::runtime::parse_chunk_subject;
fn cancel_command(kind: TransferKind) -> &'static str {
    kind.cancel_command()
}

#[test]
fn only_a_transport_fault_is_worth_retrying() {
    use crate::Error as AppError;
    assert!(retryable(&AppError::RemoteTransport(
        "Remote request timed out".into()
    )));
    // A host that answered and refused: it will refuse again.
    assert!(!retryable(&AppError::Message(
        "Download expired or does not exist.".into()
    )));
    // A local misconfiguration is not the link's fault either.
    assert!(!retryable(&AppError::Message("peer_not_connected".into())));
}

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
fn a_hash_that_does_not_match_the_bytes_is_refused() {
    let bytes = b"what we sent";
    let correct = {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(bytes))
    };

    assert_eq!(
        verified_hash(&correct, bytes, "n.txt").expect("the matching hash"),
        correct
    );
    // Case is not part of the contract: a host may hex either way.
    assert!(verified_hash(&correct.to_uppercase(), bytes, "n.txt").is_ok());

    // A hash of something else: the bytes that arrived are not the bytes sent.
    let wrong = {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(b"something else"))
    };
    let error = verified_hash(&wrong, bytes, "n.txt").expect_err("a mismatch must be refused");
    assert!(
        error
            .to_string()
            .contains("remote_upload_hash_mismatch:n.txt"),
        "the refusal must name the file, got: {error}"
    );
}

#[test]
fn the_attachment_kind_follows_the_extension() {
    for image in [
        "photo.JPG",
        "a.png",
        "b.webp",
        "c.heic",
        "d.tiff",
        "scan.svg",
        "e.jpeg",
    ] {
        assert_eq!(upload_kind(image), "image", "{image}");
    }
    for other in [
        "notes.txt",
        "archive.tar.gz",
        // No dot at all: the whole name would otherwise read as an extension.
        "Makefile",
        ".gitignore",
        "data.json",
        "report.pdf",
    ] {
        assert_eq!(upload_kind(other), "file", "{other}");
    }
}

#[test]
fn a_zero_chunk_size_is_refused_by_name() {
    assert_eq!(
        usable_chunk_size(512 * 1024).expect("a normal size"),
        512 * 1024
    );

    let error = usable_chunk_size(0).expect_err("zero must be refused");
    assert!(
        error.to_string().contains("remote_upload_bad_chunk_size"),
        "got: {error}"
    );
}

#[test]
fn an_enormous_chunk_size_becomes_one_chunk() {
    let chunk = usable_chunk_size(u64::MAX).expect("usable");
    assert_eq!(chunk, usize::MAX);
    assert_eq!(b"four".chunks(chunk).count(), 1);
}

#[test]
fn each_direction_has_its_own_cancel_command() {
    assert_eq!(cancel_command(TransferKind::Download), "download_cancel");
    assert_eq!(cancel_command(TransferKind::Upload), "upload_cancel");
}

#[test]
fn a_chunk_exceeding_the_declared_file_size_never_grows_the_buffer() {
    let mut bytes = vec![1, 2];
    append_chunk(&mut bytes, &[3, 4], 4).unwrap();
    assert_eq!(bytes, vec![1, 2, 3, 4]);
    let error = append_chunk(&mut bytes, &[5, 6, 7], 4).unwrap_err();
    assert_eq!(error.to_string(), "remote_download_size_mismatch:4:7");
    assert_eq!(bytes, vec![1, 2, 3, 4]);
}
