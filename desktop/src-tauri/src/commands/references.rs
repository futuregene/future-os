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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::test_support::HomeGuard;

    fn seeded_workspace(label: &str) -> (HomeGuard, store::WorkspaceRecord) {
        let home = HomeGuard::new(label);
        crate::store::initialize_app_store().expect("init store");
        let ws = crate::store::create_workspace(store::CreateWorkspaceInput {
            name: Some("WS".into()),
            path: std::env::temp_dir()
                .join(format!("futureos-cmd-ref-ws-{}", std::process::id()))
                .display()
                .to_string(),
            description: None,
            create_directory: Some(true),
        })
        .expect("create workspace");
        (home, ws)
    }

    #[test]
    fn resolve_references_maps_each_target_type() {
        let (_home, ws) = seeded_workspace("cmd_refs");
        // Every target type resolves to a "missing" reference on an empty
        // workspace (the store layer maps target types; unknown types are
        // reported unsupported).
        let input = store::ResolveMarkdownReferencesInput {
            workspace_id: ws.id.clone(),
            references: vec![
                store::MarkdownReferenceInput {
                    target_type: "artifact".into(),
                    target_id: "ghost".into(),
                },
                store::MarkdownReferenceInput {
                    target_type: "file".into(),
                    target_id: "ghost".into(),
                },
                store::MarkdownReferenceInput {
                    target_type: "run".into(),
                    target_id: "ghost".into(),
                },
                store::MarkdownReferenceInput {
                    target_type: "approval".into(),
                    target_id: "ghost".into(),
                },
                store::MarkdownReferenceInput {
                    target_type: "review".into(),
                    target_id: "ghost".into(),
                },
                store::MarkdownReferenceInput {
                    target_type: "bogus".into(),
                    target_id: "ghost".into(),
                },
            ],
        };
        let resolved = resolve_markdown_references(input).expect("resolve");
        assert_eq!(resolved.len(), 6);
        // Target types round-trip in order; each maps to a terminal status.
        let types: Vec<&str> = resolved.iter().map(|r| r.target_type.as_str()).collect();
        assert_eq!(
            types,
            vec!["artifact", "file", "run", "approval", "review", "bogus"]
        );
    }

    #[test]
    fn search_workspace_files_delegates_and_returns_empty_for_empty_workspace() {
        let (_home, ws) = seeded_workspace("cmd_search");
        let results = search_workspace_files(store::WorkspaceFileSearchInput {
            workspace_id: ws.id.clone(),
            query: None,
            limit: None,
        })
        .expect("search");
        assert!(results.is_empty());

        // A ghost workspace also returns empty (not an error).
        let ghost = search_workspace_files(store::WorkspaceFileSearchInput {
            workspace_id: "ghost".into(),
            query: None,
            limit: None,
        })
        .expect("ghost search");
        assert!(ghost.is_empty());
    }
}
