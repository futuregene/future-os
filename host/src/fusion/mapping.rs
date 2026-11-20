//! Stage two organizes existing statements; it does not contribute an analysis.
use super::quotes::{contains, contains_report, expand, select_text};
use anyhow::{ensure, Context, Result};
use future_fusion::parse_json;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(super) const INSTRUCTIONS: &str = r#"You are the moderator organizing the supplied analyses and deciding what is worth checking. Return ONLY JSON with these four arrays:
{"consensus":[{"point":"existing shared view","sources":[{"participant":"P1","quote":"exact excerpt"}]}],"issues":[{"topic":"existing disagreement","stances":[{"participant":"P1","status":"stated","position":"faithful summary","quote":"exact excerpt"},{"participant":"P2","status":"not_mentioned","position":"","quote":""}]}],"excluded":[{"participant":"P1","quote":"exact excerpt of existing statement","reason":"why it is outside an existing boundary or unsuitable for continued discussion","basis":"exact excerpt of an existing task boundary from Decision input"}],"review_focus":["question whose resolution could change the conclusion or next action"]}
For a participant citation you may replace quote with quote_path (for example /answer or /support within that participant’s decoded response). Select a relevant text field. For a short excerpt use quote.
1. Consensus is ONLY the intersection explicitly stated by EVERY supplied participant. Cite an exact excerpt from each participant. Agreement does not establish truth. Do not upgrade shared assumptions to verified facts.
2. Issues contain ONLY materially different existing claims, with at least two explicitly stated positions. Include every supplied participant once; mark an unmentioned position as not_mentioned with empty position and quote. Silence is not opposition. Preserve qualifications; do not invent a conflict from complementary details or omissions.
3. Excluded contains ONLY existing statements that exceed explicit task constraints/scope or are otherwise unsuitable under an existing boundary in Decision input. Quote both the statement and that boundary. Do not exclude a minority view, an unfamiliar approach or a statement merely because you think it is wrong. With no applicable existing boundary, do not invent one.
4. Review focus contains only consequential unresolved questions for the panel, up to three. Do not fill the allowance. Do not reopen a derivation already independently supplied with no concrete unresolved premise, counterexample or missing coverage. If all positions agree, the map has no material issues, and the reports already support the answer, leave review_focus empty. Combine differences driven by the same premise. Ask what evidence, counterexample or decision criterion would distinguish the positions, including when all participants rely on the same unresolved assumption. Keep the original question and other plausible answers in scope. Ask for a concrete check, not a restatement of the choices. Include a check if the task explicitly requests cross-review. Leave review_focus empty only if there are no material disagreements or unresolved checks and the shared conclusion directly answers the task with adequate independent support.
Decision criterion: keep the requested standard fixed. If positions use different definitions, quantifiers or standards of support, make that difference a focused check against Decision input rather than choosing a new standard. Ask reviewers to resolve decisive counterexamples and contradictory supplied examples; a minority objection is not resolved by agreement among others.
Granularity: organize substantive positions, not every report detail. Keep the grounds and qualifications that make a position consequential, including a well-supported minority position. Cite short, exact contiguous excerpts from the supplied public answers. Agreement based on the same source or premise is repeated support, not independent confirmation.
Completion: submit once the material positions and focused checks are represented. Questions in review_focus are the moderator's requests for review, not sourced claims or task requirements. Do not answer those questions yourself in the map, invent facts or attribute your questions to participants."#;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Mapping {
    consensus: Vec<Agreement>,
    issues: Vec<Issue>,
    excluded: Vec<Excluded>,
    #[serde(default)]
    review_focus: Option<Vec<String>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Agreement {
    point: String,
    sources: Vec<Source>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    participant: String,
    quote: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Issue {
    topic: String,
    stances: Vec<Stance>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Stance {
    participant: String,
    status: Status,
    position: String,
    quote: String,
}
#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Status {
    Stated,
    NotMentioned,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Excluded {
    participant: String,
    quote: String,
    reason: String,
    basis: String,
}

pub(super) fn validate_shape(text: &str, participants: usize) -> Result<()> {
    let mapping: Mapping = parse_json(text)?;
    if let Some(focus) = &mapping.review_focus {
        ensure!(
            focus.len() <= 3 && focus.iter().all(|question| !question.trim().is_empty()),
            "Review focus must contain at most three nonempty questions"
        );
        ensure!(
            !focus.is_empty() || (mapping.issues.is_empty() && !mapping.consensus.is_empty()),
            "Direct synthesis requires a sourced shared view and no mapped issues"
        );
    }
    let valid_id = |id: &str| -> Result<()> {
        ensure!(
            (1..=participants).any(|n| id == format!("P{n}")),
            "Unknown participant"
        );
        Ok(())
    };
    for point in mapping.consensus {
        ensure!(!point.point.trim().is_empty(), "Empty shared view");
        ensure!(!point.sources.is_empty(), "Missing shared-view sources");
        let mut seen = BTreeSet::new();
        for source in point.sources {
            valid_id(&source.participant)?;
            ensure!(
                seen.insert(source.participant),
                "Duplicate shared-view source"
            );
            ensure!(!source.quote.trim().is_empty(), "Missing source quote");
        }
    }
    for issue in mapping.issues {
        ensure!(!issue.topic.trim().is_empty(), "Invalid issue topic");
        let mut seen = BTreeSet::new();
        let mut stated = 0;
        for stance in issue.stances {
            valid_id(&stance.participant)?;
            ensure!(
                seen.insert(stance.participant),
                "Duplicate issue participant"
            );
            if stance.status == Status::NotMentioned {
                ensure!(
                    stance.position.is_empty() && stance.quote.is_empty(),
                    "Silence cannot be assigned a position"
                );
            } else {
                ensure!(
                    !stance.position.trim().is_empty() && !stance.quote.trim().is_empty(),
                    "Missing stated position or quote"
                );
                stated += 1;
            }
        }
        ensure!(stated >= 2, "A disagreement requires two stated positions");
    }
    for excluded in mapping.excluded {
        valid_id(&excluded.participant)?;
        ensure!(
            !excluded.quote.trim().is_empty()
                && !excluded.reason.trim().is_empty()
                && !excluded.basis.trim().is_empty(),
            "Missing exclusion source or existing boundary"
        );
    }
    Ok(())
}

/// Called only with a validated map. Older maps without a review decision
/// keep the normal cross-review path.
pub(super) fn can_synthesize_directly(mapping: &Value) -> bool {
    mapping["review_focus"]
        .as_array()
        .is_some_and(Vec::is_empty)
}

pub(super) fn expand_citations(text: &str, prompt: &str) -> Result<String> {
    let mut report: Value = parse_json(text)?;
    ensure!(report.is_object(), "Mapping must be a JSON object");
    let (_, analyses) = prompt
        .rsplit_once("\nIndependent analyses:\n")
        .context("Missing mapping source answers")?;
    let analyses: Vec<Value> = serde_json::from_str(analyses)?;
    let sources: BTreeMap<_, _> = analyses
        .iter()
        .filter_map(|row| {
            let id = row["participant"].as_str()?;
            Some((id, parse_json::<Value>(row["response"].as_str()?).ok()?))
        })
        .collect();
    let expand_rows = |rows: &mut Value| -> Result<()> {
        for row in rows
            .as_array_mut()
            .into_iter()
            .flatten()
            .filter_map(Value::as_object_mut)
        {
            let source = row
                .get("participant")
                .and_then(Value::as_str)
                .and_then(|id| sources.get(id));
            expand(row, "quote_path", "quote", |path| {
                source.and_then(|value| select_text(value, path))
            })?;
        }
        Ok(())
    };
    for agreement in report["consensus"].as_array_mut().into_iter().flatten() {
        if let Some(rows) = agreement.get_mut("sources") {
            expand_rows(rows)?;
        }
    }
    for issue in report["issues"].as_array_mut().into_iter().flatten() {
        if let Some(rows) = issue.get_mut("stances") {
            expand_rows(rows)?;
        }
    }
    if let Some(rows) = report.get_mut("excluded") {
        expand_rows(rows)?;
    }
    Ok(report.to_string())
}

pub(super) fn validate_sources(text: &str, prompt: &str) -> Result<()> {
    let mapping: Mapping = parse_json(text)?;
    let (prefix, analyses) = prompt
        .rsplit_once("\nIndependent analyses:\n")
        .context("Missing mapping source answers")?;
    let analyses: Vec<Value> = serde_json::from_str(analyses)?;
    let answers: BTreeMap<&str, &str> = analyses
        .iter()
        .map(|a| {
            Ok((
                a["participant"]
                    .as_str()
                    .context("Missing source participant")?,
                a["response"].as_str().context("Missing source answer")?,
            ))
        })
        .collect::<Result<_>>()?;
    let (_, input) = prefix
        .rsplit_once("\nDecision input:\n")
        .context("Missing decision input")?;
    let input: Value = serde_json::from_str(input)?;
    let mut boundaries = vec![];
    for key in ["question", "goal", "background"] {
        if let Some(value) = input[key].as_str() {
            boundaries.push(value);
        }
    }
    for value in input["constraints"].as_array().into_iter().flatten() {
        if let Some(value) = value.as_str() {
            boundaries.push(value);
        }
    }
    let quote = |participant: &str, excerpt: &str| -> Result<()> {
        ensure!(
            !excerpt.trim().is_empty()
                && answers
                    .get(participant)
                    .is_some_and(|answer| contains_report(answer, excerpt)),
            "Moderator quote does not match the participant's answer"
        );
        Ok(())
    };
    let all_participants = |ids: Vec<&str>| -> Result<()> {
        let ids: BTreeSet<_> = ids.into_iter().collect();
        ensure!(
            ids.len() == answers.len() && answers.keys().all(|id| ids.contains(id)),
            "Mapping must account for every actual participant"
        );
        Ok(())
    };
    for agreement in mapping.consensus {
        all_participants(
            agreement
                .sources
                .iter()
                .map(|s| s.participant.as_str())
                .collect(),
        )?;
        for source in agreement.sources {
            quote(&source.participant, &source.quote)?;
        }
    }
    for issue in mapping.issues {
        all_participants(
            issue
                .stances
                .iter()
                .map(|s| s.participant.as_str())
                .collect(),
        )?;
        for stance in issue.stances {
            if stance.status == Status::Stated {
                quote(&stance.participant, &stance.quote)?;
            }
        }
    }
    for excluded in mapping.excluded {
        quote(&excluded.participant, &excluded.quote)?;
        ensure!(
            boundaries
                .iter()
                .any(|boundary| contains(boundary, &excluded.basis)),
            "Exclusion invented a task boundary"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> (Value, String) {
        let input = json!({"question":"Choose a storage approach", "goal":"Keep existing data", "background":"User requests a storage comparison", "constraints":["Do not delete existing data"]});
        let analyses = json!([
            {"participant":"P1", "response":"Keep data. Choose SQLite. Delete existing data."},
            {"participant":"P3", "response":"Keep data. Choose files."},
            {"participant":"P5", "response":"Keep data."}
        ]);
        let mapping = json!({
            "consensus":[{"point":"Keep data", "sources":[
                {"participant":"P1","quote":"Keep data."},
                {"participant":"P3","quote":"Keep data."},
                {"participant":"P5","quote":"Keep data."}
            ]}],
            "issues":[{"topic":"Storage", "stances":[
                {"participant":"P1","status":"stated","position":"SQLite","quote":"Choose SQLite."},
                {"participant":"P3","status":"stated","position":"Files","quote":"Choose files."},
                {"participant":"P5","status":"not_mentioned","position":"","quote":""}
            ]}],
            "excluded":[{"participant":"P1","quote":"Delete existing data.","reason":"Exceeds the user's boundary","basis":"Do not delete existing data"}]
        });
        (
            mapping,
            format!("Instructions\nDecision input:\n{input}\nIndependent analyses:\n{analyses}"),
        )
    }
    #[test]
    fn resolves_quotes_only_from_the_named_participants_frozen_report() {
        // The original bbeh:408 mapping could not carry its hand-copied quote.
        let input = serde_json::json!({"question":"Decide","goal":"Choose","background":"Context"});
        let analyses = serde_json::json!([
            {"participant":"P1","response":serde_json::json!({"answer":r#"Keep "existing data"."#}).to_string()},
            {"participant":"P2","response":"{\"answer\":\"Keep data.\"}"}
        ]);
        let prompt = format!("\nDecision input:\n{input}\nIndependent analyses:\n{analyses}");
        let mut map = serde_json::json!({"consensus":[{"point":"Keep data","sources":[
            {"participant":"P1","quote_path":"/answer"},{"participant":"P2","quote_path":"/answer"}
        ]}],"issues":[],"excluded":[]});
        let expanded = expand_citations(&map.to_string(), &prompt).unwrap();
        validate_shape(&expanded, 2).unwrap();
        validate_sources(&expanded, &prompt).unwrap();
        let expanded: Value = parse_json(&expanded).unwrap();
        assert_eq!(
            expanded["consensus"][0]["sources"][0]["quote"],
            "Keep \"existing data\"."
        );
        map["consensus"][0]["sources"][0]["participant"] = serde_json::json!("P3");
        assert!(expand_citations(&map.to_string(), &prompt).is_err());
    }

    #[test]
    fn sourced_mapping_supports_nonconsecutive_actual_participants_and_explicit_silence() {
        let (mapping, prompt) = fixture();
        validate_shape(&mapping.to_string(), 5).unwrap();
        validate_sources(&mapping.to_string(), &prompt).unwrap();
    }
    #[test]
    fn skipping_review_requires_an_explicit_decision_and_no_mapped_disagreement() {
        let (mut mapping, prompt) = fixture();
        assert!(!can_synthesize_directly(&mapping));

        mapping["review_focus"] = json!(["Which approach preserves existing data?"]);
        validate_shape(&mapping.to_string(), 5).unwrap();
        assert!(!can_synthesize_directly(&mapping));

        mapping["review_focus"] = json!([]);
        assert!(validate_shape(&mapping.to_string(), 5).is_err());

        mapping["issues"] = json!([]);
        validate_shape(&mapping.to_string(), 5).unwrap();
        validate_sources(&mapping.to_string(), &prompt).unwrap();
        assert!(can_synthesize_directly(&mapping));
    }
    #[test]
    fn rejects_new_analysis_silence_as_opposition_and_invented_references() {
        let (mapping, prompt) = fixture();
        let mut changed = mapping.clone();
        changed["blind_spots"] = json!(["Ask about performance"]);
        assert!(validate_shape(&changed.to_string(), 5).is_err());
        let mut changed = mapping.clone();
        changed["issues"][0]["stances"][2]["position"] = json!("Opposes SQLite");
        assert!(validate_shape(&changed.to_string(), 5).is_err());
        let mut changed = mapping.clone();
        changed["consensus"][0]["sources"][0]["quote"] = json!("Invented claim");
        assert!(validate_sources(&changed.to_string(), &prompt).is_err());
        let mut changed = mapping.clone();
        changed["consensus"][0]["sources"]
            .as_array_mut()
            .unwrap()
            .pop();
        assert!(validate_sources(&changed.to_string(), &prompt).is_err());
        let mut changed = mapping.clone();
        changed["issues"][0]["stances"][0]["participant"] = json!("P2");
        assert!(validate_sources(&changed.to_string(), &prompt).is_err());
        let mut changed = mapping;
        changed["excluded"][0]["basis"] = json!("Must use SQLite");
        assert!(validate_sources(&changed.to_string(), &prompt).is_err());
    }
}
