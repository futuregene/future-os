//! Access policy: who may talk to the agent, in which conversations.
//!
//! One engine, shared by every channel, so a provider never re-implements the
//! decision of whether a message is answerable. Three knobs decide it:
//!
//! * **DM policy** — `open` / `disabled` / `allowlist`.
//! * **Group policy** — `open` / `disabled` / `allowlist`.
//! * **Mention gate** — inside an allowed group, does the bot need to be
//!   addressed before it replies?
//!
//! Policy is dynamic, not just configuration: a conversation can be switched
//! on or off at runtime with [`PolicyEngine::set_override`], which is how
//! `/enable`-style commands and "this group is disabled" replies work.

use serde::{Deserialize, Serialize};

/// Verdict for one inbound message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    Allowed,
    Denied(String),
}

/// Per-conversation policy override applied on top of the configured policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatOverride {
    /// `Some(false)` silences the conversation even when the policy allows it.
    pub enabled: Option<bool>,
    /// Per-conversation mention gate, overriding the global default.
    pub require_mention: Option<bool>,
}

/// The configured policy for one channel.
///
/// Field names match the `dm_policy` / `group_policy` blocks users already
/// write in `config.json`, so a channel's config deserializes straight into it.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccessPolicyConfig {
    #[serde(default = "default_dm_policy")]
    pub dm_policy: String,
    #[serde(default)]
    pub dm_allowlist: Vec<String>,
    #[serde(default = "default_group_policy")]
    pub group_policy: String,
    #[serde(default)]
    pub group_allowlist: Vec<String>,
    #[serde(default = "default_require_mention")]
    pub require_mention: bool,
}

fn default_dm_policy() -> String {
    "allowlist".into()
}

fn default_group_policy() -> String {
    "disabled".into()
}

fn default_require_mention() -> bool {
    true
}

impl Default for AccessPolicyConfig {
    fn default() -> Self {
        Self {
            dm_policy: default_dm_policy(),
            dm_allowlist: Vec::new(),
            group_policy: default_group_policy(),
            group_allowlist: Vec::new(),
            require_mention: default_require_mention(),
        }
    }
}

pub struct PolicyEngine {
    config: AccessPolicyConfig,
    overrides: std::collections::HashMap<String, ChatOverride>,
}

impl PolicyEngine {
    pub fn new(config: AccessPolicyConfig) -> Self {
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
                        "You are not authorized. Your sender id: {}. Ask the admin to add it to dm_allowlist.",
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
                        let require = ov.require_mention.unwrap_or(self.config.require_mention);
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
                        "This group ({}) is not in the group_allowlist",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn config(
        dm_policy: &str,
        dm_allowlist: &[&str],
        group_policy: &str,
        group_allowlist: &[&str],
        require_mention: bool,
    ) -> AccessPolicyConfig {
        AccessPolicyConfig {
            dm_policy: dm_policy.to_string(),
            dm_allowlist: dm_allowlist.iter().map(|s| s.to_string()).collect(),
            group_policy: group_policy.to_string(),
            group_allowlist: group_allowlist.iter().map(|s| s.to_string()).collect(),
            require_mention,
        }
    }

    fn override_with(enabled: Option<bool>, require_mention: Option<bool>) -> ChatOverride {
        ChatOverride {
            enabled,
            require_mention,
        }
    }

    // ─── DM policy ─────────────────────────────────────────────────────────

    #[test]
    fn dm_open_allows_anyone() {
        let engine = PolicyEngine::new(config("open", &[], "open", &[], false));
        assert_eq!(engine.check_dm("ou_anyone"), Access::Allowed);
    }

    #[test]
    fn dm_disabled_denies_even_allowlisted() {
        let engine = PolicyEngine::new(config("disabled", &["ou_alice"], "open", &[], false));
        assert!(matches!(engine.check_dm("ou_alice"), Access::Denied(_)));
    }

    #[test]
    fn dm_allowlist_allows_member_and_denies_stranger() {
        let engine = PolicyEngine::new(config("allowlist", &["ou_alice"], "open", &[], false));
        assert_eq!(engine.check_dm("ou_alice"), Access::Allowed);
        // The denial message tells the user their open_id so an admin can add
        // it — keep that contract stable.
        assert!(
            matches!(engine.check_dm("ou_mallory"), Access::Denied(ref reason) if reason.contains("ou_mallory")),
            "stranger should be denied with their open_id in the message"
        );
    }

    #[test]
    fn dm_allowlist_wildcard_allows_everyone() {
        let engine = PolicyEngine::new(config("allowlist", &["*"], "open", &[], false));
        assert_eq!(engine.check_dm("ou_anyone"), Access::Allowed);
    }

    #[test]
    fn dm_unknown_policy_falls_back_to_allowlist() {
        // Any unrecognized policy string is treated as "allowlist" (default-deny).
        let engine = PolicyEngine::new(config("bogus", &["ou_alice"], "open", &[], false));
        assert_eq!(engine.check_dm("ou_alice"), Access::Allowed);
        assert!(matches!(engine.check_dm("ou_bob"), Access::Denied(_)));
    }

    // ─── Group policy ──────────────────────────────────────────────────────

    #[test]
    fn group_open_without_mention_requirement_allows() {
        let engine = PolicyEngine::new(config("open", &[], "open", &[], false));
        assert_eq!(engine.check_group("oc_chat", false), Access::Allowed);
    }

    #[test]
    fn group_open_with_mention_requirement() {
        let engine = PolicyEngine::new(config("open", &[], "open", &[], true));
        assert!(matches!(
            engine.check_group("oc_chat", false),
            Access::Denied(_)
        ));
        assert_eq!(engine.check_group("oc_chat", true), Access::Allowed);
    }

    #[test]
    fn group_disabled_denies_by_default() {
        let engine = PolicyEngine::new(config("open", &[], "disabled", &[], false));
        assert!(matches!(
            engine.check_group("oc_chat", true),
            Access::Denied(_)
        ));
    }

    #[test]
    fn group_disabled_but_override_enabled_allows() {
        let mut engine = PolicyEngine::new(config("open", &[], "disabled", &[], true));
        engine.set_override("oc_chat".into(), override_with(Some(true), None));
        // require_mention still applies (falls back to global true)
        assert!(matches!(
            engine.check_group("oc_chat", false),
            Access::Denied(_)
        ));
        assert_eq!(engine.check_group("oc_chat", true), Access::Allowed);
    }

    #[test]
    fn override_disabled_wins_over_open_policy() {
        let mut engine = PolicyEngine::new(config("open", &[], "open", &[], false));
        engine.set_override("oc_chat".into(), override_with(Some(false), None));
        assert!(matches!(
            engine.check_group("oc_chat", true),
            Access::Denied(_)
        ));
    }

    #[test]
    fn group_allowlist_member_and_wildcard() {
        let engine = PolicyEngine::new(config("open", &[], "allowlist", &["oc_a"], false));
        assert_eq!(engine.check_group("oc_a", false), Access::Allowed);
        assert!(matches!(
            engine.check_group("oc_b", false),
            Access::Denied(_)
        ));

        let wild = PolicyEngine::new(config("open", &[], "allowlist", &["*"], false));
        assert_eq!(wild.check_group("oc_b", false), Access::Allowed);
    }

    #[test]
    fn group_allowlist_member_must_mention_when_required() {
        let engine = PolicyEngine::new(config("open", &[], "allowlist", &["oc_a"], true));
        assert!(matches!(
            engine.check_group("oc_a", false),
            Access::Denied(_)
        ));
        assert_eq!(engine.check_group("oc_a", true), Access::Allowed);
    }

    #[test]
    fn group_disabled_with_non_enabling_override_stays_denied() {
        // An override that does not explicitly enable the chat (enabled=None
        // or Some(false)) must not punch through a disabled group policy.
        let mut engine = PolicyEngine::new(config("open", &[], "disabled", &[], false));
        engine.set_override("oc_chat".into(), override_with(None, Some(false)));
        assert!(matches!(
            engine.check_group("oc_chat", true),
            Access::Denied(_)
        ));
        engine.set_override("oc_chat".into(), override_with(Some(false), None));
        assert!(matches!(
            engine.check_group("oc_chat", true),
            Access::Denied(_)
        ));
    }

    #[test]
    fn override_require_mention_beats_global() {
        let mut engine = PolicyEngine::new(config("open", &[], "open", &[], true));
        engine.set_override("oc_chat".into(), override_with(None, Some(false)));
        assert_eq!(engine.check_group("oc_chat", false), Access::Allowed);
    }

    #[test]
    fn remove_override_restores_global_behavior() {
        let mut engine = PolicyEngine::new(config("open", &[], "open", &[], false));
        engine.set_override("oc_chat".into(), override_with(Some(false), None));
        assert!(matches!(
            engine.check_group("oc_chat", false),
            Access::Denied(_)
        ));
        engine.remove_override("oc_chat");
        assert_eq!(engine.check_group("oc_chat", false), Access::Allowed);
    }
}
