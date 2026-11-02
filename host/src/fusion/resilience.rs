//! Run-local scheduling, bounded health caches, and durable public evidence.
use anyhow::{ensure, Result};
use future_fusion::{
    bounded_text, failure, failure_kind, Configuration, Consultation, ExecutionReport, FailureKind,
    ModelConfig, Step, Store,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

static HEALTH: LazyLock<Mutex<BTreeMap<String, Instant>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
static COOLDOWN: LazyLock<Mutex<BTreeMap<String, (Instant, FailureKind)>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

pub(super) struct Runtime<'a> {
    pub store: &'a Store,
    pub owner: &'a str,
    pub run: &'a Consultation,
    pub config: &'a Configuration,
    pub scope: String,
    pub health_key: String,
    report: Mutex<ExecutionReport>,
    transport_failures: Mutex<BTreeMap<String, BTreeSet<String>>>,
    permits: BTreeMap<String, Arc<Semaphore>>,
}
impl<'a> Runtime<'a> {
    pub fn new(
        store: &'a Store,
        owner: &'a str,
        run: &'a Consultation,
        scope: String,
    ) -> Result<Self> {
        let config = run
            .configuration
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Missing Fusion configuration"))?;
        let health_key = format!("{scope}:{}", serde_json::to_string(&config.moderator)?);
        let permits = config
            .panel
            .iter()
            .chain([&config.moderator])
            .map(|model| {
                (
                    provider(&model.model).to_owned(),
                    Arc::new(Semaphore::new(2)),
                )
            })
            .collect();
        Ok(Self {
            store,
            owner,
            run,
            config,
            scope,
            health_key,
            report: Mutex::new(run.report.clone().unwrap_or_default()),
            transport_failures: Mutex::new(BTreeMap::new()),
            permits,
        })
    }
    pub fn update(&self, f: impl FnOnce(&mut ExecutionReport)) -> Result<()> {
        let mut report = self.report.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut report);
        self.store
            .save_report(&self.run.id, Some(self.owner), &report)
    }
    pub fn note(
        &self,
        code: &str,
        phase: &str,
        participant: Option<&str>,
        kind: Option<FailureKind>,
    ) -> Result<()> {
        self.update(|report| report.note(code, phase, participant, kind))
    }
    pub fn report(&self) -> ExecutionReport {
        self.report
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn timeout(&self, phase: &str) -> Result<u64> {
        ensure!(!self.store.get(&self.run.id)?.terminal(), "Fusion stopped");
        future_fusion::phase_timeout_ms(
            self.config,
            phase,
            future_fusion::now_ms()
                .saturating_sub(self.run.created_at)
                .max(0) as u64,
        )
    }
    pub fn permit(&self, model: &str) -> Arc<Semaphore> {
        self.permits[provider(model)].clone()
    }
    fn provider_key(&self, model: &str) -> String {
        format!("{}:{}", self.scope, provider(model))
    }
    pub fn circuit(&self, model: &str) -> Option<FailureKind> {
        let mut cache = COOLDOWN.lock().unwrap_or_else(|e| e.into_inner());
        cache.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(15));
        cache.get(&self.provider_key(model)).map(|(_, kind)| *kind)
    }
    pub fn observe_failure(&self, model: &str, kind: FailureKind) -> Result<()> {
        let mut trip = kind.provider_wide();
        if kind == FailureKind::Transient {
            let mut failures = self
                .transport_failures
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let models = failures.entry(provider(model).into()).or_default();
            models.insert(model.into());
            trip = models.len() >= 2;
        }
        if trip {
            {
                let mut cache = COOLDOWN.lock().unwrap_or_else(|e| e.into_inner());
                cache.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(15));
                if cache.len() >= 64 {
                    cache.clear();
                }
                cache.insert(self.provider_key(model), (Instant::now(), kind));
            }
            self.invalidate_health();
            self.update(|report| {
                if report.degradations.len() < 24
                    && !report.degradations.iter().any(|v| {
                        v.code == "provider_unavailable"
                            && v.provider.as_deref() == Some(provider(model))
                    })
                {
                    report.degradations.push(future_fusion::Degradation {
                        code: "provider_unavailable".into(),
                        phase: "preparing".into(),
                        participant: None,
                        kind: Some(kind),
                        provider: Some(provider(model).into()),
                    });
                }
            })?;
        }
        Ok(())
    }
    pub fn healthy(&self) -> bool {
        let mut cache = HEALTH.lock().unwrap_or_else(|e| e.into_inner());
        cache.retain(|_, at| at.elapsed() < Duration::from_secs(60));
        cache.contains_key(&self.health_key) && self.circuit(&self.config.moderator.model).is_none()
    }
    pub fn mark_healthy(&self) {
        let mut cache = HEALTH.lock().unwrap_or_else(|e| e.into_inner());
        cache.retain(|_, at| at.elapsed() < Duration::from_secs(60));
        if cache.len() >= 64 {
            cache.clear();
        }
        cache.insert(self.health_key.clone(), Instant::now());
    }
    pub fn invalidate_health(&self) {
        HEALTH
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.health_key);
    }
    pub fn skip(
        &self,
        phase: &str,
        model: &ModelConfig,
        participant: &str,
        code: &str,
        kind: FailureKind,
    ) -> Result<()> {
        self.store.record_step(&Step {
            id: format!("{}-{phase}-{participant}-skipped", self.run.id),
            fusion_run_id: self.run.id.clone(),
            phase: phase.into(),
            model: model.model.clone(),
            participant: participant.into(),
            attempt: 0,
            input: String::new(),
            session_id: String::new(),
            agent_run_id: String::new(),
            status: "skipped".into(),
            output: String::new(),
            error: Some(code.into()),
            started_at: future_fusion::now_ms(),
            finished_at: Some(future_fusion::now_ms()),
            usage: serde_json::Value::Null,
            settings: None,
            failure_kind: Some(kind),
        })?;
        self.note(code, phase, Some(participant), Some(kind))
    }
    pub fn stop_error(&self, phase: &str, participant: &str, error: &anyhow::Error) -> Result<()> {
        let kind = failure_kind(error);
        self.note(
            if participant == "moderator" {
                "moderator_failed"
            } else {
                "participant_failed"
            },
            phase,
            Some(participant),
            Some(kind),
        )
    }
    pub fn retain(&self, step: &Step, text: &str) -> Result<()> {
        self.update(|report| report.retain(step, text))
    }
    pub fn ensure_provider(&self, model: &str) -> Result<()> {
        if self.circuit(model).is_some() {
            return Err(failure(
                FailureKind::CircuitOpen,
                "Fusion provider circuit is open",
            ));
        }
        Ok(())
    }
}
pub(super) fn provider(model: &str) -> &str {
    model.split_once('/').map(|v| v.0).unwrap_or(model)
}
pub(super) fn safe_error(error: &anyhow::Error) -> String {
    bounded_text(&format!("{error:#}"), 4096)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn health_and_circuit_are_scoped_and_repeated_one_model_is_not_an_outage() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let parent = future_fusion::Parent {
            database_id: "db".into(),
            session_id: "s".into(),
            run_id: "r".into(),
            tool_call_id: "t".into(),
        };
        let input = future_fusion::ConsultationInput {
            question: "choose".into(),
            goal: "decide".into(),
            background: "facts".into(),
            constraints: vec![],
            options: vec![],
            evidence: vec![],
            read_scope: vec![],
        };
        let (run, _) = store.prepare(parent, input, dir.path()).unwrap();
        store.ready(&run.id).unwrap();
        store.claim("owner").unwrap();
        let scope = uuid::Uuid::new_v4().to_string();
        let ctx = Runtime::new(&store, "owner", &run, scope.clone()).unwrap();
        assert!(!ctx.healthy());
        ctx.mark_healthy();
        assert!(ctx.healthy());
        let mut changed = run.clone();
        changed
            .configuration
            .as_mut()
            .unwrap()
            .moderator
            .thinking_level = "low".into();
        assert!(!Runtime::new(&store, "owner", &changed, scope)
            .unwrap()
            .healthy());
        let other = Runtime::new(&store, "owner", &run, uuid::Uuid::new_v4().to_string()).unwrap();
        assert!(!other.healthy());
        let first = &ctx.config.panel[0].model;
        ctx.observe_failure(first, FailureKind::Forbidden).unwrap();
        assert!(ctx.circuit(first).is_none());
        for _ in 0..2 {
            ctx.observe_failure(first, FailureKind::Transient).unwrap();
        }
        assert!(ctx.circuit(first).is_none());
        ctx.observe_failure(&ctx.config.panel[1].model, FailureKind::Transient)
            .unwrap();
        assert_eq!(ctx.circuit(first), Some(FailureKind::Transient));
        assert!(!ctx.healthy());
        assert!(other.circuit(first).is_none());
        other.observe_failure(first, FailureKind::Quota).unwrap();
        assert_eq!(other.circuit(first), Some(FailureKind::Quota));
    }
}
