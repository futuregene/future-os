//! Interactive model-provider setup for `future config`.
//!
//! Two setup paths are intentionally small and opinionated:
//! - FutureOS reuses the existing device-code login flow, asking before it
//!   replaces a token already stored in `auth.json`.
//! - Custom creates or updates one provider and one model across the agent's
//!   `models.json` and `auth.json` documents.
//!
//! When an agent is running, custom-provider writes go through its RPC so the
//! live registry changes atomically with the files. Otherwise the CLI uses the
//! same validated, atomic file writer exported by the agent crate.

use crate::commands::auth;
use crate::output::Output;
#[cfg(not(test))]
use crate::rpc::{grpc_addr, RunClient};
use future_agent::config::providers::{ProviderModelSpec, ProviderUpsertSpec};
#[cfg(not(test))]
use future_rpc::proto::{ProviderModel, ProviderUpsert};
use std::io;

const DEFAULT_PROVIDER_ID: &str = "custom";
const DEFAULT_CONTEXT_WINDOW: i32 = 128_000;
const DEFAULT_MAX_TOKENS: i32 = 16_384;

/// Input boundary kept injectable so the interactive flow can be tested
/// without replacing process-global stdin.
trait Prompter {
    fn read_line(&mut self) -> Result<String, String>;
    fn read_secret(&mut self) -> Result<String, String>;
}

struct StdioPrompter;

impl Prompter for StdioPrompter {
    fn read_line(&mut self) -> Result<String, String> {
        let mut value = String::new();
        let read = io::stdin()
            .read_line(&mut value)
            .map_err(|error| format!("Failed to read input: {error}"))?;
        if read == 0 {
            return Err("Input closed before configuration completed.".to_string());
        }
        Ok(value.trim().to_string())
    }

    fn read_secret(&mut self) -> Result<String, String> {
        rpassword::read_password()
            .map(|value| value.trim().to_string())
            .map_err(|error| format!("Failed to read API key: {error}"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderChoice {
    FutureOs,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CustomProviderInput {
    id: String,
    name: String,
    api: String,
    base_url: String,
    api_key: Option<String>,
    model_id: String,
    model_name: String,
    supports_images: bool,
    context_window: i32,
    max_tokens: i32,
}

/// Run the interactive provider configurator.
pub async fn configure(out: &Output) -> Result<(), String> {
    let mut prompter = StdioPrompter;
    configure_with(&mut prompter, out).await
}

async fn configure_with(prompter: &mut dyn Prompter, out: &Output) -> Result<(), String> {
    out.log("Configure a model provider:");
    out.log("  1) FutureOS");
    out.log("  2) Custom provider");

    match ask_provider_choice(prompter, out)? {
        ProviderChoice::FutureOs => configure_futureos(prompter, out).await,
        ProviderChoice::Custom => configure_custom(prompter, out).await,
    }
}

fn prompt(
    prompter: &mut dyn Prompter,
    out: &Output,
    message: &str,
    secret: bool,
) -> Result<String, String> {
    out.write_out(message);
    out.flush();
    let result = if secret {
        prompter.read_secret()
    } else {
        prompter.read_line()
    };
    // Password readers disable terminal echo, including the Enter key, so move
    // subsequent output onto a fresh line. Fake/test prompt readers need the
    // same deterministic output contract.
    if secret {
        out.log("");
    }
    result
}

fn ask_provider_choice(
    prompter: &mut dyn Prompter,
    out: &Output,
) -> Result<ProviderChoice, String> {
    loop {
        let value = prompt(prompter, out, "Select provider [1]: ", false)?;
        match value.to_ascii_lowercase().as_str() {
            "" | "1" | "future" | "futureos" => return Ok(ProviderChoice::FutureOs),
            "2" | "custom" => return Ok(ProviderChoice::Custom),
            _ => out.log_err("Please enter 1 for FutureOS or 2 for a custom provider."),
        }
    }
}

fn ask_yes_no(
    prompter: &mut dyn Prompter,
    out: &Output,
    message: &str,
    default: bool,
) -> Result<bool, String> {
    loop {
        let value = prompt(prompter, out, message, false)?;
        match value.to_ascii_lowercase().as_str() {
            "" => return Ok(default),
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => out.log_err("Please answer yes or no."),
        }
    }
}

async fn configure_futureos(prompter: &mut dyn Prompter, out: &Output) -> Result<(), String> {
    let auth_file = auth::load_auth_file().await?;
    let has_token = auth::get_future_auth_entry(&auth_file)
        .and_then(|entry| entry.key)
        .is_some_and(|key| !key.trim().is_empty());

    if has_token {
        let relogin = ask_yes_no(
            prompter,
            out,
            "A FutureOS token is already configured. Log in again? [y/N]: ",
            false,
        )?;
        if !relogin {
            out.log("FutureOS is already configured; no changes were made.");
            return Ok(());
        }
    } else {
        out.log("No FutureOS token found. Starting login...");
    }

    auth::login(None, out).await
}

async fn configure_custom(prompter: &mut dyn Prompter, out: &Output) -> Result<(), String> {
    let input = collect_custom_provider(prompter, out)?;
    let qualified_model = format!("{}/{}", input.id, input.model_id);
    save_custom_provider(&input).await?;

    out.log(&format!(
        "Configured custom provider '{}' with model '{}'.",
        input.id, qualified_model
    ));
    out.log(&format!(
        "Model configuration: {}",
        future_agent::config::providers::models_json_path().display()
    ));
    if input.api_key.is_some() {
        out.log(&format!(
            "Credential: {}",
            future_agent::config::providers::auth_json_path().display()
        ));
    }
    Ok(())
}

fn collect_custom_provider(
    prompter: &mut dyn Prompter,
    out: &Output,
) -> Result<CustomProviderInput, String> {
    let id = loop {
        let value = prompt(
            prompter,
            out,
            &format!("Provider ID [{DEFAULT_PROVIDER_ID}]: "),
            false,
        )?;
        let value = if value.is_empty() {
            DEFAULT_PROVIDER_ID.to_string()
        } else {
            value.to_ascii_lowercase()
        };
        if valid_provider_id(&value) && !is_reserved_provider_id(&value) {
            break value;
        }
        out.log_err(
            "Provider ID must use lowercase letters, digits, '-' or '_', and must not be a built-in provider ID.",
        );
    };

    let name = {
        let value = prompt(
            prompter,
            out,
            &format!("Provider display name [{id}]: "),
            false,
        )?;
        if value.is_empty() {
            id.clone()
        } else {
            value
        }
    };

    out.log("API protocol:");
    out.log("  1) OpenAI Chat Completions");
    out.log("  2) OpenAI Responses");
    out.log("  3) Anthropic Messages");
    let api = loop {
        let value = prompt(prompter, out, "Select protocol [1]: ", false)?;
        match value.to_ascii_lowercase().as_str() {
            "" | "1" | "openai" | "openai-completions" => break "openai-completions".to_string(),
            "2" | "openai-responses" | "responses" => break "openai-responses".to_string(),
            "3" | "anthropic" => break "anthropic".to_string(),
            _ => out.log_err("Please enter 1, 2, or 3."),
        }
    };

    let base_url = loop {
        let value = prompt(prompter, out, "Base URL: ", false)?;
        match reqwest::Url::parse(&value) {
            Ok(url) if matches!(url.scheme(), "http" | "https") => break value,
            _ => out.log_err("Base URL must be a valid http:// or https:// URL."),
        }
    };

    let api_key = loop {
        let value = prompt(
            prompter,
            out,
            "API key (leave blank for a keyless local endpoint): ",
            true,
        )?;
        if value.len() <= 16_384
            && value.is_ascii()
            && !value.bytes().any(|byte| byte.is_ascii_control())
        {
            break (!value.is_empty()).then_some(value);
        }
        out.log_err(
            "API key must be ASCII, contain no control characters, and be at most 16384 bytes.",
        );
    };

    let model_id = loop {
        let value = prompt(prompter, out, "Model ID (sent to the API): ", false)?;
        if !value.is_empty()
            && value.len() <= 256
            && value.is_ascii()
            && !value.bytes().any(|byte| byte.is_ascii_control())
        {
            break value;
        }
        out.log_err("Model ID is required and must be at most 256 ASCII characters.");
    };

    let model_name = {
        let value = prompt(
            prompter,
            out,
            &format!("Model display name [{model_id}]: "),
            false,
        )?;
        if value.is_empty() {
            model_id.clone()
        } else {
            value
        }
    };

    let context_window = ask_positive_i32(
        prompter,
        out,
        &format!("Context window [{DEFAULT_CONTEXT_WINDOW}]: "),
        DEFAULT_CONTEXT_WINDOW,
    )?;
    let max_tokens = loop {
        let value = ask_positive_i32(
            prompter,
            out,
            &format!("Maximum output tokens [{DEFAULT_MAX_TOKENS}]: "),
            DEFAULT_MAX_TOKENS,
        )?;
        if value <= context_window {
            break value;
        }
        out.log_err("Maximum output tokens cannot exceed the context window.");
    };
    let supports_images = ask_yes_no(
        prompter,
        out,
        "Does this model accept image input? [y/N]: ",
        false,
    )?;

    let input = CustomProviderInput {
        id,
        name,
        api,
        base_url,
        api_key,
        model_id,
        model_name,
        supports_images,
        context_window,
        max_tokens,
    };
    // Keep the CLI's local validation exactly aligned with the authoritative
    // agent writer before attempting either an RPC or an offline write.
    future_agent::config::providers::validate_provider_upsert(&provider_spec(&input))?;
    Ok(input)
}

fn ask_positive_i32(
    prompter: &mut dyn Prompter,
    out: &Output,
    message: &str,
    default: i32,
) -> Result<i32, String> {
    loop {
        let value = prompt(prompter, out, message, false)?;
        if value.is_empty() {
            return Ok(default);
        }
        if let Ok(number) = value.parse::<i32>() {
            if number > 0 {
                return Ok(number);
            }
        }
        out.log_err("Please enter a positive whole number.");
    }
}

fn valid_provider_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_".contains(&byte))
}

fn is_reserved_provider_id(id: &str) -> bool {
    id == "future"
        || future_agent::models::builtin_models_shared()
            .iter()
            .any(|model| model.provider == id)
}

fn provider_spec(input: &CustomProviderInput) -> ProviderUpsertSpec {
    ProviderUpsertSpec {
        id: input.id.clone(),
        name: Some(input.name.clone()),
        api: Some(input.api.clone()),
        base_url: Some(input.base_url.clone()),
        models: vec![ProviderModelSpec {
            id: input.model_id.clone(),
            name: input.model_name.clone(),
            modalities: if input.supports_images {
                vec!["text".to_string(), "image".to_string()]
            } else {
                vec!["text".to_string()]
            },
            context_window: input.context_window,
            max_tokens: input.max_tokens,
        }],
        replace_models: true,
        api_key: input.api_key.clone(),
        ..Default::default()
    }
}

#[cfg(not(test))]
fn provider_rpc(input: &CustomProviderInput) -> ProviderUpsert {
    ProviderUpsert {
        id: input.id.clone(),
        name: input.name.clone(),
        api: input.api.clone(),
        base_url: input.base_url.clone(),
        models: vec![ProviderModel {
            id: input.model_id.clone(),
            name: input.model_name.clone(),
            modalities: if input.supports_images {
                vec!["text".to_string(), "image".to_string()]
            } else {
                vec!["text".to_string()]
            },
            context_window: input.context_window,
            max_tokens: input.max_tokens,
        }],
        api_key: input.api_key.clone().unwrap_or_default(),
        replace_models: true,
        ..Default::default()
    }
}

async fn save_custom_provider(input: &CustomProviderInput) -> Result<(), String> {
    #[cfg(not(test))]
    {
        let client = RunClient::new(&grpc_addr());
        if client.get_agent_info().await.is_ok() {
            return client
                .upsert_provider(provider_rpc(input))
                .await
                .map(|_| ())
                .map_err(|error| format!("Future Agent did not save the provider: {error}"));
        }
    }

    future_agent::config::providers::upsert_provider(&provider_spec(input))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env::EnvGuard;
    use serde_json::Value;
    use std::collections::VecDeque;

    struct FakePrompter {
        answers: VecDeque<String>,
        secret_reads: usize,
    }

    impl FakePrompter {
        fn new(answers: &[&str]) -> Self {
            Self {
                answers: answers.iter().map(|value| value.to_string()).collect(),
                secret_reads: 0,
            }
        }

        fn next(&mut self) -> Result<String, String> {
            self.answers
                .pop_front()
                .ok_or_else(|| "test input exhausted".to_string())
        }
    }

    impl Prompter for FakePrompter {
        fn read_line(&mut self) -> Result<String, String> {
            self.next()
        }

        fn read_secret(&mut self) -> Result<String, String> {
            self.secret_reads += 1;
            self.next()
        }
    }

    fn output_text(out: &std::sync::Arc<std::sync::Mutex<Vec<u8>>>) -> String {
        String::from_utf8(out.lock().unwrap().clone()).unwrap()
    }

    #[tokio::test]
    async fn existing_future_token_can_keep_current_login() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let path = crate::constants::auth_file();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, r#"{"future":{"type":"api_key","key":"existing"}}"#)
            .await
            .unwrap();

        let mut prompt = FakePrompter::new(&["1", "n"]);
        let (out, captured) = Output::memory();
        configure_with(&mut prompt, &out).await.unwrap();
        let stdout = output_text(&captured.out);
        assert!(stdout.contains("Log in again?"), "{stdout}");
        assert!(stdout.contains("no changes were made"), "{stdout}");
    }

    #[tokio::test]
    async fn custom_provider_writes_models_and_secret_files() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        // Existing providers — including legacy camelCase fields and an
        // unrecognized non-object entry — must survive the upsert untouched.
        let auth_path = future_agent::config::providers::auth_json_path();
        tokio::fs::create_dir_all(auth_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &auth_path,
            r#"{
                "future": {"type": "api_key", "key": "future-key"},
                "azure": {"type": "api_key", "key": "azure-key", "baseUrl": "https://azure.example/openai/v1"},
                "weird-provider": "not-an-object"
            }"#,
        )
        .await
        .unwrap();
        let mut prompt = FakePrompter::new(&[
            "2",
            "acme",
            "Acme AI",
            "2",
            "https://api.acme.test/v1",
            "secret-key",
            "reasoner-v1",
            "Reasoner",
            "200000",
            "32000",
            "yes",
        ]);
        let (out, captured) = Output::memory();

        configure_with(&mut prompt, &out).await.unwrap();
        assert_eq!(prompt.secret_reads, 1);

        let models: Value = serde_json::from_str(
            &tokio::fs::read_to_string(future_agent::config::providers::models_json_path())
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(models["providers"]["acme"]["name"], "Acme AI");
        assert_eq!(models["providers"]["acme"]["api"], "openai-responses");
        assert_eq!(
            models["providers"]["acme"]["baseUrl"],
            "https://api.acme.test/v1"
        );
        assert_eq!(
            models["providers"]["acme"]["models"][0]["modalities"],
            serde_json::json!(["text", "image"])
        );
        assert_eq!(
            models["providers"]["acme"]["models"][0]["contextWindow"],
            200000
        );

        let auth: Value = serde_json::from_str(
            &tokio::fs::read_to_string(future_agent::config::providers::auth_json_path())
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(auth["acme"]["type"], "api_key");
        assert_eq!(auth["acme"]["key"], "secret-key");
        assert_eq!(auth["future"]["key"], "future-key");
        assert_eq!(auth["azure"]["key"], "azure-key");
        assert_eq!(auth["azure"]["baseUrl"], "https://azure.example/openai/v1");
        assert_eq!(auth["weird-provider"], "not-an-object");

        let stdout = output_text(&captured.out);
        assert!(!stdout.contains("secret-key"), "secret leaked: {stdout}");
        assert!(stdout.contains("acme/reasoner-v1"), "{stdout}");
    }

    #[test]
    fn defaults_and_validation_reprompt() {
        let mut prompt = FakePrompter::new(&[
            "future", // reserved provider id
            "",       // default custom
            "",       // display name defaults to id
            "9",      // invalid protocol
            "",       // default protocol
            "ftp://bad",
            "http://127.0.0.1:11434/v1",
            "", // keyless
            "llama3.2",
            "", // display name defaults
            "0",
            "", // default context
            "999999",
            "", // default max tokens
            "n",
        ]);
        let (out, captured) = Output::memory();
        let input = collect_custom_provider(&mut prompt, &out).unwrap();
        assert_eq!(input.id, "custom");
        assert_eq!(input.name, "custom");
        assert_eq!(input.api, "openai-completions");
        assert_eq!(input.api_key, None);
        assert_eq!(input.model_name, "llama3.2");
        assert_eq!(input.context_window, DEFAULT_CONTEXT_WINDOW);
        assert_eq!(input.max_tokens, DEFAULT_MAX_TOKENS);
        assert!(!input.supports_images);
        let stderr = output_text(&captured.err);
        assert!(stderr.contains("must not be a built-in"), "{stderr}");
        assert!(stderr.contains("Please enter 1, 2, or 3"), "{stderr}");
        assert!(stderr.contains("valid http:// or https://"), "{stderr}");
        assert!(stderr.contains("positive whole number"), "{stderr}");
        assert!(stderr.contains("cannot exceed"), "{stderr}");
    }

    #[test]
    fn provider_choice_reprompts() {
        let mut prompt = FakePrompter::new(&["wat", "custom"]);
        let (out, captured) = Output::memory();
        assert_eq!(
            ask_provider_choice(&mut prompt, &out).unwrap(),
            ProviderChoice::Custom
        );
        assert!(output_text(&captured.err).contains("Please enter 1"));
    }
}
