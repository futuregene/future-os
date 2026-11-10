//! Run-local scheduling, bounded health caches, and durable public evidence.
use anyhow::{ensure, Result};
use future_fusion::{
    bounded_text, failure, failure_kind, Configuration, Consultation, ExecutionReport, FailureKind,
    ModelConfig, Step, Store,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

static HEALTH: LazyLock<Mutex<BTreeMap<String, Instant>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
static COOLDOWN: LazyLock<Mutex<BTreeMap<String, (Instant, FailureKind)>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

#[derive(Default)]
pub(super) struct PanelDeadline(Option<tokio::time::Instant>);

impl PanelDeadline {
    pub fn participant_finished(
        &mut self,
        now: tokio::time::Instant,
        usable: bool,
        remaining: usize,
    ) {
        if usable && self.0.is_none() {
            self.0 = Some(now + Duration::from_secs(10 * 60));
        }
        if let Some(deadline) = self.0.as_mut().filter(|_| remaining == 1) {
            *deadline = (*deadline).min(now + Duration::from_secs(5 * 60));
        }
    }

    pub fn cutoff(&self) -> Option<tokio::time::Instant> {
        self.0
    }
}

pub(super) fn panel_timeout() -> anyhow::Error {
    failure(
        FailureKind::PanelTimeout,
        "Fusion stopped waiting for this participant after the panel deadline",
    )
}

pub(super) async fn wait_for_panel_timeout(stop: Option<&mut tokio::sync::watch::Receiver<bool>>) {
    match stop {
        Some(stop) => {
            let _ = stop.wait_for(|expired| *expired).await;
        }
        None => std::future::pending().await,
    }
}

pub(super) struct IdleProgress {
    last_content: Mutex<tokio::time::Instant>,
    duration: Duration,
}
impl IdleProgress {
    pub fn new(milliseconds: u64) -> Arc<Self> {
        Arc::new(Self {
            last_content: Mutex::new(tokio::time::Instant::now()),
            duration: Duration::from_millis(milliseconds),
        })
    }
    pub fn content_received(&self) {
        *self.last_content.lock().unwrap_or_else(|e| e.into_inner()) = tokio::time::Instant::now();
    }
    pub async fn guard<T>(
        &self,
        future: impl std::future::Future<Output = Result<T>>,
    ) -> Result<T> {
        tokio::pin!(future);
        loop {
            let deadline =
                *self.last_content.lock().unwrap_or_else(|e| e.into_inner()) + self.duration;
            tokio::select! {
                result = &mut future => return result,
                _ = tokio::time::sleep_until(deadline) => {
                    if self.last_content.lock().unwrap_or_else(|e| e.into_inner()).elapsed() >= self.duration {
                        return Err(failure(FailureKind::IdleTimeout, format!("[IDLE_TIMEOUT] Fusion received no new reasoning or text for {} seconds", self.duration.as_secs())));
                    }
                }
            }
        }
    }
}

pub(super) struct Runtime<'a> {
    pub store: &'a Store,
    pub owner: &'a str,
    pub run: &'a Consultation,
    pub config: &'a Configuration,
    pub scope: String,
    pub budget: Mutex<Option<super::budget::Budget>>,
    pub health_key: String,
    report: Mutex<ExecutionReport>,
    transport_failures: Mutex<BTreeMap<String, BTreeSet<String>>>,
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
        Ok(Self {
            store,
            owner,
            run,
            config,
            scope,
            budget: Mutex::new(None),
            health_key,
            report: Mutex::new(run.report.clone().unwrap_or_default()),
            transport_failures: Mutex::new(BTreeMap::new()),
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
    pub fn timeout(&self, phase: &str) -> Result<Option<u64>> {
        ensure!(!self.store.get(&self.run.id)?.terminal(), "Fusion stopped");
        future_fusion::phase_timeout_ms(
            self.config,
            phase,
            future_fusion::now_ms()
                .saturating_sub(self.run.created_at)
                .max(0) as u64,
        )
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
            report_quality: None,
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
    fn panel_wait_starts_with_a_usable_result_and_never_extends() {
        let now = tokio::time::Instant::now();
        let minutes = |count: u64| Duration::from_secs(count * 60);
        let mut deadline = PanelDeadline::default();

        deadline.participant_finished(now, false, 3);
        assert_eq!(deadline.cutoff(), None);
        deadline.participant_finished(now + minutes(1), true, 2);
        assert_eq!(deadline.cutoff(), Some(now + minutes(11)));
        deadline.participant_finished(now + minutes(8), true, 1);
        assert_eq!(deadline.cutoff(), Some(now + minutes(11)));
    }

    #[test]
    fn the_last_pending_participant_tightens_the_deadline_even_after_a_failure() {
        let now = tokio::time::Instant::now();
        let mut deadline = PanelDeadline::default();
        deadline.participant_finished(now, true, 2);
        deadline.participant_finished(now + Duration::from_secs(120), false, 1);
        assert_eq!(deadline.cutoff(), Some(now + Duration::from_secs(420)));

        let mut two_participants = PanelDeadline::default();
        two_participants.participant_finished(now, true, 1);
        assert_eq!(
            two_participants.cutoff(),
            Some(now + Duration::from_secs(300))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn reasoning_can_continue_far_past_the_inactivity_interval() {
        let progress = IdleProgress::new(300_000);
        let producer = progress.clone();
        let started = tokio::time::Instant::now();
        progress
            .guard(async move {
                for _ in 0..10 {
                    tokio::time::sleep(Duration::from_secs(250)).await;
                    producer.content_received();
                }
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(started.elapsed(), Duration::from_secs(2500));
    }

    #[tokio::test(start_paused = true)]
    async fn status_notifications_do_not_keep_a_silent_request_alive() {
        let progress = IdleProgress::new(300_000);
        let notices = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let sender = notices.clone();
        let result: Result<()> = progress
            .guard(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    sender.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            })
            .await;
        assert_eq!(failure_kind(&result.unwrap_err()), FailureKind::IdleTimeout);
        assert!(notices.load(std::sync::atomic::Ordering::Relaxed) >= 4);
    }
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
            legacy_options: vec![],
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
