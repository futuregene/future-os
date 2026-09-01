//! Tests for the auth / model / provider command handlers.

use crate::rpc::RpcCommand;

use crate::rpc::commands::test_support::*;
use crate::rpc::handle_command_internal;
use crate::test_support::TestHome;

#[test]
fn set_auth_rejects_missing_payload_provider_and_noop() {
    let state = make_app_state();

    let resp = parse_response(&handle_command_internal(&state, make_cmd("set_auth")));
    assert_eq!(resp["success"], false);
    assert!(resp["error"].as_str().unwrap().contains("auth_update"));

    let mut cmd = make_cmd("set_auth");
    cmd.auth_update = Some(crate::config::providers::AuthMutation {
        provider: "  ".to_string(),
        key: Some("k".to_string()),
        ..Default::default()
    });
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(resp["error"].as_str().unwrap().contains("provider"));

    let mut cmd = make_cmd("set_auth");
    cmd.auth_update = Some(crate::config::providers::AuthMutation {
        provider: "future".to_string(),
        ..Default::default()
    });
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(resp["error"].as_str().unwrap().contains("no change"));
}

#[test]
fn set_auth_writes_auth_json_and_reports_success() {
    let home = TestHome::new();
    let state = make_app_state();

    let mut cmd = make_cmd("set_auth");
    cmd.auth_update = Some(crate::config::providers::AuthMutation {
        provider: "future".to_string(),
        key: Some("k1".to_string()),
        base_url: Some("https://future-os.cn/api".to_string()),
        ..Default::default()
    });
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["provider"], "future");

    let stored = read_json(&home.auth_path());
    assert_eq!(stored["future"]["key"], "k1");
    assert_eq!(stored["future"]["base_url"], "https://future-os.cn/api");
    assert_eq!(stored["future"]["type"], "api_key");
}

#[test]
fn upsert_provider_writes_both_files_and_delete_removes_them() {
    let home = TestHome::new();
    let state = make_app_state();

    let mut cmd = make_cmd("upsert_provider");
    cmd.provider_config = Some(crate::config::providers::ProviderUpsertSpec {
        id: "myprov".to_string(),
        name: Some("My Provider".to_string()),
        api: Some("anthropic".to_string()),
        base_url: Some("https://api.example.com".to_string()),
        api_key: Some("sk-key".to_string()),
        models: vec![crate::config::providers::ProviderModelSpec {
            id: "m1".to_string(),
            name: "Model One".to_string(),
            modalities: vec!["text".to_string()],
            context_window: 128000,
            max_tokens: 16384,
        }],
        ..Default::default()
    });
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);

    let models = read_json(&home.models_path());
    assert_eq!(models["providers"]["myprov"]["name"], "My Provider");
    assert_eq!(models["providers"]["myprov"]["models"][0]["id"], "m1");
    let auth = read_json(&home.auth_path());
    assert_eq!(auth["myprov"]["key"], "sk-key");

    // create mode must reject the now-existing id
    let mut cmd = make_cmd("upsert_provider");
    cmd.provider_config = Some(crate::config::providers::ProviderUpsertSpec {
        id: "myprov".to_string(),
        name: Some("Other".to_string()),
        create_only: true,
        ..Default::default()
    });
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(resp["error"].as_str().unwrap().contains("already exists"));

    // delete removes the models.json entry and the auth entry
    let mut cmd = make_cmd("delete_provider");
    cmd.provider_config = Some(crate::config::providers::ProviderUpsertSpec {
        id: "myprov".to_string(),
        ..Default::default()
    });
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);

    let models = read_json(&home.models_path());
    assert!(models["providers"].get("myprov").is_none());
    let auth = read_json(&home.auth_path());
    assert!(auth.get("myprov").is_none());
}

#[test]
fn provider_commands_reject_missing_payload_and_empty_id() {
    let state = make_app_state();

    for cmd_type in ["upsert_provider", "delete_provider"] {
        let resp = parse_response(&handle_command_internal(&state, make_cmd(cmd_type)));
        assert_eq!(resp["success"], false, "{cmd_type} without payload");

        let mut cmd = make_cmd(cmd_type);
        cmd.provider_config = Some(crate::config::providers::ProviderUpsertSpec {
            id: " ".to_string(),
            name: Some("x".to_string()),
            ..Default::default()
        });
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false, "{cmd_type} with empty id");
    }
}

#[test]
fn get_agent_info_returns_version() {
    let state = make_app_state();
    let cmd = make_cmd("get_agent_info");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert!(resp["data"]["version"].is_string());
    assert_eq!(resp["data"]["agentInstanceId"], "agent-test-instance");
}

#[test]
fn refresh_skills_returns_skill_list() {
    let state = make_app_state();
    let cmd = make_cmd("refresh_skills");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert!(resp["data"]["skills_count"].is_number());
    assert!(resp["data"]["skills"].is_array());
    assert_eq!(
        resp["data"]["skills_count"].as_u64().unwrap(),
        resp["data"]["skills"].as_array().unwrap().len() as u64
    );
    assert!(resp["data"]["refreshed"].is_boolean());
    // The get_state snapshot must follow the discovery cache: reload_config
    // needs a session, so refresh_skills is the only post-install path that
    // can update it. A stale welcome_skills made get_state keep reporting
    // the pre-install skill list.
    let welcomed = state.welcome_skills.read().clone();
    let returned: Vec<String> = resp["data"]["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect();
    assert_eq!(welcomed, returned);
}

#[test]
fn refresh_skills_works_without_session_id() {
    // Regression: refresh_skills is sessionless. The GUI/CLI fire it right
    // after install/uninstall with NO session_id; when it lived in the
    // session-scoped branch this returned "session not found", the skills
    // cache was never invalidated, and the installed list stayed stale
    // until restart / TTL expiry. make_cmd() always injects a session id,
    // so it hid this — build the command by hand with an empty session.
    let state = make_app_state();
    let cmd: RpcCommand =
        serde_json::from_str(r#"{"id":"test_cmd","type":"refresh_skills","sessionId":""}"#)
            .unwrap();
    assert!(cmd.session_id.is_empty());
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["command"], "refresh_skills");
}

#[test]
fn set_enabled_models_accepted() {
    let state = make_app_state();
    let cmd = make_cmd("set_enabled_models");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
}

#[test]
fn reload_auth_works() {
    let state = make_app_state();
    let cmd = make_cmd("reload_auth");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
}

#[test]
fn set_default_model_reports_unsaveable_settings() {
    let home = TestHome::new();
    let state = make_app_state();
    // A valid but READ-ONLY settings.json: load succeeds, save fails.
    let settings_path = home.settings_path();
    std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
    std::fs::write(&settings_path, "{}").unwrap();
    let mut perms = std::fs::metadata(&settings_path).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&settings_path, perms).unwrap();
    let candidate = {
        let registry = state.model_registry.read();
        let model = registry.all_models().first().unwrap().clone();
        format!("{}/{}", model.provider, model.id)
    };
    let mut cmd = make_cmd("set_default_model");
    cmd.model_id = candidate;
    let resp = parse_response(&handle_command_internal(&state, cmd));
    let mut perms = std::fs::metadata(&settings_path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&settings_path, perms).unwrap();
    assert_eq!(resp["success"], false);
    assert!(resp["error"]
        .as_str()
        .unwrap()
        .contains("failed to save settings"));
}

#[test]
fn get_commands_returns_list() {
    let state = make_app_state();
    let cmd = make_cmd("get_commands");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    let commands = resp["data"]["commands"].as_array().unwrap();
    // Commands list may be empty in minimal environments (no skills installed)
    assert!(commands.iter().all(|c| c.is_object()));
}

#[test]
fn sync_future_models_without_credentials_reports_not_synced() {
    let _home = TestHome::new();
    let state = make_app_state();
    let resp = parse_response(&handle_command_internal(
        &state,
        make_cmd("sync_future_models"),
    ));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["synced"], false);
    assert_eq!(
        resp["data"]["modelCount"],
        crate::models::cached_model_count()
    );
}

#[test]
fn set_default_model_rejects_empty_and_unknown_ids() {
    let _home = TestHome::new();
    let state = make_app_state();

    let resp = parse_response(&handle_command_internal(
        &state,
        make_cmd("set_default_model"),
    ));
    assert_eq!(resp["success"], false);
    assert!(resp["error"]
        .as_str()
        .unwrap()
        .contains("model_id is empty"));

    let mut cmd = make_cmd("set_default_model");
    cmd.model_id = "no-such-provider/no-such-model".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(resp["error"]
        .as_str()
        .unwrap()
        .contains("not in the catalog"));
}

#[test]
fn set_default_model_persists_catalog_entry() {
    let home = TestHome::new();
    let state = make_app_state();
    let candidate = {
        let registry = state.model_registry.read();
        let model = registry
            .all_models()
            .first()
            .expect("builtin catalog is never empty")
            .clone();
        format!("{}/{}", model.provider, model.id)
    };
    let mut cmd = make_cmd("set_default_model");
    cmd.model_id = candidate.clone();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["defaultModel"], candidate);
    let settings = read_json(&home.settings_path());
    assert_eq!(settings["defaultModel"], candidate);
}

#[test]
fn set_default_model_reports_unloadable_settings() {
    let home = TestHome::new();
    let state = make_app_state();
    // Corrupt settings.json so load_settings fails.
    let settings_path = home.settings_path();
    std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
    std::fs::write(&settings_path, "{not json").unwrap();
    let candidate = {
        let registry = state.model_registry.read();
        let model = registry.all_models().first().unwrap().clone();
        format!("{}/{}", model.provider, model.id)
    };
    let mut cmd = make_cmd("set_default_model");
    cmd.model_id = candidate;
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(resp["error"]
        .as_str()
        .unwrap()
        .contains("failed to load settings"));
}

// ── coverage batch 1: prompt-adjacent dispatch arms ─────────────────────

#[test]
fn list_models_sorts_and_includes_builtin_providers() {
    let home = TestHome::new();
    let state = make_app_state();
    let providers: Vec<String> = {
        let registry = state.model_registry.read();
        let mut providers: Vec<String> = registry
            .all_models()
            .iter()
            .map(|m| m.provider.clone())
            .collect();
        providers.sort();
        providers.dedup();
        providers.truncate(2);
        providers
    };
    assert!(providers.len() >= 2, "catalog has multiple providers");
    let mut auth = serde_json::json!({});
    for provider in &providers {
        auth[provider] = serde_json::json!({"type": "api_key", "key": "k"});
    }
    let auth_path = home.auth_path();
    std::fs::create_dir_all(auth_path.parent().unwrap()).unwrap();
    std::fs::write(&auth_path, serde_json::to_string_pretty(&auth).unwrap()).unwrap();
    // This test writes behind the Agent command boundary, so explicitly mimic
    // the atomic Registry swap that a real provider mutation performs.
    *state.model_registry.write() = crate::models::Registry::new();

    let mut cmd = make_cmd("list_models");
    cmd.include_builtin_providers = true;
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    let models = resp["data"]["models"].as_array().unwrap();
    assert!(models.len() >= 2);
    assert!(models.iter().all(|m| m["label"].is_string()));
    assert!(resp["data"]["builtinProviders"].is_object());
}

#[test]
fn set_auth_reports_mutation_error() {
    let home = TestHome::new();
    let state = make_app_state();
    let auth_path = home.auth_path();
    std::fs::create_dir_all(auth_path.parent().unwrap()).unwrap();
    std::fs::write(&auth_path, "{corrupt").unwrap();

    let mut cmd = make_cmd("set_auth");
    cmd.auth_update = Some(crate::config::providers::AuthMutation {
        provider: "custom".to_string(),
        key: Some("k".to_string()),
        ..Default::default()
    });
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
}

#[test]
fn list_models_uses_id_label_for_unnamed_model() {
    let _home = TestHome::new();
    let state = make_app_state();
    // The file loaders normalize an absent name to the model id, so a
    // genuinely empty name only exists via the verbatim test seam.
    state
        .model_registry
        .write()
        .test_insert(crate::models::Model {
            id: "unnamed-model".to_string(),
            name: String::new(),
            provider: "custom".to_string(),
            api_key: "k".to_string(),
            output: vec!["text".to_string()],
            ..Default::default()
        });
    let resp = parse_response(&handle_command_internal(&state, make_cmd("list_models")));
    let models = resp["data"]["models"].as_array().unwrap();
    let model = models
        .iter()
        .find(|m| m["id"] == "unnamed-model")
        .expect("unnamed model listed");
    assert_eq!(model["label"], "unnamed-model");
}

#[test]
fn list_models_lists_keyless_user_models() {
    // A user model from models.json with no apiKey (e.g. a local endpoint)
    // must still appear in list_models — the credential filter only
    // applies to builtin catalog entries.
    let home = TestHome::new();
    std::fs::create_dir_all(home.models_path().parent().unwrap()).unwrap();
    std::fs::write(
        home.models_path(),
        r#"{
              "providers": {
                "local": {
                  "api": "openai-completions",
                  "baseUrl": "http://127.0.0.1:8000/v1",
                  "models": [
                    {"id": "local-model", "modalities": ["text"]}
                  ]
                }
              }
            }"#,
    )
    .unwrap();
    let state = make_app_state();
    let resp = parse_response(&handle_command_internal(&state, make_cmd("list_models")));
    let models = resp["data"]["models"].as_array().unwrap();
    let model = models
        .iter()
        .find(|m| m["id"] == "local-model")
        .expect("keyless user model listed");
    assert_eq!(model["provider"], "local");
    assert_eq!(model["label"], "local-model");
}

#[test]
fn upsert_provider_rejects_no_change_and_builtin_ids() {
    let _home = TestHome::new();
    let state = make_app_state();

    // id only, no change fields.
    let mut cmd = make_cmd("upsert_provider");
    cmd.provider_config = Some(crate::config::providers::ProviderUpsertSpec {
        id: "custom".to_string(),
        ..Default::default()
    });
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(resp["error"].as_str().unwrap().contains("no change"));

    // A pure base-URL override defines no custom provider (name/api/models/
    // key all absent), so it is legitimately allowed even under a custom
    // id. This also exercises every short-circuit arm of the
    // defines-custom-provider guard (all four operands evaluate false).
    let mut cmd = make_cmd("upsert_provider");
    cmd.provider_config = Some(crate::config::providers::ProviderUpsertSpec {
        id: "custom".to_string(),
        base_url: Some("https://override.example.com".to_string()),
        ..Default::default()
    });
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);

    // A built-in id cannot be redefined with a name.
    let builtin = {
        let registry = state.model_registry.read();
        let mut ids: Vec<String> = registry.builtin_provider_ids().into_iter().collect();
        ids.sort();
        ids.first().expect("builtin catalog").clone()
    };
    let mut cmd = make_cmd("upsert_provider");
    cmd.provider_config = Some(crate::config::providers::ProviderUpsertSpec {
        id: builtin.clone(),
        name: Some("Hijacked".to_string()),
        ..Default::default()
    });
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(resp["error"].as_str().unwrap().contains("reserved"));
}

#[test]
fn delete_provider_rejects_builtin_and_reports_storage_errors() {
    let home = TestHome::new();
    let state = make_app_state();

    let builtin = {
        let registry = state.model_registry.read();
        let mut ids: Vec<String> = registry.builtin_provider_ids().into_iter().collect();
        ids.sort();
        ids.first().expect("builtin catalog").clone()
    };
    let mut cmd = make_cmd("delete_provider");
    cmd.provider_config = Some(crate::config::providers::ProviderUpsertSpec {
        id: builtin,
        ..Default::default()
    });
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(resp["error"].as_str().unwrap().contains("reserved"));

    // Corrupt models.json → the delete write path reports an error.
    let models_path = home.models_path();
    std::fs::create_dir_all(models_path.parent().unwrap()).unwrap();
    std::fs::write(&models_path, "{corrupt").unwrap();
    let mut cmd = make_cmd("delete_provider");
    cmd.provider_config = Some(crate::config::providers::ProviderUpsertSpec {
        id: "custom-provider".to_string(),
        ..Default::default()
    });
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
}

#[test]
fn list_providers_reports_builtin_and_custom_providers() {
    let home = TestHome::new();
    let state = make_app_state();

    // Pick a real builtin provider id so the baseUrl-override branch of the
    // builtin loop (and its `override_url.unwrap_or(...)` fallback) is hit.
    let builtin_id = {
        let registry = state.model_registry.read();
        registry
            .builtin_provider_summaries()
            .keys()
            .next()
            .expect("builtin catalog is never empty")
            .clone()
    };
    // A second builtin id, this time shadowed by a full custom config (name +
    // api) so the builtin loop hits the `custom_ids.contains(..) => continue`
    // arm and the entry shows up in the custom list instead.
    let shadow_id = {
        let registry = state.model_registry.read();
        let mut ids: Vec<String> = registry
            .builtin_provider_summaries()
            .keys()
            .cloned()
            .collect();
        ids.sort();
        ids.into_iter()
            .find(|id| *id != builtin_id)
            .expect("catalog has multiple builtin providers")
    };

    // models.json: a full custom provider (name/api/models), an api-only
    // provider (name falls back to id), an override-only provider (baseUrl
    // only — filtered out of both custom_ids and the custom list), a
    // baseUrl override for a builtin id, and a full custom config that
    // shadows a second builtin id.
    std::fs::create_dir_all(home.models_path().parent().unwrap()).unwrap();
    std::fs::write(
        home.models_path(),
        serde_json::json!({
            "providers": {
                "myprov": {
                    "name": "My Provider",
                    "api": "anthropic",
                    "baseUrl": "https://api.example.com",
                    "models": [
                        {"id": "m1", "name": "Model One", "modalities": ["text"], "contextWindow": 128001, "maxTokens": 16001},
                        {"id": "m2", "modalities": ["text", "image"]}
                    ]
                },
                "noname": {
                    "api": "openai-completions",
                    "models": [{"id": "n1", "modalities": ["text"]}]
                },
                "override-only": {
                    "baseUrl": "https://override.example.com"
                },
                (builtin_id.clone()): {
                    "baseUrl": "https://custom-builtin.example.com"
                },
                (shadow_id.clone()): {
                    "name": "Shadowed Builtin",
                    "api": "openai-completions"
                }
            }
        })
        .to_string(),
    )
    .unwrap();

    // auth.json: a key for myprov (has_key true) and a keyless entry for
    // noname (has_key false).
    std::fs::create_dir_all(home.auth_path().parent().unwrap()).unwrap();
    std::fs::write(
        home.auth_path(),
        serde_json::json!({
            "myprov": {"type": "api_key", "key": "sk-key"},
            "noname": {"type": "api_key"}
        })
        .to_string(),
    )
    .unwrap();

    let resp = parse_response(&handle_command_internal(&state, make_cmd("list_providers")));
    assert_eq!(resp["success"], true);

    let builtin = resp["data"]["builtin"].as_array().unwrap();
    let custom = resp["data"]["custom"].as_array().unwrap();

    // builtin: future first, then the overridden builtin provider.
    assert_eq!(builtin[0]["id"], "future");
    let overridden = builtin
        .iter()
        .find(|p| p["id"] == builtin_id)
        .expect("overridden builtin present");
    assert_eq!(overridden["baseUrl"], "https://custom-builtin.example.com");
    assert_eq!(overridden["hasApiKey"], false);
    // The shadowed builtin id is skipped from the builtin list (custom_ids
    // collision) and surfaces as a custom provider instead.
    assert!(builtin.iter().all(|p| p["id"] != shadow_id));

    // custom: myprov, noname and the shadowed builtin (sorted by id);
    // override-only filtered out.
    let ids: Vec<&str> = custom.iter().map(|p| p["id"].as_str().unwrap()).collect();
    let mut expected = vec!["myprov", "noname", shadow_id.as_str()];
    expected.sort();
    assert_eq!(ids, expected);
    let shadowed = custom
        .iter()
        .find(|p| p["id"] == shadow_id)
        .expect("shadowed builtin present in custom list");
    assert_eq!(shadowed["name"], "Shadowed Builtin");

    let myprov = custom.iter().find(|p| p["id"] == "myprov").unwrap();
    assert_eq!(myprov["name"], "My Provider");
    assert_eq!(myprov["hasApiKey"], true);
    let myprov_models = myprov["models"].as_array().unwrap();
    assert_eq!(myprov_models.len(), 2);
    let m1 = myprov_models.iter().find(|m| m["id"] == "m1").unwrap();
    assert_eq!(m1["name"], "Model One");
    assert_eq!(m1["supportsImages"], false);
    assert_eq!(m1["contextWindow"], 128001);
    assert_eq!(m1["maxTokens"], 16001);
    let m2 = myprov_models.iter().find(|m| m["id"] == "m2").unwrap();
    assert_eq!(m2["name"], "m2");
    assert_eq!(m2["supportsImages"], true);

    let noname = custom.iter().find(|p| p["id"] == "noname").unwrap();
    assert_eq!(noname["name"], "noname");
    assert_eq!(noname["hasApiKey"], false);
}

#[test]
fn list_providers_reports_error_on_corrupt_documents() {
    let home = TestHome::new();
    let state = make_app_state();
    std::fs::create_dir_all(home.models_path().parent().unwrap()).unwrap();
    std::fs::write(home.models_path(), "{corrupt").unwrap();
    let resp = parse_response(&handle_command_internal(&state, make_cmd("list_providers")));
    assert_eq!(resp["success"], false);
}

#[test]
fn upsert_provider_create_only_reports_created() {
    let _home = TestHome::new();
    let state = make_app_state();
    let mut cmd = make_cmd("upsert_provider");
    cmd.provider_config = Some(crate::config::providers::ProviderUpsertSpec {
        id: "brand-new-prov".to_string(),
        name: Some("Brand New".to_string()),
        api: Some("openai-completions".to_string()),
        create_only: true,
        ..Default::default()
    });
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["id"], "brand-new-prov");
}

#[test]
fn get_commands_lists_discovered_skills() {
    let home = TestHome::new();
    let skill_dir = home.path().join(".future/agent/skills/cov-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: cov-skill\ndescription: coverage fixture\n---\n# body\n",
    )
    .unwrap();
    // A second skill guarantees the sort_by comparator runs (it is
    // skipped for a 0/1-element list).
    let skill_dir_b = home.path().join(".future/agent/skills/aaa-skill");
    std::fs::create_dir_all(&skill_dir_b).unwrap();
    std::fs::write(
        skill_dir_b.join("SKILL.md"),
        "---\nname: aaa-skill\ndescription: sorts before cov-skill\n---\n# body\n",
    )
    .unwrap();
    crate::skills::invalidate_skills_cache();

    let state = make_app_state();
    let resp = parse_response(&handle_command_internal(&state, make_cmd("get_commands")));
    assert_eq!(resp["success"], true);
    let commands = resp["data"]["commands"].as_array().unwrap();
    assert!(commands.iter().any(|c| c["name"] == "cov-skill"));
    // aaa-skill sorts before cov-skill, proving the comparator ran.
    let names: Vec<&str> = commands
        .iter()
        .filter_map(|c| c["name"].as_str())
        .filter(|n| n.ends_with("-skill"))
        .collect();
    assert!(
        names.windows(2).all(|w| w[0] <= w[1]),
        "{names:?} not sorted"
    );
    crate::skills::invalidate_skills_cache();
}
