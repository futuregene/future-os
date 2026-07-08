//! Field validation for custom-provider upserts (see PLAN.md "Custom Provider
//! Field Validation"). These checks depend only on the request, so they run
//! before — and separate from — the locked models.json read-modify-write in
//! [`super::write`].

use serde_json::{json, Value};

use crate::auth_store::FUTURE_PROVIDER_ID;

use super::UpsertCustomProviderInput;

// Field-validation limits for custom providers. `pub(super)` because the write
// paths reuse the same caps for the built-in key / base-URL forms.
pub(super) const PROVIDER_ID_MIN_LEN: usize = 2;
pub(super) const PROVIDER_ID_MAX_LEN: usize = 40;
pub(super) const PROVIDER_NAME_MAX_LEN: usize = 40;
pub(super) const BASE_URL_MAX_LEN: usize = 2048;
pub(super) const API_KEY_MAX_LEN: usize = 512;
pub(super) const MODEL_ID_MAX_LEN: usize = 100;
pub(super) const MODEL_NAME_MAX_LEN: usize = 60;
pub(super) const MAX_MODELS: usize = 100;
pub(super) const ALLOWED_APIS: [&str; 3] = ["openai-completions", "openai-responses", "anthropic"];

/// A custom-provider request with every field validated and normalized, ready
/// for the locked models.json read-modify-write in
/// [`super::write::upsert_custom_provider`].
pub(super) struct ValidatedCustomProvider {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) api: String,
    pub(super) base_url: String,
    /// Present only when a non-empty key was supplied (trimmed); otherwise the
    /// existing key is left untouched.
    pub(super) api_key: Option<String>,
    /// Models serialized to the models.json `modalities` shape.
    pub(super) model_values: Vec<Value>,
    /// Carried through: true when creating a new provider (vs editing).
    pub(super) create: bool,
}

/// Validate and normalize an upsert request. Contents-dependent checks (id/name
/// uniqueness against existing entries and the built-in catalog) stay in the
/// locked write path; only request-local rules live here.
pub(super) fn validate_custom_provider(
    input: UpsertCustomProviderInput,
) -> Result<ValidatedCustomProvider, crate::AppError> {
    // Provider id: lowercased, [a-z0-9_-], length-bounded, `future` reserved.
    let id = input.id.trim().to_lowercase();
    if id.is_empty() {
        return Err("Provider ID is required.".into());
    }
    if id == FUTURE_PROVIDER_ID {
        return Err(
            "`future` is reserved for built-in FutureGene; please choose another ID.".into(),
        );
    }
    if id.len() < PROVIDER_ID_MIN_LEN || id.len() > PROVIDER_ID_MAX_LEN {
        return Err(format!(
            "Provider ID length must be between {PROVIDER_ID_MIN_LEN}–{PROVIDER_ID_MAX_LEN} characters."
        )
        .into());
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err("Provider ID may only contain lowercase letters, digits, '-', and '_'.".into());
    }

    // Base URL: parseable http/https, length-bounded.
    let base_url = input.base_url.trim().to_string();
    if base_url.is_empty() {
        return Err("Base URL is required.".into());
    }
    if base_url.len() > BASE_URL_MAX_LEN {
        return Err("Base URL is too long.".into());
    }
    match reqwest::Url::parse(&base_url) {
        Ok(url) if matches!(url.scheme(), "http" | "https") => {}
        _ => return Err("Base URL must be a valid http/https address.".into()),
    }

    // API type: must be a supported value.
    let api = {
        let trimmed = input.api.trim();
        if trimmed.is_empty() {
            "openai-completions".to_string()
        } else if ALLOWED_APIS.contains(&trimmed) {
            trimmed.to_string()
        } else {
            return Err(format!("Unsupported API type: `{trimmed}`.").into());
        }
    };

    // Name: optional (falls back to id); when given, ASCII only (no CJK / emoji /
    // full-width), restricted punctuation, length-bounded.
    let name = {
        let trimmed = input.name.trim();
        if trimmed.is_empty() {
            id.clone()
        } else {
            if trimmed.chars().count() > PROVIDER_NAME_MAX_LEN {
                return Err(format!(
                    "Provider name cannot exceed {PROVIDER_NAME_MAX_LEN} characters."
                )
                .into());
            }
            if !is_provider_name_ok(trimmed) {
                return Err(
                    "Provider name may only contain letters, digits, spaces, and _.()-; Chinese / emoji / fullwidth characters are not supported."
                        .into(),
                );
            }
            trimmed.to_string()
        }
    };

    // API key: validated here, written after models.json by the caller (so an
    // invalid key doesn't leave a half-applied change).
    let api_key = input
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    if let Some(key) = api_key {
        if key.len() > API_KEY_MAX_LEN {
            return Err("API Key exceeds the maximum length.".into());
        }
        if !is_ascii_no_control(key) {
            return Err("API Key contains illegal characters.".into());
        }
    }

    // Models: validate ids/names, dedupe within the provider, cap the count.
    let mut seen_model_ids = std::collections::HashSet::new();
    let mut model_values: Vec<Value> = Vec::new();
    for model in &input.models {
        let model_id = model.id.trim();
        if model_id.is_empty() {
            continue;
        }
        if model_id.len() > MODEL_ID_MAX_LEN {
            return Err(
                format!("Model ID `{model_id}` is too long (max {MODEL_ID_MAX_LEN}).").into(),
            );
        }
        if !is_model_id_ok(model_id) {
            return Err(format!("Model ID `{model_id}` contains illegal characters.").into());
        }
        if !seen_model_ids.insert(model_id.to_string()) {
            return Err(format!("Model ID `{model_id}` is duplicated.").into());
        }
        let model_name = model.name.trim();
        let model_name = if model_name.is_empty() {
            model_id
        } else {
            if model_name.chars().count() > MODEL_NAME_MAX_LEN {
                return Err(
                    format!("Model name cannot exceed {MODEL_NAME_MAX_LEN} characters.").into(),
                );
            }
            if !is_ascii_no_control(model_name) {
                return Err(
                    format!("Model name `{model_name}` contains illegal characters.").into(),
                );
            }
            model_name
        };
        // Text is always supported; image is opt-in. Persist as `modalities` so
        // the agent's models.json loader maps it to the model's input modalities.
        let mut modalities = vec![Value::String("text".to_string())];
        if model.supports_images {
            modalities.push(Value::String("image".to_string()));
        }
        model_values.push(json!({
            "id": model_id,
            "name": model_name,
            "modalities": modalities,
        }));
    }
    if model_values.len() > MAX_MODELS {
        return Err(format!("Number of models cannot exceed {MAX_MODELS}.").into());
    }

    Ok(ValidatedCustomProvider {
        id,
        name,
        api,
        base_url,
        api_key: api_key.map(str::to_string),
        model_values,
        create: input.create,
    })
}

/// Provider display name: ASCII letters/digits, space, and `_.()-` only —
/// rejects control chars and all non-ASCII (CJK / emoji / full-width).
fn is_provider_name_ok(value: &str) -> bool {
    value.chars().all(|c| {
        c.is_ascii()
            && !c.is_control()
            && (c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '.' | '(' | ')' | '-'))
    })
}

/// Model id: ASCII, no whitespace, plus `._:/-` (covers ids like
/// `anthropic/claude-3.5-sonnet`).
fn is_model_id_ok(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|c| {
            c.is_ascii()
                && !c.is_whitespace()
                && (c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '-'))
        })
}

/// Free-form-ish text (model name, API key): ASCII, no control chars. Shared
/// with the built-in key form in [`super::write`].
pub(super) fn is_ascii_no_control(value: &str) -> bool {
    value.chars().all(|c| c.is_ascii() && !c.is_control())
}
