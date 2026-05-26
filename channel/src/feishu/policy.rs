//! Permission policy enforcement for Feishu channel bridge.

use super::config::PolicyConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    Allowed,
    Denied(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatOverride {
    pub enabled: Option<bool>,
    pub require_mention: Option<bool>,
}

pub struct PolicyEngine {
    config: PolicyConfig,
    overrides: std::collections::HashMap<String, ChatOverride>,
}

impl PolicyEngine {
    pub fn new(config: PolicyConfig) -> Self {
        Self {
            config,
            overrides: std::collections::HashMap::new(),
        }
    }

    /// Check if a DM from a user is allowed.
    pub fn check_dm(&self, open_id: &str) -> Access {
        match self.config.dm_policy.as_str() {
            "open" => Access::Allowed,
            "disabled" => Access::Denied("DMs are disabled".into()),
            _ => {
                // "allowlist" (default)
                if self.config.dm_allowlist.contains(&"*".to_string())
                    || self.config.dm_allowlist.iter().any(|id| id == open_id)
                {
                    Access::Allowed
                } else {
                    Access::Denied(format!(
                        "You are not authorized. Your open_id: {}. Ask the admin to add it to dm_allowlist.",
                        open_id
                    ))
                }
            }
        }
    }

    /// Check if a message in a group chat should be processed.
    pub fn check_group(&self, chat_id: &str, mentioned_bot: bool) -> Access {
        // Check per-chat override first
        if let Some(ov) = self.overrides.get(chat_id) {
            if let Some(false) = ov.enabled {
                return Access::Denied("This group is disabled".into());
            }
        }

        match self.config.group_policy.as_str() {
            "open" => {
                let require = self
                    .overrides
                    .get(chat_id)
                    .and_then(|o| o.require_mention)
                    .unwrap_or(self.config.require_mention);
                if require && !mentioned_bot {
                    return Access::Denied("Mention the bot to get a response".into());
                }
                Access::Allowed
            }
            "disabled" => {
                // Explicitly enabled groups still work
                if let Some(ov) = self.overrides.get(chat_id) {
                    if ov.enabled == Some(true) {
                        let require = ov
                            .require_mention
                            .unwrap_or(self.config.require_mention);
                        if require && !mentioned_bot {
                            return Access::Denied("Mention the bot to get a response".into());
                        }
                        return Access::Allowed;
                    }
                }
                Access::Denied("Group chat is disabled".into())
            }
            _ => {
                // "allowlist"
                if self.config.group_allowlist.contains(&"*".to_string())
                    || self.config.group_allowlist.iter().any(|id| id == chat_id)
                {
                    let require = self
                        .overrides
                        .get(chat_id)
                        .and_then(|o| o.require_mention)
                        .unwrap_or(self.config.require_mention);
                    if require && !mentioned_bot {
                        return Access::Denied("Mention the bot to get a response".into());
                    }
                    Access::Allowed
                } else {
                    Access::Denied(format!(
                        "This group ({}) is not in the allowlist",
                        chat_id
                    ))
                }
            }
        }
    }

    pub fn set_override(&mut self, chat_id: String, ov: ChatOverride) {
        self.overrides.insert(chat_id, ov);
    }

    pub fn remove_override(&mut self, chat_id: &str) {
        self.overrides.remove(chat_id);
    }
}
