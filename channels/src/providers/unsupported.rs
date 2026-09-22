//! A placeholder for a channel this build recognizes but cannot run.
//!
//! Declaring an unimplemented channel is useful — the CLI and docs can list it,
//! and a user who enables it gets a precise message instead of silence. It must
//! never look functional, which is why every entry point fails and the maturity
//! reads `planned`.

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use crate::bridge::ProviderCtx;
use crate::providers::traits::{ChannelDefinition, ChannelSender, Provider};

/// A provider whose implementation does not exist in this build.
pub struct Unsupported {
    definition: &'static ChannelDefinition,
}

impl Unsupported {
    /// Box a refusal for `definition`.
    pub fn boxed(definition: &'static ChannelDefinition) -> Box<dyn Provider> {
        Box::new(Self { definition })
    }

    fn refuse(&self) -> anyhow::Error {
        anyhow::anyhow!(
            "the {} channel is not implemented in this build",
            self.definition.id
        )
    }
}

#[async_trait]
impl Provider for Unsupported {
    fn definition(&self) -> &'static ChannelDefinition {
        self.definition
    }

    fn sender(&self, _ctx: &ProviderCtx) -> Result<Arc<dyn ChannelSender>> {
        Err(self.refuse())
    }

    async fn run(&self, _ctx: ProviderCtx) -> Result<()> {
        Err(self.refuse())
    }

    async fn probe(&self, _ctx: &ProviderCtx) -> Result<String> {
        Err(self.refuse())
    }
}

/// Declare a channel whose implementation has not been written yet.
macro_rules! planned_provider {
    ($definition:path) => {
        pub fn provider() -> Box<dyn $crate::providers::traits::Provider> {
            $crate::providers::unsupported::Unsupported::boxed(&$definition)
        }
    };
}

pub(crate) use planned_provider;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::traits::Maturity;

    static PLANNED: ChannelDefinition = ChannelDefinition {
        id: "planned",
        display_name: "Planned",
        description: "test double",
        docs: "",
        maturity: Maturity::Planned,
        capabilities: crate::providers::traits::Capabilities::TEXT,
        max_text_len: 100,
        length_unit: crate::transport::LengthUnit::Chars,
        config_example: r#"{"enabled": false}"#,
        requires: &[],
    };

    #[test]
    fn an_unsupported_channel_declares_itself_and_refuses_every_entry_point() {
        let provider = Unsupported::boxed(&PLANNED);
        assert_eq!(provider.definition().maturity, Maturity::Planned);
        assert!(!provider.definition().is_implemented());
    }

    #[tokio::test]
    async fn building_a_sender_or_probe_fails_with_a_readable_message() {
        let provider = Unsupported::boxed(&PLANNED);
        let ctx = crate::bridge::ProviderCtx::offline(&PLANNED);
        let error = provider.probe(&ctx).await.expect_err("probe must fail");
        assert!(error.to_string().contains("planned"), "{error}");
        assert!(error.to_string().contains("not implemented"), "{error}");
        assert!(provider.sender(&ctx).is_err());
        assert!(provider.run(ctx).await.is_err());
    }
}
