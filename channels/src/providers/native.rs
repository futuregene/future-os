//! Channels that keep their own bridge: Feishu and DingTalk.
//!
//! These two shipped first and carry years of platform-specific behaviour
//! (interactive cards, streaming card elements, approval buttons, slash
//! commands) in their own modules. They are declared here so the CLI and the
//! docs describe the complete product, and so a future migration onto the
//! shared bridge has a single place to flip.

use crate::providers::traits::{Capabilities, ChannelDefinition, Maturity};
use crate::transport::LengthUnit;

pub static FEISHU: ChannelDefinition = ChannelDefinition {
    id: "feishu",
    display_name: "Feishu / Lark",
    description:
        "Bidirectional Feishu (Lark) bot with streaming interactive cards and approval buttons.",
    docs: "docs/guide/channels-config.md#feishu",
    maturity: Maturity::Live,
    capabilities: Capabilities {
        receive: true,
        send: true,
        edit: true,
        threads: true,
        typing: true,
        reactions: true,
        media_in: true,
        media_out: false,
        mention_gate: true,
    },
    max_text_len: 4000,
    length_unit: LengthUnit::Chars,
    config_example: r#"{
  "enabled": false,
  "app_id": "",
  "app_secret": "",
  "streaming": true
}"#,
    requires: &[],
};

pub static DINGTALK: ChannelDefinition = ChannelDefinition {
    id: "dingtalk",
    display_name: "DingTalk",
    description: "DingTalk Stream-mode bot with markdown replies.",
    docs: "docs/guide/channels-config.md#dingtalk",
    maturity: Maturity::Live,
    capabilities: Capabilities {
        receive: true,
        send: true,
        edit: false,
        threads: false,
        typing: false,
        reactions: false,
        media_in: false,
        media_out: false,
        mention_gate: true,
    },
    max_text_len: 4000,
    length_unit: LengthUnit::Chars,
    config_example: r#"{
  "enabled": false,
  "client_id": "",
  "client_secret": ""
}"#,
    requires: &[],
};

/// Declarations for the self-bridged channels.
pub fn definitions() -> Vec<&'static ChannelDefinition> {
    vec![&FEISHU, &DINGTALK]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_native_definitions_are_usable_and_distinct() {
        let definitions = definitions();
        assert_eq!(definitions.len(), 2);
        assert_ne!(definitions[0].id, definitions[1].id);
        assert!(definitions
            .iter()
            .all(|definition| definition.is_implemented()));
    }

    #[test]
    fn their_config_examples_parse_and_carry_an_enabled_flag() {
        for definition in definitions() {
            let example: serde_json::Value =
                serde_json::from_str(definition.config_example).expect("valid JSON");
            assert_eq!(example["enabled"], serde_json::Value::Bool(false));
        }
    }
}
