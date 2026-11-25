//! Each reviewer revisits its own first answer using the frozen moderator map.
use super::quotes::{contains, contains_report, expand, select_text};
use anyhow::{ensure, Context, Result};
use future_fusion::{parse_json, ConsultationInput, Snapshot};
use serde::Deserialize;
use serde_json::{json, Value};

const INSTRUCTIONS: &str = r#"You are a read-only Fusion participant revisiting YOUR OWN first-stage answer. You receive only that original answer, the frozen moderator map, and the original task context. You do NOT receive other participants' full answers or anyone's third-stage review. Your participant identity in the packet is fixed; never describe another participant as yourself.
Start with the moderator's review_focus when provided. Check any restriction or assumption in that focus against the original task before adopting it. Resolve the checks that could change the outcome using supplied evidence, a counterexample, a corrected inference or an explicit decision criterion. Consider alternatives outside the listed positions when the original task supports them. The moderator map is not authoritative or verified. When responding to another participant, cite their exact original excerpt from the map and distinguish it from the moderator's paraphrase. If the excerpt is insufficient, explicitly say insufficient_information; do not invent the missing argument or infer opposition from silence. You may correct the moderator's organization using material actually visible to you.
For each consequential own position, quote your original answer and state whether you maintain, revise or withdraw it, the resulting position and your reason. For a change, identify the new evidence, inference or criterion that warrants it. Keep conditional positions conditional unless their premises have been established. Do not invent a previous position just to narrate a change. Distinguish new proposals, unverified claims and verified facts: only use verified_fact for something actually checked against identified supplied evidence or a tool result, with inspectable evidence references; agreement, memory and moderator summaries do not verify a fact. Otherwise use unverified_claim. You may investigate only through provided authorized read-only snapshot tools. Never write, execute commands or take external action.
Keep the original evidential standard: a claim that is not contradicted is not thereby established. Use a countermodel or missing premise to challenge an unsupported selection. Entity labels do not supply unknown attributes. Return ONLY JSON. current_answer must directly answer the assigned discussion task with your current, self-contained position, including any material uncertainty; the remaining fields explain only consequential changes, not a full meeting transcript:
{"participant":"your ID","current_answer":"your current answer to the assigned discussion task","responses":[{"participant":"other ID","quote":"exact excerpt from their source quote in moderator_map","decision":"agree|disagree|qualify|insufficient_information","reason":"reason"}],"self_review":[{"original_quote":"exact excerpt from your original_analysis","decision":"maintain|revise|withdraw","updated_position":"your current position","reason":"why"}],"mapping_corrections":[{"summary_quote":"exact excerpt from moderator_map","source_quote":"exact visible original excerpt supporting your correction","correction":"what the map got wrong","reason":"why"}],"additions":[{"kind":"proposal|unverified_claim|verified_fact","content":"new material","evidence":[{"source_id":"request or evidence:N or snapshot:<path>","sha256":"version from the source manifest or read tool","quote":"exact source excerpt"}]}]}
Optionally add objections:[{"participant":"source participant ID","quote":"exact excerpt from your original_analysis, current_answer, or the named participant source quote in moderator_map","reason":"decisive counterexample or missing premise","effect":"how it could change the answer"}]. Include only decision-critical objections that still need explicit disposition, not stylistic differences. A new counterexample may quote your own current_answer. These remain attributed arguments, not verified facts.
Also supply recommendation as a short standalone statement of your current conclusion, including conditions that affect its application. For a closed task, use the exact complete answer in the requested format. Put supporting reasoning in current_answer. Omit recommendation or use null when you cannot support one definite choice. The recommendation reflects this review, including any change from the first answer.
For a mapping correction, summary_path selects text within moderator_map (with or without a /moderator_map prefix). For source_path, /original_analysis selects your own visible report, /original_analysis/answer selects a field when that report is JSON, and /moderator_map/.../quote selects a visible participant excerpt. Use source_quote to copy a short exact excerpt from a plain-text report. Inline request and evidence references can omit sha256; the host fills it from the frozen source. A proposal or unverified_claim may omit evidence. For snapshot references also include start_line and end_line (one-based, at most 400 lines) matching the read tool result. Inline sources do not use line fields. Never cite a moderator/participant statement, background summary or a fabricated label as source-verified evidence. An unavailable source must remain an unverified_claim. All four arrays must exist and may be empty except self_review, which must address your own answer. Do not repeat the entire original plan. A verified_fact label is not a claim that the whole proposal was tested."#;

pub(super) fn prompt(
    participant: &str,
    input: &ConsultationInput,
    analyses: &[Value],
    mapping: &Value,
    snapshot: Option<&Snapshot>,
    lineage: Option<&future_fusion::DecisionLineage>,
) -> Result<String> {
    let own = analyses
        .iter()
        .find(|a| a["participant"].as_str() == Some(participant))
        .context("Missing reviewer's first answer")?;
    let packet = json!({"participant":participant,"task":super::evidence::prompt_input(input),"decision_lineage":lineage,"original_analysis":own["response"].as_str().context("Missing original public answer")?,"moderator_map":mapping,"source_manifest":super::evidence::Sources::new(input, snapshot).manifest()});
    Ok(format!(
        "{INSTRUCTIONS}\n{}\n{}\n{}\n{}\nReview packet:\n{}",
        future_fusion::GROUNDING_POLICY,
        super::evidence::POLICY,
        super::evidence::COVERAGE_CHECK,
        super::evidence::REPRODUCIBLE_CHECK,
        serde_json::to_string(&packet)?
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Review {
    participant: String,
    current_answer: String,
    #[serde(default)]
    recommendation: Option<String>,
    responses: Vec<Response>,
    self_review: Vec<Revision>,
    mapping_corrections: Vec<Correction>,
    additions: Vec<Addition>,
    #[serde(default)]
    objections: Vec<Objection>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Objection {
    participant: String,
    quote: String,
    reason: String,
    effect: String,
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
    #[serde(default)]
    evidence: Vec<super::evidence::Reference>,
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

pub(super) fn expand_citations(
    text: &str,
    prompt: &str,
    task: &ConsultationInput,
) -> Result<String> {
    let mut review: Value = parse_json(text)?;
    ensure!(review.is_object(), "Review must be a JSON object");
    let (_, packet) = prompt
        .rsplit_once("\nReview packet:\n")
        .context("Missing review packet")?;
    let packet: Value = serde_json::from_str(packet)?;
    let mapping = &packet["moderator_map"];
    let own = packet["original_analysis"].as_str();
    let own_fields = own.and_then(|text| parse_json::<Value>(text).ok());
    let evidence = super::evidence::Sources::new(task, None);
    if let Some(corrections) = review
        .get_mut("mapping_corrections")
        .and_then(Value::as_array_mut)
    {
        for row in corrections.iter_mut().filter_map(Value::as_object_mut) {
            expand(row, "summary_path", "summary_quote", |path| {
                select_text(mapping, path)
                    .or_else(|| select_text(mapping, path.strip_prefix("/moderator_map")?))
            })?;
            expand(row, "source_path", "source_quote", |path| {
                if path == "/original_analysis" {
                    return own.filter(|text| !text.trim().is_empty());
                }
                if let Some(path) = path.strip_prefix("/original_analysis") {
                    return own_fields
                        .as_ref()
                        .and_then(|value| select_text(value, path));
                }
                let path = path.strip_prefix("/moderator_map")?;
                let (parent, field) = path.rsplit_once('/')?;
                if field != "quote"
                    || mapping
                        .pointer(parent)?
                        .get("participant")?
                        .as_str()
                        .is_none()
                {
                    return None;
                }
                select_text(mapping, path)
            })?;
        }
    }
    if let Some(additions) = review.get_mut("additions").and_then(Value::as_array_mut) {
        for addition in additions {
            let Some(citations) = addition.get_mut("evidence").and_then(Value::as_array_mut) else {
                continue;
            };
            for citation in citations.iter_mut().filter_map(Value::as_object_mut) {
                if !citation.contains_key("sha256") {
                    if let Some(version) = citation
                        .get("source_id")
                        .and_then(Value::as_str)
                        .and_then(|id| evidence.inline_version(id))
                    {
                        citation.insert("sha256".into(), json!(version));
                    }
                }
            }
        }
    }
    Ok(review.to_string())
}

pub(super) fn validate(
    text: &str,
    prompt: &str,
    participant: &str,
    task: &ConsultationInput,
    snapshot: Option<&Snapshot>,
) -> Result<()> {
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
    ensure!(
        review
            .recommendation
            .as_ref()
            .is_none_or(|text| !text.trim().is_empty()),
        "Use null for an absent recommendation"
    );
    let own = packet["original_analysis"]
        .as_str()
        .context("Missing own analysis")?;
    let mapping = &packet["moderator_map"];
    let sources = quoted_sources(mapping);
    let evidence = super::evidence::Sources::new(task, snapshot);
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
    for (index, objection) in review.objections.iter().enumerate() {
        let visible = if objection.participant == participant {
            contains_report(own, &objection.quote)
                || contains(&review.current_answer, &objection.quote)
        } else {
            sources.iter().any(|(id, quote)| {
                *id == objection.participant && contains(quote, &objection.quote)
            })
        };
        if !visible || objection.reason.trim().is_empty() || objection.effect.trim().is_empty() {
            errors.push(format!(
                "objections[{index}] requires a visible attributed quote, reason and effect"
            ));
        }
    }
    if review.self_review.is_empty() {
        errors.push("self_review must address your own answer".into());
    }
    for (index, revision) in review.self_review.iter().enumerate() {
        if !contains_report(own, &revision.original_quote) {
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
        if !contains_report(own, &correction.source_quote)
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
        if addition.kind == AdditionKind::VerifiedFact && addition.evidence.is_empty() {
            errors.push(format!(
                "additions[{index}] labels a verified_fact without source references"
            ));
        }
        for (reference_index, reference) in addition.evidence.iter().enumerate() {
            // A structured reference claims a concrete location even when the
            // surrounding proposal or interpretation remains unverified.
            if addition.kind == AdditionKind::VerifiedFact
                || matches!(reference, super::evidence::Reference::Citation(_))
            {
                if let Err(error) = evidence.validate(reference) {
                    errors.push(format!(
                        "additions[{index}].evidence[{reference_index}]: {error}"
                    ));
                }
            }
        }
    }
    ensure!(errors.is_empty(), "{}", errors.join("\n"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Value, String, ConsultationInput) {
        let input: ConsultationInput = serde_json::from_value(json!({"question":"Choose storage", "goal":"Keep data", "background":"User requests persistence", "constraints":[]})).unwrap();
        let analyses = vec![
            json!({"participant":"P1","response":"I prefer SQLite. Keep data."}),
            json!({"participant":"P2","response":"Use files. PRIVATE_OTHER_DETAILS"}),
        ];
        let mapping = json!({"consensus":[], "issues":[{"id":"I1","topic":"Storage","stances":[
            {"participant":"P1","status":"stated","position":"SQLite","quote":"I prefer SQLite."},
            {"participant":"P2","status":"stated","position":"Files","quote":"Use files."}
        ]}], "excluded":[]});
        let prompt = prompt("P1", &input, &analyses, &mapping, None, None).unwrap();
        let review = json!({"participant":"P1", "current_answer":"Prefer SQLite pending workload evidence",
            "responses":[{"participant":"P2","quote":"Use files.","decision":"insufficient_information","reason":"No rationale in the visible excerpt"}],
            "self_review":[{"original_quote":"I prefer SQLite.","decision":"maintain","updated_position":"Prefer SQLite pending investigation","reason":"No new contrary evidence"}],
            "mapping_corrections":[], "additions":[]});
        (review, prompt, input)
    }
    #[test]
    fn frozen_review_supplies_correction_text_and_inline_version() {
        let input = serde_json::json!({
            "question":"Observed count: 7.", "goal":"Choose", "background":"Context"
        });
        let task: ConsultationInput = serde_json::from_value(input).unwrap();
        let packet = serde_json::json!({
            "participant":"P1", "task":super::super::evidence::prompt_input(&task),
            "original_analysis":"{\"answer\":\"Use seven.\"}",
            "moderator_map":{"issues":[{"topic":"Incorrect estimate"}]}
        });
        let prompt = format!("\nReview packet:\n{packet}");
        let mut review = serde_json::json!({
            "participant":"P1", "current_answer":"Use seven.",
            "responses":[],
            "self_review":[{"original_quote":"Use seven.","decision":"maintain","updated_position":"Seven","reason":"Original data"}],
            "mapping_corrections":[{"summary_path":"/issues/0/topic","source_path":"/original_analysis/answer","correction":"Use seven","reason":"Original data"}],
            "additions":[{"kind":"unverified_claim","content":"Check the result"},
                         {"kind":"verified_fact","content":"Seven is observed","evidence":[{"source_id":"request","quote":"Observed count: 7."}]}]
        });
        let expanded = expand_citations(&review.to_string(), &prompt, &task).unwrap();
        validate(&expanded, &prompt, "P1", &task, None).unwrap();
        let expanded: Value = parse_json(&expanded).unwrap();
        assert!(expanded["additions"][1]["evidence"][0]["sha256"]
            .as_str()
            .is_some());
        review["additions"][1]["evidence"] = serde_json::json!([]);
        let expanded = expand_citations(&review.to_string(), &prompt, &task).unwrap();
        assert!(validate(&expanded, &prompt, "P1", &task, None).is_err());
        review["mapping_corrections"][0]["source_path"] =
            serde_json::json!("/moderator_map/issues/0/topic");
        assert!(expand_citations(&review.to_string(), &prompt, &task).is_err());
    }

    #[test]
    fn review_packet_excludes_other_full_answers_and_accepts_sourced_self_review() {
        let (review, prompt, task) = fixture();
        assert!(!prompt.contains("PRIVATE_OTHER_DETAILS"));
        assert!(prompt.contains("I prefer SQLite."));
        assert!(prompt.contains("Use files."));
        validate(&review.to_string(), &prompt, "P1", &task, None).unwrap();
    }

    #[test]
    fn correction_can_reference_the_reviewers_own_plain_report() {
        let (mut review, prompt, task) = fixture();
        review["mapping_corrections"] = json!([{
            "summary_path":"/moderator_map/issues/0/stances/0/position", "source_path":"/original_analysis",
            "source_quote":"I prefer SQLite.",
            "correction":"This is a preference, not proof", "reason":"The original is qualified"
        }]);
        let expanded = expand_citations(&review.to_string(), &prompt, &task).unwrap();
        validate(&expanded, &prompt, "P1", &task, None).unwrap();
        let expanded: Value = parse_json(&expanded).unwrap();
        assert_eq!(
            expanded["mapping_corrections"][0]["source_quote"],
            "I prefer SQLite."
        );
        assert_eq!(
            expanded["mapping_corrections"][0]["summary_quote"],
            "SQLite"
        );
        review["mapping_corrections"][0]["source_path"] = json!("/original_analysis/answer");
        assert!(expand_citations(&review.to_string(), &prompt, &task).is_err());
    }
    #[test]
    fn quote_layout_changes_pass_and_repair_feedback_names_all_bad_fields() {
        let (mut review, prompt, task) = fixture();
        review["responses"][0]["quote"] = json!("Use\n files.");
        validate(&review.to_string(), &prompt, "P1", &task, None).unwrap();
        review["responses"][0]["quote"] = json!("Invented other view");
        review["self_review"][0]["original_quote"] = json!("Invented own view");
        let error = validate(&review.to_string(), &prompt, "P1", &task, None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("responses[0].quote"));
        assert!(error.contains("self_review[0].original_quote"));
        assert!(error.contains("Use files."));
    }

    #[test]
    fn rejects_identity_drift_invented_positions_and_unbacked_facts() {
        let (review, prompt, task) = fixture();
        let mut missing_answer = review.clone();
        missing_answer["current_answer"] = json!("");
        assert!(validate(&missing_answer.to_string(), &prompt, "P1", &task, None).is_err());
        let mut changed = review.clone();
        changed["participant"] = json!("P2");
        assert!(validate(&changed.to_string(), &prompt, "P1", &task, None).is_err());
        let mut changed = review.clone();
        changed["self_review"][0]["original_quote"] = json!("I used to prefer files.");
        assert!(validate(&changed.to_string(), &prompt, "P1", &task, None).is_err());
        let mut changed = review.clone();
        changed["responses"][0]["quote"] = json!("PRIVATE_OTHER_DETAILS");
        assert!(validate(&changed.to_string(), &prompt, "P1", &task, None).is_err());
        let mut changed = review.clone();
        changed["additions"] = json!([{"kind":"verified_fact","content":"Claim","evidence":[]}]);
        assert!(validate(&changed.to_string(), &prompt, "P1", &task, None).is_err());
        let mut changed = review;
        changed["mapping_corrections"] = json!([{"summary_quote":"SQLite","source_quote":"I prefer SQLite.","correction":"This is a preference, not proof","reason":"The original is qualified"}]);
        validate(&changed.to_string(), &prompt, "P1", &task, None).unwrap();
    }
    #[test]
    fn critical_objections_require_visible_attribution_and_material_effect() {
        let (mut review, prompt, task) = fixture();
        review["objections"] = json!([{"participant":"P1","quote":"Prefer SQLite pending workload evidence","reason":"Workload evidence is missing","effect":"Recommendation remains conditional"}]);
        validate(&review.to_string(), &prompt, "P1", &task, None).unwrap();
        review["objections"][0]["participant"] = json!("P2");
        assert!(validate(&review.to_string(), &prompt, "P1", &task, None).is_err());
        review["objections"][0]["quote"] = json!("Use files.");
        validate(&review.to_string(), &prompt, "P1", &task, None).unwrap();
        review["objections"][0]["effect"] = json!("");
        assert!(validate(&review.to_string(), &prompt, "P1", &task, None).is_err());
    }

    #[test]
    fn verified_facts_require_resolvable_versioned_source_quotes() {
        let (mut review, prompt, task) = fixture();
        review["additions"] = json!([{"kind":"verified_fact", "content":"User asks to choose storage",
            "evidence":["nonexistent-source-id"]}]);
        assert!(validate(&review.to_string(), &prompt, "P1", &task, None).is_err());
        review["additions"][0]["evidence"] = json!([{
            "source_id":"request", "sha256":super::super::evidence::digest("Choose storage"),
            "quote":"Choose storage"}]);
        validate(&review.to_string(), &prompt, "P1", &task, None).unwrap();
        review["additions"][0]["evidence"][0]["quote"] = json!("PRIVATE_OTHER_DETAILS");
        assert!(validate(&review.to_string(), &prompt, "P1", &task, None).is_err());
        review["additions"][0]["kind"] = json!("proposal");
        assert!(validate(&review.to_string(), &prompt, "P1", &task, None).is_err());
        // Unverified attributed claims retain legacy references, without gaining
        // the source-checked status merely because the string is nonempty.
        review["additions"][0]["kind"] = json!("unverified_claim");
        review["additions"][0]["evidence"] = json!(["an earlier assistant claim"]);
        validate(&review.to_string(), &prompt, "P1", &task, None).unwrap();
    }
}
