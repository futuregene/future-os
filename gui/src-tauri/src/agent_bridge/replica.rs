use std::{
    collections::{HashMap, HashSet},
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

/// How long a released run still counts as "owned" for observer purposes.
/// The collector persists `agent_end` BEFORE its lease drops, but a session
/// observer on an independent subscription may see that same event a few
/// milliseconds later — without this grace window it would treat the run as
/// unowned and persist a duplicate terminal event.
const RECENTLY_RELEASED_WINDOW: Duration = Duration::from_secs(60);

#[derive(Default)]
struct ReplicaRegistry {
    active_runs: HashSet<String>,
    local_to_canonical: HashMap<String, String>,
    /// canonical run id → when its lease was released (pruned on read).
    released: HashMap<String, Instant>,
}

/// Process-local owner registry for canonical Agent runs.
///
/// A run may have many downstream consumers, but only one collector is allowed
/// to project its upstream event stream into local storage.
pub(super) struct AgentReplicaManager {
    registry: Mutex<ReplicaRegistry>,
}

pub(super) static AGENT_REPLICAS: LazyLock<AgentReplicaManager> =
    LazyLock::new(|| AgentReplicaManager {
        registry: Mutex::new(ReplicaRegistry::default()),
    });

impl AgentReplicaManager {
    pub(super) fn acquire(&'static self, run_id: &str) -> Result<ReplicaLease, String> {
        let mut registry = self
            .registry
            .lock()
            .map_err(|_| "Agent replica registry lock poisoned".to_string())?;
        if !registry.active_runs.insert(run_id.to_string()) {
            return Err(format!(
                "an upstream collector already owns Agent run {run_id}"
            ));
        }
        Ok(ReplicaLease {
            manager: self,
            canonical_run_id: run_id.to_string(),
            local_run_id: None,
        })
    }

    /// Resolve the Agent run that a local SQLite run is currently projecting.
    /// This lets a late abort carry the identity selected by the user instead
    /// of querying whichever run happens to be active a moment later.
    pub(super) fn canonical_for_local(&self, local_run_id: &str) -> Option<String> {
        self.registry
            .lock()
            .ok()?
            .local_to_canonical
            .get(local_run_id)
            .cloned()
    }

    /// Whether a prompt-pipeline collector currently owns this canonical run
    /// or released it within the grace window. Session observers consult this
    /// before projecting a run: exactly one writer per run (the pipeline
    /// collector wins; the observer takes over runs nobody else owns).
    pub(super) fn is_owned_or_recently_released(&self, run_id: &str) -> bool {
        let Ok(mut registry) = self.registry.lock() else {
            return false;
        };
        if registry.active_runs.contains(run_id) {
            return true;
        }
        registry
            .released
            .retain(|_, at| at.elapsed() < RECENTLY_RELEASED_WINDOW);
        registry.released.contains_key(run_id)
    }
}

pub(super) struct ReplicaLease {
    manager: &'static AgentReplicaManager,
    canonical_run_id: String,
    local_run_id: Option<String>,
}

impl ReplicaLease {
    pub(super) fn bind_local(mut self, local_run_id: Option<&str>) -> Result<Self, String> {
        let Some(local_run_id) = local_run_id.filter(|run_id| !run_id.is_empty()) else {
            return Ok(self);
        };
        let mut registry = self
            .manager
            .registry
            .lock()
            .map_err(|_| "Agent replica registry lock poisoned".to_string())?;
        registry
            .local_to_canonical
            .insert(local_run_id.to_string(), self.canonical_run_id.clone());
        self.local_run_id = Some(local_run_id.to_string());
        drop(registry);
        Ok(self)
    }

    /// Consume the ownership lease while collecting one canonical Agent run.
    /// Binding and upstream lifetime are deliberately coupled here: callers
    /// cannot start a collector without registering the local projection, and
    /// dropping/finishing the future releases both atomically.
    pub(super) async fn collect(
        self,
        local_run_id: Option<&str>,
        canonical_run_id: &str,
        session_id: &str,
        thread_id: &str,
    ) -> Result<super::stream::AgentResponse, super::stream::CollectError> {
        if self.canonical_run_id != canonical_run_id {
            return Err(format!(
                "replica lease owns {}, cannot collect {canonical_run_id}",
                self.canonical_run_id
            )
            .into());
        }
        let lease = self
            .bind_local(local_run_id)
            .map_err(crate::AppError::from)?;
        let response = super::stream::collect_agent_response(
            local_run_id,
            canonical_run_id,
            session_id,
            thread_id,
        )
        .await;
        drop(lease);
        response
    }
}

impl Drop for ReplicaLease {
    fn drop(&mut self) {
        if let Ok(mut registry) = self.manager.registry.lock() {
            registry.active_runs.remove(&self.canonical_run_id);
            registry
                .released
                .insert(self.canonical_run_id.clone(), Instant::now());
            if let Some(local_run_id) = self.local_run_id.as_deref() {
                registry.local_to_canonical.remove(local_run_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentReplicaManager, ReplicaRegistry};
    use std::sync::Mutex;

    #[test]
    fn one_upstream_per_canonical_run() {
        let manager = Box::leak(Box::new(AgentReplicaManager {
            registry: Mutex::new(ReplicaRegistry::default()),
        }));
        let first = manager.acquire("run-a").unwrap();
        assert!(manager.acquire("run-a").is_err());
        drop(first);
        assert!(manager.acquire("run-a").is_ok());
    }

    #[test]
    fn local_binding_lives_exactly_as_long_as_replica_lease() {
        let manager = Box::leak(Box::new(AgentReplicaManager {
            registry: Mutex::new(ReplicaRegistry::default()),
        }));
        let lease = manager
            .acquire("agent-run")
            .unwrap()
            .bind_local(Some("sqlite-run"))
            .unwrap();
        assert_eq!(
            manager.canonical_for_local("sqlite-run").as_deref(),
            Some("agent-run")
        );
        drop(lease);
        assert!(manager.canonical_for_local("sqlite-run").is_none());
    }

    #[test]
    fn recently_released_run_still_counts_as_owned() {
        let manager = Box::leak(Box::new(AgentReplicaManager {
            registry: Mutex::new(ReplicaRegistry::default()),
        }));
        {
            let _lease = manager.acquire("run-a").unwrap();
            assert!(manager.is_owned_or_recently_released("run-a"));
        }
        // Lease dropped — the grace window keeps the run "owned" so an observer
        // seeing the terminal events a moment later doesn't double-persist.
        assert!(manager.is_owned_or_recently_released("run-a"));
        assert!(!manager.is_owned_or_recently_released("run-b"));
    }

    #[tokio::test]
    async fn rejected_collection_releases_ownership() {
        let manager = Box::leak(Box::new(AgentReplicaManager {
            registry: Mutex::new(ReplicaRegistry::default()),
        }));
        let lease = manager.acquire("run-a").unwrap();
        assert!(lease
            .collect(None, "run-b", "session", "thread")
            .await
            .is_err());
        assert!(manager.acquire("run-a").is_ok());
    }
}
