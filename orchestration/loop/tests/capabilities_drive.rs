//! Coverage drive for `capabilities/`: every builtin capability's propose
//! keyword matrix, the provider lifecycle state machine, and the catalog
//! registration validation arms.

use future_loop::capabilities::catalog::{CapabilityCatalog, CapabilityRecord};
use future_loop::capabilities::lifecycle::{
    CapabilityProvider, ProviderLifecycle, ProviderStage, CAPABILITY_ORIGINS,
};
use future_loop::capabilities::{CapabilityRegistry, ProposalKind};

fn propose(name: &str, input: &str) -> Vec<future_loop::capabilities::TypedProposal> {
    let registry = CapabilityRegistry::with_builtin();
    let cap = registry.get(name).unwrap_or_else(|| panic!("{name} registered"));
    cap.propose(input)
}

fn kinds(ps: &[future_loop::capabilities::TypedProposal]) -> Vec<ProposalKind> {
    ps.iter().map(|p| p.kind.clone()).collect()
}

// ── keyword routers ────────────────────────────────────────────────────────

#[test]
fn simple_router_capabilities() {
    // (capability, [inputs that hit each rule arm])
    let cases: &[(&str, &[&str])] = &[
        ("material_lifecycle", &["archive it", "归档", "废弃", "nothing relevant", ""]),
        ("change_quality", &["all pass + diff written", "tests fail", "all pass", ""]),
        ("explore", &["hypothesis here", "假设", "探索", "none", ""]),
        ("integration_branch", &["branch it", "集成", "none", ""]),
        ("value_connectors", &["connector plan", "连接", "none", ""]),
        ("reward_memory", &["reward this", "评价", "none", ""]),
        ("semantic_preference", &["prefer dark mode", "偏好", "none", ""]),
        ("decision_context", &["decide this", "决策", "none", ""]),
        ("agent_turn_recall", &["recall the turn", "回忆", "none", ""]),
        ("content_ops", &["content publish", "发布", "none", ""]),
        ("context_providers", &["provider context", "上下文", "none", ""]),
        ("periodic_report", &["report weekly", "报告", "none", ""]),
        ("auto_research", &["research this", "调研", "none", ""]),
    ];
    for (name, inputs) in cases {
        for input in *inputs {
            let ps = propose(name, input);
            assert!(!ps.is_empty(), "{name} / {input:?}");
        }
    }
}

// ── issue_fix deep branches ────────────────────────────────────────────────

#[test]
fn issue_fix_branch_matrix() {
    // Empty.
    assert_eq!(kinds(&propose("issue_fix", "")), vec![ProposalKind::NoFollowUp]);
    // Full signal (repro+error+expected) → investigate/fix/validate trio.
    let ps = propose(
        "issue_fix",
        "title: crash on empty input\nerror: panicked at src/main.rs:42\nrepro: run with no args\nexpected: prints usage and exits 0\nscope: cli",
    );
    assert_eq!(ps.len(), 3, "{ps:?}");
    // The `scope`/`expected` metadata keys parse into the observation.
    // Authority read-only gates BEFORE any fix.
    assert_eq!(
        kinds(&propose("issue_fix", "title: t\nerror: e\nrepro: r\nauthority: read-only")),
        vec![ProposalKind::Gate]
    );
    // Regression with repro+error → bounded repair.
    let ps = propose(
        "issue_fix",
        "title: regression\nerror: tests fail\nrepro: run ci\nbody: this previously worked; a regression returned",
    );
    assert!(ps[0].reason.to_lowercase().contains("repair"), "{ps:?}");
    // Partial signal (exactly one of repro/error/expected, enough words) → investigate.
    let ps = propose(
        "issue_fix",
        "error: the thing broke in a way that takes many words to describe properly here",
    );
    assert_eq!(ps.len(), 1);
    assert!(ps[0].reason.contains("investigate"), "{ps:?}");
    // Too little signal → triage.
    let ps = propose("issue_fix", "it broke");
    assert!(ps[0].todo.as_ref().unwrap().text.contains("Triage"), "{ps:?}");
}

// ── provider lifecycle ─────────────────────────────────────────────────────

#[test]
fn provider_lifecycle_machine() {
    for (stage, label) in [
        (ProviderStage::Declared, "declared"),
        (ProviderStage::Installed, "installed"),
        (ProviderStage::Enabled, "enabled"),
        (ProviderStage::Ready, "ready"),
    ] {
        assert_eq!(stage.label(), label);
        assert_eq!(format!("{stage}"), label);
    }
    // for_origin: builtin auto-installs; external requires declared.
    let lc = ProviderLifecycle::for_origin("builtin", true, false).unwrap();
    assert!(lc.installed && lc.enabled && lc.ready);
    let _ = ProviderLifecycle::for_origin("external", false, false);
    // Transitions + error arms.
    let mut lc = ProviderLifecycle::for_origin("external", true, false).unwrap();
    assert!(!lc.installed);
    lc.install().unwrap();
    assert!(lc.installed);
    lc.enable().unwrap();
    lc.disable().unwrap();
    assert!(!lc.enabled && !lc.ready);
    lc.uninstall().unwrap();
    assert!(!lc.installed);
    let mut undeclared = ProviderLifecycle {
        declared: false,
        installed: false,
        enabled: false,
        ready: false,
    };
    assert!(undeclared.install().is_err());
    // Provider registration validation.
    assert!(CapabilityProvider::new("", "builtin", None, true, false).is_err());
    assert!(CapabilityProvider::new("p", "unsupported-origin", None, true, false).is_err());
    let p = CapabilityProvider::new("p", "builtin", Some("1.0".into()), true, false).unwrap();
    assert_eq!(p.version.as_deref(), Some("1.0"));
    assert!(CAPABILITY_ORIGINS.contains(&"builtin"));
}

// ── catalog registration validation ────────────────────────────────────────

fn record(id: &str) -> CapabilityRecord {
    CapabilityRecord {
        id: id.to_string(),
        title: "T".into(),
        status: "available".into(),
        stage: 1,
        provider_id: "future-loop".into(),
        origin: "builtin".into(),
        visibility: "public".into(),
        user_value: "v".into(),
        next_real_step: "s".into(),
        commands: vec![],
        packets: vec![],
    }
}

#[test]
fn catalog_registration_errors() {
    let mut catalog = CapabilityCatalog::with_builtin();
    // Empty id.
    let mut r = record("");
    assert!(catalog.register_capability(r.clone()).is_err());
    // Empty required field (title).
    let mut r = record("cap-x");
    r.title = "  ".into();
    assert!(catalog.register_capability(r).is_err());
    // Bad origin / visibility.
    let mut r = record("cap-x");
    r.origin = "nope".into();
    assert!(catalog.register_capability(r).is_err());
    let mut r = record("cap-x");
    r.visibility = "nope".into();
    assert!(catalog.register_capability(r).is_err());
    // Unknown provider.
    let mut r = record("cap-x");
    r.provider_id = "ghost-provider".into();
    assert!(catalog.register_capability(r).is_err());
    // Origin mismatch with the provider.
    let mut r = record("cap-x");
    r.origin = "external".into();
    assert!(catalog.register_capability(r).is_err());
    // Duplicate id (an existing builtin).
    let r = record("issue_fix");
    assert!(catalog.register_capability(r).is_err());
    // provider_lifecycle_for: unknown capability → None; builtin → Some.
    assert!(catalog.provider_lifecycle_for("cap-ghost").is_none());
    assert!(catalog.provider_lifecycle_for("issue_fix").is_some());
    // provider lookup miss/hit.
    assert!(catalog.provider("ghost").is_none());
    assert!(!catalog.providers().is_empty());
}
