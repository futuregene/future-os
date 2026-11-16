//! Tests for the `future models providers|provider|model|probe` commands.
//!
//! Every command talks to the Agent, so these drive a mock Agent and assert
//! both halves that matter: what the CLI sent (the write really carries every
//! setting the caller did not name) and what it printed (the caller can see
//! what landed).

use super::*;
use crate::test_server::{spawn_mock, MockAgent};
use std::ffi::OsString;

fn env_for(addr: &str) -> crate::test_env::EnvGuard {
    crate::test_env::EnvGuard::set(&[("FUTURE_AGENT_GRPC_ADDR", OsString::from(addr.to_string()))])
}

const VIEW: &str = r#"{
    "builtin": [
        {"id":"future","name":"Future","baseUrl":"https://api.example.test","hasApiKey":true,"modelCount":9,"requiresBaseUrl":false},
        {"id":"deepseek","name":"DeepSeek","baseUrl":"https://api.deepseek.test","hasApiKey":true,"modelCount":3,"requiresBaseUrl":false}
    ],
    "custom": [{
        "id":"acme","name":"Acme","api":"openai-completions","baseUrl":"https://gw.example.test","hasApiKey":false,
        "models":[{"id":"m1","name":"Model One","supportsImages":true,"reasoning":true,
                   "thinkingLevels":["low","high"],"contextWindow":200000,"maxTokens":8000,
                   "inputCost":1.5,"outputCost":6.0,"cacheReadCost":0.0,"cacheWriteCost":0.0}]
    }]
}"#;

fn stdout(cap: &crate::output::Captured) -> String {
    String::from_utf8(cap.out.lock().unwrap().clone()).unwrap()
}

#[tokio::test]
async fn providers_prints_every_setting_and_echoes_the_view_as_json() {
    let _guard = crate::test_env::lock_env().await;
    let addr = spawn_mock(MockAgent::respond("list_providers", VIEW)).await;
    let _env = env_for(&addr);

    let (out, cap) = Output::memory();
    providers(&[], &out).await.expect("providers");
    let printed = stdout(&cap);
    assert!(
        printed.contains("future  built-in  https://api.example.test  key=set  models=9"),
        "{printed}"
    );
    assert!(
        printed.contains(
            "acme  custom  openai-completions  https://gw.example.test  key=none  models=1"
        ),
        "{printed}"
    );
    // Every model setting is on the line: limits, modalities, levels, prices.
    assert!(
        printed
            .contains("  m1  Model One  ctx=200000  max=8000  input=text+image  thinking=low,high"),
        "{printed}"
    );
    assert!(printed.contains("price(in=1.5,out=6,"), "{printed}");
    assert!(
        printed.contains("2 built-in providers, 1 custom providers, 1 custom models."),
        "{printed}"
    );

    let (out, cap) = Output::memory();
    providers(&["--json".to_string()], &out)
        .await
        .expect("providers json");
    let parsed: Value = serde_json::from_str(&stdout(&cap)).expect("json");
    assert_eq!(parsed["custom"][0]["models"][0]["id"], "m1");
}

#[tokio::test]
async fn a_builtin_write_sends_key_and_address_in_one_command() {
    let _guard = crate::test_env::lock_env().await;
    let agent = MockAgent::respond("list_providers", VIEW);
    let addr = spawn_mock(agent.clone()).await;
    let _env = env_for(&addr);

    let (out, _cap) = Output::memory();
    provider(
        &[
            "set".to_string(),
            "deepseek".to_string(),
            "--key".to_string(),
            "sk-live".to_string(),
            "--base-url".to_string(),
            "https://proxy.example.test".to_string(),
        ],
        &out,
    )
    .await
    .expect("provider set");

    let sent = agent.seen_of("upsert_provider");
    assert_eq!(sent.len(), 1, "{sent:?}");
    let config = sent[0].provider_config.as_ref().expect("provider_config");
    assert_eq!(config.id, "deepseek");
    assert_eq!(config.api_key, "sk-live");
    assert_eq!(config.base_url, "https://proxy.example.test");
    assert!(!config.create_only);
    // A built-in write never redefines the provider.
    assert!(config.name.is_empty() && config.api.is_empty());
    assert!(config.models.is_empty());
}

#[tokio::test]
async fn builtin_writes_refuse_to_redefine_the_provider() {
    let _guard = crate::test_env::lock_env().await;
    let agent = MockAgent::respond("list_providers", VIEW);
    let addr = spawn_mock(agent.clone()).await;
    let _env = env_for(&addr);

    let (out, _cap) = Output::memory();
    let error = provider(
        &[
            "set".to_string(),
            "deepseek".to_string(),
            "--api".to_string(),
            "anthropic".to_string(),
        ],
        &out,
    )
    .await
    .expect_err("refused");
    assert!(error.contains("built-in provider"), "{error}");
    assert!(agent.seen_of("upsert_provider").is_empty());

    let (out, _cap) = Output::memory();
    let error = provider(
        &[
            "set".to_string(),
            "future".to_string(),
            "--key".to_string(),
            "k".to_string(),
        ],
        &out,
    )
    .await
    .expect_err("refused");
    assert!(error.contains("future auth login"), "{error}");
}

#[tokio::test]
async fn creating_a_custom_provider_needs_its_name_api_and_address() {
    let _guard = crate::test_env::lock_env().await;
    let agent = MockAgent::respond("list_providers", VIEW);
    let addr = spawn_mock(agent.clone()).await;
    let _env = env_for(&addr);

    let (out, _cap) = Output::memory();
    let error = provider(
        &[
            "set".to_string(),
            "newco".to_string(),
            "--key".to_string(),
            "k".to_string(),
        ],
        &out,
    )
    .await
    .expect_err("refused");
    assert!(error.contains("--name, --api and --base-url"), "{error}");

    let (out, _cap) = Output::memory();
    provider(
        &[
            "set".to_string(),
            "newco".to_string(),
            "--name".to_string(),
            "New Co".to_string(),
            "--api".to_string(),
            "anthropic".to_string(),
            "--base-url".to_string(),
            "https://newco.example.test".to_string(),
        ],
        &out,
    )
    .await
    .expect("created");
    let sent = agent.seen_of("upsert_provider");
    assert_eq!(sent.len(), 1);
    let config = sent[0].provider_config.as_ref().expect("provider_config");
    assert!(config.create_only, "a create must not silently overwrite");
    assert_eq!(config.name, "New Co");
    assert_eq!(config.api, "anthropic");
}

#[tokio::test]
async fn editing_a_custom_provider_keeps_what_was_not_named() {
    let _guard = crate::test_env::lock_env().await;
    let agent = MockAgent::respond("list_providers", VIEW);
    let addr = spawn_mock(agent.clone()).await;
    let _env = env_for(&addr);

    let (out, _cap) = Output::memory();
    provider(
        &[
            "set".to_string(),
            "acme".to_string(),
            "--name".to_string(),
            "Acme Renamed".to_string(),
        ],
        &out,
    )
    .await
    .expect("edited");
    let sent = agent.seen_of("upsert_provider");
    let config = sent[0].provider_config.as_ref().expect("provider_config");
    assert_eq!(config.name, "Acme Renamed");
    // api / base URL come from the stored provider, and an absent --key must
    // not clear the stored credential.
    assert_eq!(config.api, "openai-completions");
    assert_eq!(config.base_url, "https://gw.example.test");
    assert!(config.api_key.is_empty());
    assert!(!config.clear_api_key);
}

#[tokio::test]
async fn model_set_adds_a_model_and_keeps_every_other_one() {
    let _guard = crate::test_env::lock_env().await;
    let mut agent = MockAgent::respond("list_providers", VIEW);
    // The view after the write: the new model is there, and m1 is untouched.
    let after = VIEW.replace(
        r#""models":[{"id":"m1""#,
        r#""models":[{"id":"gpt-4o","name":"GPT-4o","supportsImages":true,"reasoning":true,"contextWindow":128000,"maxTokens":4096,"thinkingLevels":["low","high"],"inputCost":0.0,"outputCost":0.0,"cacheReadCost":0.0,"cacheWriteCost":0.0},{"id":"m1""#,
    );
    agent
        .sequences
        .insert("list_providers".to_string(), vec![VIEW.to_string(), after]);
    let addr = spawn_mock(agent.clone()).await;
    let _env = env_for(&addr);

    let (out, cap) = Output::memory();
    model(
        &[
            "set".to_string(),
            "acme/gpt-4o".to_string(),
            "--name".to_string(),
            "GPT-4o".to_string(),
            "--context-window".to_string(),
            "128000".to_string(),
            "--max-tokens".to_string(),
            "4096".to_string(),
            "--images".to_string(),
            "--thinking-levels".to_string(),
            "low,high".to_string(),
        ],
        &out,
    )
    .await
    .expect("model set");

    let sent = agent.seen_of("upsert_provider");
    assert_eq!(sent.len(), 1);
    let config = sent[0].provider_config.as_ref().expect("provider_config");
    assert!(config.replace_models);
    assert_eq!(config.id, "acme");
    assert_eq!(config.models.len(), 2);
    let added = config
        .models
        .iter()
        .find(|model| model.id == "gpt-4o")
        .expect("added model");
    assert_eq!(added.modalities, ["text", "image"]);
    assert_eq!(added.context_window, 128_000);
    assert_eq!(added.max_tokens, 4096);
    assert_eq!(added.thinking_levels, ["low", "high"]);
    assert_eq!(added.replace_thinking_levels, Some(true));
    let kept = config
        .models
        .iter()
        .find(|model| model.id == "m1")
        .expect("kept model");
    assert_eq!(kept.context_window, 200_000);
    assert_eq!(kept.max_tokens, 8000);
    assert_eq!(kept.cost_input, 1.5);
    // An untouched model's level list is left to the Agent, so an inherited
    // provider-level list is not frozen into a model-level one.
    assert!(kept.thinking_levels.is_empty());
    assert_eq!(kept.replace_thinking_levels, None);

    let printed = stdout(&cap);
    assert!(printed.contains("Added model acme/gpt-4o"), "{printed}");
    assert!(printed.contains("thinking=low,high"), "{printed}");
}

#[tokio::test]
async fn model_edits_and_removals_name_the_model_they_could_not_find() {
    let _guard = crate::test_env::lock_env().await;
    let agent = MockAgent::respond("list_providers", VIEW);
    let addr = spawn_mock(agent.clone()).await;
    let _env = env_for(&addr);

    let (out, _cap) = Output::memory();
    let error = model_remove(&["acme/missing".to_string()], &out)
        .await
        .expect_err("refused");
    assert!(error.contains("no model `missing`"), "{error}");
    assert!(agent.seen_of("upsert_provider").is_empty());

    // A built-in provider's models come from the catalog.
    let (out, _cap) = Output::memory();
    let error = model(&["set".to_string(), "deepseek/m1".to_string()], &out)
        .await
        .expect_err("refused");
    assert!(error.contains("built-in provider"), "{error}");
}

#[tokio::test]
async fn a_single_level_probe_reports_a_refusal_as_an_answer() {
    let _guard = crate::test_env::lock_env().await;
    let agent = MockAgent::respond(
        "probe_thinking_level",
        r#"{"level":"high","ok":false,"error":"unsupported value"}"#,
    );
    let addr = spawn_mock(agent.clone()).await;
    let _env = env_for(&addr);

    let (out, cap) = Output::memory();
    probe(
        &[
            "acme/m1".to_string(),
            "--level".to_string(),
            "high".to_string(),
        ],
        &out,
    )
    .await
    .expect("a verdict, not a failure");
    let printed = stdout(&cap);
    assert!(printed.contains("rejected  unsupported value"), "{printed}");
    assert!(printed.contains("Accepted: none"), "{printed}");

    let sent = agent.seen_of("probe_thinking_level");
    let probe = sent[0].thinking_probe.as_ref().expect("thinking_probe");
    assert_eq!(probe.provider_id, "acme");
    assert_eq!(probe.model_id, "m1");
    assert_eq!(probe.level, "high");
    // Nothing is restated: the Agent resolves the configured model's wire
    // details, and the stored key is used.
    assert!(probe.api.is_empty() && probe.base_url.is_empty() && probe.api_key.is_empty());
}

#[tokio::test]
async fn a_failed_calibration_is_a_command_failure() {
    let _guard = crate::test_env::lock_env().await;
    let agent = MockAgent::respond(
        "probe_thinking_level",
        r#"{"level":"default","ok":false,"error":"invalid api key"}"#,
    );
    let addr = spawn_mock(agent.clone()).await;
    let _env = env_for(&addr);

    let (out, cap) = Output::memory();
    let error = probe(&["acme/m1".to_string()], &out)
        .await
        .expect_err("calibration failure is not a level verdict");
    assert!(error.contains("calibration request"), "{error}");
    assert!(error.contains("invalid api key"), "{error}");
    // The report still names what was tried, and the calibration came first.
    let printed = stdout(&cap);
    assert!(printed.contains("default   rejected"), "{printed}");
    let sent = agent.seen_of("probe_thinking_level");
    // One call only: with a broken endpoint no later verdict would mean
    // anything, so the run stops instead of spending one call per level.
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(
        sent[0].thinking_probe.as_ref().expect("probe").level,
        "default"
    );
}

#[tokio::test]
async fn an_explicit_probe_target_travels_verbatim() {
    let _guard = crate::test_env::lock_env().await;
    let agent = MockAgent::respond("probe_thinking_level", r#"{"level":"low","ok":true}"#);
    let addr = spawn_mock(agent.clone()).await;
    let _env = env_for(&addr);

    let (out, _cap) = Output::memory();
    probe(
        &[
            "newco/qwen".to_string(),
            "--level".to_string(),
            "low".to_string(),
            "--api".to_string(),
            "openai-completions".to_string(),
            "--base-url".to_string(),
            "https://newco.example.test".to_string(),
            "--key".to_string(),
            "sk-probe".to_string(),
        ],
        &out,
    )
    .await
    .expect("probed");
    let sent = agent.seen_of("probe_thinking_level");
    let probe = sent[0].thinking_probe.as_ref().expect("probe");
    assert_eq!(probe.api, "openai-completions");
    assert_eq!(probe.base_url, "https://newco.example.test");
    assert_eq!(probe.api_key, "sk-probe");
}

#[tokio::test]
async fn removals_report_what_the_agent_did() {
    let _guard = crate::test_env::lock_env().await;
    let agent = MockAgent::respond("list_providers", VIEW);
    let addr = spawn_mock(agent.clone()).await;
    let _env = env_for(&addr);

    let (out, _cap) = Output::memory();
    let error = provider_remove(&["deepseek".to_string()], &out)
        .await
        .expect_err("refused");
    assert!(error.contains("cannot be removed"), "{error}");

    let (out, cap) = Output::memory();
    model_remove(&["acme/m1".to_string()], &out)
        .await
        .expect("removed");
    assert!(stdout(&cap).contains("Removed model acme/m1."));
    let sent = agent.seen_of("upsert_provider");
    assert_eq!(sent.len(), 1);
    assert!(sent[0]
        .provider_config
        .as_ref()
        .expect("provider_config")
        .models
        .is_empty());
}

/// An invented flag must fail loudly, never be dropped: the whole point of the
/// shared parser is that a typo cannot look like a successful write.
#[tokio::test]
async fn an_unknown_option_is_refused() {
    let (out, _cap) = Output::memory();
    let error = model_set(&["acme/m1".to_string(), "--jsno".to_string()], &out)
        .await
        .expect_err("refused");
    assert!(error.contains("unknown option"), "{error}");
}
