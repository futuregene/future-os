//! One shared Host executor for Agent-originated, durable Fusion tool calls.
use anyhow::{ensure, Context, Result};
use future_fusion::{
    now_ms, parse_json, Conclusion, Configuration, Consultation, ModelConfig, Step, StepSettings,
    Store,
};
use future_rpc::proto::RpcCommand;
use futures::{FutureExt, StreamExt};
mod budget;
mod mapping;
mod quotes;
mod reports;
mod resilience;
mod review;
use future_fusion::{failure, failure_kind, FailureKind};
use resilience::Runtime;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static STARTED: AtomicBool = AtomicBool::new(false);
static STOPPING: AtomicBool = AtomicBool::new(false);

pub fn start() {
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    STOPPING.store(false, Ordering::Release);
    crate::runtime::spawn(async {
        if let Err(error) = serve().await {
            tracing::error!("Fusion service stopped: {error:#}");
        }
        STARTED.store(false, Ordering::Release);
    });
}
pub fn stop_admission() {
    STOPPING.store(true, Ordering::Release);
}

async fn rpc(mut command: RpcCommand) -> Result<Value> {
    command.id = uuid::Uuid::new_v4().to_string();
    let mut client = crate::agent_client::connect_agent().await?;
    // AgentClient already registers accepted child requests in request_scope.
    let response = client.execute_command(command).await?.into_inner();
    ensure!(response.success, "{}: {}", response.command, response.error);
    Ok(future_rpc::decode::response_data(&response))
}
fn command(kind: &str, session: &str) -> RpcCommand {
    RpcCommand {
        r#type: kind.into(),
        session_id: session.into(),
        ..Default::default()
    }
}

pub async fn settings() -> Result<Value, crate::Error> {
    rpc(command("get_fusion_settings", ""))
        .await
        .map_err(|error| crate::Error::Message(error.to_string()))
}

pub async fn update_settings(patch: Value) -> Result<Value, crate::Error> {
    rpc(RpcCommand {
        fusion_settings_json: serde_json::to_string(&patch)?,
        ..command("update_fusion_settings", "")
    })
    .await
    .map_err(|error| crate::Error::Message(error.to_string()))
}

async fn serve() -> Result<()> {
    let store = Store::open(&crate::future_home_root())?;
    // Old child requests may outlive a Host crash. Fence their consultation
    // before admission, then stop only internal Fusion sessions.
    let unfinished = store.unfinished_steps()?;
    store.reconcile()?;
    for step in unfinished {
        if !step.session_id.is_empty() {
            let _ = abort(&step).await;
        }
    }
    let owner = uuid::Uuid::new_v4().to_string();
    let mut active = tokio::task::JoinSet::new();
    let mut interval = tokio::time::interval(Duration::from_millis(500));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while !STOPPING.load(Ordering::Acquire) {
        interval.tick().await;
        store.heartbeat(&owner)?;
        while let Some(result) = active.try_join_next() {
            if let Err(error) = result {
                tracing::error!("Fusion worker failed: {error}");
            }
        }
        if active.len() < 2 {
            if let Some(run) = store.claim(&owner)? {
                let store = store.clone();
                let owner = owner.clone();
                active.spawn(async move {
                    let result = std::panic::AssertUnwindSafe(execute(&store, &owner, &run))
                        .catch_unwind()
                        .await;
                    let failure = match result {
                        Ok(Ok(())) => None,
                        Ok(Err(error)) => Some(format!("{error:#}")),
                        Err(_) => Some("Fusion executor panicked; inspect diagnostics".into()),
                    };
                    if let Some(error) = failure {
                        if let Err(save) = store.fail(&run.id, Some(&owner), &error) {
                            tracing::error!("Fusion failure could not be saved: {save:#}");
                        }
                        for mut step in store
                            .unfinished_steps()
                            .unwrap_or_default()
                            .into_iter()
                            .filter(|step| step.fusion_run_id == run.id)
                        {
                            if !step.session_id.is_empty() {
                                let _ = abort(&step).await;
                            }
                            step.status = "interrupted".into();
                            step.finished_at = Some(now_ms());
                            step.failure_kind = Some(FailureKind::Cancelled);
                            let _ = store.record_step(&step);
                        }
                    }
                });
            }
        }
    }
    // Stop owned child runs before ending the service. The normal Host shutdown
    // also tracks these accepted requests through request_scope.
    active.abort_all();
    while active.join_next().await.is_some() {}
    Ok(())
}

async fn execute(store: &Store, owner: &str, run: &Consultation) -> Result<()> {
    let config = configuration(run)?;
    config.validate()?;
    let status = rpc(command("get_fusion_status", "")).await?;
    ensure!(
        status["generation_policy_version"].as_u64() == Some(u64::from(config.version))
            && status["resilience_policy_version"].as_u64()
                == Some(u64::from(future_fusion::RESILIENCE_POLICY_VERSION)),
        "Agent does not support this Fusion policy; upgrade Agent and Host together"
    );
    ensure!(
        status["database_id"].as_str() == Some(&run.parent.database_id),
        "Fusion parent belongs to another Agent database/restore generation"
    );
    let scope = format!(
        "{}:{}:{}",
        run.parent.database_id, status["agent_instance_id"], status["config_revision"]
    );
    let ctx = Runtime::new(store, owner, run, scope)?;
    ensure!(
        status["context_budget_version"].as_u64()
            == Some(u64::from(future_fusion::CONTEXT_POLICY_VERSION)),
        "Upgrade Agent for Fusion context admission"
    );
    if let Err(error) = ctx.timeout("preflight") {
        ctx.note(
            "summary_budget_reserved",
            "preflight",
            None,
            Some(failure_kind(&error)),
        )?;
        return Err(error);
    }
    let mut catalog = rpc(command("list_models", "")).await?;
    let usable = |catalog: &Value, profile: &ModelConfig| -> bool {
        let Some((provider, id)) = profile.model.split_once('/') else {
            return false;
        };
        catalog["models"].as_array().is_some_and(|models| {
            models.iter().any(|model| {
                model["provider"] == provider
                    && model["id"] == id
                    && model["thinkingLevels"].as_array().is_some_and(|levels| {
                        levels
                            .iter()
                            .any(|level| level.as_str() == Some(&profile.thinking_level))
                    })
            })
        })
    };
    if config
        .panel
        .iter()
        .chain([&config.moderator])
        .any(|model| model.model.starts_with("future/") && !usable(&catalog, model))
    {
        // A missing participant must not take healthy models down with it.
        if rpc(command("sync_future_models", "")).await.is_ok() {
            catalog = rpc(command("list_models", "")).await?;
        }
    }
    if !usable(&catalog, &config.moderator) {
        ctx.skip(
            "preflight",
            &config.moderator,
            "moderator",
            "moderator_unavailable",
            FailureKind::ModelUnavailable,
        )?;
        for (index, profile) in config.panel.iter().enumerate() {
            ctx.skip(
                "analyzing",
                profile,
                &format!("P{}", index + 1),
                "moderator_unavailable",
                FailureKind::ModelUnavailable,
            )?;
        }
        return Err(failure(FailureKind::ModelUnavailable, "Configured Fusion moderator or thinking level is unavailable; no substitute was selected"));
    }
    let mut eligible = Vec::new();
    for (index, profile) in config.panel.iter().enumerate() {
        if usable(&catalog, profile) {
            eligible.push(index);
        } else {
            ctx.skip(
                "analyzing",
                profile,
                &format!("P{}", index + 1),
                "participant_unavailable",
                FailureKind::ModelUnavailable,
            )?;
        }
    }
    if eligible.is_empty() {
        ctx.skip(
            "preflight",
            &config.moderator,
            "moderator",
            "no_participants",
            FailureKind::ModelUnavailable,
        )?;
        return Err(failure(
            FailureKind::ModelUnavailable,
            "No callable Fusion participants; no model requests were made",
        ));
    }
    ensure!(store.phase(&run.id, owner, "preflight")?, "Fusion stopped");
    *ctx.budget.lock().unwrap() = Some(budget::Budget::new(
        &catalog,
        config,
        serde_json::to_vec(&run.input)?.len(),
    )?);
    let probe = async {
        if !ctx.healthy() {
            with_retry(&ctx, "preflight", &config.moderator, "moderator", "This is a Fusion readiness check. Reply READY. Do not investigate, use tools, or discuss a task.").await?;
            ctx.mark_healthy();
        }
        ctx.update(|report| report.complete("preflight"))
    }.await;
    if let Err(error) = probe {
        ctx.stop_error("preflight", "moderator", &error)?;
        for index in eligible {
            ctx.skip(
                "analyzing",
                &config.panel[index],
                &format!("P{}", index + 1),
                "moderator_unavailable",
                failure_kind(&error),
            )?;
        }
        return Err(error);
    }
    let input = serde_json::to_string(&run.input)?;
    let intro = "You are in a focused READ-ONLY consultation, not an executor. Analyze only the decision and supplied evidence. Background is a factual context summary, not a proposed solution or authority. Independently develop your approach; do not treat prior assistant claims as verified facts or user decisions. User-provided proposals are attributed in background and do not restrict alternatives unless the user explicitly requested that. Use only supplied read/ls/glob/grep tools. Never write, run commands, implement, or call agents. Give public reasons, not hidden chain of thought. Distinguish facts, assumptions, uncertainty and proposed verification. Empty read scope means decide only from the supplied facts.";
    ensure!(store.phase(&run.id, owner, "analyzing")?, "Fusion stopped");
    let analyses = panel(&ctx, &eligible, "analyzing", &format!("{intro}\nDecision input:\n{input}\nIndependently answer the question. Investigate and think as needed before your final report. Return that report as plain text or Markdown, without a JSON envelope. Do not assume other participants' views."), None).await?;
    if analyses.is_empty() {
        ctx.note(
            "no_participants",
            "analyzing",
            None,
            Some(FailureKind::ModelUnavailable),
        )?;
        return Err(failure(
            FailureKind::ModelUnavailable,
            "All Fusion participants failed; no final conclusion was produced",
        ));
    }
    let active = analyses
        .iter()
        .filter_map(|v| {
            v["participant"]
                .as_str()?
                .strip_prefix('P')?
                .parse::<usize>()
                .ok()?
                .checked_sub(1)
        })
        .collect::<Vec<_>>();
    ctx.update(|report| {
        report.complete("analyzing");
        report.analysis_participants = analyses
            .iter()
            .filter_map(|v| v["participant"].as_str().map(str::to_owned))
            .collect();
        if analyses.len() < config.panel.len() {
            report.note("participants_reduced", "analyzing", None, None);
        }
    })?;
    let mut mapping_analyses = analyses.clone();
    fit_material(&ctx, "analyzing", &mut mapping_analyses)?;
    let first = serde_json::to_string(&mapping_analyses)?;
    let mut issues = json!({"issues":[]});
    let mut reviews = vec![];
    if active.len() == 1 {
        ctx.update(|report| {
            report.strategy = "single_advisor".into();
            report.skip("issue_mapping");
            report.skip("reviewing");
            report.note("single_participant", "analyzing", None, None);
        })?;
    } else if ctx.timeout("issue_mapping").is_err() {
        ctx.update(|report| {
            report.strategy = "analysis_only".into();
            report.skip("issue_mapping");
            report.skip("reviewing");
            report.note(
                "summary_budget_reserved",
                "reviewing",
                None,
                Some(FailureKind::BudgetReserved),
            );
        })?;
    } else {
        ensure!(
            store.phase(&run.id, owner, "issue_mapping")?,
            "Fusion stopped"
        );
        let prompt = format!(
            "{}\nDecision input:\n{input}\nIndependent analyses:\n{first}",
            mapping::INSTRUCTIONS
        );
        match with_retry(
            &ctx,
            "issue_mapping",
            &config.moderator,
            "moderator",
            &prompt,
        )
        .await
        {
            Ok(mapping) => {
                issues = parse_json(&mapping.text)?;
                for (index, issue) in issues["issues"]
                    .as_array_mut()
                    .context("Missing issues")?
                    .iter_mut()
                    .enumerate()
                {
                    issue["id"] = json!(format!("I{}", index + 1));
                }
                store.record_issues(&run.id, &issues)?;
                ctx.update(|report| report.complete("issue_mapping"))?;
            }
            Err(error) if failure_kind(&error) == FailureKind::InvalidOutput => {
                ctx.update(|report| {
                    report.strategy = "analysis_only".into();
                    report.skip("issue_mapping");
                    report.note(
                        "mapping_invalid",
                        "issue_mapping",
                        Some("moderator"),
                        Some(FailureKind::InvalidOutput),
                    );
                })?;
            }
            Err(error) if failure_kind(&error) == FailureKind::BudgetReserved => {
                ctx.update(|report| {
                    report.strategy = "analysis_only".into();
                    report.skip("issue_mapping");
                    report.note(
                        "summary_budget_reserved",
                        "issue_mapping",
                        None,
                        Some(FailureKind::BudgetReserved),
                    );
                })?;
            }
            Err(error) => {
                ctx.stop_error("issue_mapping", "moderator", &error)?;
                return Err(error);
            }
        }
        let review_has_time = ctx.timeout("reviewing").is_ok();
        if review_has_time && ctx.report().strategy != "analysis_only" {
            ensure!(store.phase(&run.id, owner, "reviewing")?, "Fusion stopped");
            reviews = panel(&ctx, &active, "reviewing", "", Some((&analyses, &issues))).await?;
            ctx.update(|report| {
                if !reviews.is_empty() {
                    report.complete("reviewing");
                } else {
                    report.skip("reviewing");
                }
                report.review_participants = reviews
                    .iter()
                    .filter_map(|v| v["participant"].as_str().map(str::to_owned))
                    .collect();
                if reviews.len() < active.len() {
                    report.note("reviews_reduced", "reviewing", None, None);
                }
                if reviews.is_empty() {
                    report.strategy = "analysis_only".into();
                    report.note("no_reviews", "reviewing", None, None);
                }
            })?;
        } else {
            ctx.update(|report| {
                report.strategy = "analysis_only".into();
                report.skip("reviewing");
                if !review_has_time {
                    report.note(
                        "summary_budget_reserved",
                        "reviewing",
                        None,
                        Some(FailureKind::BudgetReserved),
                    );
                }
            })?;
        }
    }
    ensure!(
        store.phase(&run.id, owner, "synthesizing")?,
        "Fusion stopped"
    );
    // One current report per role. Pack the chosen mix together, including
    // analysis-only roles; separate pools could overbook the final moderator.
    let mut current = analyses
        .iter()
        .map(|analysis| {
            let reviewed = reviews
                .iter()
                .find(|r| r["participant"] == analysis["participant"]);
            let mut source = reviewed.unwrap_or(analysis).clone();
            source["source_phase"] = json!(if reviewed.is_some() {
                "reviewing"
            } else {
                "analysis_only"
            });
            source
        })
        .collect::<Vec<_>>();
    fit_material(&ctx, "synthesizing", &mut current)?;
    let report = ctx.report();
    let coverage = json!({"strategy":report.strategy,"completed_phases":report.completed_phases,"skipped_phases":report.skipped_phases,"analysis_participants":report.analysis_participants,"review_participants":report.review_participants,"degradations":report.degradations});
    let material = current.iter().map(|source| {
        let response = source["response"].as_str().unwrap_or_default();
        format!("Participant {} ({}) report, {} UTF-8 bytes. Validation issues: {}. Shortened for context: {}\n{}",
            source["participant"].as_str().unwrap_or_default(),
            source["source_phase"].as_str().unwrap_or_default(), response.len(),
            source["validation_errors"], source["forwarded_truncated"].as_bool().unwrap_or(false), response)
    }).collect::<Vec<_>>().join("\n\n");
    let prompt = format!("You are the moderator. Directly answer the original question, producing a useful, self-contained conclusion for the main agent. Synthesize ONLY the supplied participants' current answers and supporting reviews. Use the original task only for its goal and boundaries. Distinguish shared views from a recommendation you select from existing proposals. Preserve material disagreement and evidence limitations; do not revive withdrawn positions. Do not add facts, proposals, omissions, clarification questions, conditions sections or next-step instructions. Participant verification labels are reports, not system proof. Reports with validation_errors are usable participant opinions, not validated quotations or verified facts. Do not present their unmatched quotations as established source evidence. Put any essential applicability qualification directly in the conclusion. Do not repeat meeting history, participant-by-participant recaps or general unknowns that do not affect the answer. Include supporting reasons or unresolved issues only when essential, without repeating the answer. You have no tools. Return only a plain text or Markdown report directly answering the question, without a JSON envelope or fixed section template. Respect actual coverage; full participation does not imply truth or unanimity. Missing reviews mean no cross-review.\nOriginal task:\n{}\nCurrent participant material:\n{}\nCoverage:\n{}", serde_json::to_string(&run.input)?, material, coverage);

    let text = match with_retry(
        &ctx,
        "synthesizing",
        &config.moderator,
        "moderator",
        &prompt,
    )
    .await
    {
        Ok(report) => report.text,
        Err(error) => {
            ctx.stop_error("synthesizing", "moderator", &error)?;
            return Err(error);
        }
    };
    let result = Conclusion {
        conclusion: text,
        ..Default::default()
    };
    ctx.update(|report| report.complete("synthesizing"))?;
    let report = ctx.report();
    let completeness = if analyses.len() == 1 {
        "single"
    } else if !report.degradations.is_empty() || !report.skipped_phases.is_empty() {
        "partial"
    } else {
        "full"
    };
    store.finish(&run.id, owner, result, completeness)?;
    Ok(())
}

fn fit_material(ctx: &Runtime<'_>, phase: &str, material: &mut [Value]) -> Result<()> {
    let shortened = ctx
        .budget
        .lock()
        .unwrap()
        .as_mut()
        .context("Missing Fusion context budget")?
        .fit_material(phase, material)?;
    for participant in shortened {
        ctx.note("report_truncated", phase, Some(&participant), None)?;
    }
    Ok(())
}

async fn panel(
    ctx: &Runtime<'_>,
    indices: &[usize],
    phase: &str,
    prompt: &str,
    review_material: Option<(&[Value], &Value)>,
) -> Result<Vec<Value>> {
    let mut pending = futures::stream::FuturesUnordered::new();
    for &index in indices {
        let model = &ctx.config.panel[index];
        let participant = format!("P{}", index + 1);
        let participant_prompt = if let Some((analyses, mapping)) = review_material {
            review::prompt(&participant, &ctx.run.input, analyses, mapping)?
        } else {
            format!("Your participant ID is {participant}.\n{prompt}")
        };
        pending.push(async move {
            let answer = with_retry(ctx, phase, model, &participant, &participant_prompt).await;
            (participant, model, answer)
        });
    }
    let mut good = vec![];
    let mut persistence_error = None;
    while let Some((participant, model, result)) = pending.next().await {
        match result {
            Ok(report) => good.push(json!({"participant": participant, "model": model.model, "response": report.text, "validation_errors":report.validation_errors})),
            Err(error) => {
                if let Err(save) = ctx.stop_error(phase, &participant, &error) {
                    persistence_error = Some(save);
                }
            }
        }
    }
    if let Some(error) = persistence_error {
        return Err(error);
    }
    good.sort_by(|a, b| a["participant"].as_str().cmp(&b["participant"].as_str()));
    Ok(good)
}

fn configuration(run: &Consultation) -> Result<&Configuration> {
    run.configuration
        .as_ref()
        .context("Fusion configuration snapshot is missing; start a new discussion")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RetryReason {
    OutputLimit,
    InvalidOutput,
    Transient,
}

impl RetryReason {
    fn code(self) -> &'static str {
        match self {
            Self::OutputLimit => "output_limit",
            Self::InvalidOutput => "invalid_output",
            Self::Transient => "transient",
        }
    }
}

fn retry_for_kind(kind: FailureKind) -> Option<RetryReason> {
    match kind {
        FailureKind::OutputLimit => Some(RetryReason::OutputLimit),
        FailureKind::InvalidOutput => Some(RetryReason::InvalidOutput),
        FailureKind::RateLimited | FailureKind::Transient => Some(RetryReason::Transient),
        _ => None,
    }
}

fn attempt_settings(
    config: &Configuration,
    model: &ModelConfig,
    retry: Option<RetryReason>,
    elapsed_ms: u64,
) -> Result<StepSettings> {
    let remaining_ms = config
        .run_timeout_secs
        .saturating_mul(1000)
        .saturating_sub(elapsed_ms);
    ensure!(remaining_ms > 0, "Fusion total time budget exhausted");
    Ok(StepSettings {
        thinking_level: model.thinking_level.clone(),
        max_output_tokens: 0, // Filled by per-attempt context allocation.
        context_window: 0,
        max_public_bytes: 0,
        report_revision: 0,
        timeout_ms: config
            .step_timeout_secs
            .saturating_mul(1000)
            .min(remaining_ms),
        retry_reason: retry.map(|reason| reason.code().into()),
    })
}

async fn with_retry(
    ctx: &Runtime<'_>,
    phase: &str,
    model: &ModelConfig,
    participant: &str,
    prompt: &str,
) -> Result<reports::Report> {
    if let Err(error) = ctx
        .ensure_provider(&model.model)
        .and_then(|_| ctx.timeout(phase).map(|_| ()))
    {
        ctx.skip(
            phase,
            model,
            participant,
            "step_skipped",
            failure_kind(&error),
        )?;
        return Err(error);
    }
    let semaphore = ctx.permit(&model.model);
    let _permit = match tokio::time::timeout(
        Duration::from_millis(ctx.timeout(phase)?),
        semaphore.acquire(),
    )
    .await
    {
        Ok(permit) => permit?,
        Err(_) => {
            ctx.skip(
                phase,
                model,
                participant,
                "summary_budget_reserved",
                FailureKind::BudgetReserved,
            )?;
            return Err(failure(
                FailureKind::BudgetReserved,
                "Fusion provider queue exceeded phase budget",
            ));
        }
    };
    let mut last = failure(FailureKind::Unknown, "Fusion step not started");
    let mut retry_reason = None;
    for attempt in 0..2 {
        if let Err(error) = ctx.ensure_provider(&model.model) {
            if attempt == 0 {
                ctx.skip(
                    phase,
                    model,
                    participant,
                    "provider_unavailable",
                    FailureKind::CircuitOpen,
                )?;
            }
            return Err(error);
        }
        let timeout_ms = match ctx.timeout(phase) {
            Ok(timeout) => timeout,
            Err(error) => {
                if attempt == 0 {
                    ctx.skip(
                        phase,
                        model,
                        participant,
                        "summary_budget_reserved",
                        failure_kind(&error),
                    )?;
                }
                return Err(error);
            }
        };
        let mut settings = attempt_settings(
            ctx.config,
            model,
            retry_reason,
            now_ms().saturating_sub(ctx.run.created_at).max(0) as u64,
        )?;
        settings.timeout_ms = timeout_ms;
        ctx.budget
            .lock()
            .unwrap()
            .as_ref()
            .context("Missing Fusion context budget")?
            .apply(&model.model, phase, prompt, &mut settings)?;
        let input = match retry_reason {
            Some(RetryReason::OutputLimit) => format!("{prompt}\nPrevious attempt reached the provider output limit. Return a complete answer in the requested format."),
            Some(RetryReason::InvalidOutput) => format!("{prompt}\nPrevious attempt had invalid output. Return a complete public answer in exactly the requested format, using only supplied participant references."),
            _ => prompt.to_owned(),
        };
        let input = if settings.max_public_bytes > 0 {
            format!(
                "{input}\n{}",
                reports::capacity_instruction(settings.max_public_bytes)
            )
        } else {
            input
        };
        // Include the actual report instruction and repair suffix, without
        // treating transport escapes as additional model-visible text.
        ctx.budget
            .lock()
            .unwrap()
            .as_ref()
            .context("Missing Fusion context budget")?
            .apply(&model.model, phase, &input, &mut settings)?;
        let mut step = Step {
            id: format!("{}-{phase}-{participant}-{attempt}", ctx.run.id),
            fusion_run_id: ctx.run.id.clone(),
            phase: phase.into(),
            model: model.model.clone(),
            participant: participant.into(),
            attempt,
            input,
            session_id: String::new(),
            agent_run_id: format!("run_{}", uuid::Uuid::new_v4()),
            status: "starting".into(),
            output: String::new(),
            error: None,
            started_at: now_ms(),
            finished_at: None,
            usage: Value::Null,
            settings: Some(settings),
            failure_kind: None,
            report_quality: None,
        };
        ctx.store.record_step(&step)?;
        let result = tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            step_inner(ctx, &mut step, prompt),
        )
        .await;
        let result = match result {
            Ok(Ok(text)) => Ok(text),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(failure(
                if phase != "synthesizing" && ctx.timeout(phase).is_err() {
                    FailureKind::BudgetReserved
                } else {
                    FailureKind::Timeout
                },
                "Fusion model timed out",
            )),
        };
        step.finished_at = Some(now_ms());
        match result {
            Ok(report) => {
                let text = &report.text;
                step.status = "completed".into();
                step.output = future_fusion::bounded_text(text, 2400);
                ctx.store.record_step(&step)?;
                if matches!(phase, "analyzing" | "reviewing") {
                    ctx.retain(&step, text)?;
                }
                if participant == "moderator" {
                    ctx.mark_healthy();
                }
                return Ok(report);
            }
            Err(error) => {
                let kind = failure_kind(&error);
                step.status = "failed".into();
                step.error = Some(resilience::safe_error(&error));
                step.failure_kind = Some(kind);
                // Always fence the owned request, even when persistence fails.
                let saved = ctx.store.record_step(&step);
                // Trip the gate before awaiting cancellation/receipt RPCs so
                // another successful slot cannot dispatch into a known outage.
                if participant == "moderator" {
                    ctx.invalidate_health();
                }
                let observed = ctx.observe_failure(&model.model, kind);
                let fenced = if step.session_id.is_empty() {
                    true
                } else {
                    stop_before_retry(&step).await.is_ok()
                };
                saved?;
                if !fenced {
                    ctx.note(
                        "retry_abort_unconfirmed",
                        phase,
                        Some(participant),
                        Some(kind),
                    )?;
                    return Err(error);
                }
                observed?;
                let reason = retry_for_kind(kind);
                last = error;
                if attempt == 1
                    || reason.is_none()
                    || ctx.circuit(&model.model).is_some()
                    || step
                        .settings
                        .as_ref()
                        .is_some_and(|s| s.report_revision > 0)
                {
                    break;
                }
                retry_reason = reason;
                if reason == Some(RetryReason::Transient) {
                    let delay = Duration::from_millis(1000 + rand::random::<u16>() as u64 % 500);
                    if ctx.timeout(phase)? <= delay.as_millis() as u64 {
                        return Err(failure(
                            FailureKind::BudgetReserved,
                            "Fusion retry would consume final summary reserve",
                        ));
                    }
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
    Err(last)
}

fn validate_step_output(phase: &str, text: &str, participants: usize) -> Result<()> {
    if phase == "issue_mapping" {
        mapping::validate_shape(text, participants)?;
    } else {
        ensure!(!text.trim().is_empty(), "Empty Fusion report");
    }
    Ok(())
}

async fn abort(step: &Step) -> Result<()> {
    rpc(RpcCommand {
        run_id: step.agent_run_id.clone(),
        ..command("abort", &step.session_id)
    })
    .await?;
    Ok(())
}
/// A timeout or lost RPC response can leave an accepted child running. Never
/// submit a second paid attempt until that exact child is durably terminal.
async fn stop_before_retry(step: &Step) -> Result<()> {
    let terminal = |state: &Value| {
        matches!(
            state["requestedRun"]["status"].as_str(),
            Some(
                "completed"
                    | "failed"
                    | "error"
                    | "cancelled"
                    | "interrupted"
                    | "interrupted_by_restart"
                    | "incomplete"
            )
        )
    };
    if let Ok(state) = rpc(RpcCommand {
        run_id: step.agent_run_id.clone(),
        ..command("get_state", &step.session_id)
    })
    .await
    {
        if terminal(&state) {
            return Ok(());
        }
    }
    abort(step).await?;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let state = rpc(RpcCommand {
                run_id: step.agent_run_id.clone(),
                ..command("get_state", &step.session_id)
            })
            .await?;
            if terminal(&state) {
                return Ok::<(), anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await??;
    Ok(())
}
async fn step_inner(
    ctx: &Runtime<'_>,
    step: &mut Step,
    source_prompt: &str,
) -> Result<reports::Report> {
    let store = ctx.store;
    let run = ctx.run;
    ctx.ensure_provider(&step.model)?;
    let created = rpc(crate::agent_commands::new_session_command(
        String::new(),
        run.snapshot_root.clone(),
        "fusion",
        run.id.clone(),
        json!({"fusionRunId":run.id,"stepId":step.id,"parentSessionId":run.parent.session_id}),
        Some(step.model.clone()),
        step.settings
            .as_ref()
            .map(|settings| settings.thinking_level.clone()),
    ))
    .await?;
    step.session_id = created["sessionId"]
        .as_str()
        .context("Agent did not return a child session")?
        .into();
    store.record_step(step)?;
    // Explicit selection fails rather than letting new_session's defaults win.
    rpc(crate::agent_commands::set_model_command(
        step.model.clone(),
        step.session_id.clone(),
    ))
    .await?;
    let system_prompt = match step.phase.as_str() {
        "issue_mapping" => "You are a read-only Fusion moderator organizing supplied participant statements. Do not investigate, verify facts, introduce independent analysis or call tools. Follow only the organization task and submit once it is complete.",
        "synthesizing" => "You are a read-only Fusion moderator synthesizing supplied participant reports. Answer the original topic from that material only. Do not investigate, introduce independent findings or call tools.",
        "preflight" => "Follow only the readiness-check request. Do not investigate or call tools.",
        _ => "You are a read-only Fusion deliberation participant. Never act on the workspace or external systems. Follow the focused decision task and use only the provided snapshot tools.",
    };
    rpc(RpcCommand {
        system_prompt: system_prompt.into(),
        ..command("set_system_prompt", &step.session_id)
    })
    .await?;
    let capacity = step
        .settings
        .as_ref()
        .context("Missing report settings")?
        .max_public_bytes;
    let input = step.input.clone();
    let phase = step.phase.clone();
    let participant = step.participant.clone();
    let original_run_id = step.agent_run_id.clone();
    let mut child = ChildSession {
        ctx,
        step,
        generation: 0,
    };
    let report = reports::collect(&mut child, input, &phase, capacity, |text| {
        validate_step_output(&phase, text, ctx.config.panel.len())?;
        if phase == "issue_mapping" {
            mapping::validate_sources(text, source_prompt)?;
        } else if phase == "reviewing" {
            review::validate(text, source_prompt, &participant)?;
        }
        Ok(())
    })
    .await?;
    if !report.validation_errors.is_empty() {
        let step = &mut *child.step;
        if report.generation == 0 {
            // A failed repair may still own an accepted Agent request. Fence it
            // before committing the retained original as a completed step.
            stop_before_retry(step).await?;
        }
        step.report_quality = Some(future_fusion::ReportQuality {
            validation_errors: report.validation_errors.clone(),
            source_run_id: if report.generation == 0 {
                original_run_id
            } else {
                step.agent_run_id.clone()
            },
        });
        ctx.note(
            "report_unvalidated",
            &phase,
            Some(&participant),
            Some(FailureKind::InvalidOutput),
        )?;
    }
    Ok(report)
}

struct ChildSession<'a, 'b, 'c> {
    ctx: &'a Runtime<'b>,
    step: &'c mut Step,
    generation: u32,
}
impl reports::ReportSession for ChildSession<'_, '_, '_> {
    async fn generate(&mut self, message: String, report_only: bool) -> Result<String> {
        let ctx = self.ctx;
        let step = &mut *self.step;
        ctx.ensure_provider(&step.model)?;
        if report_only {
            let settings = step.settings.as_mut().context("Missing report settings")?;
            settings.report_revision = settings.report_revision.saturating_add(1);
            step.agent_run_id = format!("run_{}", uuid::Uuid::new_v4());
        }
        let request_id = format!("{}-generation-{}", step.id, self.generation);
        self.generation = self.generation.saturating_add(1);
        // Persist the new run binding before admission. Session/model/attempt
        // stay unchanged; events retain every revision and owned Agent run.
        ctx.store.record_step(step)?;
        let accepted = rpc(RpcCommand {
            message,
            requested_run_id: step.agent_run_id.clone(),
            client_request_id: request_id,
            busy_policy: "enqueue_if_busy".into(),
            ..command("prompt", &step.session_id)
        })
        .await?;
        ensure!(
            accepted["run_id"].as_str() == Some(&step.agent_run_id),
            "Agent accepted a different Fusion run"
        );
        step.status = "running".into();
        ctx.store.record_step(step)?;
        loop {
            if STOPPING.load(Ordering::Acquire) || ctx.store.get(&ctx.run.id)?.terminal() {
                let _ = abort(step).await;
                anyhow::bail!("Fusion cancelled");
            }
            let state = rpc(RpcCommand {
                run_id: step.agent_run_id.clone(),
                ..command("get_state", &step.session_id)
            })
            .await?;
            if let Some(status) = state["requestedRun"]["status"].as_str() {
                // Session usage includes investigation and every report revision,
                // including billed cost and cache categories.
                step.usage = state["usage"].clone();
                match status {
                    "completed" => {
                        crate::request_scope::release_run(&step.session_id, &step.agent_run_id);
                        let answer =
                            rpc(command("get_last_assistant_text", &step.session_id)).await?;
                        let text = answer["text"].as_str().ok_or_else(|| {
                            failure(FailureKind::InvalidOutput, "Missing Fusion model response")
                        })?;
                        if text.trim().is_empty() {
                            return Err(failure(
                                FailureKind::InvalidOutput,
                                "Empty Fusion model response",
                            ));
                        }
                        return Ok(text.into());
                    }
                    "failed"
                    | "error"
                    | "cancelled"
                    | "interrupted"
                    | "interrupted_by_restart"
                    | "incomplete" => {
                        anyhow::bail!("Inner model {status}: {}", state["requestedRun"]["error"])
                    }
                    _ => {}
                }
            }
            ctx.ensure_provider(&step.model)?;
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }
}

/// Shared UI read adapter; the Agent validates parent source and step ownership.
pub async fn query(
    session_id: String,
    run_id: String,
    tool_call_id: String,
    step_id: Option<String>,
    offset: Option<i64>,
) -> Result<Value, crate::Error> {
    let mut client = crate::agent_client::connect_agent().await?;
    crate::agent_bridge::verify_session_database(&mut client, &session_id).await?;
    let response = client
        .execute_command(RpcCommand {
            run_id,
            tool_call_id: Some(tool_call_id),
            entry_id: step_id.unwrap_or_default(),
            offset,
            ..command("get_fusion", &session_id)
        })
        .await
        .map_err(|e| crate::Error::Message(e.to_string()))?
        .into_inner();
    ensure_response(response)
}
fn ensure_response(response: future_rpc::proto::RpcResponse) -> Result<Value, crate::Error> {
    if !response.success {
        return Err(crate::Error::Message(response.error));
    }
    Ok(future_rpc::decode::response_data(&response))
}
pub async fn query_for_run(
    run_id: String,
    tool_call_id: String,
    step_id: Option<String>,
    offset: Option<i64>,
) -> Result<Value, crate::Error> {
    let run = crate::store::get_run(&run_id)?
        .ok_or_else(|| crate::Error::Message("Parent run not found".into()))?;
    let thread = crate::store::get_thread(&run.thread_id)?
        .ok_or_else(|| crate::Error::Message("Parent thread not found".into()))?;
    query(
        thread.agent_session_id.unwrap_or(thread.id),
        run_id,
        tool_call_id,
        step_id,
        offset,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn attempt_templates_preserve_thinking_and_defer_capacity_allocation() {
        let mut config = Configuration::default();
        // Persisted pre-change budgets must not be reapplied.
        config.moderator.max_output_tokens = 16_384;
        config.moderator.retry_max_output_tokens = 32_768;
        for model in config.panel.iter().chain([&config.moderator]) {
            let first = attempt_settings(&config, model, None, 0).unwrap();
            let retry =
                attempt_settings(&config, model, Some(RetryReason::OutputLimit), 10_000).unwrap();
            assert_eq!(first.max_output_tokens, 0);
            assert_eq!(retry.max_output_tokens, 0);
            assert_eq!(retry.thinking_level, first.thinking_level);
            assert_eq!(retry.retry_reason.as_deref(), Some("output_limit"));
            let transient =
                attempt_settings(&config, model, Some(RetryReason::Transient), 10_000).unwrap();
            assert_eq!(transient.max_output_tokens, first.max_output_tokens);
        }
    }

    #[test]
    fn synthesis_accepts_natural_text_without_a_fixed_format_or_word_limit() {
        let text = "Public conclusion. ".repeat(10000);
        validate_step_output("synthesizing", &text, 3).unwrap();
    }

    #[test]
    fn no_retry_for_timeout_cancellation_auth_or_unknown_failures() {
        for failure in [
            "Fusion model timed out",
            "Fusion cancelled",
            "Authentication failed (HTTP 401)",
            "HTTP 429 insufficient_quota",
            "HTTP 403 model forbidden",
            "unclassified failure",
        ] {
            assert!(retry_for_kind(FailureKind::classify(failure)).is_none());
        }
        assert!(matches!(
            retry_for_kind(FailureKind::classify(
                "Inner model interrupted: [OUTPUT_LIMIT] finish_length"
            )),
            Some(RetryReason::OutputLimit)
        ));
        assert!(matches!(
            retry_for_kind(FailureKind::classify("[UPSTREAM_DISCONNECTED]")),
            Some(RetryReason::Transient)
        ));
    }

    #[test]
    fn attempt_deadline_cannot_reset_the_discussion_budget() {
        let config = Configuration::default();
        let limit = config.run_timeout_secs * 1000;
        assert_eq!(
            attempt_settings(&config, &config.panel[0], None, limit - 500)
                .unwrap()
                .timeout_ms,
            500
        );
        assert!(attempt_settings(&config, &config.panel[0], None, limit).is_err());
    }

    #[test]
    fn rejects_invalid_moderator_output_before_recording_completion() {
        assert!(validate_step_output(
            "issue_mapping",
            r#"{"consensus":[],"issues":[],"excluded":[]}"#,
            3
        )
        .is_ok());
        assert!(validate_step_output(
            "issue_mapping",
            r#"{"consensus":[],"issues":[],"excluded":[],"blind_spots":["new idea"]}"#,
            3
        )
        .is_err());
        assert!(validate_step_output("synthesizing", "   ", 3).is_err());
        assert!(
            validate_step_output("synthesizing", "A direct answer with essential facts.", 3)
                .is_ok()
        );
    }
}
