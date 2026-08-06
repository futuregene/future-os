//! Capability provider lifecycle state machine (G-23) — LoopX
//! `capabilities/registry.py`, natively.
//!
//! A provider moves through `declared → installed → enabled → ready` with
//! hard legality constraints:
//!
//! - `installed && !declared` is illegal (you cannot install an undeclared
//!   provider);
//! - `enabled && !installed` is illegal (you cannot enable an uninstalled
//!   provider);
//! - `ready && !enabled` is illegal (a disabled provider is never ready).
//!
//! Origins are validated against `builtin` / `extension`. Builtin providers
//! default to fully ready; extension providers start declared-only and reach
//! ready through install → enable → (doctor-verified) readiness.

use std::fmt;

/// LoopX CAPABILITY_ORIGINS.
pub const CAPABILITY_ORIGINS: [&str; 2] = ["builtin", "extension"];

/// LoopX CAPABILITY_VISIBILITIES.
pub const CAPABILITY_VISIBILITIES: [&str; 2] = ["public", "internal"];

/// The four lifecycle stages (LoopX registry.py provider lifecycle).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProviderStage {
    Declared,
    Installed,
    Enabled,
    Ready,
}

impl ProviderStage {
    pub fn label(&self) -> &'static str {
        match self {
            ProviderStage::Declared => "declared",
            ProviderStage::Installed => "installed",
            ProviderStage::Enabled => "enabled",
            ProviderStage::Ready => "ready",
        }
    }
}

impl fmt::Display for ProviderStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// The lifecycle truth of one capability provider (LoopX registry.py
/// normalizes every provider to these four booleans).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProviderLifecycle {
    pub declared: bool,
    pub installed: bool,
    pub enabled: bool,
    pub ready: bool,
}

impl ProviderLifecycle {
    /// Construct a lifecycle, enforcing the legality constraints (fail closed).
    pub fn new(
        declared: bool,
        installed: bool,
        enabled: bool,
        ready: bool,
    ) -> Result<Self, String> {
        let lc = Self {
            declared,
            installed,
            enabled,
            ready,
        };
        lc.validate()?;
        Ok(lc)
    }

    /// Validate the four booleans against the legality constraints.
    pub fn validate(&self) -> Result<(), String> {
        if self.installed && !self.declared {
            return Err("provider cannot be installed but undeclared".into());
        }
        if self.enabled && !self.installed {
            return Err("provider cannot be enabled but uninstalled".into());
        }
        if self.ready && !self.enabled {
            return Err("provider cannot be ready but disabled".into());
        }
        Ok(())
    }

    /// The derived stage (highest true flag).
    pub fn stage(&self) -> ProviderStage {
        if self.ready {
            ProviderStage::Ready
        } else if self.enabled {
            ProviderStage::Enabled
        } else if self.installed {
            ProviderStage::Installed
        } else {
            ProviderStage::Declared
        }
    }

    /// Default lifecycle for a provider record (LoopX: declared defaults
    /// true, installed defaults to `origin == "builtin"`, enabled defaults to
    /// installed, ready defaults to enabled) — then validated.
    pub fn for_origin(origin: &str, declared: bool, installed: bool) -> Result<Self, String> {
        let installed = if installed { true } else { origin == "builtin" };
        let enabled = installed;
        let ready = enabled;
        Self::new(declared, installed, enabled, ready)
    }

    /// declared → installed (installation requires a declared provider).
    pub fn install(&mut self) -> Result<(), String> {
        if !self.declared {
            return Err("cannot install an undeclared provider".into());
        }
        self.installed = true;
        Ok(())
    }

    /// installed → enabled.
    pub fn enable(&mut self) -> Result<(), String> {
        if !self.installed {
            return Err("cannot enable an uninstalled provider".into());
        }
        self.enabled = true;
        Ok(())
    }

    /// enabled → ready (readiness is granted by a doctor-verified entrypoint;
    /// the caller owns that check).
    pub fn mark_ready(&mut self) -> Result<(), String> {
        if !self.enabled {
            return Err("cannot mark a disabled provider ready".into());
        }
        self.ready = true;
        Ok(())
    }

    /// enabled → disabled (ready is cleared with it).
    pub fn disable(&mut self) -> Result<(), String> {
        self.enabled = false;
        self.ready = false;
        Ok(())
    }

    /// installed → removed (enabled/ready cleared).
    pub fn uninstall(&mut self) -> Result<(), String> {
        self.enabled = false;
        self.ready = false;
        self.installed = false;
        Ok(())
    }
}

/// A registered capability provider (LoopX registry.py register_provider).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CapabilityProvider {
    pub id: String,
    pub origin: String,
    pub version: Option<String>,
    pub lifecycle: ProviderLifecycle,
}

impl CapabilityProvider {
    /// Register a provider from raw flags (validates origin + lifecycle).
    pub fn new(
        id: &str,
        origin: &str,
        version: Option<String>,
        declared: bool,
        installed: bool,
    ) -> Result<Self, String> {
        if id.trim().is_empty() {
            return Err("provider requires a non-empty id".into());
        }
        if !CAPABILITY_ORIGINS.contains(&origin) {
            return Err(format!(
                "provider `{id}` has unsupported origin `{origin}`; expected one of {CAPABILITY_ORIGINS:?}"
            ));
        }
        let lifecycle = ProviderLifecycle::for_origin(origin, declared, installed)?;
        Ok(Self {
            id: id.to_string(),
            origin: origin.to_string(),
            version,
            lifecycle,
        })
    }

    /// A builtin provider is declared + installed + enabled + ready.
    pub fn builtin(id: &str) -> Self {
        Self::new(id, "builtin", None, true, true).expect("builtin lifecycle is legal")
    }

    /// An extension provider starts declared-only (origin validated).
    pub fn extension(id: &str, version: Option<String>) -> Self {
        Self::new(id, "extension", version, true, false).expect("extension lifecycle is legal")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_defaults_to_ready() {
        let p = CapabilityProvider::builtin("issue_fix");
        assert_eq!(p.lifecycle.stage(), ProviderStage::Ready);
        assert!(p.lifecycle.ready);
    }

    #[test]
    fn extension_starts_declared_only() {
        let mut p = CapabilityProvider::extension("ext-x", Some("1.0.0".into()));
        assert_eq!(p.lifecycle.stage(), ProviderStage::Declared);
        assert!(!p.lifecycle.installed);
        p.lifecycle.install().unwrap();
        assert_eq!(p.lifecycle.stage(), ProviderStage::Installed);
        p.lifecycle.enable().unwrap();
        assert_eq!(p.lifecycle.stage(), ProviderStage::Enabled);
        p.lifecycle.mark_ready().unwrap();
        assert_eq!(p.lifecycle.stage(), ProviderStage::Ready);
    }

    #[test]
    fn illegal_lifecycles_fail_closed() {
        assert!(ProviderLifecycle::new(false, true, false, false).is_err()); // installed&&!declared
        assert!(ProviderLifecycle::new(true, false, true, false).is_err()); // enabled&&!installed
        assert!(ProviderLifecycle::new(true, true, false, true).is_err()); // ready&&!enabled
        assert!(ProviderLifecycle::new(true, true, true, true).is_ok());
    }

    #[test]
    fn disable_clears_ready() {
        let mut p = CapabilityProvider::builtin("x");
        p.lifecycle.disable().unwrap();
        assert_eq!(p.lifecycle.stage(), ProviderStage::Installed);
        assert!(!p.lifecycle.enabled && !p.lifecycle.ready);
        p.lifecycle.enable().unwrap();
        p.lifecycle.mark_ready().unwrap();
        assert_eq!(p.lifecycle.stage(), ProviderStage::Ready);
    }

    #[test]
    fn unknown_origin_is_rejected() {
        assert!(CapabilityProvider::new("x", "evil", None, true, false).is_err());
    }

    #[test]
    fn transitions_enforce_stage_ordering() {
        let mut p = CapabilityProvider::extension("e", None);
        assert!(p.lifecycle.enable().is_err()); // cannot enable before install
        assert!(p.lifecycle.mark_ready().is_err());
        p.lifecycle.install().unwrap();
        assert!(p.lifecycle.mark_ready().is_err()); // cannot be ready before enabled
        p.lifecycle.enable().unwrap();
        p.lifecycle.mark_ready().unwrap();
        assert!(p.lifecycle.install().is_ok()); // re-install is idempotent (flag already true)
    }
}
