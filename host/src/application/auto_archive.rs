//! Daily, opt-in archiving of conversation trees whose latest meaningful
//! activity is older than the configured idle period.

use std::collections::HashSet;
use std::sync::{LazyLock, RwLock};

use crate::store;

static FOREGROUND_THREAD_ID: LazyLock<RwLock<Option<String>>> = LazyLock::new(|| RwLock::new(None));

/// The desktop's actual foreground conversation, not merely `last_opened_at`.
/// Headless hosts have no foreground row and therefore skip nothing here.
pub fn set_foreground_thread(thread_id: Option<String>) {
    *FOREGROUND_THREAD_ID
        .write()
        .unwrap_or_else(|poison| poison.into_inner()) = thread_id;
}

fn foreground_thread_id() -> Option<String> {
    FOREGROUND_THREAD_ID
        .read()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone()
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AutoArchiveReport {
    pub archived_trees: usize,
    pub purged_sessions: usize,
    pub failed_cleanups: usize,
}

fn activity_at(thread: &store::ThreadRecord) -> i64 {
    thread
        .last_message_at
        .unwrap_or(thread.created_at)
        .max(thread.restored_at.unwrap_or(0))
}

fn candidate_tree_ids(
    threads: &[store::ThreadRecord],
    streaming: &HashSet<String>,
    foreground: Option<&str>,
    now: i64,
    idle_days: u32,
) -> Result<Vec<String>, crate::Error> {
    let idle_ms = i64::from(idle_days) * 24 * 60 * 60 * 1000;
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for thread in threads.iter().filter(|thread| thread.status == "active") {
        let members = store::thread_archive_closure(&thread.id)?;
        let Some(key) = members.iter().map(|member| member.id.clone()).min() else {
            continue;
        };
        if !seen.insert(key.clone()) {
            continue;
        }
        if members.iter().any(|member| {
            member.pinned
                || foreground == Some(member.id.as_str())
                || streaming.contains(&member.id)
        }) {
            continue;
        }
        let mut active = false;
        for member in &members {
            if store::has_active_run(&member.id)? {
                active = true;
                break;
            }
        }
        if active {
            continue;
        }
        let activity = members.iter().map(activity_at).max().unwrap_or(0);
        if activity > now || now - activity < idle_ms {
            continue;
        }
        candidates.push(key);
    }
    Ok(candidates)
}

/// Execute one automatic pass. Every candidate is revalidated by the store's
/// normal archive precondition immediately before mutation.
pub async fn auto_archive_now() -> Result<AutoArchiveReport, crate::Error> {
    let mut report = AutoArchiveReport::default();
    let settings = store::get_app_settings()?;
    if !settings.auto_archive_enabled {
        return Ok(report);
    }
    let now = store::now_millis();
    let streaming: HashSet<String> = crate::agent_bridge::reconcile_streaming_observers()
        .await
        .unwrap_or_default()
        .into_iter()
        .collect();
    let foreground = foreground_thread_id();
    let candidates = candidate_tree_ids(
        &store::list_threads()?,
        &streaming,
        foreground.as_deref(),
        now,
        settings.auto_archive_idle_days,
    )?;
    for thread_id in candidates {
        let current_streaming: HashSet<String> =
            crate::agent_bridge::reconcile_streaming_observers()
                .await
                .unwrap_or_default()
                .into_iter()
                .collect();
        let still_candidate = candidate_tree_ids(
            &store::list_threads()?,
            &current_streaming,
            foreground.as_deref(),
            store::now_millis(),
            settings.auto_archive_idle_days,
        )?
        .contains(&thread_id);
        if !still_candidate {
            continue;
        }
        let streaming_ids: Vec<String> = current_streaming.into_iter().collect();
        match store::archive_thread_tree(&thread_id, &streaming_ids) {
            Ok(_) => {
                report.archived_trees += 1;
                if settings.archive_run_log_cleanup {
                    let cleanup = super::archive_cleanup::thread_tree_if_enabled(&thread_id).await;
                    report.purged_sessions += cleanup.purged_sessions;
                    report.failed_cleanups += cleanup.failed_sessions;
                }
            }
            Err(error) => {
                // A new run, pin or restore since candidate selection is a
                // normal skip, not a failed scan and not a user notification.
                tracing::debug!(event = "auto_archive_candidate_skipped", thread_id, %error);
            }
        }
    }
    if report.archived_trees > 0 {
        crate::events::emit_threads_updated();
        crate::events::notify(
            "auto-archive-completed",
            serde_json::json!({
                "count": report.archived_trees,
                "idleDays": settings.auto_archive_idle_days,
                "cleanupEnabled": settings.archive_run_log_cleanup,
                "purgedSessions": report.purged_sessions,
                "failedCleanups": report.failed_cleanups,
            }),
        );
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(id: &str, activity: i64) -> store::ThreadRecord {
        store::ThreadRecord {
            id: id.into(),
            workspace_id: "w".into(),
            mode: "chat".into(),
            title: id.into(),
            title_seed: None,
            status: "active".into(),
            pinned: false,
            readonly: false,
            agent_session_id: Some(id.into()),
            parent_session_id: None,
            asset_root_id: None,
            last_message_at: Some(activity),
            last_opened_at: None,
            created_at: activity,
            updated_at: activity,
            archived_at: None,
            restored_at: None,
            deleted_at: None,
        }
    }

    #[test]
    fn restore_starts_a_fresh_idle_period() {
        let mut restored = thread("t", 1_000);
        restored.restored_at = Some(9_000);
        assert_eq!(activity_at(&restored), 9_000);
    }

    #[test]
    fn empty_conversations_fall_back_to_creation_not_metadata_update() {
        let mut empty = thread("t", 1_000);
        empty.last_message_at = None;
        empty.updated_at = 99_000;
        assert_eq!(activity_at(&empty), 1_000);
    }

    #[tokio::test]
    async fn only_the_expired_conversations_in_a_workspace_are_candidates() {
        let _home = crate::auth_store::test_support::HomeGuard::new("auto-archive-six");
        crate::store::initialize_app_store().unwrap();
        let workspace = crate::store::create_workspace(crate::store::CreateWorkspaceInput {
            name: Some("workspace".into()),
            path: std::env::temp_dir()
                .join("future-auto-archive-six")
                .display()
                .to_string(),
            description: None,
            create_directory: Some(true),
        })
        .unwrap();
        let now = crate::store::now_millis();
        let mut ids = Vec::new();
        for index in 0..6 {
            let session_id = format!("thread-{index}");
            let created = crate::store::create_thread(crate::store::CreateThreadInput {
                mode: "workspace".into(),
                title: Some(session_id.clone()),
                workspace_id: Some(workspace.id.clone()),
                workspace_path: None,
                workspace_name: None,
                agent_session_id: Some(session_id),
            })
            .unwrap();
            let activity = if index < 2 {
                now - 31 * 24 * 60 * 60 * 1000
            } else {
                now
            };
            crate::store::record_thread_message_activity(&created.id, activity).unwrap();
            ids.push(created.id);
        }

        let candidates = candidate_tree_ids(
            &crate::store::list_threads().unwrap(),
            &HashSet::new(),
            None,
            now,
            30,
        )
        .unwrap();
        assert_eq!(candidates, vec![ids[0].clone(), ids[1].clone()]);

        crate::store::update_app_settings(crate::store::UpdateAppSettingsInput {
            auto_archive_enabled: Some(true),
            ..Default::default()
        })
        .unwrap();
        let mock = crate::agent_bridge::test_support::mock_agent();
        for _ in 0..3 {
            mock.push_data(
                "list_streaming_sessions",
                serde_json::json!({"sessionIds": []}),
            );
        }
        let report = auto_archive_now().await.unwrap();
        assert_eq!(report.archived_trees, 2);
        for (index, id) in ids.iter().enumerate() {
            let thread = crate::store::get_thread(id).unwrap().unwrap();
            if index < 2 {
                assert_eq!(thread.status, "archived");
            } else {
                assert_eq!(thread.status, "active");
            }
        }
    }

    #[tokio::test]
    async fn a_pinned_conversation_is_never_a_candidate() {
        let _home = crate::auth_store::test_support::HomeGuard::new("auto-archive-pinned");
        crate::store::initialize_app_store().unwrap();
        let now = crate::store::now_millis();
        let long_idle = now - 31 * 24 * 60 * 60 * 1000;
        let pinned = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".into(),
            title: Some("pinned".into()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some("pinned-session".into()),
        })
        .unwrap();
        crate::store::record_thread_message_activity(&pinned.id, long_idle).unwrap();
        crate::store::pin_thread(crate::store::PinThreadInput {
            thread_id: pinned.id.clone(),
            pinned: true,
        })
        .unwrap();
        let idle = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".into(),
            title: Some("idle".into()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some("idle-session".into()),
        })
        .unwrap();
        crate::store::record_thread_message_activity(&idle.id, long_idle).unwrap();

        let candidates = candidate_tree_ids(
            &crate::store::list_threads().unwrap(),
            &HashSet::new(),
            None,
            now,
            30,
        )
        .unwrap();
        // Pinning keeps a conversation out of automatic archiving even after it
        // has been idle far past the threshold.
        assert_eq!(candidates, vec![idle.id]);
    }

    #[tokio::test]
    async fn automatic_archiving_stays_off_until_it_is_enabled() {
        let _home = crate::auth_store::test_support::HomeGuard::new("auto-archive-default-off");
        crate::store::initialize_app_store().unwrap();
        let now = crate::store::now_millis();
        let idle = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".into(),
            title: Some("idle".into()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some("default-off-session".into()),
        })
        .unwrap();
        crate::store::record_thread_message_activity(&idle.id, now - 31 * 24 * 60 * 60 * 1000)
            .unwrap();

        // The shipped default is off, so a pass archives nothing at all.
        assert!(
            !crate::store::get_app_settings()
                .unwrap()
                .auto_archive_enabled
        );
        let mock = crate::agent_bridge::test_support::mock_agent();
        mock.push_data(
            "list_streaming_sessions",
            serde_json::json!({"sessionIds": []}),
        );
        let report = auto_archive_now().await.unwrap();
        assert_eq!(report.archived_trees, 0);
        assert_eq!(
            crate::store::get_thread(&idle.id).unwrap().unwrap().status,
            "active"
        );
    }
}
