mod app_settings;
mod approval_config;
mod artifacts;
mod cleanup;
mod markdown_refs;
mod messages;
mod records;
mod research;
mod review;
mod runs;
mod schema;
mod support;
mod threads;
mod workspaces;

use support::*;

pub use app_settings::{
    get_app_settings, update_app_settings, AppSettings, UpdateAppSettingsInput,
};
pub use artifacts::{
    artifact_type_from_path, create_artifact, delete_artifact, ensure_artifact,
    import_attachment_artifact, list_artifacts,
};
pub use cleanup::{
    cancel_stale_approval_requests, clear_finished_runs, get_thread_cleanup_summary,
};
pub use markdown_refs::{resolve_markdown_references, search_reference_targets};
pub use messages::{append_message, list_messages};
pub use records::*;
pub use research::{list_research_resources, promote_artifact_to_research};
pub use review::{
    decide_approval_request, ensure_approval_request, ensure_review_change, list_approval_requests,
    list_review_changesets, list_review_file_changes, update_review_changeset_status,
};
pub use runs::{
    append_run_event, complete_tool_call, create_run, list_run_events, list_runs, list_tool_calls,
    list_tool_outputs, update_run_status, upsert_tool_call,
};
pub use support::{get_approval_request, get_run};
pub use threads::{
    archive_thread, create_thread, delete_thread, get_recent_thread, get_thread, list_threads,
    pin_thread, rename_thread, restore_thread, update_thread_model,
};
pub use workspaces::{
    create_workspace, get_or_create_chat_workspace, get_workspace, list_workspaces,
};

pub fn app_data_path() -> Result<AppDataPath, String> {
    Ok(AppDataPath {
        app_dir: app_dir()?.display().to_string(),
        db_path: db_path()?.display().to_string(),
    })
}

pub fn initialize_app_store() -> Result<(), String> {
    ensure_app_dirs()?;
    let conn = connect()?;
    apply_schema(&conn)?;
    Ok(())
}
