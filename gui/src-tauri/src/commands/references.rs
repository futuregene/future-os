//! Markdown reference resolution and `@`-mention search Tauri commands.

use crate::store;

#[tauri::command]
pub fn resolve_markdown_references(
    input: store::ResolveMarkdownReferencesInput,
) -> Result<Vec<store::ResolvedMarkdownReference>, crate::AppError> {
    store::resolve_markdown_references(input)
}

#[tauri::command]
pub fn search_reference_targets(
    input: store::SearchReferenceTargetsInput,
) -> Result<Vec<store::ReferenceTargetSearchResult>, crate::AppError> {
    store::search_reference_targets(input)
}
