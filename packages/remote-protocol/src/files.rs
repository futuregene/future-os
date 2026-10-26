//! Wire responses for Remote file preparation and upload completion.

/// The `mode` a `download_prepare` may carry.
///
/// A variant names the *client that is asking*, not a size or a format, because
/// the two clients have different reasons to limit a transfer and one policy
/// served neither: a phone decodes previews in a 10 MiB budget and can only act
/// on types its own reader knows, while a desktop is saving a copy and was being
/// refused a 12.5 MB installer because it spoke the phone's dialect.
///
/// `preview` and `original` are the phone's pair (preview is the resized/validated
/// variant, `original` the bytes unchanged). `download` is a desktop save-as and
/// carries the desktop's own ceiling.
pub const VARIANT_PREVIEW: &str = "preview";
pub const VARIANT_ORIGINAL: &str = "original";
pub const VARIANT_DESKTOP_DOWNLOAD: &str = "download";

/// What a host answers for a `mode` it does not implement.
///
/// This is a matched contract rather than prose: a newer client compares it
/// against a host's refusal and, on a match, asks again for `original` — the
/// behaviour that host already has. That is what makes the desktop variant
/// additive, so an old host costs one extra request instead of a failed
/// download. Keep the wording stable.
pub const UNSUPPORTED_VARIANT: &str = "Unsupported download variant.";

/// What the host says about a prepared download.
///
/// Only the fields this client acts on: the size and chunk size drive the pull
/// loop, and the hash is the integrity check. The host also sends a MIME type
/// and the variant it settled on, which a save-to-a-chosen-path flow has no use
/// for — the user already picked the destination, and the name the host chose
/// comes back with the file.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadInfo {
    pub transfer_id: String,
    pub name: String,
    pub size: u64,
    pub content_hash: String,
    pub chunk_bytes: u64,
}

/// What the host says about a started upload.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadInit {
    pub upload_id: String,
    pub chunk_bytes: u64,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadComplete {
    pub content_hash: String,
}
