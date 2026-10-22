use super::*;
use crate::llm::{
    adapters::AdapterRegistry,
    schema::{
        clamp_thinking_level, normalize_declared_levels, ApiProtocol, ModelRequest,
        ReasoningEffortValues, ResolvedModelTarget, THINKING_LEVELS,
    },
};
use serde_json::{json, Value};
use std::str::FromStr;

fn configured_model(api: &str, entry: Value) -> Model {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models.json");
    std::fs::write(
        &path,
        json!({"providers": {"custom": {
            "api": api, "baseUrl": "https://example.test/v1", "models": [entry]
        }}})
        .to_string(),
    )
    .unwrap();
    let (mut models, _) = load_user_models_with_overrides(path.to_str().unwrap()).unwrap();
    enrich_user_models(&mut models, &builtin_models_shared());
    models.remove(0)
}

fn high_request(model: &Model) -> Value {
    request_at_level(model, "high")
}

fn request_at_level(model: &Model, level: &str) -> Value {
    let mut target =
        ResolvedModelTarget::from_model(model, String::new(), None, Some(16384)).unwrap();
    target.generation.thinking_level = level.to_string();
    target.generation.thinking_budget = 8192;
    AdapterRegistry::default()
        .get(ApiProtocol::from_str(&model.api).unwrap())
        .unwrap()
        .build_body(
            &target,
            &ModelRequest {
                model: model.id.clone(),
                system_prompt: String::new(),
                messages: vec![],
                tools: vec![],
            },
        )
        .unwrap()
}

fn configured_model_with_level_map(entry: Value, level_map: Value) -> Model {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models.json");
    std::fs::write(
        &path,
        json!({"providers": {"custom": {
            "api": "openai-completions",
            "baseUrl": "https://example.test/v1",
            "thinkingLevelMap": level_map,
            "models": [entry]
        }}})
        .to_string(),
    )
    .unwrap();
    let (mut models, _) = load_user_models_with_overrides(path.to_str().unwrap()).unwrap();
    enrich_user_models(&mut models, &builtin_models_shared());
    models.remove(0)
}

#[test]
fn custom_reasoning_defaults_on_and_sends_high_for_unlisted_responses_model() {
    let model = configured_model("openai-responses", json!({"id": "unlisted-deployment"}));
    assert!(model.reasoning);
    let body = high_request(&model);
    assert_eq!(body["reasoning"]["effort"], "high");
    assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
}

#[test]
fn custom_reasoning_false_survives_catalog_enrichment_and_omits_controls() {
    for (api, id, compat) in [
        ("openai-responses", "gpt-5.6-sol", json!({})),
        ("anthropic", "claude-opus-4-8", json!({})),
        (
            "anthropic",
            "claude-sonnet-4-5",
            json!({"anthropicThinkingMode": "manual"}),
        ),
        (
            "openai-completions",
            "deepseek-v4-pro",
            json!({"thinkingFormat": "deepseek"}),
        ),
        (
            "openai-completions",
            "local-qwen",
            json!({"thinkingFormat": "qwen-chat-template"}),
        ),
        (
            "openai-completions",
            "local-qwen",
            json!({"thinkingFormat": "qwen"}),
        ),
        (
            "openai-completions",
            "local-glm",
            json!({"thinkingFormat": "zai"}),
        ),
        (
            "openai-completions",
            "local-mini",
            json!({"thinkingFormat": "reasoning-split"}),
        ),
        (
            "openai-completions",
            "local-openai",
            json!({"thinkingFormat": "openai", "supportsReasoningEffort": true}),
        ),
        ("openai-completions", "local-chat", json!({})),
    ] {
        let model = configured_model(api, json!({"id": id, "reasoning": false, "compat": compat}));
        assert!(!model.reasoning, "{api}/{id}: explicit false must win");
        let body = high_request(&model);
        for key in [
            "reasoning",
            "reasoning_effort",
            "include",
            "thinking",
            "enable_thinking",
            "chat_template_kwargs",
            "reasoning_split",
            "output_config",
        ] {
            assert!(
                body.get(key).is_none(),
                "{api}/{id} unexpectedly sent {key}: {body}"
            );
        }
    }
}

#[test]
fn custom_reasoning_default_does_not_rewrite_non_openai_token_parameters() {
    for id in [
        "gpt-4.1",
        "deepseek-v4-pro",
        "Qwen3.8-27B-AWQ",
        "local-deployment",
    ] {
        let model = configured_model("openai-completions", json!({"id": id}));
        assert!(
            model.reasoning,
            "custom {id} defaults on regardless of catalog flag"
        );
        let body = high_request(&model);
        assert_eq!(body["max_tokens"], 16384, "{id}");
        assert!(body.get("max_completion_tokens").is_none(), "{id}");
    }
    let builtin = builtin_models_shared();
    assert!(
        builtin.iter().any(|m| m.id == "gpt-4.1" && !m.reasoning),
        "builtin flags remain authoritative"
    );
}

#[test]
fn custom_reasoning_true_sends_each_protocols_controls() {
    for (api, id, compat, pointer, expected) in [
        (
            "openai-completions",
            "local",
            json!({}),
            "/reasoning_effort",
            json!("high"),
        ),
        (
            "openai-completions",
            "local",
            json!({"thinkingFormat": "qwen-chat-template"}),
            "/chat_template_kwargs/enable_thinking",
            json!(true),
        ),
        (
            "anthropic",
            "claude-opus-4-8",
            json!({}),
            "/output_config/effort",
            json!("high"),
        ),
    ] {
        let model = configured_model(api, json!({"id": id, "reasoning": true, "compat": compat}));
        assert_eq!(
            high_request(&model).pointer(pointer),
            Some(&expected),
            "{api}"
        );
    }
}

#[test]
fn declared_effort_vocabulary_snaps_each_internal_level() {
    // A provider that accepts only low/high/xhigh/max (DeepSeek V4.1 style):
    // declaring the vocabulary is what keeps the internal "medium" from being
    // sent verbatim.
    let model = configured_model(
        "openai-completions",
        json!({"id": "local", "compat": {"reasoningEffortValues": ["low", "high", "xhigh", "max"]}}),
    );
    for (level, expected) in [
        ("minimal", "low"),
        ("low", "low"),
        ("medium", "high"),
        ("high", "high"),
        ("xhigh", "xhigh"),
    ] {
        assert_eq!(
            request_at_level(&model, level)["reasoning_effort"],
            json!(expected),
            "{level}"
        );
    }
    assert!(request_at_level(&model, "off")
        .get("reasoning_effort")
        .is_none());
}

#[test]
fn declared_effort_vocabulary_keeps_standard_levels_when_listed() {
    // A vocabulary that already contains "medium" must not snap it away.
    let model = configured_model(
        "openai-completions",
        json!({"id": "local", "compat": {"reasoningEffortValues": ["minimal", "low", "medium", "high"]}}),
    );
    for level in ["minimal", "low", "medium", "high"] {
        assert_eq!(
            request_at_level(&model, level)["reasoning_effort"],
            json!(level)
        );
    }
    // Deeper-than-standard levels clamp to the deepest declared value.
    assert_eq!(
        request_at_level(&model, "xhigh")["reasoning_effort"],
        json!("high")
    );
}

#[test]
fn undeclared_effort_uses_the_standard_enum() {
    let model = configured_model("openai-completions", json!({"id": "local"}));
    for level in ["minimal", "low", "medium", "high"] {
        assert_eq!(
            request_at_level(&model, level)["reasoning_effort"],
            json!(level),
            "standard level sent verbatim"
        );
    }
    assert_eq!(
        request_at_level(&model, "xhigh")["reasoning_effort"],
        json!("high"),
        "non-standard depth clamps to the strongest standard value"
    );
}

#[test]
fn effort_vocabulary_null_or_empty_disables_the_parameter() {
    for compat in [
        json!({"reasoningEffortValues": null}),
        json!({"reasoningEffortValues": []}),
    ] {
        let model = configured_model(
            "openai-completions",
            json!({"id": "local", "compat": compat}),
        );
        let body = request_at_level(&model, "high");
        assert!(
            body.get("reasoning_effort").is_none(),
            "expected no reasoning_effort for {body}"
        );
    }
}

#[test]
fn numeric_level_map_wins_over_the_vocabulary() {
    // thinkingLevelMap still wins: models that expose an integer effort range
    // can be steered exactly, and the value keeps its JSON number type.
    let model = configured_model_with_level_map(
        json!({
            "id": "local",
            "compat": {"reasoningEffortValues": ["low", "high", "xhigh", "max"]}
        }),
        json!({"medium": 50, "high": null}),
    );
    assert_eq!(
        request_at_level(&model, "medium")["reasoning_effort"],
        json!(50)
    );
    assert!(request_at_level(&model, "high")
        .get("reasoning_effort")
        .is_none());
}

#[test]
fn disabled_vocabulary_still_sends_the_thinking_toggle() {
    // "Do not send reasoning_effort" must not also disable thinking: the
    // provider-specific toggle (here DeepSeek's `thinking`) is independent.
    let model = configured_model(
        "openai-completions",
        json!({
            "id": "local",
            "compat": {"thinkingFormat": "deepseek", "reasoningEffortValues": null}
        }),
    );
    let body = request_at_level(&model, "high");
    assert_eq!(body["thinking"], json!({"type": "enabled"}));
    assert!(body.get("reasoning_effort").is_none());
}

#[test]
fn responses_effort_uses_the_same_resolution() {
    let model = configured_model(
        "openai-responses",
        json!({"id": "unlisted-deployment", "compat": {"reasoningEffortValues": ["low", "high"]}}),
    );
    assert_eq!(
        request_at_level(&model, "medium")["reasoning"]["effort"],
        json!("high")
    );
    assert_eq!(
        request_at_level(&model, "minimal")["reasoning"]["effort"],
        json!("low")
    );
}

#[test]
fn reasoning_effort_values_must_be_strings_or_numbers() {
    let model = configured_model(
        "openai-completions",
        json!({"id": "local", "compat": {"reasoningEffortValues": ["low", {"bogus": true}]}}),
    );
    let error = ResolvedModelTarget::from_model(&model, String::new(), None, None).unwrap_err();
    assert!(
        error.to_string().contains("reasoningEffortValues"),
        "{error}"
    );
}

#[test]
fn effort_vocabulary_rejects_descending_known_levels() {
    // The list doubles as a depth scale for snapping, so a descending known
    // vocabulary would silently invert every snap decision.
    let model = configured_model(
        "openai-completions",
        json!({"id": "local", "compat": {"reasoningEffortValues": ["high", "low"]}}),
    );
    let error = ResolvedModelTarget::from_model(&model, String::new(), None, None).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("reasoningEffortValues"), "{message}");
    assert!(message.contains("ascending"), "{message}");
}

#[test]
fn effort_vocabulary_rejects_repeated_known_levels() {
    // A repeat is not strictly ascending either — it would make the snap
    // ambiguous about which entry the level maps to.
    let model = configured_model(
        "openai-completions",
        json!({"id": "local", "compat": {"reasoningEffortValues": ["low", "low"]}}),
    );
    let error = ResolvedModelTarget::from_model(&model, String::new(), None, None).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("reasoningEffortValues"), "{message}");
    assert!(message.contains("ascending"), "{message}");
}

#[test]
fn effort_vocabulary_mixed_names_and_numbers_parses() {
    // Numbers carry no static rank, so they cannot break the known-name
    // ordering check and stay legal between names.
    let model = configured_model(
        "openai-completions",
        json!({"id": "local", "compat": {"reasoningEffortValues": ["low", 50, "xhigh"]}}),
    );
    let target = ResolvedModelTarget::from_model(&model, String::new(), None, None)
        .expect("mixed vocabulary must parse");
    assert!(matches!(
        target.capabilities.reasoning.effort_values,
        ReasoningEffortValues::Values(_)
    ));
}

#[test]
fn thinking_level_map_rejects_boolean_values() {
    // resolve_effort sends mapped values verbatim; a boolean there is a
    // configuration typo and must fail at parse time, not in the request.
    let model = configured_model_with_level_map(json!({"id": "local"}), json!({"high": true}));
    let error = ResolvedModelTarget::from_model(&model, String::new(), None, None).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("thinkingLevelMap"), "{message}");
    assert!(
        message.contains("`custom`") && message.contains("`local`") && message.contains("`high`"),
        "error must name provider, model, and level: {message}"
    );
}

#[test]
fn provider_level_compat_carries_the_effort_vocabulary() {
    // The vocabulary is per provider as often as per model: declared once at
    // the provider level it must reach every model of that provider.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models.json");
    std::fs::write(
        &path,
        json!({"providers": {"custom": {
            "api": "openai-completions",
            "baseUrl": "https://example.test/v1",
            "compat": {"reasoningEffortValues": ["low", "high", "xhigh", "max"]},
            "models": [{"id": "local"}]
        }}})
        .to_string(),
    )
    .unwrap();
    let (mut models, _) = load_user_models_with_overrides(path.to_str().unwrap()).unwrap();
    assert_eq!(
        models[0].compat.get("reasoningEffortValues"),
        Some(&json!(["low", "high", "xhigh", "max"])),
        "provider-level compat must reach the model at load time"
    );
    enrich_user_models(&mut models, &builtin_models_shared());
    let model = models.remove(0);
    assert_eq!(
        request_at_level(&model, "medium")["reasoning_effort"],
        json!("high"),
        "medium snaps to the deeper neighbor on a tie"
    );
}

// ─── thinkingLevels: the product-level vocabulary a model offers ─────────

/// A user model loaded from a provider block exactly as `models.json` files it,
/// so the provider-level default and the model-level override are exercised
/// through the real loader rather than a hand-built `Model`.
fn configured_model_with_provider(mut provider: Value, entry: Value) -> Model {
    provider["api"] = json!("openai-completions");
    provider["baseUrl"] = json!("https://example.test/v1");
    provider["models"] = json!([entry]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models.json");
    std::fs::write(
        &path,
        json!({"providers": {"custom": provider}}).to_string(),
    )
    .unwrap();
    let (mut models, _) = load_user_models_with_overrides(path.to_str().unwrap()).unwrap();
    enrich_user_models(&mut models, &builtin_models_shared());
    models.remove(0)
}

fn all_six() -> Vec<String> {
    THINKING_LEVELS
        .iter()
        .map(|level| level.to_string())
        .collect()
}

#[test]
fn undeclared_thinking_levels_keep_all_six_and_clamp_nothing() {
    // The whole point of scoping this change: a model that says nothing about its
    // vocabulary behaves exactly as it did before `thinkingLevels` existed.
    let model = configured_model("openai-completions", json!({"id": "unlisted"}));
    assert_eq!(model.allowed_thinking_levels(), all_six());
    for level in THINKING_LEVELS {
        assert_eq!(
            clamp_thinking_level(level, &model.allowed_thinking_levels()),
            level,
            "{level}"
        );
    }
    // A name outside the scale is not a level this build can snap: it is passed
    // through, exactly as the request path does today.
    assert_eq!(
        clamp_thinking_level("ultra", &model.allowed_thinking_levels()),
        "ultra"
    );
}

#[test]
fn provider_level_thinking_levels_are_the_default_and_a_model_may_override() {
    let inherited = configured_model_with_provider(
        json!({"thinkingLevels": ["low", "high"]}),
        json!({"id": "local"}),
    );
    assert_eq!(
        inherited.allowed_thinking_levels(),
        vec!["low".to_string(), "high".to_string()]
    );

    let overridden = configured_model_with_provider(
        json!({"thinkingLevels": ["low", "high"]}),
        json!({"id": "local", "thinkingLevels": ["off", "xhigh"]}),
    );
    assert_eq!(
        overridden.allowed_thinking_levels(),
        vec!["off".to_string(), "xhigh".to_string()],
        "the model-level list wins over the provider default"
    );

    // An explicit empty list is a declaration too: "this model constrains
    // nothing", which is six levels — not "inherit the provider's two".
    let emptied = configured_model_with_provider(
        json!({"thinkingLevels": ["low", "high"]}),
        json!({"id": "local", "thinkingLevels": []}),
    );
    assert_eq!(emptied.allowed_thinking_levels(), all_six());
}

#[test]
fn a_model_without_thinking_controls_offers_only_off() {
    let bare = configured_model(
        "openai-completions",
        json!({"id": "plain", "reasoning": false}),
    );
    assert_eq!(bare.allowed_thinking_levels(), vec!["off".to_string()]);

    // A contradictory record (levels declared, thinking disabled) follows the
    // stronger statement: no thinking controls means `off` alone is reachable.
    let declared = configured_model(
        "openai-completions",
        json!({"id": "plain", "reasoning": false, "thinkingLevels": ["off", "high"]}),
    );
    assert_eq!(declared.allowed_thinking_levels(), vec!["off".to_string()]);
    assert_eq!(declared.clamp_thinking_level("high"), "off");
}

#[test]
fn a_declared_vocabulary_is_normalized_to_canonical_order() {
    let model = configured_model(
        "openai-completions",
        json!({"id": "local", "thinkingLevels": ["high", "off", "low"]}),
    );
    assert_eq!(
        model.allowed_thinking_levels(),
        vec!["off".to_string(), "low".to_string(), "high".to_string()],
        "the pickers and the cycle command walk the scale, not the file order"
    );

    // A typo is reported (to the log) and dropped rather than offered as a level
    // no i18n label or budget arm knows.
    let (levels, unknown) = normalize_declared_levels(["low", "mid", "high"]);
    assert_eq!(levels, vec!["low".to_string(), "high".to_string()]);
    assert_eq!(unknown, vec!["mid".to_string()]);
}

#[test]
fn clamp_snaps_to_the_nearest_declared_level_and_ties_go_shallow() {
    let allowed = vec!["off".to_string(), "low".to_string(), "high".to_string()];
    for (level, expected) in [
        ("off", "off"),
        ("minimal", "off"),
        // One step from both neighbours: the shallower one wins, because the
        // unmatched value was never a level this model let the user pick.
        ("medium", "low"),
        ("high", "high"),
        ("xhigh", "high"),
    ] {
        assert_eq!(clamp_thinking_level(level, &allowed), expected, "{level}");
    }

    // No off switch at all: the shallowest declared level is as low as it goes.
    let always_thinking = vec!["low".to_string(), "high".to_string()];
    assert_eq!(clamp_thinking_level("off", &always_thinking), "low");
    assert_eq!(clamp_thinking_level("medium", &always_thinking), "low");
    assert_eq!(clamp_thinking_level("xhigh", &always_thinking), "high");
}
