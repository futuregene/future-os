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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::test_support::HomeGuard;

    /// Fresh fake HOME + schema, then a user workspace + a workspace-mode
    /// thread. Returns the guard (must outlive the store pool) and the thread.
    fn seeded(label: &str) -> (HomeGuard, store::ThreadRecord) {
        let home = HomeGuard::new(label);
        crate::store::initialize_app_store().expect("init store");
        let ws = crate::store::create_workspace(store::CreateWorkspaceInput {
            name: Some("WS".into()),
            path: std::env::temp_dir()
                .join(format!("futureos-cmd-art-ws-{}", std::process::id()))
                .display()
                .to_string(),
            description: None,
            create_directory: Some(true),
        })
        .expect("create workspace");
        let thread = crate::store::create_thread(store::CreateThreadInput {
            mode: "workspace".into(),
            title: Some("Artifacts".into()),
            workspace_id: Some(ws.id.clone()),
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .expect("create thread");
        (home, thread)
    }

    fn artifact_input(thread: &store::ThreadRecord) -> store::CreateArtifactInput {
        store::CreateArtifactInput {
            workspace_id: thread.workspace_id.clone(),
            thread_id: Some(thread.id.clone()),
            run_id: None,
            title: "report.md".into(),
            artifact_type: "document".into(),
            path: Some("/ws/report.md".into()),
            content: Some("# hi".into()),
            content_storage: None,
            summary: Some("summary".into()),
        }
    }

    #[test]
    fn create_list_and_delete_round_trip() {
        let (_home, thread) = seeded("cmd_artifacts");
        let created = create_artifact(artifact_input(&thread)).expect("create artifact");
        assert_eq!(created.thread_id.as_deref(), Some(thread.id.as_str()));

        let listed = list_artifacts(thread.id.clone()).expect("list artifacts");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);

        let deleted = delete_artifact(created.id.clone()).expect("delete artifact");
        assert_eq!(deleted.id, created.id);
        assert!(list_artifacts(thread.id)
            .expect("list after delete")
            .is_empty());
    }

    #[test]
    fn import_attachment_artifact_delegates() {
        let home = HomeGuard::new("cmd_import_artifact");
        crate::store::initialize_app_store().expect("init store");
        let thread = crate::store::create_thread(store::CreateThreadInput {
            mode: "chat".into(),
            title: Some("Chat".into()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .expect("create chat thread");

        let source =
            std::env::temp_dir().join(format!("futureos-cmd-import-{}.png", std::process::id()));
        image::RgbImage::from_pixel(1, 1, image::Rgb([9, 9, 9]))
            .save(&source)
            .expect("write source image");

        let imported = import_attachment_artifact(store::ImportAttachmentArtifactInput {
            thread_id: thread.id.clone(),
            path: source.display().to_string(),
        })
        .expect("import attachment artifact");
        assert_eq!(imported.thread_id.as_deref(), Some(thread.id.as_str()));
        assert_eq!(imported.content_storage.as_deref(), Some("file"));
        let _ = std::fs::remove_file(&source);
        drop(home);
    }
}
