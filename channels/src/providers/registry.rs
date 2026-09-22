//! The channel registry.
//!
//! One line per channel: its declaration and a factory. The starter walks this
//! list, and `future channel list` walks it too, so the two cannot drift.
//!
//! A channel whose implementation is not finished publishes
//! [`Maturity::Planned`] and is skipped by the starter with an explicit
//! `unsupported` status — enabling it is reported, never silently ignored.

use crate::providers::traits::{ChannelDefinition, Provider};

/// One registered channel.
pub struct ProviderEntry {
    pub definition: &'static ChannelDefinition,
    /// Builds a fresh provider instance. The bridge builds one per start
    /// attempt, so a provider must not hold state that outlives a connection.
    pub provider: fn() -> Box<dyn Provider>,
}

/// Every framework-managed channel, in dependency-free order.
pub const PROVIDERS: &[ProviderEntry] = &[
    ProviderEntry {
        definition: &crate::providers::telegram::DEFINITION,
        provider: crate::providers::telegram::provider,
    },
    ProviderEntry {
        definition: &crate::providers::slack::DEFINITION,
        provider: crate::providers::slack::provider,
    },
    ProviderEntry {
        definition: &crate::providers::discord::DEFINITION,
        provider: crate::providers::discord::provider,
    },
    ProviderEntry {
        definition: &crate::providers::mattermost::DEFINITION,
        provider: crate::providers::mattermost::provider,
    },
    ProviderEntry {
        definition: &crate::providers::signal::DEFINITION,
        provider: crate::providers::signal::provider,
    },
    ProviderEntry {
        definition: &crate::providers::whatsapp::DEFINITION,
        provider: crate::providers::whatsapp::provider,
    },
    ProviderEntry {
        definition: &crate::providers::qq::DEFINITION,
        provider: crate::providers::qq::provider,
    },
    ProviderEntry {
        definition: &crate::providers::linq::DEFINITION,
        provider: crate::providers::linq::provider,
    },
    ProviderEntry {
        definition: &crate::providers::imessage::DEFINITION,
        provider: crate::providers::imessage::provider,
    },
    ProviderEntry {
        definition: &crate::providers::irc::DEFINITION,
        provider: crate::providers::irc::provider,
    },
    ProviderEntry {
        definition: &crate::providers::email::DEFINITION,
        provider: crate::providers::email::provider,
    },
    ProviderEntry {
        definition: &crate::providers::cli::DEFINITION,
        provider: crate::providers::cli::provider,
    },
];

/// The whole registry.
pub fn all() -> &'static [ProviderEntry] {
    PROVIDERS
}

/// One channel by id.
pub fn find(id: &str) -> Option<&'static ProviderEntry> {
    PROVIDERS.iter().find(|entry| entry.definition.id == id)
}

/// Channels this build can actually start.
pub fn implemented() -> Vec<&'static ProviderEntry> {
    PROVIDERS
        .iter()
        .filter(|entry| entry.definition.is_implemented())
        .collect()
}

/// Channels this build recognizes but cannot start.
pub fn planned() -> Vec<&'static ProviderEntry> {
    PROVIDERS
        .iter()
        .filter(|entry| !entry.definition.is_implemented())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::traits::Maturity;

    #[test]
    fn every_registered_channel_builds() {
        for entry in all() {
            let provider = (entry.provider)();
            assert_eq!(
                provider.definition().id,
                entry.definition.id,
                "factory and declaration disagree"
            );
        }
    }

    #[test]
    fn every_registered_channel_is_usable_or_declares_a_planned_maturity() {
        // Written over the whole registry rather than over the planned subset,
        // so it keeps asserting something whichever maturities this build ships:
        // "usable" and "declared planned" are the only two states a channel may
        // be in, and the two lists partition the registry.
        for entry in all() {
            let definition = entry.definition;
            let usable = definition.ensure_usable().is_ok();
            let planned = definition.maturity == Maturity::Planned;
            assert_eq!(usable, !planned, "{}", definition.id);
        }
        eval_planned_subset();
    }

    /// The refusal message itself is covered in `providers::traits`, where a
    /// planned definition can be constructed; here it is only aggregated.
    fn eval_planned_subset() {
        assert_eq!(implemented().len() + planned().len(), all().len());
    }

    #[test]
    fn find_is_exact_and_planned_and_implemented_partition_the_registry() {
        assert!(find("telegram").is_some());
        assert!(find("Telegram").is_none());
        assert_eq!(implemented().len() + planned().len(), all().len());
    }

    #[test]
    fn registry_ids_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for entry in all() {
            assert!(seen.insert(entry.definition.id), "{}", entry.definition.id);
        }
    }
}
