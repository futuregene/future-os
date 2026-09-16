//! Session management — 1:1 compatible with Go internal/session/
//!
//! Split into focused submodules by responsibility:
//! - [`entry`]: the `SessionEntry` journal-line model + entry-type constants
//! - [`model`]: the `Session` / `SessionSummary` in-memory models
//! - [`run_journal`]: run lifecycle markers + unterminated-run recovery helpers
//! - [`projection`]: entries ↔ LLM-message mappings and `truncate_visible`
//! - [`repair`]: in-memory healing of corrupted/legacy session histories
//! - [`fork`]: `fork_session` and its entry-walk helper
//! - [`manager`]: SQLite session persistence and run recovery
//! - [`summary`]: session listing from SQLite
//! - [`persistence`]: the async `SessionPersistence` write pipeline
//!
//! Every public item is re-exported here so callers keep using
//! `crate::session::…` unchanged.

/// Compact summary titles fit the mobile session list's single-line layout.
/// Measured in approximate Unicode display columns, not UTF-8 bytes.
pub const SESSION_TITLE_MAX_WIDTH: usize = 32;

mod checkpoint;
pub(crate) mod compaction_ops;
#[cfg(test)]
mod compaction_ops_tests;
mod database;
pub(crate) mod display;
mod history_index;
mod history_query;
pub(crate) use history_query::HISTORY_DEFAULT_BYTES;
mod legacy_import;
pub use legacy_import::ImportRecord;
mod entry;
mod fork;
mod manager;
mod model;
mod persistence;
mod projection;
mod records;
mod repair;
mod run_journal;
pub mod sqlite_store;
mod summary;
mod tools;

pub use checkpoint::{checkpoint_to_entry, entry_to_checkpoint, latest_context_checkpoint};
pub use entry::{
    SessionEntry, ENTRY_TYPE_ASSISTANT, ENTRY_TYPE_COMPACTION, ENTRY_TYPE_CUSTOM,
    ENTRY_TYPE_CUSTOM_MESSAGE, ENTRY_TYPE_LABEL, ENTRY_TYPE_MODEL_CHANGE, ENTRY_TYPE_RUN_STARTED,
    ENTRY_TYPE_RUN_TERMINAL, ENTRY_TYPE_SESSION_INFO, ENTRY_TYPE_SYSTEM,
    ENTRY_TYPE_THINKING_LEVEL_CHANGE, ENTRY_TYPE_TOOL, ENTRY_TYPE_USER,
};
pub use fork::{fork_session, set_creation_provenance};
pub use manager::Manager;
pub use model::{Session, SessionSummary, CURRENT_SESSION_VERSION};
pub use persistence::SessionPersistence;
pub use projection::{
    agent_message_to_entry, build_context, entries_to_agent_messages, truncate_visible,
};
pub use run_journal::{
    find_run_terminal, find_unterminated_run, is_run_marker, next_run_sequence,
    RUN_STATE_CANCELLED, RUN_STATE_COMPLETED, RUN_STATE_ERROR, RUN_STATE_INCOMPLETE,
    RUN_STATE_INTERRUPTED_BY_RESTART,
};
