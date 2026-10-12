use super::*;
use crate::session::projection::{agent_message_to_entry, entries_to_agent_messages};
use crate::session::repair::{entry_text_starts_with, TOOL_LOST_PLACEHOLDER_PREFIX};
use crate::session::run_journal::RUN_STATE_COMPLETED;
use crate::session::{
    ENTRY_TYPE_ASSISTANT, ENTRY_TYPE_MODEL_CHANGE, ENTRY_TYPE_SESSION_INFO, ENTRY_TYPE_SYSTEM,
    ENTRY_TYPE_TOOL,
};
use crate::session::{ENTRY_TYPE_RUN_STARTED, ENTRY_TYPE_RUN_TERMINAL};
use crate::types::ToolCall;
use crate::utils::generate_id;
use chrono::Local;

fn raw_lines(manager: &Manager, id: &str) -> String {
    manager
        .storage()
        .unwrap()
        .entries(id)
        .unwrap()
        .into_iter()
        .map(|v| format!("{v}\n"))
        .collect()
}

fn temp_manager(tag: &str) -> (std::path::PathBuf, Manager) {
    let dir = std::env::temp_dir().join(format!("future-{tag}-{}", generate_id()));
    let manager = Manager::new(dir.clone());
    (dir, manager)
}

mod display_cache;
mod encoding;
mod fork_validation;
mod metadata;
mod run_recovery;
mod storage;
mod tool_deduplication;
mod tool_recovery;
