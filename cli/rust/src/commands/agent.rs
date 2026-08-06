//! `future agent` — 1:1 port of cli/src/commands/agent.ts.
//!
//! `agent status` shows the running agent's version and skills count.

use crate::output::Output;
use crate::rpc::{grpc_addr, RunClient};
use serde_json::{json, Value};

/// `agentStatus(jsonFlag)`.
pub async fn agent_status(json: bool, out: &Output) -> Result<(), String> {
    let client = RunClient::new(&grpc_addr());

    let info = match client.get_agent_info().await {
        Ok(info) => info,
        Err(msg) => {
            // `if (jsonFlag) console.log(JSON.stringify({error: msg}));
            //  else console.error(\`Error: ${msg}\`); process.exit(1);`
            if json {
                out.log(&json!({ "error": msg }).to_string());
            } else {
                out.log_err(&format!("Error: {msg}"));
            }
            return Err(crate::HANDLED_EXIT.to_string());
        }
    };

    if json {
        // `console.log(JSON.stringify(info, null, 2))`
        out.log(&serde_json::to_string_pretty(&info).map_err(|e| e.to_string())?);
        return Ok(());
    }

    // `info.version` / `info.skillsCount`
    let version = info.get("version").and_then(Value::as_str).unwrap_or("");
    let skills_count = info.get("skillsCount").and_then(Value::as_i64).unwrap_or(0);
    out.log(&format!("  Version:  {version}"));
    out.log(&format!("  Skills:   {skills_count} loaded"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::Output;

    #[tokio::test]
    async fn agent_status_agent_down_plain() {
        let _guard = crate::test_env::lock_env().await;
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from("127.0.0.1:1"),
        )]);
        let (out, cap) = Output::memory();
        let result = agent_status(false, &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.starts_with("Error: "), "stderr: {stderr}");
        assert_eq!(
            String::from_utf8(cap.out.lock().unwrap().clone()).unwrap(),
            ""
        );
    }

    #[tokio::test]
    async fn agent_status_agent_down_json() {
        let _guard = crate::test_env::lock_env().await;
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from("127.0.0.1:1"),
        )]);
        let (out, cap) = Output::memory();
        let result = agent_status(true, &out).await;
        assert_eq!(result, Err(crate::HANDLED_EXIT.to_string()));
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.starts_with("{\"error\":"), "stdout: {stdout}");
    }
}
