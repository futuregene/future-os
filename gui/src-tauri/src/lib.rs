mod agent_bridge;
mod agent_proto;
mod agent_providers;
mod fs_commands;
mod git_review;
mod store;

use fs_commands::{export_artifact_file, open_path, read_text_file_preview, save_pasted_image};

#[tauri::command]
fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[tauri::command]
fn app_data_path() -> Result<store::AppDataPath, String> {
    store::app_data_path()
}

#[tauri::command]
fn initialize_app_store() -> Result<(), String> {
    store::initialize_app_store()
}

#[tauri::command]
fn cancel_stale_approval_requests() -> Result<usize, String> {
    store::cancel_stale_approval_requests()
}

#[tauri::command]
fn list_agent_providers() -> Result<agent_providers::ProvidersView, String> {
    agent_providers::list_agent_providers()
}

#[tauri::command]
fn upsert_custom_provider(
    input: agent_providers::UpsertCustomProviderInput,
) -> Result<agent_providers::ProvidersView, String> {
    agent_providers::upsert_custom_provider(input)
}

#[tauri::command]
fn delete_custom_provider(id: String) -> Result<agent_providers::ProvidersView, String> {
    agent_providers::delete_custom_provider(id)
}

#[tauri::command]
fn get_app_settings() -> Result<store::AppSettings, String> {
    store::get_app_settings()
}

#[tauri::command]
fn update_app_settings(input: store::UpdateAppSettingsInput) -> Result<store::AppSettings, String> {
    store::update_app_settings(input)
}

#[tauri::command]
fn clear_finished_runs(thread_id: String) -> Result<usize, String> {
    store::clear_finished_runs(&thread_id)
}

#[tauri::command]
fn list_threads() -> Result<Vec<store::ThreadRecord>, String> {
    store::list_threads()
}

#[tauri::command]
fn list_workspaces() -> Result<Vec<store::WorkspaceRecord>, String> {
    store::list_workspaces()
}

#[tauri::command]
fn create_workspace(input: store::CreateWorkspaceInput) -> Result<store::WorkspaceRecord, String> {
    let workspace = store::create_workspace(input)?;
    ensure_workspace_git_repo(&workspace);
    Ok(workspace)
}

/// Initialise git for a user workspace whose directory is not tracked yet.
/// No-op for temporary chat workspaces and when git is not installed.
fn ensure_workspace_git_repo(workspace: &store::WorkspaceRecord) {
    if workspace.kind != "user" {
        return;
    }
    git_review::ensure_git_init(std::path::Path::new(&workspace.path));
}

#[tauri::command]
fn ensure_workspace_git(workspace_id: String) -> Result<bool, String> {
    let workspace = store::get_workspace(&workspace_id)?
        .ok_or_else(|| "Workspace could not be loaded.".to_string())?;
    if workspace.kind != "user" {
        return Ok(false);
    }
    Ok(git_review::ensure_git_init(std::path::Path::new(
        &workspace.path,
    )))
}

#[tauri::command]
fn get_or_create_chat_workspace(
    thread_id: String,
    title: Option<String>,
) -> Result<store::WorkspaceRecord, String> {
    store::get_or_create_chat_workspace(&thread_id, title)
}

#[tauri::command]
fn get_thread(thread_id: String) -> Result<Option<store::ThreadRecord>, String> {
    store::get_thread(&thread_id)
}

#[tauri::command]
fn get_recent_thread() -> Result<Option<store::ThreadRecord>, String> {
    store::get_recent_thread()
}

#[tauri::command]
fn create_thread(input: store::CreateThreadInput) -> Result<store::ThreadRecord, String> {
    let thread = store::create_thread(input)?;
    if thread.mode == "workspace" {
        if let Ok(Some(workspace)) = store::get_workspace(&thread.workspace_id) {
            ensure_workspace_git_repo(&workspace);
        }
    }
    Ok(thread)
}

#[tauri::command]
fn rename_thread(input: store::RenameThreadInput) -> Result<store::ThreadRecord, String> {
    store::rename_thread(input)
}

#[tauri::command]
fn update_thread_model(
    input: store::UpdateThreadModelInput,
) -> Result<store::ThreadRecord, String> {
    store::update_thread_model(input)
}

#[tauri::command]
fn pin_thread(input: store::PinThreadInput) -> Result<store::ThreadRecord, String> {
    store::pin_thread(input)
}

#[tauri::command]
fn archive_thread(thread_id: String) -> Result<store::ThreadRecord, String> {
    store::archive_thread(&thread_id)
}

#[tauri::command]
fn restore_thread(thread_id: String) -> Result<store::ThreadRecord, String> {
    store::restore_thread(&thread_id)
}

#[tauri::command]
fn delete_thread(thread_id: String) -> Result<store::ThreadRecord, String> {
    store::delete_thread(&thread_id)
}

#[tauri::command]
fn get_thread_cleanup_summary(thread_id: String) -> Result<store::ThreadCleanupSummary, String> {
    store::get_thread_cleanup_summary(&thread_id)
}

#[tauri::command]
fn list_messages(thread_id: String) -> Result<Vec<store::MessageRecord>, String> {
    store::list_messages(&thread_id)
}

#[tauri::command]
fn append_message(input: store::AppendMessageInput) -> Result<store::MessageRecord, String> {
    store::append_message(input)
}

#[tauri::command]
fn create_run(input: store::CreateRunInput) -> Result<store::RunRecord, String> {
    store::create_run(input)
}

#[tauri::command]
fn list_runs(thread_id: String) -> Result<Vec<store::RunRecord>, String> {
    store::list_runs(&thread_id)
}

#[tauri::command]
fn update_run_status(input: store::UpdateRunStatusInput) -> Result<store::RunRecord, String> {
    store::update_run_status(input)
}

#[tauri::command]
async fn abort_run(thread_id: String, run_id: String) -> Result<store::RunRecord, String> {
    if let Err(error) = agent_bridge::abort_agent_thread(&thread_id).await {
        if !is_agent_unavailable_error(&error) {
            return Err(error);
        }
        eprintln!("FutureOS agent abort skipped because agent is unavailable: {error}");
    }
    store::update_run_status(store::UpdateRunStatusInput {
        run_id,
        status: "cancelled".to_string(),
        error_message: Some("Terminated by user.".to_string()),
        error_type: Some("abort_requested".to_string()),
    })
}

#[tauri::command]
fn list_run_events(run_id: String) -> Result<Vec<store::RunEventRecord>, String> {
    store::list_run_events(&run_id)
}

#[tauri::command]
fn list_tool_calls(run_id: String) -> Result<Vec<store::ToolCallRecord>, String> {
    store::list_tool_calls(&run_id)
}

#[tauri::command]
fn list_tool_outputs(tool_call_id: String) -> Result<Vec<store::ToolOutputRecord>, String> {
    store::list_tool_outputs(&tool_call_id)
}

#[tauri::command]
fn list_approval_requests(thread_id: String) -> Result<Vec<store::ApprovalRequestRecord>, String> {
    store::list_approval_requests(&thread_id)
}

#[tauri::command]
async fn decide_approval_request(
    input: store::DecideApprovalRequestInput,
) -> Result<store::ApprovalRequestRecord, String> {
    let current = store::get_approval_request(&input.approval_request_id)?
        .ok_or_else(|| "Approval request could not be loaded.".to_string())?;
    if current.status == "pending" {
        if let Err(error) = agent_bridge::notify_agent_approval_decision(&current, &input).await {
            if is_stale_approval_error(&error) {
                return store::decide_approval_request(store::DecideApprovalRequestInput {
                    approval_request_id: input.approval_request_id,
                    status: "cancelled".to_string(),
                    decision_note: Some("Cancelled because the approval request is no longer active in Future Agent.".to_string()),
                });
            }
            return Err(error);
        }
    }
    let updated = store::decide_approval_request(input)?;
    if let Some(run_id) = &updated.run_id {
        if let Ok(Some(run)) = store::get_run(run_id) {
            if !matches!(run.status.as_str(), "completed" | "failed" | "cancelled") {
                let _ = store::update_run_status(store::UpdateRunStatusInput {
                    run_id: run_id.clone(),
                    status: "running".to_string(),
                    error_message: None,
                    error_type: None,
                });
            }
        }
    }
    Ok(updated)
}

fn is_stale_approval_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("approval request") && normalized.contains("not pending")
}

fn is_agent_unavailable_error(error: &str) -> bool {
    error.starts_with("Unable to connect to Future Agent")
}

#[tauri::command]
fn list_review_changesets(thread_id: String) -> Result<Vec<store::ReviewChangesetRecord>, String> {
    store::list_review_changesets(&thread_id)
}

#[tauri::command]
fn update_review_changeset_status(
    input: store::UpdateReviewChangesetStatusInput,
) -> Result<store::ReviewChangesetRecord, String> {
    store::update_review_changeset_status(input)
}

#[tauri::command]
fn list_review_file_changes(
    changeset_id: String,
) -> Result<Vec<store::ReviewFileChangeRecord>, String> {
    store::list_review_file_changes(&changeset_id)
}

#[tauri::command]
fn get_git_review(
    workspace_id: String,
    base: Option<String>,
    custom_base: Option<String>,
) -> Result<git_review::GitReview, String> {
    git_review::get_git_review(workspace_id, base, custom_base)
}

#[tauri::command]
fn list_artifacts(thread_id: String) -> Result<Vec<store::ArtifactRecord>, String> {
    store::list_artifacts(&thread_id)
}

#[tauri::command]
fn create_artifact(input: store::CreateArtifactInput) -> Result<store::ArtifactRecord, String> {
    store::create_artifact(input)
}

#[tauri::command]
fn import_attachment_artifact(
    input: store::ImportAttachmentArtifactInput,
) -> Result<store::ArtifactRecord, String> {
    store::import_attachment_artifact(input)
}

#[tauri::command]
fn delete_artifact(artifact_id: String) -> Result<store::ArtifactRecord, String> {
    store::delete_artifact(&artifact_id)
}

#[tauri::command]
fn promote_artifact_to_research(
    artifact_id: String,
) -> Result<store::ResearchResourceRecord, String> {
    store::promote_artifact_to_research(&artifact_id)
}

#[tauri::command]
fn list_research_resources(
    workspace_id: String,
) -> Result<Vec<store::ResearchResourceRecord>, String> {
    store::list_research_resources(&workspace_id)
}

#[tauri::command]
fn resolve_markdown_references(
    input: store::ResolveMarkdownReferencesInput,
) -> Result<Vec<store::ResolvedMarkdownReference>, String> {
    store::resolve_markdown_references(input)
}

#[tauri::command]
fn search_reference_targets(
    input: store::SearchReferenceTargetsInput,
) -> Result<Vec<store::ReferenceTargetSearchResult>, String> {
    store::search_reference_targets(input)
}

#[tauri::command]
async fn list_agent_models() -> Result<Vec<agent_bridge::AgentModelOption>, String> {
    agent_bridge::list_agent_models().await
}

#[tauri::command]
async fn agent_prompt(
    message: String,
    image_paths: Option<Vec<String>>,
    thread_id: String,
    session_id: Option<String>,
    run_id: Option<String>,
    model_id: Option<String>,
    thinking_level: Option<String>,
) -> Result<agent_bridge::AgentPromptResponse, String> {
    agent_bridge::agent_prompt(
        message,
        image_paths,
        thread_id,
        session_id,
        run_id,
        model_id,
        thinking_level,
    )
    .await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|_| {
            if let Err(error) = store::initialize_app_store() {
                eprintln!("FutureOS store initialization failed: {error}");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_version,
            app_data_path,
            open_path,
            read_text_file_preview,
            export_artifact_file,
            initialize_app_store,
            cancel_stale_approval_requests,
            get_app_settings,
            update_app_settings,
            list_agent_providers,
            upsert_custom_provider,
            delete_custom_provider,
            clear_finished_runs,
            list_threads,
            list_workspaces,
            create_workspace,
            ensure_workspace_git,
            save_pasted_image,
            get_or_create_chat_workspace,
            get_thread,
            get_recent_thread,
            create_thread,
            rename_thread,
            update_thread_model,
            pin_thread,
            archive_thread,
            restore_thread,
            delete_thread,
            get_thread_cleanup_summary,
            list_messages,
            append_message,
            create_run,
            list_runs,
            update_run_status,
            abort_run,
            list_run_events,
            list_tool_calls,
            list_tool_outputs,
            list_approval_requests,
            decide_approval_request,
            list_review_changesets,
            update_review_changeset_status,
            list_review_file_changes,
            get_git_review,
            list_artifacts,
            create_artifact,
            import_attachment_artifact,
            delete_artifact,
            promote_artifact_to_research,
            list_research_resources,
            resolve_markdown_references,
            search_reference_targets,
            list_agent_models,
            agent_prompt
        ])
        .run(tauri::generate_context!())
        .expect("error while running FutureOS");
}
