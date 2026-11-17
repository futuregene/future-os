//! Closed-task answer contract. Validation checks provenance, never a gold answer.
use anyhow::{ensure, Result};
use serde::Deserialize;
use serde_json::Value;

pub(super) const INSTRUCTIONS: &str = r#"Closed answer protocol: return ONLY a JSON object with answer, support, unresolved, and checks:
{"answer":"the complete user-facing answer in exactly the user's requested format","support":"concise public justification, separate from the answer","unresolved":[],"checks":[]}
This is a full-parent answer-only task. The backend will deliver answer verbatim; no later model will re-solve or reformat it. Include any requested FINAL_ANSWER line in answer itself. Do not append option descriptions, alternative spellings, meeting notes or qualifications that the user did not request; put necessary justification in support. If the user requests an explanation, answer includes that explanation. Never select a guessed value just to satisfy a numeric format. Keep actual uncertainty visible when the task cannot be determined.
In support, distinguish what is established from what is only compatible. For every selected claim, ask whether another model of the supplied facts could make it false. If so, it is not established unless the task explicitly asks for possible outcomes. Do not infer unknown attributes from entity labels or their spelling. Treat conditional antecedents as proof obligations; priority alone does not satisfy them. Give only the decisive derivation, not an exhaustive investigation or a recap of every rule.
checks contains only consequential source checks. Each check has claim, quote, assessment (supports|does_not_establish|contradicts), and effect. quote must be a short exact excerpt from parent_request (or question if no parent_request), not from a prior assistant assertion. Explain what the excerpt actually establishes and what it leaves unknown. Assumptions and conditional implications cannot silently become observations. Resolve source mistakes before publishing answer; repeating an advisor's assertion is not a check. Participants may leave checks empty; the final moderator must include at least one decisive source check. unresolved lists only gaps that actually prevent the supported answer or change its applicability. Return an empty array when none remain."#;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Answer {
    pub answer: String,
    pub support: String,
    pub unresolved: Vec<String>,
    pub checks: Vec<Check>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Check {
    claim: String,
    quote: String,
    assessment: Assessment,
    effect: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Assessment {
    Supports,
    DoesNotEstablish,
    Contradicts,
}

pub(super) fn parse(text: &str) -> Result<Answer> {
    future_fusion::parse_json(text)
}

pub(super) fn validate(text: &str, task: &str, final_report: bool) -> Result<()> {
    let answer = parse(text)?;
    ensure!(
        !answer.answer.trim().is_empty(),
        "Missing user-facing answer"
    );
    ensure!(
        !answer.support.trim().is_empty(),
        "Missing public justification"
    );
    ensure!(
        !final_report || !answer.checks.is_empty(),
        "Final answer requires a decisive source check against the original request"
    );
    ensure!(
        answer.unresolved.iter().all(|s| !s.trim().is_empty()),
        "Empty unresolved item"
    );
    for check in answer.checks {
        ensure!(
            !check.claim.trim().is_empty() && !check.effect.trim().is_empty(),
            "Source check must identify the claim and its effect on the answer"
        );
        ensure!(super::quotes::contains(task, &check.quote), "Source check quote is not present in the preserved request; do not use an assistant claim as a supplied fact: {}", check.quote);
        match check.assessment {
            Assessment::Supports | Assessment::DoesNotEstablish | Assessment::Contradicts => {}
        }
    }
    Ok(())
}

pub(super) fn public_text(report: Answer) -> String {
    let checks = report
        .checks
        .into_iter()
        .map(|c| {
            format!(
                "Claim: {}\nSource quote: {}\nAssessment: {:?}\nEffect: {}",
                c.claim, c.quote, c.assessment, c.effect
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Answer:\n{}\nPublic support:\n{}\nUnresolved:\n{}\nSource checks:\n{}",
        report.answer,
        report.support,
        report.unresolved.join("\n"),
        checks
    )
}

/// Agreement is a scheduling shortcut; synthesis must still audit source support.
pub(super) fn clear_agreement(analyses: &[Value], expected: usize) -> bool {
    if analyses.len() != expected || analyses.len() < 2 {
        return false;
    }
    let reports: Option<Vec<_>> = analyses
        .iter()
        .map(|item| {
            if item["forwarded_truncated"] == true
                || item["validation_errors"]
                    .as_array()
                    .is_some_and(|v| !v.is_empty())
            {
                return None;
            }
            parse(item["response"].as_str()?).ok()
        })
        .collect();
    let Some(reports) = reports else {
        return false;
    };
    reports
        .iter()
        .all(|r| r.unresolved.is_empty() && r.answer.trim() == reports[0].answer.trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn source_checks_cannot_quote_an_advisor_as_the_original_task() {
        let mut value = json!({"answer":"result", "support":"a derivation", "unresolved":[], "checks":[{"claim":"value supplied", "quote":"invented value", "assessment":"supports", "effect":"choose result"}]});
        assert!(validate(&value.to_string(), "If a value is supplied, use it.", true).is_err());
        value["checks"][0]["quote"] = json!("If a value is supplied, use it.");
        value["checks"][0]["assessment"] = json!("does_not_establish");
        assert!(validate(&value.to_string(), "If a value is supplied, use it.", true).is_ok());
        value["checks"] = json!([]);
        assert!(validate(&value.to_string(), "task", true).is_err());
        assert!(validate(&value.to_string(), "task", false).is_ok());
    }
    #[test]
    fn unresolved_reduced_or_invalid_reports_cannot_shortcut_review() {
        let report = json!({"answer":"answer", "support":"independent derivation", "unresolved":[], "checks":[]}).to_string();
        let mut reports = vec![
            json!({"response":report,"validation_errors":[]}),
            json!({"response":report,"validation_errors":[]}),
        ];
        assert!(clear_agreement(&reports, 2));
        assert!(!clear_agreement(&reports, 3));
        reports[1]["validation_errors"] = json!(["source mismatch"]);
        assert!(!clear_agreement(&reports, 2));
        reports[1]["validation_errors"] = json!([]);
        reports[1]["response"] = json!(json!({"answer":"answer","support":"derivation","unresolved":["missing condition"],"checks":[]}).to_string());
        assert!(!clear_agreement(&reports, 2));
    }
}
