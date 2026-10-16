//! Read-only tool discovery, separate from permission to connect to an Agent.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliCapabilities {
    pub format_version: u32,
    pub agent_version: String,
    pub agent_protocol: u32,
    pub storage_schema: u32,
    pub commands: Vec<String>,
}

impl Default for CliCapabilities {
    fn default() -> Self {
        Self {
            format_version: 1,
            agent_version: crate::connection::EXPECTED_AGENT_VERSION.into(),
            agent_protocol: crate::connection::AGENT_PROTOCOL_VERSION,
            storage_schema: crate::connection::AGENT_STORAGE_SCHEMA_VERSION,
            commands: vec!["skills.init".into()],
        }
    }
}

impl CliCapabilities {
    pub fn validate(&self, command: Option<&str>) -> Result<(), String> {
        use crate::connection::*;
        if self.format_version != 1
            || release_core(&self.agent_version).is_none()
            || release_core(&self.agent_version) != release_core(EXPECTED_AGENT_VERSION)
            || self.agent_protocol != AGENT_PROTOCOL_VERSION
            || self.storage_schema != AGENT_STORAGE_SCHEMA_VERSION
        {
            return Err(format!(
                "CLI Agent contract {}/{}/{} is incompatible; expected {}/{}/{}",
                self.agent_version,
                self.agent_protocol,
                self.storage_schema,
                EXPECTED_AGENT_VERSION,
                AGENT_PROTOCOL_VERSION,
                AGENT_STORAGE_SCHEMA_VERSION
            ));
        }
        if let Some(command) = command {
            if !self.commands.iter().any(|supported| supported == command) {
                return Err(format!("CLI does not advertise capability {command}"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_are_independent_of_development_commit_suffixes() {
        let mut info = CliCapabilities::default();
        info.agent_version = format!(
            "{}-different+local",
            crate::connection::release_core(&info.agent_version).unwrap()
        );
        assert!(info.validate(Some("skills.init")).is_ok());
        info.agent_protocol += 1;
        assert!(info.validate(None).is_err());
    }

    #[test]
    fn tool_capabilities_do_not_follow_from_agent_compatibility() {
        let mut info = CliCapabilities::default();
        info.commands.clear();
        assert!(info.validate(None).is_ok());
        assert!(info.validate(Some("skills.init")).is_err());
        info.agent_version = "invalid".into();
        assert!(info.validate(None).is_err());
    }
}
