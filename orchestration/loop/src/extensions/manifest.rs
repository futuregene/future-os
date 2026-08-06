//! Extension manifest (G-21) — LoopX `extensions/manifest.py`, natively.
//!
//! v1 is DECLARATIVE ONLY (security tradeoff from the P3 plan): a manifest
//! declares the extension's provider record, the capabilities it `provides`
//! and the capability implementations it `implements`. No native code is
//! loaded (no dlopen / subprocess execution of extension code) — that is the
//! P4 process-runtime concern. LoopX reads TOML manifests; this Rust-native
//! implementation reads the same schema as JSON (`loopx_extension_manifest_v0`),
//! keeping the field names and validation rules identical.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const EXTENSION_MANIFEST_SCHEMA_VERSION: &str = "loopx_extension_manifest_v0";
/// LoopX LOOPX_EXTENSION_API_VERSION.
pub const LOOPX_EXTENSION_API_VERSION: u32 = 1;
/// The versioned lower-snake protocol token (LoopX _PROTOCOL_RE).
pub const PROTOCOL_TOKEN_RE: &str = "^[a-z][a-z0-9_]{0,63}_v\\d+$";

/// A capability a manifest provides (LoopX manifest `[[provides]]`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestProvidedCapability {
    pub id: String,
    pub kind: String,
    pub visibility: String,
}

/// A capability implementation a manifest implements (LoopX `[[implements]]`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestImplementation {
    pub capability_id: String,
    pub protocol: String,
}

/// Declarative runtime contract (LoopX `_runtime_contract`). v1 validates the
/// contract shape + permissions; the executable entrypoint is resolved by the
/// readiness doctor (no code is executed by the manifest loader).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestRuntime {
    pub protocol: String,
    /// Either `entrypoint` (command path) or `python_module` — exactly one.
    pub entrypoint: Option<String>,
    pub python_module: Option<String>,
    pub args: Vec<String>,
    pub doctor_args: Vec<String>,
    pub required_permissions: Vec<String>,
    pub timeout_seconds: u32,
}

/// A parsed extension manifest (LoopX `load_extension_manifest` return shape).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionManifest {
    pub schema_version: String,
    pub provider: ManifestProvider,
    pub capabilities: Vec<ManifestProvidedCapability>,
    pub implementations: Vec<ManifestImplementation>,
    pub runtime: Option<ManifestRuntime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestProvider {
    pub id: String,
    pub origin: String,
    pub declared: bool,
    pub installed: bool,
    pub enabled: bool,
    pub ready: bool,
    pub version: String,
    pub requires_loopx_api: String,
    pub permissions: Vec<String>,
}

fn required_string(value: &serde_json::Value, key: &str, context: &str) -> Result<String, String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{context} requires non-empty string `{key}`"))
}

fn string_list(value: &serde_json::Value, key: &str, context: &str) -> Result<Vec<String>, String> {
    let items = value
        .get(key)
        .cloned()
        .unwrap_or(serde_json::Value::Array(vec![]));
    let arr = items
        .as_array()
        .ok_or_else(|| format!("{context} requires `{key}` to be an array of strings"))?;
    let mut out = vec![];
    for item in arr {
        let s = item
            .as_str()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("{context} requires `{key}` to be an array of strings"))?;
        out.push(s);
    }
    Ok(out)
}

/// LoopX `_require_compatible_loopx_api`: clauses like `>=1,<2` compared
/// against the runtime API version. Fail closed on incompatible requirements.
pub fn require_compatible_loopx_api(requirement: &str) -> Result<(), String> {
    let clauses: Vec<&str> = requirement
        .split(',')
        .map(|c| c.trim())
        .filter(|c| !c.is_empty())
        .collect();
    if clauses.is_empty() {
        return Err(format!("invalid `requires_loopx_api` `{requirement}`"));
    }
    for clause in clauses {
        let (op, num) = if let Some(rest) = clause.strip_prefix(">=") {
            (">=", rest)
        } else if let Some(rest) = clause.strip_prefix("<=") {
            ("<=", rest)
        } else if let Some(rest) = clause.strip_prefix("==") {
            ("==", rest)
        } else if let Some(rest) = clause.strip_prefix('>') {
            (">", rest)
        } else if let Some(rest) = clause.strip_prefix('<') {
            ("<", rest)
        } else {
            ("==", clause)
        };
        let wanted: u32 = num
            .trim()
            .parse()
            .map_err(|_| format!("invalid `requires_loopx_api` clause `{clause}`"))?;
        let ok = match op {
            ">=" => LOOPX_EXTENSION_API_VERSION >= wanted,
            "<=" => LOOPX_EXTENSION_API_VERSION <= wanted,
            ">" => LOOPX_EXTENSION_API_VERSION > wanted,
            "<" => LOOPX_EXTENSION_API_VERSION < wanted,
            _ => LOOPX_EXTENSION_API_VERSION == wanted,
        };
        if !ok {
            return Err(format!(
                "manifest requires LoopX extension API `{requirement}`, but this runtime provides `{LOOPX_EXTENSION_API_VERSION}`"
            ));
        }
    }
    Ok(())
}

/// Validate a protocol token against the versioned lower-snake rule.
pub fn validate_protocol_token(protocol: &str) -> bool {
    let bytes = protocol.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_lowercase() {
        return false;
    }
    let mut saw_digit_v = false;
    for (i, b) in bytes.iter().enumerate() {
        if !(b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_') {
            return false;
        }
        if i == 0 && !b.is_ascii_lowercase() {
            return false;
        }
        // version suffix: `_v<digits>` at the end
        if *b == b'_' && i + 2 < bytes.len() && bytes[i + 1] == b'v' {
            let rest = &protocol[i + 2..];
            if !rest.is_empty() && rest.bytes().all(|c| c.is_ascii_digit()) {
                saw_digit_v = true;
            }
        }
    }
    if !saw_digit_v {
        return false;
    }
    // ensure it ends with _v<digits>
    let Some(underscore) = protocol.rfind("_v") else {
        return false;
    };
    let suffix = &protocol[underscore + 2..];
    !suffix.is_empty() && suffix.bytes().all(|c| c.is_ascii_digit())
}

/// Load + validate a declarative manifest from JSON, without executing
/// anything. Returns a normalized [`ExtensionManifest`].
pub fn load_extension_manifest(path: &Path) -> Result<ExtensionManifest, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read extension manifest `{}`: {e}", path.display()))?;
    let raw: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("cannot read extension manifest `{}`: {e}", path.display()))?;
    let context = format!("extension manifest `{}`", path.display());
    validate_manifest_value(&raw, &context)
}

/// Validate a raw manifest JSON value (shared with contract tests).
pub fn validate_manifest_value(
    raw: &serde_json::Value,
    context: &str,
) -> Result<ExtensionManifest, String> {
    if !raw.is_object() {
        return Err(format!("{context} must contain a JSON object"));
    }
    let schema_version = required_string(raw, "schema_version", context)?;
    if schema_version != EXTENSION_MANIFEST_SCHEMA_VERSION {
        return Err(format!(
            "{context} has unsupported schema_version `{schema_version}`; expected `{EXTENSION_MANIFEST_SCHEMA_VERSION}`"
        ));
    }
    let extension_id = required_string(raw, "id", context)?;
    let version = required_string(raw, "version", context)?;
    let requires_loopx_api = required_string(raw, "requires_loopx_api", context)?;
    require_compatible_loopx_api(&requires_loopx_api)?;
    let permissions = string_list(raw, "permissions", context)?;

    // Runtime contract (optional).
    let runtime = match raw.get("runtime") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => {
            let rt_context = format!("{context} runtime");
            let protocol = required_string(value, "protocol", &rt_context)?;
            if !validate_protocol_token(&protocol) {
                return Err(format!(
                    "{rt_context} protocol must be a versioned lower-snake token"
                ));
            }
            let entrypoint = value
                .get("entrypoint")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let python_module = value
                .get("python_module")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            if entrypoint.is_none() == python_module.is_none() {
                return Err(format!(
                    "{rt_context} requires exactly one of `entrypoint` or `python_module`"
                ));
            }
            let args = string_list(value, "args", &rt_context)?;
            let doctor_args = string_list(value, "doctor_args", &rt_context)?;
            let required_permissions = string_list(value, "required_permissions", &rt_context)?;
            let undeclared: Vec<&String> = required_permissions
                .iter()
                .filter(|p| !permissions.contains(p))
                .collect();
            if !undeclared.is_empty() {
                return Err(format!(
                    "{rt_context} requires undeclared permissions {undeclared:?}"
                ));
            }
            let timeout_seconds = value
                .get("timeout_seconds")
                .and_then(|v| v.as_u64())
                .unwrap_or(30) as u32;
            if !(1..=120).contains(&timeout_seconds) {
                return Err(format!(
                    "{rt_context} timeout_seconds must be an integer from 1 to 120"
                ));
            }
            Some(ManifestRuntime {
                protocol,
                entrypoint,
                python_module,
                args,
                doctor_args,
                required_permissions,
                timeout_seconds,
            })
        }
    };

    // [[provides]] capabilities.
    let mut capabilities = vec![];
    let provides = raw
        .get("provides")
        .cloned()
        .unwrap_or(serde_json::Value::Array(vec![]));
    let provides = provides
        .as_array()
        .ok_or_else(|| format!("{context} requires `provides` to be an array of objects"))?;
    for (index, item) in provides.iter().enumerate() {
        let item_context = format!("{context} provides[{index}]");
        if !item.is_object() {
            return Err(format!("{item_context} must be an object"));
        }
        capabilities.push(ManifestProvidedCapability {
            id: required_string(item, "id", &item_context)?,
            kind: required_string(item, "kind", &item_context)?,
            visibility: item
                .get("visibility")
                .and_then(|v| v.as_str())
                .unwrap_or("public")
                .trim()
                .to_string(),
        });
    }

    // [[implements]] capability implementations.
    let mut implementations = vec![];
    let implements = raw
        .get("implements")
        .cloned()
        .unwrap_or(serde_json::Value::Array(vec![]));
    let implements = implements
        .as_array()
        .ok_or_else(|| format!("{context} requires `implements` to be an array of objects"))?;
    for (index, item) in implements.iter().enumerate() {
        let item_context = format!("{context} implements[{index}]");
        if !item.is_object() {
            return Err(format!("{item_context} must be an object"));
        }
        let protocol = required_string(item, "protocol", &item_context)?;
        if !validate_protocol_token(&protocol) {
            return Err(format!(
                "{item_context} protocol must be a versioned lower-snake token"
            ));
        }
        if runtime.is_none() {
            return Err(format!("{item_context} requires an executable runtime"));
        }
        if runtime.as_ref().map(|r| r.protocol.as_str()) != Some(protocol.as_str()) {
            return Err(format!(
                "{item_context} protocol must match runtime protocol `{}`",
                runtime.as_ref().map(|r| r.protocol.as_str()).unwrap_or("")
            ));
        }
        implementations.push(ManifestImplementation {
            capability_id: required_string(item, "capability_id", &item_context)?,
            protocol,
        });
    }

    if runtime.is_none() && capabilities.is_empty() && implementations.is_empty() {
        return Err(format!(
            "{context} requires an executable `runtime`, `provides`, or `implements`"
        ));
    }

    Ok(ExtensionManifest {
        schema_version,
        provider: ManifestProvider {
            id: extension_id,
            origin: "extension".to_string(),
            declared: true,
            installed: false,
            enabled: false,
            ready: false,
            version,
            requires_loopx_api,
            permissions,
        },
        capabilities,
        implementations,
        runtime,
    })
}

/// Locate a bundled example manifest (used by CLI/tests); returns a path for
/// a temp file. Callers own the file lifetime.
pub fn write_example_manifest(root: &Path, extension_id: &str) -> PathBuf {
    let path = root.join(format!("{extension_id}.json"));
    let json = serde_json::json!({
        "schema_version": EXTENSION_MANIFEST_SCHEMA_VERSION,
        "id": extension_id,
        "version": "1.0.0",
        "requires_loopx_api": ">=1",
        "permissions": ["shell"],
        "runtime": {
            "protocol": "command_json_v0",
            "entrypoint": "echo",
            "args": [],
            "doctor_args": ["--version"],
            "required_permissions": ["shell"],
            "timeout_seconds": 30
        },
        "provides": [
            {
                "id": format!("{extension_id}_capability"),
                "kind": "domain_rule",
                "visibility": "public"
            }
        ],
        "implements": [
            {
                "capability_id": format!("{extension_id}_capability"),
                "protocol": "command_json_v0"
            }
        ]
    });
    std::fs::write(&path, serde_json::to_string_pretty(&json).unwrap()).unwrap();
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_manifest() -> serde_json::Value {
        serde_json::json!({
            "schema_version": EXTENSION_MANIFEST_SCHEMA_VERSION,
            "id": "ext-demo",
            "version": "1.0.0",
            "requires_loopx_api": ">=1,<3",
            "permissions": ["shell", "network"],
            "runtime": {
                "protocol": "command_json_v0",
                "entrypoint": "/bin/echo",
                "args": [],
                "doctor_args": ["--version"],
                "required_permissions": ["shell"],
                "timeout_seconds": 30
            },
            "provides": [{"id": "ext-cap", "kind": "domain_rule", "visibility": "public"}],
            "implements": [{"capability_id": "ext-cap", "protocol": "command_json_v0"}]
        })
    }

    #[test]
    fn parses_valid_manifest() {
        let manifest = validate_manifest_value(&valid_manifest(), "test").unwrap();
        assert_eq!(manifest.provider.id, "ext-demo");
        assert_eq!(manifest.provider.origin, "extension");
        assert!(!manifest.provider.installed);
        assert_eq!(manifest.capabilities.len(), 1);
        assert_eq!(manifest.implementations.len(), 1);
        let runtime = manifest.runtime.unwrap();
        assert_eq!(runtime.protocol, "command_json_v0");
    }

    #[test]
    fn incompatible_api_fails_closed() {
        let mut m = valid_manifest();
        m["requires_loopx_api"] = serde_json::json!(">=99");
        assert!(validate_manifest_value(&m, "test").is_err());
    }

    #[test]
    fn bad_protocol_token_rejected() {
        let mut m = valid_manifest();
        m["runtime"]["protocol"] = serde_json::json!("Upper_Case_v0");
        assert!(validate_manifest_value(&m, "test").is_err());
        m = valid_manifest();
        m["runtime"]["protocol"] = serde_json::json!("no_version_suffix");
        assert!(validate_manifest_value(&m, "test").is_err());
    }

    #[test]
    fn undeclared_runtime_permission_rejected() {
        let mut m = valid_manifest();
        m["runtime"]["required_permissions"] = serde_json::json!(["network"]);
        m["permissions"] = serde_json::json!(["shell"]);
        assert!(validate_manifest_value(&m, "test").is_err());
    }

    #[test]
    fn empty_manifest_rejected() {
        assert!(validate_manifest_value(&serde_json::json!({}), "test").is_err());
    }

    #[test]
    fn runtime_requires_exactly_one_entrypoint_kind() {
        let mut m = valid_manifest();
        m["runtime"].as_object_mut().unwrap().remove("entrypoint");
        m["runtime"]
            .as_object_mut()
            .unwrap()
            .remove("python_module");
        assert!(validate_manifest_value(&m, "test").is_err());
    }

    #[test]
    fn declarative_only_without_runtime_is_allowed() {
        let mut m = valid_manifest();
        m.as_object_mut().unwrap().remove("runtime");
        m.as_object_mut().unwrap().remove("implements");
        let manifest = validate_manifest_value(&m, "test").unwrap();
        assert!(manifest.runtime.is_none());
        assert!(manifest.implementations.is_empty());
    }
}
