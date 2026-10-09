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
    #[serde(alias = "additionalPermissions")]
    pub additional_permissions: Option<crate::sandbox::windows_request::AdditionalPermissions>,
}
impl ShellParams {
    pub fn parse(args: serde_json::Value) -> Result<Self> {
        if serde_json::to_vec(&args)?.len() > 131_072 {
            return Err(anyhow!("Shell input exceeds 131072 serialized bytes"));
        }
        let params: Self = serde_json::from_value(args)?;
        if params.command.trim().is_empty() || params.command.contains('\0') {
            return Err(anyhow!("command must be nonempty and contain no NUL"));
        }
        if params.command.len() > 65_536 {
            return Err(anyhow!("Command input exceeds 65536 bytes"));
        }
        if params.timeout.is_some_and(|n| !(1..=600).contains(&n)) {
            return Err(anyhow!("timeout must be 1..600 seconds"));
        }
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
            crate::sandbox::windows_request::prepare(&sandbox, &params.command, permissions)?;
        }
        Ok(params)
    }
}
pub(crate) fn validate(args: &serde_json::Value) -> Result<()> {
    ShellParams::parse(args.clone()).map(|_| ())
}
