//! Console hook manager for Chromium CDP — port of
//! `cli/src/browser/chromium/chromium-console-hook.ts`.

use super::cdp_connection::CdpSession;
use crate::browser::scripts::console_hook_invocation_source;
use serde_json::{json, Value};

/// `installConsoleHook(session)` — safe to call multiple times; idempotent.
pub async fn install_console_hook(session: &CdpSession) {
    let params = json!({ "expression": console_hook_invocation_source() });
    let _ = session.send("Runtime.evaluate", params.as_object()).await;
}

/// `readConsoleLogs(session, level?)`.
pub async fn read_console_logs(
    session: &CdpSession,
    level: Option<&str>,
) -> Result<Vec<ConsoleLog>, String> {
    let raw = session
        .send(
            "Runtime.evaluate",
            Some(&json!({ "expression": "(globalThis.__futureConsoleLogs) || []", "returnByValue": true }).as_object().unwrap().clone()),
        )
        .await
        .map_err(|e| e.to_string())?;

    let value = raw
        .get("result")
        .and_then(|r| r.get("value"))
        .cloned()
        .unwrap_or(Value::Null);
    let logs = match value {
        Value::Array(items) => items
            .iter()
            .filter_map(|e| {
                let obj = e.as_object()?;
                let level = obj
                    .get("level")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_default();
                let text = obj
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_default();
                let time = obj
                    .get("time")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_default();
                Some(ConsoleLog { level, text, time })
            })
            .filter(|e| level.map(|l| e.level == l).unwrap_or(true))
            .collect(),
        _ => Vec::new(),
    };
    Ok(logs)
}

/// One buffered console entry.
#[derive(Debug, Clone)]
pub struct ConsoleLog {
    pub level: String,
    pub text: String,
    pub time: String,
}

/// `withTemporaryPreload(session, action)` — wrap an action with a preload
/// script so the hook survives the next navigation.
pub async fn with_temporary_preload<F, T>(session: &CdpSession, action: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    let result = session
        .send(
            "Page.addScriptToEvaluateOnNewDocument",
            Some(
                &json!({ "source": console_hook_invocation_source() })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .map_err(|e| e.to_string())?;
    let identifier = result
        .get("identifier")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let outcome = action.await;

    let _ = session
        .send(
            "Page.removeScriptToEvaluateOnNewDocument",
            Some(
                &json!({ "identifier": identifier })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await;

    outcome
}
