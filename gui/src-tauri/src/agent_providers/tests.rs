//! Tests for the provider view and write paths. The built-in catalog normally
//! arrives from the agent over the `list_models` RPC; tests inject a fixture
//! catalog into the synchronous `_with_catalog` cores instead, so nothing here
//! needs a running agent.

use super::catalog::{future_models_cache_path, models_json_path, CatalogProviderSummary};
use super::write::{
    set_builtin_provider_base_url_with_catalog, update_builtin_provider_key_with_catalog,
    upsert_custom_provider_with_catalog,
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
    ok.models = vec![CustomProviderModel {
        id: "anthropic/claude-3.5-sonnet".to_string(),
        name: String::new(),
        supports_images: false,
    }];
    assert!(upsert_custom_provider_with_catalog(ok, &catalog).is_ok());

    // Whitespace in model id is rejected.
    let mut bad = input("p2", "P2", true);
    bad.models = vec![CustomProviderModel {
        id: "bad id".to_string(),
        name: String::new(),
        supports_images: false,
    }];
    assert!(upsert_custom_provider_with_catalog(bad, &catalog).is_err());

    // Duplicate model ids are rejected.
    let mut dup = input("p3", "P3", true);
    dup.models = vec![
        CustomProviderModel {
            id: "m".to_string(),
            name: String::new(),
            supports_images: false,
        },
        CustomProviderModel {
            id: "m".to_string(),
            name: String::new(),
            supports_images: false,
        },
    ];
    assert!(upsert_custom_provider_with_catalog(dup, &catalog).is_err());
}

#[test]
fn empty_provider_and_model_names_fall_back_to_their_ids() {
    let _home = HomeGuard::new("name-fallbacks");
    let catalog = fixture_catalog();
    let mut input = input("provider-id", "", true);
    input.models = vec![CustomProviderModel {
        id: "model-id".to_string(),
        name: String::new(),
        supports_images: false,
    }];

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
        CustomProviderModel {
            id: "text-only".to_string(),
            name: String::new(),
            supports_images: false,
        },
        CustomProviderModel {
            id: "vision".to_string(),
            name: String::new(),
            supports_images: true,
        },
    ];
    upsert_custom_provider_with_catalog(in_, &catalog).unwrap();

    // Persisted as a `modalities` array the agent reads.
    let doc = config_io::read_json_lenient(&models_json_path().unwrap());
    let models = doc["providers"]["p1"]["models"].as_array().unwrap();
    let vision = models.iter().find(|m| m["id"] == "vision").unwrap();
    assert_eq!(vision["modalities"], json!(["text", "image"]));
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
