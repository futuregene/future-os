//! A complete ledger of consequential disagreements, separate from prose packing.
use anyhow::{ensure, Context, Result};
use future_fusion::{parse_json, Consultation};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const MARKER: &str = "\nDecision audit packet:\n";
pub(super) const INSTRUCTIONS: &str = r#"Decision audit protocol: the packet lists consequential objections, not every difference. Address EVERY listed ID exactly once in decision_audit.dispositions. Each disposition has id, decision (accepted|rejected|unresolved), reason, effect, and basis (array of {source_id,quote}, exact excerpts from packet.sources). Accepted/rejected require at least one source excerpt AND a reason explaining why it answers the objection; quotation or majority alone is not a refutation. Accepted means its impact is incorporated; rejected means it does not overturn the conclusion; unresolved means the decisive check remains open. Never mark resolution settled with an unresolved objection. Use resolution conditional or insufficient_information and explain applicability in the answer and unresolved list. Source matches verify provenance, not truth or entailment. A missing premise cannot be disposed of merely by demanding disproof of the conclusion that depends on it. Distinguish the conclusion that is usable under established premises from an optional extension that needs an extra assumption. If an unresolved premise changes which answer or action is justified, identify exactly that dependency; do not merely label the metadata conditional while presenting the extension as established. A user-requested conditional plan or possible outcome may remain conditional; do not suppress useful supported results.
Return JSON with decision_audit:{resolution:"settled|conditional|insufficient_information",dispositions:[],decision_update:null}. For a reopened decision, decision_update must be {decision:"maintain|revise",reason:"specific evidence/counterexample/corrected inference and its impact under the inherited criterion"}. Maintaining means preserving the previous user-facing answer verbatim. Revising needs a concrete justification; another vote or confidence is not enough. Inherit unresolved checks until explicitly disposed of. The previous conclusion is the working result; do not restart the whole problem. For closed tasks add decision_audit to the closed answer protocol. For general tasks use {conclusion:"self-contained answer in the requested format",unresolved:[],decision_audit:{...}}; only conclusion is delivered as prose."#;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Audit {
    pub resolution: Resolution,
    pub dispositions: Vec<Disposition>,
    pub decision_update: Option<Update>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Resolution {
    Settled,
    Conditional,
    InsufficientInformation,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Disposition {
    id: String,
    decision: Decision,
    reason: String,
    effect: String,
    basis: Vec<Basis>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Decision {
    Accepted,
    Rejected,
    Unresolved,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Basis {
    source_id: String,
    quote: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Update {
    decision: Change,
    reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Change {
    Maintain,
    Revise,
}

pub(super) fn packet(
    run: &Consultation,
    mapping: &Value,
    reviews: &[Value],
    current: &[Value],
) -> Value {
    let mut objections = BTreeMap::new();
    let mut sources = BTreeMap::new();
    sources.insert(
        "request".to_string(),
        json!(run
            .input
            .parent_request
            .as_deref()
            .unwrap_or(&run.input.question)),
    );
    for (index, evidence) in run.input.evidence.iter().enumerate() {
        sources.insert(format!("evidence:{index}"), json!(evidence));
    }
    for report in current {
        if let (Some(id), Some(text)) =
            (report["participant"].as_str(), report["response"].as_str())
        {
            sources.insert(
                format!("report:{id}"),
                json!({
                    "response":text, "source_phase":report["source_phase"],
                    "validation_errors":report["validation_errors"],
                    "forwarded_truncated":report["forwarded_truncated"],
                }),
            );
        }
    }
    for issue in mapping["issues"].as_array().into_iter().flatten() {
        if let Some(id) = issue["id"].as_str() {
            objections.insert(id.to_string(), issue.clone());
        }
    }
    // Review validators already require these quotes to be from the named
    // participant's visible original excerpt. Retained invalid reviews cannot
    // manufacture new mandatory objections.
    for review in reviews {
        if review["validation_errors"]
            .as_array()
            .is_some_and(|v| !v.is_empty())
        {
            continue;
        }
        let Some(text) = review["response"].as_str() else {
            continue;
        };
        let Ok(value) = parse_json::<Value>(text) else {
            continue;
        };
        for (i, objection) in value["objections"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            objections.insert(
                format!(
                    "O{}-{}",
                    review["participant"].as_str().unwrap_or_default(),
                    i + 1
                ),
                objection.clone(),
            );
        }
        for (i, response) in value["responses"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            if response["decision"] != "agree" {
                let id = format!(
                    "R{}-{}",
                    review["participant"].as_str().unwrap_or_default(),
                    i + 1
                );
                objections.insert(id, response.clone());
            }
        }
    }
    if let Some(lineage) = &run.lineage {
        sources.insert("previous_decision".into(), json!(lineage.previous_result));
        sources.insert(
            "previous_request".into(),
            json!(lineage
                .previous_input
                .parent_request
                .as_deref()
                .unwrap_or(&lineage.previous_input.question)),
        );
        for (index, evidence) in lineage.previous_input.evidence.iter().enumerate() {
            sources.insert(format!("previous_evidence:{index}"), json!(evidence));
        }
        for (i, pending) in lineage.previous_result.unresolved.iter().enumerate() {
            objections.insert(
                format!("previous:{}:U{}", lineage.revisit.previous_run_id, i + 1),
                json!({"unresolved":pending}),
            );
        }
    }
    for (id, objection) in &objections {
        sources.insert(format!("objection:{id}"), objection.clone());
    }
    json!({"objections":objections,"sources":sources,"lineage":run.lineage})
}

pub(super) fn required(packet: &Value) -> bool {
    packet["lineage"].is_object()
        || packet["objections"]
            .as_object()
            .is_some_and(|v| !v.is_empty())
}
pub(super) fn suffix(packet: &Value) -> Result<String> {
    Ok(format!(
        "\n{INSTRUCTIONS}{MARKER}{}",
        serde_json::to_string(packet)?
    ))
}
fn strings(value: &Value) -> Vec<&str> {
    match value {
        Value::String(s) => vec![s],
        Value::Array(a) => a.iter().flat_map(strings).collect(),
        Value::Object(o) => o.values().flat_map(strings).collect(),
        _ => vec![],
    }
}
// Validate against backend-owned state, never a ledger parsed out of participant
// prose; an untrusted report may itself contain prompt-like delimiters.
pub(super) fn validate(text: &str, packet: &Value, closed: bool) -> Result<()> {
    let result: Value = parse_json(text)?;
    let audit: Audit = serde_json::from_value(result["decision_audit"].clone())
        .context("Missing or invalid decision_audit")?;
    let objections = packet["objections"]
        .as_object()
        .context("Missing objection ledger")?;
    let mut seen = BTreeSet::new();
    let mut errors = vec![];
    let mut hints = BTreeMap::new();
    for (index, d) in audit.dispositions.iter().enumerate() {
        let field = format!("decision_audit.dispositions[{index}] ({})", d.id);
        if !objections.contains_key(&d.id) {
            errors.push(format!("{field}: unknown objection ID"));
        }
        if !seen.insert(&d.id) {
            errors.push(format!("{field}: duplicate objection ID"));
        }
        if d.reason.trim().is_empty() || d.effect.trim().is_empty() {
            errors.push(format!("{field}: requires reason and effect"));
        }
        if d.decision != Decision::Unresolved && d.basis.is_empty() {
            errors.push(format!(
                "{field}: resolved objection requires a source basis"
            ));
        }
        for (reference, basis) in d.basis.iter().enumerate() {
            let field = format!("{field}.basis[{reference}]");
            let Some(source) = packet["sources"].get(&basis.source_id) else {
                errors.push(format!("{field}: unknown source {:?}", basis.source_id));
                continue;
            };
            let material = if basis.source_id.starts_with("report:") {
                &source["response"]
            } else {
                source
            };
            // Reports are JSON-encoded strings in the packet. Their decoded
            // string fields are the original wording too, not paraphrases.
            let decoded = if basis.source_id.starts_with("report:") {
                material
                    .as_str()
                    .and_then(|text| parse_json::<Value>(text).ok())
            } else {
                None
            };
            let mut candidates = decoded.as_ref().map(strings).unwrap_or_default();
            candidates.extend(strings(material));
            if !candidates
                .iter()
                .any(|s| super::quotes::contains(s, &basis.quote))
            {
                errors.push(format!("{field}.quote is absent from source {:?}; supplied: {:?}. Copy an exact source excerpt, without adding words.",
                    basis.source_id, future_fusion::bounded_text(&basis.quote, 240)));
                if hints.len() < 8 {
                    hints.insert(
                        basis.source_id.clone(),
                        future_fusion::bounded_text(&serde_json::to_string(&candidates)?, 1600),
                    );
                }
            }
        }
        if d.decision == Decision::Unresolved && audit.resolution == Resolution::Settled {
            errors.push(format!("{field}: unresolved objection cannot be settled"));
        }
    }
    let missing = objections
        .keys()
        .filter(|id| !seen.contains(id))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        errors.push(format!(
            "Every critical objection needs an explicit disposition; missing: {missing:?}"
        ));
    }
    ensure!(errors.is_empty(), "{}\nOriginal source excerpts for local quote repair (remaining sources stay in the original packet): {}",
        errors.join("\n"), serde_json::to_string(&hints)?);
    let answer = result[if closed { "answer" } else { "conclusion" }]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .context("Missing user-facing conclusion")?;
    let unresolved: Vec<String> =
        serde_json::from_value(result["unresolved"].clone()).context("Missing unresolved array")?;
    ensure!(
        unresolved.iter().all(|s| !s.trim().is_empty()),
        "Empty unresolved item"
    );
    ensure!(
        audit.resolution != Resolution::Settled || unresolved.is_empty(),
        "Settled answer cannot retain unresolved checks"
    );
    if packet["lineage"].is_object() {
        let update = audit
            .decision_update
            .as_ref()
            .context("Reopened decision requires decision_update")?;
        ensure!(
            !update.reason.trim().is_empty(),
            "Decision update requires a concrete justification"
        );
        let old = &packet["lineage"]["previous_result"];
        let prior = old["final_answer"]
            .as_str()
            .or_else(|| old["conclusion"].as_str())
            .context("Missing inherited conclusion")?;
        ensure!(
            update.decision != Change::Maintain || answer.trim() == prior.trim(),
            "Maintaining a decision must preserve its answer; explain a changed answer as revise"
        );
    } else {
        ensure!(
            audit.decision_update.is_none(),
            "Cannot invent a previous decision"
        );
    }
    Ok(())
}

pub(super) fn attach(result: &mut future_fusion::Conclusion, text: &str) -> Result<()> {
    let value: Value = parse_json(text)?;
    if let Some(raw) = value.get("decision_audit") {
        let audit: Audit = serde_json::from_value(raw.clone())?;
        for d in audit
            .dispositions
            .iter()
            .filter(|d| d.decision == Decision::Unresolved)
        {
            result
                .unresolved
                .push(format!("{}: {} — {}", d.id, d.reason, d.effect));
        }
        result.decision_audit = Some(raw.clone());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Value, Value) {
        let packet = json!({"objections":{"I1":{"topic":"Missing premise"}},"sources":{"request":"If the premise holds, return 7."},"lineage":null});
        let value = json!({"conclusion":"Return 7 if the premise holds.","unresolved":[],"decision_audit":{"resolution":"conditional","dispositions":[{"id":"I1","decision":"accepted","reason":"The antecedent is not established","effect":"Keep the answer conditional","basis":[{"source_id":"request","quote":"If the premise holds"}]}],"decision_update":null}});
        (value, packet)
    }
    #[test]
    fn ledger_survives_report_packing_and_preserves_quality_metadata() {
        let run: Consultation = serde_json::from_value(json!({
            "id":"run","parent":{"database_id":"db","session_id":"s","run_id":"r","tool_call_id":"t"},
            "input":{"question":"Choose storage","goal":"Keep data","background":"Unknown workload","constraints":[],"evidence":["Observed 100 writes per second"]},
            "workspace":"/workspace","snapshot_root":"/snapshot","status":"running","phase":"synthesizing",
            "completeness":"none","created_at":0,"updated_at":0
        })).unwrap();
        let long_objection = "critical premise ".repeat(1000);
        let mapping = json!({"issues":[{"id":"I1","topic":long_objection}]});
        let current = vec![
            json!({"participant":"P1","response":"Packed answer","source_phase":"reviewing","forwarded_truncated":true,"validation_errors":["unmatched quote"]}),
        ];
        let reviews = vec![
            json!({"participant":"P1","response":json!({"objections":[{"quote":"fabricated"}]}).to_string(),"validation_errors":["unmatched quote"]}),
        ];
        let packet = packet(&run, &mapping, &reviews, &current);
        assert_eq!(packet["objections"]["I1"]["topic"], long_objection);
        assert_eq!(packet["objections"].as_object().unwrap().len(), 1);
        assert_eq!(packet["sources"]["report:P1"]["forwarded_truncated"], true);
        assert_eq!(
            packet["sources"]["report:P1"]["validation_errors"],
            json!(["unmatched quote"])
        );
        assert_eq!(
            packet["sources"]["evidence:0"],
            "Observed 100 writes per second"
        );
        assert!(required(&packet));
    }

    #[test]
    fn omitted_duplicate_or_unattributed_dispositions_cannot_finalize() {
        let (value, prompt) = fixture();
        validate(&value.to_string(), &prompt, false).unwrap();
        for invalid in [
            json!([]),
            json!([
                value["decision_audit"]["dispositions"][0],
                value["decision_audit"]["dispositions"][0]
            ]),
        ] {
            let mut changed = value.clone();
            changed["decision_audit"]["dispositions"] = invalid;
            assert!(validate(&changed.to_string(), &prompt, false).is_err());
        }
        for (field, replacement) in [
            ("id", json!("unknown")),
            ("reason", json!("")),
            ("basis", json!([])),
        ] {
            let mut changed = value.clone();
            changed["decision_audit"]["dispositions"][0][field] = replacement;
            assert!(validate(&changed.to_string(), &prompt, false).is_err());
        }
        let mut changed = value;
        changed["decision_audit"]["dispositions"][0]["basis"][0]["quote"] =
            json!("The premise holds.");
        assert!(validate(&changed.to_string(), &prompt, false).is_err());
    }
    #[test]
    fn unresolved_objections_remain_visible_and_cannot_be_declared_settled() {
        let (mut value, prompt) = fixture();
        value["decision_audit"]["dispositions"][0]["decision"] = json!("unresolved");
        value["decision_audit"]["dispositions"][0]["basis"] = json!([]);
        validate(&value.to_string(), &prompt, false).unwrap();
        let mut result = future_fusion::Conclusion::default();
        attach(&mut result, &value.to_string()).unwrap();
        assert!(result.unresolved[0].starts_with("I1:"));
        value["decision_audit"]["resolution"] = json!("settled");
        assert!(validate(&value.to_string(), &prompt, false).is_err());
    }
    #[test]
    fn quote_repair_lists_every_bad_reference_and_the_original_source_wording() {
        let (mut value, packet) = fixture();
        value["decision_audit"]["dispositions"][0]["basis"] = json!([
            {"source_id":"request","quote":"If the extra premise holds"},
            {"source_id":"request","quote":"return the 7"}
        ]);
        let error = validate(&value.to_string(), &packet, false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("basis[0].quote"));
        assert!(error.contains("basis[1].quote"));
        assert!(error.contains("If the premise holds, return 7."));
        assert!(error.contains("Copy an exact source excerpt"));
    }
    #[test]
    fn report_citations_match_original_json_string_values_without_accepting_paraphrases() {
        let (mut value, mut packet) = fixture();
        packet["sources"]["report:P1"] = json!({"response":json!({"current_answer":"Keep \"existing records\".\nCheck the condition."}).to_string()});
        value["decision_audit"]["dispositions"][0]["basis"] =
            json!([{"source_id":"report:P1","quote":"Keep \"existing records\"."}]);
        validate(&value.to_string(), &packet, false).unwrap();
        value["decision_audit"]["dispositions"][0]["basis"][0]["quote"] =
            json!("Keep all existing records.");
        assert!(validate(&value.to_string(), &packet, false).is_err());
    }

    #[test]
    fn inherited_answer_changes_require_explicit_revision() {
        let (mut value, prompt) = fixture();
        let mut packet = prompt;
        packet["lineage"] = json!({"previous_result":{"conclusion":"Earlier conditional answer"}});
        let prompt = packet;
        assert!(validate(&value.to_string(), &prompt, false).is_err());
        value["decision_audit"]["decision_update"] =
            json!({"decision":"maintain","reason":"Same evidence"});
        assert!(validate(&value.to_string(), &prompt, false).is_err());
        value["decision_audit"]["decision_update"] = json!({"decision":"revise","reason":"Corrected the inference that an antecedent is established by a conditional"});
        validate(&value.to_string(), &prompt, false).unwrap();
        value["conclusion"] = json!("Earlier conditional answer");
        value["decision_audit"]["decision_update"]["decision"] = json!("maintain");
        validate(&value.to_string(), &prompt, false).unwrap();
    }
}
