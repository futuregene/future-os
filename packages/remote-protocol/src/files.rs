//! Wire responses for Remote file preparation and upload completion.

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
