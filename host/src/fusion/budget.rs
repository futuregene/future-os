//! Conservative admission using UTF-8 bytes (not chars/4 token estimates).
use anyhow::{ensure, Context, Result};
use future_fusion::{Configuration, StepSettings};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone)]
pub(super) struct Budget {
    models: BTreeMap<String, (usize, usize)>,
    participant_bytes: usize,
    mapping_bytes: usize,
}
impl Budget {
    pub fn new(catalog: &Value, config: &Configuration, input_bytes: usize) -> Result<Self> {
        let mut models = BTreeMap::new();
        for profile in config.panel.iter().chain([&config.moderator]) {
            let Some(model) = catalog["models"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|m| {
                    format!(
                        "{}/{}",
                        m["provider"].as_str().unwrap_or_default(),
                        m["id"].as_str().unwrap_or_default()
                    ) == profile.model
                })
            else {
                continue;
            };
            let window = model["contextWindow"]
                .as_u64()
                .context("Fusion requires a known model context window")?
                as usize;
            ensure!(window > 0, "Fusion requires a positive context window");
            let output = model["maxTokens"]
                .as_u64()
                .filter(|n| *n > 0)
                .context("Fusion requires a known positive output capacity")?
                as usize;
            models.insert(profile.model.clone(), (window, output.max(1).min(window)));
        }
        let &(window, output) = models
            .get(&config.moderator.model)
            .context("Missing moderator capacity")?;
        // Reserve generation space for capacity planning, independently of the
        // actual per-request generation limit. Count real task/report bytes;
        // transport JSON escaping is not model-visible context.
        let base = future_fusion::CONTEXT_MARGIN
            .saturating_add(input_bytes)
            .saturating_add(16_384);
        let room = window.saturating_sub(output.min(window / 4).saturating_add(base));
        let review_room = models
            .values()
            .map(|(w, o)| w.saturating_sub(o.min(&(w / 4)).saturating_add(base)))
            .min()
            .unwrap_or(0);
        // A reviewer receives its own report and the moderator map.
        let participant_bytes = (room / config.panel.len().max(1)).min(review_room / 2);
        ensure!(participant_bytes >= 1024, "Task context leaves insufficient moderator capacity; narrow the discussion before dispatch");
        let mapping_bytes = review_room.saturating_sub(participant_bytes).min(room);
        ensure!(
            mapping_bytes >= 1024,
            "Insufficient context for a sourced moderator map"
        );
        Ok(Self {
            models,
            participant_bytes,
            mapping_bytes,
        })
    }
    pub fn apply(
        &self,
        model: &str,
        phase: &str,
        prompt: &str,
        settings: &mut StepSettings,
    ) -> Result<()> {
        let &(window, natural_output) = self
            .models
            .get(model)
            .context("Unknown Fusion model capacity")?;
        let public_bytes = match phase {
            "analyzing" | "reviewing" | "synthesizing" => self.participant_bytes,
            "issue_mapping" => self.mapping_bytes,
            _ => usize::MAX,
        };
        let remaining =
            window.saturating_sub(prompt.len().saturating_add(future_fusion::CONTEXT_MARGIN));
        let output = natural_output.min(remaining);
        // Generation includes reasoning as well as public text. Only public
        // text is forwarded; its byte allocation is validated independently.
        // A downstream byte quota must never become a reasoning-token cap.
        ensure!(
            output >= 256,
            "Fusion prompt cannot fit the model context with output reserve"
        );
        settings.context_window = window;
        settings.max_output_tokens = output.min(i32::MAX as usize) as i32;
        settings.max_public_bytes = if public_bytes == usize::MAX {
            0
        } else {
            public_bytes
        };
        future_fusion::guard_context(prompt.len(), window, output)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn catalog(window: usize) -> Value {
        json!({"models":[
            {"provider":"future","id":"deepseek-flash","contextWindow":window,"maxTokens":32768},
            {"provider":"future","id":"glm-5.3-flash","contextWindow":window,"maxTokens":32768},
            {"provider":"future","id":"qwen3.7-plus","contextWindow":window,"maxTokens":32768}
        ]})
    }
    #[test]
    fn sharing_more_roles_reduces_output_without_overbooking_moderator() {
        let mut config = Configuration::default();
        let smaller = Budget::new(&catalog(262144), &config, 1000).unwrap();
        config
            .panel
            .extend([config.panel[0].clone(), config.panel[0].clone()]);
        let larger = Budget::new(&catalog(262144), &config, 1000).unwrap();
        assert!(larger.participant_bytes < smaller.participant_bytes);
        assert!(larger.participant_bytes * 5 + 32768 + future_fusion::CONTEXT_MARGIN < 262144);
        assert!(Budget::new(&catalog(32768), &config, 30000).is_err());
        assert!(Budget::new(&catalog(0), &config, 0).is_err());
        let mut mixed = catalog(1048576);
        mixed["models"][1]["contextWindow"] = json!(131072);
        assert!(Budget::new(&mixed, &Configuration::default(), 1000).is_ok());
    }
    #[test]
    fn participant_reasoning_is_independent_of_forwarded_public_bytes() {
        let mut catalog = catalog(1_000_000);
        for model in catalog["models"].as_array_mut().unwrap() {
            model["maxTokens"] = json!(1_000_000);
        }
        for count in 2..=5 {
            let mut config = Configuration::default();
            config.panel = vec![config.panel[0].clone(); count];
            // Reproduce the device run that reduced DeepSeek to 9,929 tokens
            // by dividing its 39,718-byte public allocation by four.
            let budget = Budget::new(&catalog, &config, 1748).unwrap();
            for phase in ["analyzing", "reviewing"] {
                for retry in [None, Some(super::super::RetryReason::OutputLimit)] {
                    let mut settings =
                        super::super::attempt_settings(&config, &config.panel[0], retry, 0)
                            .unwrap();
                    budget
                        .apply(&config.panel[0].model, phase, "task", &mut settings)
                        .unwrap();
                    assert_eq!(
                        settings.max_output_tokens,
                        1_000_000 - 4 - future_fusion::CONTEXT_MARGIN as i32
                    );
                    assert_eq!(settings.max_public_bytes, budget.participant_bytes);
                    assert!(settings.max_output_tokens as usize > settings.max_public_bytes / 4);
                    future_fusion::guard_context(
                        4,
                        settings.context_window,
                        settings.max_output_tokens as usize,
                    )
                    .unwrap();
                }
            }
            if count == 3 {
                assert!(budget.participant_bytes > 39_718 * 4);
            }
        }
    }

    #[test]
    fn actual_prompt_admission_is_checked_before_dispatch() {
        let config = Configuration::default();
        let budget = Budget::new(&catalog(262144), &config, 1000).unwrap();
        let mut settings =
            super::super::attempt_settings(&config, &config.panel[0], None, 0).unwrap();
        budget
            .apply(&config.panel[0].model, "analyzing", "task", &mut settings)
            .unwrap();
        assert!(settings.max_output_tokens > 0);
        assert!(settings.max_public_bytes > 0);
        budget
            .apply(
                &config.moderator.model,
                "issue_mapping",
                "task",
                &mut settings,
            )
            .unwrap();
        assert_eq!(settings.max_output_tokens, 32768);
        assert!(settings.max_public_bytes > 0);
        assert!(budget
            .apply(
                &config.moderator.model,
                "synthesizing",
                &"x".repeat(262144),
                &mut settings
            )
            .is_err());
    }
}
