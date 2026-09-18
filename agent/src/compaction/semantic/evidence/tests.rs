use super::*;

struct CancelledSummary(Option<std::sync::Arc<AtomicBool>>);

#[async_trait::async_trait]
impl LLMProvider for CancelledSummary {
    async fn stream_model(
        &self,
        _request: crate::llm::schema::ModelRequest,
    ) -> anyhow::Result<tokio_stream::wrappers::ReceiverStream<crate::llm::schema::ModelStreamEvent>>
    {
        use crate::llm::schema::{FinishReason, ModelStreamEvent};
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        tx.try_send(ModelStreamEvent::TextDelta {
            id: "s".into(),
            text: "valid summary".into(),
        })
        .unwrap();
        let reason = if let Some(flag) = &self.0 {
            flag.store(true, Ordering::Relaxed);
            FinishReason::Stop
        } else {
            FinishReason::Cancelled
        };
        tx.try_send(ModelStreamEvent::Finish {
            reason,
            usage: None,
        })
        .unwrap();
        Ok(tokio_stream::wrappers::ReceiverStream::new(rx))
    }
}

#[tokio::test]
async fn cancelled_summary_never_becomes_a_deterministic_checkpoint() {
    let manager = ContextManager {
        enabled: true,
        reserve_tokens: 1600,
        keep_recent_tokens: 1000,
        context_window: 128_000,
        model: "m".into(),
    };
    let raw = vec![
        message("u", "user", vec![ContentBlock::text("keep my requirement")]),
        message("a", "assistant", vec![call("x", "config.json")]),
        message("t", "tool", vec![result("x", "cap=128MiB", false)]),
    ];
    // Both a provider cancellation and a user interrupt during a successful
    // response must propagate, not invoke the ordinary failure fallback.
    for user_interrupt in [false, true] {
        let flag = std::sync::Arc::new(AtomicBool::new(false));
        let provider = CancelledSummary(user_interrupt.then(|| flag.clone()));
        let fallback = AtomicBool::new(false);
        let result = manager
            .prepare_evidence_with_summary(
                crate::compaction::project_prompt_context(&raw, None, None, 128_000),
                &raw,
                CompactionTrigger::Manual,
                CompactionPhase::Standalone,
                None,
                &flag,
                None,
                Some(&provider),
                None,
                &[],
                None,
                Some(&|_| {
                    fallback.store(true, Ordering::Relaxed);
                }),
            )
            .await;
        assert!(matches!(result, Err(ContextError::Cancelled)), "{result:?}");
        assert!(!fallback.load(Ordering::Relaxed));
    }
}

#[test]
fn fork_remaps_index_references_without_rewriting_historical_text() {
    let original = "old-entry";
    let mut ids = HashMap::new();
    ids.insert(original.to_string(), "child-entry".to_string());
    for header in [HEADER, HEADER_WITH_SUMMARY] {
        let row = serde_json::json!({"entryId":original,"head":original,"tail":original,"target":original});
        let suffix = format!("\n\n{HANDOFF_SUMMARY_HEADER}\n\n{{\"entryId\":\"{original}\"}}\n\n[Retention note: historical {original}]");
        let note = format!("User compaction note (verbatim; the deterministic selector does not interpret it): {original}");
        let mut blocks = vec![ContentBlock::text(format!(
            "{header}\nCoverage cutoff: \"{original}\"\n{note}\n{row}{suffix}"
        ))];
        remap_evidence_references(&mut blocks, &ids).unwrap();
        let ContentBlock::Text { text } = &blocks[0] else {
            panic!("text expected")
        };
        assert!(text.contains("Coverage cutoff: \"child-entry\""));
        assert!(text.contains(&note));
        assert!(text.ends_with(&suffix));
        let mapped: serde_json::Value = serde_json::from_str(text.lines().nth(3).unwrap()).unwrap();
        assert_eq!(mapped["entryId"], "child-entry");
        for key in ["head", "tail", "target"] {
            assert_eq!(mapped[key], original);
        }
    }
}

#[test]
fn fork_rejects_dangling_evidence_references() {
    let ids = HashMap::from([("cutoff".into(), "child-cutoff".into())]);
    let mut summary = vec![ContentBlock::text(format!(
        "{HEADER}\nCoverage cutoff: \"cutoff\"\n{{\"entryId\":\"missing\"}}"
    ))];
    assert!(remap_evidence_references(&mut summary, &ids).is_none());
}

use serde_json::json;

fn message(id: &str, role: &str, content: Vec<ContentBlock>) -> AgentMessage {
    let mut message = AgentMessage {
        role: role.into(),
        content,
        ..Default::default()
    };
    message
        .metadata
        .get_or_insert_with(Default::default)
        .insert(AgentMessage::JOURNAL_ENTRY_ID_KEY.into(), json!(id));
    message
}
fn call(id: &str, path: &str) -> ContentBlock {
    ContentBlock::tool_call(id, "read", json!({"path":path}), Default::default())
}
fn result(id: &str, text: &str, error: bool) -> ContentBlock {
    ContentBlock::tool_result(id, text, error)
}
fn rows(text: &str) -> Vec<serde_json::Value> {
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

#[test]
fn evidence_is_bounded_scoped_unicode_safe_and_does_not_copy_hidden_content() {
    let huge = format!(
        "HEAD{}MIDDLE_SECRET{}TAIL",
        "中🙂".repeat(10000),
        "中🙂".repeat(10000)
    );
    let raw = vec![
        message(
            "call",
            "assistant",
            vec![
                ContentBlock::reasoning("THINKING_SECRET", Default::default()),
                call("x", r"C:\\data\\CONFIG.json"),
            ],
        ),
        message("result", "tool", vec![result("x", &huge, false)]),
        message("future-call", "assistant", vec![call("x", "future.txt")]),
        message(
            "future-result",
            "tool",
            vec![result("x", "FUTURE_SECRET", false)],
        ),
    ];
    let first = build(&raw, "result", None, 2048, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        first,
        build(&raw, "result", None, 2048, &AtomicBool::new(false)).unwrap()
    );
    assert!(estimate_text_tokens(&first) <= 2048);
    assert!(first.contains("HEAD") && first.contains("TAIL"));
    assert!(
        !first.contains("MIDDLE_SECRET")
            && !first.contains("THINKING_SECRET")
            && !first.contains("FUTURE_SECRET")
    );
    let rows = rows(&first);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["entryId"], "result");
    assert_eq!(rows[0]["middleOmitted"], true);
    assert!(important_path(r"C:\\data\\CONFIG.json"));
}

#[test]
fn repeated_and_ambiguous_tool_call_ids_never_misattribute_paths() {
    let raw = vec![
        message("a", "assistant", vec![call("same", "old.cfg")]),
        message("b", "tool", vec![result("same", "old", false)]),
        message("c", "assistant", vec![call("same", "new.cfg")]),
        message("d", "tool", vec![result("same", "new", false)]),
        message(
            "e",
            "assistant",
            vec![
                call("duplicate", "left.cfg"),
                call("duplicate", "right.cfg"),
            ],
        ),
        message(
            "f",
            "tool",
            vec![
                result("duplicate", "uncertain", false),
                result("duplicate", "also uncertain", false),
            ],
        ),
    ];
    let records = rows(&build(&raw, "f", None, 2048, &AtomicBool::new(false)).unwrap());
    assert_eq!(
        records.iter().find(|r| r["entryId"] == "b").unwrap()["target"],
        "old.cfg"
    );
    assert_eq!(
        records.iter().find(|r| r["entryId"] == "d").unwrap()["target"],
        "new.cfg"
    );
    for record in records.iter().filter(|r| r["entryId"] == "f") {
        assert!(record["target"].is_null());
    }
}

#[test]
fn errors_and_first_latest_config_evidence_survive_budgeted_selection() {
    let mut raw = Vec::new();
    for index in 0..40 {
        let id = format!("call-{index}");
        let path = if index == 0 || index == 39 {
            "settings.json".to_owned()
        } else {
            format!("trace-{index}.log")
        };
        raw.push(message(&id, "assistant", vec![call(&id, &path)]));
        raw.push(message(
            &format!("result-{index}"),
            "tool",
            vec![result(
                &id,
                &format!("value {index} {}", "x".repeat(2000)),
                index == 20,
            )],
        ));
    }
    let evidence = build(&raw, "result-39", None, 2048, &AtomicBool::new(false)).unwrap();
    let selected = rows(&evidence);
    for id in ["result-0", "result-20", "result-39"] {
        assert!(selected.iter().any(|row| row["entryId"] == id), "{id}");
    }
    assert!(estimate_text_tokens(&evidence) <= 2048);
    assert!(selected.len() < 40);
}

#[test]
fn current_c_rebuilds_old_evidence_without_using_a_prior_model_summary() {
    let mut raw = vec![
        message(
            "u",
            "user",
            vec![ContentBlock::text("keep original requirement")],
        ),
        message("a", "assistant", vec![call("x", "config.json")]),
        message("t", "tool", vec![result("x", "version=old", false)]),
    ];
    let manager = ContextManager {
        enabled: true,
        reserve_tokens: 1600,
        keep_recent_tokens: 1000,
        context_window: 8000,
        model: "m".into(),
    };
    let projected = super::super::super::project_prompt_context(&raw, None, None, 8000);
    let ContextPreparation::Compacted { checkpoint, .. } = prepare(
        &manager,
        projected,
        &raw,
        CompactionTrigger::Manual,
        CompactionPhase::Standalone,
        None,
        &AtomicBool::new(false),
        None,
    )
    .unwrap() else {
        panic!("checkpoint expected")
    };
    let mut old_a = *checkpoint;
    old_a.algorithm_version = "semantic-s2-v1".into();
    old_a.summary = vec![ContentBlock::text("PRIOR_MODEL_SUMMARY_SENTINEL")];
    raw.push(message("u2", "user", vec![ContentBlock::text("continue")]));
    raw.push(message("a2", "assistant", vec![call("y", "config.json")]));
    raw.push(message(
        "t2",
        "tool",
        vec![result("y", "version=new", false)],
    ));
    let projected = crate::compaction::project_prompt_context(&raw, Some(&old_a), None, 8000);
    let ContextPreparation::Compacted { checkpoint, .. } = prepare(
        &manager,
        projected,
        &raw,
        CompactionTrigger::Manual,
        CompactionPhase::Standalone,
        None,
        &AtomicBool::new(false),
        None,
    )
    .unwrap() else {
        panic!("checkpoint expected")
    };
    let text = checkpoint
        .summary
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert!(text.contains("version=old") && text.contains("version=new"));
    assert!(!text.contains("PRIOR_MODEL_SUMMARY_SENTINEL"));
    assert_eq!(checkpoint.algorithm_version, ALGORITHM_DETERMINISTIC);
    assert!(checkpoint.protected_entry_ids.contains(&"u".into()));
    assert_eq!(
        crate::session::checkpoint_to_entry(&checkpoint)
            .content
            .unwrap()["schema_version"],
        3
    );
}

#[test]
fn omitted_assistant_text_is_not_mislabeled_as_a_generated_summary() {
    let raw = vec![
        message("u", "user", vec![ContentBlock::text("keep original")]),
        message(
            "a",
            "assistant",
            vec![ContentBlock::text("large original output ".repeat(10000))],
        ),
    ];
    let manager = ContextManager {
        enabled: true,
        reserve_tokens: 1600,
        keep_recent_tokens: 1000,
        context_window: 8000,
        model: "m".into(),
    };
    let ContextPreparation::Compacted { checkpoint, .. } = prepare(
        &manager,
        crate::compaction::project_prompt_context(&raw, None, None, 8000),
        &raw,
        CompactionTrigger::Manual,
        CompactionPhase::Standalone,
        None,
        &AtomicBool::new(false),
        None,
    )
    .unwrap() else {
        panic!("C expected")
    };
    let text = checkpoint
        .summary
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert!(text.contains("not summarized"));
    assert_eq!(checkpoint.protected_entry_ids, vec!["u"]);
}

#[test]
fn notes_cancellation_and_missing_boundaries_are_explicit() {
    let raw = vec![message("u", "user", vec![ContentBlock::text("hello")])];
    assert!(build(&raw, "absent", None, 2048, &AtomicBool::new(false)).is_err());
    assert!(matches!(
        build(&raw, "u", None, 2048, &AtomicBool::new(true)),
        Err(ContextError::Cancelled)
    ));
    assert!(matches!(
        build(
            &raw,
            "u",
            Some(&"x".repeat(10000)),
            512,
            &AtomicBool::new(false)
        ),
        Err(ContextError::BudgetExceeded(_))
    ));
    assert!(build(
        &raw,
        "u",
        Some("do not deploy"),
        2048,
        &AtomicBool::new(false)
    )
    .unwrap()
    .contains("selector does not interpret"));
}

/// Confirms the suspicion directly: at the size where compaction actually fires, does
/// `fit_messages` truncate? Dropping from the middle breaks the cache prefix, so if it
/// truncates here the request stops being cache-served exactly when it matters.
#[test]
fn fit_messages_truncates_at_the_compaction_trigger() {
    let manager = ContextManager {
        enabled: true,
        reserve_tokens: 32_000,
        keep_recent_tokens: 4_000,
        context_window: 1_000_000,
        model: "m".into(),
    };
    // The trigger is the window minus the output reservation and margin; the same
    // quantity the runtime compares the estimate against.
    let margin = 2_048_u64.min(manager.context_window as u64 / 16);
    let trigger = (manager.context_window as u64)
        .saturating_sub(32_000)
        .saturating_sub(margin);
    // A conversation sitting exactly at the trigger threshold.
    let per_message = 2_000;
    let count = (trigger as usize / per_message).max(12);
    let messages: Vec<AgentMessage> = (0..count)
        .map(|_i| AgentMessage::new_user("user", serde_json::json!("x".repeat(per_message * 4))))
        .collect();
    let cost: u64 = messages
        .iter()
        .map(super::super::projected_token_cost_message)
        .sum();
    let fitted = fit_messages(messages.clone(), &manager, 2_048);
    eprintln!(
        "trigger={trigger} conversation={cost} messages={} fitted={}",
        messages.len(),
        fitted.len()
    );
    assert_eq!(
        fitted.len(),
        messages.len(),
        "a conversation that fits the window must not be truncated; truncating drops \
         the middle and destroys prefix-cache reuse"
    );
}

/// When the conversation genuinely exceeds one request, keep the leading originals
/// plus as much of the newest history as fits — not a hard-coded handful.
#[test]
fn fit_messages_keeps_the_newest_history_when_it_must_drop() {
    let manager = ContextManager {
        enabled: true,
        reserve_tokens: 32_000,
        keep_recent_tokens: 4_000,
        context_window: 1_000_000,
        model: "m".into(),
    };
    // Twice the window: dropping is unavoidable.
    let per_message = 2_000;
    let count = 1_000; // ~2M tokens
    let messages: Vec<AgentMessage> = (0..count)
        .map(|_i| AgentMessage::new_user("user", serde_json::json!("x".repeat(per_message * 4))))
        .collect();
    let fitted = fit_messages(messages.clone(), &manager, 2_048);
    eprintln!("messages={} fitted={}", messages.len(), fitted.len());
    assert!(
        fitted.len() > 8,
        "should keep far more than a token few messages: kept {}",
        fitted.len()
    );
    assert!(
        fitted.len() < messages.len(),
        "an oversized conversation must still be reduced"
    );
    // The kept messages must be the leading originals plus the newest ones.
    let kept_tokens: u64 = fitted
        .iter()
        .map(super::super::projected_token_cost_message)
        .sum();
    assert!(
        kept_tokens <= 1_000_000_u64 - 2_048 - 2_048,
        "the kept set must fit the window: {kept_tokens}"
    );
}

/// Returns whatever text it was constructed with, as one streamed delta.
struct ScriptedSummary(Option<&'static str>);

struct BudgetSummary {
    text: String,
    finish: crate::llm::schema::FinishReason,
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl LLMProvider for BudgetSummary {
    async fn stream_model(
        &self,
        _: crate::llm::schema::ModelRequest,
    ) -> anyhow::Result<tokio_stream::wrappers::ReceiverStream<crate::llm::schema::ModelStreamEvent>>
    {
        anyhow::bail!("must preserve the explicit summary output reservation")
    }

    async fn stream_model_with_output_limit(
        &self,
        request: crate::llm::schema::ModelRequest,
        limit: i32,
    ) -> anyhow::Result<tokio_stream::wrappers::ReceiverStream<crate::llm::schema::ModelStreamEvent>>
    {
        use crate::llm::schema::ModelStreamEvent;
        self.calls.fetch_add(1, Ordering::Relaxed);
        assert_eq!(
            limit, 8192,
            "do not squeeze the provider's reasoning allowance"
        );
        assert!(request
            .messages
            .last()
            .unwrap()
            .text()
            .contains("4096 tokens"));
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        tx.try_send(ModelStreamEvent::TextDelta {
            id: "s".into(),
            text: self.text.clone(),
        })?;
        tx.try_send(ModelStreamEvent::Finish {
            reason: self.finish.clone(),
            usage: Some(crate::types::Usage {
                prompt_tokens: 100_000,
                completion_tokens: 4181,
                reasoning_tokens: Some(516),
                credit_cost: Some(0.125),
                ..Default::default()
            }),
        })?;
        Ok(tokio_stream::wrappers::ReceiverStream::new(rx))
    }
}

#[tokio::test]
async fn summary_admission_uses_the_complete_projection_and_reports_fallbacks() {
    use crate::compaction::{set_request_budget, CompactionSummaryStatus};
    use crate::llm::schema::FinishReason;
    let manager = ContextManager {
        enabled: true,
        reserve_tokens: 25_600,
        keep_recent_tokens: 8000,
        context_window: 128_000,
        model: "m".into(),
    };
    let raw = vec![
        message(
            "u",
            "user",
            vec![ContentBlock::text("keep this exact requirement")],
        ),
        message(
            "a",
            "assistant",
            vec![
                ContentBlock::text("keep this exact conclusion"),
                call("x", "config.json"),
            ],
        ),
        message(
            "t",
            "tool",
            vec![result("x", &"version=old\n".repeat(40_000), true)],
        ),
    ];
    for (tokens, finish, expected, reason) in [
        (
            4111,
            FinishReason::Stop,
            CompactionSummaryStatus::Generated,
            None,
        ),
        (
            40_000,
            FinishReason::Stop,
            CompactionSummaryStatus::EvidenceOnly,
            Some("exceeding its admitted budget"),
        ),
        (
            4111,
            FinishReason::Length,
            CompactionSummaryStatus::EvidenceOnly,
            Some("provider length limit"),
        ),
        (
            0,
            FinishReason::Stop,
            CompactionSummaryStatus::EvidenceOnly,
            Some("before a complete response"),
        ),
    ] {
        let provider = BudgetSummary {
            text: "x".repeat(tokens * 4),
            finish,
            calls: Default::default(),
        };
        assert_eq!(estimate_text_tokens(&provider.text), tokens as u64);
        let fallbacks = std::sync::Mutex::new(Vec::new());
        let usage_count = std::sync::atomic::AtomicUsize::new(0);
        let mut prompt = crate::compaction::project_prompt_context(&raw, None, None, 128_000);
        set_request_budget(&mut prompt, "fixed input overhead", &[], 16_384);
        let ContextPreparation::Compacted { checkpoint, prompt } = manager
            .prepare_evidence_with_summary(
                prompt,
                &raw,
                CompactionTrigger::Manual,
                CompactionPhase::Standalone,
                None,
                &AtomicBool::new(false),
                None,
                Some(&provider),
                Some("fixed input overhead"),
                &[],
                Some(&|_| {
                    usage_count.fetch_add(1, Ordering::Relaxed);
                }),
                Some(&|reason| fallbacks.lock().unwrap().push(reason.to_string())),
            )
            .await
            .unwrap()
        else {
            panic!("checkpoint expected")
        };
        let outcome = checkpoint.summary_outcome.as_ref().unwrap();
        assert_eq!(outcome.status, expected);
        // Existing empty-response retries are unchanged; admission itself adds none.
        let attempts = if tokens == 0 { 3 } else { 1 };
        assert_eq!(provider.calls.load(Ordering::Relaxed), attempts);
        assert_eq!(
            usage_count.load(Ordering::Relaxed),
            attempts,
            "diagnostics must not double-charge"
        );
        assert_eq!(outcome.attempt_usage.len(), attempts);
        assert_eq!(outcome.attempt_usage[0].reasoning_tokens, Some(516));
        assert_eq!(outcome.attempt_usage[0].credit_cost, Some(0.125));
        assert_eq!(checkpoint.protected_entry_ids, vec!["u", "a"]);
        assert!(checkpoint.tokens_after < checkpoint.tokens_before);
        assert!(checkpoint.tokens_after <= manager.input_limit_for_usage(&prompt.usage));
        assert!(
            summary_text(&checkpoint).contains("version=old"),
            "evidence survives even on failure"
        );
        if let Some(reason) = reason {
            assert_eq!(fallbacks.lock().unwrap().len(), 1);
            assert!(
                outcome.fallback_reason.as_ref().unwrap().contains(reason),
                "{:?}",
                outcome.fallback_reason
            );
            assert_eq!(checkpoint.algorithm_version, ALGORITHM_DETERMINISTIC);
            assert!(!summary_text(&checkpoint).contains(HANDOFF_SUMMARY_HEADER));
        } else {
            assert!(fallbacks.lock().unwrap().is_empty());
            assert!(outcome.fallback_reason.is_none());
            assert_eq!(checkpoint.algorithm_version, ALGORITHM_SUMMARIZED);
            assert!(
                summary_text(&checkpoint).ends_with(&provider.text),
                "accepted summary is not truncated"
            );
        }
    }
}

#[async_trait::async_trait]
impl crate::types::LLMProvider for ScriptedSummary {
    async fn stream_model(
        &self,
        _request: crate::llm::schema::ModelRequest,
    ) -> anyhow::Result<tokio_stream::wrappers::ReceiverStream<crate::llm::schema::ModelStreamEvent>>
    {
        use crate::llm::schema::{FinishReason, ModelStreamEvent};
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        if let Some(text) = self.0 {
            let _ = tx
                .send(ModelStreamEvent::TextDelta {
                    id: "s".into(),
                    text: text.into(),
                })
                .await;
        }
        let _ = tx
            .send(ModelStreamEvent::Finish {
                reason: FinishReason::Stop,
                usage: None,
            })
            .await;
        Ok(tokio_stream::wrappers::ReceiverStream::new(rx))
    }
}

fn summary_text(checkpoint: &crate::compaction::ContextCheckpoint) -> String {
    checkpoint
        .summary
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The index header claims what the message contains, so it has to follow the outcome:
/// "no model summary was generated" is true for plain C and for a failed summary call, and
/// false as soon as one succeeds.
#[tokio::test]
async fn evidence_header_states_whether_a_summary_actually_accompanied_it() {
    let manager = ContextManager {
        enabled: true,
        reserve_tokens: 1600,
        keep_recent_tokens: 1000,
        context_window: 128_000,
        model: "m".into(),
    };
    let raw = vec![
        message(
            "u",
            "user",
            vec![ContentBlock::text("keep this requirement")],
        ),
        message("a", "assistant", vec![call("x", "config.json")]),
        message("t", "tool", vec![result("x", "cap=128MiB", false)]),
    ];

    async fn run(
        manager: &ContextManager,
        raw: &[AgentMessage],
        provider: Option<&dyn crate::types::LLMProvider>,
    ) -> ContextPreparation {
        let projected = crate::compaction::project_prompt_context(raw, None, None, 128_000);
        match provider {
            Some(provider) => manager
                .prepare_evidence_with_summary(
                    projected,
                    raw,
                    CompactionTrigger::Manual,
                    CompactionPhase::Standalone,
                    None,
                    &AtomicBool::new(false),
                    None,
                    Some(provider),
                    None,
                    &[],
                    None,
                    None,
                )
                .await
                .unwrap(),
            None => manager
                .prepare_evidence(
                    projected,
                    raw,
                    CompactionTrigger::Manual,
                    CompactionPhase::Standalone,
                    None,
                    &AtomicBool::new(false),
                    None,
                )
                .unwrap(),
        }
    }

    // Plain C: the index stands alone and says so.
    let ContextPreparation::Compacted { checkpoint, .. } = run(&manager, &raw, None).await else {
        panic!("checkpoint expected")
    };
    let text = summary_text(&checkpoint);
    assert_eq!(checkpoint.algorithm_version, ALGORITHM_DETERMINISTIC);
    assert!(
        text.starts_with("Deterministic tool-evidence index; no model summary was generated."),
        "{text}"
    );
    assert!(!text.contains("followed by a model-written handoff summary"));

    // A successful summary: the same index, headed by the sentence that matches it.
    let ContextPreparation::Compacted { checkpoint, .. } = run(
        &manager,
        &raw,
        Some(&ScriptedSummary(Some("## Objective\n- keep it"))),
    )
    .await
    else {
        panic!("checkpoint expected")
    };
    let text = summary_text(&checkpoint);
    assert_eq!(checkpoint.algorithm_version, ALGORITHM_SUMMARIZED);
    assert!(
        text.starts_with(
            "Deterministic tool-evidence index, followed by a model-written handoff summary of \
             the same history."
        ),
        "{text}"
    );
    assert!(
        !text.contains("no model summary was generated"),
        "the index must not deny the summary it carries: {text}"
    );
    assert!(text.contains(HANDOFF_SUMMARY_HEADER) && text.contains("## Objective"));

    // A failed summary falls back to plain C, and the header says so again.
    let ContextPreparation::Compacted { checkpoint, .. } =
        run(&manager, &raw, Some(&ScriptedSummary(None))).await
    else {
        panic!("checkpoint expected")
    };
    let text = summary_text(&checkpoint);
    assert_eq!(checkpoint.algorithm_version, ALGORITHM_DETERMINISTIC);
    assert!(
        text.starts_with("Deterministic tool-evidence index; no model summary was generated."),
        "{text}"
    );
    assert!(!text.contains(HANDOFF_SUMMARY_HEADER));
}
