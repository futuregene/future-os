//! Review changeset and git-diff Tauri commands.

use crate::{git_review, store};

#[tauri::command]
pub fn list_review_changesets(
    thread_id: String,
) -> Result<Vec<store::ReviewChangesetRecord>, crate::AppError> {
    store::list_review_changesets(&thread_id)
}

#[tauri::command]
pub fn update_review_changeset_status(
    input: store::UpdateReviewChangesetStatusInput,
) -> Result<store::ReviewChangesetRecord, crate::AppError> {
    store::update_review_changeset_status(input)
}

#[tauri::command]
pub fn list_review_file_changes(
    changeset_id: String,
) -> Result<Vec<store::ReviewFileChangeRecord>, crate::AppError> {
    store::list_review_file_changes(&changeset_id)
}

#[tauri::command]
pub fn get_git_review(
    workspace_id: String,
    base: Option<String>,
    custom_base: Option<String>,
) -> Result<git_review::GitReview, crate::AppError> {
    git_review::get_git_review(workspace_id, base, custom_base)
}
