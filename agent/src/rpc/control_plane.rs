//! Control-plane events and revision belong to one Agent service instance.
use super::{SseBroadcaster, SseEvent};
use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc,
};

pub struct ControlPlane {
    broadcaster: Arc<SseBroadcaster>,
    revision: AtomicI64,
    pub(crate) provider_command_lock: std::sync::Mutex<()>,
}
impl Default for ControlPlane {
    fn default() -> Self {
        Self {
            broadcaster: Arc::new(SseBroadcaster::new()),
            revision: AtomicI64::new(0),
            provider_command_lock: std::sync::Mutex::new(()),
        }
    }
}
impl ControlPlane {
    pub fn global_events_broadcaster(&self) -> Arc<SseBroadcaster> {
        self.broadcaster.clone()
    }

    /// Config-only alias on this explicit service instance.
    #[deprecated(note = "use global_events_broadcaster")]
    pub fn global_config_broadcaster(&self) -> Arc<SseBroadcaster> {
        self.global_events_broadcaster()
    }

    pub fn current_config_revision(&self) -> i64 {
        self.revision.load(Ordering::Acquire)
    }

    /// Publish only after the mutation is durable and live Agent state refreshed.
    pub fn publish_provider_config_changed(
        &self,
        provider_id: &str,
        operation: &str,
        auth_changed: bool,
        models_changed: bool,
    ) -> i64 {
        let revision = self.revision.fetch_add(1, Ordering::AcqRel) + 1;
        self.broadcaster.broadcast(SseEvent::new(
            "provider_config_changed",
            serde_json::json!({
                "revision": revision,
                "providerId": provider_id,
                "operation": operation,
                "authChanged": auth_changed,
                "modelsChanged": models_changed,
            }),
        ));
        revision
    }

    /// Announce a newly minted session on the global control-plane stream. Fired
    /// after a new session is durably created and activated (never during ordinary
    /// disk hydration) so other clients can surface it without waiting for their
    /// discovery polls. An idempotent hint: consumers reconcile against their own
    /// state, so no revision counter.
    pub fn publish_session_created(&self, session_id: &str, created_by: &str, cwd: &str) {
        self.publish_session_created_with_creator(session_id, created_by, "", cwd);
    }

    /// Creator-aware session announcement. The three-argument wrapper remains for
    /// source compatibility and emits an empty creator id (legacy semantics).
    pub fn publish_session_created_with_creator(
        &self,
        session_id: &str,
        created_by: &str,
        creator_id: &str,
        cwd: &str,
    ) {
        self.broadcaster.broadcast(SseEvent::new(
            "session_created",
            serde_json::json!({
                "sessionId": session_id,
                "createdBy": created_by,
                "creatorId": creator_id,
                "cwd": cwd,
            }),
        ));
    }

    /// Announce that a session no longer exists, so clients holding a mirror of
    /// it (the desktop's conversation list, a paired phone) can drop their copy
    /// immediately instead of showing a conversation the Agent cannot load. Fired
    /// by `delete_session` only after the session is gone from memory and disk —
    /// there is nothing to reconcile against afterwards, so the payload carries
    /// just the identity.
    pub fn publish_session_deleted(&self, session_id: &str) {
        self.broadcaster.broadcast(SseEvent::new(
            "session_deleted",
            serde_json::json!({"sessionId": session_id}),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn revisions_and_subscriptions_are_isolated_between_service_instances() {
        let first = ControlPlane::default();
        let second = ControlPlane::default();
        let mut old = first.global_events_broadcaster().subscribe();
        let mut new = second.global_events_broadcaster().subscribe();
        assert_eq!(
            first.publish_provider_config_changed("first", "updated", true, true),
            1
        );
        assert_eq!(second.current_config_revision(), 0);
        assert!(old.try_recv().unwrap().data.contains("first"));
        assert!(matches!(
            new.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
        assert_eq!(
            second.publish_provider_config_changed("second", "updated", true, true),
            1
        );
        assert!(new.try_recv().unwrap().data.contains("second"));
        assert!(matches!(
            old.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
        drop(first);
        assert!(matches!(
            old.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Closed)
        ));
    }
}
