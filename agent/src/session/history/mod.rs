//! Session history: display projection/indexing, paged transport and original
//! text recall. Index updates participate in transcript write transactions;
//! paging materializes selected bodies after leaving the database worker.
mod display_index;
mod display_projection;
pub(super) mod fragment_cache;
pub(super) mod paging;
mod recall;
mod recovery;

pub(crate) use recovery::PROJECTION_VERSION;
pub(super) use recovery::{ensure_history_index, mark_history_index_ready, with_history_index};

pub(super) use display_index::{
    insert_entry_shape, refresh_session_metadata, update_display_index_from,
};
pub(crate) use display_projection::project_entries;
pub(crate) use recall::{HISTORY_DEFAULT_BYTES, HISTORY_DEFAULT_SESSIONS};
