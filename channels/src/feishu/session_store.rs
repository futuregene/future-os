//! Feishu's view of the shared conversation → agent-session store.
//!
//! The store itself lives in [`crate::session_store`] so every channel maps a
//! platform conversation onto an agent session the same way. This module only
//! keeps the historical import paths resolving.

pub use crate::session_store::{SessionEntry, SessionStore};
