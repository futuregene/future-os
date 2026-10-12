//! Session history: display projection/indexing, paged transport and original
//! text recall. Index updates participate in transcript write transactions;
//! paging materializes selected bodies after leaving the database worker.
mod display_index;
mod display_projection;
mod paging;
mod recall;

pub(super) use display_index::{
    insert_entry_shape, refresh_session_metadata, update_display_index_from,
};
pub(crate) use display_projection::project_entries;
pub(crate) use recall::{HISTORY_DEFAULT_BYTES, HISTORY_DEFAULT_SESSIONS};
