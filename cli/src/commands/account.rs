//! `future account` — 1:1 port of cli/src/commands/account.ts.
//!
//! profile / balance via the Future platform API, authenticated with the
//! stored API key from auth.json.

use crate::constants::{auth_file, FUTURE_AUTH_PROVIDER};
use crate::output::Output;
use crate::utils::platform::get_platform_url;
use serde_json::{json, Value};
use std::time::Duration;

/// `isAccountCommand(command)` — type-guard port; `undefined` is not a command.
pub fn is_account_command(command: Option<&str>) -> bool {
    matches!(command, Some("profile" | "balance"))
}

/// `account(command, args)` — dispatch to profile/balance.
pub async fn account(command: &str, args: &[String], out: &Output) -> Result<(), String> {
    // `const jsonFlag = args.includes("--json");`
    let json_flag = args.iter().any(|a| a == "--json");
    match command {
        "profile" => account_profile(json_flag, out).await,
        "balance" => account_balance(json_flag, out).await,
        // The dispatch guards via is_account_command; a direct caller with an
        // unknown subcommand gets an error rather than a panic.
        other => Err(format!("Unknown account command: {other}")),
    }
}

// ── Auth helpers ────────────────────────────────────────────────────────────

struct AccountAuth {
    api_key: String,
    platform_url: String,
}

/// `loadAccountAuth()` — read the API key from auth.json.
async fn load_account_auth() -> Result<AccountAuth, String> {
    let raw = match tokio::fs::read_to_string(auth_file()).await {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(
                "No API key found. Run \"future auth login\" first, or set the FUTURE_API_KEY environment variable."
                    .to_string(),
            );
        }
        Err(e) => return Err(e.to_string()),
    };

    let parsed: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    if !parsed.is_object() {
        return Err(format!(
            "{} must contain a JSON object.",
            auth_file().display()
        ));
    }

    // `const future = parsed[FUTURE_AUTH_PROVIDER];`
    let future = parsed.get(FUTURE_AUTH_PROVIDER);
    if !future.is_some_and(Value::is_object) {
        return Err(format!(
            "No \"{FUTURE_AUTH_PROVIDER}\" provider in {}.",
            auth_file().display()
        ));
    }
    let future = future.unwrap();

    // `typeof future.key === "string" ? future.key : undefined`
    let key = future.get("key").and_then(Value::as_str);
    if key.is_none() {
        return Err(format!(
            "No API key for \"{FUTURE_AUTH_PROVIDER}\" in {}. Run \"future auth login\" first.",
            auth_file().display()
        ));
    }

    Ok(AccountAuth {
        api_key: key.unwrap().to_string(),
        platform_url: get_platform_url(None).await,
    })
}

// ── HTTP helpers ────────────────────────────────────────────────────────────

/// `platformGet<T>(url, apiKey)` — GET with Bearer auth, 30s timeout.
async fn platform_get(url: &str, api_key: &str) -> Result<Value, String> {
    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(api_key)
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let status = response.status();
    let body: Value = response.json().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        // `body.message ?? body.error ?? \`HTTP ${status}\``
        let message = body
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| body.get("error").and_then(Value::as_str))
            .map(str::to_string)
            .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
        return Err(message);
    }
    Ok(body)
}

// ── Profile ─────────────────────────────────────────────────────────────────

/// `accountProfile(jsonFlag)`.
async fn account_profile(json_flag: bool, out: &Output) -> Result<(), String> {
    let auth = load_account_auth().await?;
    let url = format!("{}/client/v1/account/profile", auth.platform_url);
    let profile = platform_get(&url, &auth.api_key).await?;

    if json_flag {
        // `JSON.stringify({email, user_id, email_verified, created_at}, null, 2)`
        let output = json!({
            "email": profile.get("email").cloned().unwrap_or(Value::Null),
            "user_id": profile.get("user_id").cloned().unwrap_or(Value::Null),
            "email_verified": profile.get("email_verified").cloned().unwrap_or(Value::Null),
            "created_at": profile.get("created_at").cloned().unwrap_or(Value::Null),
        });
        out.log(&serde_json::to_string_pretty(&output).map_err(|e| e.to_string())?);
    } else {
        out.log(&format!(
            "  Email:           {}",
            profile.get("email").and_then(Value::as_str).unwrap_or("")
        ));
        out.log(&format!(
            "  User ID:         {}",
            profile.get("user_id").and_then(Value::as_str).unwrap_or("")
        ));
        out.log(&format!(
            "  Email verified:  {}",
            profile
                .get("email_verified")
                .map(Value::to_string)
                .unwrap_or_default()
        ));
        out.log(&format!(
            "  Created:         {}",
            profile
                .get("created_at")
                .and_then(Value::as_str)
                .unwrap_or("")
        ));
    }
    Ok(())
}

// ── Balance ─────────────────────────────────────────────────────────────────

/// `accountBalance(jsonFlag)`.
async fn account_balance(json_flag: bool, out: &Output) -> Result<(), String> {
    let auth = load_account_auth().await?;
    let url = format!("{}/client/v1/account/balance", auth.platform_url);
    let balance = platform_get(&url, &auth.api_key).await?;

    // `balance_credits / 10_000_000_000` — internal units → credits.
    let raw_credits = balance
        .get("balance_credits")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let credits = raw_credits / 10_000_000_000.0;

    if json_flag {
        let output = json!({
            "balance_credits": balance.get("balance_credits").cloned().unwrap_or(Value::Null),
            "credits": to_fixed(&credits, 3),
        });
        out.log(&serde_json::to_string_pretty(&output).map_err(|e| e.to_string())?);
    } else {
        // `credits.toFixed(3)` — Rust {:.3} matches for all non-tie values.
        out.log(&format!("  Balance: {credits:.3} credits"));
    }
    Ok(())
}

/// `Number(x.toFixed(3))` — round half away from zero at 3 decimals.
fn to_fixed(x: &f64, decimals: u32) -> f64 {
    let factor = 10f64.powi(decimals as i32);
    (x * factor).round() / factor
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
    async fn account_without_auth_key_errors() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        // No auth file at all.
        let (code, stdout, stderr) = run(&["account", "profile"]).await;
        assert_eq!(code, 1);
        assert_eq!(stdout, "");
        assert!(stderr.contains("No API key found. Run \"future auth login\" first"));

        // Auth file present but no future provider.
        let path = auth_file();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, "{}").await.unwrap();
        let (code, _, stderr) = run(&["account", "balance"]).await;
        assert_eq!(code, 1);
        assert!(stderr.contains("No \"future\" provider in"));

        // Provider present but key missing.
        tokio::fs::write(&path, "{\"future\": {\"base_url\": \"https://x/api\"}}")
            .await
            .unwrap();
        let (code, _, stderr) = run(&["account", "profile"]).await;
        assert_eq!(code, 1);
        assert!(stderr.contains("No API key for \"future\" in"));
    }

    #[tokio::test]
    async fn to_fixed_rounding() {
        assert_eq!(to_fixed(&1.23456, 3), 1.235);
        assert_eq!(to_fixed(&1.0, 3), 1.0);
        assert_eq!(to_fixed(&0.0, 3), 0.0);
        assert_eq!(to_fixed(&(12345678901.0 / 1e10), 3), 1.235);
    }

    // ── HTTP-mock backed flows ──────────────────────────────────────

    /// Write auth.json with a key + base_url pointing at `platform_url`.
    async fn write_auth(platform_url: &str) {
        let path = auth_file();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &path,
            format!("{{\"future\": {{\"key\": \"sk-test\", \"base_url\": \"{platform_url}\"}}}}"),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn profile_text_and_json() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let base = crate::test_server::spawn_http_recording(
            vec![crate::test_server::HttpRoute::json(
                "/client/v1/account/profile",
                200,
                "{\"email\":\"a@b.c\",\"user_id\":\"u1\",\"email_verified\":true,\"created_at\":\"2026-01-01\"}",
            )],
            Some(requests.clone()),
        )
        .await;
        write_auth(&base).await;

        // Text mode.
        let (code, stdout, stderr) = run(&["account", "profile"]).await;
        assert_eq!(code, 0);
        assert!(stderr.is_empty());
        assert!(
            stdout.contains("  Email:           a@b.c"),
            "stdout: {stdout}"
        );
        assert!(stdout.contains("  User ID:         u1"), "stdout: {stdout}");
        assert!(
            stdout.contains("  Email verified:  true"),
            "stdout: {stdout}"
        );
        assert!(
            stdout.contains("  Created:         2026-01-01"),
            "stdout: {stdout}"
        );

        // JSON mode: only the four known keys, in TS order.
        let (code, stdout, _) = run(&["account", "profile", "--json"]).await;
        assert_eq!(code, 0);
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(parsed["email"], "a@b.c");
        assert_eq!(parsed["user_id"], "u1");
        assert_eq!(parsed["email_verified"], true);
        assert_eq!(parsed["created_at"], "2026-01-01");
        assert_eq!(parsed.as_object().unwrap().len(), 4);

        // Bearer auth header sent.
        let recorded = requests.lock().unwrap();
        assert!(!recorded.is_empty());
        let first_request = &recorded[0];
        assert!(
            first_request.contains("authorization: Bearer sk-test"),
            "request: {first_request}"
        );
    }

    #[tokio::test]
    async fn profile_missing_fields_render_empty() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/account/profile",
            200,
            "{}",
        )])
        .await;
        write_auth(&base).await;
        let (code, stdout, _) = run(&["account", "profile"]).await;
        assert_eq!(code, 0);
        assert!(stdout.contains("  Email:           \n"), "stdout: {stdout}");
        // JSON mode renders explicit nulls.
        let (code, stdout, _) = run(&["account", "profile", "--json"]).await;
        assert_eq!(code, 0);
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(parsed["email"], Value::Null);
    }

    #[tokio::test]
    async fn balance_text_and_json() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/account/balance",
            200,
            "{\"balance_credits\": 12345678901}",
        )])
        .await;
        write_auth(&base).await;
        let (code, stdout, _) = run(&["account", "balance"]).await;
        assert_eq!(code, 0);
        assert_eq!(stdout, "  Balance: 1.235 credits\n");

        let (code, stdout, _) = run(&["account", "balance", "--json"]).await;
        assert_eq!(code, 0);
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(parsed["balance_credits"], 12345678901_i64);
        assert_eq!(parsed["credits"], 1.235);
    }

    #[tokio::test]
    async fn balance_missing_field_is_zero() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/account/balance",
            200,
            "{}",
        )])
        .await;
        write_auth(&base).await;
        let (code, stdout, _) = run(&["account", "balance"]).await;
        assert_eq!(code, 0);
        assert_eq!(stdout, "  Balance: 0.000 credits\n");
    }

    #[tokio::test]
    async fn http_error_message_extraction() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        // message field wins, then error field, then HTTP status fallback.
        for (body, expected) in [
            ("{\"message\":\"nope\"}", "nope"),
            ("{\"error\":\"broken\"}", "broken"),
            ("{}", "HTTP 401"),
        ] {
            let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
                "/client/v1/account/profile",
                401,
                body,
            )])
            .await;
            write_auth(&base).await;
            let (code, _, stderr) = run(&["account", "profile"]).await;
            assert_eq!(code, 1);
            assert_eq!(stderr, format!("{expected}\n"), "body: {body}");
        }
    }

    #[tokio::test]
    async fn platform_get_transport_and_json_errors() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        // Connection refused.
        write_auth("http://127.0.0.1:1").await;
        let (code, _, stderr) = run(&["account", "balance"]).await;
        assert_eq!(code, 1);
        assert!(!stderr.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn account_auth_file_unreadable() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set(&[("HOME", dir.path().as_os_str().to_owned())]);
        write_auth("http://127.0.0.1:1").await;
        // chmod 000 → read fails with a non-NotFound IO error.
        use std::os::unix::fs::PermissionsExt;
        let auth_path = dir.path().join(".future").join("agent").join("auth.json");
        tokio::fs::set_permissions(&auth_path, std::fs::Permissions::from_mode(0o000))
            .await
            .unwrap();
        let (code, _, stderr) = run(&["account", "profile"]).await;
        assert_eq!(code, 1);
        assert!(!stderr.is_empty());
        tokio::fs::set_permissions(&auth_path, std::fs::Permissions::from_mode(0o644))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn auth_file_edge_cases() {
        let _guard = crate::test_env::lock_env().await;
        let _home = EnvGuard::temp_home();
        let path = auth_file();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        // Non-object JSON.
        tokio::fs::write(&path, "[1,2]").await.unwrap();
        let (code, _, stderr) = run(&["account", "profile"]).await;
        assert_eq!(code, 1);
        assert!(
            stderr.contains("must contain a JSON object"),
            "stderr: {stderr}"
        );
        // Invalid JSON.
        tokio::fs::write(&path, "{oops").await.unwrap();
        let (code, _, stderr) = run(&["account", "profile"]).await;
        assert_eq!(code, 1);
        assert!(!stderr.is_empty());
        // future provider not an object.
        tokio::fs::write(&path, "{\"future\": 42}").await.unwrap();
        let (code, _, stderr) = run(&["account", "profile"]).await;
        assert_eq!(code, 1);
        assert!(
            stderr.contains("No \"future\" provider in"),
            "stderr: {stderr}"
        );
        // key not a string.
        tokio::fs::write(&path, "{\"future\": {\"key\": 7}}")
            .await
            .unwrap();
        let (code, _, stderr) = run(&["account", "profile"]).await;
        assert_eq!(code, 1);
        assert!(
            stderr.contains("No API key for \"future\" in"),
            "stderr: {stderr}"
        );
    }

    #[tokio::test]
    async fn account_dispatch_rejects_unknown_command() {
        // The `_` arm: dispatch guards via is_account_command, but a direct
        // call surfaces an error instead of the old unreachable!() panic.
        let (out, _) = Output::memory();
        let err = account("bogus", &[], &out).await.unwrap_err();
        assert!(err.contains("Unknown account command"), "err: {err}");
    }
}
