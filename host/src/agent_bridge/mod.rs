mod approval;
mod client {
    pub use crate::agent_client::{connect_agent, get_agent_readiness, map_rpc_error, AgentClient};
    pub use crate::agent_commands::*;
}
pub mod config;
pub mod config_observer;
mod delete_outbox;
mod global_loop;
mod headless;
mod import;
pub mod models;
mod observer;
pub mod persist;
mod presence;
mod prompt;
mod queries;
mod reconciliation;
mod replica;
pub mod review;
mod run_control;
mod session;
mod session_events;
pub mod skills;
mod stream;
#[cfg(test)]
pub(crate) use crate::test_support::agent as test_support;
#[cfg(test)]
mod tests;

pub use self::approval::{decide_approval, inject_session_rule, reconcile_pending_approvals};
pub use self::client::get_agent_readiness;
pub use self::client::{
    connect_agent, get_available_models_command, get_run_state_command,
    get_session_entries_before_command, get_session_entries_page_command, get_state_command,
    is_session_not_found, list_streaming_sessions_command, map_rpc_error,
    set_default_model_command, set_model_command, set_session_name_command,
    set_thinking_level_command, RpcResponseExt,
};

pub use self::config_observer::spawn_provider_config_observer;
pub use self::delete_outbox::{reconcile_delete_outbox, spawn_delete_outbox_worker};
pub use self::headless::{
    prepare_prompt_persisted_with_trigger, run_prepared_prompt_with_acceptance, PreparedPrompt,
};
pub(crate) use self::import::{list_agent_session_ids, session_has_messages};
pub use self::models::{list_agent_models, list_builtin_providers, AgentModelOption};
#[cfg(any(test, feature = "test-support"))]
pub use self::observer::has_observer;
pub(crate) use self::observer::reconcile_deleted_session;
pub use self::observer::{
    drop_observer, ensure_observer_for_session, ensure_observer_for_thread,
    reconcile_streaming_observers, spawn_session_discovery, spawn_streaming_observer_monitor,
};
pub use self::presence::{
    require_present_with_client, session_presence, session_presence_with_client,
    verify_session_database, verify_thread_source, verify_thread_source_with_client,
};
#[cfg(any(test, feature = "test-support"))]
pub use self::prompt::agent_prompt;
pub use self::prompt::agent_prompt_with_acceptance;
pub use self::prompt::{agent_prompt_with_model_context, AgentPromptRequest, AgentPromptResponse};
pub use self::queries::provision_agent_session_with_policy;
#[cfg(test)]
use self::queries::reload_agent_credentials;
pub use self::queries::{
    generate_session_title, get_available_models, get_events_since, get_events_since_payload,
    get_session_entries, get_session_entries_before, get_session_messages, get_session_state,
    get_session_storage_format, probe_sandbox, probe_windows_sandbox, rename_session,
    request_future_models_refresh, reset_windows_sandbox, set_default_model, set_session_model,
    set_session_thinking_level, sync_future_models, RequestFutureModelsRefreshResult,
    SandboxProbeResult, SyncFutureModelsResult, WindowsSandboxProbeResult,
};
pub use self::queries::{
    get_event_content_range, get_events_since_page, get_events_since_page_until, get_run_snapshot,
    get_session_entries_before_with_client, get_session_entries_with_client, get_tool_call_args,
    provision_agent_session, query_tools, session_has_entries,
};
pub use self::reconciliation::{
    attach_remote_stream, reconcile_interrupted_runs, reconcile_thread_workspace,
    spawn_active_run_watchdog,
};
pub use self::run_control::{abort_run, compact_agent_session, compact_thread_context};
pub use self::run_control::{abort_session, wait_for_agent_idle, AbortSessionOutcome};
pub use self::session::fork_agent_session;
pub use self::session::set_agent_sandbox_policy_tier;
pub use self::session_events::spawn_session_events_observer;

/// Test-only: stop every process-global loop that re-attaches an Agent stream.
///
/// A fixture that owns the mock's script queue must own these too: the mock
/// hands each scripted stream to whoever attaches first, so a loop left running
/// by an earlier test takes the script this one is about to push, and this test
/// then waits on an unscripted (forever-parked) stream while holding the mock's
/// lock — the whole suite stops with no `test result` line.
#[cfg(any(test, feature = "test-support"))]
pub fn stop_global_agent_observers_for_test() {
    config_observer::stop_provider_config_observer_for_test();
    session_events::stop_session_events_observer_for_test();
}
pub use self::skills::{
    install_skill, list_available_skills, list_installed_skills, refresh_skills, suggest_skill,
    sync_skills, uninstall_skill, AvailableSkill, InstalledSkill, SkillCandidate,
};
#[cfg(any(test, feature = "test-support"))]
pub use review::capture_before;
#[cfg(any(test, feature = "test-support"))]
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
    plan_settled_run_reconciliation, reconcile_active_run_once, reconcile_run_gone,
    settle_from_agent_terminal, watchdog_interval, ActiveRunAction, SettledRunLedger,
    TEST_WATCHDOG_STOP, WATCHDOG_GRACE_SECS, WATCHDOG_INTERVAL_SECS, WATCHDOG_ORPHAN_SECS,
};
use self::replica::AGENT_REPLICAS;
use self::run_control::{mark_run_completed_if_active, mark_run_failed_if_active};
pub use self::session::workspace_path_for_thread;
use self::session::{
    ensure_agent_session, ensure_named_agent_session, set_agent_permission_level,
    set_agent_sandbox_policy,
};

#[cfg(any(test, feature = "test-support"))]
pub use observer::cancel_all_observers;
pub use observer::settings_event_payload;
