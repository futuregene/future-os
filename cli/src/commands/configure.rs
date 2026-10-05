//! Interactive model-provider setup for `future config`.
//!
//! Two setup paths are intentionally small and opinionated:
//! - FutureOS reuses the existing device-code login flow, asking before it
//!   replaces a token already stored in `auth.json`.
//! - Custom creates or updates one provider across the agent's `models.json`
//!   and `auth.json` documents. Existing models can be kept, edited or deleted
//!   and new ones added, each with its own token limits and per-1M-token prices,
//!   so configuring a provider no longer replaces its other models.
//!
//! When an agent is running, custom-provider writes go through its RPC so the
//! live registry changes atomically with the files. Otherwise the CLI uses the
//! same validated, atomic file writer exported by the agent crate.

use crate::commands::auth;
use crate::output::Output;
#[cfg(not(test))]
use crate::rpc::{grpc_addr, RunClient};
use future_agent::config::providers::{ModelCostSpec, ProviderModelSpec, ProviderUpsertSpec};
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

#[derive(Debug, Clone, PartialEq)]
struct CustomProviderInput {
    id: String,
    name: String,
    api: String,
    base_url: String,
    api_key: Option<String>,
    models: Vec<ModelInput>,
}

/// One model entry collected by the wizard. Mirrors the fields the agent's
/// provider writer persists, so a kept/edited model round-trips through
/// `models.json` without losing its numbers or prices.
#[derive(Debug, Clone, PartialEq)]
struct ModelInput {
    id: String,
    name: String,
    supports_images: bool,
    /// Whether thinking controls are sent. Carried through for kept models
    /// (the wizard does not prompt for it, and the loader defaults to true)
    /// so editing a provider never silently flips it.
    reasoning: bool,
    context_window: i32,
    max_tokens: i32,
    cost: ModelCostInput,
}

impl ModelInput {
    /// Repair a persisted model whose limits are missing or inconsistent so
    /// re-writing it always passes the agent's validator (both limits must be
    /// positive and the output limit must not exceed the window).
    fn normalize(&mut self) {
        if self.context_window <= 0 {
            self.context_window = DEFAULT_CONTEXT_WINDOW;
        }
        if self.max_tokens <= 0 {
            self.max_tokens = DEFAULT_MAX_TOKENS;
        }
        if self.max_tokens > self.context_window {
            self.max_tokens = self.context_window;
        }
    }
}

/// Per-1M-token prices in the same currency the UI displays (CNY). All-zero
/// means "unpriced": the entry omits `cost` and the agent falls back to a
/// built-in catalog price for a matching model id.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct ModelCostInput {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
}

impl ModelCostInput {
    fn is_unset(&self) -> bool {
        self.input == 0.0 && self.output == 0.0 && self.cache_read == 0.0 && self.cache_write == 0.0
    }

    /// Read the `cost` object of a persisted model, ignoring non-finite or
    /// negative prices rather than failing the whole edit.
    fn from_value(value: Option<&serde_json::Value>) -> Self {
        let field = |key: &str| {
            value
                .and_then(|cost| cost.get(key))
                .and_then(serde_json::Value::as_f64)
                .filter(|price| price.is_finite() && *price >= 0.0)
                .unwrap_or(0.0)
        };
        Self {
            input: field("input"),
            output: field("output"),
            cache_read: field("cache_read"),
            cache_write: field("cache_write"),
        }
    }
}

/// The provider already on disk, used to seed defaults and to keep existing
/// models when the user edits a provider instead of replacing it.
#[derive(Debug, Clone, Default, PartialEq)]
struct ExistingProvider {
    name: Option<String>,
    api: Option<String>,
    base_url: Option<String>,
    models: Vec<ModelInput>,
}

impl ExistingProvider {
    fn is_present(&self) -> bool {
        self.name.is_some()
            || self.api.is_some()
            || self.base_url.is_some()
            || !self.models.is_empty()
    }
}

/// What to do with a model that already exists on the provider being edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelDisposition {
    Keep,
    Edit,
    Delete,
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
    save_custom_provider(&input).await?;

    let models = input
        .models
        .iter()
        .map(|model| format!("{}/{}", input.id, model.id))
        .collect::<Vec<_>>()
        .join(", ");
    out.log(&format!(
        "Configured custom provider '{}' with {} model(s): {}.",
        input.id,
        input.models.len(),
        models
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

    let existing = load_existing_provider(&id)?;
    if existing.is_present() {
        out.log(&format!(
            "Provider '{id}' already exists; its current values are the defaults."
        ));
    }

    let name_default = existing.name.clone().unwrap_or_else(|| id.clone());
    let name = {
        let value = prompt(
            prompter,
            out,
            &format!("Provider display name [{name_default}]: "),
            false,
        )?;
        if value.is_empty() {
            name_default
        } else {
            value
        }
    };

    out.log("API protocol:");
    out.log("  1) OpenAI Chat Completions");
    out.log("  2) OpenAI Responses");
    out.log("  3) Anthropic Messages");
    // A recognized current protocol is offered by number; anything else is
    // offered verbatim so editing a provider never silently rewrites it.
    let (api_default_label, api_default_value) = match existing.api.as_deref() {
        Some("openai-completions") => ("1".to_string(), "openai-completions".to_string()),
        Some("openai-responses") => ("2".to_string(), "openai-responses".to_string()),
        Some("anthropic") => ("3".to_string(), "anthropic".to_string()),
        Some(current) if !current.is_empty() => (current.to_string(), current.to_string()),
        _ => ("1".to_string(), "openai-completions".to_string()),
    };
    let api = loop {
        let value = prompt(
            prompter,
            out,
            &format!("Select protocol [{api_default_label}]: "),
            false,
        )?;
        let value = if value.is_empty() {
            api_default_value.clone()
        } else {
            value.to_ascii_lowercase()
        };
        match value.as_str() {
            "1" | "openai" | "openai-completions" => break "openai-completions".to_string(),
            "2" | "openai-responses" | "responses" => break "openai-responses".to_string(),
            "3" | "anthropic" => break "anthropic".to_string(),
            _ if value == api_default_value => break api_default_value.clone(),
            _ => out.log_err("Please enter 1, 2, or 3."),
        }
    };

    let base_default = existing.base_url.clone().unwrap_or_default();
    let base_url = loop {
        let message = if base_default.is_empty() {
            "Base URL: ".to_string()
        } else {
            format!("Base URL [{base_default}]: ")
        };
        let value = prompt(prompter, out, &message, false)?;
        let value = if value.is_empty() {
            base_default.clone()
        } else {
            value
        };
        if value.is_empty() {
            out.log_err("Base URL is required.");
            continue;
        }
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

    let mut models: Vec<ModelInput> = Vec::new();
    if !existing.models.is_empty() {
        out.log("Existing models:");
        for (index, model) in existing.models.iter().enumerate() {
            out.log(&format!("  {}) {}", index + 1, describe_model(model)));
        }
        for model in &existing.models {
            match ask_model_disposition(prompter, out, model)? {
                ModelDisposition::Keep => models.push(model.clone()),
                ModelDisposition::Delete => {}
                ModelDisposition::Edit => {
                    let mut taken: Vec<String> =
                        models.iter().map(|kept| kept.id.clone()).collect();
                    taken.extend(
                        existing
                            .models
                            .iter()
                            .filter(|sibling| sibling.id != model.id)
                            .map(|sibling| sibling.id.clone()),
                    );
                    models.push(collect_model(prompter, out, Some(model), &taken)?);
                }
            }
        }
    }

    loop {
        let (message, add_by_default) = if models.is_empty() {
            ("Add a model? [Y/n]: ", true)
        } else {
            ("Add another model? [y/N]: ", false)
        };
        if !ask_yes_no(prompter, out, message, add_by_default)? {
            break;
        }
        let taken = models
            .iter()
            .map(|model| model.id.clone())
            .collect::<Vec<_>>();
        let model = collect_model(prompter, out, None, &taken)?;
        models.push(model);
    }
    if models.is_empty() {
        return Err("At least one model is required.".to_string());
    }

    let input = CustomProviderInput {
        id,
        name,
        api,
        base_url,
        api_key,
        models,
    };
    // Keep the CLI's local validation exactly aligned with the authoritative
    // agent writer before attempting either an RPC or an offline write.
    future_agent::config::providers::validate_provider_upsert(&provider_spec(&input))?;
    Ok(input)
}

fn ask_model_disposition(
    prompter: &mut dyn Prompter,
    out: &Output,
    model: &ModelInput,
) -> Result<ModelDisposition, String> {
    loop {
        let value = prompt(
            prompter,
            out,
            &format!("  {} — keep, edit or delete? [k/e/d] [k]: ", model.id),
            false,
        )?;
        match value.to_ascii_lowercase().as_str() {
            "" | "k" | "keep" => return Ok(ModelDisposition::Keep),
            "e" | "edit" => return Ok(ModelDisposition::Edit),
            "d" | "delete" => return Ok(ModelDisposition::Delete),
            _ => out.log_err("Please enter k to keep, e to edit, or d to delete."),
        }
    }
}

/// Collect one model's fields. `current` seeds every prompt with the existing
/// model's value (used when editing); `taken_ids` are the sibling model ids a
/// new/renamed id must not shadow.
fn collect_model(
    prompter: &mut dyn Prompter,
    out: &Output,
    current: Option<&ModelInput>,
    taken_ids: &[String],
) -> Result<ModelInput, String> {
    let id = loop {
        let message = match current {
            Some(model) => format!("Model ID (sent to the API) [{}]: ", model.id),
            None => "Model ID (sent to the API): ".to_string(),
        };
        let value = prompt(prompter, out, &message, false)?;
        let value = match current {
            Some(model) if value.is_empty() => model.id.clone(),
            _ => value,
        };
        if value.is_empty()
            || value.len() > 256
            || !value.is_ascii()
            || value.bytes().any(|byte| byte.is_ascii_control())
        {
            out.log_err("Model ID is required and must be at most 256 ASCII characters.");
            continue;
        }
        if taken_ids.iter().any(|taken| taken == &value) {
            out.log_err("Another model in this provider already uses that ID.");
            continue;
        }
        break value;
    };

    let name_default = match current {
        Some(model) => model.name.clone(),
        None => id.clone(),
    };
    let model_name = {
        let value = prompt(
            prompter,
            out,
            &format!("Model display name [{name_default}]: "),
            false,
        )?;
        if value.is_empty() {
            name_default
        } else {
            value
        }
    };

    let context_default = current
        .map(|model| model.context_window)
        .unwrap_or(DEFAULT_CONTEXT_WINDOW);
    let context_window = ask_positive_i32(
        prompter,
        out,
        &format!("Context window [{context_default}]: "),
        context_default,
    )?;
    let max_default = current
        .map(|model| model.max_tokens)
        .unwrap_or(DEFAULT_MAX_TOKENS);
    let max_tokens = loop {
        let value = ask_positive_i32(
            prompter,
            out,
            &format!("Maximum output tokens [{max_default}]: "),
            max_default,
        )?;
        if value <= context_window {
            break value;
        }
        out.log_err("Maximum output tokens cannot exceed the context window.");
    };
    let images_default = current.is_some_and(|model| model.supports_images);
    let supports_images = ask_yes_no(
        prompter,
        out,
        if images_default {
            "Does this model accept image input? [Y/n]: "
        } else {
            "Does this model accept image input? [y/N]: "
        },
        images_default,
    )?;

    out.log("Prices per 1M tokens in CNY (0 inherits a built-in price, or stays unpriced).");
    let cost_default = current.map(|model| model.cost).unwrap_or_default();
    let cost = ModelCostInput {
        input: ask_non_negative_f64(
            prompter,
            out,
            &format!("Input price [{}]: ", cost_default.input),
            cost_default.input,
        )?,
        output: ask_non_negative_f64(
            prompter,
            out,
            &format!("Output price [{}]: ", cost_default.output),
            cost_default.output,
        )?,
        cache_read: ask_non_negative_f64(
            prompter,
            out,
            &format!("Cache read price [{}]: ", cost_default.cache_read),
            cost_default.cache_read,
        )?,
        cache_write: ask_non_negative_f64(
            prompter,
            out,
            &format!("Cache write price [{}]: ", cost_default.cache_write),
            cost_default.cache_write,
        )?,
    };

    Ok(ModelInput {
        id,
        name: model_name,
        supports_images,
        reasoning: current.map(|model| model.reasoning).unwrap_or(true),
        context_window,
        max_tokens,
        cost,
    })
}

fn ask_non_negative_f64(
    prompter: &mut dyn Prompter,
    out: &Output,
    message: &str,
    default: f64,
) -> Result<f64, String> {
    loop {
        let value = prompt(prompter, out, message, false)?;
        if value.is_empty() {
            return Ok(default);
        }
        if let Ok(number) = value.parse::<f64>() {
            if number.is_finite() && number >= 0.0 {
                return Ok(number);
            }
        }
        out.log_err("Please enter a non-negative number.");
    }
}

fn describe_model(model: &ModelInput) -> String {
    let modalities = if model.supports_images {
        "text+image"
    } else {
        "text"
    };
    let prices = if model.cost.is_unset() {
        "unpriced".to_string()
    } else {
        format!(
            "prices {}/{}/{}/{}",
            model.cost.input, model.cost.output, model.cost.cache_read, model.cost.cache_write
        )
    };
    format!(
        "{} — context {}, max {}, {}, {}",
        model.id, model.context_window, model.max_tokens, modalities, prices
    )
}

/// Read the provider already persisted under `id` from `models.json` so the
/// wizard can offer its values as defaults and keep its models. A missing file
/// or provider yields the empty default; an unreadable file is a real error,
/// because writing over it would clobber state the wizard cannot see.
fn load_existing_provider(id: &str) -> Result<ExistingProvider, String> {
    let (models_document, _auth_document) =
        future_agent::config::providers::read_provider_documents()?;
    let entry = models_document
        .get("providers")
        .and_then(serde_json::Value::as_object)
        .and_then(|providers| providers.get(id));
    let Some(entry) = entry.and_then(serde_json::Value::as_object) else {
        return Ok(ExistingProvider::default());
    };
    Ok(ExistingProvider {
        name: entry
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        api: entry
            .get("api")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        base_url: entry
            .get("baseUrl")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        models: entry
            .get("models")
            .and_then(serde_json::Value::as_array)
            .map(|entries| entries.iter().filter_map(parse_existing_model).collect())
            .unwrap_or_default(),
    })
}

fn parse_existing_model(entry: &serde_json::Value) -> Option<ModelInput> {
    let object = entry.as_object()?;
    let id = object.get("id")?.as_str()?.to_string();
    if id.is_empty() {
        return None;
    }
    let name = object
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(id.as_str())
        .to_string();
    let supports_images = object
        .get("modalities")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|modalities| {
            modalities
                .iter()
                .any(|modality| modality.as_str() == Some("image"))
        });
    let limit = object.get("limit");
    let context_window = object
        .get("contextWindow")
        .and_then(serde_json::Value::as_i64)
        .or_else(|| {
            limit
                .and_then(|limit| limit.get("context"))
                .and_then(serde_json::Value::as_i64)
        })
        .unwrap_or(0);
    let max_tokens = object
        .get("maxTokens")
        .and_then(serde_json::Value::as_i64)
        .or_else(|| {
            limit
                .and_then(|limit| limit.get("output"))
                .and_then(serde_json::Value::as_i64)
        })
        .unwrap_or(0);
    let mut model = ModelInput {
        id,
        name,
        supports_images,
        reasoning: object
            .get("reasoning")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true),
        context_window: i32::try_from(context_window).unwrap_or(0),
        max_tokens: i32::try_from(max_tokens).unwrap_or(0),
        cost: ModelCostInput::from_value(object.get("cost")),
    };
    model.normalize();
    Some(model)
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
        models: input
            .models
            .iter()
            .map(|model| ProviderModelSpec {
                id: model.id.clone(),
                name: model.name.clone(),
                modalities: if model.supports_images {
                    vec!["text".to_string(), "image".to_string()]
                } else {
                    vec!["text".to_string()]
                },
                context_window: model.context_window,
                max_tokens: model.max_tokens,
                reasoning: model.reasoning,
                cost: ModelCostSpec {
                    input: model.cost.input,
                    output: model.cost.output,
                    cache_read: model.cost.cache_read,
                    cache_write: model.cost.cache_write,
                },
            })
            .collect(),
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
        models: input
            .models
            .iter()
            .map(|model| ProviderModel {
                id: model.id.clone(),
                name: model.name.clone(),
                modalities: if model.supports_images {
                    vec!["text".to_string(), "image".to_string()]
                } else {
                    vec!["text".to_string()]
                },
                context_window: model.context_window,
                max_tokens: model.max_tokens,
                reasoning: Some(model.reasoning),
                cost_input: model.cost.input,
                cost_output: model.cost.output,
                cost_cache_read: model.cost.cache_read,
                cost_cache_write: model.cost.cache_write,
            })
            .collect(),
        api_key: input.api_key.clone().unwrap_or_default(),
        replace_models: true,
        ..Default::default()
    }
}

async fn save_custom_provider(input: &CustomProviderInput) -> Result<(), String> {
    #[cfg(not(test))]
    {
        let client = RunClient::new(&grpc_addr());
        if client.probe_agent().await.is_ok() {
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

    async fn write_models_json(document: Value) {
        let path = future_agent::config::providers::models_json_path();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, serde_json::to_string_pretty(&document).unwrap())
            .await
            .unwrap();
    }

    async fn read_models_json() -> Value {
        let path = future_agent::config::providers::models_json_path();
        serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap()
    }

    /// An existing provider with two models, one of them priced.
    fn seeded_provider() -> Value {
        serde_json::json!({
            "providers": {
                "acme": {
                    "name": "Acme",
                    "api": "openai-completions",
                    "baseUrl": "https://api.acme.test/v1",
                    "models": [
                        {
                            "id": "reasoner-v1",
                            "name": "Reasoner",
                            "modalities": ["text", "image"],
                            "contextWindow": 200000,
                            "maxTokens": 32000,
                            "reasoning": true,
                            "cost": { "input": 1.0, "output": 4.0, "cache_read": 0.02, "cache_write": 0.0 }
                        },
                        {
                            "id": "fast-v1",
                            "name": "Fast",
                            "modalities": ["text"],
                            "contextWindow": 128000,
                            "maxTokens": 16384,
                            "reasoning": false
                        }
                    ]
                }
            }
        })
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
            "", // add a model
            "reasoner-v1",
            "Reasoner",
            "200000",
            "32000",
            "yes",
            "1",    // input price
            "4",    // output price
            "0.02", // cache read price
            "",     // cache write price stays unset
            "",     // no more models
        ]);
        let (out, captured) = Output::memory();

        let seeded = tokio::fs::read_to_string(&auth_path).await.unwrap();
        assert!(
            seeded.contains("future-key"),
            "seed replaced before configure: len={}, keys={:?}, path_stable={}",
            seeded.len(),
            serde_json::from_str::<Value>(&seeded)
                .ok()
                .and_then(|v| v.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>())),
            future_agent::config::providers::auth_json_path() == auth_path
        );
        configure_with(&mut prompt, &out).await.unwrap();
        assert_eq!(
            future_agent::config::providers::auth_json_path(),
            auth_path,
            "HOME changed despite env lock"
        );
        assert_eq!(prompt.secret_reads, 1);

        let models = read_models_json().await;
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
        // The prices the wizard collected reach `models.json`.
        assert_eq!(
            models["providers"]["acme"]["models"][0]["cost"],
            serde_json::json!({"input": 1.0, "output": 4.0, "cache_read": 0.02, "cache_write": 0.0})
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

    #[tokio::test]
    async fn defaults_and_validation_reprompt() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let mut prompt = FakePrompter::new(&[
            "future", // reserved provider id
            "",       // default custom
            "",       // display name defaults to id
            "9",      // invalid protocol
            "",       // default protocol
            "ftp://bad",
            "http://127.0.0.1:11434/v1",
            "", // keyless
            "", // add a model (default yes)
            "llama3.2",
            "",       // display name defaults
            "0",      // invalid context
            "",       // default context
            "999999", // exceeds the context
            "",       // default max tokens
            "n",      // images
            "",       // input price
            "",       // output price
            "",       // cache read price
            "",       // cache write price
            "",       // no more models
        ]);
        let (out, captured) = Output::memory();
        let input = collect_custom_provider(&mut prompt, &out).unwrap();
        assert_eq!(input.id, "custom");
        assert_eq!(input.name, "custom");
        assert_eq!(input.api, "openai-completions");
        assert_eq!(input.api_key, None);
        assert_eq!(input.models.len(), 1);
        assert_eq!(input.models[0].id, "llama3.2");
        assert_eq!(input.models[0].name, "llama3.2");
        assert_eq!(input.models[0].context_window, DEFAULT_CONTEXT_WINDOW);
        assert_eq!(input.models[0].max_tokens, DEFAULT_MAX_TOKENS);
        assert!(!input.models[0].supports_images);
        assert!(input.models[0].cost.is_unset());
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

    /// `ask_yes_no` accepts only the documented spellings (plus the blank
    /// default) and re-prompts on anything else — it must never guess, because
    /// the answer decides whether an existing token is replaced.
    #[test]
    fn ask_yes_no_reprompts_and_honours_the_default() {
        let (out, captured) = Output::memory();
        // Unrecognised, then the explicit spellings.
        let mut prompt = FakePrompter::new(&["maybe", "YES"]);
        assert!(ask_yes_no(&mut prompt, &out, "? ", false).unwrap());
        assert!(output_text(&captured.err).contains("Please answer yes or no."));

        let mut prompt = FakePrompter::new(&["nope", "n"]);
        assert!(!ask_yes_no(&mut prompt, &out, "? ", true).unwrap());

        // Blank takes the caller's default, either way round.
        let mut prompt = FakePrompter::new(&[""]);
        assert!(ask_yes_no(&mut prompt, &out, "? ", true).unwrap());
        let mut prompt = FakePrompter::new(&[""]);
        assert!(!ask_yes_no(&mut prompt, &out, "? ", false).unwrap());
    }

    /// Every prompt is fallible and every one of them propagates: a prompter
    /// that fails at *any* stage ends the flow with that error rather than
    /// continuing with a default (which would write a provider from
    /// half-collected input). Truncating the answer list at each length
    /// therefore has to fail at each stage.
    #[tokio::test]
    async fn a_prompter_failure_at_any_stage_propagates() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let choice = ["2"];
        let custom = [
            "acme",
            "Acme AI",
            "2",
            "https://api.acme.test/v1",
            "secret-key",
            "", // add a model
            "reasoner-v1",
            "Reasoner",
            "200000",
            "32000",
            "yes",
            "1",
            "4",
            "0.02",
            "0",
        ];

        // The custom-collection stages (one prompt each).
        for taken in 0..custom.len() {
            let (out, _captured) = Output::memory();
            let mut prompt = FakePrompter::new(&custom[..taken]);
            let err = collect_custom_provider(&mut prompt, &out)
                .expect_err("a failed prompt must not yield an input");
            assert_eq!(err, "test input exhausted", "taken={taken}");
        }

        // …and through the full `configure_with` entry, including the
        // provider-choice prompt that precedes them.
        for taken in 0..custom.len() {
            let (out, _captured) = Output::memory();
            let mut all: Vec<&str> = choice.to_vec();
            all.extend_from_slice(&custom[..taken]);
            let mut prompt = FakePrompter::new(&all);
            let err = configure_with(&mut prompt, &out)
                .await
                .expect_err("a failed prompt must abort the wizard");
            assert_eq!(err, "test input exhausted", "taken={taken}");
        }
    }

    /// The two field validators that have a documented limit: an API key or a
    /// model ID that is not ASCII, carries a control character, or is too long
    /// is refused and the prompt repeats — the boundary is inclusive on the
    /// limit (16 384 bytes / 256 characters are accepted, the next byte is
    /// not). Also covered: the blank key is allowed (a keyless local endpoint)
    /// while a blank model ID is not.
    #[tokio::test]
    async fn api_key_and_model_id_limits_reprompt_at_the_boundary() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let (out, captured) = Output::memory();
        let long_key = "k".repeat(16_385);
        let long_model = "m".repeat(257);
        let mut prompt = FakePrompter::new(&[
            "acme", // id
            "",     // name
            "1",    // protocol
            "https://api.acme.test/v1",
            "café",          // non-ASCII key → refused
            "with\u{7}bell", // control character → refused
            &long_key,       // over the byte limit → refused
            "ok-key",        // accepted
            "",              // add a model
            "",              // blank model id → refused
            "modèle",        // non-ASCII model → refused
            &long_model,     // over the length limit → refused
            "good-model",    // accepted
            "",              // model display name
            "",              // context window
            "",              // max tokens
            "n",             // images
            "",              // input price
            "",              // output price
            "",              // cache read price
            "",              // cache write price
            "",              // no more models
        ]);
        let input = collect_custom_provider(&mut prompt, &out).expect("accepted");
        assert_eq!(input.api_key.as_deref(), Some("ok-key"));
        assert_eq!(input.models.len(), 1);
        assert_eq!(input.models[0].id, "good-model");
        assert_eq!(
            input.models[0].name, "good-model",
            "the name defaults to the id"
        );
        let stderr = output_text(&captured.err);
        assert_eq!(
            stderr
                .matches("API key must be ASCII, contain no control characters, and be at most 16384 bytes.")
                .count(),
            3,
            "{stderr}"
        );
        assert_eq!(
            stderr
                .matches("Model ID is required and must be at most 256 ASCII characters.")
                .count(),
            3,
            "{stderr}"
        );
    }

    /// A prompter that fails while the FutureOS path is asking its question
    /// propagates that failure instead of treating it as "no" — a login must
    /// never be started, or skipped, on a prompt that never got an answer.
    #[tokio::test]
    async fn a_prompter_failure_at_the_relogin_question_propagates() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let _env = EnvGuard::set(&[
            ("HOME", dir.path().as_os_str().to_owned()),
            ("USERPROFILE", dir.path().as_os_str().to_owned()),
        ]);
        let path = crate::constants::auth_file();
        tokio::fs::create_dir_all(path.parent().expect("parent"))
            .await
            .expect("mkdir");
        tokio::fs::write(
            &path,
            r#"{"future":{"type":"api_key","key":"existing","base_url":"http://127.0.0.1:1"}}"#,
        )
        .await
        .expect("write");

        // No answers at all: the question itself cannot be read.
        let mut prompt = FakePrompter::new(&[]);
        let (out, _captured) = Output::memory();
        let err = configure_futureos(&mut prompt, &out)
            .await
            .expect_err("an unreadable answer is not a default");
        assert_eq!(err, "test input exhausted");
        // …and the stored credential was not touched.
        let after: Value =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.expect("read"))
                .expect("json");
        assert_eq!(after["future"]["key"], "existing");
    }

    /// A `collect_custom_provider` answer that fails the agent's own validator
    /// validators cannot silently disagree: the CLI accepts a 64-character id
    /// and the authoritative writer is what finally judges the whole spec.
    #[tokio::test]
    async fn the_agent_validator_is_the_final_authority() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let (out, _captured) = Output::memory();
        let mut prompt = FakePrompter::new(&[
            &"a".repeat(64),
            "",
            "1",
            "https://api.acme.test/v1",
            "",
            "", // add a model
            "m",
            "",
            "",
            "",
            "n",
            "",
            "",
            "",
            "",
            "", // no more models
        ]);
        let input = collect_custom_provider(&mut prompt, &out).expect("a 64-char id is valid");
        assert_eq!(input.id.len(), 64);
        // A 65-character id never reaches the validator — the prompt loop
        // rejects it first, which is the boundary the two share.
        let mut prompt = FakePrompter::new(&[
            &"b".repeat(65),
            "custom",
            "",
            "1",
            "https://api.acme.test/v1",
            "",
            "", // add a model
            "m",
            "",
            "",
            "",
            "n",
            "",
            "",
            "",
            "",
            "", // no more models
        ]);
        let input = collect_custom_provider(&mut prompt, &out).expect("retries with a valid id");
        assert_eq!(input.id, "custom");
    }

    /// Prices are optional: leaving all four blank writes no `cost` object, so
    /// the entry keeps inheriting a built-in catalog price (or stays unpriced).
    #[tokio::test]
    async fn blank_prices_leave_a_model_unpriced() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let mut prompt = FakePrompter::new(&[
            "2",
            "acme",
            "",
            "1",
            "https://api.acme.test/v1",
            "",
            "", // add a model
            "m",
            "",
            "",
            "",
            "n",
            "",
            "",
            "",
            "", // all four prices blank
            "", // no more models
        ]);
        let (out, _captured) = Output::memory();
        configure_with(&mut prompt, &out).await.unwrap();
        let models = read_models_json().await;
        assert!(
            models["providers"]["acme"]["models"][0]
                .get("cost")
                .is_none(),
            "no prices means no cost entry: {models}"
        );
    }

    /// Editing an existing provider must not replace the models it already has:
    /// kept models survive (prices included) and a new one is appended.
    #[tokio::test]
    async fn editing_a_provider_keeps_existing_models() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        write_models_json(seeded_provider()).await;

        let mut prompt = FakePrompter::new(&[
            "2", "acme", "",  // name keeps "Acme"
            "",  // protocol keeps openai-completions
            "",  // base URL keeps the existing one
            "",  // key blank leaves the stored credential alone
            "",  // keep reasoner-v1
            "",  // keep fast-v1
            "y", // add another model
            "new-v1", "", "", "", "n", "", "", "", "", "", // no more models
        ]);
        let (out, captured) = Output::memory();
        configure_with(&mut prompt, &out).await.unwrap();

        let models = read_models_json().await;
        let entries = models["providers"]["acme"]["models"].as_array().unwrap();
        assert_eq!(entries.len(), 3, "kept two and added one: {models}");
        assert_eq!(entries[0]["id"], "reasoner-v1");
        assert_eq!(entries[1]["id"], "fast-v1");
        assert_eq!(entries[2]["id"], "new-v1");
        // The kept model's prices survive, and the unpriced one stays unpriced
        // rather than gaining a zeroed `cost` object.
        assert_eq!(entries[0]["cost"]["input"], 1.0);
        assert_eq!(entries[0]["cost"]["cache_read"], 0.02);
        assert!(entries[1].get("cost").is_none(), "{models}");
        // A kept model keeps its thinking setting instead of being reset to
        // the wizard's default.
        assert_eq!(entries[1]["reasoning"], false, "{models}");

        let stdout = output_text(&captured.out);
        assert!(stdout.contains("already exists"), "{stdout}");
        assert!(stdout.contains("reasoner-v1"), "{stdout}");
    }

    /// A client that does not know model-level `compat` (the CLI wizard) must
    /// not strip it when it rewrites a provider: the agent merges the fields
    /// the RPC cannot carry onto the entry that already had that model id.
    #[tokio::test]
    async fn editing_a_provider_keeps_model_level_compat() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        write_models_json(serde_json::json!({
            "providers": {
                "omlx": {
                    "name": "omlx",
                    "api": "openai-completions",
                    "baseUrl": "http://127.0.0.1:8000/v1",
                    "models": [{
                        "id": "Qwen3.8-27B-oQ4e-mtp",
                        "name": "Qwen3.8-27B-oQ4e-mtp",
                        "modalities": ["text", "image"],
                        "contextWindow": 204800,
                        "maxTokens": 32768,
                        "reasoning": true,
                        "compat": { "thinkingFormat": "qwen-chat-template" }
                    }]
                }
            }
        }))
        .await;

        let mut prompt = FakePrompter::new(&[
            "2", "omlx", // custom provider, edit the existing one
            "",     // name keeps
            "",     // protocol keeps
            "",     // base URL keeps
            "",     // key blank leaves the stored credential alone
            "",     // keep the model
            "n",    // add no more models
        ]);
        let (out, _captured) = Output::memory();
        configure_with(&mut prompt, &out).await.unwrap();

        let models = read_models_json().await;
        let entry = &models["providers"]["omlx"]["models"][0];
        assert_eq!(entry["id"], "Qwen3.8-27B-oQ4e-mtp");
        assert_eq!(
            entry["compat"],
            serde_json::json!({ "thinkingFormat": "qwen-chat-template" }),
            "the wizard must not strip model-level compat: {models}"
        );
    }

    /// Editing one model only changes that model: its siblings are untouched
    /// and the edited one keeps the values that were not re-entered.
    #[tokio::test]
    async fn editing_a_model_updates_its_price() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        write_models_json(seeded_provider()).await;

        let mut prompt = FakePrompter::new(&[
            "2", "acme", "", "", "", "", "e",   // edit reasoner-v1
            "",    // model id keeps
            "",    // name keeps
            "",    // context keeps
            "",    // max tokens keeps
            "",    // images keeps
            "2.5", // input price changes
            "",    // output keeps (4)
            "",    // cache read keeps (0.02)
            "",    // cache write keeps (0)
            "",    // keep fast-v1
            "",    // no more models
        ]);
        let (out, _captured) = Output::memory();
        configure_with(&mut prompt, &out).await.unwrap();

        let models = read_models_json().await;
        let entries = models["providers"]["acme"]["models"].as_array().unwrap();
        assert_eq!(entries.len(), 2, "{models}");
        assert_eq!(entries[0]["id"], "reasoner-v1");
        assert_eq!(entries[0]["contextWindow"], 200000, "{models}");
        assert_eq!(entries[0]["cost"]["input"], 2.5);
        assert_eq!(entries[0]["cost"]["output"], 4.0);
        assert_eq!(entries[0]["cost"]["cache_read"], 0.02);
    }

    #[tokio::test]
    async fn deleting_a_model_removes_it() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        write_models_json(seeded_provider()).await;

        let mut prompt = FakePrompter::new(&[
            "2", "acme", "", "", "", "", "d", // delete reasoner-v1
            "",  // keep fast-v1
            "",  // no more models
        ]);
        let (out, _captured) = Output::memory();
        configure_with(&mut prompt, &out).await.unwrap();

        let models = read_models_json().await;
        let entries = models["providers"]["acme"]["models"].as_array().unwrap();
        assert_eq!(entries.len(), 1, "{models}");
        assert_eq!(entries[0]["id"], "fast-v1");
    }

    /// A provider must not end up model-less: deleting every model without
    /// adding one is refused before anything is written.
    #[tokio::test]
    async fn deleting_every_model_without_adding_one_is_refused() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        write_models_json(seeded_provider()).await;

        let mut prompt = FakePrompter::new(&[
            "2", "acme", "", "", "", "", "d", "d", "n", // no model added
        ]);
        let (out, _captured) = Output::memory();
        let err = configure_with(&mut prompt, &out)
            .await
            .expect_err("an empty model list is not a provider");
        assert_eq!(err, "At least one model is required.");
        // The existing provider was not rewritten.
        assert_eq!(read_models_json().await, seeded_provider());
    }

    /// Two models in one provider cannot share an id; the second prompt repeats
    /// until it is unique.
    #[tokio::test]
    async fn a_duplicate_model_id_is_refused() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let mut prompt = FakePrompter::new(&[
            "2",
            "acme",
            "",
            "1",
            "https://api.acme.test/v1",
            "",
            "",  // add a model
            "m", // first model
            "",
            "",
            "",
            "n",
            "",
            "",
            "",
            "",
            "y",  // add another
            "m",  // duplicate → refused
            "m2", // accepted
            "",
            "",
            "",
            "n",
            "",
            "",
            "",
            "",
            "", // no more models
        ]);
        let (out, captured) = Output::memory();
        configure_with(&mut prompt, &out).await.unwrap();
        let models = read_models_json().await;
        let entries = models["providers"]["acme"]["models"].as_array().unwrap();
        assert_eq!(entries.len(), 2, "{models}");
        assert_eq!(entries[0]["id"], "m");
        assert_eq!(entries[1]["id"], "m2");
        let stderr = output_text(&captured.err);
        assert!(stderr.contains("already uses that ID"), "{stderr}");
    }

    /// With no stored token the FutureOS path says so and hands off to the
    /// login flow.
    ///
    /// `auth::login` always targets `DEFAULT_PLATFORM_URL` (the override it
    /// would need is the caller's, and `configure_futureos` passes `None`), so
    /// the login itself is a real network round trip and is *not* followed:
    /// the call is bounded by a timeout and only the hand-off it logged before
    /// starting is asserted. That is the whole claim — the message is printed,
    /// the question is not asked, and control reaches the login call.
    #[tokio::test]
    async fn futureos_without_a_token_hands_off_to_login() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let _env = EnvGuard::set(&[
            ("HOME", dir.path().as_os_str().to_owned()),
            ("USERPROFILE", dir.path().as_os_str().to_owned()),
        ]);
        let path = crate::constants::auth_file();
        tokio::fs::create_dir_all(path.parent().expect("parent"))
            .await
            .expect("mkdir");
        // A `future` entry with no key is not a token.
        tokio::fs::write(&path, r#"{"future":{"base_url":"http://127.0.0.1:1"}}"#)
            .await
            .expect("write auth.json");

        let mut prompt = FakePrompter::new(&[]);
        let (out, captured) = Output::memory();
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            configure_futureos(&mut prompt, &out),
        )
        .await;

        let stdout = output_text(&captured.out);
        assert!(
            stdout.contains("No FutureOS token found. Starting login..."),
            "{stdout}"
        );
        assert!(
            !stdout.contains("Log in again?"),
            "no token means no replace question: {stdout}"
        );
    }

    /// An existing token is never silently replaced: declining leaves the file
    /// untouched, and accepting proceeds to the login flow (which is bounded
    /// for the same reason as above, so only the hand-off is asserted).
    #[tokio::test]
    async fn futureos_existing_token_asks_before_replacing_it() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let _env = EnvGuard::set(&[
            ("HOME", dir.path().as_os_str().to_owned()),
            ("USERPROFILE", dir.path().as_os_str().to_owned()),
        ]);
        let path = crate::constants::auth_file();
        tokio::fs::create_dir_all(path.parent().expect("parent"))
            .await
            .expect("mkdir");
        let seeded =
            r#"{"future":{"type":"api_key","key":"keep-me","base_url":"http://127.0.0.1:1"}}"#;
        tokio::fs::write(&path, seeded).await.expect("write");

        // Declining: reported, and the stored key is untouched.
        let mut prompt = FakePrompter::new(&["n"]);
        let (out, captured) = Output::memory();
        configure_futureos(&mut prompt, &out)
            .await
            .expect("declining is not an error");
        let stdout = output_text(&captured.out);
        assert!(stdout.contains("Log in again?"), "{stdout}");
        assert!(stdout.contains("no changes were made"), "{stdout}");
        assert_eq!(
            tokio::fs::read_to_string(&path).await.expect("read"),
            seeded
        );

        // Accepting reaches the login hand-off (bounded; see the test above).
        let mut prompt = FakePrompter::new(&["y"]);
        let (out, captured) = Output::memory();
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            configure_futureos(&mut prompt, &out),
        )
        .await;
        let stdout = output_text(&captured.out);
        assert!(stdout.contains("Log in again?"), "{stdout}");
        assert!(
            !stdout.contains("no changes were made"),
            "accepting must not take the decline path: {stdout}"
        );
        assert_eq!(
            tokio::fs::read_to_string(&path).await.expect("read"),
            seeded
        );
    }
}
