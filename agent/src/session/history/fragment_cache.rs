//! Disposable render cache. SQLite owns durability; these anonymous files hold
//! only JSON derived for an oversized display entry's consecutive RPC fragments.
use anyhow::{ensure, Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

const CACHE_BYTES: usize = 128 * 1024 * 1024;
const CACHE_ENTRIES: usize = 16;
const IDLE_TTL: Duration = Duration::from_secs(120);
const LIVE_FILE_LIMIT: usize = 64;
const LIVE_FILE_BYTES: usize = 512 * 1024 * 1024;
static LIVE: LazyLock<parking_lot::Mutex<(usize, usize)>> = LazyLock::new(Default::default);

/// Includes in-flight Arcs and files evicted from an LRU, not only its resident
/// entries. A slow client cannot bypass the disposable-file budget by retaining
/// the previous page while repeatedly creating a new one.
struct Spool {
    bytes: usize,
}
impl Spool {
    fn new() -> std::io::Result<Self> {
        let mut live = LIVE.lock();
        if live.0 >= LIVE_FILE_LIMIT {
            return Err(std::io::Error::other(
                "derived file budget is busy; retry this read",
            ));
        }
        live.0 += 1;
        Ok(Self { bytes: 0 })
    }
    fn grow(&mut self, bytes: usize) -> std::io::Result<()> {
        let mut live = LIVE.lock();
        let total = live
            .1
            .checked_add(bytes)
            .ok_or_else(|| std::io::Error::other("derived file size overflow"))?;
        if total > LIVE_FILE_BYTES {
            return Err(std::io::Error::other(
                "derived file byte budget exceeded; retry or request a smaller history range",
            ));
        }
        live.1 = total;
        self.bytes += bytes;
        Ok(())
    }
}
impl Drop for Spool {
    fn drop(&mut self) {
        let mut live = LIVE.lock();
        live.0 -= 1;
        live.1 -= self.bytes;
    }
}

#[derive(Default)]
pub(in crate::session) struct FragmentCache {
    items: Vec<Cached>,
    bytes: usize,
}
struct Cached {
    key: String,
    used: Instant,
    body: Arc<Fragment>,
}
impl FragmentCache {
    pub(in crate::session) fn remove(&mut self, key: &str) {
        if let Some(at) = self.items.iter().position(|item| item.key == key) {
            self.bytes -= self.items.remove(at).body.len;
        }
    }
    pub(in crate::session) fn get(&mut self, key: &str) -> Option<Arc<Fragment>> {
        self.expire();
        let at = self.items.iter().position(|item| item.key == key)?;
        let mut item = self.items.remove(at);
        item.used = Instant::now();
        let body = item.body.clone();
        self.items.push(item);
        Some(body)
    }
    pub(in crate::session) fn insert(&mut self, key: String, body: Arc<Fragment>) {
        self.expire();
        // Oversized single items still work losslessly without cache admission.
        // The cache is an acceleration and never permission to read a body.
        if body.len > CACHE_BYTES {
            return;
        }
        self.remove(&key);
        while self.items.len() >= CACHE_ENTRIES || self.bytes + body.len > CACHE_BYTES {
            self.bytes -= self.items.remove(0).body.len;
        }
        self.bytes += body.len;
        self.items.push(Cached {
            key,
            used: Instant::now(),
            body,
        });
    }
    fn expire(&mut self) {
        let now = Instant::now();
        self.items
            .retain(|item| now.duration_since(item.used) < IDLE_TTL);
        self.bytes = self.items.iter().map(|item| item.body.len).sum();
    }
}

pub(in crate::session) struct Fragment {
    pub(in crate::session) len: usize,
    pub(super) sha256: String,
    pub(super) entry_id: Value,
    file: parking_lot::Mutex<File>,
    _spool: Spool,
}
impl Fragment {
    pub(in crate::session) fn read(&self, offset: usize, limit: usize) -> Result<String> {
        ensure!(offset < self.len, "invalid history fragment offset");
        let mut bytes = vec![0; limit.min(self.len - offset)];
        {
            let mut file = self.file.lock();
            file.seek(SeekFrom::Start(u64::try_from(offset)?))?;
            file.read_exact(&mut bytes)?;
        }
        let end = match std::str::from_utf8(&bytes) {
            Ok(_) => bytes.len(),
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            Err(_) => anyhow::bail!("invalid history fragment offset"),
        };
        ensure!(end > 0, "history fragment made no progress");
        bytes.truncate(end);
        Ok(String::from_utf8(bytes)?)
    }
}

/// Derived original-text representation; disposable and shared across its
/// consecutive range requests, never a second durable transcript body.
pub(in crate::session) fn text(text: &str) -> Result<Arc<Fragment>> {
    let mut spool = Spool::new()?;
    spool.grow(text.len())?;
    let mut file = BufWriter::with_capacity(64 * 1024, tempfile::tempfile()?);
    file.write_all(text.as_bytes())?;
    let file = file.into_inner().context("flush derived text")?;
    Ok(Arc::new(Fragment {
        len: text.len(),
        sha256: format!("{:x}", Sha256::digest(text.as_bytes())),
        entry_id: Value::Null,
        file: parking_lot::Mutex::new(file),
        _spool: spool,
    }))
}

pub(super) enum Encoded {
    Inline(usize),
    Large(Arc<Fragment>),
}
impl Encoded {
    pub(super) fn len(&self) -> usize {
        match self {
            Self::Inline(len) => *len,
            Self::Large(body) => body.len,
        }
    }
}

/// Keep ordinary entries in memory. Crossing the transport budget spills once
/// to a buffered anonymous file and hashes that same stream. No fsync, database
/// write, persistent cache directory or whole-entry serialized String is needed.
pub(super) fn encode(value: &Value, inline_limit: usize) -> Result<Encoded> {
    let mut writer = RenderWriter {
        buffer: Vec::new(),
        file: None,
        hash: Sha256::new(),
        len: 0,
        inline_limit,
        spool: None,
    };
    serde_json::to_writer(&mut writer, value)?;
    match writer.file {
        None => Ok(Encoded::Inline(writer.len)),
        Some(file) => {
            let file = file.into_inner().context("flush rendered history entry")?;
            Ok(Encoded::Large(Arc::new(Fragment {
                len: writer.len,
                sha256: format!("{:x}", writer.hash.finalize()),
                entry_id: value["id"].clone(),
                file: parking_lot::Mutex::new(file),
                _spool: writer
                    .spool
                    .context("rendered history has no file reservation")?,
            })))
        }
    }
}
struct RenderWriter {
    buffer: Vec<u8>,
    file: Option<BufWriter<File>>,
    hash: Sha256,
    len: usize,
    inline_limit: usize,
    spool: Option<Spool>,
}
impl Write for RenderWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let length = self
            .len
            .checked_add(bytes.len())
            .ok_or_else(|| std::io::Error::other("history entry length overflow"))?;
        if self.file.is_none() && length > self.inline_limit {
            let mut spool = Spool::new()?;
            spool.grow(self.buffer.len())?;
            let mut file = BufWriter::with_capacity(64 * 1024, tempfile::tempfile()?);
            file.write_all(&self.buffer)?;
            self.hash.update(&self.buffer);
            self.buffer = Vec::new();
            self.file = Some(file);
            self.spool = Some(spool);
        }
        if let Some(file) = &mut self.file {
            self.spool
                .as_mut()
                .expect("spooled render reservation")
                .grow(bytes.len())?;
            file.write_all(bytes)?;
            self.hash.update(bytes);
        } else {
            self.buffer.extend_from_slice(bytes);
        }
        self.len = length;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(file) = &mut self.file {
            file.flush()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn spilling_serializes_and_hashes_once_and_fragments_preserve_utf8() {
        let value = json!({"id":"e","text":"中🙂\n\"\\".repeat(300)});
        let expected = serde_json::to_string(&value).unwrap();
        let Encoded::Large(body) = encode(&value, 32).unwrap() else {
            panic!("must spill")
        };
        assert_eq!(body.len, expected.len());
        assert_eq!(
            body.sha256,
            format!("{:x}", Sha256::digest(expected.as_bytes()))
        );
        let mut actual = String::new();
        while actual.len() < body.len {
            let piece = body.read(actual.len(), 43).unwrap();
            assert!(piece.len() <= 43);
            actual.push_str(&piece);
        }
        assert_eq!(actual, expected);
        let mid = expected.find('中').unwrap() + 1;
        assert!(body.read(mid, 43).is_err());
        assert!(body.read(body.len, 43).is_err());
    }

    #[test]
    fn small_entries_never_create_a_file_and_caches_are_disposable() {
        assert!(matches!(
            encode(&json!({"id":"e"}), 1024).unwrap(),
            Encoded::Inline(_)
        ));
        let Encoded::Large(body) = encode(&json!({"id":"e","text":"x".repeat(500)}), 32).unwrap()
        else {
            panic!("must spill")
        };
        let mut cache = FragmentCache::default();
        cache.insert("generation-1".into(), body.clone());
        assert!(Arc::ptr_eq(&cache.get("generation-1").unwrap(), &body));
        assert!(cache.get("generation-2").is_none());
        cache.items[0].used = Instant::now() - IDLE_TTL;
        assert!(cache.get("generation-1").is_none());
        assert_eq!(cache.bytes, 0);
        // Eviction releases acceleration only. An in-flight Arc still reads
        // the same derived bytes independently from the cache's lifetime.
        assert!(!body.read(0, 40).unwrap().is_empty());
    }

    #[test]
    fn lru_bounds_entries_bytes_and_replaces_an_existing_key() {
        let Encoded::Large(body) = encode(&json!({"id":"e","text":"x".repeat(500)}), 32).unwrap()
        else {
            panic!("must spill")
        };
        let mut cache = FragmentCache::default();
        for index in 0..CACHE_ENTRIES {
            cache.insert(index.to_string(), body.clone());
        }
        cache.get("0").unwrap();
        cache.insert("next".into(), body.clone());
        assert_eq!(cache.items.len(), CACHE_ENTRIES);
        assert!(cache.get("1").is_none());
        assert!(cache.get("0").is_some());
        let bytes = cache.bytes;
        cache.insert("0".into(), body);
        assert_eq!(cache.bytes, bytes);
        assert!(cache.bytes <= CACHE_BYTES);
    }
}
