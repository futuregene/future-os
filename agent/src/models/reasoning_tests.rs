//! Thinking-level behavior end to end through the real loader + adapters.
//!
//! The contract: `thinkingLevels` is the list of wire values the model accepts.
//! `default` is implicit (always first, never stored) and sends no thinking
//! parameter; `off` sends the protocol's explicit disable spelling; every other
//! value travels verbatim. `reasoning` is legacy metadata with no behavior.

use super::*;
use crate::llm::{
    adapters::AdapterRegistry,
    schema::{
        clamp_thinking_level, ApiProtocol, ModelRequest, ResolvedModelTarget,
        DEFAULT_THINKING_LEVEL,
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
