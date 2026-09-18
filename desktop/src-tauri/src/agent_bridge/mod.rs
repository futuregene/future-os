mod approval;
mod client;
pub(crate) mod config;
mod config_observer;
mod delete_outbox;
mod headless;
mod import;
mod models;
mod observer;
mod persist;
mod prompt;
mod queries;
mod reconciliation;
mod replica;
mod review;
mod run_control;
mod session;
mod session_events;
mod skills;
mod stream;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use self::test_support::get_state_payload;

pub use self::approval::{decide_approval, inject_session_rule, reconcile_pending_approvals};
pub(crate) use self::client::get_agent_info;
pub(crate) use self::client::raw_agent_addr;
pub use self::client::{
    connect_agent, delete_session_command, get_available_models_command, get_run_state_command,
    get_session_entries_before_command, get_session_entries_page_command, get_state_command,
    list_streaming_sessions_command, map_rpc_error, set_default_model_command, set_model_command,
    set_session_name_command, set_thinking_level_command, RpcResponseExt,
};
pub use self::config_observer::spawn_provider_config_observer;
pub use self::delete_outbox::{reconcile_delete_outbox, spawn_delete_outbox_worker};
pub use self::headless::{
    prepare_prompt_persisted_with_trigger, run_prepared_prompt_with_acceptance, PreparedPrompt,
};
pub(crate) use self::import::{import_missing_sessions, list_agent_session_ids};
pub use self::models::{list_agent_models, list_builtin_providers, AgentModelOption};
pub use self::observer::{
    drop_observer, ensure_observer_for_thread, seed_observers_from_store, spawn_session_discovery,
};
#[cfg(test)]
pub use self::prompt::agent_prompt;
pub(crate) use self::prompt::agent_prompt_with_acceptance;
pub use self::prompt::{agent_prompt_with_model_context, AgentPromptRequest, AgentPromptResponse};
#[cfg(test)]
use self::queries::reload_agent_credentials;
pub use self::queries::{
    generate_session_title, get_available_models, get_events_since, get_events_since_payload,
    get_session_entries, get_session_entries_before, get_session_messages, get_session_state,
    probe_sandbox, probe_windows_sandbox, rename_session, reset_windows_sandbox, set_default_model,
    set_session_model, set_session_thinking_level, sync_future_models, SandboxProbeResult,
    SyncFutureModelsResult, WindowsSandboxProbeResult,
};
pub(crate) use self::queries::{
    get_events_since_page, get_run_snapshot, provision_agent_session, query_tools,
};
pub use self::reconciliation::{
    attach_remote_stream, reconcile_interrupted_runs, reconcile_thread_workspace,
    spawn_active_run_watchdog,
};
pub use self::run_control::{abort_run, compact_agent_session, compact_thread_context};
pub(crate) use self::run_control::{abort_session, wait_for_agent_idle};
pub use self::session::fork_agent_session;
pub use self::session_events::spawn_session_events_observer;
pub use self::skills::{list_installed_skills, refresh_skills, InstalledSkill};
#[cfg(test)]
pub use review::capture_before;
#[cfg(test)]
pub use review::finalize_after;
pub use review::retry as retry_run_review;

use serde::Serialize;

pub use self::client::AttachmentInput;
use self::client::{base_command, prompt_command};
#[cfg(test)]
use self::delete_outbox::{delete_outbox_interval, TEST_OUTBOX_STOP};
#[cfg(test)]
use self::prompt::auto_name_thread;
use self::queries::fetch_all_session_entries_with_client;
#[cfg(test)]
use self::queries::{next_events_cursor, EVENTS_PAGE_SIZE};
#[cfg(test)]
use self::reconciliation::{
    active_run_watchdog_pass, check_and_reanimate_run, plan_active_run_reconciliation,
    reconcile_active_run_once, reconcile_run_gone, settle_from_agent_terminal, watchdog_interval,
    ActiveRunAction, TEST_WATCHDOG_STOP, WATCHDOG_GRACE_SECS, WATCHDOG_INTERVAL_SECS,
    WATCHDOG_ORPHAN_SECS,
};
use self::replica::AGENT_REPLICAS;
use self::run_control::{mark_run_completed_if_active, mark_run_failed_if_active};
pub(crate) use self::session::workspace_path_for_thread;
use self::session::{ensure_agent_session, set_agent_permission_level, set_agent_sandbox_policy};
