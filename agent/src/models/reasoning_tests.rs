use super::*;
use crate::llm::{
    adapters::AdapterRegistry,
    schema::{ApiProtocol, ModelRequest, ResolvedModelTarget},
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
    let mut target =
        ResolvedModelTarget::from_model(model, String::new(), None, Some(16384)).unwrap();
    target.generation.thinking_level = "high".into();
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
