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

/// `platformUrlOverride ? trimTrailingSlashes(override) : DEFAULT_PLATFORM_URL`.
fn resolve_login_platform_url(platform_url_override: Option<String>) -> String {
    match platform_url_override {
        Some(url) => trim_trailing_slash(&url),
        None => DEFAULT_PLATFORM_URL.to_string(),
    }
}

/// `login(platformUrlOverride?)` — device-code OAuth flow.
pub async fn login(platform_url_override: Option<String>, out: &Output) -> Result<(), String> {
    let auth_data = load_auth_file().await?;
    // `platformUrlOverride ? platformUrlOverride.replace(/\/+$/, "") : DEFAULT_PLATFORM_URL`
    let platform_url = resolve_login_platform_url(platform_url_override);

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

    let Some(current) = current.filter(|c| c.key.is_some()) else {
        out.log("Not logged in.");
        return Ok(());
    };
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
    // Invariant: auth_file_path() is always `<home>/.future/agent/auth.json`.
    let parent = path.parent().expect("auth file path has a parent");
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| e.to_string())?;
    // Serializing plain JSON values is infallible.
    let contents = format!(
        "{}\n",
        serde_json::to_string_pretty(auth_file).expect("auth json serializes")
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
    let (command, args) = opener_command(url);
    std::process::Command::new(command)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}

/// The platform opener command. `#[cfg]` (not `cfg!`) so off-platform arms
/// are never compiled into this target.
#[cfg(target_os = "macos")]
fn opener_command(url: &str) -> (&'static str, Vec<String>) {
    ("open", vec![url.to_string()])
}

/// Windows opener: `cmd /c start "" <url>`.
#[cfg(windows)]
fn opener_command(url: &str) -> (&'static str, Vec<String>) {
    (
        "cmd",
        vec![
            "/c".to_string(),
            "start".to_string(),
            String::new(),
            url.to_string(),
        ],
    )
}

/// Linux/other opener: xdg-open.
#[cfg(not(any(target_os = "macos", windows)))]
fn opener_command(url: &str) -> (&'static str, Vec<String>) {
    ("xdg-open", vec![url.to_string()])
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

        // Invalid auth.json (parse failure → outer Err), plain output.
        tokio::fs::write(&path, "{not json").await.unwrap();
        let (code, stdout, _) = run(&["auth", "credential"]).await;
        assert_eq!(code, 0);
        assert_eq!(stdout, "Not logged in.\n");
        // Same, with --json.
        let (code, stdout, _) = run(&["auth", "credential", "--json"]).await;
        assert_eq!(code, 0);
        assert!(stdout.contains("\"error\""), "{stdout}");
    }

    #[tokio::test]
    async fn logout_without_any_future_entry_reports_not_logged_in() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let path = auth_file();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, r#"{"openai":{"key":"keep"}}"#)
            .await
            .unwrap();
        let (code, stdout, _) = run(&["auth", "logout"]).await;
        assert_eq!(code, 0);
        assert_eq!(stdout, "Not logged in.\n");
        // The other provider's entry was left alone.
        let after: Value =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        assert_eq!(after["openai"]["key"], "keep");
    }

    #[tokio::test]
    async fn write_auth_file_failures() {
        let _guard = crate::test_env::lock_env().await;

        // HOME points at a regular FILE → create_dir_all(parent) fails.
        let tmp = tempfile::tempdir().unwrap();
        let file_home = tmp.path().join("home-file");
        tokio::fs::write(&file_home, "x").await.unwrap();
        let _env = EnvGuard::set(&[("HOME", file_home.as_os_str().to_os_string())]);
        let err = write_auth_file(&json!({})).await.unwrap_err();
        assert!(!err.is_empty());
        drop(_env);

        // auth.json exists as a DIRECTORY → the write fails.
        let _home = EnvGuard::temp_home();
        tokio::fs::create_dir_all(auth_file()).await.unwrap();
        let err = write_auth_file(&json!({})).await.unwrap_err();
        assert!(!err.is_empty());
    }

    #[tokio::test]
    async fn login_with_invalid_auth_file_propagates_load_error() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let path = auth_file();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, "{not json").await.unwrap();
        let (code, _, _) = run(&["auth", "login", "--url", "http://127.0.0.1:1"]).await;
        assert_eq!(code, 1);
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn login_poll_failure_arms() {
        // Token poll returns a non-JSON body.
        let base = crate::test_server::spawn_http(vec![
            crate::test_server::HttpRoute::json(
                "/client/v1/oauth/device/code",
                200,
                "{\"device_code\":\"dc-1\",\"user_code\":\"U\",\"verification_uri\":\"https://x\",\"verification_uri_complete\":\"\",\"expires_in\":60,\"interval\":0}",
            ),
            crate::test_server::HttpRoute::json("/client/v1/oauth/device/token", 200, "not json"),
        ])
        .await;
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let opener = fake_opener_dir();
        let _env = EnvGuard::set(&[("PATH", opener.path().as_os_str().to_os_string())]);
        let (code, stdout, stderr) = run(&["auth", "login", "--url", &base]).await;
        assert_eq!(code, 1, "code={code} stdout={stdout:?} stderr={stderr:?}");
        assert!(stderr.contains("Network error"), "{stderr}");

        // Token poll returns JSON in the wrong shape.
        let base = crate::test_server::spawn_http(vec![
            crate::test_server::HttpRoute::json(
                "/client/v1/oauth/device/code",
                200,
                "{\"device_code\":\"dc-1\",\"user_code\":\"U\",\"verification_uri\":\"https://x\",\"verification_uri_complete\":\"\",\"expires_in\":60,\"interval\":0}",
            ),
            crate::test_server::HttpRoute::json(
                "/client/v1/oauth/device/token",
                200,
                "{\"api_key\": 42}",
            ),
        ])
        .await;
        let (code, _, _) = run(&["auth", "login", "--url", &base]).await;
        assert_eq!(code, 1);

        // Token poll dies at the transport level (server drops the request).
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            // First request: the device/code POST → a valid response.
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 8192];
            let _ = socket.read(&mut buf).await;
            let body = "{\"device_code\":\"dc\",\"user_code\":\"U\",\"verification_uri\":\"https://x\",\"verification_uri_complete\":\"\",\"expires_in\":60,\"interval\":0}";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(resp.as_bytes()).await;
            let _ = socket.shutdown().await;
            // The token poll connection is dropped unread. Bounded so the
            // task completes in-test (exactly one poll happens: the network
            // error propagates immediately).
            let (socket, _) = listener.accept().await.expect("poll connection");
            drop(socket);
        });
        let (code, _, _) = run(&["auth", "login", "--url", &format!("http://{addr}")]).await;
        assert_eq!(code, 1);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), server).await;
    }

    #[tokio::test]
    async fn logout_entry_without_key_reports_not_logged_in() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let path = auth_file();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        // The future entry exists but has no key field.
        tokio::fs::write(&path, r#"{"future":{"type":"api_key"}}"#)
            .await
            .unwrap();
        let (code, stdout, _) = run(&["auth", "logout"]).await;
        assert_eq!(code, 0);
        assert_eq!(stdout, "Not logged in.\n");
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

    // ── Device-code login flow (HTTP mock) ─────────────────────────
    //
    // All login tests manipulate PATH so the browser opener is a fake (or
    // missing) binary — never a real browser. Unix-only: on Windows the
    // opener is `cmd /c start`, which CreateProcess finds in System32 even
    // with an empty PATH (and would really open a browser).

    /// A temp dir holding a fake `open`/`xdg-open` that exits 0.
    #[cfg(not(windows))]
    fn fake_opener_dir() -> tempfile::TempDir {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        for name in ["open", "xdg-open"] {
            let bin = dir.path().join(name);
            std::fs::write(&bin, "#!/bin/sh\nexit 0\n").expect("write");
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        dir
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn login_success_after_pending_poll() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        // Pre-existing entry with a custom type — save_auth preserves it.
        let path = auth_file();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &path,
            "{\"future\": {\"type\": \"oauth\"}, \"openai\": {\"key\": \"keep\"}}",
        )
        .await
        .unwrap();

        let base = crate::test_server::spawn_http(vec![
            crate::test_server::HttpRoute::json(
                "/client/v1/oauth/device/code",
                200,
                "{\"device_code\":\"dc-1\",\"user_code\":\"ABCD-EFGH\",\"verification_uri\":\"https://x/verify\",\"verification_uri_complete\":\"https://x/verify?c=1\",\"expires_in\":60,\"interval\":0}",
            ),
            crate::test_server::HttpRoute::sequence(
                "/client/v1/oauth/device/token",
                vec![
                    (400, "{\"error\":\"authorization_pending\"}"),
                    (400, "{\"error\":\"slow_down\"}"),
                    (200, "{\"api_key\":\"sk-new\",\"api_key_id\":\"id1\",\"token_type\":\"bearer\"}"),
                ],
            ),
        ])
        .await;
        let opener = fake_opener_dir();
        let _env = EnvGuard::set(&[("PATH", opener.path().as_os_str().to_os_string())]);

        let (code, stdout, stderr) = run(&["auth", "login", "--url", &base]).await;
        assert_eq!(code, 0, "stderr: {stderr}");
        assert!(
            stdout.contains("Opened Future Platform Console:"),
            "stdout: {stdout}"
        );
        assert!(
            stdout.contains("  https://x/verify?c=1\n"),
            "stdout: {stdout}"
        );
        assert!(stdout.contains("  ABCD-EFGH"), "stdout: {stdout}");
        assert!(
            stdout.contains("Waiting for authorization..."),
            "stdout: {stdout}"
        );
        // Two pending polls printed dots before the grant.
        assert!(stdout.contains(".."), "stdout: {stdout}");
        assert!(
            stdout.contains("Saved Future API key to"),
            "stdout: {stdout}"
        );

        let saved: Value =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        assert_eq!(saved["future"]["key"], "sk-new");
        assert_eq!(saved["future"]["type"], "oauth");
        assert_eq!(saved["future"]["base_url"], format!("{base}/api"));
        assert_eq!(saved["openai"]["key"], "keep");
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn login_browser_open_failure_and_plain_verification_uri() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let base = crate::test_server::spawn_http(vec![
            crate::test_server::HttpRoute::json(
                "/client/v1/oauth/device/code",
                200,
                "{\"device_code\":\"dc-1\",\"user_code\":\"WXYZ\",\"verification_uri\":\"https://x/verify\",\"expires_in\":60,\"interval\":0}",
            ),
            crate::test_server::HttpRoute::json(
                "/client/v1/oauth/device/token",
                200,
                "{\"api_key\":\"sk-2\"}",
            ),
        ])
        .await;
        // Empty PATH → no opener binary → manual-open message.
        let empty = tempfile::tempdir().expect("tempdir");
        let _env = EnvGuard::set(&[("PATH", empty.path().as_os_str().to_os_string())]);
        let (code, stdout, _) = run(&["auth", "login", "--url", &base]).await;
        assert_eq!(code, 0);
        assert!(
            stdout.contains("Open this URL in your browser:"),
            "stdout: {stdout}"
        );
        assert!(stdout.contains("  https://x/verify\n"), "stdout: {stdout}");
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn login_expired_device_code() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/oauth/device/code",
            200,
            "{\"device_code\":\"dc-1\",\"user_code\":\"WXYZ\",\"verification_uri\":\"https://x/verify\",\"expires_in\":0,\"interval\":0}",
        )])
        .await;
        let empty = tempfile::tempdir().expect("tempdir");
        let _env = EnvGuard::set(&[("PATH", empty.path().as_os_str().to_os_string())]);
        let (code, _, stderr) = run(&["auth", "login", "--url", &base]).await;
        assert_eq!(code, 1);
        assert_eq!(stderr, "Device authorization expired.\n");
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn login_device_code_post_error_variants() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let empty = tempfile::tempdir().expect("tempdir");
        let _env = EnvGuard::set(&[("PATH", empty.path().as_os_str().to_os_string())]);
        // message field.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/oauth/device/code",
            500,
            "{\"message\":\"broken server\"}",
        )])
        .await;
        let (code, _, stderr) = run(&["auth", "login", "--url", &base]).await;
        assert_eq!(code, 1);
        assert_eq!(stderr, "broken server\n");
        // No message → status fallback.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/oauth/device/code",
            500,
            "{}",
        )])
        .await;
        let (code, _, stderr) = run(&["auth", "login", "--url", &base]).await;
        assert_eq!(code, 1);
        assert_eq!(stderr, "Request failed with 500\n");
        // Non-JSON body → Network error.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/oauth/device/code",
            200,
            "not json",
        )])
        .await;
        let (code, _, stderr) = run(&["auth", "login", "--url", &base]).await;
        assert_eq!(code, 1);
        assert!(stderr.contains("Network error"), "stderr: {stderr}");
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn login_token_error_other_than_pending() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let base = crate::test_server::spawn_http(vec![
            crate::test_server::HttpRoute::json(
                "/client/v1/oauth/device/code",
                200,
                "{\"device_code\":\"dc-1\",\"user_code\":\"WXYZ\",\"verification_uri\":\"https://x/verify\",\"expires_in\":60,\"interval\":0}",
            ),
            crate::test_server::HttpRoute::json(
                "/client/v1/oauth/device/token",
                403,
                "{\"error\":\"access_denied\",\"message\":\"Denied by user\"}",
            ),
        ])
        .await;
        let empty = tempfile::tempdir().expect("tempdir");
        let _env = EnvGuard::set(&[("PATH", empty.path().as_os_str().to_os_string())]);
        let (code, _, stderr) = run(&["auth", "login", "--url", &base]).await;
        assert_eq!(code, 1);
        assert_eq!(stderr, "Denied by user\n");
    }

    #[tokio::test]
    async fn status_and_credential_remaining_variants() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let path = auth_file();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();

        // Logged in WITHOUT base_url → default platform URL fallback.
        tokio::fs::write(&path, "{\"future\": {\"key\": \"k\"}}")
            .await
            .unwrap();
        let (code, stdout, _) = run(&["auth", "status"]).await;
        assert_eq!(code, 0);
        assert_eq!(
            stdout,
            format!("Platform: {DEFAULT_PLATFORM_URL}\nAPI: {DEFAULT_PLATFORM_URL}/api/v1\n")
        );

        // Entry present but key missing → Not logged in.
        tokio::fs::write(&path, "{\"future\": {\"type\": \"api_key\"}}")
            .await
            .unwrap();
        let (code, stdout, _) = run(&["auth", "status"]).await;
        assert_eq!(code, 0);
        assert_eq!(stdout, "Not logged in.\n");
        let (code, stdout, _) = run(&["auth", "credential"]).await;
        assert_eq!(code, 0);
        assert_eq!(stdout, "Not logged in.\n");

        // Credential --json while logged in.
        tokio::fs::write(
            &path,
            "{\"future\": {\"key\": \"k9\", \"base_url\": \"https://x/api\"}}",
        )
        .await
        .unwrap();
        let (code, stdout, _) = run(&["auth", "credential", "--json"]).await;
        assert_eq!(code, 0);
        assert_eq!(
            stdout,
            "{\"api_key\":\"k9\",\"endpoint\":\"https://x/api/v1\"}\n"
        );

        // Credential --json with a corrupt auth file → error JSON.
        tokio::fs::write(&path, "{oops").await.unwrap();
        let (code, stdout, _) = run(&["auth", "credential", "--json"]).await;
        assert_eq!(code, 0);
        assert!(stdout.starts_with("{\"error\":\""), "stdout: {stdout}");
    }

    // ── Remainder coverage ────────────────────────────────────────────

    #[test]
    fn resolve_login_platform_url_arms() {
        assert_eq!(resolve_login_platform_url(None), DEFAULT_PLATFORM_URL);
        assert_eq!(
            resolve_login_platform_url(Some("http://x/".to_string())),
            "http://x"
        );
    }

    #[tokio::test]
    async fn credential_json_entry_without_key_and_without_base_url() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let path = auth_file_path();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();

        // Entry exists but has NO key → JSON "Not logged in.".
        tokio::fs::write(&path, r#"{"future": {"base_url": "http://p/api"}}"#)
            .await
            .unwrap();
        let (out, cap) = Output::memory();
        credential(true, &out).await.unwrap();
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert_eq!(stdout.trim(), r#"{"error":"Not logged in."}"#);

        // Key but NO base_url → get_platform_url fallback for the endpoint.
        tokio::fs::write(&path, r#"{"future": {"key": "k"}}"#)
            .await
            .unwrap();
        let (out, cap) = Output::memory();
        credential(true, &out).await.unwrap();
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("\"api_key\":\"k\""), "{stdout}");
        assert!(stdout.contains("/api/v1"), "{stdout}");
    }

    #[tokio::test]
    async fn logout_full_entry_arm_coverage() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let path = auth_file_path();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        // Entry with key + type + base_url → all insert arms run.
        tokio::fs::write(
            &path,
            r#"{"future": {"type": "oauth", "key": "k", "base_url": "http://p/api"}}"#,
        )
        .await
        .unwrap();
        let (out, cap) = Output::memory();
        logout(&out).await.unwrap();
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("Removed Future API key"), "{stdout}");
        // The entry survives sans key (type + base_url kept).
        let remaining: Value =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        let entry = remaining.get("future").unwrap();
        assert!(entry.get("key").is_none());
        assert_eq!(entry.get("type").and_then(Value::as_str), Some("oauth"));
        assert_eq!(
            entry.get("base_url").and_then(Value::as_str),
            Some("http://p/api")
        );

        // Entry without a key → "Not logged in." (nothing removed).
        let (out, cap) = Output::memory();
        logout(&out).await.unwrap();
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("Not logged in."), "{stdout}");
    }

    #[tokio::test]
    async fn load_auth_file_non_object_and_unreadable() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let path = auth_file_path();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();

        // Valid JSON but not an object → "must contain a JSON object".
        tokio::fs::write(&path, "[1,2]").await.unwrap();
        let err = load_auth_file().await.unwrap_err();
        assert!(err.contains("must contain a JSON object"), "{err}");

        // Unreadable (chmod 000) → raw IO error string.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::write(&path, "{}").await.unwrap();
            tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000))
                .await
                .unwrap();
            let err = load_auth_file().await.unwrap_err();
            assert!(!err.is_empty());
            tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn write_auth_file_mkdir_failure() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().unwrap();
        // $HOME/.future is a REGULAR FILE → create_dir_all(agent) fails.
        tokio::fs::write(dir.path().join(".future"), "x")
            .await
            .unwrap();
        let _home =
            crate::test_env::EnvGuard::set(&[("HOME", dir.path().as_os_str().to_os_string())]);
        let err = write_auth_file(&json!({})).await.unwrap_err();
        assert!(!err.is_empty());
    }

    #[tokio::test]
    async fn write_auth_file_write_failure() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().unwrap();
        let _home =
            crate::test_env::EnvGuard::set(&[("HOME", dir.path().as_os_str().to_os_string())]);
        // auth.json as a DIRECTORY: mkdir succeeds, the file write fails.
        let agent_dir = dir.path().join(".future").join("agent");
        tokio::fs::create_dir_all(agent_dir.join("auth.json"))
            .await
            .unwrap();
        let err = write_auth_file(&json!({})).await.unwrap_err();
        assert!(!err.is_empty());
    }
}
