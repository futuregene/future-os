//! Compact terminal projection; original records remain available by step ID.
use super::i18n::{t, Key, Language};
use serde_json::Value;
use std::collections::BTreeMap;

fn state(step: &Value) -> Key {
    if step["failure_kind"] == "budget_reserved" {
        return Key::FusionSkipped;
    }
    match step["status"].as_str().unwrap_or_default() {
        "starting" | "running" => Key::FusionThinking,
        "completed" => Key::FusionThought,
        "skipped" => Key::FusionSkipped,
        "cancelled" => Key::FusionStopped,
        "interrupted" => Key::FusionThinkInterrupted,
        "failed" => {
            let error = step["error"].as_str().unwrap_or_default().to_lowercase();
            if step["failure_kind"] == "timeout"
                || error.contains("timed out")
                || error.contains("timeout")
            {
                Key::FusionTimeout
            } else if step["failure_kind"] == "output_limit"
                || error.contains("output_limit")
                || error.contains("finish_length")
            {
                Key::FusionOutputLimit
            } else {
                Key::FusionThinkFailed
            }
        }
        _ => Key::FusionWaiting,
    }
}
pub(super) fn progress(data: &Value, lang: Language) -> String {
    let phases = ["analyzing", "issue_mapping", "reviewing", "synthesizing"];
    let phase = data["run"]["phase"].as_str().unwrap_or("preparing");
    let index = phases.iter().position(|p| *p == phase);
    let status = data["run"]["status"].as_str().unwrap_or_default();
    let terminal = matches!(status, "completed" | "failed" | "cancelled" | "interrupted");
    let label = match status {
        "completed" => Key::FusionComplete,
        "failed" => Key::FusionFailed,
        "cancelled" => Key::FusionCancelled,
        "interrupted" => Key::FusionInterrupted,
        _ => match phase {
            "preflight" => Key::FusionChecking,
            "analyzing" => Key::FusionAnalyzing,
            "issue_mapping" => Key::FusionMapping,
            "reviewing" => Key::FusionReviewing,
            "synthesizing" => Key::FusionSummarizing,
            _ => Key::FusionPreparing,
        },
    };
    let report = &data["run"]["report"];
    let contains_phase = |field: &str, phase: &str| {
        report[field]
            .as_array()
            .is_some_and(|values| values.iter().any(|value| value == phase))
    };
    let bar = (0..4)
        .map(|n| {
            if contains_phase("skipped_phases", phases[n]) {
                "...."
            } else if contains_phase("completed_phases", phases[n])
                || (report.is_null()
                    && (status == "completed" || index.is_some_and(|index| n < index)))
            {
                "===="
            } else if Some(n) == index {
                ">>>>"
            } else {
                "----"
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let mut text = format!("{}  [{bar}]", t(lang, label));
    let mut latest = BTreeMap::new();
    for step in data["steps"].as_array().into_iter().flatten() {
        let key = format!("{}:{}", step["phase"], step["participant"]);
        let replace = latest
            .get(&key)
            .is_none_or(|old: &&Value| step["attempt"].as_i64() >= old["attempt"].as_i64());
        if replace {
            latest.insert(key, step);
        }
    }
    let config = serde_json::from_value::<future_fusion::Configuration>(
        data["run"]["configuration"].clone(),
    )
    .ok();
    let roles = if let Some(config) = config {
        let mut roles = vec![(
            Some("moderator".to_string()),
            config.moderator.model,
            Some(t(lang, Key::FusionMainRole).to_string()),
        )];
        roles.extend(config.panel.into_iter().enumerate().map(|(index, model)| {
            (
                Some(format!("P{}", index + 1)),
                model.model,
                Some(super::i18n::render(
                    lang,
                    Key::FusionParticipantRole,
                    &[&(index + 1).to_string()],
                )),
            )
        }));
        roles
    } else {
        future_fusion::PANEL_MODELS
            .iter()
            .map(|model| (None, model.to_string(), None))
            .collect()
    };
    for (participant, model, role) in roles {
        let mut steps = latest
            .values()
            .copied()
            .filter(|s| {
                s["model"].as_str() == Some(model.as_str())
                    && participant
                        .as_deref()
                        .is_none_or(|participant| s["participant"].as_str() == Some(participant))
            })
            .collect::<Vec<_>>();
        steps.sort_by_key(|s| s["started_at"].as_i64());
        let current = steps
            .iter()
            .copied()
            .rfind(|s| s["phase"].as_str() == Some(phase));
        let mut key = current
            .or_else(|| steps.last().copied())
            .map(state)
            .unwrap_or(Key::FusionWaiting);
        if !terminal && matches!(phase, "analyzing" | "reviewing") && current.is_none() {
            key = Key::FusionWaiting;
        }
        if terminal && matches!(key, Key::FusionThinking | Key::FusionWaiting) {
            key = if status == "cancelled" {
                Key::FusionStopped
            } else {
                Key::FusionThinkInterrupted
            };
        }
        if terminal
            && key == Key::FusionThought
            && steps.iter().any(|s| {
                matches!(
                    state(s),
                    Key::FusionThinkFailed
                        | Key::FusionTimeout
                        | Key::FusionOutputLimit
                        | Key::FusionStopped
                        | Key::FusionThinkInterrupted
                )
            })
        {
            key = Key::FusionModelPartial;
        }
        let name = match model.as_str() {
            "future/deepseek-flash" => "DeepSeek Flash",
            "future/glm-5.3-flash" => "GLM 5.3 Flash",
            "future/qwen3.7-plus" => "Qwen 3.7 Plus",
            _ => &model,
        };
        let role = role.map(|role| format!("{role} · ")).unwrap_or_default();
        text.push_str(&format!("\n{role}{name} · {}", t(lang, key)));
    }
    let reasons = report["degradations"].as_array();
    let has =
        |code: &str| reasons.is_some_and(|items| items.iter().any(|item| item["code"] == code));
    let mut notices = vec![];
    if !report.is_null() {
        if status == "failed" {
            notices.push(Key::FusionNoConclusion);
        }
        if has("moderator_unavailable") || has("moderator_failed") {
            notices.push(Key::FusionModeratorFailed);
        } else if has("no_participants") {
            notices.push(Key::FusionNoParticipants);
        } else if has("provider_unavailable") {
            notices.push(Key::FusionProviderUnavailable);
        } else if has("single_participant") {
            notices.push(Key::FusionSingleParticipant);
        } else if [
            "participant_failed",
            "participant_unavailable",
            "participants_reduced",
            "reviews_reduced",
            "no_reviews",
        ]
        .iter()
        .any(|code| has(code))
        {
            notices.push(Key::FusionReducedParticipation);
        }
        if has("summary_budget_reserved") {
            notices.push(Key::FusionBudgetReserved);
        } else if has("mapping_invalid") {
            notices.push(Key::FusionDirectReview);
        }
        if status == "failed"
            && report["available_results"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
        {
            notices.push(Key::FusionMaterialsRetained);
        }
    }
    if terminal
        && notices.is_empty()
        && data["run"]["completeness"]
            .as_str()
            .is_some_and(|v| v != "full")
    {
        notices.push(Key::FusionPartial);
    }
    for key in notices {
        text.push_str(&format!("\n{}", t(lang, key)));
    }

    text
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_participant_is_visible_in_a_completed_discussion() {
        let data = serde_json::json!({"run":{"status":"completed","phase":"synthesizing","completeness":"partial"},"steps":[{"model":"future/glm-5.3-flash","phase":"reviewing","participant":"P2","status":"failed","error":"Fusion model timed out","attempt":0,"started_at":1}]});
        let summary = progress(&data, Language::Zh);
        assert!(summary.contains("GLM 5.3 Flash · 思考超时"));
        assert!(summary.contains("部分模型未完成讨论"));
        assert!(!summary.contains("started_at"));
    }
    #[test]
    fn skipped_phases_and_missing_conclusion_have_distinct_projection() {
        let data = serde_json::json!({"run":{"status":"failed","phase":"synthesizing","completeness":"none","report":{"completed_phases":["analyzing"],"skipped_phases":["issue_mapping","reviewing"],"degradations":[{"code":"moderator_failed"}],"available_results":[{"text":"analysis"}]}},"steps":[{"model":"future/glm-5.3-flash","phase":"analyzing","participant":"P2","status":"skipped","attempt":0,"started_at":1}]});
        let text = progress(&data, Language::Zh);
        assert!(text.contains("[==== .... .... >>>>]"));
        assert!(text.contains("GLM 5.3 Flash · 已跳过"));
        assert!(text.contains("本次未产出综合结论。"));
        assert!(text.contains("主模型不可用，未自动更换模型。"));
        assert!(text.contains("已有分析与降级原因已保留"));
    }
}
