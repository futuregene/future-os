use super::catalog::{future_models_cache_path, models_json_path};
use super::*;
use crate::auth_store::test_support::HomeGuard;
use serde_json::json;

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
    upsert_custom_provider(input("dashscope", "DashScope", true)).unwrap();
    // Re-creating the same id must fail rather than silently overwrite.
    let err = upsert_custom_provider(input("dashscope", "Other", true)).unwrap_err();
    assert!(err.to_string().contains("already exists"));
}

#[test]
fn edit_allows_same_id() {
    let _home = HomeGuard::new("edit-id");
    upsert_custom_provider(input("dashscope", "DashScope", true)).unwrap();
    // Editing (create = false) the same id is fine.
    upsert_custom_provider(input("dashscope", "DashScope 2", false)).unwrap();
    let view = list_agent_providers().unwrap();
    assert_eq!(view.custom.len(), 1);
    assert_eq!(view.custom[0].name, "DashScope 2");
}

#[test]
fn rejects_duplicate_name_case_insensitive() {
    let _home = HomeGuard::new("dup-name");
    upsert_custom_provider(input("p1", "DashScope", true)).unwrap();
    let err = upsert_custom_provider(input("p2", "dashscope", true)).unwrap_err();
    assert!(err.to_string().contains("already exists"));
}

#[test]
fn rejects_builtin_name() {
    let _home = HomeGuard::new("builtin-name");
    let err = upsert_custom_provider(input("mine", "futuregene", true)).unwrap_err();
    assert!(err.to_string().contains("built-in"));
}

#[test]
fn reserves_future_id() {
    let _home = HomeGuard::new("future-id");
    let err = upsert_custom_provider(input("future", "Mine", true)).unwrap_err();
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
    let view = list_agent_providers().unwrap();
    assert!(view.custom.iter().all(|p| p.id != "future"));
    assert!(view.custom.iter().any(|p| p.id == "zai"));
}

#[test]
fn list_includes_catalog_providers_after_future() {
    let _home = HomeGuard::new("catalog-list");
    let view = list_agent_providers().unwrap();
    assert_eq!(view.builtin.first().map(|p| p.id.as_str()), Some("future"));
    let openai = view.builtin.iter().find(|p| p.id == "openai").unwrap();
    assert_eq!(openai.name, "OpenAI");
    assert!(openai.model_count > 0);
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

    let view = list_agent_providers().unwrap();
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
        r#"{"providers":{"openai":{"name":"My OpenAI","api":"openai-completions","baseUrl":"https://proxy.example.com/v1","models":[]}}}"#,
    )
    .unwrap();
    let view = list_agent_providers().unwrap();
    assert!(view.builtin.iter().all(|p| p.id != "openai"));
    assert_eq!(view.custom.len(), 1);
    assert_eq!(view.custom[0].id, "openai");
}

#[test]
fn update_builtin_provider_key_sets_and_clears_auth_entry() {
    let _home = HomeGuard::new("builtin-key");
    let view = update_builtin_provider_key(UpdateBuiltinProviderKeyInput {
        id: "openai".to_string(),
        api_key: Some("sk-test".to_string()),
    })
    .unwrap();
    assert!(
        view.builtin
            .iter()
            .find(|provider| provider.id == "openai")
            .unwrap()
            .has_api_key
    );
    assert_eq!(
        crate::auth_store::read()
            .unwrap()
            .get("openai")
            .and_then(Value::as_object)
            .and_then(|entry| entry.get("key"))
            .and_then(Value::as_str),
        Some("sk-test")
    );

    let view = update_builtin_provider_key(UpdateBuiltinProviderKeyInput {
        id: "openai".to_string(),
        api_key: None,
    })
    .unwrap();
    assert!(
        !view
            .builtin
            .iter()
            .find(|provider| provider.id == "openai")
            .unwrap()
            .has_api_key
    );
    assert!(crate::auth_store::read()
        .unwrap()
        .get("openai")
        .and_then(Value::as_object)
        .and_then(|entry| entry.get("key"))
        .is_none());
}

#[test]
fn create_rejects_builtin_catalog_id_and_name() {
    let _home = HomeGuard::new("builtin-collision");
    let id_err = upsert_custom_provider(input("openai", "OpenAI Proxy", true)).unwrap_err();
    assert!(id_err.to_string().contains("built-in"));

    let name_err = upsert_custom_provider(input("p1", "OpenAI", true)).unwrap_err();
    assert!(name_err.to_string().contains("built-in"));
}

#[test]
fn id_is_lowercased() {
    let _home = HomeGuard::new("id-lower");
    upsert_custom_provider(input("DashScope", "DashScope", true)).unwrap();
    let view = list_agent_providers().unwrap();
    assert_eq!(view.custom.len(), 1);
    assert_eq!(view.custom[0].id, "dashscope");
}

#[test]
fn rejects_bad_id_charset_and_length() {
    let _home = HomeGuard::new("id-bad");
    // Disallowed punctuation (dot/space).
    assert!(upsert_custom_provider(input("a.b", "A", true)).is_err());
    assert!(upsert_custom_provider(input("a b", "A", true)).is_err());
    // Too short.
    assert!(upsert_custom_provider(input("a", "A", true)).is_err());
}

#[test]
fn rejects_non_ascii_name() {
    let _home = HomeGuard::new("name-cjk");
    assert!(upsert_custom_provider(input("p1", "中文", true)).is_err());
    assert!(upsert_custom_provider(input("p2", "ＦＵＬＬ", true)).is_err());
    assert!(upsert_custom_provider(input("p3", "emoji 🚀", true)).is_err());
}

#[test]
fn rejects_bad_base_url_and_api() {
    let _home = HomeGuard::new("url-api");
    let mut bad_url = input("p1", "P1", true);
    bad_url.base_url = "ftp://example.com".to_string();
    assert!(upsert_custom_provider(bad_url).is_err());

    let mut bad_api = input("p2", "P2", true);
    bad_api.api = "made-up".to_string();
    assert!(upsert_custom_provider(bad_api).is_err());
}

#[test]
fn validates_models() {
    let _home = HomeGuard::new("models");
    // Valid composite model id with `/` and `.`.
    let mut ok = input("p1", "P1", true);
    ok.models = vec![CustomProviderModel {
        id: "anthropic/claude-3.5-sonnet".to_string(),
        name: String::new(),
        supports_images: false,
    }];
    assert!(upsert_custom_provider(ok).is_ok());

    // Whitespace in model id is rejected.
    let mut bad = input("p2", "P2", true);
    bad.models = vec![CustomProviderModel {
        id: "bad id".to_string(),
        name: String::new(),
        supports_images: false,
    }];
    assert!(upsert_custom_provider(bad).is_err());

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
    assert!(upsert_custom_provider(dup).is_err());
}

#[test]
fn model_modalities_round_trip() {
    let _home = HomeGuard::new("modalities");
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
    upsert_custom_provider(in_).unwrap();

    // Persisted as a `modalities` array the agent reads.
    let doc = config_io::read_json_lenient(&models_json_path().unwrap());
    let models = doc["providers"]["p1"]["models"].as_array().unwrap();
    let vision = models.iter().find(|m| m["id"] == "vision").unwrap();
    assert_eq!(vision["modalities"], json!(["text", "image"]));
    let text_only = models.iter().find(|m| m["id"] == "text-only").unwrap();
    assert_eq!(text_only["modalities"], json!(["text"]));

    // And surfaces back through the view as supports_images.
    let view = list_agent_providers().unwrap();
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
fn azure_provider_requires_base_url_flag() {
    let _home = HomeGuard::new("azure-requires");
    let view = list_agent_providers().unwrap();
    let azure = view
        .builtin
        .iter()
        .find(|p| p.id == "azure-openai-responses")
        .expect("azure provider present in catalog");
    assert!(azure.requires_base_url);
    assert!(azure.base_url.contains("YOUR_RESOURCE"));
}

#[test]
fn set_builtin_base_url_override_keeps_provider_builtin() {
    let _home = HomeGuard::new("azure-override");
    let view = set_builtin_provider_base_url(SetBuiltinProviderBaseUrlInput {
        id: "azure-openai-responses".to_string(),
        base_url: "https://my-res.openai.azure.com/openai/v1".to_string(),
    })
    .unwrap();

    // Still built-in (not moved to custom), with the override applied and the
    // requires-base-url flag intact so it stays editable.
    assert!(view.custom.iter().all(|p| p.id != "azure-openai-responses"));
    let azure = view
        .builtin
        .iter()
        .find(|p| p.id == "azure-openai-responses")
        .unwrap();
    assert_eq!(azure.base_url, "https://my-res.openai.azure.com/openai/v1");
    assert!(azure.requires_base_url);
    assert!(azure.model_count > 0);

    // Persisted as a plain baseUrl override the agent reads.
    let doc = config_io::read_json_lenient(&models_json_path().unwrap());
    assert_eq!(
        doc["providers"]["azure-openai-responses"]["baseUrl"],
        json!("https://my-res.openai.azure.com/openai/v1")
    );

    // Clearing removes the override entirely.
    set_builtin_provider_base_url(SetBuiltinProviderBaseUrlInput {
        id: "azure-openai-responses".to_string(),
        base_url: String::new(),
    })
    .unwrap();
    let doc = config_io::read_json_lenient(&models_json_path().unwrap());
    assert!(doc["providers"].get("azure-openai-responses").is_none());
}

#[test]
fn set_builtin_base_url_rejects_placeholder_and_bad_url() {
    let _home = HomeGuard::new("azure-reject");
    let placeholder = set_builtin_provider_base_url(SetBuiltinProviderBaseUrlInput {
        id: "azure-openai-responses".to_string(),
        base_url: "https://YOUR_RESOURCE.openai.azure.com/openai/v1".to_string(),
    });
    assert!(placeholder.is_err());

    let bad = set_builtin_provider_base_url(SetBuiltinProviderBaseUrlInput {
        id: "azure-openai-responses".to_string(),
        base_url: "ftp://example.com".to_string(),
    });
    assert!(bad.is_err());
}
