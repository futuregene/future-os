//! Per-conversation retention after a Run finalizes. Historical review recovery
//! belongs to the requested conversation, never a process-wide startup sweep.

use std::collections::HashSet;

use crate::store;
use crate::Error;

use super::repository::ShadowRepo;

/// Run changesets kept per Thread (§12.3).
const RETENTION_KEEP: usize = 10;

/// Prune a Thread's old run changesets, delete their shadow refs, and let git
/// gc when warranted. Called after each Run finalizes.
pub fn enforce_retention(thread_id: &str) {
    if let Err(error) = try_enforce_retention(thread_id) {
        tracing::error!(
            target: "future_host::shadow_review",
            thread_id, error = %error,
            "retention failed"
        );
    }
}

fn try_enforce_retention(thread_id: &str) -> Result<(), Error> {
    let pruned = store::prune_thread_changesets(thread_id, RETENTION_KEEP)?;
    if pruned.is_empty() {
        return Ok(());
    }

    let mut workspaces: HashSet<String> = HashSet::new();
    for (workspace_id, run_id) in &pruned {
        workspaces.insert(workspace_id.clone());
        if let Ok(repo) = ShadowRepo::open_bare(workspace_id) {
            let _ = repo.delete_ref(&ShadowRepo::snapshot_ref(thread_id, run_id, "before"));
            let _ = repo.delete_ref(&ShadowRepo::snapshot_ref(thread_id, run_id, "after"));
        }
    }
    for workspace_id in workspaces {
        if let Ok(repo) = ShadowRepo::open_bare(&workspace_id) {
            repo.gc_auto();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::auth_store::test_support::HomeGuard;
    use crate::store::{
        CreateRunInput, CreateThreadInput, CreateWorkspaceInput, UpsertRunChangesetInput,
    };

    struct Setup {
        _home: HomeGuard,
        workspace_id: String,
        thread_id: String,
    }

    fn setup(name: &str) -> Setup {
        let home = HomeGuard::new(name);
        store::initialize_app_store().unwrap();
        let dir =
            std::env::temp_dir().join(format!("futureos-maint-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let ws = store::create_workspace(CreateWorkspaceInput {
            name: Some(name.into()),
            path: dir.display().to_string(),
            description: None,
            create_directory: Some(false),
        })
        .unwrap();
        let thread = store::create_thread(CreateThreadInput {
            mode: "workspace".into(),
            title: Some(name.into()),
            workspace_id: Some(ws.id.clone()),
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .unwrap();
        Setup {
            _home: home,
            workspace_id: ws.id,
            thread_id: thread.id,
        }
    }

    #[test]
    fn wrappers_log_errors_when_store_uninitialized() {
        let _home = HomeGuard::new("maintenance-uninit");
        // No initialize_app_store: the empty DB has no tables, so each
        // best-effort entry point fails and logs instead of panicking.
        enforce_retention("thread-1");
    }

    #[test]
    fn retention_returns_early_when_nothing_to_prune() {
        let s = setup("retention-empty");
        try_enforce_retention(&s.thread_id).unwrap();
    }

    #[test]
    fn retention_prunes_and_deletes_refs() {
        let s = setup("retention-prune");
        for _ in 0..11 {
            let run = store::create_run(CreateRunInput {
                id: None,
                thread_id: s.thread_id.clone(),
                trigger_message_id: None,
                model_provider: None,
                model_id: None,
            })
            .unwrap();
            store::upsert_run_changeset(UpsertRunChangesetInput {
                run_id: run.id.clone(),
                thread_id: s.thread_id.clone(),
                workspace_id: Some(s.workspace_id.clone()),
                title: "t".into(),
                completeness: "complete".into(),
                confidence: "normal".into(),
                ..Default::default()
            })
            .unwrap();
        }
        try_enforce_retention(&s.thread_id).unwrap();
    }
}
