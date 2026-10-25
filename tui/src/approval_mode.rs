//! Approval mode: the four values a session can run under, and their wire form.
//!
//! Kept separate from the (deleted) sandbox panel because the workbench needs
//! exactly this much of it — the launchpad's mode chip cycles it, the status bar
//! prints it — while the panel's rendering, descriptions and platform probes are
//! gone with the inline UI.

/// The product approval mode a session runs under — the four values the
/// desktop's `ApprovalTier` and the CLI's `--sandbox` share. `Auto` is not a
/// fourth tier on the wire: it is `tier = sandbox` plus `reviewer = model`
/// (the agent's `SandboxPolicy::mode()` spelling).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApprovalMode {
    /// `"manual"` — approval rules on, no OS sandbox, the human approves.
    Manual,
    /// `"sandbox"` — approval rules on, shell inside the OS sandbox, the human
    /// approves what the sandbox lets through.
    Sandbox,
    /// `"auto"` — the sandbox tier plus the model reviewer.
    Auto,
    /// `"off"` — no approval, no sandbox, everything runs.
    Off,
}

impl ApprovalMode {
    /// Display order of the picker, matching the desktop composer dropdown.
    pub const ALL: [ApprovalMode; 4] = [
        ApprovalMode::Manual,
        ApprovalMode::Sandbox,
        ApprovalMode::Auto,
        ApprovalMode::Off,
    ];

    /// The canonical mode name — the wire value the agent stores and the
    /// spelling persisted in settings (`manual` | `sandbox` | `auto` | `off`).
    pub fn as_str(self) -> &'static str {
        match self {
            ApprovalMode::Manual => "manual",
            ApprovalMode::Sandbox => "sandbox",
            ApprovalMode::Auto => "auto",
            ApprovalMode::Off => "off",
        }
    }

    /// The `(tier, reviewer)` pair the agent's `set_sandbox_policy` carries.
    pub fn to_wire(self) -> (&'static str, &'static str) {
        match self {
            ApprovalMode::Manual => ("manual", "user"),
            ApprovalMode::Sandbox => ("sandbox", "user"),
            ApprovalMode::Auto => ("sandbox", "model"),
            ApprovalMode::Off => ("off", "user"),
        }
    }

    /// The mode string the agent round-trips (`SandboxPolicy::mode()`).
    pub fn from_wire(raw: &str) -> Option<ApprovalMode> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "manual" => Some(ApprovalMode::Manual),
            "sandbox" => Some(ApprovalMode::Sandbox),
            "auto" => Some(ApprovalMode::Auto),
            "off" => Some(ApprovalMode::Off),
            _ => None,
        }
    }

    /// The label shown in the mode picker (desktop `approvalTier.*`).
    pub fn label(self) -> &'static str {
        match self {
            ApprovalMode::Manual => "Manual",
            ApprovalMode::Sandbox => "Sandboxed",
            ApprovalMode::Auto => "Automatic review",
            ApprovalMode::Off => "Full access",
        }
    }

    /// The short badge the status bar prints (`manual` / `sandbox` / `auto` /
    /// `full`). A display spelling only — never persist it.
    pub fn badge(self) -> &'static str {
        match self {
            ApprovalMode::Manual => "manual",
            ApprovalMode::Sandbox => "sandbox",
            ApprovalMode::Auto => "auto",
            ApprovalMode::Off => "full",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_round_trips_through_its_wire_spelling() {
        for mode in ApprovalMode::ALL {
            assert_eq!(ApprovalMode::from_wire(mode.as_str()), Some(mode));
            // Case and padding are tolerated: both arrive from a settings file
            // a human may have edited.
            assert_eq!(
                ApprovalMode::from_wire(&format!(" {} ", mode.as_str().to_uppercase())),
                Some(mode)
            );
        }
        assert_eq!(ApprovalMode::from_wire("turbo"), None);
        assert_eq!(ApprovalMode::from_wire(""), None);
    }

    #[test]
    fn auto_is_the_sandbox_tier_with_the_model_reviewer() {
        // `auto` is not a fourth tier: the desktop and the CLI agree on this
        // pair, and the agent stores it as such.
        assert_eq!(ApprovalMode::Auto.to_wire(), ("sandbox", "model"));
        assert_eq!(ApprovalMode::Manual.to_wire(), ("manual", "user"));
        assert_eq!(ApprovalMode::Off.to_wire(), ("off", "user"));
    }

    #[test]
    fn the_badge_is_a_display_spelling_not_a_value() {
        assert_eq!(ApprovalMode::Off.badge(), "full");
        assert_eq!(ApprovalMode::Off.as_str(), "off");
        assert_eq!(ApprovalMode::Auto.label(), "Automatic review");
    }
}
