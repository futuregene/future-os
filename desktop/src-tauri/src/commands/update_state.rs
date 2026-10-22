//! Shared transaction protocol. Platform adapters implement preparation and
//! commit; only a new, healthy application may confirm an installed version.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Stage {
    Checking,
    Downloading,
    Verified,
    Preparing,
    Prepared,
    Cleanup,
    AwaitingExit,
    Installing,
    AwaitingValidation,
    Finished,
}
impl Stage {
    pub(super) fn parse(value: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(value.into())).ok()
    }
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Checking => "checking",
            Self::Downloading => "downloading",
            Self::Verified => "verified",
            Self::Preparing => "preparing",
            Self::Prepared => "prepared",
            Self::Cleanup => "cleanup",
            Self::AwaitingExit => "awaiting-exit",
            Self::Installing => "installing",
            Self::AwaitingValidation => "awaiting-validation",
            Self::Finished => "finished",
        }
    }
    pub(super) fn program_untouched(self) -> bool {
        matches!(
            self,
            Self::Checking
                | Self::Downloading
                | Self::Verified
                | Self::Preparing
                | Self::Prepared
                | Self::Cleanup
        )
    }
    pub(super) fn permits(self, next: Self) -> bool {
        // Repeating a durable write is idempotent. Going backwards can hide a
        // partial install and must never make an operation look retryable.
        (next as u8) >= (self as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_distinguishes_staging_from_destructive_work() {
        for stage in [
            Stage::Checking,
            Stage::Downloading,
            Stage::Verified,
            Stage::Preparing,
            Stage::Prepared,
            Stage::Cleanup,
        ] {
            assert!(stage.program_untouched());
        }
        for stage in [
            Stage::AwaitingExit,
            Stage::Installing,
            Stage::AwaitingValidation,
        ] {
            assert!(!stage.program_untouched());
        }
        assert!(!Stage::Installing.permits(Stage::Prepared));
        assert!(Stage::Installing.permits(Stage::AwaitingValidation));
        assert!(Stage::parse("unknown").is_none());
    }
}
