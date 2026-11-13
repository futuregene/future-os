//! Atomic updates of the authoritative current session metadata.
use super::Manager;
use anyhow::{Context, Result};
use rusqlite::OptionalExtension;

impl Manager {
    pub fn update_session_info(&self, id: &str, key: &str, value: serde_json::Value) -> Result<()> {
        self.update_session_info_fields(id, [(key.to_owned(), value)].into_iter().collect())
    }

    /// The parent recorded for `id`, or `None` when it has none — including
    /// when the session has no metadata row yet or is unknown.
    ///
    /// Reads the metadata column directly rather than loading the session: this
    /// answers a lineage question, and the delegation guard walks a chain with
    /// it, so loading whole histories to read one field would make creating a
    /// sub-session cost O(history) per level.
    pub fn parent_session_id(&self, id: &str) -> Result<Option<String>> {
        let id = id.to_owned();
        self.storage()?.db.read(move |db| {
            let id=crate::session::identity::BoundIdentity::session(db,&id)?;
            let parent:Option<Option<String>>=db.query_row(
                "SELECT CASE WHEN json_valid(current_metadata_json) THEN CASE WHEN json_type(current_metadata_json,'$.parent_session_id')='text' THEN json_extract(current_metadata_json,'$.parent_session_id') END END FROM sessions WHERE id=?1",
                [&id],|row|row.get(0)).optional().context("session metadata could not be read")?;
            Ok(parent.flatten().map(|parent|parent.trim().to_owned()).filter(|parent|!parent.is_empty()))
        })
    }

    /// Save one metadata snapshot so related usage counters survive restart together.
    /// The journal payloads needed to price a session's spend request by
    /// request; see `SqliteStore::pricing_event_payloads`.
    pub(crate) fn pricing_event_payloads(&self, id: &str) -> Result<Vec<String>> {
        self.storage()?.pricing_event_payloads(id)
    }

    pub(crate) fn update_session_info_fields(
        &self,
        id: &str,
        fields: serde_json::Map<String, serde_json::Value>,
    ) -> Result<()> {
        self.prepare_session(id)?;
        self.storage()?.patch_segmented_metadata(id, fields)?;
        self.invalidate_display_projection(id);
        Ok(())
    }
}
