//! Channel definitions and the registry that drives them.
//!
//! Every channel has a [`ChannelDefinition`] — its id, capabilities, message
//! limits, and how complete it is — so the CLI, the docs and the starter can
//! all describe the same set without duplicating a list. Adding a channel means
//! adding one module here and one line in [`registry`].

pub mod registry;
pub mod traits;

/// The channel provider implementations.
pub mod cli;
pub mod discord;
pub mod email;
pub mod imessage;
pub mod irc;
pub mod linq;
pub mod mattermost;
pub mod qq;
pub mod signal;
pub mod slack;
pub mod telegram;
pub mod wecom;
pub mod whatsapp;

mod native;
mod unsupported;

pub use traits::{Capabilities, ChannelDefinition, ChannelSender, Maturity, Provider};

/// Declarations for the channels that keep their own bridge (Feishu, DingTalk).
pub use native::definitions as native_definitions;

/// Every channel this build knows about, in display order.
///
/// Includes the channels implemented by their own bridge (`feishu`, `dingtalk`)
/// so `future channel list` describes the whole product, not just the ones the
/// framework starts.
pub fn all_definitions() -> Vec<&'static ChannelDefinition> {
    let mut definitions: Vec<&'static ChannelDefinition> = registry::all()
        .iter()
        .map(|entry| entry.definition)
        .collect();
    definitions.extend(native::definitions());
    definitions
}

/// Look up one channel's declaration by id.
pub fn definition(id: &str) -> Option<&'static ChannelDefinition> {
    all_definitions()
        .into_iter()
        .find(|definition| definition.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The anchor GitHub derives from a markdown heading: lower-cased,
    /// punctuation dropped, spaces turned into hyphens. Backticks disappear,
    /// so a heading written ``### `feishu` `` anchors at `#feishu`.
    fn heading_slug(line: &str) -> Option<String> {
        let heading = line.strip_prefix('#')?.trim_start_matches('#').trim();
        let kept: String = heading
            .to_lowercase()
            .chars()
            .filter(|character| character.is_alphanumeric() || " -_".contains(*character))
            .collect();
        Some(kept.replace(' ', "-"))
    }

    /// The workspace root, so a `docs/...` path resolves the way it does for a
    /// reader.
    fn workspace_root() -> &'static std::path::Path {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("the crate sits inside the workspace")
    }

    /// Why one `docs` value leads nowhere, or `None` when it resolves.
    fn docs_problem(root: &std::path::Path, docs: &str) -> Option<String> {
        if docs.is_empty() {
            return Some("declares no documentation".to_string());
        }
        let (path_text, anchor) = match docs.split_once('#') {
            Some((path, anchor)) => (path, Some(anchor)),
            None => (docs, None),
        };
        let path = root.join(path_text);
        let Ok(content) = std::fs::read_to_string(&path) else {
            return Some(format!(
                "points at `{docs}`, which cannot be read; expected a file at {}",
                path.display()
            ));
        };
        let anchor = anchor?;
        let found = content
            .lines()
            .filter_map(heading_slug)
            .any(|candidate| candidate == anchor);
        (!found).then(|| format!("points at `{docs}`, but that document has no such heading"))
    }

    /// Which of `definitions` are not documented anywhere reachable.
    ///
    /// Takes the definitions rather than reading the registry, so each failure
    /// mode can be exercised with a definition built for it. Reporting only
    /// ever runs when something is wrong, so a loop over the real registry
    /// would leave its own failure lines uncovered — and a check that is never
    /// observed failing is not known to work.
    fn undocumented(definitions: &[&ChannelDefinition], root: &std::path::Path) -> Vec<String> {
        let mut problems = Vec::new();
        for definition in definitions {
            if let Some(problem) = docs_problem(root, definition.docs) {
                problems.push(format!("{} {problem}", definition.id));
            }
        }
        problems
    }

    /// A declaration whose only interesting property is its `docs` value.
    fn documenting(id: &'static str, docs: &'static str) -> ChannelDefinition {
        ChannelDefinition {
            id,
            display_name: "Probe",
            description: "test double",
            docs,
            maturity: crate::providers::traits::Maturity::Preview,
            capabilities: crate::providers::traits::Capabilities::TEXT,
            max_text_len: 100,
            length_unit: crate::transport::LengthUnit::Chars,
            config_example: "{}",
            requires: &[],
        }
    }

    #[test]
    fn every_channel_documents_itself_somewhere_that_exists() {
        // `docs` is user-facing: it is what `future channel list --json`
        // reports. A path nobody wrote reads as documentation from the outside
        // while sending the reader nowhere, which is how all fourteen channels
        // shipped pointing at per-channel pages that were never created. The
        // crate's tests are the only place this is enforced automatically —
        // the documentation gate is run by hand.
        let problems = undocumented(&all_definitions(), workspace_root());
        let report = problems.join("\n");
        assert!(problems.is_empty(), "{report}");
    }

    #[test]
    fn a_docs_target_is_rejected_when_it_cannot_be_followed() {
        // The three ways `docs` can lie, each with a declaration of its own so
        // the invariant above is not the only thing asserting this.
        let root = workspace_root();
        let good = documenting("good", "docs/guide/channels-providers.md#telegram");
        let anchored = documenting("good-bare", "docs/guide/channels-providers.md");
        let elsewhere = documenting("gone", "docs/guide/channels-telegram.md");
        let wrong_anchor = documenting("typo", "docs/guide/channels-providers.md#telegram-typo");
        let silent = documenting("silent", "");

        let problems = undocumented(&[&good, &anchored], root);
        assert!(problems.is_empty(), "resolvable targets: {problems:?}");

        let problems = undocumented(&[&good, &elsewhere, &wrong_anchor], root);
        assert_eq!(problems.len(), 2, "{problems:?}");
        let missing_file = &problems[0];
        let missing_anchor = &problems[1];
        let named = missing_file.starts_with("gone points at `docs/guide/channels-telegram.md`");
        assert!(named, "a missing file must be named: {missing_file}");
        let explained = missing_file.contains("which cannot be read");
        assert!(explained, "{missing_file}");
        let distinguished = missing_anchor.contains("no such heading");
        assert!(
            distinguished,
            "an anchor that resolves nowhere reads differently: {missing_anchor}"
        );

        let problems = undocumented(&[&silent], root);
        assert_eq!(problems, vec!["silent declares no documentation"]);
    }

    #[test]
    fn an_anchor_is_the_slug_github_makes() {
        assert_eq!(heading_slug("### `feishu`").as_deref(), Some("feishu"));
        assert_eq!(
            heading_slug("### iMessage (macOS)").as_deref(),
            Some("imessage-macos")
        );
        assert_eq!(heading_slug("#a  b").as_deref(), Some("a--b"));
        // Not a heading at all.
        assert_eq!(heading_slug("text"), None);
    }

    #[test]
    fn every_channel_id_is_unique() {
        let mut seen = std::collections::HashSet::new();
        for definition in all_definitions() {
            assert!(
                seen.insert(definition.id),
                "duplicate channel id: {}",
                definition.id
            );
        }
    }

    #[test]
    fn every_channel_is_describable() {
        for definition in all_definitions() {
            assert!(!definition.display_name.is_empty(), "{}", definition.id);
            assert!(!definition.description.is_empty(), "{}", definition.id);
            assert!(!definition.docs.is_empty(), "{}", definition.id);
            assert!(definition.max_text_len > 0, "{}", definition.id);
            // A minimal config example must at least be valid JSON with the
            // `enabled` flag every provider reads.
            let example: serde_json::Value = serde_json::from_str(definition.config_example)
                .unwrap_or_else(|error| panic!("{}: {error}", definition.id));
            assert!(
                example.get("enabled").is_some(),
                "{} example needs an `enabled` flag",
                definition.id
            );
        }
    }

    #[test]
    fn a_channel_is_usable_exactly_when_its_maturity_is_not_planned() {
        for definition in all_definitions() {
            let usable = definition.ensure_usable().is_ok();
            let planned = definition.maturity == Maturity::Planned;
            assert_eq!(usable, !planned, "{}", definition.id);
        }
    }

    #[test]
    fn definition_lookup_finds_known_and_rejects_unknown() {
        assert_eq!(definition("telegram").map(|d| d.id), Some("telegram"));
        assert!(definition("nope").is_none());
    }

    #[test]
    fn the_native_bridges_are_still_described() {
        // feishu and dingtalk run their own mature bridges; they must still show
        // up in `future channel list`.
        for id in ["feishu", "dingtalk"] {
            let definition = definition(id).unwrap_or_else(|| panic!("{id} missing"));
            assert!(definition.is_implemented(), "{id} should be usable");
        }
    }
}
