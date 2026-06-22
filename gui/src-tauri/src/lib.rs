mod agent_bridge;
mod agent_proto;
mod agent_providers;
mod commands;
mod error;
mod git_review;
mod run_error;
mod store;

use commands::*;
use error::AppError;

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
