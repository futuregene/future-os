//! Thinking-level behavior end to end through the real loader + adapters.
//!
//! The contract: `thinkingLevels` is the set of levels the model offers (the
//! pickers show exactly this list). `default` is implicit (always first, never
//! stored) and sends no thinking parameter; `off` sends the protocol's explicit
//! disable spelling; every other value travels verbatim — unless the model
//! declares `compat.reasoningEffort`, which names the values its access path
//! actually accepts and onto which each level is mapped (see
//! `ReasoningEffortVocabulary`). `reasoning` is legacy metadata with no
//! behavior.

use super::*;
use crate::llm::{
    adapters::AdapterRegistry,
    schema::{
        clamp_thinking_level, effort_wire_value, parse_reasoning_effort_vocabulary, ApiProtocol,
        ModelRequest, ProtocolConfig, ResolvedModelTarget, DEFAULT_THINKING_LEVEL,
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

fn request_at_level(model: &Model, level: &str) -> Value {
    let mut target =
        ResolvedModelTarget::from_model(model, String::new(), None, Some(16384)).unwrap();
    target.generation.thinking_level = level.to_string();
    target.generation.thinking_budget = match level {
        "minimal" => 2000,
        "low" => 4000,
        "medium" => 8000,
        "high" => 16000,
        "xhigh" | "max" => 24000,
        "off" | "default" | "" => 0,
        _ => 8000,
    };
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

fn levels(model: &Model) -> Vec<String> {
    model.allowed_thinking_levels()
}

// ─── effective list ──────────────────────────────────────────────────────

#[test]
fn unconfigured_model_offers_only_default() {
    let model = configured_model("openai-completions", json!({"id": "unlisted"}));
    assert_eq!(levels(&model), vec![DEFAULT_THINKING_LEVEL.to_string()]);
    assert!(model.clamp_thinking_level("high") == DEFAULT_THINKING_LEVEL);
}

#[test]
fn reasoning_flag_has_no_effect_on_the_offered_levels() {
    // The legacy flag is preserved in the payload but no longer widens or
    // narrows what a model offers.
    let model = configured_model(
        "openai-completions",
        json!({"id": "legacy", "reasoning": false, "thinkingLevels": ["low", "high"]}),
    );
    assert!(!model.reasoning);
    assert_eq!(
        levels(&model),
        vec![
            DEFAULT_THINKING_LEVEL.to_string(),
            "low".to_string(),
            "high".to_string()
        ]
    );
}

#[test]
fn provider_level_levels_are_inherited_and_a_model_may_override() {
    let inherited = configured_model_with_provider(
        json!({"thinkingLevels": ["low", "high"]}),
        json!({"id": "inherited"}),
    );
    assert_eq!(
        levels(&inherited),
        vec![
            DEFAULT_THINKING_LEVEL.to_string(),
            "low".to_string(),
            "high".to_string()
        ]
    );

    let overridden = configured_model_with_provider(
        json!({"thinkingLevels": ["low", "high"]}),
        json!({"id": "overridden", "thinkingLevels": ["max"]}),
    );
    assert_eq!(
        levels(&overridden),
        vec![DEFAULT_THINKING_LEVEL.to_string(), "max".to_string()]
    );
}

#[test]
fn an_explicit_empty_list_means_only_default() {
    let model = configured_model_with_provider(
        json!({"thinkingLevels": ["low", "high"]}),
        json!({"id": "cleared", "thinkingLevels": []}),
    );
    assert_eq!(levels(&model), vec![DEFAULT_THINKING_LEVEL.to_string()]);
}

#[test]
fn levels_are_normalized_into_the_fixed_display_order() {
    let model = configured_model(
        "openai-completions",
        json!({"id": "messy", "thinkingLevels": ["high", " default ", "", "high", "xhigh", "low", "max"]}),
    );
    assert_eq!(
        levels(&model),
        vec![
            DEFAULT_THINKING_LEVEL.to_string(),
            "low".to_string(),
            "high".to_string(),
            "xhigh".to_string(),
            "max".to_string(),
        ],
        "known levels follow the product scale; `default` is implicit and duplicates collapse"
    );
}

#[test]
fn unknown_wire_values_keep_their_order_after_known_levels() {
    let model = configured_model(
        "openai-completions",
        json!({"id": "custom-vocab-order", "thinkingLevels": ["turbo", "low", "deep"]}),
    );
    assert_eq!(
        levels(&model),
        vec![
            DEFAULT_THINKING_LEVEL.to_string(),
            "low".to_string(),
            "turbo".to_string(),
            "deep".to_string(),
        ]
    );
}

#[test]
fn unknown_wire_values_are_accepted() {
    let model = configured_model(
        "openai-completions",
        json!({"id": "custom-vocab", "thinkingLevels": ["turbo", "deep"]}),
    );
    assert_eq!(
        levels(&model),
        vec![
            DEFAULT_THINKING_LEVEL.to_string(),
            "turbo".to_string(),
            "deep".to_string()
        ]
    );
    let body = request_at_level(&model, "turbo");
    assert_eq!(body["reasoning_effort"], json!("turbo"));
}

#[test]
fn clamp_keeps_offered_levels_and_falls_back_to_default() {
    let model = configured_model(
        "openai-completions",
        json!({"id": "declared", "thinkingLevels": ["low", "high"]}),
    );
    assert_eq!(model.clamp_thinking_level("high"), "high");
    assert_eq!(model.clamp_thinking_level("off"), DEFAULT_THINKING_LEVEL);
    assert_eq!(model.clamp_thinking_level("medium"), DEFAULT_THINKING_LEVEL);
    assert_eq!(
        model.clamp_thinking_level(DEFAULT_THINKING_LEVEL),
        DEFAULT_THINKING_LEVEL
    );
}

// ─── wire behavior ──────────────────────────────────────────────────────

#[test]
fn default_sends_no_thinking_parameter_for_any_chat_format() {
    for format in [
        "",
        "openai",
        "qwen",
        "qwen-chat-template",
        "deepseek",
        "zai",
        "reasoning-split",
    ] {
        let entry = if format.is_empty() {
            json!({"id": format!("default-{format}"), "thinkingLevels": ["high"]})
        } else {
            json!({"id": format!("default-{format}"), "thinkingLevels": ["high"], "compat": {"thinkingFormat": format}})
        };
        let model = configured_model("openai-completions", entry);
        let body = request_at_level(&model, DEFAULT_THINKING_LEVEL);
        for key in [
            "thinking",
            "reasoning_effort",
            "enable_thinking",
            "chat_template_kwargs",
            "reasoning_split",
            "reasoning",
        ] {
            assert!(
                body.get(key).is_none(),
                "format {format:?} must not send {key}: {body}"
            );
        }
    }
}

#[test]
fn a_declared_value_travels_verbatim_for_each_chat_format() {
    for (format, expected_patch) in [
        ("", json!({"reasoning_effort": "max"})),
        ("openai", json!({"reasoning_effort": "max"})),
        (
            "deepseek",
            json!({"thinking": {"type": "enabled"}, "reasoning_effort": "max"}),
        ),
        (
            "zai",
            json!({"enable_thinking": true, "reasoning_effort": "max"}),
        ),
        (
            "reasoning-split",
            json!({"thinking": "enabled", "reasoning_split": true}),
        ),
    ] {
        let entry = if format.is_empty() {
            json!({"id": format!("wire-{format}"), "thinkingLevels": ["max"]})
        } else {
            json!({"id": format!("wire-{format}"), "thinkingLevels": ["max"], "compat": {"thinkingFormat": format}})
        };
        let model = configured_model("openai-completions", entry);
        let body = request_at_level(&model, "max");
        for (key, value) in expected_patch.as_object().unwrap() {
            assert_eq!(body.get(key), Some(value), "format {format:?} key {key}");
        }
    }
}

#[test]
fn off_sends_each_protocols_disable_spelling() {
    for (format, expected) in [
        ("deepseek", json!({"thinking": {"type": "disabled"}})),
        ("qwen", json!({"enable_thinking": false})),
        ("zai", json!({"enable_thinking": false})),
        ("reasoning-split", json!({"thinking": "disabled"})),
    ] {
        let model = configured_model(
            "openai-completions",
            json!({"id": format!("off-{format}"), "thinkingLevels": ["off"], "compat": {"thinkingFormat": format}}),
        );
        let body = request_at_level(&model, "off");
        for (key, value) in expected.as_object().unwrap() {
            assert_eq!(body.get(key), Some(value), "format {format} key {key}");
        }
        assert!(
            body.get("reasoning_effort").is_none(),
            "off sends no effort"
        );
    }
}

#[test]
fn responses_sends_effort_verbatim_and_omits_default_and_off() {
    let model = configured_model(
        "openai-responses",
        json!({"id": "resp", "thinkingLevels": ["high", "max"]}),
    );
    assert_eq!(
        request_at_level(&model, "max")["reasoning"]["effort"],
        json!("max")
    );
    for level in [DEFAULT_THINKING_LEVEL, "off"] {
        let body = request_at_level(&model, level);
        assert!(body.get("reasoning").is_none(), "{level} must send none");
    }
}

#[test]
fn anthropic_adaptive_sends_the_declared_effort_verbatim() {
    let model = configured_model(
        "anthropic",
        json!({"id": "claude-opus-4-6", "thinkingLevels": ["low", "max"]}),
    );
    let body = request_at_level(&model, "max");
    assert_eq!(body["thinking"]["type"], "adaptive");
    assert_eq!(body["output_config"]["effort"], json!("max"));
}

#[test]
fn anthropic_manual_uses_the_budget_mapping() {
    let model = configured_model(
        "anthropic",
        json!({"id": "claude-manual", "thinkingLevels": ["high"]}),
    );
    let body = request_at_level(&model, "high");
    assert_eq!(body["thinking"]["type"], "enabled");
    // `high` -> 16k budget, clamped below max_tokens by the adapter.
    assert_eq!(body["thinking"]["budget_tokens"], json!(16000));

    let body = request_at_level(&model, DEFAULT_THINKING_LEVEL);
    assert!(body.get("thinking").is_none());
}

#[test]
fn thinking_format_is_still_derived_from_supported_parameters() {
    let qwen = configured_model(
        "openai-completions",
        json!({"id": "qwen", "thinkingLevels": ["high"], "supportedParameters": ["enable_thinking"]}),
    );
    assert_eq!(
        qwen.compat.get("thinkingFormat").and_then(Value::as_str),
        Some("qwen")
    );

    let deepseek = configured_model(
        "openai-completions",
        json!({"id": "deepseek", "thinkingLevels": ["high"], "supportedParameters": ["reasoning_effort"]}),
    );
    assert_eq!(
        deepseek
            .compat
            .get("thinkingFormat")
            .and_then(Value::as_str),
        Some("deepseek")
    );
}

#[test]
fn max_token_fallback_uses_the_declared_levels() {
    let thinking = Model {
        thinking_levels: vec!["high".to_string()],
        ..Default::default()
    };
    let plain = Model::default();
    assert_eq!(effective_max_tokens(&thinking), 32000);
    assert_eq!(effective_max_tokens(&plain), 16384);
}

#[test]
fn clamp_helper_matches_the_model_method() {
    let allowed = vec![
        DEFAULT_THINKING_LEVEL.to_string(),
        "low".to_string(),
        "high".to_string(),
    ];
    assert_eq!(clamp_thinking_level("high", &allowed), "high");
    assert_eq!(
        clamp_thinking_level("medium", &allowed),
        DEFAULT_THINKING_LEVEL
    );
    assert_eq!(clamp_thinking_level("", &allowed), DEFAULT_THINKING_LEVEL);
}

// ─── declared effort vocabularies ───────────────────────────────────────

/// The shape that produced the live failure: `westlakehpc/deepseek-flash`
/// accepts `low|high|xhigh|max`, or an integer 1..=100 that the gateway only
/// validates inside `chat_template_kwargs`. Sending `medium` verbatim answered
///
/// ```text
/// HTTP 400: DeepSeek V4.1 reasoning_effort must be low, high, xhigh, max,
/// or an integer within [1, 100] in chat_template_kwargs
/// ```
fn deepseek_v4_entry() -> Value {
    json!({
        "id": "deepseek-flash",
        "thinkingLevels": ["off", "low", "medium", "high", "xhigh", "max"],
        "compat": {
            "thinkingFormat": "deepseek",
            "reasoningEffort": {
                "values": ["low", "high", "xhigh", "max"],
                "integer": {"min": 1, "max": 100},
                "param": "chat_template_kwargs.reasoning_effort"
            }
        }
    })
}

#[test]
fn a_declared_vocabulary_replaces_a_spelling_the_provider_refuses() {
    let model = configured_model("openai-completions", deepseek_v4_entry());
    assert_eq!(
        levels(&model),
        vec![
            DEFAULT_THINKING_LEVEL.to_string(),
            "off".to_string(),
            "low".to_string(),
            "medium".to_string(),
            "high".to_string(),
            "xhigh".to_string(),
            "max".to_string(),
        ],
        "the middle level stays selectable — that is the whole point"
    );
    // The session clamps to this list, so `medium` survives clamping.
    assert_eq!(model.clamp_thinking_level("medium"), "medium");

    // A spelled level travels as itself, inside the declared parameter.
    let body = request_at_level(&model, "low");
    assert_eq!(body["thinking"], json!({"type": "enabled"}));
    assert_eq!(
        body["chat_template_kwargs"]["reasoning_effort"],
        json!("low")
    );
    assert!(body.get("reasoning_effort").is_none());

    // The middle level is sent as the integer the provider accepts, rather
    // than a spelling it refuses with a 400.
    let body = request_at_level(&model, "medium");
    assert_eq!(body["chat_template_kwargs"]["reasoning_effort"], json!(50));

    // `off` still disables thinking and sends no effort at all.
    let body = request_at_level(&model, "off");
    assert_eq!(body["thinking"], json!({"type": "disabled"}));
    assert!(body.get("chat_template_kwargs").is_none());
    assert!(body.get("reasoning_effort").is_none());

    // Every level the model offers lands on a value the declaration accepts.
    let vocabulary = parse_reasoning_effort_vocabulary(&model)
        .unwrap()
        .expect("declared vocabulary");
    for level in levels(&model) {
        if let Some(wire) = effort_wire_value(&level, Some(&vocabulary)) {
            assert!(
                vocabulary.accepts(&wire),
                "`{level}` travels as {wire}, which the declaration does not accept"
            );
        }
    }
}

#[test]
fn a_vocabulary_without_an_integer_range_snaps_to_a_spelling() {
    // A provider that names only two spellings and takes no number: the middle
    // level cannot be invented, so it snaps to the closest accepted one.
    let model = configured_model(
        "openai-completions",
        json!({
            "id": "names-only",
            "thinkingLevels": ["minimal", "low", "medium", "high", "xhigh"],
            "compat": {"reasoningEffort": {"values": ["low", "high"]}}
        }),
    );
    assert_eq!(
        request_at_level(&model, "medium")["reasoning_effort"],
        json!("low")
    );
    assert_eq!(
        request_at_level(&model, "high")["reasoning_effort"],
        json!("high")
    );
    assert_eq!(
        request_at_level(&model, "minimal")["reasoning_effort"],
        json!("low")
    );
    assert_eq!(
        request_at_level(&model, "xhigh")["reasoning_effort"],
        json!("high")
    );
}

#[test]
fn a_vocabulary_works_for_every_chat_format_and_for_responses() {
    // The declaration is data about the access path, not about one thinking
    // format: each format keeps its own toggle and places the value where the
    // declaration says (or at `reasoning.effort` for the Responses API).
    for format in ["openai", "qwen", "qwen-chat-template", "deepseek", "zai"] {
        let model = configured_model(
            "openai-completions",
            json!({
                "id": format!("vocab-{format}"),
                "thinkingLevels": ["low", "medium", "high"],
                "compat": {
                    "thinkingFormat": format,
                    "reasoningEffort": {
                        "values": ["low", "high"],
                        "integer": {"min": 1, "max": 100}
                    }
                }
            }),
        );
        let body = request_at_level(&model, "medium");
        assert_eq!(
            body["reasoning_effort"],
            json!(50),
            "format {format:?} must map `medium` instead of sending it"
        );
    }

    let responses = configured_model(
        "openai-responses",
        json!({
            "id": "vocab-responses",
            "thinkingLevels": ["low", "medium", "high"],
            "compat": {"reasoningEffort": {
                "values": ["low", "high"],
                "integer": {"min": 1, "max": 100}
            }}
        }),
    );
    assert_eq!(
        request_at_level(&responses, "medium")["reasoning"]["effort"],
        json!(50)
    );
}

/// The regression this guards: "I forgot to declare the vocabulary" is only a
/// *choice* while every offered level is sent as itself. Every registered model
/// is walked at every level it offers, and the value that actually goes on the
/// wire has to be one the model declares it accepts — so adding a model whose
/// levels the provider refuses turns this red before a user meets the 400.
#[test]
fn every_registered_model_at_every_level_sends_only_declared_values() {
    /// Effort values across every place a level can end up.
    fn sent_effort(body: &Value) -> Option<Value> {
        let nested = |path: &[&str]| -> Option<Value> {
            let mut value = body;
            for key in path {
                value = value.get(key)?;
            }
            Some(value.clone())
        };
        nested(&["reasoning_effort"])
            .or_else(|| nested(&["chat_template_kwargs", "reasoning_effort"]))
            .or_else(|| nested(&["reasoning", "effort"]))
    }

    let mut models = builtin_models();
    // User-shaped declarations (the shape the live failure had), through the
    // real loader.
    for entry in [
        deepseek_v4_entry(),
        json!({"id": "sweep-names", "thinkingLevels": ["low", "high"]}),
        json!({"id": "sweep-raw", "thinkingLevels": ["minimal", "low", "medium", "high"]}),
        json!({
            "id": "sweep-split",
            "thinkingLevels": ["low", "high"],
            "compat": {"thinkingFormat": "reasoning-split"}
        }),
    ] {
        models.push(configured_model("openai-completions", entry));
    }
    for entry in [
        json!({"id": "sweep-responses", "thinkingLevels": ["low", "medium", "max"]}),
        json!({
            "id": "sweep-responses-vocab",
            "thinkingLevels": ["low", "medium", "max"],
            "compat": {"reasoningEffort": {"values": ["low", "max"], "integer": {"min": 1, "max": 64}}}
        }),
    ] {
        models.push(configured_model("openai-responses", entry));
    }
    models.push(configured_model(
        "anthropic",
        json!({"id": "sweep-anthropic", "thinkingLevels": ["low", "medium", "max"]}),
    ));

    let mut checked = 0usize;
    let mut declared = 0usize;
    for model in &models {
        if ApiProtocol::from_str(&model.api).is_err() {
            continue; // an unknown api never reaches an adapter
        }
        let vocabulary = parse_reasoning_effort_vocabulary(model).unwrap();
        let target = ResolvedModelTarget::from_model(model, String::new(), None, Some(16384))
            .unwrap_or_else(|error| panic!("{}: {error}", model.id));
        let protocol = target.protocol.clone();
        let depth_is_a_budget = matches!(protocol, ProtocolConfig::AnthropicMessages(_));
        let format_has_no_depth = matches!(
            &protocol,
            ProtocolConfig::OpenAiChat(config)
                if matches!(config.reasoning, crate::llm::schema::ChatReasoningFormat::ReasoningSplit)
        );
        if vocabulary.is_some() {
            declared += 1;
        }

        for level in model.allowed_thinking_levels() {
            let body = request_at_level(model, &level);
            let sent = sent_effort(&body);
            let enabled = !level.is_empty() && level != DEFAULT_THINKING_LEVEL && level != "off";
            match &vocabulary {
                Some(vocabulary) => {
                    if let Some(sent) = &sent {
                        assert!(
                            vocabulary.accepts(sent),
                            "{} at `{level}` sends {sent}, which it does not declare \
                             (accepts {})",
                            model.id,
                            vocabulary.describe()
                        );
                    } else if enabled && !depth_is_a_budget && !format_has_no_depth {
                        // A declared level whose effort vanished is the silent
                        // failure this test exists to catch.
                        let wire = effort_wire_value(&level, Some(vocabulary));
                        panic!(
                            "{} at `{level}` sends no effort at all (mapping: {wire:?}); \
                             body: {body}",
                            model.id
                        );
                    }
                }
                None => {
                    if let Some(sent) = &sent {
                        assert_eq!(
                            sent,
                            &json!(level),
                            "{} at `{level}` has no declared vocabulary, so the level must \
                             travel verbatim",
                            model.id
                        );
                    }
                }
            }
            checked += 1;
        }
    }

    assert!(checked > 1_000, "walked too few levels: {checked}");
    assert!(
        declared >= 2,
        "walked too few vocabulary models: {declared}"
    );
}
