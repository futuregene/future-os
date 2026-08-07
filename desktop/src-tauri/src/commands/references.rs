//! Markdown reference resolution and the `@`-mention file-search Tauri command.

use crate::store;

#[tauri::command]
pub fn resolve_markdown_references(
    input: store::ResolveMarkdownReferencesInput,
) -> Result<Vec<store::ResolvedMarkdownReference>, crate::AppError> {
    store::resolve_markdown_references(input)
}

#[tauri::command]
pub fn search_workspace_files(
    input: store::WorkspaceFileSearchInput,
) -> Result<Vec<store::WorkspaceFileResult>, crate::AppError> {
    store::search_workspace_files(input)
}
