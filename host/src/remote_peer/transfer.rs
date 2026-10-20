//! Desktop file destinations and preview-cache adapter for Remote transfers.
#[cfg(test)]
pub use future_remote_client::transfer::DownloadInfo;
use future_remote_client::transfer::{DownloadedFile, UploadedFile};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub async fn download(
    desktop: &str,
    session: &str,
    path: &str,
    name: Option<&str>,
    variant: &str,
) -> Result<DownloadedFile, crate::Error> {
    super::runtime::client()
        .download(desktop, session, path, name, variant)
        .await
        .map_err(Into::into)
}
pub async fn upload(desktop: &str, name: &str, bytes: &[u8]) -> Result<UploadedFile, crate::Error> {
    super::runtime::client()
        .upload(desktop, name, bytes)
        .await
        .map_err(Into::into)
}
#[cfg(test)]
pub async fn finish(desktop: &str, info: DownloadInfo) -> Result<DownloadedFile, crate::Error> {
    super::runtime::client()
        .finish_download(desktop, info)
        .await
        .map_err(Into::into)
}
#[cfg(test)]
pub async fn fetch(desktop: &str, info: &DownloadInfo) -> Result<Vec<u8>, crate::Error> {
    super::runtime::client()
        .fetch_download_for_test(desktop, info)
        .await
        .map_err(Into::into)
}
#[cfg(test)]
pub async fn begin_upload_for_test(
    desktop: &str,
    name: &str,
    size: u64,
) -> Result<(), crate::Error> {
    super::runtime::client()
        .begin_upload_for_test(desktop, name, size)
        .await
        .map_err(Into::into)
}
/// How many pulled previews are kept on this machine.
///
/// The cache exists so that looking at the same file twice does not pull it twice,
/// not so that every file ever opened is kept. Sixty-four entries is well past
/// what a reading session touches, and the directory holds only preview variants
/// (bounded by the host, which resizes images and refuses anything over 10 MiB).
const PREVIEW_CACHE_MAX_ENTRIES: usize = 64;

/// How long a pulled preview stays.
///
/// The bytes are another machine's file, kept only long enough to look at: a week
/// is longer than any session and short enough that a cache nobody prunes by hand
/// cannot become a second copy of someone's folder.
const PREVIEW_CACHE_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Where a preview pulled from `path` on a host is cached, creating its directory.
///
/// The **name is kept, at the end of the file name**, because the overlay picks
/// its renderer from the extension (`previewKindForPath`): caching
/// "forest-plot.png" as an opaque digest would make every image arrive as an
/// unreadable blob. What precedes it is a digest of the source — machine, session
/// and path — so the same file pulled twice lands on the same entry instead of
/// accumulating copies, and two files with one name stay apart.
pub fn preview_cache_path(
    desktop_id: &str,
    session_id: &str,
    path: &str,
    name: &str,
) -> Result<PathBuf, crate::Error> {
    let mut hasher = Sha256::new();
    // Separators that cannot occur in a path, so two different triples cannot
    // hash alike by concatenation.
    hasher.update(desktop_id.as_bytes());
    hasher.update([0x1f]);
    hasher.update(session_id.as_bytes());
    hasher.update([0x1f]);
    hasher.update(path.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    let root = crate::store::remote_previews_dir()?;
    std::fs::create_dir_all(&root)?;
    Ok(root.join(format!("{}__{}", &digest[..16], cache_file_name(name))))
}

/// The host's name, reduced to something this filesystem can hold.
///
/// The extension survives — it is what the overlay reads — while separators,
/// control characters and the names a filesystem reserves are replaced. A name
/// with nothing usable left becomes `preview`, which is still previewable by
/// content sniffing on the image path and readable as text.
fn cache_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
        .collect();
    // Leading dots and spaces are how Windows spells "no name", and a name of
    // nothing but them has no extension to keep either.
    let cleaned = cleaned.trim_matches(['.', ' ']);
    if cleaned.is_empty() {
        "preview".to_string()
    } else {
        cleaned.to_string()
    }
}

/// Keep the preview cache bounded: the newest entries up to the count, and nothing
/// past its age.
///
/// Best effort by design — a cache that cannot be pruned is not a reason to fail
/// the preview the user just asked for, and the next one prunes again.
pub fn prune_preview_cache() {
    let Ok(root) = crate::store::remote_previews_dir() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    let now = SystemTime::now();
    let mut kept: Vec<(SystemTime, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        // A partially written file (the `.part` neighbour of `write_atomically`)
        // is not an entry: the next write replaces it, and counting it here would
        // evict a real preview for a temporary one.
        if path.extension().is_some_and(|ext| ext == "part") {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let expired = now
            .duration_since(modified)
            .is_ok_and(|age| age > PREVIEW_CACHE_TTL);
        if expired {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        kept.push((modified, path));
    }
    if kept.len() <= PREVIEW_CACHE_MAX_ENTRIES {
        return;
    }
    kept.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    for (_, path) in kept.split_off(PREVIEW_CACHE_MAX_ENTRIES) {
        let _ = std::fs::remove_file(&path);
    }
}

/// Write a downloaded file to a path the caller chose, atomically.
///
/// Takes the path as a string because that is what arrives from the UI, and
/// keeps the call site short enough to stay on one line — a wrapped call would
/// put its `?` alone on a line that then reads as uncovered whatever the tests
/// do.
pub fn write_to_path(destination: &str, bytes: &[u8]) -> Result<(), crate::Error> {
    write_atomically(std::path::Path::new(destination), bytes)
}

/// Write a downloaded file to `destination`, atomically.
///
/// A partial file that looks complete is the one failure a download must not
/// have, so the bytes go to a temporary neighbour and are renamed into place
/// only once the whole file is on disk.
pub fn write_atomically(destination: &Path, bytes: &[u8]) -> Result<(), crate::Error> {
    let file_name = destination
        .file_name()
        .ok_or_else(|| crate::Error::Message("remote_download_bad_destination".into()))?;
    let mut temporary = destination.to_path_buf();
    temporary.set_file_name(format!(".{}.part", file_name.to_string_lossy()));
    std::fs::write(&temporary, bytes)
        .map_err(|error| crate::Error::Message(format!("remote_download_write_failed: {error}")))?;
    std::fs::rename(&temporary, destination).map_err(|error| {
        // Leave no debris behind: the rename failed, so the partial copy is
        // removed rather than left beside the destination.
        let _ = std::fs::remove_file(&temporary);
        crate::Error::Message(format!("remote_download_write_failed: {error}"))
    })
}
