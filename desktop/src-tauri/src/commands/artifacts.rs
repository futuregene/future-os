//! Artifact Tauri commands.

use crate::store;

#[tauri::command]
pub fn list_artifacts(thread_id: String) -> Result<Vec<store::ArtifactRecord>, crate::AppError> {
    store::list_artifacts(&thread_id)
}

#[tauri::command]
pub fn create_artifact(
    input: store::CreateArtifactInput,
) -> Result<store::ArtifactRecord, crate::AppError> {
    store::create_artifact(input)
}

#[tauri::command]
pub fn import_attachment_artifact(
    input: store::ImportAttachmentArtifactInput,
) -> Result<store::ArtifactRecord, crate::AppError> {
    store::import_attachment_artifact(input)
}

#[tauri::command]
pub fn delete_artifact(artifact_id: String) -> Result<store::ArtifactRecord, crate::AppError> {
    store::delete_artifact(&artifact_id)
}
