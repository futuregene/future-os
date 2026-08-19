//! Tests for the provider view and write paths. The built-in catalog normally
//! arrives from the agent over the `list_models` RPC; tests inject a fixture
//! catalog into the synchronous `_with_catalog` cores instead, so nothing here
//! needs a running agent.

#![allow(clippy::await_holding_lock)]

use super::catalog::{future_models_cache_path, models_json_path, CatalogProviderSummary};
use super::write::{
    delete_custom_provider_with_catalog, set_builtin_provider_base_url_with_catalog,
    update_builtin_provider_key_with_catalog, upsert_custom_provider_with_catalog,
};
use super::*;
use crate::auth_store::test_support::HomeGuard;
use serde_json::json;
use std::collections::BTreeMap;

/// Fixture catalog standing in for the agent-provided one: a regular provider
/// with a real base URL and a placeholder provider that requires the user to
/// supply their own base URL.
fn fixture_catalog() -> BTreeMap<String, CatalogProviderSummary> {
    let mut catalog = BTreeMap::new();
    catalog.insert(
        "deepseek".to_string(),
        CatalogProviderSummary {
            name: "DeepSeek".to_string(),
            base_url: "https://api.deepseek.com/v1".to_string(),
            model_count: 3,
        },
    );
    catalog.insert(
        "azure-openai-responses".to_string(),
        CatalogProviderSummary {
            name: "Azure OpenAI Responses".to_string(),
            base_url: "https://YOUR_RESOURCE.openai.azure.com/openai".to_string(),
            model_count: 1,
        },
    );
    catalog
}

/// The view as rebuilt from the (HomeGuard-redirected) config files with a
/// fixture catalog — the test stand-in for `list_agent_providers`.
fn providers_view(catalog: &BTreeMap<String, CatalogProviderSummary>) -> ProvidersView {
    refresh_view_with_catalog(catalog).unwrap()
}

fn input(id: &str, name: &str, create: bool) -> UpsertCustomProviderInput {
    UpsertCustomProviderInput {
        id: id.to_string(),
        name: name.to_string(),
        api: "openai-completions".to_string(),
        base_url: "https://api.example.com/v1".to_string(),
        api_key: None,
        models: vec![],
        create,
    }
}

fn custom_model(id: &str, name: &str, supports_images: bool) -> CustomProviderModel {
    CustomProviderModel {
        id: id.to_string(),
        name: name.to_string(),
        supports_images,
        context_window: 128_000,
        max_tokens: 16_384,
    }
}

#[test]
fn create_rejects_existing_id() {
    let _home = HomeGuard::new("dup-id");
    let catalog = fixture_catalog();
    upsert_custom_provider_with_catalog(input("dashscope", "DashScope", true), &catalog).unwrap();
    // Re-creating the same id must fail rather than silently overwrite.
    let err = upsert_custom_provider_with_catalog(input("dashscope", "Other", true), &catalog)
        .unwrap_err();
    assert!(err.to_string().contains("already exists"));
}

#[test]
fn edit_allows_same_id() {
    let _home = HomeGuard::new("edit-id");
    let catalog = fixture_catalog();
    upsert_custom_provider_with_catalog(input("dashscope", "DashScope", true), &catalog).unwrap();
    // Editing (create = false) the same id is fine.
    let view =
        upsert_custom_provider_with_catalog(input("dashscope", "DashScope 2", false), &catalog)
            .unwrap();
    assert_eq!(view.custom.len(), 1);
    assert_eq!(view.custom[0].name, "DashScope 2");
}

#[test]
fn rejects_duplicate_name_case_insensitive() {
    let _home = HomeGuard::new("dup-name");
    let catalog = fixture_catalog();
    upsert_custom_provider_with_catalog(input("p1", "DashScope", true), &catalog).unwrap();
    let err =
        upsert_custom_provider_with_catalog(input("p2", "dashscope", true), &catalog).unwrap_err();
    assert!(err.to_string().contains("already exists"));
}

#[test]
fn rejects_builtin_name() {
    let _home = HomeGuard::new("builtin-name");
    let catalog = fixture_catalog();
    let err =
        upsert_custom_provider_with_catalog(input("mine", "Future", true), &catalog).unwrap_err();
    assert!(err.to_string().contains("built-in"));
}

#[test]
fn reserves_future_id() {
    let _home = HomeGuard::new("future-id");
    let catalog = fixture_catalog();
    let err =
        upsert_custom_provider_with_catalog(input("future", "Mine", true), &catalog).unwrap_err();
    assert!(err.to_string().contains("future") || err.to_string().contains("reserved"));
}

#[test]
fn list_filters_stray_future_entry() {
    let _home = HomeGuard::new("future-filter");
    // Simulate a hand-edited models.json that contains a `future` provider.
    let path = models_json_path().unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        r#"{"providers":{"future":{"name":"Bogus","baseUrl":"x"},"zai":{"name":"ZAI","baseUrl":"y"}}}"#,
    )
    .unwrap();
    let view = providers_view(&fixture_catalog());
    assert!(view.custom.iter().all(|p| p.id != "future"));
    assert!(view.custom.iter().any(|p| p.id == "zai"));
}

#[test]
fn list_includes_catalog_providers_after_future() {
    let _home = HomeGuard::new("catalog-list");
    let view = providers_view(&fixture_catalog());
    assert_eq!(view.builtin.first().map(|p| p.id.as_str()), Some("future"));
    let deepseek = view.builtin.iter().find(|p| p.id == "deepseek").unwrap();
    assert_eq!(deepseek.name, "DeepSeek");
    assert_eq!(deepseek.model_count, 3);
}

#[test]
fn future_provider_uses_cached_model_count() {
    let _home = HomeGuard::new("future-count");
    let path = future_models_cache_path().unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        r#"{"fetched_at":1,"models":[{"id":"m1"},{"id":"m2"}]}"#,
    )
    .unwrap();

    let view = providers_view(&fixture_catalog());
    assert_eq!(
        view.builtin
            .iter()
            .find(|provider| provider.id == "future")
            .map(|provider| provider.model_count),
        Some(2)
    );
}

#[test]
fn custom_provider_shadows_builtin_catalog_provider() {
    let _home = HomeGuard::new("catalog-shadow");
    let path = models_json_path().unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        r#"{"providers":{"deepseek":{"name":"My DeepSeek","api":"openai-completions","baseUrl":"https://proxy.example.com/v1","models":[]}}}"#,
    )
    .unwrap();
    let view = providers_view(&fixture_catalog());
    assert!(view.builtin.iter().all(|p| p.id != "deepseek"));
    assert_eq!(view.custom.len(), 1);
    assert_eq!(view.custom[0].id, "deepseek");
}

#[test]
fn update_builtin_provider_key_sets_and_clears_auth_entry() {
    let _home = HomeGuard::new("builtin-key");
    let catalog = fixture_catalog();
    let view = update_builtin_provider_key_with_catalog(
        UpdateBuiltinProviderKeyInput {
            id: "deepseek".to_string(),
            api_key: Some("sk-test".to_string()),
        },
        &catalog,
    )
    .unwrap();
    assert!(
        view.builtin
            .iter()
            .find(|provider| provider.id == "deepseek")
            .unwrap()
            .has_api_key
    );
    assert_eq!(
        crate::auth_store::read()
            .unwrap()
            .get("deepseek")
            .and_then(Value::as_object)
            .and_then(|entry| entry.get("key"))
            .and_then(Value::as_str),
        Some("sk-test")
    );

    let view = update_builtin_provider_key_with_catalog(
        UpdateBuiltinProviderKeyInput {
            id: "deepseek".to_string(),
            api_key: None,
        },
        &catalog,
    )
    .unwrap();
    assert!(
        !view
            .builtin
            .iter()
            .find(|provider| provider.id == "deepseek")
            .unwrap()
            .has_api_key
    );
    assert!(crate::auth_store::read()
        .unwrap()
        .get("deepseek")
        .and_then(Value::as_object)
        .and_then(|entry| entry.get("key"))
        .is_none());
}

#[test]
fn create_rejects_builtin_catalog_id_and_name() {
    let _home = HomeGuard::new("builtin-collision");
    let catalog = fixture_catalog();
    let id_err =
        upsert_custom_provider_with_catalog(input("deepseek", "DeepSeek Proxy", true), &catalog)
            .unwrap_err();
    assert!(id_err.to_string().contains("built-in"));

    let name_err =
        upsert_custom_provider_with_catalog(input("p1", "DeepSeek", true), &catalog).unwrap_err();
    assert!(name_err.to_string().contains("built-in"));
}

#[test]
fn id_is_lowercased() {
    let _home = HomeGuard::new("id-lower");
    let catalog = fixture_catalog();
    upsert_custom_provider_with_catalog(input("DashScope", "DashScope", true), &catalog).unwrap();
    let view = providers_view(&catalog);
    assert_eq!(view.custom.len(), 1);
    assert_eq!(view.custom[0].id, "dashscope");
}

#[test]
fn rejects_bad_id_charset_and_length() {
    let _home = HomeGuard::new("id-bad");
    let catalog = fixture_catalog();
    // Disallowed punctuation (dot/space).
    assert!(upsert_custom_provider_with_catalog(input("a.b", "A", true), &catalog).is_err());
    assert!(upsert_custom_provider_with_catalog(input("a b", "A", true), &catalog).is_err());
    // Too short.
    assert!(upsert_custom_provider_with_catalog(input("a", "A", true), &catalog).is_err());
}

#[test]
fn rejects_non_ascii_name() {
    let _home = HomeGuard::new("name-cjk");
    let catalog = fixture_catalog();
    assert!(upsert_custom_provider_with_catalog(input("p1", "中文", true), &catalog).is_err());
    assert!(upsert_custom_provider_with_catalog(input("p2", "ＦＵＬＬ", true), &catalog).is_err());
}

#[test]
fn rejects_bad_base_url_and_api() {
    let _home = HomeGuard::new("url-api");
    let catalog = fixture_catalog();
    let mut bad_url = input("p1", "P1", true);
    bad_url.base_url = "ftp://example.com".to_string();
    assert!(upsert_custom_provider_with_catalog(bad_url, &catalog).is_err());

    let mut bad_api = input("p2", "P2", true);
    bad_api.api = "made-up".to_string();
    assert!(upsert_custom_provider_with_catalog(bad_api, &catalog).is_err());
}

#[test]
fn validates_models() {
    let _home = HomeGuard::new("models");
    let catalog = fixture_catalog();
    // Valid composite model id with `/` and `.`.
    let mut ok = input("p1", "P1", true);
    ok.models = vec![custom_model("anthropic/claude-3.5-sonnet", "", false)];
    assert!(upsert_custom_provider_with_catalog(ok, &catalog).is_ok());

    // Whitespace in model id is rejected.
    let mut bad = input("p2", "P2", true);
    bad.models = vec![custom_model("bad id", "", false)];
    assert!(upsert_custom_provider_with_catalog(bad, &catalog).is_err());

    // Duplicate model ids are rejected.
    let mut dup = input("p3", "P3", true);
    dup.models = vec![
        custom_model("m", "", false),
        custom_model("m", "", false),
    ];
    assert!(upsert_custom_provider_with_catalog(dup, &catalog).is_err());
}

#[test]
fn empty_provider_and_model_names_fall_back_to_their_ids() {
    let _home = HomeGuard::new("name-fallbacks");
    let catalog = fixture_catalog();
    let mut input = input("provider-id", "", true);
    input.models = vec![custom_model("model-id", "", false)];

    let view = upsert_custom_provider_with_catalog(input, &catalog).unwrap();

    let provider = view
        .custom
        .iter()
        .find(|provider| provider.id == "provider-id")
        .unwrap();
    assert_eq!(provider.name, "provider-id");
    assert_eq!(provider.models[0].name, "model-id");
}

#[test]
fn model_modalities_round_trip() {
    let _home = HomeGuard::new("modalities");
    let catalog = fixture_catalog();
    let mut in_ = input("p1", "P1", true);
    in_.models = vec![
        custom_model("text-only", "", false),
        custom_model("vision", "", true),
    ];
    upsert_custom_provider_with_catalog(in_, &catalog).unwrap();

    // Persisted as a `modalities` array the agent reads.
    let doc = config_io::read_json_lenient(&models_json_path().unwrap());
    let models = doc["providers"]["p1"]["models"].as_array().unwrap();
    let vision = models.iter().find(|m| m["id"] == "vision").unwrap();
    assert_eq!(vision["modalities"], json!(["text", "image"]));
    assert_eq!(vision["contextWindow"], json!(128000));
    assert_eq!(vision["maxTokens"], json!(16384));
    let text_only = models.iter().find(|m| m["id"] == "text-only").unwrap();
    assert_eq!(text_only["modalities"], json!(["text"]));

    // And surfaces back through the view as supports_images.
    let view = providers_view(&catalog);
    let provider = view.custom.iter().find(|p| p.id == "p1").unwrap();
    assert!(
        provider
            .models
            .iter()
            .find(|m| m.id == "vision")
            .unwrap()
            .supports_images
    );
    assert!(
        !provider
            .models
            .iter()
            .find(|m| m.id == "text-only")
            .unwrap()
            .supports_images
    );
}

#[test]
fn catalog_base_url_placeholder_drives_requires_base_url() {
    let _home = HomeGuard::new("catalog-base-urls");
    let view = providers_view(&fixture_catalog());
    // The placeholder provider needs a user-supplied Base URL...
    let azure = view
        .builtin
        .iter()
        .find(|p| p.id == "azure-openai-responses")
        .expect("azure present in catalog");
    assert!(azure.requires_base_url);
    // ...while a regular catalog provider with a real URL does not.
    let deepseek = view
        .builtin
        .iter()
        .find(|p| p.id == "deepseek")
        .expect("deepseek present in catalog");
    assert!(!deepseek.requires_base_url);
    assert!(!deepseek.base_url.is_empty());
}

#[test]
fn set_builtin_base_url_override_keeps_provider_builtin() {
    let _home = HomeGuard::new("override");
    let catalog = fixture_catalog();
    let view = set_builtin_provider_base_url_with_catalog(
        SetBuiltinProviderBaseUrlInput {
            id: "deepseek".to_string(),
            base_url: "https://custom-deepseek.example.com/v1".to_string(),
        },
        &catalog,
    )
    .unwrap();

    // Still built-in (not moved to custom), with the override applied.
    assert!(view.custom.iter().all(|p| p.id != "deepseek"));
    let deepseek = view.builtin.iter().find(|p| p.id == "deepseek").unwrap();
    assert_eq!(deepseek.base_url, "https://custom-deepseek.example.com/v1");
    assert_eq!(deepseek.model_count, 3);

    // Persisted as a plain baseUrl override the agent reads.
    let doc = config_io::read_json_lenient(&models_json_path().unwrap());
    assert_eq!(
        doc["providers"]["deepseek"]["baseUrl"],
        json!("https://custom-deepseek.example.com/v1")
    );

    // Clearing removes the override entirely.
    set_builtin_provider_base_url_with_catalog(
        SetBuiltinProviderBaseUrlInput {
            id: "deepseek".to_string(),
            base_url: String::new(),
        },
        &catalog,
    )
    .unwrap();
    let doc = config_io::read_json_lenient(&models_json_path().unwrap());
    assert!(doc["providers"].get("deepseek").is_none());
}

#[test]
fn set_builtin_base_url_rejects_placeholder_and_bad_url() {
    let _home = HomeGuard::new("reject-bad-url");
    let catalog = fixture_catalog();
    let placeholder = set_builtin_provider_base_url_with_catalog(
        SetBuiltinProviderBaseUrlInput {
            id: "deepseek".to_string(),
            base_url: "https://YOUR_RESOURCE.deepseek.example.com/v1".to_string(),
        },
        &catalog,
    );
    assert!(placeholder.is_err());

    let bad = set_builtin_provider_base_url_with_catalog(
        SetBuiltinProviderBaseUrlInput {
            id: "deepseek".to_string(),
            base_url: "ftp://example.com".to_string(),
        },
        &catalog,
    );
    assert!(bad.is_err());
}

// ── validate.rs field-rule coverage ─────────────────────────────────────────

use super::validate::{model_json_values, validate_custom_provider};
use crate::remote::test_support::{ensure_mock_agent, mock_agent_lock};

fn valid_input() -> UpsertCustomProviderInput {
    input("prov", "Prov", true)
}

#[test]
fn validate_rejects_missing_and_reserved_ids() {
    let _home = HomeGuard::new("val-ids");
    let mut empty = valid_input();
    empty.id = "  ".to_string();
    let error = validate_custom_provider(empty).unwrap_err();
    assert!(error.to_string().contains("required"));

    let mut reserved = valid_input();
    reserved.id = "Future".to_string();
    let error = validate_custom_provider(reserved).unwrap_err();
    assert!(error.to_string().contains("reserved"));

    let mut long = valid_input();
    long.id = "a".repeat(41);
    assert!(validate_custom_provider(long).is_err());
}

#[test]
fn validate_base_url_rules() {
    let _home = HomeGuard::new("val-url");
    let mut empty = valid_input();
    empty.base_url = " ".to_string();
    assert!(validate_custom_provider(empty)
        .unwrap_err()
        .to_string()
        .contains("required"));

    let mut long = valid_input();
    long.base_url = format!("https://{}.com", "a".repeat(2048));
    assert!(validate_custom_provider(long)
        .unwrap_err()
        .to_string()
        .contains("too long"));

    let mut scheme = valid_input();
    scheme.base_url = "ftp://example.com".to_string();
    assert!(validate_custom_provider(scheme).is_err());

    let mut garbage = valid_input();
    garbage.base_url = "not a url".to_string();
    assert!(validate_custom_provider(garbage).is_err());
}

#[test]
fn validate_api_and_name_rules() {
    let _home = HomeGuard::new("val-api-name");
    let mut bad_api = valid_input();
    bad_api.api = "grpc".to_string();
    assert!(validate_custom_provider(bad_api)
        .unwrap_err()
        .to_string()
        .contains("Unsupported API type"));

    // An empty api defaults to openai-completions.
    let mut default_api = valid_input();
    default_api.api = String::new();
    let validated = validate_custom_provider(default_api).unwrap();
    assert_eq!(validated.api, "openai-completions");

    let mut long_name = valid_input();
    long_name.name = "n".repeat(41);
    assert!(validate_custom_provider(long_name)
        .unwrap_err()
        .to_string()
        .contains("cannot exceed"));

    let mut control_name = valid_input();
    control_name.name = "bad\tname".to_string();
    assert!(validate_custom_provider(control_name).is_err());
}

#[test]
fn validate_api_key_rules() {
    let _home = HomeGuard::new("val-key");
    let mut long_key = valid_input();
    long_key.api_key = Some("k".repeat(513));
    assert!(validate_custom_provider(long_key)
        .unwrap_err()
        .to_string()
        .contains("maximum length"));

    let mut control_key = valid_input();
    control_key.api_key = Some("key\nwith\nnewlines".to_string());
    assert!(validate_custom_provider(control_key)
        .unwrap_err()
        .to_string()
        .contains("illegal characters"));

    // A whitespace-only key is treated as absent (existing key untouched).
    let mut blank_key = valid_input();
    blank_key.api_key = Some("   ".to_string());
    assert!(validate_custom_provider(blank_key)
        .unwrap()
        .api_key
        .is_none());
}

#[test]
fn validate_model_rules() {
    let _home = HomeGuard::new("val-models");
    let model = |id: &str, name: &str| custom_model(id, name, false);

    // Empty model ids are skipped, not rejected.
    let mut skipped = valid_input();
    skipped.models = vec![model("  ", "")];
    assert!(validate_custom_provider(skipped).unwrap().models.is_empty());

    let mut long_id = valid_input();
    long_id.models = vec![model(&"m".repeat(101), "")];
    assert!(validate_custom_provider(long_id)
        .unwrap_err()
        .to_string()
        .contains("too long"));

    let mut bad_id = valid_input();
    bad_id.models = vec![model("bad id", "")];
    assert!(validate_custom_provider(bad_id)
        .unwrap_err()
        .to_string()
        .contains("illegal characters"));

    let mut long_name = valid_input();
    long_name.models = vec![model("m1", &"n".repeat(61))];
    assert!(validate_custom_provider(long_name)
        .unwrap_err()
        .to_string()
        .contains("cannot exceed"));

    let mut control_name = valid_input();
    control_name.models = vec![model("m1", "bad\nname")];
    assert!(validate_custom_provider(control_name).is_err());

    let mut too_many = valid_input();
    too_many.models = (0..101)
        .map(|index| model(&format!("m{index}"), ""))
        .collect();
    assert!(validate_custom_provider(too_many)
        .unwrap_err()
        .to_string()
        .contains("cannot exceed"));

    // Image support maps to the modalities pair.
    let mut vision = valid_input();
    vision.models = vec![custom_model("v1", "Vision", true)];
    let validated = validate_custom_provider(vision).unwrap();
    assert_eq!(validated.models[0].modalities, ["text", "image"]);
    let values = model_json_values(&validated.models);
    assert_eq!(values[0]["modalities"], json!(["text", "image"]));
    assert_eq!(values[0]["name"], json!("Vision"));
    assert_eq!(values[0]["contextWindow"], json!(128000));
    assert_eq!(values[0]["maxTokens"], json!(16384));

    let mut invalid_limits = valid_input();
    invalid_limits.models = vec![CustomProviderModel {
        context_window: 4096,
        max_tokens: 8192,
        ..custom_model("m1", "", false)
    }];
    assert!(validate_custom_provider(invalid_limits)
        .unwrap_err()
        .to_string()
        .contains("cannot exceed"));
}

// ── catalog.rs ──────────────────────────────────────────────────────────────

#[test]
fn catalog_unavailable_logs_and_returns_empty() {
    let _home = HomeGuard::new("cat-down");
    let map = super::catalog::catalog_unavailable(crate::AppError::Message("down".to_string()));
    assert!(map.is_empty());
}

#[tokio::test]
async fn builtin_catalog_providers_fetches_from_the_agent() {
    let _lock = mock_agent_lock();
    let _home = HomeGuard::new("cat-fetch");
    ensure_mock_agent();
    let catalog = super::catalog::builtin_catalog_providers().await;
    assert!(catalog.contains_key("deepseek"));
    assert!(catalog.contains_key("azure-openai-responses"));
    // FutureGene and the empty id are filtered out of the GUI catalog.
    assert!(!catalog.contains_key("future"));
    assert!(!catalog.contains_key(""));
    // Second call hits the process cache.
    let again = super::catalog::builtin_catalog_providers().await;
    assert_eq!(again.len(), catalog.len());
}

#[tokio::test]
async fn list_agent_providers_builds_the_view() {
    let _lock = mock_agent_lock();
    let _home = HomeGuard::new("cat-list");
    ensure_mock_agent();
    let view = list_agent_providers().await.unwrap();
    assert_eq!(view.builtin.first().map(|p| p.id.as_str()), Some("future"));
    assert!(view.builtin.iter().any(|p| p.id == "deepseek"));
}

// ── write.rs async command paths (mock agent) ───────────────────────────────

fn key_input(id: &str, api_key: Option<&str>) -> UpdateBuiltinProviderKeyInput {
    UpdateBuiltinProviderKeyInput {
        id: id.to_string(),
        api_key: api_key.map(str::to_string),
    }
}

#[tokio::test]
async fn builtin_key_update_applied_by_agent_and_validated() {
    let _lock = mock_agent_lock();
    let _home = HomeGuard::new("wr-key-rpc");
    let agent = ensure_mock_agent();

    // Agent applies the change: the view comes straight back, no local write.
    let view = update_builtin_provider_key(key_input("deepseek", Some("sk-live")))
        .await
        .unwrap();
    assert!(view.builtin.iter().any(|p| p.id == "deepseek"));
    assert!(agent.served("set_auth", ""));
    assert!(crate::auth_store::read().unwrap().get("deepseek").is_none());

    // Clearing a key takes the clear path.
    update_builtin_provider_key(key_input("deepseek", None))
        .await
        .unwrap();

    // Request-local validation runs before the RPC.
    assert!(update_builtin_provider_key(key_input("", Some("k")))
        .await
        .is_err());
    assert!(update_builtin_provider_key(key_input("future", Some("k")))
        .await
        .is_err());
    assert!(update_builtin_provider_key(key_input("unknown", Some("k")))
        .await
        .is_err());
    assert!(
        update_builtin_provider_key(key_input("deepseek", Some(&"k".repeat(513))))
            .await
            .is_err()
    );
    assert!(
        update_builtin_provider_key(key_input("deepseek", Some("bad\nkey")))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn builtin_key_update_never_falls_back_to_local_writes() {
    let _lock = mock_agent_lock();
    let _home = HomeGuard::new("wr-key-local");
    let agent = ensure_mock_agent();
    // A legacy Agent cannot turn Desktop into a second writer.
    agent.script("set_auth", false, json!(null), "unknown command: set_auth");
    let error = update_builtin_provider_key(key_input("deepseek", Some("sk-local")))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("unknown command"));
    assert!(crate::auth_store::read().unwrap().get("deepseek").is_none());
    assert!(!agent.served("reload_auth", ""));

    // An explicit rejection (NOT "unknown command") surfaces as an error.
    agent.script("set_auth", false, json!(null), "key rejected by policy");
    let error = update_builtin_provider_key(key_input("deepseek", Some("sk-no")))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("key rejected"));
}

#[tokio::test]
async fn builtin_base_url_update_paths() {
    let _lock = mock_agent_lock();
    let _home = HomeGuard::new("wr-url-rpc");
    let agent = ensure_mock_agent();
    let base_url_input = |id: &str, base_url: &str| SetBuiltinProviderBaseUrlInput {
        id: id.to_string(),
        base_url: base_url.to_string(),
    };

    // Agent-applied.
    let view = set_builtin_provider_base_url(base_url_input(
        "azure-openai-responses",
        "https://my.openai.azure.com/openai",
    ))
    .await
    .unwrap();
    assert!(view
        .builtin
        .iter()
        .any(|p| p.id == "azure-openai-responses"));
    assert!(agent.served("upsert_provider", ""));

    // Validation before the RPC.
    assert!(
        set_builtin_provider_base_url(base_url_input("", "https://x.com"))
            .await
            .is_err()
    );
    assert!(
        set_builtin_provider_base_url(base_url_input("future", "https://x.com"))
            .await
            .is_err()
    );
    assert!(
        set_builtin_provider_base_url(base_url_input("unknown", "https://x.com"))
            .await
            .is_err()
    );
    let long = format!("https://{}.com", "a".repeat(2048));
    assert!(
        set_builtin_provider_base_url(base_url_input("deepseek", &long))
            .await
            .is_err()
    );
    assert!(
        set_builtin_provider_base_url(base_url_input("deepseek", "ftp://x.com"))
            .await
            .is_err()
    );
    assert!(set_builtin_provider_base_url(base_url_input(
        "deepseek",
        "https://YOUR_RESOURCE.x.com"
    ))
    .await
    .is_err());

    // A legacy Agent error is surfaced and the local catalog remains unchanged.
    agent.script(
        "upsert_provider",
        false,
        json!(null),
        "unknown command: upsert_provider",
    );
    let error =
        set_builtin_provider_base_url(base_url_input("deepseek", "https://local.example.com/v1"))
            .await
            .unwrap_err();
    assert!(error.to_string().contains("unknown command"));
    assert!(
        config_io::read_json_lenient(&models_json_path().unwrap())["providers"]["deepseek"]
            .is_null()
    );

    // Explicit rejection surfaces.
    agent.script("upsert_provider", false, json!(null), "bad base url");
    assert!(
        set_builtin_provider_base_url(base_url_input("deepseek", "https://y.com"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn custom_provider_upsert_paths() {
    let _lock = mock_agent_lock();
    let _home = HomeGuard::new("wr-upsert-rpc");
    let agent = ensure_mock_agent();

    // Agent-applied create: the RPC carries the validated provider; the mock
    // does not persist files, so the view simply re-reads the (empty) locals.
    let mut create = valid_input();
    create.api_key = Some("sk-new".to_string());
    create.models = vec![custom_model("m1", "", true)];
    upsert_custom_provider(create).await.unwrap();
    assert!(agent.served("upsert_provider", ""));

    // A legacy Agent error must not write models.json or auth.json locally.
    agent.script(
        "upsert_provider",
        false,
        json!(null),
        "unknown command: upsert_provider",
    );
    let mut local = input("localp", "LocalP", true);
    local.api_key = Some("sk-local".to_string());
    let error = upsert_custom_provider(local).await.unwrap_err();
    assert!(error.to_string().contains("unknown command"));
    let doc = config_io::read_json_lenient(&models_json_path().unwrap());
    assert!(doc["providers"]["localp"].is_null());
    assert!(crate::auth_store::read().unwrap().get("localp").is_none());

    // Explicit rejection surfaces.
    agent.script(
        "upsert_provider",
        false,
        json!(null),
        "duplicate provider id",
    );
    assert!(upsert_custom_provider(input("another", "Another", true))
        .await
        .is_err());
}

#[test]
fn upsert_refuses_to_create_against_an_unavailable_catalog() {
    let _home = HomeGuard::new("wr-upsert-nocat");
    let error = upsert_custom_provider_with_catalog(valid_input(), &BTreeMap::new()).unwrap_err();
    assert!(error.to_string().contains("catalog is unavailable"));
}

#[test]
fn upsert_local_rejects_a_non_object_providers_root() {
    let _home = HomeGuard::new("wr-upsert-badroot");
    let path = models_json_path().unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, json!({ "providers": [1, 2, 3] }).to_string()).unwrap();
    let error = upsert_custom_provider_with_catalog(valid_input(), &fixture_catalog()).unwrap_err();
    assert!(error.to_string().contains("not an object"));
}

#[cfg(unix)]
#[test]
fn upsert_local_rolls_back_when_the_key_write_fails() {
    let _home = HomeGuard::new("wr-upsert-rollback");
    // models.json write succeeds but the key write is injected-failed → the
    // models file is restored to its exact pre-call bytes.
    let path = models_json_path().unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{}\n").unwrap();
    super::write::INJECT_AUTH_WRITE_FAILURE.store(true, std::sync::atomic::Ordering::Relaxed);
    let mut create = valid_input();
    create.api_key = Some("sk-x".to_string());
    assert!(upsert_custom_provider_with_catalog(create, &fixture_catalog()).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}\n");
}

#[cfg(unix)]
#[test]
fn upsert_local_rolls_back_when_models_write_fails() {
    use std::os::unix::fs::PermissionsExt;
    let _home = HomeGuard::new("wr-upsert-readonly");
    let path = models_json_path().unwrap();
    let dir = path.parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&path, "{}\n").unwrap();
    let permissions = std::fs::metadata(&dir).unwrap().permissions();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = upsert_custom_provider_with_catalog(valid_input(), &fixture_catalog());
    std::fs::set_permissions(&dir, permissions).unwrap();
    assert!(result.is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}\n");
}

#[tokio::test]
async fn custom_provider_delete_paths() {
    let _lock = mock_agent_lock();
    let _home = HomeGuard::new("wr-delete-rpc");
    let agent = ensure_mock_agent();

    // Seed a provider locally (models.json + auth.json).
    upsert_custom_provider_with_catalog(input("gone", "Gone", true), &fixture_catalog()).unwrap();

    // Agent-applied delete (the mock does not persist; the RPC is the proof).
    delete_custom_provider("gone".to_string()).await.unwrap();
    assert!(agent.served("delete_provider", ""));

    // Validation guards.
    assert!(delete_custom_provider("  ".to_string()).await.is_err());
    assert!(delete_custom_provider("future".to_string()).await.is_err());
    assert!(delete_custom_provider("deepseek".to_string())
        .await
        .is_err());

    // A legacy Agent error leaves both local entries untouched.
    upsert_custom_provider_with_catalog(input("gone2", "Gone2", true), &fixture_catalog()).unwrap();
    crate::auth_store::set_provider_key("gone2", "sk-g").unwrap();
    agent.script(
        "delete_provider",
        false,
        json!(null),
        "unknown command: delete_provider",
    );
    let error = delete_custom_provider("gone2".to_string())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("unknown command"));
    assert!(crate::auth_store::read().unwrap().get("gone2").is_some());
    assert!(
        config_io::read_json_lenient(&models_json_path().unwrap())["providers"]["gone2"]
            .is_object()
    );

    // Explicit rejection surfaces.
    agent.script(
        "delete_provider",
        false,
        json!(null),
        "cannot delete in use",
    );
    assert!(delete_custom_provider("whatever".to_string())
        .await
        .is_err());
}

#[test]
fn delete_local_is_a_noop_for_unknown_ids() {
    let _home = HomeGuard::new("wr-delete-noop");
    let catalog = fixture_catalog();
    // Nothing in either file → Ok, no files created.
    delete_custom_provider_with_catalog("ghost".to_string(), &catalog).unwrap();
    assert!(!models_json_path().unwrap().exists());
}

#[cfg(unix)]
#[test]
fn delete_local_rolls_back_when_the_auth_write_fails() {
    let _home = HomeGuard::new("wr-delete-rollback");
    upsert_custom_provider_with_catalog(input("rb", "RB", true), &fixture_catalog()).unwrap();
    crate::auth_store::set_provider_key("rb", "sk-rb").unwrap();
    let path = models_json_path().unwrap();
    let before = std::fs::read_to_string(&path).unwrap();
    super::write::INJECT_AUTH_WRITE_FAILURE.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(delete_custom_provider_with_catalog("rb".to_string(), &fixture_catalog()).is_err());
    // models.json is restored to its exact pre-call bytes.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn delete_local_rolls_back_when_models_write_fails() {
    use std::os::unix::fs::PermissionsExt;
    let _home = HomeGuard::new("wr-delete-readonly");
    upsert_custom_provider_with_catalog(input("ro", "RO", true), &fixture_catalog()).unwrap();
    let path = models_json_path().unwrap();
    let before = std::fs::read_to_string(&path).unwrap();
    let dir = path.parent().unwrap().to_path_buf();
    let permissions = std::fs::metadata(&dir).unwrap().permissions();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = delete_custom_provider_with_catalog("ro".to_string(), &fixture_catalog());
    std::fs::set_permissions(&dir, permissions).unwrap();
    assert!(result.is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
}

#[test]
fn override_only_detection_and_base_url_override_lookup() {
    let _home = HomeGuard::new("wr-override-only");
    use super::write::{is_override_only, provider_base_url_override};
    assert!(is_override_only(&json!({})));
    assert!(is_override_only(&json!({ "baseUrl": "https://x.com" })));
    assert!(is_override_only(&json!({ "name": "  " })));
    assert!(!is_override_only(&json!({ "name": "N" })));
    assert!(!is_override_only(&json!({ "api": "anthropic" })));
    assert!(!is_override_only(&json!({ "models": [{ "id": "m" }] })));

    let models = json!({
        "providers": {
            "with": { "baseUrl": "https://override.example.com" },
            "blank": { "baseUrl": "  " },
            "none": { "name": "N" },
        }
    });
    assert_eq!(
        provider_base_url_override(&models, "with").as_deref(),
        Some("https://override.example.com")
    );
    assert_eq!(provider_base_url_override(&models, "blank"), None);
    assert_eq!(provider_base_url_override(&models, "none"), None);
    assert_eq!(provider_base_url_override(&models, "missing"), None);
    assert_eq!(provider_base_url_override(&json!({}), "with"), None);
}

#[test]
fn clear_base_url_keeps_entries_with_other_fields() {
    let _home = HomeGuard::new("wr-clear-keep");
    let catalog = fixture_catalog();
    let path = models_json_path().unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        json!({ "providers": { "deepseek": { "baseUrl": "https://x.com", "compat": { "a": 1 } } } })
            .to_string(),
    )
    .unwrap();
    set_builtin_provider_base_url_with_catalog(
        SetBuiltinProviderBaseUrlInput {
            id: "deepseek".to_string(),
            base_url: String::new(),
        },
        &catalog,
    )
    .unwrap();
    // The entry survives (it still carries `compat`), minus the override.
    let doc = config_io::read_json_lenient(&path);
    assert!(doc["providers"]["deepseek"].get("baseUrl").is_none());
    assert_eq!(doc["providers"]["deepseek"]["compat"]["a"], json!(1));
}

#[test]
fn clear_base_url_is_a_noop_without_a_providers_object() {
    let _home = HomeGuard::new("wr-clear-no-providers");
    let catalog = fixture_catalog();
    let path = models_json_path().unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, json!({}).to_string()).unwrap();
    set_builtin_provider_base_url_with_catalog(
        SetBuiltinProviderBaseUrlInput {
            id: "deepseek".to_string(),
            base_url: String::new(),
        },
        &catalog,
    )
    .unwrap();
}

#[test]
fn clear_base_url_is_a_noop_when_the_entry_is_not_an_object() {
    let _home = HomeGuard::new("wr-clear-no-entry");
    let catalog = fixture_catalog();
    let path = models_json_path().unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        json!({ "providers": { "deepseek": "not-an-object" } }).to_string(),
    )
    .unwrap();
    set_builtin_provider_base_url_with_catalog(
        SetBuiltinProviderBaseUrlInput {
            id: "deepseek".to_string(),
            base_url: String::new(),
        },
        &catalog,
    )
    .unwrap();
}
