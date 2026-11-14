//! `future models providers|provider|model|probe` — the non-interactive model
//! settings surface.
//!
//! `future config` is the guided wizard, and `future models` answers "what can
//! the agent use right now". These four subcommands are the scriptable middle:
//! they read and write exactly the provider/model configuration the desktop's
//! Settings -> Providers page and the phone's provider editor write, through
//! the same Agent RPCs, so all three clients share one validation and one
//! configuration revision. They need a running Agent for that reason (the
//! Agent is the only writer).
//!
//! Nothing here re-implements a rule the Agent already enforces (id charset,
//! name collisions, token limits, thinking-level caps): the CLI builds the
//! write and reports the Agent's refusal verbatim.

use crate::flags::parse_flags;
use crate::output::Output;
use crate::rpc::{grpc_addr, RunClient};
use future_rpc::proto::{ProviderModel, ProviderUpsert, ThinkingLevelProbe};
use serde_json::{json, Value};

/// The thinking levels the provider editors offer. `default` — send no
/// thinking parameter at all — is implicit and always available, so it is not
/// in this list; a probe tries all of them unless the caller names a subset.
const KNOWN_THINKING_LEVELS: [&str; 7] =
    ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

/// The reserved level meaning "send no thinking/effort parameter at all".
const DEFAULT_THINKING_LEVEL: &str = "default";

/// The account provider: signed in through `future auth login`, never edited
/// through a provider write.
const FUTURE_PROVIDER: &str = "future";

const PROVIDER_USAGE: &str =
    "usage: future models provider set <id> [--name <name>] [--api <api>] \
[--base-url <url>] [--clear-base-url] [--key <key>] [--clear-key] [--json]\n       \
future models provider remove <id> [--json]";
const MODEL_USAGE: &str = "usage: future models model set <provider>/<model> [--name <name>] \
[--context-window <tokens>] [--max-tokens <tokens>] [--images|--text-only] \
[--thinking-levels <a,b,c>] [--clear-thinking-levels] [--input-cost <n>] [--output-cost <n>] \
[--cache-read-cost <n>] [--cache-write-cost <n>] [--json]\n       \
future models model remove <provider>/<model>";
const PROBE_USAGE: &str = "usage: future models probe <provider>/<model> [--level <level>] \
[--levels <a,b,c>] [--api <api>] [--base-url <url>] [--key <key>] [--insecure] [--json]";

/// The Agent endpoint: the first positional, or the configured default.
fn agent_address(positional: &[String]) -> String {
    positional.first().cloned().unwrap_or_else(grpc_addr)
}

/// Report a failure the way the other `future models` commands do.
fn fail(json_flag: bool, out: &Output, message: &str) -> Result<(), String> {
    if json_flag {
        out.log(&json!({ "error": message }).to_string());
        Err(crate::HANDLED_EXIT.to_string())
    } else {
        Err(message.to_string())
    }
}

/// `future models providers [--json] [<addr>]` — the provider + model settings
/// view (built-in providers, custom providers and every model setting).
pub async fn providers(args: &[String], out: &Output) -> Result<(), String> {
    let flags = parse_flags(args, &[], &[], &["--json"])?;
    let client = RunClient::new(&agent_address(&flags.positional));
    let view = match client.list_providers().await {
        Ok(view) => view,
        Err(message) => {
            return fail(
                flags.has("--json"),
                out,
                &format!("failed to read the provider configuration: {message}"),
            )
        }
    };
    if flags.has("--json") {
        out.log(&serde_json::to_string_pretty(&view).expect("provider view serializes"));
        return Ok(());
    }
    print_providers(&view, out);
    Ok(())
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn number(value: &Value, key: &str) -> f64 {
    value.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

fn integer(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn key_state(value: &Value) -> &'static str {
    if value
        .get("hasApiKey")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "set"
    } else {
        "none"
    }
}

fn print_providers(view: &Value, out: &Output) {
    let builtin = view.get("builtin").and_then(Value::as_array);
    let custom = view.get("custom").and_then(Value::as_array);
    for provider in builtin.into_iter().flatten() {
        out.log(&format!(
            "{}  built-in  {}  key={}  models={}",
            text(provider, "id"),
            text(provider, "baseUrl"),
            key_state(provider),
            integer(provider, "modelCount")
        ));
    }
    for provider in custom.into_iter().flatten() {
        out.log(&format!(
            "{}  custom  {}  {}  key={}  models={}",
            text(provider, "id"),
            text(provider, "api"),
            text(provider, "baseUrl"),
            key_state(provider),
            provider
                .get("models")
                .and_then(Value::as_array)
                .map_or(0, Vec::len)
        ));
        for model in provider
            .get("models")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            print_model_line(model, "  ", out);
        }
    }
    let builtin_count = builtin.map_or(0, Vec::len);
    let custom_count = custom.map_or(0, Vec::len);
    let model_count: usize = custom
        .into_iter()
        .flatten()
        .map(|provider| {
            provider
                .get("models")
                .and_then(Value::as_array)
                .map_or(0, Vec::len)
        })
        .sum();
    out.log("");
    out.log(&format!(
        "{builtin_count} built-in providers, {custom_count} custom providers, {model_count} custom models."
    ));
}

/// One model's settings on a single line. Printed by `providers` and as the
/// result of a successful `model set`, so both read the same way.
fn print_model_line(model: &Value, indent: &str, out: &Output) {
    let levels = model
        .get("thinkingLevels")
        .and_then(Value::as_array)
        .map(|levels| {
            levels
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default();
    let levels = if levels.is_empty() {
        "none".to_string()
    } else {
        levels
    };
    let modalities = if model
        .get("supportsImages")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "text+image"
    } else {
        "text"
    };
    let name = text(model, "name");
    let name = if name.is_empty() {
        String::new()
    } else {
        format!("  {name}")
    };
    out.log(&format!(
        "{indent}{}{name}  ctx={}  max={}  input={}  thinking={}  price(in={},out={},cache-read={},cache-write={})",
        text(model, "id"),
        integer(model, "contextWindow"),
        integer(model, "maxTokens"),
        modalities,
        levels,
        number(model, "inputCost"),
        number(model, "outputCost"),
        number(model, "cacheReadCost"),
        number(model, "cacheWriteCost")
    ));
}

/// Read the provider view, mapping an unreachable Agent to the message this
/// group uses everywhere.
async fn read_view(client: &RunClient) -> Result<Value, String> {
    client
        .list_providers()
        .await
        .map_err(|message| format!("failed to read the provider configuration: {message}"))
}

fn find_provider<'a>(view: &'a Value, id: &str, list: &str) -> Option<&'a Value> {
    view.get(list)
        .and_then(Value::as_array)?
        .iter()
        .find(|provider| text(provider, "id") == id)
}

/// Print the provider's current state after a write (or the whole view under
/// `--json`), so a caller never has to guess what landed.
fn report_view(view: &Value, json_flag: bool, out: &Output) {
    if json_flag {
        out.log(&serde_json::to_string_pretty(view).expect("provider view serializes"));
        return;
    }
    print_providers(view, out);
}

/// `future models provider set|remove ...`
pub async fn provider(args: &[String], out: &Output) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("set") => provider_set(&args[1..], out).await,
        Some("remove") => provider_remove(&args[1..], out).await,
        _ => Err(PROVIDER_USAGE.to_string()),
    }
}

async fn provider_set(args: &[String], out: &Output) -> Result<(), String> {
    let flags = parse_flags(
        args,
        &["--name", "--api", "--base-url", "--key"],
        &[],
        &["--clear-key", "--clear-base-url", "--json"],
    )?;
    let json_flag = flags.has("--json");
    let Some(id) = flags.positional.first().map(String::as_str) else {
        return Err(PROVIDER_USAGE.to_string());
    };
    let id = id.trim().to_lowercase();
    if flags.value("--key").is_some() && flags.has("--clear-key") {
        return Err("use --key or --clear-key, not both".to_string());
    }
    if flags.value("--base-url").is_some() && flags.has("--clear-base-url") {
        return Err("use --base-url or --clear-base-url, not both".to_string());
    }

    let client = RunClient::new(&grpc_addr());
    let view = read_view(&client).await?;

    // The account provider is signed in, not configured: its address and
    // credential belong to the sign-in flow.
    if id == FUTURE_PROVIDER {
        return fail(
            json_flag,
            out,
            "`future` is signed in with `future auth login`; its key and address are managed there.",
        );
    }

    let mut upsert = ProviderUpsert {
        id: id.clone(),
        ..Default::default()
    };
    if let Some(base_url) = flags.value("--base-url") {
        upsert.base_url = base_url.to_string();
    }
    upsert.clear_base_url = flags.has("--clear-base-url");
    if let Some(key) = flags.value("--key") {
        upsert.api_key = key.to_string();
    }
    upsert.clear_api_key = flags.has("--clear-key");

    if find_provider(&view, &id, "builtin").is_some() {
        // A built-in provider's models come from the catalog; only its
        // credential and address are the user's to change.
        if flags.value("--name").is_some() || flags.value("--api").is_some() {
            return fail(
                json_flag,
                out,
                &format!(
                    "`{id}` is a built-in provider: its models come from the catalog, so only \
                     --key/--clear-key and --base-url/--clear-base-url can change."
                ),
            );
        }
        if upsert.api_key.is_empty()
            && !upsert.clear_api_key
            && upsert.base_url.is_empty()
            && !upsert.clear_base_url
        {
            return fail(
                json_flag,
                out,
                "no provider changes were supplied: pass --key, --clear-key, --base-url or \
                 --clear-base-url.",
            );
        }
        if let Err(message) = client.upsert_provider(upsert).await {
            return fail(
                json_flag,
                out,
                &format!("failed to update `{id}`: {message}"),
            );
        }
    } else if let Some(existing) = find_provider(&view, &id, "custom") {
        // An edit keeps everything the caller did not name.
        upsert.name = flags
            .value("--name")
            .map(str::to_string)
            .unwrap_or_else(|| text(existing, "name"));
        upsert.api = flags
            .value("--api")
            .map(str::to_string)
            .unwrap_or_else(|| text(existing, "api"));
        if upsert.base_url.is_empty() && !upsert.clear_base_url {
            upsert.base_url = text(existing, "baseUrl");
        }
        if let Err(message) = client.upsert_provider(upsert).await {
            return fail(
                json_flag,
                out,
                &format!("failed to update `{id}`: {message}"),
            );
        }
    } else {
        let (Some(name), Some(api), Some(base_url)) = (
            flags.value("--name"),
            flags.value("--api"),
            flags.value("--base-url"),
        ) else {
            return fail(
                json_flag,
                out,
                &format!(
                    "provider `{id}` is not configured yet; creating it needs --name, --api and \
                     --base-url."
                ),
            );
        };
        upsert.name = name.to_string();
        upsert.api = api.to_string();
        upsert.base_url = base_url.to_string();
        upsert.create_only = true;
        if let Err(message) = client.upsert_provider(upsert).await {
            return fail(
                json_flag,
                out,
                &format!("failed to create `{id}`: {message}"),
            );
        }
    }

    match read_view(&client).await {
        Ok(view) => {
            report_view(&view, json_flag, out);
            Ok(())
        }
        Err(message) => fail(json_flag, out, &message),
    }
}

async fn provider_remove(args: &[String], out: &Output) -> Result<(), String> {
    let flags = parse_flags(args, &[], &[], &["--json"])?;
    let json_flag = flags.has("--json");
    let Some(id) = flags.positional.first().map(String::as_str) else {
        return Err(PROVIDER_USAGE.to_string());
    };
    let id = id.trim().to_lowercase();

    let client = RunClient::new(&grpc_addr());
    let view = read_view(&client).await?;
    if find_provider(&view, &id, "builtin").is_some() {
        return fail(
            json_flag,
            out,
            &format!(
                "`{id}` is a built-in provider and cannot be removed; clear its key with \
                 `future models provider set {id} --clear-key` instead."
            ),
        );
    }
    if find_provider(&view, &id, "custom").is_none() {
        return fail(
            json_flag,
            out,
            &format!("provider `{id}` is not configured."),
        );
    }
    if let Err(message) = client.delete_provider(&id).await {
        return fail(
            json_flag,
            out,
            &format!("failed to remove `{id}`: {message}"),
        );
    }
    match read_view(&client).await {
        Ok(view) => {
            report_view(&view, json_flag, out);
            Ok(())
        }
        Err(message) => fail(json_flag, out, &message),
    }
}

/// Split `provider/model` on the first `/`, which is the only separator a
/// reference uses (`openrouter/auto` is a model *id*, not a provider).
fn split_reference(reference: &str) -> Option<(String, String)> {
    let (provider, model) = reference.split_once('/')?;
    let provider = provider.trim();
    let model = model.trim();
    (!provider.is_empty() && !model.is_empty()).then(|| (provider.to_string(), model.to_string()))
}

fn provider_model_from_view(model: &Value) -> ProviderModel {
    ProviderModel {
        id: text(model, "id"),
        name: text(model, "name"),
        modalities: if model
            .get("supportsImages")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            vec!["text".to_string(), "image".to_string()]
        } else {
            vec!["text".to_string()]
        },
        context_window: integer(model, "contextWindow") as i32,
        max_tokens: integer(model, "maxTokens") as i32,
        reasoning: Some(
            model
                .get("reasoning")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        ),
        // Absent = the Agent keeps the stored list, which is what an edit that
        // did not name the levels must do.
        thinking_levels: Vec::new(),
        replace_thinking_levels: None,
        cost_input: number(model, "inputCost"),
        cost_output: number(model, "outputCost"),
        cost_cache_read: number(model, "cacheReadCost"),
        cost_cache_write: number(model, "cacheWriteCost"),
    }
}

fn parse_levels(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|level| !level.is_empty() && *level != DEFAULT_THINKING_LEVEL)
        .map(str::to_string)
        .collect()
}

/// `future models model set|remove ...`
pub async fn model(args: &[String], out: &Output) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("set") => model_set(&args[1..], out).await,
        Some("remove") => model_remove(&args[1..], out).await,
        _ => Err(MODEL_USAGE.to_string()),
    }
}

/// The provider a model write targets: an error naming the fix when it is a
/// built-in provider or not configured at all.
fn writable_provider<'a>(view: &'a Value, provider: &str) -> Result<&'a Value, String> {
    if let Some(found) = find_provider(view, provider, "custom") {
        return Ok(found);
    }
    if find_provider(view, provider, "builtin").is_some() {
        return Err(format!(
            "`{provider}` is a built-in provider: its models come from the catalog, so only \
             --key/--clear-key and --base-url/--clear-base-url can change."
        ));
    }
    Err(format!(
        "provider `{provider}` is not configured; create it with \
         `future models provider set {provider} --name <name> --api <api> --base-url <url>`."
    ))
}

async fn model_set(args: &[String], out: &Output) -> Result<(), String> {
    let flags = parse_flags(
        args,
        &[
            "--name",
            "--context-window",
            "--max-tokens",
            "--thinking-levels",
            "--input-cost",
            "--output-cost",
            "--cache-read-cost",
            "--cache-write-cost",
        ],
        &[],
        &[
            "--images",
            "--text-only",
            "--clear-thinking-levels",
            "--json",
        ],
    )?;
    let json_flag = flags.has("--json");
    let Some(reference) = flags.positional.first() else {
        return Err(MODEL_USAGE.to_string());
    };
    let Some((provider_id, model_id)) = split_reference(reference) else {
        return Err(format!(
            "expected <provider>/<model>, got `{reference}`\n{MODEL_USAGE}"
        ));
    };
    if flags.has("--images") && flags.has("--text-only") {
        return Err("use --images or --text-only, not both".to_string());
    }
    if flags.value("--thinking-levels").is_some() && flags.has("--clear-thinking-levels") {
        return Err("use --thinking-levels or --clear-thinking-levels, not both".to_string());
    }

    let client = RunClient::new(&grpc_addr());
    let view = read_view(&client).await?;
    let provider = writable_provider(&view, &provider_id)?;

    let mut models: Vec<ProviderModel> = provider
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(provider_model_from_view)
        .collect();
    let existing = models.iter().position(|model| model.id == model_id);
    // A model that is not configured yet starts from the editors' defaults, so
    // `model set acme/new --context-window ...` is an add and an edit alike.
    let mut target = existing
        .map(|index| {
            let mut model = models[index].clone();
            // The view's levels are informational here: only an explicit flag
            // replaces them (see below), and re-sending them would turn an
            // inherited provider-level list into a model-level one.
            model.thinking_levels.clear();
            model.replace_thinking_levels = None;
            model
        })
        .unwrap_or(ProviderModel {
            id: model_id.clone(),
            name: String::new(),
            modalities: vec!["text".to_string()],
            context_window: 128_000,
            max_tokens: 16_384,
            reasoning: Some(true),
            thinking_levels: Vec::new(),
            replace_thinking_levels: None,
            cost_input: 0.0,
            cost_output: 0.0,
            cost_cache_read: 0.0,
            cost_cache_write: 0.0,
        });

    if let Some(name) = flags.value("--name") {
        target.name = name.to_string();
    }
    if let Some(raw) = flags.value("--context-window") {
        target.context_window = parse_positive_int("--context-window", raw)?;
    }
    if let Some(raw) = flags.value("--max-tokens") {
        target.max_tokens = parse_positive_int("--max-tokens", raw)?;
    }
    if flags.has("--images") {
        target.modalities = vec!["text".to_string(), "image".to_string()];
    }
    if flags.has("--text-only") {
        target.modalities = vec!["text".to_string()];
    }
    if let Some(raw) = flags.value("--thinking-levels") {
        target.thinking_levels = parse_levels(raw);
        target.replace_thinking_levels = Some(true);
    }
    if flags.has("--clear-thinking-levels") {
        target.thinking_levels = Vec::new();
        target.replace_thinking_levels = Some(true);
    }
    for (flag, field) in [
        ("--input-cost", &mut target.cost_input),
        ("--output-cost", &mut target.cost_output),
        ("--cache-read-cost", &mut target.cost_cache_read),
        ("--cache-write-cost", &mut target.cost_cache_write),
    ] {
        if let Some(raw) = flags.value(flag) {
            *field = parse_price(flag, raw)?;
        }
    }

    let created = existing.is_none();
    match existing {
        Some(index) => models[index] = target,
        None => models.push(target),
    }

    let upsert = ProviderUpsert {
        id: provider_id.clone(),
        name: text(provider, "name"),
        api: text(provider, "api"),
        base_url: text(provider, "baseUrl"),
        models,
        replace_models: true,
        ..Default::default()
    };
    if let Err(message) = client.upsert_provider(upsert).await {
        return fail(
            json_flag,
            out,
            &format!("failed to save `{provider_id}/{model_id}`: {message}"),
        );
    }

    let view = match read_view(&client).await {
        Ok(view) => view,
        Err(message) => return fail(json_flag, out, &message),
    };
    if json_flag {
        out.log(&serde_json::to_string_pretty(&view).expect("provider view serializes"));
        return Ok(());
    }
    let saved = saved_model(&view, &provider_id, &model_id);
    match saved {
        Some(model) => {
            out.log(&format!(
                "{} model {provider_id}/{model_id}",
                if created { "Added" } else { "Saved" }
            ));
            print_model_line(&model, "  ", out);
            out.log("The running Agent (and every connected client) now uses these settings.");
            Ok(())
        }
        None => fail(
            false,
            out,
            &format!("the Agent accepted the write but `{model_id}` is not in the saved view"),
        ),
    }
}

/// The saved model as the Agent's own view reports it, which is what landed —
/// not what the caller sent.
fn saved_model(view: &Value, provider_id: &str, model_id: &str) -> Option<Value> {
    find_provider(view, provider_id, "custom")
        .and_then(|provider| provider.get("models").and_then(Value::as_array))
        .and_then(|models| {
            models
                .iter()
                .find(|model| text(model, "id") == model_id)
                .cloned()
        })
}

async fn model_remove(args: &[String], out: &Output) -> Result<(), String> {
    let flags = parse_flags(args, &[], &[], &["--json"])?;
    let json_flag = flags.has("--json");
    let Some(reference) = flags.positional.first() else {
        return Err(MODEL_USAGE.to_string());
    };
    let Some((provider_id, model_id)) = split_reference(reference) else {
        return Err(format!(
            "expected <provider>/<model>, got `{reference}`\n{MODEL_USAGE}"
        ));
    };

    let client = RunClient::new(&grpc_addr());
    let view = read_view(&client).await?;
    let provider = writable_provider(&view, &provider_id)?;
    let mut models: Vec<ProviderModel> = provider
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(provider_model_from_view)
        .collect();
    let before = models.len();
    models.retain(|model| model.id != model_id);
    if models.len() == before {
        return fail(
            json_flag,
            out,
            &format!("provider `{provider_id}` has no model `{model_id}`."),
        );
    }

    let upsert = ProviderUpsert {
        id: provider_id.clone(),
        name: text(provider, "name"),
        api: text(provider, "api"),
        base_url: text(provider, "baseUrl"),
        models,
        replace_models: true,
        ..Default::default()
    };
    if let Err(message) = client.upsert_provider(upsert).await {
        return fail(
            json_flag,
            out,
            &format!("failed to remove `{provider_id}/{model_id}`: {message}"),
        );
    }
    match read_view(&client).await {
        Ok(view) => {
            if json_flag {
                out.log(&serde_json::to_string_pretty(&view).expect("provider view serializes"));
            } else {
                out.log(&format!("Removed model {provider_id}/{model_id}."));
            }
            Ok(())
        }
        Err(message) => fail(json_flag, out, &message),
    }
}

fn parse_positive_int(flag: &str, raw: &str) -> Result<i32, String> {
    let value: i32 = raw
        .parse()
        .map_err(|_| format!("{flag} must be a whole number of tokens, got `{raw}`"))?;
    if value <= 0 {
        return Err(format!("{flag} must be positive, got {value}"));
    }
    Ok(value)
}

fn parse_price(flag: &str, raw: &str) -> Result<f64, String> {
    let value: f64 = raw
        .parse()
        .map_err(|_| format!("{flag} must be a number, got `{raw}`"))?;
    if !value.is_finite() || value < 0.0 {
        return Err(format!("{flag} must be a non-negative number, got `{raw}`"));
    }
    Ok(value)
}

/// `future models probe <provider>/<model> [...]` — ask the Agent which
/// thinking levels the provider actually accepts.
///
/// The probes go through the Agent's own adapters, so an accepted level is one
/// whose outbound parameter really is accepted; the CLI only sequences them and
/// reports. The `default` level (no thinking parameter at all) comes first in
/// the default set: if that is refused, the endpoint/key/model id is wrong and
/// no level verdict would mean anything.
pub async fn probe(args: &[String], out: &Output) -> Result<(), String> {
    let flags = parse_flags(
        args,
        &["--level", "--levels", "--api", "--base-url", "--key"],
        &[],
        &["--json", "--insecure"],
    )?;
    let json_flag = flags.has("--json");
    let Some(reference) = flags.positional.first() else {
        return Err(PROBE_USAGE.to_string());
    };
    let Some((provider_id, model_id)) = split_reference(reference) else {
        return Err(format!(
            "expected <provider>/<model>, got `{reference}`\n{PROBE_USAGE}"
        ));
    };
    if flags.value("--level").is_some() && flags.value("--levels").is_some() {
        return Err("use --level or --levels, not both".to_string());
    }

    let levels: Vec<String> = if let Some(level) = flags.value("--level") {
        let level = level.trim();
        if level.is_empty() {
            return Err("--level needs a level name".to_string());
        }
        vec![level.to_string()]
    } else if let Some(levels) = flags.value("--levels") {
        let levels = parse_levels(levels);
        if levels.is_empty() {
            return Err("--levels named no level: pass off,low,high,...".to_string());
        }
        levels
    } else {
        // The calibration call first, then every level the editors offer.
        std::iter::once(DEFAULT_THINKING_LEVEL.to_string())
            .chain(
                KNOWN_THINKING_LEVELS
                    .iter()
                    .map(|level| (*level).to_string()),
            )
            .collect()
    };
    let single_level_only = flags.value("--level").is_some();

    let client = RunClient::new(&grpc_addr());
    let mut probes = Vec::new();
    for level in &levels {
        let outcome = client
            .probe_thinking_level(ThinkingLevelProbe {
                api: flags.value("--api").unwrap_or_default().trim().to_string(),
                base_url: flags
                    .value("--base-url")
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
                api_key: flags.value("--key").unwrap_or_default().trim().to_string(),
                provider_id: provider_id.clone(),
                model_id: model_id.clone(),
                level: level.clone(),
                // A configured provider inherits its stored decision inside the
                // agent; `--insecure` states it for one that is not saved yet.
                skip_tls_verify: flags.has("--insecure").then_some(true),
            })
            .await
            .map_err(|message| {
                format!(
                    "the thinking-level probe did not complete: {message}\n\
                     A model that is not configured yet needs --api and --base-url."
                )
            })?;
        let accepted = outcome.get("ok").and_then(Value::as_bool).unwrap_or(false);
        // The calibration call is the gate: if the endpoint refuses the request
        // that carries no thinking parameter at all, the base URL / key / model
        // id is wrong and no level verdict from the remaining calls would mean
        // anything. Stop there rather than spending one call per level.
        let calibration = !single_level_only && level == DEFAULT_THINKING_LEVEL;
        probes.push(outcome);
        if calibration && !accepted {
            break;
        }
    }

    let accepted: Vec<String> = probes
        .iter()
        .filter(|probe| probe.get("ok").and_then(Value::as_bool).unwrap_or(false))
        .map(|probe| text(probe, "level"))
        .collect();

    if json_flag {
        out.log(
            &serde_json::to_string_pretty(&json!({
                "model": format!("{provider_id}/{model_id}"),
                "accepted": accepted,
                "probes": probes,
            }))
            .expect("probe report serializes"),
        );
    } else {
        out.log(&format!("{provider_id}/{model_id}"));
        for probe in &probes {
            let level = text(probe, "level");
            if probe.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                out.log(&format!("  {level:<9} ok"));
            } else {
                out.log(&format!("  {level:<9} rejected  {}", text(probe, "error")));
            }
        }
        if accepted.is_empty() {
            out.log("Accepted: none (only the implicit default level).");
        } else {
            out.log(&format!("Accepted: {}", accepted.join(",")));
        }
    }

    // A refused level is an answer, so a single-level probe always succeeds.
    // The calibration call is different: its failure means the endpoint, key or
    // model id is wrong, which a caller scripted against must not read as
    // "this level is not offered".
    if !single_level_only {
        if let Some(calibration) = probes
            .iter()
            .find(|probe| text(probe, "level") == DEFAULT_THINKING_LEVEL)
        {
            if !calibration
                .get("ok")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                return Err(format!(
                    "the endpoint refused the calibration request (no thinking parameter): {}",
                    text(calibration, "error")
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
