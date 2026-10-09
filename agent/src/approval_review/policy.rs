use super::{
    prompt::{AUTHORIZATIONS, REASONS, RISKS},
    types::{ProviderAssessment, ReviewStatus, Verdict},
    PROBABILITY_SUM_TOLERANCE, THRESHOLD,
};
use serde_json::{json, Value};

/// Confidence in the allowed authorization set, rather than in an individual
/// label. A split between Medium and High is harmless when both are permitted.
fn authorization_support(probabilities: &Value, risk: usize) -> Option<f64> {
    let probabilities = probabilities.get("authorization")?.as_object()?;
    if probabilities.len() != AUTHORIZATIONS.len() {
        return None;
    }
    let values: Vec<f64> = AUTHORIZATIONS
        .iter()
        .map(|(key, _)| probabilities.get(*key)?.as_f64())
        .collect::<Option<_>>()?;
    if values
        .iter()
        .any(|p| !p.is_finite() || !(0.0..=1.0).contains(p))
        || (values.iter().sum::<f64>() - 1.0).abs() > PROBABILITY_SUM_TOLERANCE
    {
        return None;
    }
    let start = match risk {
        0 => 1, // Low: low + medium + high, excluding unknown.
        1 => 2, // Medium: medium + high.
        2 => 3, // High: high only.
        _ => return None,
    };
    Some(values[start..].iter().sum::<f64>().min(1.0))
}

pub fn decide(assessment: ProviderAssessment) -> Verdict {
    let r = &assessment.reported;
    let mut status = ReviewStatus::Rejected;
    let rank = |s: &str, options: &[(&str, &str)]| options.iter().position(|(name, _)| *name == s);
    let Some(risk) = rank(&r.risk, RISKS) else {
        return Verdict::error(ReviewStatus::ReviewError, "invalid_choice");
    };
    let Some(auth) = rank(&r.authorization, AUTHORIZATIONS) else {
        return Verdict::error(ReviewStatus::ReviewError, "invalid_choice");
    };
    if rank(&r.reason_code, REASONS).is_none() {
        return Verdict::error(ReviewStatus::ReviewError, "invalid_choice");
    }
    let minimum = match r.reason_code.as_str() {
        "routine_bounded_action" | "insufficient_information" => 0,
        "bounded_external_side_effect" => 1,
        "catastrophic_system_destruction"
        | "irreversible_critical_data_loss"
        | "loss_of_security_control" => 3,
        _ => 2,
    };
    let risk = risk.max(minimum);
    let auth = if r.reason_code == "authorization_scope_mismatch" {
        auth.min(1)
    } else {
        auth
    };
    let mut confidence = assessment.confidence;
    // Derived policy confidence is host-owned, never supplied by a provider.
    confidence.remove("authorization_support");
    if let Some(support) = authorization_support(&assessment.probabilities, risk) {
        confidence.insert("authorization_support".into(), support);
    }
    let effective = if r.reason_code == "insufficient_information" {
        status = ReviewStatus::ReviewUncertain;
        json!({"risk": null, "authorization": null})
    } else {
        let allows = match risk {
            0 => true,
            1 => auth >= 2,
            2 => auth == 3,
            _ => false,
        };
        if allows {
            // The raw provider confidence measures the exact authorization
            // label, not the sum of allowed outcomes. Preserve it for audit,
            // but use the accepted probability mass in the execution policy.
            status = if ["risk", "authorization_support", "reason_code"]
                .iter()
                .all(|name| {
                    confidence
                        .get(*name)
                        .is_some_and(|v| v.is_finite() && *v >= THRESHOLD && *v <= 1.0)
                }) {
                ReviewStatus::Approved
            } else {
                ReviewStatus::ReviewUncertain
            };
        }
        json!({"risk": RISKS[risk].0, "authorization": AUTHORIZATIONS[auth].0})
    };
    Verdict {
        status,
        reported: Some(assessment.reported),
        effective,
        confidence,
        probabilities: assessment.probabilities,
        model: Some(assessment.model),
        provider_request_id: assessment.request_id,
        error_code: None,
    }
}
