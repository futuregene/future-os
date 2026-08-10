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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::chromium::cdp_connection::CdpConnection;
    use crate::test_cdp::MockCdp;

    async fn session_over(mock: &MockCdp) -> (std::sync::Arc<CdpConnection>, CdpSession) {
        let conn = CdpConnection::connect(&mock.ws_url, 5_000).await.unwrap();
        let session = CdpSession::new("S-1", conn.clone());
        (conn, session)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn install_is_idempotent_and_ignores_failures() {
        let mock = MockCdp::start().await;
        let (conn, session) = session_over(&mock).await;
        install_console_hook(&session).await;
        install_console_hook(&session).await;
        let evals = mock.commands_of("Runtime.evaluate");
        assert_eq!(evals.len(), 2);
        assert!(evals[0]["expression"]
            .as_str()
            .unwrap()
            .contains("__futureConsoleHookInstalled"));

        // Send failure is swallowed (best-effort install).
        mock.state
            .lock()
            .unwrap()
            .fail_methods
            .insert("Runtime.evaluate".to_string());
        install_console_hook(&session).await;
        conn.disconnect().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn read_console_logs_parses_filters_and_defaults() {
        let mock = MockCdp::start().await;
        mock.state.lock().unwrap().console_logs = json!([
            {"level": "log", "text": "hi", "time": "t1"},
            {"level": "error", "text": "boom", "time": "t2"},
            {"level": "warn"},                       // missing text/time → ""
            "not-an-object",                          // skipped
            {"level": 42, "text": 7, "time": false},  // non-string → ""
        ]);
        let (conn, session) = session_over(&mock).await;

        let logs = read_console_logs(&session, None).await.unwrap();
        assert_eq!(logs.len(), 4);
        assert_eq!(logs[0].level, "log");
        assert_eq!(logs[1].text, "boom");
        assert_eq!(logs[2].text, "");
        assert_eq!(logs[3].level, "");

        // Level filter.
        let errors = read_console_logs(&session, Some("error")).await.unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].time, "t2");

        // Non-array value → empty.
        mock.state.lock().unwrap().console_logs = json!("junk");
        assert!(read_console_logs(&session, None).await.unwrap().is_empty());

        // Send failure → Err.
        mock.state
            .lock()
            .unwrap()
            .fail_methods
            .insert("Runtime.evaluate".to_string());
        assert!(read_console_logs(&session, None).await.is_err());
        conn.disconnect().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn temporary_preload_wraps_action_with_identifier() {
        let mock = MockCdp::start().await;
        let (conn, session) = session_over(&mock).await;
        let value = with_temporary_preload(&session, async { Ok(42) })
            .await
            .unwrap();
        assert_eq!(value, 42);
        let removes = mock.commands_of("Page.removeScriptToEvaluateOnNewDocument");
        assert_eq!(removes.len(), 1);
        assert_eq!(removes[0]["identifier"], json!("preload-1"));

        // Action errors propagate (and the preload is still removed).
        let err: Result<(), String> =
            with_temporary_preload(&session, async { Err("action boom".to_string()) }).await;
        assert_eq!(err.unwrap_err(), "action boom");
        assert_eq!(
            mock.commands_of("Page.removeScriptToEvaluateOnNewDocument")
                .len(),
            2
        );

        // Add-script failure → Err before the action runs.
        mock.state
            .lock()
            .unwrap()
            .fail_methods
            .insert("Page.addScriptToEvaluateOnNewDocument".to_string());
        let err: Result<(), String> = with_temporary_preload(&session, async { Ok(()) }).await;
        assert!(err.unwrap_err().contains("mock failure"));
        conn.disconnect().await;
    }
}
