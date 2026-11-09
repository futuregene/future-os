//! Each reviewer revisits its own first answer using the frozen moderator map.
use super::quotes::contains;
use anyhow::{ensure, Context, Result};
use future_fusion::{parse_json, ConsultationInput};
use serde::Deserialize;
use serde_json::{json, Value};

const INSTRUCTIONS: &str = r#"You are a read-only Fusion participant revisiting YOUR OWN first-stage answer. You receive only that original answer, the frozen moderator map, and the original task context. You do NOT receive other participants' full answers or anyone's third-stage review. Your participant identity in the packet is fixed; never describe another participant as yourself.
Start with the moderator's review_focus when provided. Resolve the checks that could change the outcome using supplied evidence, a counterexample, a corrected inference or an explicit decision criterion. Consider alternatives outside the listed positions when the original task supports them. The moderator map is not authoritative or verified. When responding to another participant, cite their exact original excerpt from the map and distinguish it from the moderator's paraphrase. If the excerpt is insufficient, explicitly say insufficient_information; do not invent the missing argument or infer opposition from silence. You may correct the moderator's organization using material actually visible to you.
For each consequential own position, quote your original answer and state whether you maintain, revise or withdraw it, the resulting position and your reason. For a change, identify the new evidence, inference or criterion that warrants it. Keep conditional positions conditional unless their premises have been established. Do not invent a previous position just to narrate a change. Distinguish new proposals, unverified claims and verified facts: only use verified_fact for something actually checked against identified supplied evidence or a tool result, with inspectable evidence references; agreement, memory and moderator summaries do not verify a fact. Otherwise use unverified_claim. You may investigate only through provided authorized read-only snapshot tools. Never write, execute commands or take external action.
Return ONLY JSON. current_answer must directly answer the original question with your current, self-contained position, including any material uncertainty; the remaining fields explain only consequential changes, not a full meeting transcript:
{"participant":"your ID","current_answer":"your current answer to the original question","responses":[{"participant":"other ID","quote":"exact excerpt from their source quote in moderator_map","decision":"agree|disagree|qualify|insufficient_information","reason":"reason"}],"self_review":[{"original_quote":"exact excerpt from your original_analysis","decision":"maintain|revise|withdraw","updated_position":"your current position","reason":"why"}],"mapping_corrections":[{"summary_quote":"exact excerpt from moderator_map","source_quote":"exact visible original excerpt supporting your correction","correction":"what the map got wrong","reason":"why"}],"additions":[{"kind":"proposal|unverified_claim|verified_fact","content":"new material","evidence":["inspectable source reference for verified_fact"]}]}
All four arrays must exist and may be empty except self_review, which must address your own answer. Do not repeat the entire original plan. A verified_fact label is not a claim that the whole proposal was tested."#;

pub(super) fn prompt(
    participant: &str,
    input: &ConsultationInput,
    analyses: &[Value],
    mapping: &Value,
) -> Result<String> {
    let own = analyses
        .iter()
        .find(|a| a["participant"].as_str() == Some(participant))
        .context("Missing reviewer's first answer")?;
    let packet = json!({"participant":participant,"task":input,"original_analysis":own["response"].as_str().context("Missing original public answer")?,"moderator_map":mapping});
    Ok(format!(
        "{INSTRUCTIONS}\nReview packet:\n{}",
        serde_json::to_string(&packet)?
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Review {
    participant: String,
    current_answer: String,
    responses: Vec<Response>,
    self_review: Vec<Revision>,
    mapping_corrections: Vec<Correction>,
    additions: Vec<Addition>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    participant: String,
    quote: String,
    decision: ResponseDecision,
    reason: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ResponseDecision {
    Agree,
    Disagree,
    Qualify,
    InsufficientInformation,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Revision {
    original_quote: String,
    decision: RevisionDecision,
    updated_position: String,
    reason: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum RevisionDecision {
    Maintain,
    Revise,
    Withdraw,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Correction {
    summary_quote: String,
    source_quote: String,
    correction: String,
    reason: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Addition {
    kind: AdditionKind,
    content: String,
    evidence: Vec<String>,
}
#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum AdditionKind {
    Proposal,
    UnverifiedClaim,
    VerifiedFact,
}

fn strings(value: &Value) -> Vec<&str> {
    match value {
        Value::String(s) => vec![s],
        Value::Array(items) => items.iter().flat_map(strings).collect(),
        Value::Object(fields) => fields.values().flat_map(strings).collect(),
        _ => vec![],
    }
}
fn quoted_sources(mapping: &Value) -> Vec<(&str, &str)> {
    let shared = mapping["consensus"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|v| v["sources"].as_array().into_iter().flatten());
    let stances = mapping["issues"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|v| v["stances"].as_array().into_iter().flatten());
    let excluded = mapping["excluded"].as_array().into_iter().flatten();
    shared
        .chain(stances)
        .chain(excluded)
        .filter_map(|source| Some((source["participant"].as_str()?, source["quote"].as_str()?)))
        .collect()
}

pub(super) fn validate(text: &str, prompt: &str, participant: &str) -> Result<()> {
    let review: Review = parse_json(text)?;
    let (_, packet) = prompt
        .rsplit_once("\nReview packet:\n")
        .context("Missing review packet")?;
    let packet: Value = serde_json::from_str(packet)?;
    ensure!(
        review.participant == participant && packet["participant"].as_str() == Some(participant),
        "Reviewer identity mismatch"
    );
    ensure!(
        !review.current_answer.trim().is_empty(),
        "Missing current answer to the topic"
    );
    let own = packet["original_analysis"]
        .as_str()
        .context("Missing own analysis")?;
    let mapping = &packet["moderator_map"];
    let sources = quoted_sources(mapping);
    let mut errors = vec![];
    for (index, response) in review.responses.iter().enumerate() {
        if response.participant == participant
            || !sources
                .iter()
                .any(|(id, quote)| *id == response.participant && contains(quote, &response.quote))
        {
            let visible = sources
                .iter()
                .filter(|(id, _)| *id == response.participant)
                .map(|(_, quote)| *quote)
                .collect::<Vec<_>>();
            errors.push(format!("responses[{index}].quote must cite the named OTHER participant's visible original excerpt. Supplied quote: {:?}. Visible source excerpts for {}: {}",
                response.quote, response.participant,
                future_fusion::bounded_text(&serde_json::to_string(&visible)?, 1600)));
        }
        if response.reason.trim().is_empty() {
            errors.push(format!("responses[{index}].reason is empty"));
        }
        let _ = &response.decision;
    }
    if review.self_review.is_empty() {
        errors.push("self_review must address your own answer".into());
    }
    for (index, revision) in review.self_review.iter().enumerate() {
        if !contains(own, &revision.original_quote) {
            errors.push(format!("self_review[{index}].original_quote does not match your original_analysis: {:?}. Copy a contiguous excerpt from original_analysis; do not invent an earlier position.", revision.original_quote));
        }
        if revision.updated_position.trim().is_empty() || revision.reason.trim().is_empty() {
            errors.push(format!(
                "self_review[{index}] needs updated_position and reason"
            ));
        }
        let _ = &revision.decision;
    }
    let summaries = strings(mapping);
    for (index, correction) in review.mapping_corrections.iter().enumerate() {
        if !summaries
            .iter()
            .any(|s| contains(s, &correction.summary_quote))
        {
            errors.push(format!("mapping_corrections[{index}].summary_quote must identify a visible moderator statement"));
        }
        if !contains(own, &correction.source_quote)
            && !sources
                .iter()
                .any(|(_, q)| contains(q, &correction.source_quote))
        {
            errors.push(format!(
                "mapping_corrections[{index}].source_quote lacks a visible original source"
            ));
        }
        if correction.correction.trim().is_empty() || correction.reason.trim().is_empty() {
            errors.push(format!(
                "mapping_corrections[{index}] needs correction and reason"
            ));
        }
    }
    for (index, addition) in review.additions.iter().enumerate() {
        if addition.content.trim().is_empty() {
            errors.push(format!("additions[{index}].content is empty"));
        }
        if addition.kind == AdditionKind::VerifiedFact
            && (addition.evidence.is_empty()
                || addition.evidence.iter().any(|e| e.trim().is_empty()))
        {
            errors.push(format!(
                "additions[{index}] labels a verified_fact without inspectable evidence references"
            ));
        }
    }
    ensure!(errors.is_empty(), "{}", errors.join("\n"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Value, String) {
        let input: ConsultationInput = serde_json::from_value(json!({"question":"Choose storage", "goal":"Keep data", "background":"User requests persistence", "constraints":[]})).unwrap();
        let analyses = vec![
            json!({"participant":"P1","response":"I prefer SQLite. Keep data."}),
            json!({"participant":"P2","response":"Use files. PRIVATE_OTHER_DETAILS"}),
        ];
        let mapping = json!({"consensus":[], "issues":[{"id":"I1","topic":"Storage","stances":[
            {"participant":"P1","status":"stated","position":"SQLite","quote":"I prefer SQLite."},
            {"participant":"P2","status":"stated","position":"Files","quote":"Use files."}
        ]}], "excluded":[]});
        let prompt = prompt("P1", &input, &analyses, &mapping).unwrap();
        let review = json!({"participant":"P1", "current_answer":"Prefer SQLite pending workload evidence",
            "responses":[{"participant":"P2","quote":"Use files.","decision":"insufficient_information","reason":"No rationale in the visible excerpt"}],
            "self_review":[{"original_quote":"I prefer SQLite.","decision":"maintain","updated_position":"Prefer SQLite pending investigation","reason":"No new contrary evidence"}],
            "mapping_corrections":[], "additions":[]});
        (review, prompt)
    }
    #[test]
    fn review_packet_excludes_other_full_answers_and_accepts_sourced_self_review() {
        let (review, prompt) = fixture();
        assert!(!prompt.contains("PRIVATE_OTHER_DETAILS"));
        assert!(prompt.contains("I prefer SQLite."));
        assert!(prompt.contains("Use files."));
        validate(&review.to_string(), &prompt, "P1").unwrap();
    }
    #[test]
    fn quote_layout_changes_pass_and_repair_feedback_names_all_bad_fields() {
        let (mut review, prompt) = fixture();
        review["responses"][0]["quote"] = json!("Use\n files.");
        validate(&review.to_string(), &prompt, "P1").unwrap();
        review["responses"][0]["quote"] = json!("Invented other view");
        review["self_review"][0]["original_quote"] = json!("Invented own view");
        let error = validate(&review.to_string(), &prompt, "P1")
            .unwrap_err()
            .to_string();
        assert!(error.contains("responses[0].quote"));
        assert!(error.contains("self_review[0].original_quote"));
        assert!(error.contains("Use files."));
    }

    #[test]
    fn rejects_identity_drift_invented_positions_and_unbacked_facts() {
        let (review, prompt) = fixture();
        let mut missing_answer = review.clone();
        missing_answer["current_answer"] = json!("");
        assert!(validate(&missing_answer.to_string(), &prompt, "P1").is_err());
        let mut changed = review.clone();
        changed["participant"] = json!("P2");
        assert!(validate(&changed.to_string(), &prompt, "P1").is_err());
        let mut changed = review.clone();
        changed["self_review"][0]["original_quote"] = json!("I used to prefer files.");
        assert!(validate(&changed.to_string(), &prompt, "P1").is_err());
        let mut changed = review.clone();
        changed["responses"][0]["quote"] = json!("PRIVATE_OTHER_DETAILS");
        assert!(validate(&changed.to_string(), &prompt, "P1").is_err());
        let mut changed = review.clone();
        changed["additions"] = json!([{"kind":"verified_fact","content":"Claim","evidence":[]}]);
        assert!(validate(&changed.to_string(), &prompt, "P1").is_err());
        let mut changed = review;
        changed["mapping_corrections"] = json!([{"summary_quote":"SQLite","source_quote":"I prefer SQLite.","correction":"This is a preference, not proof","reason":"The original is qualified"}]);
        validate(&changed.to_string(), &prompt, "P1").unwrap();
    }
}
