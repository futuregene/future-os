//! `future auth` — 1:1 port of cli/src/commands/auth.ts.
//!
//! login (device-code OAuth), status, credential, logout, plus the shared
//! auth.json helpers (loadAuthFile / writeAuthFile / getFutureAuthEntry)
//! used by account.ts and doctor.ts.

use crate::constants::{auth_file, DEFAULT_PLATFORM_URL, FUTURE_AUTH_PROVIDER};
use crate::output::Output;
use crate::utils::platform::get_platform_url;
use crate::utils::string::trim_trailing_slash;
use crate::utils::time::sleep;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::path::PathBuf;

/// `DeviceCodeResponse` from auth.ts.
#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    #[serde(default)]
    device_code: String,
    #[serde(default)]
    user_code: String,
    #[serde(default)]
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: String,
    #[serde(default)]
    expires_in: u64,
    #[serde(default)]
    interval: u64,
}

/// `DeviceTokenResponse` from auth.ts.
#[derive(Debug, Deserialize)]
struct DeviceTokenResponse {
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    #[allow(dead_code)]
    api_key_id: String,
    #[serde(default)]
    #[allow(dead_code)]
    token_type: String,
}

/// `DeviceErrorResponse` from auth.ts.
#[derive(Debug, Deserialize)]
struct DeviceErrorResponse {
    #[serde(default)]
    error: String,
    #[serde(default)]
    message: String,
}

/// `FutureAuthEntry` from auth.ts — the sanitized `future` provider entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FutureAuthEntry {
    pub type_: Option<String>,
    pub key: Option<String>,
    pub base_url: Option<String>,
}

/// `login(platformUrlOverride?)` — device-code OAuth flow.
pub async fn login(platform_url_override: Option<String>, out: &Output) -> Result<(), String> {
    let auth_data = load_auth_file().await?;
    // `platformUrlOverride ? platformUrlOverride.replace(/\/+$/, "") : DEFAULT_PLATFORM_URL`
    let platform_url = match platform_url_override {
        Some(url) => trim_trailing_slash(&url),
        None => DEFAULT_PLATFORM_URL.to_string(),
    };

    let client = http_client();
    let device: DeviceCodeResponse = post(
        &client,
        &platform_url,
        "/client/v1/oauth/device/code",
        &json!({ "client_name": "Future OS CLI" }),
    )
    .await?;

    // `device.verification_uri_complete || device.verification_uri`
    let verification_url = if !device.verification_uri_complete.is_empty() {
        device.verification_uri_complete.clone()
    } else {
        device.verification_uri.clone()
    };
    let opened = open_browser(&verification_url).await;
    out.log(if opened {
        "Opened Future Platform Console:"
    } else {
        "Open this URL in your browser:"
    });
    out.log(&format!("  {verification_url}"));
    out.log("");
    out.log("Sign in and authorize this device code:");
    out.log(&format!("  {}", device.user_code));
    out.log("");
    out.log("Waiting for authorization...");

    // `const startedAt = Date.now(); while (Date.now() - startedAt < device.expires_in * 1000)`
    let started_at = now_ms();
    while now_ms() - started_at < device.expires_in * 1000 {
        sleep(device.interval * 1000).await;
        let response = try_fetch_post(
            &client,
            &format!("{platform_url}/client/v1/oauth/device/token"),
            &json!({ "device_code": device.device_code }),
        )
        .await?;
        let status = response.status();
        let body: Value = response
            .json()
            .await
            .map_err(|e| format!("Network error: {e}"))?;

        if status.is_success() {
            // `response.ok` — token granted.
            let token: DeviceTokenResponse =
                serde_json::from_value(body).map_err(|e| format!("Network error: {e}"))?;
            save_auth(&auth_data, &token, &platform_url).await?;
            out.log(&format!(
                "Saved Future API key to {}",
                auth_file_path().display()
            ));
            return Ok(());
        }

        // `error.error === "authorization_pending" || error.error === "slow_down"`
        let error: DeviceErrorResponse =
            serde_json::from_value(body).unwrap_or(DeviceErrorResponse {
                error: String::new(),
                message: String::new(),
            });
        if error.error == "authorization_pending" || error.error == "slow_down" {
            // `process.stdout.write(".")` — no trailing newline.
            out.write_out(".");
            continue;
        }
        return Err(error.message);
    }

    Err("Device authorization expired.".to_string())
}

/// `status()` — show login state.
pub async fn status(out: &Output) -> Result<(), String> {
    let result: Result<(), String> = async {
        let auth_file = load_auth_file().await?;
        let auth = get_future_auth_entry(&auth_file);
        let Some(auth) = auth else {
            out.log("Not logged in.");
            return Ok(());
        };
        if auth.key.is_none() {
            out.log("Not logged in.");
            return Ok(());
        }
        // `auth.base_url ? auth.base_url.replace(/\/api\/?$/, "") : await getPlatformUrl()`
        let platform_url = match &auth.base_url {
            Some(base_url) => strip_api_suffix(base_url),
            None => get_platform_url(None).await,
        };
        out.log(&format!("Platform: {platform_url}"));
        out.log(&format!("API: {platform_url}/api/v1"));
        Ok(())
    }
    .await;
    match result {
        Ok(()) => Ok(()),
        Err(_) => {
            out.log("Not logged in.");
            Ok(())
        }
    }
}

/// `credential({ json })` — output the API key + endpoint for scripting.
pub async fn credential(json: bool, out: &Output) -> Result<(), String> {
    let result: Result<(), String> = async {
        let auth_file = load_auth_file().await?;
        let auth = get_future_auth_entry(&auth_file);
        let Some(auth) = auth else {
            if json {
                out.log(&json!({ "error": "Not logged in." }).to_string());
            } else {
                out.log("Not logged in.");
            }
            return Ok(());
        };
        let Some(key) = auth.key else {
            if json {
                out.log(&json!({ "error": "Not logged in." }).to_string());
            } else {
                out.log("Not logged in.");
            }
            return Ok(());
        };
        let platform_url = match &auth.base_url {
            Some(base_url) => strip_api_suffix(base_url),
            None => get_platform_url(None).await,
        };
        let output = json!({
            "api_key": key,
            "endpoint": format!("{platform_url}/api/v1"),
        });
        out.log(&output.to_string());
        Ok(())
    }
    .await;
    match result {
        Ok(()) => Ok(()),
        Err(err) => {
            if json {
                out.log(&json!({ "error": err }).to_string());
            } else {
                out.log("Not logged in.");
            }
            Ok(())
        }
    }
}

/// `logout()` — remove the stored Future API key.
pub async fn logout(out: &Output) -> Result<(), String> {
    let auth_file = load_auth_file().await?;
    let current = get_future_auth_entry(&auth_file);
    let mut removed_key = false;

    if let Some(current) = current {
        if current.key.is_some() {
            // `const next = { ...current }; delete next.key;` — the sanitized
            // entry (type/key/base_url only) minus the key.
            let mut next = Map::new();
            if let Some(type_) = current.type_ {
                next.insert("type".to_string(), Value::String(type_));
            }
            if let Some(base_url) = current.base_url {
                next.insert("base_url".to_string(), Value::String(base_url));
            }
            let mut auth_file = auth_file;
            if let Some(obj) = auth_file.as_object_mut() {
                obj.insert(FUTURE_AUTH_PROVIDER.to_string(), Value::Object(next));
            }
            write_auth_file(&auth_file).await?;
            out.log(&format!(
                "Removed Future API key from {}",
                auth_file_path().display()
            ));
            removed_key = true;
        }
    }

    if !removed_key {
        out.log("Not logged in.");
    }
    Ok(())
}

/// `auth.baseUrl.replace(/\/api\/?$/, "")` — strip a trailing `/api` or
/// `/api/` (nothing else, no trailing-slash trim).
pub fn strip_api_suffix(base_url: &str) -> String {
    base_url
        .strip_suffix("/api/")
        .or_else(|| base_url.strip_suffix("/api"))
        .unwrap_or(base_url)
        .to_string()
}

// ── HTTP helpers ──────────────────────────────────────────────────────────

/// reqwest client — Node `fetch` has no default timeout and the device-code
/// poll loop controls its own pacing; reqwest's 30s crate default only
/// matters for a hung connection, which is acceptable.
fn http_client() -> reqwest::Client {
    reqwest::Client::new()
}

/// `tryFetch(url, init)` — network failures are wrapped in
/// `Network error: <msg>` (single-part form; reqwest exposes no separate
/// cause message like Node's fetch does).
async fn try_fetch_post(
    client: &reqwest::Client,
    url: &str,
    body: &Value,
) -> Result<reqwest::Response, String> {
    match client
        .post(url)
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .await
    {
        Ok(response) => Ok(response),
        Err(error) => Err(format!("Network error: {error}")),
    }
}

/// `post<T>(apiUrl, path, body)` — throws `data.message ?? "Request failed
/// with {status}"` when the response is not ok.
async fn post<T: for<'de> Deserialize<'de>>(
    client: &reqwest::Client,
    api_url: &str,
    path: &str,
    body: &Value,
) -> Result<T, String> {
    let response = try_fetch_post(client, &format!("{api_url}{path}"), body).await?;
    let status = response.status();
    let data: Value = response
        .json()
        .await
        .map_err(|e| format!("Network error: {e}"))?;
    let message = data
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_string);
    if !status.is_success() {
        return Err(message.unwrap_or_else(|| format!("Request failed with {}", status.as_u16())));
    }
    serde_json::from_value(data).map_err(|e| format!("Network error: {e}"))
}

// ── auth.json ─────────────────────────────────────────────────────────────

/// `~/.future/agent/auth.json` (resolved per call, see constants.rs).
fn auth_file_path() -> PathBuf {
    auth_file()
}

/// `loadAuthFile()` — ENOENT → `{}`; non-object JSON → error.
pub async fn load_auth_file() -> Result<Value, String> {
    match tokio::fs::read_to_string(auth_file_path()).await {
        Ok(contents) => match serde_json::from_str::<Value>(&contents) {
            Ok(value) if value.is_object() => Ok(value),
            Ok(_) => Err(format!(
                "{} must contain a JSON object.",
                auth_file_path().display()
            )),
            Err(e) => Err(e.to_string()),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(e.to_string()),
    }
}

/// `saveAuth(authFile, token, platformUrl)` — merge into the `future` entry.
async fn save_auth(
    auth_file: &Value,
    token: &DeviceTokenResponse,
    platform_url: &str,
) -> Result<(), String> {
    let current = get_future_auth_entry(auth_file).unwrap_or_default();
    let mut next = Map::new();
    next.insert(
        "type".to_string(),
        Value::String(current.type_.unwrap_or_else(|| "api_key".to_string())),
    );
    next.insert("key".to_string(), Value::String(token.api_key.clone()));
    next.insert(
        "base_url".to_string(),
        Value::String(format!("{platform_url}/api")),
    );
    let mut auth_file = auth_file.clone();
    if let Some(obj) = auth_file.as_object_mut() {
        obj.insert(FUTURE_AUTH_PROVIDER.to_string(), Value::Object(next));
    }
    write_auth_file(&auth_file).await
}

/// `writeAuthFile(authFile)` — mkdir -p, write pretty JSON + newline, 0600.
async fn write_auth_file(auth_file: &Value) -> Result<(), String> {
    let path = auth_file_path();
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    let contents = format!(
        "{}\n",
        serde_json::to_string_pretty(auth_file).map_err(|e| e.to_string())?
    );
    let mut opts = tokio::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        // tokio's OpenOptions has an inherent `mode` on unix.
        opts.mode(0o600);
    }
    let mut file = opts.open(&path).await.map_err(|e| e.to_string())?;
    tokio::io::AsyncWriteExt::write_all(&mut file, contents.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// `getFutureAuthEntry(authFile)` — sanitized `future` provider entry.
pub fn get_future_auth_entry(auth_file: &Value) -> Option<FutureAuthEntry> {
    let value = auth_file.get(FUTURE_AUTH_PROVIDER)?;
    let obj = value.as_object()?;
    Some(FutureAuthEntry {
        type_: obj.get("type").and_then(Value::as_str).map(str::to_string),
        key: obj.get("key").and_then(Value::as_str).map(str::to_string),
        base_url: obj
            .get("base_url")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

// ── Misc ──────────────────────────────────────────────────────────────────

/// `Date.now()`.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// `openBrowser(url)` — spawn the platform opener detached with stdio
/// ignored; resolves true when the process spawned.
async fn open_browser(url: &str) -> bool {
    let (command, args): (&str, Vec<String>) = if cfg!(target_os = "macos") {
        ("open", vec![url.to_string()])
    } else if cfg!(windows) {
        (
            "cmd",
            vec![
                "/c".to_string(),
                "start".to_string(),
                String::new(),
                url.to_string(),
            ],
        )
    } else {
        ("xdg-open", vec![url.to_string()])
    };
    std::process::Command::new(command)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::Output;
    use crate::test_env::EnvGuard;

    async fn run(args: &[&str]) -> (i32, String, String) {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let (out, cap) = Output::memory();
        let code = crate::dispatch(&args, &out).await;
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        (code, stdout, stderr)
    }

    #[tokio::test]
    async fn status_not_logged_in() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let (code, stdout, stderr) = run(&["auth", "status"]).await;
        assert_eq!(code, 0);
        assert_eq!(stdout, "Not logged in.\n");
        assert_eq!(stderr, "");
    }

    #[tokio::test]
    async fn status_logged_in() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let path = auth_file();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &path,
            serde_json::to_string_pretty(&json!({
                "future": { "type": "api_key", "key": "k123", "base_url": "https://future-os.cn/api" }
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        let (code, stdout, _) = run(&["auth", "status"]).await;
        assert_eq!(code, 0);
        assert_eq!(
            stdout,
            "Platform: https://future-os.cn\nAPI: https://future-os.cn/api/v1\n"
        );
    }

    #[tokio::test]
    async fn credential_variants() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();

        // Not logged in, plain.
        let (code, stdout, _) = run(&["auth", "credential"]).await;
        assert_eq!(code, 0);
        assert_eq!(stdout, "Not logged in.\n");

        // Not logged in, --json.
        let (code, stdout, _) = run(&["auth", "credential", "--json"]).await;
        assert_eq!(code, 0);
        assert_eq!(stdout, "{\"error\":\"Not logged in.\"}\n");

        // Logged in (base_url without /api suffix).
        let path = auth_file();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &path,
            serde_json::to_string_pretty(&json!({
                "future": { "key": "k456", "base_url": "https://example.com" }
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        let (code, stdout, _) = run(&["auth", "credential"]).await;
        assert_eq!(code, 0);
        assert_eq!(
            stdout,
            "{\"api_key\":\"k456\",\"endpoint\":\"https://example.com/api/v1\"}\n"
        );
    }

    #[tokio::test]
    async fn logout_removes_key_and_reports_not_logged_in() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let path = auth_file();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &path,
            serde_json::to_string_pretty(&json!({
                "openai": { "key": "keep-me" },
                "future": { "type": "api_key", "key": "drop-me", "base_url": "https://x/api" }
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let (code, stdout, _) = run(&["auth", "logout"]).await;
        assert_eq!(code, 0);
        assert_eq!(
            stdout,
            format!("Removed Future API key from {}\n", path.display())
        );
        // Other providers untouched; future entry keeps type + base_url.
        let after: Value =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        assert_eq!(after["openai"]["key"], "keep-me");
        assert_eq!(after["future"]["key"], Value::Null);
        assert_eq!(after["future"]["type"], "api_key");
        assert_eq!(after["future"]["base_url"], "https://x/api");

        // Second logout: no key left.
        let (code, stdout, _) = run(&["auth", "logout"]).await;
        assert_eq!(code, 0);
        assert_eq!(stdout, "Not logged in.\n");
    }

    #[tokio::test]
    async fn invalid_auth_file_reports_not_logged_in() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let path = auth_file();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, "not json{").await.unwrap();
        let (code, stdout, _) = run(&["auth", "status"]).await;
        assert_eq!(code, 0);
        assert_eq!(stdout, "Not logged in.\n");
    }

    #[tokio::test]
    async fn strip_api_suffix_behavior() {
        assert_eq!(strip_api_suffix("https://x/api"), "https://x");
        assert_eq!(strip_api_suffix("https://x/api/"), "https://x");
        assert_eq!(strip_api_suffix("https://x"), "https://x");
        assert_eq!(strip_api_suffix("https://x/apix"), "https://x/apix");
    }

    #[tokio::test]
    async fn auth_file_permissions_are_0600() {
        #[cfg(unix)]
        {
            let _guard = crate::test_env::lock_env().await;
            let _home = EnvGuard::temp_home();
            let path = auth_file();
            let auth = json!({ "future": { "key": "k" } });
            // Write through the private helper path directly.
            write_auth_file(&auth).await.unwrap();
            use std::os::unix::fs::PermissionsExt;
            let mode = tokio::fs::metadata(&path)
                .await
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[tokio::test]
    async fn get_future_auth_entry_edge_cases() {
        assert!(get_future_auth_entry(&json!({})).is_none());
        assert!(get_future_auth_entry(&json!({ "future": 5 })).is_none());
        assert!(get_future_auth_entry(&json!({ "future": null })).is_none());
        let entry = get_future_auth_entry(&json!({
            "future": { "key": "k", "type": 7, "base_url": ["x"] }
        }))
        .unwrap();
        // Non-string fields are dropped.
        assert_eq!(entry.key.as_deref(), Some("k"));
        assert_eq!(entry.type_, None);
        assert_eq!(entry.base_url, None);
    }
}
