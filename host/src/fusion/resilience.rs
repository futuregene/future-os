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
static COOLDOWN: LazyLock<Mutex<BTreeMap<String, Cooldown>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

struct Cooldown {
    at: Instant,
    delay: Duration,
    kind: FailureKind,
}
impl Cooldown {
    fn after_failure(previous: Option<&Self>, kind: FailureKind) -> Self {
        Self {
            at: Instant::now(),
            delay: previous.map_or(Duration::from_secs(15), |p| {
                (p.delay * 2).min(Duration::from_secs(300))
            }),
            kind,
        }
    }
}

/// Silence a phase tolerates from its still-pending participants before it
/// stops waiting for them.
const PANEL_WINDOW: Duration = Duration::from_secs(10 * 60);
/// Same, once only one participant is still pending.
const PANEL_LAST_WINDOW: Duration = Duration::from_secs(5 * 60);
/// Hard bound on that wait, however long the pending participants keep talking.
const PANEL_CEILING: Duration = Duration::from_secs(30 * 60);
/// How often a waiting phase re-reads content progress to recompute its cutoff.
pub(super) const PANEL_POLL: Duration = Duration::from_secs(15);

/// Content progress shared by the participants of one panel. A model that is
/// still producing reasoning or text is working, not hung, so the wait is
/// anchored to the latest progress instead of to the fastest answer.
#[derive(Clone, Default)]
pub(super) struct ProgressClock(Arc<Mutex<Option<tokio::time::Instant>>>);

impl ProgressClock {
    pub fn reset(&self) {
        *self.lock() = None;
    }
    pub fn touch(&self) {
        *self.lock() = Some(tokio::time::Instant::now());
    }
    pub fn last(&self) -> Option<tokio::time::Instant> {
        *self.lock()
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<tokio::time::Instant>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub(super) struct PanelDeadline {
    phase_deadline: Option<tokio::time::Instant>,
    first_finished: Option<tokio::time::Instant>,
}

impl PanelDeadline {
    pub fn new(phase_deadline: Option<tokio::time::Instant>) -> Self {
        Self {
            phase_deadline,
            first_finished: None,
        }
    }

    /// The first terminal result (an answer or a failure) starts the progress-based wait.
    pub fn participant_finished(&mut self, now: tokio::time::Instant) {
        self.first_finished.get_or_insert(now);
    }

    /// A configured phase deadline applies even before a participant finishes.
    /// Afterwards the silence window and existing hard ceiling can cut the wait short.
    pub fn cutoff(
        &self,
        pending: usize,
        progress: Option<tokio::time::Instant>,
    ) -> Option<tokio::time::Instant> {
        let progress_cutoff = self.first_finished.map(|first| {
            let window = if pending <= 1 {
                PANEL_LAST_WINDOW
            } else {
                PANEL_WINDOW
            };
            let anchor = progress.map_or(first, |at| at.max(first));
            (anchor + window).min(first + PANEL_CEILING)
        });
        progress_cutoff.into_iter().chain(self.phase_deadline).min()
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
    panel: ProgressClock,
}
impl IdleProgress {
    pub fn new(milliseconds: u64, panel: ProgressClock) -> Arc<Self> {
        Arc::new(Self {
            last_content: Mutex::new(tokio::time::Instant::now()),
            duration: Duration::from_millis(milliseconds),
            panel,
        })
    }
    pub fn content_received(&self) {
        *self.last_content.lock().unwrap_or_else(|e| e.into_inner()) = tokio::time::Instant::now();
        self.panel.touch();
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
    /// Content progress of the panel currently waiting, reset per phase.
    pub progress: ProgressClock,
    pub synthesis_audit: Mutex<Option<serde_json::Value>>,
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
            progress: ProgressClock::default(),
            synthesis_audit: Mutex::new(None),
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
        cache.retain(|_, state| state.at.elapsed() < Duration::from_secs(900));
        cache
            .get(&self.provider_key(model))
            .filter(|state| state.at.elapsed() < state.delay)
            .map(|state| state.kind)
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
                cache.retain(|_, state| state.at.elapsed() < Duration::from_secs(900));
                if cache.len() >= 64 {
                    cache.clear();
                }
                let key = self.provider_key(model);
                let next = Cooldown::after_failure(cache.get(&key), kind);
                cache.insert(key, next);
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
        COOLDOWN
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.provider_key(&self.config.moderator.model));
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
    fn repeated_provider_outages_back_off_without_unbounded_delay() {
        let mut state = Cooldown::after_failure(None, FailureKind::Transient);
        assert_eq!(state.delay, Duration::from_secs(15));
        for _ in 0..8 {
            state = Cooldown::after_failure(Some(&state), FailureKind::Transient);
        }
        assert_eq!(state.delay, Duration::from_secs(300));
    }

    #[test]
    fn panel_wait_starts_after_any_terminal_result_and_never_extends() {
        let now = tokio::time::Instant::now();
        let minutes = |count: u64| Duration::from_secs(count * 60);
        let mut deadline = PanelDeadline::new(None);

        assert_eq!(deadline.cutoff(3, None), None);
        deadline.participant_finished(now);
        assert_eq!(deadline.cutoff(3, None), Some(now + minutes(10)));
        deadline.participant_finished(now + minutes(1));
        assert_eq!(deadline.cutoff(3, None), Some(now + minutes(10)));
    }

    #[test]
    fn content_progress_keeps_a_working_panel_alive_up_to_the_ceiling() {
        let now = tokio::time::Instant::now();
        let minutes = |count: u64| Duration::from_secs(count * 60);
        let mut deadline = PanelDeadline::new(None);
        deadline.participant_finished(now);

        // The run that exposed this: two panelists were still reasoning ten
        // minutes after the fastest answer and were cut mid-work.
        assert_eq!(
            deadline.cutoff(2, Some(now + minutes(8))),
            Some(now + minutes(18))
        );
        // Progress that stopped long ago does not buy more time.
        assert_eq!(
            deadline.cutoff(2, Some(now + minutes(2))),
            Some(now + minutes(12))
        );
        assert_eq!(deadline.cutoff(2, None), Some(now + minutes(10)));
        // A panel that never stops producing is still bounded.
        assert_eq!(
            deadline.cutoff(2, Some(now + minutes(29))),
            Some(now + minutes(30))
        );
    }

    #[test]
    fn the_last_pending_participant_tightens_the_deadline_even_after_a_failure() {
        let now = tokio::time::Instant::now();
        let mut deadline = PanelDeadline::new(None);
        deadline.participant_finished(now);
        deadline.participant_finished(now + Duration::from_secs(120));
        assert_eq!(
            deadline.cutoff(1, None),
            Some(now + Duration::from_secs(300))
        );

        let mut two_participants = PanelDeadline::new(None);
        two_participants.participant_finished(now);
        assert_eq!(
            two_participants.cutoff(2, None),
            Some(now + Duration::from_secs(600))
        );
    }

    #[test]
    fn configured_deadline_applies_before_a_report_and_cannot_be_extended_by_progress() {
        let now = tokio::time::Instant::now();
        let limit = now + Duration::from_secs(900);
        let mut deadline = PanelDeadline::new(Some(limit));

        assert_eq!(deadline.cutoff(3, None), Some(limit));
        assert_eq!(
            deadline.cutoff(3, Some(now + Duration::from_secs(899))),
            Some(limit)
        );

        deadline.participant_finished(now + Duration::from_secs(60));
        assert_eq!(
            deadline.cutoff(2, None),
            Some(now + Duration::from_secs(660))
        );
        assert_eq!(
            deadline.cutoff(2, Some(now + Duration::from_secs(899))),
            Some(limit)
        );
    }

    #[test]
    fn panel_progress_is_shared_by_the_participants_of_one_phase() {
        let clock = ProgressClock::default();
        assert_eq!(clock.last(), None);
        IdleProgress::new(300_000, clock.clone()).content_received();
        assert!(clock.last().is_some());
        clock.reset();
        assert_eq!(clock.last(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn reasoning_can_continue_far_past_the_inactivity_interval() {
        let progress = IdleProgress::new(300_000, ProgressClock::default());
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
        let progress = IdleProgress::new(300_000, ProgressClock::default());
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
            answer_only: false,
            parent_request: None,
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
