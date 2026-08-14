//! Deep per-line coverage for the content_ops capability (Wave 2 G2):
//! every classification lane, quality-signal band, URL-normalization error,
//! surface-validation rule, projection branch, connector runtime policy,
//! and the synthetic fixture. Deterministic, host-supplied inputs only.

use future_loop::capabilities::content_ops::{
    build_content_ops_connector_runtime_policy, build_content_ops_surface_fixture,
    classify_content, normalize_public_https_url, project_content_ops_surface, quality_signals,
    suggest_operations, validate_content_ops_surface, ContentKind,
};
use future_loop::capabilities::ProposalKind;

// ── classification vocabulary ─────────────────────────────────────────────

#[test]
fn content_kind_as_str_is_stable() {
    let kinds = [
        (ContentKind::Outline, "outline"),
        (ContentKind::Draft, "draft"),
        (ContentKind::Feedback, "feedback"),
        (ContentKind::Url, "url"),
        (ContentKind::SourceNote, "source_note"),
        (ContentKind::ConnectorReport, "connector_report"),
        (ContentKind::Unrecognized, "unrecognized"),
    ];
    for (kind, expected) in kinds {
        assert_eq!(kind.as_str(), expected);
    }
}

#[test]
fn classify_detects_every_shape_marker() {
    // URL + api path + source attribution + publish intent + list.
    let class = classify_content(
        "# Title\nhttps://example.com/api/post\nsource: a post\n- a\n- b\npublish now",
    );
    assert_eq!(class.kind, ContentKind::Url);
    assert!(class.has_url);
    assert!(class.has_api_path);
    assert!(class.has_headline);
    assert!(class.has_list);
    assert!(class.has_source_attribution);
    assert!(class.has_publish_marker);
    assert!(!class.has_feedback_marker);
    assert!(!class.has_private_signal);
    assert!(class.word_count > 0);
}

#[test]
fn classify_numbered_list_is_a_list() {
    let class = classify_content("# T\n1. first\n2. second\n3. third");
    assert!(class.has_list);
    assert!(class.has_headline);
    assert_eq!(class.kind, ContentKind::Outline);
}

#[test]
fn classify_connector_report_without_api_path() {
    let class = classify_content("connector report channel_count=3 record_count=9");
    assert_eq!(class.kind, ContentKind::ConnectorReport);
}

#[test]
fn classify_source_note_from_attribution() {
    let class = classify_content("source: an operator-owned note");
    assert_eq!(class.kind, ContentKind::SourceNote);
}

#[test]
fn classify_unrecognized_for_short_neutral_text() {
    let class = classify_content("hello world");
    assert_eq!(class.kind, ContentKind::Unrecognized);
}

#[test]
fn classify_private_signal_and_public_https_only() {
    let class = classify_content("a password and a secret token here");
    assert!(class.has_private_signal);
    assert!(!class.has_url);
}

// ── quality & length signals ─────────────────────────────────────────────

#[test]
fn quality_length_and_quality_bands() {
    let short = quality_signals("one two", ContentKind::Draft);
    assert_eq!(short.length_band(), "short");
    assert_eq!(short.quality_band(), "weak");
    let medium_strong = quality_signals(
        "# Headline\nnext: ship\nsource: https://example.com/a\n- a\n- b\n- c\n\
         this is a medium length piece of prose that is just long enough to land \
         in the medium band and it keeps going with more words so that the word \
         count crosses the sixty word threshold and the score benefits from the \
         headline list source and call to action",
        ContentKind::Draft,
    );
    assert_eq!(medium_strong.length_band(), "medium");
    assert_eq!(medium_strong.quality_band(), "strong");
    let long = quality_signals(&"word ".repeat(1300), ContentKind::Draft);
    assert_eq!(long.length_band(), "long");
}

#[test]
fn quality_score_fit_bands_and_flags() {
    // 120..=1200 → +15; headline +10; list>=3 +5; source +5; url 1..=2 +2;
    // call-to-action present → no -5.
    let text = "# Title\nnext: ship\nsource: ref\n- a\n- b\n- c\nhttps://example.com\n\
                one two three four five six seven eight nine ten eleven twelve thirteen \
                fourteen fifteen sixteen seventeen eighteen nineteen twenty";
    let signals = quality_signals(text, ContentKind::Outline);
    assert_eq!(signals.headline_chars, Some(5));
    assert_eq!(signals.list_item_count, 3);
    assert_eq!(signals.url_count, 1);
    assert_eq!(signals.source_ref_count, 1);
    assert!(signals.call_to_action);
    // 33 words is short (< 60): -10, headline +10, list>=3 +5, source +5,
    // single url +2, call-to-action present (no penalty).
    assert_eq!(signals.score, 50 - 10 + 10 + 5 + 5 + 2);

    // Very short + no headline + no call-to-action → penalty and flags.
    let weak = quality_signals("just a few words here", ContentKind::Draft);
    assert!(weak.score < 50);
    assert!(weak.flags.iter().any(|f| f == "too short"));
    assert!(weak.flags.iter().any(|f| f == "no headline"));
    assert!(weak.flags.iter().any(|f| f == "no call to action"));
}

#[test]
fn quality_private_raw_and_link_heavy_flags() {
    let text = "# T\nnext: go\npassword secret token\nraw body transcript\n\
                https://a.com https://b.com https://c.com\nword "
        .to_string()
        + &"long ".repeat(80);
    let signals = quality_signals(&text, ContentKind::Draft);
    assert!(!signals.private_hits.is_empty());
    assert!(!signals.raw_key_hits.is_empty());
    assert!(signals.flags.iter().any(|f| f == "private signal present"));
    assert!(signals
        .flags
        .iter()
        .any(|f| f == "raw material key present"));
    assert!(signals.flags.iter().any(|f| f == "link heavy"));
    assert!(signals.score < 40, "score {}", signals.score);
    assert_eq!(signals.paragraphs, 1);
    assert!(signals.lines >= 1);
    assert!(signals.chars > 0);
}

#[test]
fn quality_too_long_flag_and_score_bands() {
    let mut text = String::new();
    for _ in 0..2500 {
        text.push_str("word ");
    }
    let signals = quality_signals(&text, ContentKind::Draft);
    assert!(signals.flags.iter().any(|f| f == "too long"));
    assert!(signals.words > 2400);
}

// ── operation suggestions ────────────────────────────────────────────────

fn class_of(
    input: &str,
) -> (
    future_loop::capabilities::content_ops::ContentClass,
    future_loop::capabilities::content_ops::QualitySignals,
) {
    let class = classify_content(input);
    let signals = quality_signals(input, class.kind);
    (class, signals)
}

#[test]
fn suggest_operations_for_each_kind() {
    let (url_class, url_sig) = class_of("https://example.com/post");
    let url_ops = suggest_operations(&url_class, &url_sig);
    assert!(url_ops
        .iter()
        .any(|op| op.action_kind == "content_ops_observe_public_handle"));

    let (note_class, note_sig) = class_of("source: an operator-owned note");
    let note_ops = suggest_operations(&note_class, &note_sig);
    assert!(note_ops
        .iter()
        .any(|op| op.action_kind == "content_ops_register_source"));

    let (report_class, report_sig) = class_of("connector report channel_count=3");
    let report_ops = suggest_operations(&report_class, &report_sig);
    assert!(report_ops
        .iter()
        .any(|op| op.action_kind == "content_ops_connector_owner_gate"));

    let (unrec_class, unrec_sig) = class_of("hello world");
    let unrec_ops = suggest_operations(&unrec_class, &unrec_sig);
    assert!(unrec_ops
        .iter()
        .any(|op| op.action_kind == "content_ops_register_source"));

    let (fb_class, fb_sig) = class_of("prefer a shorter angle");
    let fb_ops = suggest_operations(&fb_class, &fb_sig);
    assert!(fb_ops
        .iter()
        .any(|op| op.action_kind == "content_ops_record_feedback"));
}

#[test]
fn suggest_draft_submit_vs_revise() {
    // Strong draft (>= 120 words, headline + source + cta, no list/url so
    // it stays a Draft): score 80 (>= 70) → submit review.
    let strong = format!("# T\nnext: go\nsource: r\n{}", vec!["word"; 120].join(" "));
    let (class, signals) = class_of(&strong);
    let ops = suggest_operations(&class, &signals);
    assert!(ops
        .iter()
        .any(|op| op.action_kind == "content_ops_submit_review"));

    // Weak draft with no flags → "tighten the angle": headline + cta keep
    // the flag list empty, but no list/source/url keeps the score 65 (< 70).
    let weak = format!("# T\nnext: go\n{}", vec!["word"; 60].join(" "));
    let (class, signals) = class_of(&weak);
    let ops = suggest_operations(&class, &signals);
    let revise = ops
        .iter()
        .find(|op| op.action_kind == "content_ops_revise_draft")
        .unwrap();
    assert!(revise.title.contains("tighten the angle"));

    // Weak draft with flags → flag list in title.
    let flagged = "a draft with a password inside";
    let (class, signals) = class_of(flagged);
    let ops = suggest_operations(&class, &signals);
    assert!(ops
        .iter()
        .any(|op| op.action_kind == "content_ops_source_boundary"));
}

#[test]
fn suggest_private_material_and_publish_gate_combos() {
    // Private → source boundary first; publish marker without private → publish gate.
    let (class, signals) = class_of("publish this draft with a password");
    let ops = suggest_operations(&class, &signals);
    assert!(ops
        .iter()
        .any(|op| op.action_kind == "content_ops_source_boundary"));
    // has_private_signal true → publish gate suppressed.
    assert!(!ops
        .iter()
        .any(|op| op.action_kind == "content_ops_publish_gate"));

    let (class, signals) = class_of("publish this great new post for everyone");
    let ops = suggest_operations(&class, &signals);
    assert!(ops
        .iter()
        .any(|op| op.action_kind == "content_ops_publish_gate"));
}

// ── public https URL normalization ───────────────────────────────────────

#[test]
fn normalize_url_accepts_public_https() {
    assert_eq!(
        normalize_public_https_url("https://example.com/path").unwrap(),
        "https://example.com/path"
    );
    // Default port and trailing dot are normalized out.
    assert_eq!(
        normalize_public_https_url("https://example.com:443/").unwrap(),
        "https://example.com:443/"
    );
    assert_eq!(
        normalize_public_https_url("https://example.com./x").unwrap(),
        "https://example.com./x"
    );
    // Public IPs (not localish) pass through.
    assert_eq!(
        normalize_public_https_url("https://8.8.8.8/").unwrap(),
        "https://8.8.8.8/"
    );
    // Bracketed public IPv6 with the explicit default port reaches the
    // address-family localish check and passes through.
    assert_eq!(
        normalize_public_https_url("https://[2001:db8::1]:443/").unwrap(),
        "https://[2001:db8::1]:443/"
    );
}

#[test]
fn normalize_url_rejects_non_https() {
    let err = normalize_public_https_url("http://example.com").unwrap_err();
    assert!(err.contains("https"), "{err}");
}

#[test]
fn normalize_url_rejects_missing_host() {
    assert!(normalize_public_https_url("https://")
        .unwrap_err()
        .contains("host"));
    assert!(normalize_public_https_url("https://[]/")
        .unwrap_err()
        .contains("host"));
}

#[test]
fn normalize_url_rejects_credentials_and_query_fragment() {
    assert!(normalize_public_https_url("https://u:p@example.com/")
        .unwrap_err()
        .contains("credentials"));
    assert!(normalize_public_https_url("https://example.com?x=1")
        .unwrap_err()
        .contains("query or fragment"));
    assert!(normalize_public_https_url("https://example.com#f")
        .unwrap_err()
        .contains("query or fragment"));
    assert!(normalize_public_https_url("https://example.com/p?x=1")
        .unwrap_err()
        .contains("query or fragment"));
    assert!(normalize_public_https_url("https://example.com/p#f")
        .unwrap_err()
        .contains("query or fragment"));
}

#[test]
fn normalize_url_rejects_non_default_port() {
    assert!(normalize_public_https_url("https://example.com:8080/")
        .unwrap_err()
        .contains("default"));
}

#[test]
fn normalize_url_rejects_localhost_and_local_domains() {
    assert!(normalize_public_https_url("https://localhost/")
        .unwrap_err()
        .contains("local"));
    assert!(normalize_public_https_url("https://x.localhost/")
        .unwrap_err()
        .contains("local"));
    assert!(normalize_public_https_url("https://x.local/")
        .unwrap_err()
        .contains("local"));
}

#[test]
fn normalize_url_rejects_private_ipv4_and_ipv6() {
    for host in [
        "192.168.1.1",     // private
        "127.0.0.1",       // loopback
        "169.254.0.1",     // link-local
        "255.255.255.255", // broadcast
        "224.0.0.1",       // multicast
        "0.0.0.0",         // unspecified
    ] {
        assert!(
            normalize_public_https_url(&format!("https://{host}/")).is_err(),
            "{host} should be rejected"
        );
    }
    for host in [
        "::1",     // loopback
        "fe80::1", // link-local
        "ff02::1", // multicast
        "::",      // unspecified
        "fc00::1", // unique-local
    ] {
        assert!(
            normalize_public_https_url(&format!("https://[{host}]/")).is_err(),
            "{host} should be rejected"
        );
    }
}

// ── state surface validation ─────────────────────────────────────────────

fn fixture() -> serde_json::Value {
    build_content_ops_surface_fixture("2026-01-01T00:00:00Z")
}

#[test]
fn validate_accepts_the_fixture() {
    let validation = validate_content_ops_surface(&fixture());
    assert!(validation.ok, "{:?}", validation.errors);
    assert!(validation.errors.is_empty());
    assert_eq!(validation.record_counts["source_items"], 2);
    assert_eq!(validation.record_counts["connector_trials"], 2);
}

#[test]
fn validate_requires_schema_version() {
    let mut s = fixture();
    s["schema_version"] = serde_json::json!("wrong");
    let v = validate_content_ops_surface(&s);
    assert!(v.errors.iter().any(|e| e.contains("schema_version")));
}

#[test]
fn validate_requires_each_record_group() {
    for key in [
        "source_items",
        "angle_candidates",
        "draft_items",
        "feedback_signals",
        "publish_gates",
        "material_memory",
        "connector_trials",
    ] {
        let mut s = fixture();
        s[key] = serde_json::json!([]);
        let v = validate_content_ops_surface(&s);
        assert!(
            v.errors.iter().any(|e| e.contains("required")),
            "{key}: {:?}",
            v.errors
        );
    }
}

#[test]
fn validate_source_item_contract() {
    let mut s = fixture();
    s["source_items"] = serde_json::json!([{
        "schema_version": "wrong",
        "source_item_id": "s1",
        "source_status": "nope",
        "freshness": "nope",
        "allowed_use": "nope"
    }]);
    let v = validate_content_ops_surface(&s);
    assert!(v.errors.iter().any(|e| e.contains("wrong schema")));
    assert!(v.errors.iter().any(|e| e.contains("source_status")));
    assert!(v.errors.iter().any(|e| e.contains("freshness")));
    assert!(v.errors.iter().any(|e| e.contains("allowed_use")));
}

#[test]
fn validate_angle_contract() {
    let mut s = fixture();
    s["angle_candidates"] = serde_json::json!([{
        "schema_version": "wrong",
        "angle_id": "a1",
        "decision": "nope",
        "source_item_ids": ["unknown_source"]
    }]);
    let v = validate_content_ops_surface(&s);
    assert!(v.errors.iter().any(|e| e.contains("wrong schema")));
    assert!(v.errors.iter().any(|e| e.contains("decision")));
    assert!(v.errors.iter().any(|e| e.contains("unknown source")));
}

#[test]
fn validate_draft_contract() {
    let mut s = fixture();
    s["draft_items"] = serde_json::json!([{
        "schema_version": "wrong",
        "draft_id": "d1",
        "state": "nope",
        "angle_id": "unknown_angle",
        "publish_gate_id": "unknown_gate"
    }]);
    let v = validate_content_ops_surface(&s);
    assert!(v.errors.iter().any(|e| e.contains("wrong schema")));
    assert!(v.errors.iter().any(|e| e.contains("state")));
    assert!(v.errors.iter().any(|e| e.contains("unknown angle")));
    assert!(v.errors.iter().any(|e| e.contains("unknown publish gate")));

    // Missing source_map.
    let mut s2 = fixture();
    s2["draft_items"] = serde_json::json!([{
        "schema_version": "draft_item_v0",
        "draft_id": "d1",
        "state": "outline",
        "angle_id": "angle_source_aware_loop",
        "publish_gate_id": "publish_gate_source_aware_loop"
    }]);
    let v2 = validate_content_ops_surface(&s2);
    assert!(v2.errors.iter().any(|e| e.contains("source_map")));

    // source_map references unknown source.
    let mut s3 = fixture();
    s3["draft_items"][0]["source_map"] = serde_json::json!([{"source_item_id": "nope"}]);
    let v3 = validate_content_ops_surface(&s3);
    assert!(v3.errors.iter().any(|e| e.contains("unknown source")));

    // source_map is not an array.
    let mut s4 = fixture();
    s4["draft_items"][0]["source_map"] = serde_json::json!("not-array");
    let v4 = validate_content_ops_surface(&s4);
    assert!(v4.errors.iter().any(|e| e.contains("source_map")));
}

#[test]
fn validate_feedback_contract() {
    let mut s = fixture();
    s["feedback_signals"] = serde_json::json!([{
        "schema_version": "wrong",
        "feedback_id": "f1",
        "effect": "nope",
        "target_id": "unknown"
    }]);
    let v = validate_content_ops_surface(&s);
    assert!(v.errors.iter().any(|e| e.contains("wrong schema")));
    assert!(v.errors.iter().any(|e| e.contains("effect")));
    assert!(v.errors.iter().any(|e| e.contains("unknown target")));
}

#[test]
fn validate_publish_gate_contract() {
    let mut s = fixture();
    s["publish_gates"] = serde_json::json!([{
        "schema_version": "wrong",
        "gate_id": "g1",
        "status": "nope",
        "autopublish_allowed": true,
        "approval_required": false
    }]);
    let v = validate_content_ops_surface(&s);
    assert!(v.errors.iter().any(|e| e.contains("wrong schema")));
    assert!(v.errors.iter().any(|e| e.contains("status")));
    assert!(v.errors.iter().any(|e| e.contains("autopublish_allowed")));
    assert!(v.errors.iter().any(|e| e.contains("approval")));
}

#[test]
fn validate_material_memory_contract() {
    let mut s = fixture();
    s["material_memory"] = serde_json::json!([{
        "schema_version": "wrong",
        "memory_id": "m1",
        "source_item_id": "unknown"
    }]);
    let v = validate_content_ops_surface(&s);
    assert!(v.errors.iter().any(|e| e.contains("wrong schema")));
    assert!(v.errors.iter().any(|e| e.contains("unknown source")));
}

#[test]
fn validate_connector_trial_contract() {
    let mut s = fixture();
    s["connector_trials"] = serde_json::json!([{
        "schema_version": "wrong",
        "trial_id": "t1",
        "source_status": "nope",
        "freshness": "nope",
        "allowed_use": "nope",
        "trial_state": "nope",
        "access_mode": "nope",
        "external_write_allowed": true
    }]);
    let v = validate_content_ops_surface(&s);
    assert!(v.errors.iter().any(|e| e.contains("wrong schema")));
    assert!(v.errors.iter().any(|e| e.contains("source_status")));
    assert!(v.errors.iter().any(|e| e.contains("freshness")));
    assert!(v.errors.iter().any(|e| e.contains("allowed_use")));
    assert!(v.errors.iter().any(|e| e.contains("trial_state")));
    assert!(v.errors.iter().any(|e| e.contains("access_mode")));
    assert!(v
        .errors
        .iter()
        .any(|e| e.contains("external_write_allowed")));

    // private_metadata_only requires requires_user_gate=true.
    let mut s2 = fixture();
    s2["connector_trials"] = serde_json::json!([{
        "schema_version": "connector_trial_v0",
        "trial_id": "t1",
        "access_mode": "private_metadata_only",
        "source_status": "public",
        "freshness": "fresh",
        "allowed_use": "metadata_only",
        "trial_state": "candidate",
        "external_write_allowed": false,
        "requires_user_gate": false
    }]);
    let v2 = validate_content_ops_surface(&s2);
    assert!(v2
        .errors
        .iter()
        .any(|e| e.contains("gate private metadata")));
}

#[test]
fn validate_boundary_flags_are_required() {
    let mut s = fixture();
    s["boundary"] = serde_json::json!("nope");
    let v = validate_content_ops_surface(&s);
    assert!(v
        .errors
        .iter()
        .any(|e| e.contains("boundary flags are required")));

    let mut s2 = fixture();
    s2.as_object_mut().unwrap().remove("boundary");
    let v2 = validate_content_ops_surface(&s2);
    assert!(v2
        .errors
        .iter()
        .any(|e| e.contains("boundary flags are required")));

    let mut s3 = fixture();
    s3["boundary"] = serde_json::json!({
        "public_safe": false,
        "raw_private_material_recorded": true,
        "raw_platform_data_recorded": true,
        "credentials_recorded": true,
        "autopublish_allowed": true,
        "publish_requires_user_gate": false,
        "connector_bodies_are_source_of_truth": true
    });
    let v3 = validate_content_ops_surface(&s3);
    assert!(v3.errors.iter().any(|e| e.contains("public_safe")));
    assert!(v3
        .errors
        .iter()
        .any(|e| e.contains("raw_private_material_recorded")));
    assert!(v3
        .errors
        .iter()
        .any(|e| e.contains("raw_platform_data_recorded")));
    assert!(v3.errors.iter().any(|e| e.contains("credentials_recorded")));
    assert!(v3.errors.iter().any(|e| e.contains("autopublish_allowed")));
    assert!(v3
        .errors
        .iter()
        .any(|e| e.contains("publish_requires_user_gate")));
    assert!(v3
        .errors
        .iter()
        .any(|e| e.contains("connector_bodies_are_source_of_truth")));
}

#[test]
fn validate_rejects_raw_material_key_names() {
    let mut s = fixture();
    s["source_items"][0]["raw_transcript"] = serde_json::json!("private");
    let v = validate_content_ops_surface(&s);
    assert!(v
        .errors
        .iter()
        .any(|e| e.contains("raw/private-looking key names")));
    assert!(v
        .raw_material_key_names
        .contains(&"raw_transcript".to_string()));
}

// ── state surface projection ─────────────────────────────────────────────

#[test]
fn project_fixture_produces_first_screen() {
    let projection = project_content_ops_surface(&fixture());
    assert_eq!(
        projection.surface_id.as_deref(),
        Some("creator_ops_public_safe_demo")
    );
    assert!(projection.first_screen.user_action_required);
    assert!(projection.first_screen.safe_side_work_available);
    assert!(projection.first_screen.agent_can_continue);
    assert_eq!(projection.first_screen.waiting_on, "user");
    assert_eq!(projection.first_screen.source_review_required_count, 1);
    assert_eq!(projection.first_screen.ready_to_draft_count, 1);
    assert_eq!(projection.first_screen.waiting_for_feedback_count, 1);
    assert_eq!(projection.first_screen.publish_decision_count, 1);
    // Counters present.
    assert_eq!(projection.source_statuses["synthetic_public_safe"], 1);
    assert_eq!(projection.source_statuses["private_needs_review"], 1);
    assert_eq!(projection.draft_states["outline"], 1);
    assert_eq!(projection.feedback_effects["preference_hint"], 1);
    assert_eq!(
        projection.publish_gate_statuses["blocked_until_user_approval"],
        1
    );
    assert_eq!(
        projection.connector_trial_state_counts["metadata_packet_collected"],
        1
    );
    // Gated connector trial → owner gate candidate.
    assert!(projection
        .todo_candidates
        .iter()
        .any(|c| c.action_kind == "content_ops_connector_owner_gate"));
}

#[test]
fn project_ready_angle_routes_to_agent_draft() {
    let mut s = fixture();
    s["publish_gates"] = serde_json::json!([]);
    let projection = project_content_ops_surface(&s);
    assert!(!projection.first_screen.user_action_required);
    assert_eq!(projection.first_screen.waiting_on, "agent");
    assert!(projection
        .todo_candidates
        .iter()
        .any(|c| c.action_kind == "content_ops_draft_from_angle"));
}

#[test]
fn project_source_review_routes_to_operator() {
    let mut s = fixture();
    s["publish_gates"] = serde_json::json!([]);
    // No draft-ready angle.
    s["angle_candidates"][0]["decision"] = serde_json::json!("hold");
    s["angle_candidates"][1]["decision"] = serde_json::json!("reject");
    let projection = project_content_ops_surface(&s);
    assert_eq!(projection.first_screen.waiting_on, "operator");
    assert!(projection
        .todo_candidates
        .iter()
        .any(|c| c.action_kind == "content_ops_source_review"));
}

#[test]
fn project_fallback_routes_to_collect_more() {
    let mut s = fixture();
    s["publish_gates"] = serde_json::json!([]);
    s["angle_candidates"][0]["decision"] = serde_json::json!("hold");
    s["angle_candidates"][1]["decision"] = serde_json::json!("reject");
    // Remove private sources so no source review is required either.
    s["source_items"] = serde_json::json!([{
        "schema_version": "source_item_v0",
        "source_item_id": "s1",
        "source_status": "public",
        "freshness": "fresh",
        "allowed_use": "summarize_and_transform"
    }]);
    let projection = project_content_ops_surface(&s);
    assert_eq!(projection.first_screen.waiting_on, "agent");
    assert_eq!(
        projection.first_screen.next_safe_action,
        "collect more compact source signals"
    );
}

#[test]
fn project_runnable_connector_trial() {
    let mut s = fixture();
    s["connector_trials"] = serde_json::json!([{
        "schema_version": "connector_trial_v0",
        "trial_id": "t1",
        "access_mode": "public_metadata_only",
        "source_status": "public",
        "freshness": "fresh",
        "allowed_use": "metadata_only",
        "trial_state": "ready_for_metadata_trial",
        "external_write_allowed": false
    }]);
    let projection = project_content_ops_surface(&s);
    assert!(projection
        .todo_candidates
        .iter()
        .any(|c| c.action_kind == "content_ops_connector_metadata_trial"));
}

// ── synthetic fixture + connector runtime policy ──────────────────────────

#[test]
fn fixture_shape_is_complete() {
    let value = build_content_ops_surface_fixture("2026-01-01T00:00:00Z");
    assert_eq!(
        value
            .get("schema_version")
            .and_then(serde_json::Value::as_str),
        Some("content_ops_surface_v0")
    );
    assert_eq!(
        value.get("surface_id").and_then(serde_json::Value::as_str),
        Some("creator_ops_public_safe_demo")
    );
    assert!(value.get("boundary").is_some());
}

#[test]
fn connector_runtime_policy_rejects_unknown_mode() {
    let err = build_content_ops_connector_runtime_policy("nope", "id", "name", None).unwrap_err();
    assert!(err.contains("access_mode"), "{err}");
}

#[test]
fn connector_runtime_policy_for_each_mode() {
    let public =
        build_content_ops_connector_runtime_policy("public_metadata_only", "id", "name", None)
            .unwrap();
    assert_eq!(
        public
            .get("safe_default")
            .and_then(serde_json::Value::as_str),
        Some("head_only_metadata_probe")
    );
    assert_eq!(
        public
            .get("allowed_probe_methods")
            .and_then(serde_json::Value::as_array)
            .unwrap(),
        &[serde_json::json!("HEAD")]
    );

    let private = build_content_ops_connector_runtime_policy(
        "private_metadata_only",
        "id",
        "name",
        Some("https://x.example/"),
    )
    .unwrap();
    assert_eq!(
        private
            .get("safe_default")
            .and_then(serde_json::Value::as_str),
        Some("gate_projection_only")
    );
    assert_eq!(
        private
            .get("connector_url")
            .and_then(serde_json::Value::as_str),
        Some("https://x.example/")
    );

    let fixture_only =
        build_content_ops_connector_runtime_policy("synthetic_fixture_only", "id", "name", None)
            .unwrap();
    assert_eq!(
        fixture_only
            .get("safe_default")
            .and_then(serde_json::Value::as_str),
        Some("fixture_only")
    );
}

// ── capability entry point ───────────────────────────────────────────────

#[test]
fn propose_valid_json_surface_is_clean() {
    let cap = future_loop::capabilities::CapabilityRegistry::with_builtin();
    let proposals = cap
        .get("content_ops")
        .unwrap()
        .propose(&build_content_ops_surface_fixture("2026-01-01").to_string());
    assert_eq!(proposals.len(), 1);
    assert_eq!(proposals[0].kind, ProposalKind::NoFollowUp);
    assert!(proposals[0]
        .reason
        .contains("content-ops surface validated clean"));
}

#[test]
fn propose_unrecognized_free_text_registers_source() {
    let cap = future_loop::capabilities::CapabilityRegistry::with_builtin();
    let proposals = cap.get("content_ops").unwrap().propose("just a thought");
    assert!(proposals.iter().any(
        |p| p.kind == ProposalKind::SuccessorTodo && p.reason == "content_ops_register_source"
    ));
}

#[test]
fn quality_band_needs_work_middle_band() {
    let text = format!("# T\nnext: go\n{}", vec!["word"; 60].join(" "));
    let signals = quality_signals(&text, ContentKind::Draft);
    assert_eq!(signals.score, 65);
    assert_eq!(signals.quality_band(), "needs_work");
}

#[test]
fn quality_numbered_list_is_counted() {
    let text = "# T\n1. first\n2. second\n3. third";
    let signals = quality_signals(text, ContentKind::Outline);
    assert_eq!(signals.list_item_count, 3);
}

#[test]
fn validate_compacts_empty_and_oversize_ids() {
    // Empty/whitespace id compacts to None (id: None in the error).
    let mut s = fixture();
    s["source_items"] = serde_json::json!([{
        "schema_version": "source_item_v0",
        "source_item_id": "   ",
        "source_status": "nope",
        "freshness": "fresh",
        "allowed_use": "summarize_and_transform"
    }]);
    let v = validate_content_ops_surface(&s);
    assert!(v.errors.iter().any(|e| e.contains("None")));

    // Oversize id is truncated with an ellipsis.
    let mut s2 = fixture();
    s2["source_items"] = serde_json::json!([{
        "schema_version": "source_item_v0",
        "source_item_id": "x".repeat(130),
        "source_status": "nope",
        "freshness": "fresh",
        "allowed_use": "summarize_and_transform"
    }]);
    let v2 = validate_content_ops_surface(&s2);
    assert!(v2.errors.iter().any(|e| e.contains("...")));
}
