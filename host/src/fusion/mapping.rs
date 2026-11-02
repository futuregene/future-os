//! Stage two organizes existing statements; it does not contribute an analysis.
use anyhow::{ensure, Context, Result};
use future_fusion::parse_json;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(super) const INSTRUCTIONS: &str = r#"You are the moderator acting ONLY as a faithful organizer, not an analyst or judge. Return ONLY JSON with exactly these three arrays:
{"consensus":[{"point":"existing shared view","sources":[{"participant":"P1","quote":"exact excerpt"}]}],"issues":[{"topic":"existing disagreement","stances":[{"participant":"P1","status":"stated","position":"faithful summary","quote":"exact excerpt"},{"participant":"P2","status":"not_mentioned","position":"","quote":""}]}],"excluded":[{"participant":"P1","quote":"exact excerpt of existing statement","reason":"why it is outside an existing boundary or unsuitable for continued discussion","basis":"exact excerpt of an existing task boundary from Decision input"}]}
1. Consensus is ONLY the intersection explicitly stated by EVERY supplied participant. Cite an exact excerpt from each participant. Agreement does not establish truth. Do not upgrade shared assumptions to verified facts.
2. Issues contain ONLY materially different existing claims, with at least two explicitly stated positions. Include every supplied participant once; mark an unmentioned position as not_mentioned with empty position and quote. Silence is not opposition. Preserve qualifications; do not invent a conflict from complementary details or omissions.
3. Excluded contains ONLY existing statements that exceed explicit task constraints/scope or are otherwise unsuitable under an existing boundary in Decision input. Quote both the statement and that boundary. Do not exclude a minority view, an unfamiliar approach or a statement merely because you think it is wrong. With no applicable existing boundary, do not invent one.
Never add blind spots, omissions, clarification questions, new checks, new facts, proposed solutions, recommendations or your own technical judgment. Do not resolve disagreements or merge approaches. Use empty arrays when a category has no entries. Use only supplied participant IDs. Quotes must be exact contiguous excerpts of the supplied public answers, not paraphrases or background."#;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Mapping {
    consensus: Vec<Agreement>,
    issues: Vec<Issue>,
    excluded: Vec<Excluded>,
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
                    .is_some_and(|answer| answer.contains(excerpt)),
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
                .any(|boundary| boundary.contains(&excluded.basis)),
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
    fn sourced_mapping_supports_nonconsecutive_actual_participants_and_explicit_silence() {
        let (mapping, prompt) = fixture();
        validate_shape(&mapping.to_string(), 5).unwrap();
        validate_sources(&mapping.to_string(), &prompt).unwrap();
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
