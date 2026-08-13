//! G-23 capability lifecycle + catalog contract tests: the provider
//! lifecycle state machine (declared → installed → enabled → ready with
//! legality constraints), the 15-record builtin catalog (14 domain packs +
//! pr_review_queue experimental), and catalog queryability (status / stage /
//! provider / commands / packets).

use future_loop::capabilities::catalog::CapabilityCatalog;
use future_loop::capabilities::lifecycle::{CapabilityProvider, ProviderLifecycle, ProviderStage};

#[test]
fn builtin_catalog_has_15_records_with_future_loop_statuses() {
    let catalog = CapabilityCatalog::with_builtin();
    assert_eq!(catalog.records(false).len(), 15);
    assert_eq!(catalog.providers().len(), 1);
    // reference catalog status alignment.
    assert_eq!(catalog.get("issue_fix").unwrap().status, "active-preview");
    assert_eq!(catalog.get("auto_research").unwrap().status, "experimental");
    assert_eq!(
        catalog.get("decision_context").unwrap().status,
        "experimental"
    );
    assert_eq!(
        catalog.get("material_lifecycle").unwrap().status,
        "experimental"
    );
    assert_eq!(
        catalog.get("value_connectors").unwrap().status,
        "compatibility-facade"
    );
    assert_eq!(
        catalog.get("periodic_report").unwrap().status,
        "active-preview"
    );
    // The 15th capability shipped its P2-3 rule version as active-preview.
    let pr = catalog.get("pr_review_queue").unwrap();
    assert_eq!(pr.status, "active-preview");
    assert!(!pr.is_experimental());
}

#[test]
fn catalog_is_queryable_by_status_stage_provider() {
    let catalog = CapabilityCatalog::with_builtin();
    let record = catalog.get("issue_fix").unwrap();
    assert_eq!(record.provider_id, "future-loop-core");
    assert_eq!(record.origin, "builtin");
    assert_eq!(record.stage, 0);
    assert!(record.is_public());
    assert!(!record.commands.is_empty());
    assert!(!record.packets.is_empty());
    assert_eq!(
        catalog.provider_lifecycle_for("issue_fix").unwrap().stage(),
        ProviderStage::Ready
    );
    // Every public record carries the reference required fields.
    for record in catalog.records(false) {
        assert!(
            !record.user_value.trim().is_empty(),
            "{} user_value",
            record.id
        );
        assert!(
            !record.next_real_step.trim().is_empty(),
            "{} next_real_step",
            record.id
        );
        assert!(!record.title.trim().is_empty(), "{} title", record.id);
    }
}

#[test]
fn lifecycle_transitions_enforce_legality_constraints() {
    // installed && !declared → illegal
    assert!(ProviderLifecycle::new(false, true, false, false).is_err());
    // enabled && !installed → illegal
    assert!(ProviderLifecycle::new(true, false, true, false).is_err());
    // ready && !enabled → illegal
    assert!(ProviderLifecycle::new(true, true, false, true).is_err());
    // legal full chain
    assert!(ProviderLifecycle::new(true, true, true, true).is_ok());
}

#[test]
fn extension_provider_stage_progression() {
    let mut provider = CapabilityProvider::extension("ext-x", Some("1.0.0".into()));
    assert_eq!(provider.origin, "extension");
    assert_eq!(provider.lifecycle.stage(), ProviderStage::Declared);
    provider.lifecycle.install().unwrap();
    assert_eq!(provider.lifecycle.stage(), ProviderStage::Installed);
    provider.lifecycle.enable().unwrap();
    assert_eq!(provider.lifecycle.stage(), ProviderStage::Enabled);
    provider.lifecycle.mark_ready().unwrap();
    assert_eq!(provider.lifecycle.stage(), ProviderStage::Ready);
    // disable drops ready
    provider.lifecycle.disable().unwrap();
    assert_eq!(provider.lifecycle.stage(), ProviderStage::Installed);
    assert!(!provider.lifecycle.ready);
}

#[test]
fn unknown_origin_and_duplicate_provider_fail_closed() {
    assert!(CapabilityProvider::new("x", "evil", None, true, false).is_err());
    let mut catalog = CapabilityCatalog::new();
    catalog
        .register_provider(CapabilityProvider::builtin("p"))
        .unwrap();
    assert!(catalog
        .register_provider(CapabilityProvider::builtin("p"))
        .is_err());
    // Capability referencing an unknown provider is rejected.
    let bad = future_loop::capabilities::catalog::CapabilityRecord {
        id: "c".into(),
        title: "t".into(),
        status: "active-preview".into(),
        stage: 0,
        provider_id: "nope".into(),
        origin: "builtin".into(),
        visibility: "public".into(),
        user_value: "v".into(),
        next_real_step: "n".into(),
        commands: vec![],
        packets: vec![],
    };
    assert!(catalog.register_capability(bad).is_err());
}

#[test]
fn experimental_capability_commands_are_gated() {
    let catalog = CapabilityCatalog::with_builtin();
    // Hidden by default.
    assert!(catalog.commands_for("auto_research", false).is_empty());
    assert!(catalog.commands_for("material_lifecycle", false).is_empty());
    // Visible with the include flag (G-24 gate).
    assert_eq!(catalog.commands_for("material_lifecycle", true).len(), 1);
    assert_eq!(catalog.commands_for("auto_research", true).len(), 1);
    // pr_review_queue shipped its P2-3 rule version → active-preview visible.
    assert_eq!(catalog.commands_for("pr_review_queue", false).len(), 1);
    assert_eq!(catalog.commands_for("issue_fix", false).len(), 1);
}

#[test]
fn provider_catalog_composition_with_extension_provider() {
    let mut catalog = CapabilityCatalog::new();
    catalog
        .register_provider(CapabilityProvider::extension("ext-p", Some("2.0.0".into())))
        .unwrap();
    catalog
        .register_capability(future_loop::capabilities::catalog::CapabilityRecord {
            id: "ext-cap".into(),
            title: "Extension Cap".into(),
            status: "active-preview".into(),
            stage: 0,
            provider_id: "ext-p".into(),
            origin: "extension".into(),
            visibility: "public".into(),
            user_value: "v".into(),
            next_real_step: "n".into(),
            commands: vec![],
            packets: vec![],
        })
        .unwrap();
    assert_eq!(catalog.get("ext-cap").unwrap().origin, "extension");
    // Origin mismatch between capability and provider is rejected.
    let bad = future_loop::capabilities::catalog::CapabilityRecord {
        id: "ext-cap2".into(),
        title: "t".into(),
        status: "active-preview".into(),
        stage: 0,
        provider_id: "ext-p".into(),
        origin: "builtin".into(),
        visibility: "public".into(),
        user_value: "v".into(),
        next_real_step: "n".into(),
        commands: vec![],
        packets: vec![],
    };
    assert!(catalog.register_capability(bad).is_err());
}
