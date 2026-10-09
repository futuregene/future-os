use super::*;
#[test]
fn matrix_and_reason_invariants_are_deterministic() {
    for (ri, (risk, _)) in RISKS.iter().enumerate() {
        for (ai, (auth, _)) in AUTHORIZATIONS.iter().enumerate() {
            let expected = match ri {
                0 => ai >= 1, // Unknown-only authorization has no allowed support.
                1 => ai >= 2,
                2 => ai == 3,
                _ => false,
            };
            assert_eq!(
                decide(assessment(risk, auth, "routine_bounded_action")).approved(),
                expected,
                "{risk}/{auth}"
            );
        }
    }
    for (reason, _) in REASONS {
        let verdict = decide(assessment("low", "high", reason));
        match *reason {
            "insufficient_information" => {
                assert_eq!(verdict.status, "review_uncertain");
                assert!(verdict.effective["risk"].is_null());
            }
            "authorization_scope_mismatch" => {
                assert_eq!(verdict.effective["authorization"], "low");
                assert!(!verdict.approved());
            }
            "catastrophic_system_destruction"
            | "irreversible_critical_data_loss"
            | "loss_of_security_control" => assert!(!verdict.approved()),
            "routine_bounded_action" | "bounded_external_side_effect" => {
                assert!(verdict.approved())
            }
            _ => {
                assert_eq!(verdict.effective["risk"], "high");
                assert!(verdict.approved());
            }
        }
    }
}
#[test]
fn confidence_can_only_tighten_and_is_finite() {
    for c in [0.749, 0.75, 0.751, f64::NAN, 1.1] {
        let mut a = assessment("high", "high", "protected_secret_access");
        a.confidence.insert("risk".into(), c);
        assert_eq!(
            decide(a).approved(),
            c.is_finite() && (0.75..=1.0).contains(&c)
        );
    }
    let mut a = assessment("critical", "high", "routine_bounded_action");
    a.confidence.clear();
    assert_eq!(decide(a).status, "rejected");
}

#[test]
fn routine_low_risk_file_creation_does_not_require_authorization_confidence() {
    // Regression for the desktop test-file request: a certain ordinary local
    // write was previously vetoed by the authorization classifier's 32%.
    let mut a = assessment("low", "high", "routine_bounded_action");
    a.confidence = [
        ("risk".into(), 1.0),
        ("authorization".into(), 0.32),
        ("reason_code".into(), 0.97),
    ]
    .into();
    a.probabilities["authorization"] = json!({"high":0.49,"medium":0.19,"low":0.31,"unknown":0.01});
    let verdict = decide(a.clone());
    assert!(verdict.approved());
    assert_eq!(verdict.confidence["authorization"], 0.32);
    assert!((verdict.confidence["authorization_support"] - 0.99).abs() < 1e-9);
    for auth in ["low", "medium", "high"] {
        assert!(decide(assessment("low", auth, "routine_bounded_action")).approved());
    }
    // The policy uses allowed probability mass, not exact-label confidence.
    a.confidence.remove("authorization");
    assert!(decide(a.clone()).approved());
    for key in ["risk", "reason_code"] {
        for confidence in [0.749, f64::NAN, 1.1] {
            let mut uncertain = a.clone();
            uncertain.confidence.insert(key.into(), confidence);
            assert_eq!(decide(uncertain).status, "review_uncertain");
        }
        let mut uncertain = a.clone();
        uncertain.confidence.remove(key);
        assert_eq!(decide(uncertain).status, "review_uncertain");
    }
}

#[test]
fn authorization_probability_is_combined_for_each_risk_permission_set() {
    for (risk, support, expected) in [
        ("low", 0.99, "approved"),
        ("medium", 0.68, "review_uncertain"),
        ("high", 0.49, "review_uncertain"),
    ] {
        let mut a = assessment(risk, "high", "routine_bounded_action");
        a.confidence.insert("authorization".into(), 0.32);
        a.probabilities["authorization"] =
            json!({"high":0.49,"medium":0.19,"low":0.31,"unknown":0.01});
        let verdict = decide(a);
        assert_eq!(verdict.status, expected);
        assert!((verdict.confidence["authorization_support"] - support).abs() < 1e-9);
    }
    let mut a = assessment("medium", "high", "bounded_external_side_effect");
    a.confidence.insert("authorization".into(), 0.1);
    a.probabilities["authorization"] = json!({"high":0.4,"medium":0.4,"low":0.19,"unknown":0.01});
    assert!(decide(a.clone()).approved());
    a.reported.risk = "high".into();
    assert_eq!(decide(a).status, "review_uncertain");

    for high in [0.749, 0.75, 0.751] {
        let mut a = assessment("high", "high", "protected_secret_access");
        a.probabilities["authorization"] =
            json!({"high":high,"medium":1.0-high,"low":0.0,"unknown":0.0});
        assert_eq!(decide(a).approved(), high >= THRESHOLD);
    }
    let mut a = assessment("low", "high", "routine_bounded_action");
    a.probabilities["authorization"] = json!({"high":0.4,"medium":0.2,"low":0.14,"unknown":0.26});
    assert_eq!(decide(a.clone()).status, "review_uncertain");
    a.probabilities["authorization"] = json!({"high":0.4,"medium":0.2,"low":0.15,"unknown":0.25});
    assert!(decide(a.clone()).approved());
    for probabilities in [
        Value::Null,
        json!({"high":1.0}),
        json!({"high":1.0,"medium":0.0,"low":0.0,"unknown":1.0}),
        json!({"high":1.1,"medium":-0.1,"low":0.0,"unknown":0.0}),
    ] {
        a.probabilities["authorization"] = probabilities;
        a.confidence.insert("authorization_support".into(), 1.0);
        assert!(!decide(a.clone()).approved());
    }
}

#[test]
fn authorization_aggregation_respects_reason_floors_and_absolute_denials() {
    for (reason, _) in REASONS.iter().filter(|(reason, _)| {
        !matches!(
            *reason,
            "routine_bounded_action" | "bounded_external_side_effect"
        )
    }) {
        let mut a = assessment("low", "high", reason);
        a.probabilities["authorization"] =
            json!({"high":0.4,"medium":0.4,"low":0.19,"unknown":0.01});
        assert!(
            !decide(a).approved(),
            "reason floor/denial must survive: {reason}"
        );
    }
    let verdict = decide(assessment("low", "high", "authorization_scope_mismatch"));
    assert_eq!(verdict.status, "rejected");
    assert_eq!(verdict.effective["authorization"], "low");
}
