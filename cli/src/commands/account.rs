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
        _ => unreachable!("dispatch guards is_account_command"),
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
}
