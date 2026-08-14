//! Contract tests for the content_ops capability (Wave 2 G2 deepening):
//! classification → deterministic quality/length signals → the finite
//! ordered-operation vocabulary mapped onto the proposal set, plus the
//! state-surface validation contract.

use future_loop::capabilities::content_ops::{
    classify_content, quality_signals, validate_content_ops_surface, ContentKind,
};
use future_loop::capabilities::{CapabilityRegistry, ProposalKind};

#[test]
fn registry_exposes_content_ops() {
    let registry = CapabilityRegistry::with_builtin();
    let cap = registry.get("content_ops").expect("content_ops registered");
    assert_eq!(cap.name(), "content_ops");
    assert!(!cap.describe().is_empty());
}

#[test]
fn empty_input_is_no_follow_up() {
    let registry = CapabilityRegistry::with_builtin();
    let proposals = registry.get("content_ops").unwrap().propose("   ");
    assert_eq!(proposals.len(), 1);
    assert_eq!(proposals[0].kind, ProposalKind::NoFollowUp);
}

#[test]
fn outline_input_proposes_a_draft_successor() {
    let registry = CapabilityRegistry::with_builtin();
    let proposals = registry
        .get("content_ops")
        .unwrap()
        .propose("# outline\n- point one\n- point two\n- point three");
    assert!(
        proposals
            .iter()
            .any(|p| p.kind == ProposalKind::SuccessorTodo
                && p.reason == "content_ops_draft_from_angle"),
        "{proposals:?}"
    );
}

#[test]
fn private_material_raises_a_user_gate_first() {
    let registry = CapabilityRegistry::with_builtin();
    let proposals = registry
        .get("content_ops")
        .unwrap()
        .propose("draft mentioning the password and a secret token");
    assert_eq!(proposals[0].kind, ProposalKind::Gate);
    assert_eq!(proposals[0].reason, "content_ops_source_boundary");
    assert!(proposals[0]
        .gate_question
        .as_deref()
        .unwrap()
        .contains("boundaries"));
}

#[test]
fn connector_report_raises_owner_gate() {
    let registry = CapabilityRegistry::with_builtin();
    let proposals = registry
        .get("content_ops")
        .unwrap()
        .propose("connector report channel_count=3 record_count=9");
    assert!(proposals.iter().any(|p| p.kind == ProposalKind::Gate));
}

#[test]
fn classification_vocabulary_is_stable() {
    let url = classify_content("https://example.com/post");
    assert_eq!(url.kind, ContentKind::Url);
    let draft = classify_content(
        "The team decided to ship the release next week. Every member agreed \
         that the timeline is reasonable. The documentation team will update \
         the user guide in three parts. The first part covers installation \
         steps. The second part explains configuration options in detail. \
         The third part describes the new pricing model. Support engineers \
         will then verify each example against the current build. The final \
         announcement goes out after all checks pass and the changelog is \
         complete. Several other tasks remain on the board but none of them \
         block this delivery target and the team is confident about the schedule.",
    );
    assert_eq!(draft.kind, ContentKind::Draft);
    let feedback = classify_content("prefer the shorter angle for this draft");
    assert_eq!(feedback.kind, ContentKind::Feedback);
}

#[test]
fn quality_signals_are_bounded_and_deterministic() {
    let signals = quality_signals("one two three four five six", ContentKind::Draft);
    assert_eq!(signals.words, 6);
    assert_eq!(signals.lines, 1);
    assert!(
        (0..=100).contains(&signals.score),
        "score {} out of range",
        signals.score
    );
    // Deterministic: same input → same signals.
    let again = quality_signals("one two three four five six", ContentKind::Draft);
    assert_eq!(signals.score, again.score);
}

#[test]
fn empty_surface_fails_validation_with_boundary_requirement() {
    let surface = serde_json::json!({});
    let validation = validate_content_ops_surface(&surface);
    assert!(!validation.ok);
    assert!(
        validation.errors.iter().any(|e| e.contains("boundary")),
        "{:?}",
        validation.errors
    );
}

#[test]
fn invalid_surface_proposal_is_a_repair_gate() {
    let registry = CapabilityRegistry::with_builtin();
    let proposals = registry
        .get("content_ops")
        .unwrap()
        .propose("{\"source_items\": []}");
    assert_eq!(proposals.len(), 1);
    assert_eq!(proposals[0].kind, ProposalKind::Gate);
    assert!(proposals[0]
        .gate_question
        .as_deref()
        .unwrap()
        .contains("Repair the content-ops surface"));
}

#[test]
fn non_json_garbage_degrades_gracefully() {
    let registry = CapabilityRegistry::with_builtin();
    // Starts with '{' but is not JSON → falls through to free-text path
    // (Unrecognized → register-source successor).
    let proposals = registry
        .get("content_ops")
        .unwrap()
        .propose("{ not really json");
    assert!(!proposals.is_empty());
    assert!(
        proposals
            .iter()
            .all(|p| matches!(p.kind, ProposalKind::SuccessorTodo | ProposalKind::Gate)),
        "{proposals:?}"
    );
}
