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
    /// Non-empty: replace the `models` list.
    pub models: Vec<ProviderModelSpec>,
    /// Fail when the provider already exists (create mode).
    pub create_only: bool,
    /// Non-empty: also store as this provider's `auth.json` key.
    pub api_key: Option<String>,
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
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    let serialized = format!(
        "{}\n",
        serde_json::to_string_pretty(&Value::Object(map.clone()))
            .map_err(|error| format!("failed to serialize config: {error}"))?
    );
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
            std::io::Write::write_all(&mut file, serialized.as_bytes())?;
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
    if !spec.models.is_empty() {
        let models = spec
            .models
            .iter()
            .map(|model| {
                json!({
                    "id": model.id,
                    "name": model.name,
                    "modalities": model.modalities,
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

/// Create/update a provider across both files. The API key is written AFTER
/// the in-memory models.json validation passes but BEFORE the models.json
/// write (mirroring the GUI): a models.json write failure then rolls the
/// orphaned key back for a brand-new provider, so auth.json never accumulates
/// dangling credentials.
pub fn upsert_provider_files(
    auth_path: &Path,
    models_path: &Path,
    spec: &ProviderUpsertSpec,
) -> Result<(), String> {
    with_config_lock(|| {
        let mut models_doc = read_json_object(models_path)?;
        let provider_existed = models_doc
            .get("providers")
            .and_then(Value::as_object)
            .map(|providers| providers.contains_key(&spec.id))
            .unwrap_or(false);

        // Validate + apply in memory first; nothing is written if this fails.
        apply_provider_upsert(&mut models_doc, spec)?;

        if let Some(key) = &spec.api_key {
            let mut auth = read_json_object(auth_path)?;
            upsert_auth_entry(&mut auth, &spec.id)
                .insert("key".to_string(), Value::String(key.clone()));
            write_json_atomic(auth_path, &auth, true)?;
        }

        if let Err(error) = write_json_atomic(models_path, &models_doc, false) {
            if !provider_existed && spec.api_key.is_some() {
                // Roll back the orphaned key from the brand-new provider.
                if let Ok(mut auth) = read_json_object(auth_path) {
                    if auth.remove(&spec.id).is_some() {
                        let _ = write_json_atomic(auth_path, &auth, true);
                    }
                }
            }
            return Err(error);
        }
        Ok(())
    })
}

/// Remove a provider's models.json entry AND its auth.json entry.
pub fn delete_provider_files(auth_path: &Path, models_path: &Path, id: &str) -> Result<(), String> {
    with_config_lock(|| {
        let mut models_doc = read_json_object(models_path)?;
        if apply_provider_delete(&mut models_doc, id) {
            write_json_atomic(models_path, &models_doc, false)?;
        }
        let mut auth = read_json_object(auth_path)?;
        if auth.remove(id).is_some() {
            write_json_atomic(auth_path, &auth, true)?;
        }
        Ok(())
    })
}

/// Default-path wrappers used by the RPC handlers.
pub fn mutate_auth(mutation: &AuthMutation) -> Result<(), String> {
    mutate_auth_file(&auth_json_path(), mutation)
}
pub fn upsert_provider(spec: &ProviderUpsertSpec) -> Result<(), String> {
    upsert_provider_files(&auth_json_path(), &models_json_path(), spec)
}
pub fn delete_provider(id: &str) -> Result<(), String> {
    delete_provider_files(&auth_json_path(), &models_json_path(), id)
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
            }],
            ..Default::default()
        };
        apply_provider_upsert(&mut root, &spec).unwrap();
        let provider = &root["providers"]["myprov"];
        assert_eq!(provider["name"], json!("My Provider"));
        assert_eq!(provider["baseUrl"], json!("https://new.example.com"));
        assert_eq!(provider["compat"], json!("strict"), "unmanaged fields kept");
        assert_eq!(provider["models"][0]["modalities"][1], json!("image"));
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
}
