//! A complete ledger of consequential disagreements, separate from prose packing.
use super::quotes::select_text as source_text;
use anyhow::{ensure, Context, Result};
use future_fusion::{parse_json, Consultation};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const MARKER: &str = "\nDecision audit packet:\n";
pub(super) const INSTRUCTIONS: &str = r#"Decision audit protocol: the packet lists consequential objections, not every difference. Address EVERY listed ID exactly once in decision_audit.dispositions. Each disposition has id, decision (accepted|rejected|unresolved), reason, effect, and basis (array of {source_id,source_path} or {source_id,quote}). Prefer source_path, a JSON Pointer to an existing text field in that source, such as /reason for an objection or /current_answer for a participant report. For participant reports use /response for the source report itself, or a path such as /current_answer for a text field inside its decoded JSON. Cite substantive report text; validation_errors and forwarded_truncated describe processing and cannot serve as evidence. The host copies the selected text verbatim. Use quote for an exact excerpt when you need part of a longer source or the source is plain text. Supply only one of source_path and quote. Accepted/rejected require at least one source citation AND a reason explaining why it answers the objection; quotation or majority alone is not a refutation. Accepted means its impact is incorporated; rejected means it does not overturn the conclusion; unresolved means the decisive check remains open. Never mark resolution settled with an unresolved objection. Each support disposition identifies the initial candidate and the current report for that advisor. Evaluate the original recommendation and its key support alongside any later revision. For support dispositions, accepted means the initial recommendation is incorporated, rejected means a materially different recommendation replaces it, and unresolved means its decisive comparison remains open. If replacing the recommendation, identify the premise, inference, or comparison that fails, explain how that changes the answer under the user’s criterion, and cite the particular original passage or rebutting material. A source showing that someone changed their answer proves only the revision; quote the reasoning or evidence behind it. A competing interpretation can limit certainty without invalidating the original interpretation. When the task asks for a judgment or estimate, use its stated criterion to choose the best-supported answer and retain material uncertainty; evidence for a related question does not decide the requested judgment. An accepted missing-premise objection remains an open dependency until that premise is established or its dependent claim is excluded. Do not accept the gap and then use the same assumption to select an unconditional answer. Option construction, inferred author intent and absence of contradiction do not discharge it. Set resolution according to whether the original task is resolved. Use settled only when unresolved is empty and no disposition is unresolved. If you can establish that the supplied information is insufficient to answer the task, use insufficient_information and list what is missing, even if you are certain that it is missing. If you can answer only under an unverified assumption, use conditional and identify that assumption in the answer and unresolved list. Source matches verify provenance, not truth or entailment. A missing premise cannot be disposed of merely by demanding disproof of the conclusion that depends on it. Distinguish the conclusion that is usable under established premises from an optional extension that needs an extra assumption. If an unresolved premise changes which answer or action is justified, identify exactly that dependency; do not merely label the metadata conditional while presenting the extension as established. A user-requested conditional plan or possible outcome may remain conditional; do not suppress useful supported results.
Apply accepted objections and unresolved dependencies to the actual user-facing answer before returning it; this report is delivered directly, with no later model reconciliation. Return JSON with decision_audit:{resolution:"settled|conditional|insufficient_information",dispositions:[],decision_update:null}. For a reopened decision, decision_update must be {decision:"maintain|revise",reason:"specific evidence/counterexample/corrected inference and its impact under the inherited criterion"}. Maintaining means preserving the previous user-facing answer verbatim. Revising needs a concrete justification; another vote or confidence is not enough. Inherit unresolved checks until explicitly disposed of. The previous conclusion is the working result; do not restart the whole problem. For closed tasks add decision_audit to the closed answer protocol. For general tasks use {conclusion:"self-contained answer in the requested format",unresolved:[],decision_audit:{...}}; only conclusion is delivered as prose."#;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Audit {
    pub resolution: Resolution,
    pub dispositions: Vec<Disposition>,
    pub decision_update: Option<Update>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consistency: Option<Consistency>,
}
/// Preserve optional audit metadata from historical moderator-check reports.
/// New synthesis does not request or require this field.
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    quote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_path: Option<String>,
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
            let initial = report["source_phase"] == "initial_analysis";
            let source_id = if initial {
                format!("initial:{id}")
            } else {
                format!("report:{id}")
            };
            sources.insert(
                source_id.clone(),
                json!({
                    "response":text, "source_phase":report["source_phase"],
                    "validation_errors":report["validation_errors"],
                    "forwarded_truncated":report["forwarded_truncated"],
                }),
            );
            if !initial {
                let initial_source = if report["source_phase"] == "reviewing" {
                    format!("initial:{id}")
                } else {
                    source_id.clone()
                };
                // Reuse the existing per-advisor support disposition on every
                // path. It now connects the first analysis and the current view.
                objections.insert(format!("support:{id}"), json!({
                    "topic":"Determine whether the original recommendation and its key reasoning survive the review and the original task. If the recommendation is superseded, explain the decisive error or contrary evidence and how it changes the answer.",
                    "initial_source":initial_source,
                    "report_source":source_id,
                }));
            }
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
fn participant_source(source_id: &str) -> bool {
    source_id.starts_with("report:") || source_id.starts_with("initial:")
}

fn citation_material<'a>(source: &'a Value, source_id: &str) -> (&'a Value, Option<Value>) {
    if !participant_source(source_id) {
        return (source, None);
    }
    let response = &source["response"];
    (
        response,
        response.as_str().and_then(|text| parse_json(text).ok()),
    )
}

fn citation_text<'a>(
    source: &'a Value,
    source_id: &str,
    decoded: Option<&'a Value>,
    path: &str,
) -> Option<&'a str> {
    if participant_source(source_id) {
        return if path == "/response" {
            source_text(source, path)
        } else {
            source_text(decoded?, path)
        };
    }
    source_text(source, path)
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
            let (material, decoded) = citation_material(source, &basis.source_id);
            match (&basis.source_path, &basis.quote) {
                (Some(path), None) => {
                    if citation_text(source, &basis.source_id, decoded.as_ref(), path).is_none() {
                        errors.push(format!(
                            "{field}.source_path {path:?} is not a text field in source {:?}",
                            basis.source_id
                        ));
                    }
                }
                (None, Some(quote)) => {
                    let mut candidates = decoded.as_ref().map(strings).unwrap_or_default();
                    candidates.extend(strings(material));
                    if !candidates.iter().any(|s| super::quotes::contains(s, quote)) {
                        errors.push(format!("{field}.quote is absent from source {:?}; supplied: {:?}. Copy an exact source excerpt, without adding words, or select a source_path.",
                            basis.source_id, future_fusion::bounded_text(quote, 240)));
                        if hints.len() < 8 {
                            hints.insert(
                                basis.source_id.clone(),
                                future_fusion::bounded_text(
                                    &serde_json::to_string(&candidates)?,
                                    1600,
                                ),
                            );
                        }
                    }
                }
                _ => errors.push(format!("{field}: supply one of source_path and quote")),
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
        "Settled answer cannot retain unresolved checks. Use insufficient_information if a missing requirement prevents the requested answer, or conditional if the answer depends on an unverified assumption. Keep material gaps in unresolved; use settled only when no such gaps remain."
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

pub(super) fn attach(
    result: &mut future_fusion::Conclusion,
    text: &str,
    packet: &Value,
) -> Result<()> {
    let value: Value = parse_json(text)?;
    if let Some(raw) = value.get("decision_audit") {
        let mut audit: Audit = serde_json::from_value(raw.clone())?;
        for basis in audit.dispositions.iter_mut().flat_map(|d| &mut d.basis) {
            if let Some(path) = basis.source_path.take() {
                let source = &packet["sources"][&basis.source_id];
                let (_, decoded) = citation_material(source, &basis.source_id);
                basis.quote = Some(
                    citation_text(source, &basis.source_id, decoded.as_ref(), &path)
                        .context("Decision audit source changed after validation")?
                        .to_owned(),
                );
            }
        }
        for d in audit
            .dispositions
            .iter()
            .filter(|d| d.decision == Decision::Unresolved)
        {
            result
                .unresolved
                .push(format!("{}: {} — {}", d.id, d.reason, d.effect));
        }
        result.decision_audit = Some(serde_json::to_value(audit)?);
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
    fn a_revision_keeps_the_initial_candidate_and_a_single_support_disposition() {
        let run: Consultation = serde_json::from_value(json!({
            "id":"run","parent":{"database_id":"db","session_id":"s","run_id":"r","tool_call_id":"t"},
            "input":{"question":"Choose a plan","goal":"Use supplied facts","background":"","constraints":[],"evidence":[]},
            "workspace":"/workspace","snapshot_root":"/snapshot","status":"running","phase":"synthesizing",
            "completeness":"full","created_at":0,"updated_at":0
        })).unwrap();
        let initial = json!({"participant":"P1","source_phase":"initial_analysis","response":"Choose X because it meets both limits."});
        let current = json!({"participant":"P1","source_phase":"reviewing","response":"Choose Y."});
        let packet = packet(&run, &json!({"issues":[]}), &[], &[initial, current]);
        assert_eq!(packet["objections"].as_object().unwrap().len(), 1);
        assert_eq!(
            packet["objections"]["support:P1"]["initial_source"],
            "initial:P1"
        );
        assert_eq!(
            packet["objections"]["support:P1"]["report_source"],
            "report:P1"
        );
        for (id, expected) in [
            ("initial:P1", "Choose X because it meets both limits."),
            ("report:P1", "Choose Y."),
        ] {
            let source = &packet["sources"][id];
            assert_eq!(citation_text(source, id, None, "/response"), Some(expected));
        }
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
        attach(&mut result, &value.to_string(), &prompt).unwrap();
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
    fn recorded_citation_failures_support_verified_source_selection() {
        // One retained failing citation per benchmark case. Full reports are
        // left to the live replay because a source path does not prove a claim.
        let cases: Vec<Value> =
            serde_json::from_str(include_str!("fixtures/citation-failures.json")).unwrap();
        assert_eq!(cases.len(), 7);
        for case in cases {
            let (mut value, mut packet) = fixture();
            let case_id = case["case"].as_str().unwrap();
            let source_id = case["source_id"].as_str().unwrap();
            let quote = case["quote"].as_str().unwrap();
            if case_id == "bbeh-130" {
                let task = case["source"].as_str().unwrap();
                let answer = json!({
                    "answer":"Example answer", "support":"Example support", "unresolved":[],
                    "checks":[{"claim":"Image detail", "quote":quote, "assessment":"supports",
                               "effect":"Affects the proposed answer"}]
                });
                let error =
                    super::super::answer::validate_sources(&answer.to_string(), &[task], true)
                        .unwrap_err();
                assert!(error
                    .to_string()
                    .contains("not present in the preserved request"));
                continue;
            }
            packet["sources"][source_id] = case["source"].clone();
            value["decision_audit"]["dispositions"][0]["basis"] =
                json!([{"source_id":source_id,"quote":quote}]);
            let error = validate(&value.to_string(), &packet, false).unwrap_err();
            assert!(
                error.to_string().contains("quote is absent from source"),
                "{case_id}: {error}"
            );

            value["decision_audit"]["dispositions"][0]["basis"] =
                json!([{"source_id":source_id,"source_path":case["source_path"]}]);
            let report = value.to_string();
            validate(&report, &packet, false).unwrap();
            let mut conclusion = future_fusion::Conclusion::default();
            attach(&mut conclusion, &report, &packet).unwrap();
            let stored = conclusion.decision_audit.unwrap();
            let recorded = &stored["dispositions"][0]["basis"][0];
            let (source, decoded) = citation_material(&case["source"], source_id);
            let path = case["source_path"].as_str().unwrap();
            assert_eq!(
                recorded["quote"],
                source_text(decoded.as_ref().unwrap_or(source), path).unwrap()
            );
            assert!(recorded.get("source_path").is_none());
        }
    }

    #[test]
    fn report_response_pointer_resolves_plain_and_structured_source_text() {
        let (mut value, mut packet) = fixture();
        value["decision_audit"]["dispositions"][0]["basis"] =
            json!([{"source_id":"report:P1","source_path":"/response"}]);
        for text in ["Public answer", r#"{"current_answer":"Public answer"}"#] {
            packet["sources"]["report:P1"] =
                json!({"response":text,"validation_errors":["Private validation feedback"]});
            let report = value.to_string();
            validate(&report, &packet, false).unwrap();
            let mut result = future_fusion::Conclusion::default();
            attach(&mut result, &report, &packet).unwrap();
            assert_eq!(
                result.decision_audit.unwrap()["dispositions"][0]["basis"][0]["quote"],
                text
            );
        }
    }

    #[test]
    fn source_paths_reject_invalid_fields_and_ambiguous_citations() {
        let (mut value, mut packet) = fixture();
        packet["sources"]["report:P1"] = json!({"response":"{\"participant\":\"P1\",\"reason\":\" \",\"count\":5,\"text\":\"Original evidence\"}"});
        for path in [
            "/missing",
            "/participant",
            "/reason",
            "/count",
            "/validation_errors/0",
            "",
        ] {
            value["decision_audit"]["dispositions"][0]["basis"] =
                json!([{"source_id":"report:P1","source_path":path}]);
            assert!(
                validate(&value.to_string(), &packet, false).is_err(),
                "{path}"
            );
        }
        value["decision_audit"]["dispositions"][0]["basis"] =
            json!([{"source_id":"report:P1","source_path":"/text","quote":"Original evidence"}]);
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
