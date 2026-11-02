use super::*;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ShellParams {
    pub command: String,
    pub timeout: Option<u64>,
    #[serde(default)]
    pub escalated: Option<bool>,
    pub justification: Option<String>,
    #[cfg(not(windows))]
    #[serde(alias = "additionalPermissions")]
    pub additional_permissions: Option<crate::sandbox::file_request::AdditionalPermissions>,
    #[cfg(windows)]
    #[serde(alias = "additionalPermissions")]
    pub additional_permissions: Option<crate::sandbox::windows_request::AdditionalPermissions>,
}
impl ShellParams {
    pub fn parse(args: serde_json::Value) -> Result<Self> {
        if serde_json::to_vec(&args)?.len() > 131_072 {
            return Err(anyhow!("Shell input exceeds 131072 serialized bytes"));
        }
        let mut params: Self = serde_json::from_value(args)?;
        if params.command.trim().is_empty() || params.command.contains('\0') {
            return Err(anyhow!("command must be nonempty and contain no NUL"));
        }
        if params.command.len() > 65_536 {
            return Err(anyhow!("Command input exceeds 65536 bytes"));
        }
        if params.timeout.is_some_and(|n| n < 1) {
            return Err(anyhow!("timeout must be at least 1 second"));
        }
        // An over-long request is clamped, not rejected: 600 s is the ceiling
        // this tool enforces anyway, and the old rejection threw away a whole
        // round trip (a full context re-read) just to be told the number again.
        // Measured over the sessions in one Agent database: 137 calls failed
        // this way and had to be re-sent.
        params.timeout = params.timeout.map(|n| n.min(MAX_TIMEOUT_SECS));
        if params
            .justification
            .as_ref()
            .is_some_and(|s| s.len() > 2048)
        {
            return Err(anyhow!("Justification exceeds 2048 bytes"));
        }
        if params.escalated == Some(true)
            && params
                .justification
                .as_deref()
                .is_none_or(|s| s.trim().is_empty())
        {
            return Err(anyhow!("escalated requires a justification"));
        }
        reject_dangerous_command(&params.command)?;
        let sandbox = TOOL_SCOPE
            .try_with(|scope| scope.sandbox.clone())
            .unwrap_or_default();
        if let Some(permissions) = &params.additional_permissions {
            if params.escalated == Some(true) {
                return Err(anyhow!(
                    "additional_permissions cannot be combined with escalated"
                ));
            }
            #[cfg(windows)]
            crate::sandbox::windows_request::prepare(&sandbox, &params.command, permissions)?;
            #[cfg(not(windows))]
            crate::sandbox::file_request::prepare(&sandbox, &params.command, permissions)?;
        }
        Ok(params)
    }
}
pub(crate) fn validate(args: &serde_json::Value) -> Result<()> {
    ShellParams::parse(args.clone()).map(|_| ())
}
