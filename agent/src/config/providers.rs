//! Agent-owned write path for `~/.future/agent/auth.json` and `models.json`
//! (audit item 2). The agent is the sole writer of its config files: the
//! `set_auth` / `upsert_provider` / `delete_provider` RPCs land here instead
//! of clients editing the files out-of-band and patching state with
//! `reload_auth` afterwards.
//!
//! Read/write semantics mirror the GUI's `config_io` so both sides agree on
//! the file contract: a missing file is an empty object, a corrupt file or a
//! non-object root is an error (never silently clobbered), and writes are
//! temp-file + rename with owner-only permissions for `auth.json`. All
//! read-modify-write cycles are serialized on a process-wide lock because
//! gRPC commands may be handled concurrently.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{json, Map, Value};

/// Consistent `(models.json, auth.json)` object snapshot used by provider RPCs.
pub type ProviderDocuments = (Map<String, Value>, Map<String, Value>);

/// One mutation of an `auth.json` provider entry (the domain carrier of the
/// proto `AuthUpdate` sub-message; the gRPC layer maps proto onto this).
#[derive(Debug, Clone, Default)]
pub struct AuthMutation {
    /// Provider entry name (e.g. "future", "openai").
    pub provider: String,
    /// `Some(non-empty)`: set the entry's `key` field.
    pub key: Option<String>,
    /// Remove the entry's `key` field (logout keeps `base_url`).
    pub clear_key: bool,
    /// `Some(non-empty)`: set the entry's `base_url` field (auth.json name).
    pub base_url: Option<String>,
    /// Remove the entry's `base_url` field.
    pub clear_base_url: bool,
    /// Remove the whole provider entry.
    pub remove_entry: bool,
    /// Remove the legacy `platform_base_url` field (environment switches).
    pub remove_platform_base_url: bool,
}

/// A model entry under a custom provider, persisted to `models.json`.
#[derive(Debug, Clone)]
pub struct ProviderModelSpec {
    pub id: String,
    pub name: String,
    /// Input modalities, e.g. `["text"]` or `["text", "image"]`.
    pub modalities: Vec<String>,
    /// Maximum total context window, in tokens.
    pub context_window: i32,
    /// Maximum tokens generated in one response.
    pub max_tokens: i32,
}

/// Create/update of a `models.json` `providers` entry, optionally with the
/// provider's API key written to `auth.json` in the same step (domain carrier
/// of the proto `ProviderUpsert` sub-message).
#[derive(Debug, Clone, Default)]
pub struct ProviderUpsertSpec {
    /// Provider id — the key in `models.json` `providers`.
    pub id: String,
    /// Non-empty: set `name`.
    pub name: Option<String>,
    /// Non-empty: set `api`.
    pub api: Option<String>,
    /// Non-empty: set the `baseUrl` override.
    pub base_url: Option<String>,
    /// Remove the `baseUrl` override; drop the entry if nothing remains.
    pub clear_base_url: bool,
    /// Replacement `models` list. Applied even when empty when
    /// [`Self::replace_models`] is true.
    pub models: Vec<ProviderModelSpec>,
    /// Presence bit for `models`; repeated proto fields cannot distinguish an
    /// omitted list from an explicit request to clear it.
    pub replace_models: bool,
    /// Fail when the provider already exists (create mode).
    pub create_only: bool,
    /// Non-empty: also store as this provider's `auth.json` key.
    pub api_key: Option<String>,
    /// Remove the provider key in the same two-file transaction.
    pub clear_api_key: bool,
}

impl ProviderUpsertSpec {
    /// Whether this mutation changes the models.json side of provider state.
    /// Non-empty model lists remain supported for older RPC clients that do
    /// not yet send the explicit replacement presence bit.
    fn changes_models_document(&self) -> bool {
        self.name.is_some()
            || self.api.is_some()
            || self.base_url.is_some()
            || self.clear_base_url
            || self.replace_models
            || !self.models.is_empty()
    }
}

fn valid_provider_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_".contains(&byte))
}

fn ascii_without_control(value: &str) -> bool {
    value.is_ascii() && !value.bytes().any(|byte| byte.is_ascii_control())
}

pub fn validate_auth_mutation(mutation: &AuthMutation) -> Result<(), String> {
    let provider = mutation.provider.trim();
    if !valid_provider_id(provider) {
        return Err("provider id must use lowercase letters, digits, '-' or '_'".to_string());
    }
    if let Some(key) = mutation.key.as_deref() {
        if key.len() > 16_384 || !ascii_without_control(key) {
            return Err("API key is invalid or too long".to_string());
        }
    }
    if let Some(base_url) = mutation.base_url.as_deref() {
        let url = reqwest::Url::parse(base_url)
            .map_err(|_| "base URL must be a valid http/https address".to_string())?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err("base URL must be a valid http/https address".to_string());
        }
    }
    Ok(())
}

pub fn validate_provider_upsert(spec: &ProviderUpsertSpec) -> Result<(), String> {
    let id = spec.id.trim();
    if !valid_provider_id(id) {
        return Err("provider id must use lowercase letters, digits, '-' or '_'".to_string());
    }
    if let Some(name) = spec.name.as_deref() {
        if name.is_empty() || name.len() > 128 || !ascii_without_control(name) {
            return Err("provider name is invalid or too long".to_string());
        }
    }
    if let Some(api) = spec.api.as_deref() {
        if !matches!(api, "openai-completions" | "openai-responses" | "anthropic") {
            return Err("unsupported provider API type".to_string());
        }
    }
    if let Some(base_url) = spec.base_url.as_deref() {
        let url = reqwest::Url::parse(base_url)
            .map_err(|_| "base URL must be a valid http/https address".to_string())?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err("base URL must be a valid http/https address".to_string());
        }
    }
    if let Some(key) = spec.api_key.as_deref() {
        if key.len() > 16_384 || !ascii_without_control(key) {
            return Err("API key is invalid or too long".to_string());
        }
    }
    if spec.api_key.is_some() && spec.clear_api_key {
        return Err("cannot set and clear an API key in the same mutation".to_string());
    }
    if spec.models.len() > 100 {
        return Err("provider has too many models".to_string());
    }
    let mut model_ids = std::collections::HashSet::new();
    for model in &spec.models {
        if model.id.is_empty()
            || model.id.len() > 256
            || !ascii_without_control(&model.id)
            || !model_ids.insert(model.id.as_str())
        {
            return Err("provider model id is invalid or duplicated".to_string());
        }
        if model.name.len() > 128 || !ascii_without_control(&model.name) {
            return Err("provider model name is invalid or too long".to_string());
        }
        if model.modalities.is_empty()
            || model
                .modalities
                .iter()
                .any(|modality| !matches!(modality.as_str(), "text" | "image"))
        {
            return Err("provider model modalities are invalid".to_string());
        }
        if model.context_window <= 0 || model.max_tokens <= 0 {
            return Err("provider model token limits must be positive".to_string());
        }
        if model.max_tokens > model.context_window {
            return Err("provider model max tokens cannot exceed its context window".to_string());
        }
    }
    Ok(())
}

/// Serializes every config read-modify-write: commands are handled
/// concurrently, and two interleaved RMWs would lose each other's update.
static CONFIG_WRITE_LOCK: Mutex<()> = Mutex::new(());

fn with_config_lock<R>(f: impl FnOnce() -> Result<R, String>) -> Result<R, String> {
    let guard = CONFIG_WRITE_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let result = f();
    drop(guard);
    result
}

/// `~/.future/agent/auth.json` — the same file the GUI's `auth_store` owns.
pub fn auth_json_path() -> PathBuf {
    crate::utils::default_config_dir().join("auth.json")
}

/// `~/.future/agent/models.json` (delegates to the models module's path so
/// reader and writer never diverge).
pub fn models_json_path() -> PathBuf {
    PathBuf::from(crate::models::user_models_path())
}

/// Strict JSON object read, matching the GUI's `config_io::read_json_object`:
/// missing file → empty object; corrupt JSON or a non-object root → error, so
/// a write can never clobber an unreadable (e.g. hand-edited) file.
fn read_json_object(path: &Path) -> Result<Map<String, Value>, String> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(error) => {
            return Err(format!("failed to read {}: {error}", path.display()));
        }
    };
    let value: Value = serde_json::from_str(&contents)
        .map_err(|error| format!("{} is corrupt: {error}", path.display()))?;
    match value {
        Value::Object(map) => Ok(map),
        _ => Err(format!("{} does not contain a JSON object", path.display())),
    }
}

/// Atomic write: serialize pretty (+ trailing newline, matching the GUI's
/// config files) to a uniquely-named sibling temp file, set permissions, then
/// `rename` over the target. Owner-only (0600) for `auth.json`, which holds
/// API keys; 0644 elsewhere.
fn write_json_atomic(
    path: &Path,
    map: &Map<String, Value>,
    owner_only: bool,
) -> Result<(), String> {
    let serialized = format!(
        "{}\n",
        serde_json::to_string_pretty(&Value::Object(map.clone()))
            .map_err(|error| format!("failed to serialize config: {error}"))?
    );
    write_bytes_atomic(path, serialized.as_bytes(), owner_only)
}

/// Low-level atomic write of raw bytes: temp-file + `rename`, so a torn write
/// can never leave a half-written config. Shared by the JSON writer and by the
/// transactional rollback path (which restores the *exact* original bytes, not
/// a re-serialization that could reorder or reformat them).
fn write_bytes_atomic(path: &Path, bytes: &[u8], owner_only: bool) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    static TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config.json".to_string());
    let tmp = path.with_file_name(format!(
        "{file_name}.tmp.{}.{}",
        std::process::id(),
        TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let write_result = (|| -> std::io::Result<()> {
        {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            let mut file = if owner_only {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600).open(&tmp)?
                }
                #[cfg(not(unix))]
                options.open(&tmp)?
            } else {
                options.open(&tmp)?
            };
            std::io::Write::write_all(&mut file, bytes)?;
            file.sync_all()?;
        }
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    write_result.map_err(|error| format!("failed to write {}: {error}", path.display()))
}

/// Capture a file's exact pre-mutation bytes for transactional rollback.
/// `Ok(None)` means the file did not exist (rollback must delete, not empty,
/// it). Any OTHER read error (permission, transient I/O) surfaces as `Err` —
/// it must never be confused with "did not exist", which would make rollback
/// DELETE an existing file.
fn snapshot_file(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "failed to read {} for rollback: {error}",
            path.display()
        )),
    }
}

/// Best-effort restore of a snapshot taken by [`snapshot_file`]. Used only on
/// the error path, so a failed restore is logged to stderr rather than
/// shadowing the original write error being returned to the caller.
fn restore_file(path: &Path, snapshot: Option<&[u8]>, owner_only: bool) {
    let result = match snapshot {
        Some(bytes) => write_bytes_atomic(path, bytes, owner_only),
        None => std::fs::remove_file(path)
            .map_err(|error| format!("failed to remove {}: {error}", path.display())),
    };
    if let Err(error) = result {
        // A `NotFound` remove just means the file was never created — fine.
        eprintln!(
            "FutureOS: config rollback could not restore {}: {error}",
            path.display()
        );
    }
}

/// Upsert a provider entry in an in-memory auth map: normalizes a missing or
/// non-object entry to `{}` and defaults `type` to `api_key` (mirrors the
/// GUI's `upsert_provider_entry`).
fn upsert_auth_entry<'a>(auth: &'a mut Map<String, Value>, id: &str) -> &'a mut Map<String, Value> {
    let entry = auth.entry(id.to_string()).or_insert_with(|| json!({}));
    if !entry.is_object() {
        *entry = json!({});
    }
    let object = entry
        .as_object_mut()
        .expect("entry was just normalized to an object");
    object
        .entry("type".to_string())
        .or_insert_with(|| Value::String("api_key".to_string()));
    object
}

/// Apply one auth mutation to an in-memory auth map (pure; file orchestration
/// is [`mutate_auth_file`]).
pub fn apply_auth_mutation(auth: &mut Map<String, Value>, mutation: &AuthMutation) {
    if mutation.remove_entry {
        auth.remove(&mutation.provider);
        return;
    }

    let sets_something = mutation.key.is_some() || mutation.base_url.is_some();
    if !sets_something {
        // Pure removal (logout-style): only touch an existing entry, never
        // create one — matches the GUI's remove_provider_key semantics.
        if let Some(entry) = auth
            .get_mut(&mutation.provider)
            .and_then(Value::as_object_mut)
        {
            if mutation.clear_key {
                entry.remove("key");
            }
            if mutation.clear_base_url {
                entry.remove("base_url");
            }
            if mutation.remove_platform_base_url {
                entry.remove("platform_base_url");
            }
        }
        return;
    }

    let entry = upsert_auth_entry(auth, &mutation.provider);
    if let Some(key) = &mutation.key {
        entry.insert("key".to_string(), Value::String(key.clone()));
    }
    if mutation.clear_key {
        entry.remove("key");
    }
    if let Some(base_url) = &mutation.base_url {
        entry.insert("base_url".to_string(), Value::String(base_url.clone()));
    }
    if mutation.clear_base_url {
        entry.remove("base_url");
    }
    if mutation.remove_platform_base_url {
        entry.remove("platform_base_url");
    }
}

/// Apply a provider upsert to an in-memory models.json root map (pure; file
/// orchestration is [`upsert_provider_files`]). Field-level validation
/// (lengths, formats) is the GUI's job before it sends the RPC; the agent
/// enforces the file-state invariants it owns: create-mode uniqueness and
/// provider-name uniqueness within models.json.
pub fn apply_provider_upsert(
    root: &mut Map<String, Value>,
    spec: &ProviderUpsertSpec,
) -> Result<(), String> {
    if !spec.changes_models_document() {
        let exists = root
            .get("providers")
            .and_then(Value::as_object)
            .is_some_and(|providers| providers.contains_key(&spec.id));
        if spec.create_only && exists {
            return Err(format!("Provider ID `{}` already exists.", spec.id));
        }
        return Ok(());
    }
    let providers_value = root
        .entry("providers".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let providers = providers_value
        .as_object_mut()
        .ok_or_else(|| "models.json `providers` is not an object.".to_string())?;

    if spec.create_only && providers.contains_key(&spec.id) {
        return Err(format!("Provider ID `{}` already exists.", spec.id));
    }
    if let Some(name) = &spec.name {
        let normalized = name.trim().to_lowercase();
        let taken = providers.iter().any(|(other_id, config)| {
            other_id != &spec.id
                && config
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(other_id)
                    .trim()
                    .to_lowercase()
                    == normalized
        });
        if taken {
            return Err(format!("Provider name `{name}` already exists."));
        }
    }

    // Merge into the existing entry, preserving fields the RPC does not
    // manage (e.g. `compat`).
    let mut provider = providers
        .get(&spec.id)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(name) = &spec.name {
        provider.insert("name".to_string(), Value::String(name.clone()));
    }
    if let Some(api) = &spec.api {
        provider.insert("api".to_string(), Value::String(api.clone()));
    }
    if let Some(base_url) = &spec.base_url {
        provider.insert("baseUrl".to_string(), Value::String(base_url.clone()));
    }
    if spec.clear_base_url {
        provider.remove("baseUrl");
    }
    if spec.replace_models || !spec.models.is_empty() {
        let models = spec
            .models
            .iter()
            .map(|model| {
                json!({
                    "id": model.id,
                    "name": model.name,
                    "modalities": model.modalities,
                    "contextWindow": model.context_window,
                    "maxTokens": model.max_tokens,
                })
            })
            .collect::<Vec<_>>();
        provider.insert("models".to_string(), Value::Array(models));
    }

    if spec.clear_base_url && provider.is_empty() {
        // Nothing but a cleared override remained — drop the entry entirely
        // (matches the GUI's clear path for built-in Base URL overrides).
        providers.remove(&spec.id);
    } else {
        providers.insert(spec.id.clone(), Value::Object(provider));
    }
    Ok(())
}

/// Remove a provider's models.json entry (pure). Returns whether an entry was
/// present, so callers can skip the file write for a no-op delete.
pub fn apply_provider_delete(root: &mut Map<String, Value>, id: &str) -> bool {
    root.get_mut("providers")
        .and_then(Value::as_object_mut)
        .map(|providers| providers.remove(id).is_some())
        .unwrap_or(false)
}

/// Read-modify-write one auth mutation into the given auth.json.
pub fn mutate_auth_file(auth_path: &Path, mutation: &AuthMutation) -> Result<(), String> {
    with_config_lock(|| {
        let mut auth = read_json_object(auth_path)?;
        apply_auth_mutation(&mut auth, mutation);
        write_json_atomic(auth_path, &auth, true)
    })
}

/// Create/update a provider across both files, transactionally. models.json is
/// written first, then auth.json; each write is atomic (temp-file + rename), so
/// a failed write leaves that file untouched. On failure, ONLY the file(s)
/// already written in this call are rolled back to their exact pre-mutation
/// bytes — an unmodified file is never restored (restoring it could clobber a
/// concurrent writer's just-committed change).
pub fn upsert_provider_files(
    auth_path: &Path,
    models_path: &Path,
    spec: &ProviderUpsertSpec,
) -> Result<(), String> {
    with_config_lock(|| {
        // A snapshot read error must abort BEFORE any write, never be treated
        // as "file did not exist" (which would make rollback delete the file).
        // Only models.json can be rolled back in an upsert — auth.json is
        // written last and atomically, so a failed auth write leaves it
        // untouched and there is nothing to restore.
        let models_change = spec.changes_models_document();
        let models_snapshot = if models_change {
            Some(snapshot_file(models_path)?)
        } else {
            None
        };

        // Validate + apply in memory first; nothing is written if this fails.
        let models_doc = if models_change {
            let mut models = read_json_object(models_path)?;
            apply_provider_upsert(&mut models, spec)?;
            Some(models)
        } else {
            None
        };

        // Prepare the auth mutation in memory too, so all reads/validations
        // precede the first disk write.
        let mut auth_doc = None;
        if spec.api_key.is_some() || spec.clear_api_key {
            let mut auth = read_json_object(auth_path)?;
            if let Some(key) = &spec.api_key {
                upsert_auth_entry(&mut auth, &spec.id)
                    .insert("key".to_string(), Value::String(key.clone()));
            } else if let Some(entry) = auth.get_mut(&spec.id).and_then(Value::as_object_mut) {
                entry.remove("key");
            }
            auth_doc = Some(auth);
        }

        // Persist models.json, then auth.json. Restore only what this call
        // already wrote: a failed models write means nothing was persisted; a
        // failed auth write means only models.json changed.
        if let Some(models) = models_doc {
            if let Err(error) = write_json_atomic(models_path, &models, false) {
                restore_file(
                    models_path,
                    models_snapshot
                        .as_ref()
                        .and_then(|snapshot| snapshot.as_deref()),
                    false,
                );
                return Err(error);
            }
        }
        if let Some(auth) = auth_doc {
            if let Err(error) = write_json_atomic(auth_path, &auth, true) {
                if models_change {
                    restore_file(
                        models_path,
                        models_snapshot
                            .as_ref()
                            .and_then(|snapshot| snapshot.as_deref()),
                        false,
                    );
                }
                return Err(error);
            }
        }
        Ok(())
    })
}

/// Remove a provider's models.json entry AND its auth.json entry,
/// transactionally. Same rule as [`upsert_provider_files`]: models.json is
/// written first, then auth.json, and on failure only models.json (the file
/// already written this call) is restored — a failed auth write leaves auth.json
/// untouched, and restoring it could clobber a concurrent writer's just-committed
/// change.
pub fn delete_provider_files(auth_path: &Path, models_path: &Path, id: &str) -> Result<(), String> {
    with_config_lock(|| {
        let models_snapshot = snapshot_file(models_path)?;

        let mut models_doc = read_json_object(models_path)?;
        let models_changed = apply_provider_delete(&mut models_doc, id);

        let mut auth = read_json_object(auth_path)?;
        let auth_changed = auth.remove(id).is_some();

        if !models_changed && !auth_changed {
            // Nothing to remove; leave both files untouched.
            return Ok(());
        }

        if models_changed {
            if let Err(error) = write_json_atomic(models_path, &models_doc, false) {
                restore_file(models_path, models_snapshot.as_deref(), false);
                return Err(error);
            }
        }
        if auth_changed {
            if let Err(error) = write_json_atomic(auth_path, &auth, true) {
                restore_file(models_path, models_snapshot.as_deref(), false);
                return Err(error);
            }
        }
        Ok(())
    })
}

/// Default-path wrappers used by the RPC handlers.
pub fn mutate_auth(mutation: &AuthMutation) -> Result<(), String> {
    validate_auth_mutation(mutation)?;
    mutate_auth_file(&auth_json_path(), mutation)
}
pub fn upsert_provider(spec: &ProviderUpsertSpec) -> Result<(), String> {
    validate_provider_upsert(spec)?;
    upsert_provider_files(&auth_json_path(), &models_json_path(), spec)
}
pub fn delete_provider(id: &str) -> Result<(), String> {
    if !valid_provider_id(id.trim()) {
        return Err("provider id must use lowercase letters, digits, '-' or '_'".to_string());
    }
    delete_provider_files(&auth_json_path(), &models_json_path(), id)
}

/// Read a consistent Agent-owned snapshot of the two provider configuration
/// documents. The same lock as mutations prevents a view from combining the
/// models half of one revision with the auth half of another.
pub fn read_provider_documents() -> Result<ProviderDocuments, String> {
    with_config_lock(|| {
        Ok((
            read_json_object(&models_json_path())?,
            read_json_object(&auth_json_path())?,
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths(label: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let auth = dir.path().join(format!("{label}-auth.json"));
        let models = dir.path().join(format!("{label}-models.json"));
        (dir, auth, models)
    }

    fn mutation(provider: &str) -> AuthMutation {
        AuthMutation {
            provider: provider.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn set_key_creates_entry_with_type_default() {
        let mut auth = Map::new();
        let m = AuthMutation {
            key: Some("sk-123".to_string()),
            ..mutation("openai")
        };
        apply_auth_mutation(&mut auth, &m);
        let entry = auth["openai"].as_object().unwrap();
        assert_eq!(entry["key"], json!("sk-123"));
        assert_eq!(entry["type"], json!("api_key"));
    }

    #[test]
    fn future_login_sets_key_and_base_url_preserving_others() {
        let mut auth: Map<String, Value> = serde_json::from_str(
            r#"{"future":{"type":"api_key","base_url":"https://old.example.com/api"},"zai":{"type":"api_key","key":"keep"}}"#,
        )
        .unwrap();
        let m = AuthMutation {
            key: Some("new-key".to_string()),
            base_url: Some("https://future-os.cn/api".to_string()),
            ..mutation("future")
        };
        apply_auth_mutation(&mut auth, &m);
        assert_eq!(auth["future"]["key"], json!("new-key"));
        assert_eq!(
            auth["future"]["base_url"],
            json!("https://future-os.cn/api")
        );
        assert_eq!(auth["zai"]["key"], json!("keep"));
    }

    #[test]
    fn clear_key_only_touches_existing_entry() {
        let mut auth: Map<String, Value> = serde_json::from_str(
            r#"{"future":{"type":"api_key","key":"k","base_url":"https://x/api"}}"#,
        )
        .unwrap();
        apply_auth_mutation(
            &mut auth,
            &AuthMutation {
                clear_key: true,
                ..mutation("future")
            },
        );
        assert!(auth["future"].get("key").is_none());
        assert_eq!(auth["future"]["base_url"], json!("https://x/api"));

        // A clear on a missing entry must not create one.
        apply_auth_mutation(
            &mut auth,
            &AuthMutation {
                clear_key: true,
                ..mutation("ghost")
            },
        );
        assert!(auth.get("ghost").is_none());
    }

    #[test]
    fn env_switch_pins_base_url_and_drops_key_and_platform_base_url() {
        let mut auth: Map<String, Value> = serde_json::from_str(
            r#"{"future":{"type":"api_key","key":"old","base_url":"https://future-os.cn/api","platform_base_url":"https://future-os.cn"}}"#,
        )
        .unwrap();
        let m = AuthMutation {
            base_url: Some("https://test.future-os.cn/api".to_string()),
            clear_key: true,
            remove_platform_base_url: true,
            ..mutation("future")
        };
        apply_auth_mutation(&mut auth, &m);
        let future = auth["future"].as_object().unwrap();
        assert_eq!(future["base_url"], json!("https://test.future-os.cn/api"));
        assert!(future.get("key").is_none());
        assert!(future.get("platform_base_url").is_none());
    }

    #[test]
    fn remove_entry_drops_whole_provider() {
        let mut auth: Map<String, Value> =
            serde_json::from_str(r#"{"dashscope":{"type":"api_key","key":"k"}}"#).unwrap();
        apply_auth_mutation(
            &mut auth,
            &AuthMutation {
                remove_entry: true,
                ..mutation("dashscope")
            },
        );
        assert!(auth.get("dashscope").is_none());
    }

    #[test]
    fn upsert_creates_and_merges_preserving_extra_fields() {
        let mut root: Map<String, Value> = serde_json::from_str(
            r#"{"providers":{"myprov":{"baseUrl":"https://old","compat":"strict"}}}"#,
        )
        .unwrap();
        let spec = ProviderUpsertSpec {
            id: "myprov".to_string(),
            name: Some("My Provider".to_string()),
            api: Some("openai-completions".to_string()),
            base_url: Some("https://new.example.com".to_string()),
            models: vec![ProviderModelSpec {
                id: "m1".to_string(),
                name: "Model One".to_string(),
                modalities: vec!["text".to_string(), "image".to_string()],
                context_window: 128000,
                max_tokens: 16384,
            }],
            ..Default::default()
        };
        apply_provider_upsert(&mut root, &spec).unwrap();
        let provider = &root["providers"]["myprov"];
        assert_eq!(provider["name"], json!("My Provider"));
        assert_eq!(provider["baseUrl"], json!("https://new.example.com"));
        assert_eq!(provider["compat"], json!("strict"), "unmanaged fields kept");
        assert_eq!(provider["models"][0]["modalities"][1], json!("image"));
        assert_eq!(provider["models"][0]["contextWindow"], json!(128000));
        assert_eq!(provider["models"][0]["maxTokens"], json!(16384));
    }

    #[test]
    fn explicit_empty_models_replaces_the_existing_list() {
        let mut root: Map<String, Value> = serde_json::from_str(
            r#"{"providers":{"myprov":{"name":"My","models":[{"id":"old"}]}}}"#,
        )
        .unwrap();
        apply_provider_upsert(
            &mut root,
            &ProviderUpsertSpec {
                id: "myprov".to_string(),
                replace_models: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(root["providers"]["myprov"]["models"], json!([]));
    }

    #[test]
    fn auth_only_upsert_does_not_create_models_file() {
        let (_dir, auth, models) = temp_paths("auth-only-upsert");
        upsert_provider_files(
            &auth,
            &models,
            &ProviderUpsertSpec {
                id: "deepseek".to_string(),
                api_key: Some("sk-new".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!models.exists());
        let stored: Value = serde_json::from_str(&std::fs::read_to_string(auth).unwrap()).unwrap();
        assert_eq!(stored["deepseek"]["key"], json!("sk-new"));
    }

    #[test]
    fn builtin_url_and_key_are_committed_by_one_upsert() {
        let (_dir, auth, models) = temp_paths("builtin-atomic");
        upsert_provider_files(
            &auth,
            &models,
            &ProviderUpsertSpec {
                id: "azure-openai-responses".to_string(),
                base_url: Some("https://tenant.openai.azure.com/openai".to_string()),
                api_key: Some("azure-key".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        let stored_models: Value =
            serde_json::from_str(&std::fs::read_to_string(&models).unwrap()).unwrap();
        let stored_auth: Value =
            serde_json::from_str(&std::fs::read_to_string(&auth).unwrap()).unwrap();
        assert_eq!(
            stored_models["providers"]["azure-openai-responses"]["baseUrl"],
            json!("https://tenant.openai.azure.com/openai")
        );
        assert_eq!(
            stored_auth["azure-openai-responses"]["key"],
            json!("azure-key")
        );

        upsert_provider_files(
            &auth,
            &models,
            &ProviderUpsertSpec {
                id: "azure-openai-responses".to_string(),
                base_url: Some("https://other.openai.azure.com/openai".to_string()),
                clear_api_key: true,
                ..Default::default()
            },
        )
        .unwrap();
        let stored_models: Value =
            serde_json::from_str(&std::fs::read_to_string(models).unwrap()).unwrap();
        let stored_auth: Value =
            serde_json::from_str(&std::fs::read_to_string(auth).unwrap()).unwrap();
        assert_eq!(
            stored_models["providers"]["azure-openai-responses"]["baseUrl"],
            json!("https://other.openai.azure.com/openai")
        );
        assert!(stored_auth["azure-openai-responses"].get("key").is_none());
    }

    #[test]
    fn upsert_create_only_rejects_existing_id() {
        let mut root: Map<String, Value> =
            serde_json::from_str(r#"{"providers":{"myprov":{"name":"x"}}}"#).unwrap();
        let spec = ProviderUpsertSpec {
            id: "myprov".to_string(),
            create_only: true,
            ..Default::default()
        };
        let error = apply_provider_upsert(&mut root, &spec).unwrap_err();
        assert!(error.contains("already exists"));
    }

    #[test]
    fn upsert_rejects_duplicate_name() {
        let mut root: Map<String, Value> =
            serde_json::from_str(r#"{"providers":{"a":{"name":"Shared"}}}"#).unwrap();
        let spec = ProviderUpsertSpec {
            id: "b".to_string(),
            name: Some("shared".to_string()),
            ..Default::default()
        };
        let error = apply_provider_upsert(&mut root, &spec).unwrap_err();
        assert!(error.contains("already exists"));
    }

    #[test]
    fn clear_base_url_drops_override_and_empty_entry() {
        let mut root: Map<String, Value> =
            serde_json::from_str(r#"{"providers":{"openai":{"baseUrl":"https://override"}}}"#)
                .unwrap();
        let spec = ProviderUpsertSpec {
            id: "openai".to_string(),
            clear_base_url: true,
            ..Default::default()
        };
        apply_provider_upsert(&mut root, &spec).unwrap();
        assert!(root["providers"].get("openai").is_none());

        // With other fields present the entry survives without baseUrl.
        let mut root: Map<String, Value> = serde_json::from_str(
            r#"{"providers":{"myprov":{"name":"x","baseUrl":"https://override"}}}"#,
        )
        .unwrap();
        let spec = ProviderUpsertSpec {
            id: "myprov".to_string(),
            clear_base_url: true,
            ..Default::default()
        };
        apply_provider_upsert(&mut root, &spec).unwrap();
        assert_eq!(root["providers"]["myprov"], json!({"name": "x"}));
    }

    #[test]
    fn mutate_auth_file_roundtrip_and_corrupt_guard() {
        let (_dir, auth, _models) = temp_paths("mutate");
        mutate_auth_file(
            &auth,
            &AuthMutation {
                key: Some("k1".to_string()),
                ..mutation("future")
            },
        )
        .unwrap();
        mutate_auth_file(
            &auth,
            &AuthMutation {
                base_url: Some("https://p/api".to_string()),
                ..mutation("future")
            },
        )
        .unwrap();
        let stored: Value = serde_json::from_str(&std::fs::read_to_string(&auth).unwrap()).unwrap();
        assert_eq!(stored["future"]["key"], json!("k1"));
        assert_eq!(stored["future"]["base_url"], json!("https://p/api"));

        // Corrupt file: the mutation must fail, not clobber.
        std::fs::write(&auth, "{ not json").unwrap();
        let result = mutate_auth_file(
            &auth,
            &AuthMutation {
                key: Some("k2".to_string()),
                ..mutation("future")
            },
        );
        assert!(result.is_err());
        assert_eq!(std::fs::read_to_string(&auth).unwrap(), "{ not json");
    }

    #[cfg(unix)]
    #[test]
    fn auth_file_is_owner_only_models_is_not() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, auth, models) = temp_paths("perms");
        mutate_auth_file(
            &auth,
            &AuthMutation {
                key: Some("k".to_string()),
                ..mutation("p")
            },
        )
        .unwrap();
        upsert_provider_files(
            &auth,
            &models,
            &ProviderUpsertSpec {
                id: "p".to_string(),
                name: Some("P".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        let mode = std::fs::metadata(&auth).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let mode = std::fs::metadata(&models).unwrap().permissions().mode() & 0o777;
        assert_ne!(mode, 0o600);
    }

    #[test]
    fn upsert_files_writes_key_and_models_together() {
        let (_dir, auth, models) = temp_paths("upsert");
        let spec = ProviderUpsertSpec {
            id: "myprov".to_string(),
            name: Some("My".to_string()),
            api: Some("anthropic".to_string()),
            base_url: Some("https://api.example.com".to_string()),
            api_key: Some("sk-key".to_string()),
            models: vec![ProviderModelSpec {
                id: "m1".to_string(),
                name: "m1".to_string(),
                modalities: vec!["text".to_string()],
                context_window: 128000,
                max_tokens: 16384,
            }],
            ..Default::default()
        };
        upsert_provider_files(&auth, &models, &spec).unwrap();

        let auth_doc: Value =
            serde_json::from_str(&std::fs::read_to_string(&auth).unwrap()).unwrap();
        assert_eq!(auth_doc["myprov"]["key"], json!("sk-key"));
        let models_doc: Value =
            serde_json::from_str(&std::fs::read_to_string(&models).unwrap()).unwrap();
        assert_eq!(models_doc["providers"]["myprov"]["name"], json!("My"));
    }

    #[test]
    fn delete_files_removes_both_entries() {
        let (_dir, auth, models) = temp_paths("delete");
        let spec = ProviderUpsertSpec {
            id: "myprov".to_string(),
            name: Some("My".to_string()),
            api_key: Some("sk".to_string()),
            ..Default::default()
        };
        upsert_provider_files(&auth, &models, &spec).unwrap();
        delete_provider_files(&auth, &models, "myprov").unwrap();

        let auth_doc: Value =
            serde_json::from_str(&std::fs::read_to_string(&auth).unwrap()).unwrap();
        assert!(auth_doc.get("myprov").is_none());
        let models_doc: Value =
            serde_json::from_str(&std::fs::read_to_string(&models).unwrap()).unwrap();
        assert!(models_doc["providers"].get("myprov").is_none());

        // Deleting again is a no-op.
        delete_provider_files(&auth, &models, "myprov").unwrap();
    }

    #[test]
    fn snapshot_restore_roundtrips_exact_bytes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("cfg.json");

        // A missing file snapshots to Ok(None).
        assert_eq!(snapshot_file(&path).unwrap(), None);

        // Non-alphabetical key order: restore must return the exact bytes, not
        // a re-serialization (which could reorder/reformat them).
        let content = "{\n  \"b\": 1,\n  \"a\": 2\n}\n";
        std::fs::write(&path, content).unwrap();
        let snap = snapshot_file(&path).unwrap();
        assert_eq!(snap.as_deref(), Some(content.as_bytes()));

        std::fs::write(&path, "mutated").unwrap();
        restore_file(&path, snap.as_deref(), false);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content);

        // Restoring a None snapshot removes the file entirely.
        restore_file(&path, None, false);
        assert!(!path.exists());
    }

    #[test]
    fn upsert_snapshot_read_error_aborts_without_touching_files() {
        let (_dir, auth, models) = temp_paths("upsert-snapshot-err");
        // Pre-existing models.json content that must survive the aborted upsert.
        let original =
            "{\n  \"providers\": {\n    \"keep\": {\n      \"name\": \"Keep\"\n    }\n  }\n}\n";
        std::fs::write(&models, original).unwrap();
        // Making auth a directory makes its snapshot read fail (not NotFound) —
        // the upsert must abort BEFORE any write, not treat it as "no file".
        std::fs::create_dir_all(&auth).unwrap();

        let spec = ProviderUpsertSpec {
            id: "newprov".to_string(),
            name: Some("New".to_string()),
            api_key: Some("sk-x".to_string()),
            ..Default::default()
        };
        assert!(upsert_provider_files(&auth, &models, &spec).is_err());

        // No partial "newprov" entry, and the models file is untouched.
        assert_eq!(std::fs::read_to_string(&models).unwrap(), original);
    }

    #[test]
    fn delete_no_change_leaves_files_untouched() {
        let (_dir, auth, models) = temp_paths("delete-noop");
        let original_models =
            "{\n  \"providers\": {\n    \"keep\": {\n      \"name\": \"Keep\"\n    }\n  }\n}\n";
        std::fs::write(&models, original_models).unwrap();
        std::fs::write(&auth, "{}\n").unwrap();

        // Deleting an unknown provider touches neither file.
        delete_provider_files(&auth, &models, "ghost").unwrap();
        assert_eq!(std::fs::read_to_string(&models).unwrap(), original_models);
        assert_eq!(std::fs::read_to_string(&auth).unwrap(), "{}\n");
    }

    #[test]
    fn read_json_object_rejects_non_object_document() {
        let (_dir, auth, _models) = temp_paths("nonobject");
        std::fs::write(&auth, "[1, 2, 3]\n").unwrap();
        let error = read_json_object(&auth).unwrap_err();
        assert!(error.contains("does not contain a JSON object"), "{error}");
    }

    #[test]
    fn write_atomic_fails_when_parent_is_a_file() {
        let (_dir, auth, _models) = temp_paths("parent-file");
        std::fs::write(&auth, "{}\n").unwrap(); // parent-to-be is a FILE
        let blocked = auth.join("config.json");
        let error = write_bytes_atomic(&blocked, b"{}", false).unwrap_err();
        assert!(error.contains("failed to create"), "{error}");
    }

    #[test]
    fn write_atomic_cleans_up_tmp_when_rename_fails() {
        let (_dir, _auth, _models) = temp_paths("rename-fails");
        // Renaming the temp file onto an existing DIRECTORY fails (EISDIR),
        // even for root — no permission bits involved.
        let target = _dir.path().join("as-directory");
        std::fs::create_dir(&target).unwrap();
        let error = write_bytes_atomic(&target, b"{}", false).unwrap_err();
        assert!(error.contains("failed to write"), "{error}");
        // The temp file was cleaned up.
        let leftovers = std::fs::read_dir(_dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn snapshot_file_reports_non_notfound_read_errors() {
        let (_dir, _auth, _models) = temp_paths("snapshot-dir");
        let error = snapshot_file(_dir.path()).unwrap_err();
        assert!(error.contains("failed to read"), "{error}");
    }

    #[test]
    fn restore_file_logs_when_rollback_remove_fails() {
        let (_dir, auth, _models) = temp_paths("restore-missing");
        // snapshot None → rollback removes the file; it never existed, so the
        // removal fails and the warning path runs (stderr, non-fatal).
        restore_file(&auth, None, false);
    }

    #[test]
    fn apply_auth_mutation_clears_urls_on_existing_entry() {
        let mut auth: Map<String, Value> = serde_json::from_str(
            r#"{"future":{"type":"api_key","base_url":"https://x/api","platform_base_url":"https://p/api"}}"#,
        )
        .unwrap();
        apply_auth_mutation(
            &mut auth,
            &AuthMutation {
                clear_base_url: true,
                remove_platform_base_url: true,
                ..mutation("future")
            },
        );
        assert!(auth["future"].get("base_url").is_none());
        assert!(auth["future"].get("platform_base_url").is_none());
    }

    #[test]
    fn apply_auth_mutation_upsert_clears_url_fields() {
        let mut auth: Map<String, Value> = serde_json::from_str(
            r#"{"future":{"type":"api_key","base_url":"https://x/api","platform_base_url":"https://p/api"}}"#,
        )
        .unwrap();
        apply_auth_mutation(
            &mut auth,
            &AuthMutation {
                key: Some("k".to_string()),
                clear_base_url: true,
                remove_platform_base_url: true,
                ..mutation("future")
            },
        );
        assert_eq!(auth["future"]["key"], json!("k"));
        assert!(auth["future"].get("base_url").is_none());
        assert!(auth["future"].get("platform_base_url").is_none());
    }

    #[test]
    fn upsert_auth_entry_normalizes_non_object_entry() {
        let mut auth: Map<String, Value> =
            serde_json::from_str(r#"{"openai":"not-an-object"}"#).unwrap();
        apply_auth_mutation(
            &mut auth,
            &AuthMutation {
                key: Some("k".to_string()),
                ..mutation("openai")
            },
        );
        assert_eq!(auth["openai"]["key"], json!("k"));
        assert_eq!(auth["openai"]["type"], json!("api_key"));
    }

    #[cfg(unix)]
    fn skip_if_root() -> bool {
        // Permission-bit failure injection does not work for root.
        unsafe { libc::geteuid() == 0 }
    }

    #[cfg(unix)]
    fn make_readonly(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o555)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn upsert_restores_models_when_models_write_fails() {
        // Permission-bit injection does not work for root; `then` with a fn
        // item keeps the skip edge branchless (a dead `if` closing brace or
        // uncalled closure line would linger in per-line coverage).
        let _ = (!skip_if_root()).then(upsert_models_write_fails_body);
    }

    #[cfg(unix)]
    fn upsert_models_write_fails_body() {
        let (_dir, auth, _models) = temp_paths("upsert-models-fail");
        let readonly = _dir.path().join("readonly");
        std::fs::create_dir(&readonly).unwrap();
        let models = readonly.join("models.json");
        std::fs::write(&models, "{}\n").unwrap();
        make_readonly(&readonly);
        let spec = ProviderUpsertSpec {
            id: "newprov".to_string(),
            name: Some("New".to_string()),
            ..Default::default()
        };
        let error = upsert_provider_files(&auth, &models, &spec).unwrap_err();
        assert!(error.contains("failed to write"), "{error}");
        // Rollback could not restore into the read-only dir either (logged).
        assert_eq!(std::fs::read_to_string(&models).unwrap(), "{}\n");
    }

    #[cfg(unix)]
    #[test]
    fn upsert_restores_models_when_auth_write_fails() {
        let _ = (!skip_if_root()).then(upsert_auth_write_fails_body);
    }

    #[cfg(unix)]
    fn upsert_auth_write_fails_body() {
        let (_dir, _auth, _models) = temp_paths("upsert-auth-fail");
        let readonly = _dir.path().join("readonly");
        std::fs::create_dir(&readonly).unwrap();
        let auth = readonly.join("auth.json");
        std::fs::write(&auth, "{}\n").unwrap();
        let models = _dir.path().join("models.json");
        std::fs::write(&models, "{}\n").unwrap();
        make_readonly(&readonly);
        let spec = ProviderUpsertSpec {
            id: "newprov".to_string(),
            name: Some("New".to_string()),
            api_key: Some("sk-x".to_string()),
            ..Default::default()
        };
        let error = upsert_provider_files(&auth, &models, &spec).unwrap_err();
        assert!(error.contains("failed to write"), "{error}");
        // models.json was rolled back to its original content.
        assert_eq!(std::fs::read_to_string(&models).unwrap(), "{}\n");
    }

    #[test]
    fn delete_touches_only_the_file_that_changed() {
        // Provider only in models.json → auth write skipped entirely.
        let (_dir, auth, models) = temp_paths("delete-models-only");
        std::fs::write(
            &models,
            r#"{"providers":{"gone":{"name":"Gone"}}}"#.to_string() + "\n",
        )
        .unwrap();
        std::fs::write(&auth, "{}\n").unwrap();
        delete_provider_files(&auth, &models, "gone").unwrap();
        assert!(!std::fs::read_to_string(&models).unwrap().contains("gone"));
        assert_eq!(std::fs::read_to_string(&auth).unwrap(), "{}\n");

        // Provider only in auth.json → models write skipped entirely.
        let (_dir2, auth2, models2) = temp_paths("delete-auth-only");
        std::fs::write(&models2, "{}\n").unwrap();
        std::fs::write(&auth2, r#"{"gone":{"type":"api_key","key":"k"}}"#).unwrap();
        delete_provider_files(&auth2, &models2, "gone").unwrap();
        assert_eq!(std::fs::read_to_string(&models2).unwrap(), "{}\n");
        assert!(!std::fs::read_to_string(&auth2).unwrap().contains("gone"));
    }

    #[cfg(unix)]
    #[test]
    fn delete_restores_models_when_models_write_fails() {
        let _ = (!skip_if_root()).then(delete_models_write_fails_body);
    }

    #[cfg(unix)]
    fn delete_models_write_fails_body() {
        let (_dir, auth, _m) = temp_paths("delete-models-fail");
        let readonly = _dir.path().join("readonly");
        std::fs::create_dir(&readonly).unwrap();
        let models = readonly.join("models.json");
        std::fs::write(&models, "{\"providers\":{\"gone\":{\"name\":\"G\"}}}\n").unwrap();
        std::fs::write(&auth, "{}\n").unwrap();
        make_readonly(&readonly);
        let error = delete_provider_files(&auth, &models, "gone").unwrap_err();
        assert!(error.contains("failed to write"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn delete_restores_models_when_auth_write_fails() {
        let _ = (!skip_if_root()).then(delete_auth_write_fails_body);
    }

    #[cfg(unix)]
    fn delete_auth_write_fails_body() {
        let (_dir, _a, _m) = temp_paths("delete-auth-fail");
        let readonly = _dir.path().join("readonly");
        std::fs::create_dir(&readonly).unwrap();
        let auth = readonly.join("auth.json");
        std::fs::write(&auth, "{\"gone\":{\"type\":\"api_key\"}}\n").unwrap();
        let models = _dir.path().join("models.json");
        std::fs::write(&models, "{\"providers\":{\"gone\":{\"name\":\"G\"}}}\n").unwrap();
        make_readonly(&readonly);
        let error = delete_provider_files(&auth, &models, "gone").unwrap_err();
        assert!(error.contains("failed to write"), "{error}");
        // models.json rolled back: the provider entry is present again.
        assert!(std::fs::read_to_string(&models).unwrap().contains("gone"));
    }

    #[test]
    fn write_atomic_handles_parentless_path() {
        // "/" has no parent: the create_dir_all guard is skipped entirely.
        let error = write_bytes_atomic(std::path::Path::new("/"), b"{}", false).unwrap_err();
        assert!(error.contains("failed to write"), "{error}");
    }
}
