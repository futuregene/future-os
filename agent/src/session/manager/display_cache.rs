//! Revision-bound cache for complete display exports. Indexed pages bypass it.
use super::Manager;
use anyhow::{anyhow, Result};
use std::sync::Arc;

pub(super) const DISPLAY_PROJECTION_CACHE_CAPACITY: usize = 12;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SessionRevision {
    pub(super) revision: i64,
}

pub(super) struct DisplayProjectionCacheEntry {
    session_id: String,
    revision: SessionRevision,
    entries: Arc<Vec<serde_json::Value>>,
}

impl Manager {
    pub(crate) fn session_revision(&self, id: &str) -> Result<SessionRevision> {
        Ok(SessionRevision {
            revision: self
                .storage()?
                .revision(id)?
                .ok_or_else(|| anyhow!("session not found"))?,
        })
    }

    pub(crate) fn cached_display_projection(
        &self,
        id: &str,
        revision: &SessionRevision,
    ) -> Option<Arc<Vec<serde_json::Value>>> {
        let mut cache = self.display_projection_cache.lock();
        let index = cache
            .iter()
            .position(|entry| entry.session_id == id && entry.revision == *revision)?;
        let entry = cache.remove(index);
        let entries = entry.entries.clone();
        cache.push(entry);
        Some(entries)
    }

    pub(crate) fn cache_display_projection(
        &self,
        id: &str,
        revision: SessionRevision,
        entries: Arc<Vec<serde_json::Value>>,
    ) {
        let mut cache = self.display_projection_cache.lock();
        cache.retain(|entry| entry.session_id != id);
        cache.push(DisplayProjectionCacheEntry {
            session_id: id.to_string(),
            revision,
            entries,
        });
        if cache.len() > DISPLAY_PROJECTION_CACHE_CAPACITY {
            cache.remove(0);
        }
    }

    pub(super) fn invalidate_display_projection(&self, id: &str) {
        self.display_projection_cache
            .lock()
            .retain(|entry| entry.session_id != id);
    }
}
