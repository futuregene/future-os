//! Compact terminal projection; original records remain available by step ID.
use super::i18n::{t, Key, Language};
use serde_json::Value;
use std::collections::BTreeMap;

fn state(step: &Value) -> Key {
    match step["status"].as_str().unwrap_or_default() {
        "starting" | "running" => Key::FusionThinking,
        "completed" => Key::FusionThought,
        "cancelled" => Key::FusionStopped,
        "interrupted" => Key::FusionThinkInterrupted,
        "failed" => {
            let error = step["error"].as_str().unwrap_or_default().to_lowercase();
            if error.contains("timed out") || error.contains("timeout") {
                Key::FusionTimeout
            } else if error.contains("output_limit") || error.contains("finish_length") {
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
    let index = phases.iter().position(|p| *p == phase).unwrap_or(0);
    let status = data["run"]["status"].as_str().unwrap_or_default();
    let terminal = matches!(status, "completed" | "failed" | "cancelled" | "interrupted");
    let label = match status {
        "completed" => Key::FusionComplete,
        "failed" => Key::FusionFailed,
        "cancelled" => Key::FusionCancelled,
        "interrupted" => Key::FusionInterrupted,
        _ => match phase {
            "analyzing" => Key::FusionAnalyzing,
            "issue_mapping" => Key::FusionMapping,
            "reviewing" => Key::FusionReviewing,
            "synthesizing" => Key::FusionSummarizing,
            _ => Key::FusionPreparing,
        },
    };
    let bar = (0..4)
        .map(|n| {
            if status == "completed" || n < index {
                "===="
            } else if n == index {
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
    if terminal
        && data["run"]["completeness"]
            .as_str()
            .is_some_and(|v| v != "full")
    {
        text.push_str(&format!("\n{}", t(lang, Key::FusionPartial)));
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
}
