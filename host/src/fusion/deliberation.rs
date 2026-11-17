//! A complete ledger of consequential disagreements, separate from prose packing.
use anyhow::{ensure, Context, Result};
use future_fusion::{parse_json, Consultation};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const MARKER: &str = "\nDecision audit packet:\n";
pub(super) const INSTRUCTIONS: &str = r#"Decision audit protocol: the packet lists consequential objections, not every difference. Address EVERY listed ID exactly once in decision_audit.dispositions. Each disposition has id, decision (accepted|rejected|unresolved), reason, effect, and basis (array of {source_id,quote}, exact excerpts from packet.sources). Accepted/rejected require at least one source excerpt AND a reason explaining why it answers the objection; quotation or majority alone is not a refutation. Accepted means its impact is incorporated; rejected means it does not overturn the conclusion; unresolved means the decisive check remains open. Never mark resolution settled with an unresolved objection. An accepted missing-premise objection remains an open dependency until that premise is established or its dependent claim is excluded. Do not accept the gap and then use the same assumption to select an unconditional answer. Option construction, inferred author intent and absence of contradiction do not discharge it. Use resolution conditional or insufficient_information and explain applicability in the answer and unresolved list. Source matches verify provenance, not truth or entailment. A missing premise cannot be disposed of merely by demanding disproof of the conclusion that depends on it. Distinguish the conclusion that is usable under established premises from an optional extension that needs an extra assumption. If an unresolved premise changes which answer or action is justified, identify exactly that dependency; do not merely label the metadata conditional while presenting the extension as established. A user-requested conditional plan or possible outcome may remain conditional; do not suppress useful supported results.
The backend will separately reconcile the draft with the final answer before delivery. Leave consistency null in this draft. Return JSON with decision_audit:{resolution:"settled|conditional|insufficient_information",dispositions:[],decision_update:null}. For a reopened decision, decision_update must be {decision:"maintain|revise",reason:"specific evidence/counterexample/corrected inference and its impact under the inherited criterion"}. Maintaining means preserving the previous user-facing answer verbatim. Revising needs a concrete justification; another vote or confidence is not enough. Inherit unresolved checks until explicitly disposed of. The previous conclusion is the working result; do not restart the whole problem. For closed tasks add decision_audit to the closed answer protocol. For general tasks use {conclusion:"self-contained answer in the requested format",unresolved:[],decision_audit:{...}}; only conclusion is delivered as prose."#;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Audit {
    pub resolution: Resolution,
    pub dispositions: Vec<Disposition>,
    pub decision_update: Option<Update>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consistency: Option<Consistency>,
}
/// Internal consistency is a model assessment, never backend proof of truth.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Consistency {
    answer: String,
    dependencies: Vec<Dependency>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Dependency {
    id: String,
    status: DependencyStatus,
    reason: String,
    answer_quote: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum DependencyStatus {
    Discharged,
    Excluded,
    Conditional,
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
            // All convergence paths have this obligation, including a single
            // advisor, direct consensus, and an invalid/missing disagreement map.
            // This is a backend request to check support, not an invented stance.
            objections.insert(format!("support:{id}"), json!({
                "topic":"Check consequential premises and limitations in this advisor's recommendation against the original task. Identify any unsupported dependency and its effect on the delivered answer; agreement or compatibility alone does not establish it.",
                "report_source":format!("report:{id}"),
            }));
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

pub(super) const CONSISTENCY_INSTRUCTIONS: &str = r#"Reconcile ONLY your draft final report with the original task and the decision audit packet already in this session. This is a final consistency check, not a new investigation. No tools, new facts, votes, or full re-solving. Review every disposition, including support:* obligations on single-advisor/consensus paths, against the actual answer that will be delivered.
For each consequential dependency choose discharged (the supplied evidence or a corrected inference actually establishes what the answer needs), excluded (the unsupported dependent claim is absent from the delivered answer), or conditional (the delivered answer explicitly states its assumption and applicability). An accepted missing-premise objection is NOT discharged by a guessed intention, option design, lack of a counterexample, or changing the original decision criterion. Quoting the objection proves only that it was raised. If your disposition accepts that a premise is missing, propagate its impact to the answer: remove the unsupported selection or recommendation, or make its applicability explicit if the user requested a conditional result. Do not keep an unconditional extension and merely qualify the support/metadata. For a strict closed answer format, deliver the established answer; keep speculative extensions out of it. Do not remove a supported claim merely because some other part is uncertain. A discharged counterexample needs a source-grounded explanation of why it fails, rather than a restatement of your preferred answer.
Correct only inconsistencies you identify. Preserve the answer when the audit supports it. Return the complete replacement report in its original format, including all source checks and dispositions. Add decision_audit.consistency:{answer:"exact copy of the final answer field (or conclusion for general tasks)",dependencies:[{id:"each disposition ID exactly once",status:"discharged|excluded|conditional",reason:"specific relationship between the disposition, established support, and actual final answer",answer_quote:"for conditional only: exact excerpt of the delivered answer stating the dependency; otherwise empty string"}]}. Conditional dependencies require resolution conditional or insufficient_information and a nonempty unresolved list. This is a model assessment, not a verification label. Stop after this report."#;

pub(super) fn validate_consistency(text: &str, packet: &Value, closed: bool) -> Result<()> {
    validate(text, packet, closed)?;
    let value: Value = parse_json(text)?;
    let audit: Audit = serde_json::from_value(value["decision_audit"].clone())?;
    let consistency = audit
        .consistency
        .as_ref()
        .context("Missing final conclusion consistency check")?;
    let answer = value[if closed { "answer" } else { "conclusion" }]
        .as_str()
        .context("Missing final answer")?;
    ensure!(
        consistency.answer == answer,
        "Consistency check must bind to the exact delivered answer, not a different draft"
    );
    let ids = audit
        .dispositions
        .iter()
        .map(|d| d.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    for dependency in &consistency.dependencies {
        ensure!(
            ids.contains(dependency.id.as_str()) && seen.insert(dependency.id.as_str()),
            "Consistency dependencies must cover each disposition exactly once: {}",
            dependency.id
        );
        ensure!(
            !dependency.reason.trim().is_empty(),
            "Dependency {} needs a concrete answer-impact reason",
            dependency.id
        );
        ensure!(
            dependency.status != DependencyStatus::Discharged
                || !audit
                    .dispositions
                    .iter()
                    .any(|d| d.id == dependency.id && d.decision == Decision::Unresolved),
            "Unresolved objection {} cannot be declared discharged by the consistency check",
            dependency.id
        );
        if dependency.status == DependencyStatus::Conditional {
            ensure!(
                audit.resolution != Resolution::Settled,
                "Conditional dependency {} cannot have a settled resolution",
                dependency.id
            );
            ensure!(
                value["unresolved"]
                    .as_array()
                    .is_some_and(|v| !v.is_empty()),
                "Conditional dependency {} needs an unresolved check",
                dependency.id
            );
            ensure!(
                !dependency.answer_quote.trim().is_empty()
                    && super::quotes::contains(answer, &dependency.answer_quote),
                "Conditional dependency {} must be qualified in the actual delivered answer",
                dependency.id
            );
        } else {
            ensure!(
                dependency.answer_quote.is_empty(),
                "Only conditional dependencies use answer_quote"
            );
        }
    }
    ensure!(
        seen == ids,
        "Consistency check omitted a disposition dependency"
    );
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
        assert_eq!(packet["objections"].as_object().unwrap().len(), 2);
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
    fn single_advisor_and_consensus_without_a_map_still_require_support_dispositions() {
        let run: Consultation = serde_json::from_value(json!({
            "id":"run","parent":{"database_id":"db","session_id":"s","run_id":"r","tool_call_id":"t"},
            "input":{"question":"Recommend a plan","goal":"Use supplied facts","background":"","constraints":[],"evidence":[]},
            "workspace":"/workspace","snapshot_root":"/snapshot","status":"running","phase":"synthesizing",
            "completeness":"single","created_at":0,"updated_at":0
        })).unwrap();
        let report = json!({"participant":"P3","response":"Choose X, but premise Y is unverified","validation_errors":["bad source quote"]});
        let single = packet(
            &run,
            &json!({"issues":[]}),
            &[],
            std::slice::from_ref(&report),
        );
        assert!(required(&single));
        assert!(single["objections"]["support:P3"].is_object());
        assert_eq!(
            single["sources"]["report:P3"]["validation_errors"],
            json!(["bad source quote"])
        );
        let mut second = report.clone();
        second["participant"] = json!("P1");
        let consensus = packet(&run, &json!({"issues":[]}), &[], &[report, second]);
        assert!(required(&consensus));
        assert_eq!(consensus["objections"].as_object().unwrap().len(), 2);
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
    fn final_check_binds_dependencies_and_qualifications_to_the_delivered_answer() {
        let (mut value, packet) = fixture();
        assert!(validate_consistency(&value.to_string(), &packet, false).is_err());
        value["unresolved"] = json!(["The antecedent is not established"]);
        value["decision_audit"]["consistency"] = json!({"answer":value["conclusion"],"dependencies":[{"id":"I1","status":"conditional","reason":"Only applies if the premise holds","answer_quote":"if the premise holds"}]});
        validate_consistency(&value.to_string(), &packet, false).unwrap();
        let good = value.clone();
        value["conclusion"] = json!("Return 7 unconditionally.");
        assert!(validate_consistency(&value.to_string(), &packet, false).is_err());
        value["decision_audit"]["consistency"]["answer"] = value["conclusion"].clone();
        assert!(validate_consistency(&value.to_string(), &packet, false).is_err());
        value = good.clone();
        value["decision_audit"]["resolution"] = json!("settled");
        assert!(validate_consistency(&value.to_string(), &packet, false).is_err());
        value = good;
        value["decision_audit"]["consistency"]["dependencies"] = json!([]);
        assert!(validate_consistency(&value.to_string(), &packet, false).is_err());
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
