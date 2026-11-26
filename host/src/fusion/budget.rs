//! Conservative admission using UTF-8 bytes (not chars/4 token estimates).
use anyhow::{ensure, Context, Result};
use future_fusion::{Configuration, StepSettings, USER_IMAGE_CONTEXT_TOKENS};
use serde_json::Value;
use std::collections::BTreeMap;

const MIN_REPORT_BYTES: usize = 1024;
const MIN_GENERATION_TOKENS: usize = 256;

#[derive(Clone, Copy)]
struct ModelCapacity {
    window: usize,
    output: usize,
    report_output_reserve: usize,
    image_limit: usize,
}

impl ModelCapacity {
    fn report_room(self, base: usize) -> usize {
        self.window.saturating_sub(
            self.report_output_reserve
                .min(self.window / 4)
                .saturating_add(base)
                .saturating_add(self.image_limit.saturating_mul(USER_IMAGE_CONTEXT_TOKENS)),
        )
    }
}

#[derive(Clone)]
pub(super) struct Budget {
    models: BTreeMap<String, ModelCapacity>,
    participant_bytes: usize,
    mapping_bytes: usize,
    participant_pool: usize,
    moderator_room: usize,
    review_room: usize,
}
impl Budget {
    pub fn new(
        catalog: &Value,
        config: &Configuration,
        input_bytes: usize,
        user_image_count: usize,
    ) -> Result<Self> {
        let mut models = BTreeMap::new();
        // Reserve generation space for planning, independently of the actual
        // per-request generation limit. Count model-visible material.
        let base = future_fusion::CONTEXT_MARGIN
            .saturating_add(input_bytes)
            .saturating_add(16_384);
        let minimum_room = MIN_REPORT_BYTES * config.panel.len().max(2);
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
            let catalog_output = model["maxTokens"]
                .as_u64()
                .filter(|n| *n > 0)
                .with_context(|| format!("Fusion model {} requires positive maxTokens from list_models; upgrade Agent and Host together", profile.model))?
                as usize;
            let output = future_fusion::output_limit_override(
                model["provider"].as_str().unwrap_or_default(),
                model["id"].as_str().unwrap_or_default(),
            )
            .map_or(catalog_output, |limit| limit as usize);
            let mut capacity = ModelCapacity {
                window,
                output: output.min(window),
                // Forwarded report sizing keeps the catalog reserve even for
                // roles that have a larger per-request generation budget.
                report_output_reserve: catalog_output,
                image_limit: 0,
            };
            if model["supportsImages"] == true {
                let image_room = capacity.report_room(base).saturating_sub(minimum_room);
                capacity.image_limit = user_image_count.min(image_room / USER_IMAGE_CONTEXT_TOKENS);
            }
            models.insert(profile.model.clone(), capacity);
        }
        let moderator = models
            .get(&config.moderator.model)
            .context("Missing moderator capacity")?;
        let room = moderator.report_room(base);
        let review_room = models
            .values()
            .map(|model| model.report_room(base))
            .min()
            .unwrap_or(0);
        // A reviewer receives its own report and the moderator map.
        let participant_bytes = (room / config.panel.len().max(1)).min(review_room / 2);
        ensure!(participant_bytes >= MIN_REPORT_BYTES, "Task context leaves insufficient moderator capacity; narrow the discussion before dispatch");
        let mapping_bytes = review_room.saturating_sub(participant_bytes).min(room);
        ensure!(
            mapping_bytes >= MIN_REPORT_BYTES,
            "Insufficient context for a sourced moderator map"
        );
        Ok(Self {
            models,
            participant_bytes,
            mapping_bytes,
            participant_pool: participant_bytes.saturating_mul(config.panel.len()),
            moderator_room: room,
            review_room,
        })
    }
    /// Pool unused allocations before discarding usable material. Full reports
    /// stay in child sessions; only forwarded material is shortened if needed.
    pub fn fit_material(&mut self, phase: &str, reports: &mut [Value]) -> Result<Vec<String>> {
        let costs = reports
            .iter()
            .map(|report| {
                super::reports::forwarded_bytes(
                    phase,
                    report["response"].as_str().unwrap_or_default(),
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let per_report = if phase == "analyzing" {
            self.review_room.saturating_sub(MIN_REPORT_BYTES)
        } else {
            usize::MAX
        };
        let mut limits = vec![0; reports.len()];
        let mut pending = (0..reports.len()).collect::<Vec<_>>();
        let mut remaining = if phase == "analyzing" {
            self.participant_pool
        } else {
            self.moderator_room
        };
        while !pending.is_empty() {
            let share = (remaining / pending.len()).min(per_report);
            let small = pending
                .iter()
                .copied()
                .filter(|&i| costs[i] <= share)
                .collect::<Vec<_>>();
            if small.is_empty() {
                for i in pending {
                    limits[i] = share;
                }
                break;
            }
            for i in small {
                limits[i] = costs[i];
                remaining = remaining.saturating_sub(costs[i]);
                pending.retain(|&j| i != j);
            }
        }
        let mut shortened = vec![];
        for (report, limit) in reports.iter_mut().zip(limits) {
            let original = report["response"].as_str().unwrap_or_default();
            if super::reports::forwarded_bytes(phase, original)? <= limit {
                continue;
            }
            // Preserve the direct answer before spending context on the review
            // bookkeeping. Never claim the omitted supporting fields passed.
            let parsed = future_fusion::parse_json::<Value>(original).ok();
            let answer = parsed
                .as_ref()
                .and_then(|v| v["current_answer"].as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(original);
            let text = super::reports::fit_text(answer, limit)?;
            report["response"] = serde_json::json!(text);
            report["forwarded_truncated"] = serde_json::json!(true);
            shortened.push(
                report["participant"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            );
        }
        if phase == "analyzing" {
            let largest = reports
                .iter()
                .map(|report| {
                    // The reviewer receives its own answer as an escaped string.
                    super::reports::forwarded_bytes(
                        phase,
                        report["response"].as_str().unwrap_or_default(),
                    )
                })
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .max()
                .unwrap_or(0);
            self.mapping_bytes = self
                .mapping_bytes
                .min(self.review_room.saturating_sub(largest));
        }
        Ok(shortened)
    }

    pub fn apply(
        &self,
        model: &str,
        phase: &str,
        prompt: &str,
        settings: &mut StepSettings,
    ) -> Result<()> {
        let capacity = self
            .models
            .get(model)
            .context("Unknown Fusion model capacity")?;
        let public_bytes = match phase {
            "analyzing" | "reviewing" | "synthesizing" => self.participant_bytes,
            "issue_mapping" => self.mapping_bytes,
            _ => usize::MAX,
        };
        let remaining = capacity
            .window
            .saturating_sub(prompt.len().saturating_add(future_fusion::CONTEXT_MARGIN));
        let image_limit = if phase == "preflight" {
            0
        } else {
            let image_room = remaining.saturating_sub(MIN_GENERATION_TOKENS);
            capacity
                .image_limit
                .min(image_room / USER_IMAGE_CONTEXT_TOKENS)
        };
        let image_tokens = image_limit * USER_IMAGE_CONTEXT_TOKENS;
        let remaining = remaining - image_tokens;
        let output = capacity.output.min(remaining);
        // Generation includes reasoning as well as public text. Only public
        // text is forwarded; its byte allocation is validated independently.
        // A downstream byte quota must never become a reasoning-token cap.
        ensure!(
            output >= MIN_GENERATION_TOKENS,
            "Fusion prompt cannot fit the model context with output reserve"
        );
        settings.context_window = capacity.window;
        settings.max_output_tokens = output.min(i32::MAX as usize) as i32;
        settings.user_image_limit = image_limit;
        settings.max_public_bytes = if public_bytes == usize::MAX {
            0
        } else {
            public_bytes
        };
        future_fusion::guard_context(
            prompt.len().saturating_add(image_tokens),
            capacity.window,
            output,
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn catalog(window: usize, provider: &str) -> Value {
        json!({"models":[
            {"provider":provider,"id":"qwen3.8-flash","contextWindow":window,"maxTokens":32768},
            {"provider":provider,"id":"glm-5.3-flash","contextWindow":window,"maxTokens":32768},
            {"provider":provider,"id":"kimi-k2.6","contextWindow":window,"maxTokens":32768}
        ]})
    }

    fn custom_config() -> Configuration {
        let mut config = Configuration::default();
        for profile in config.panel.iter_mut().chain([&mut config.moderator]) {
            profile.model = profile.model.replacen("future/", "fixture/", 1);
        }
        config
    }

    #[test]
    fn pools_spare_capacity_and_bounds_the_combined_moderator_input() {
        let mut budget =
            Budget::new(&catalog(262144, "fixture"), &custom_config(), 1000, 0).unwrap();
        let long = "A".repeat(budget.participant_bytes + 2000);
        let mut reports = vec![
            json!({"participant":"P1","response":long}),
            json!({"participant":"P2","response":"short"}),
            json!({"participant":"P3","response":"short"}),
        ];
        assert!(budget
            .fit_material("reviewing", &mut reports)
            .unwrap()
            .is_empty());
        assert_eq!(reports[0]["response"].as_str().unwrap(), long);
        for report in &mut reports {
            report["response"] = json!("中文\"".repeat(budget.participant_pool));
        }
        assert_eq!(
            budget
                .fit_material("reviewing", &mut reports)
                .unwrap()
                .len(),
            3
        );
        let bytes: usize = reports
            .iter()
            .map(|r| {
                super::super::reports::forwarded_bytes("reviewing", r["response"].as_str().unwrap())
                    .unwrap()
            })
            .sum();
        assert!(bytes <= budget.moderator_room);
        assert!(reports.iter().all(|r| r["forwarded_truncated"] == true));
    }

    #[test]
    fn typed_rpc_model_catalog_supports_the_default_fusion_budget() {
        let mut source = catalog(1_000_000, "future");
        // Match the required public model-list envelope and entries, not a
        // Host-only fixture that bypasses RPC's field filtering.
        source["defaultModel"] = json!("future/qwen3.8-flash");
        source["isScoped"] = json!(false);
        for model in source["models"].as_array_mut().unwrap() {
            model["label"] = model["id"].clone();
            model["supportsImages"] = json!(false);
            model["thinkingLevel"] = json!("default");
            model["isDefault"] = json!(false);
            model["recommended"] = json!(false);
        }
        let payload = future_rpc::encode::response_payload("list_models", &source).unwrap();
        let response = future_rpc::proto::RpcResponse {
            command: "list_models".into(),
            success: true,
            payload: Some(payload),
            ..Default::default()
        };
        let decoded = future_rpc::decode::response_data(&response);
        let config = Configuration::default();
        let budget = Budget::new(&decoded, &config, 1748, 0).unwrap();
        let large_prompt = "x".repeat(900_000);
        for profile in config.panel.iter().chain([&config.moderator]) {
            let mut settings = super::super::attempt_settings(&config, profile, None, 0).unwrap();
            budget
                .apply(&profile.model, "analyzing", "task", &mut settings)
                .unwrap();
            assert_eq!(settings.max_output_tokens, 131_072);
            budget
                .apply(&profile.model, "analyzing", &large_prompt, &mut settings)
                .unwrap();
            assert_eq!(
                settings.max_output_tokens as usize,
                100_000 - future_fusion::CONTEXT_MARGIN
            );
        }
        let mut legacy = decoded;
        legacy["models"][0]
            .as_object_mut()
            .unwrap()
            .remove("maxTokens");
        let Err(error) = Budget::new(&legacy, &config, 1748, 0) else {
            panic!("invented capacity for an old Agent");
        };
        assert!(error.to_string().contains("future/qwen3.8-flash"));
    }

    #[test]
    fn sharing_more_roles_reduces_output_without_overbooking_moderator() {
        let mut config = custom_config();
        let smaller = Budget::new(&catalog(262144, "fixture"), &config, 1000, 0).unwrap();
        config
            .panel
            .extend([config.panel[0].clone(), config.panel[0].clone()]);
        let larger = Budget::new(&catalog(262144, "fixture"), &config, 1000, 0).unwrap();
        assert!(larger.participant_bytes < smaller.participant_bytes);
        assert!(larger.participant_bytes * 5 + 32768 + future_fusion::CONTEXT_MARGIN < 262144);
        assert!(Budget::new(&catalog(32768, "fixture"), &config, 30000, 0).is_err());
        assert!(Budget::new(&catalog(0, "fixture"), &config, 0, 0).is_err());
        let mut mixed = catalog(1048576, "fixture");
        mixed["models"][1]["contextWindow"] = json!(131072);
        assert!(Budget::new(&mixed, &custom_config(), 1000, 0).is_ok());
    }
    #[test]
    fn participant_reasoning_is_independent_of_forwarded_public_bytes() {
        let mut catalog = catalog(1_000_000, "fixture");
        for model in catalog["models"].as_array_mut().unwrap() {
            model["maxTokens"] = json!(1_000_000);
        }
        for count in 2..=5 {
            let mut config = custom_config();
            config.panel = vec![config.panel[0].clone(); count];
            // Reproduce the device run that reduced a model to 9,929 tokens
            // by dividing its 39,718-byte public allocation by four.
            let budget = Budget::new(&catalog, &config, 1748, 0).unwrap();
            for phase in ["analyzing", "reviewing"] {
                for retry in [None, Some(super::super::RetryReason::InvalidOutput)] {
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
        let config = custom_config();
        let budget = Budget::new(&catalog(262144, "fixture"), &config, 1000, 0).unwrap();
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

    #[test]
    fn image_context_is_reserved_only_for_vision_steps_after_preflight() {
        let config = custom_config();
        let window = 262144;
        let mut catalog = catalog(window, "fixture");
        for model in catalog["models"].as_array_mut().unwrap() {
            model["supportsImages"] = json!(model["id"] == "qwen3.8-flash");
            model["maxTokens"] = json!(window);
        }
        let text_only = Budget::new(&catalog, &config, 1000, 0).unwrap();
        let images = Budget::new(&catalog, &config, 1000, 2).unwrap();
        let image_tokens = 2 * USER_IMAGE_CONTEXT_TOKENS;
        assert_eq!(
            images.moderator_room + image_tokens,
            text_only.moderator_room
        );
        assert_eq!(images.review_room + image_tokens, text_only.review_room);

        for (model, phase, reserve) in [
            (&config.moderator, "preflight", 0),
            (&config.moderator, "synthesizing", image_tokens),
            (&config.panel[1], "reviewing", 0),
        ] {
            let mut settings = super::super::attempt_settings(&config, model, None, 0).unwrap();
            images
                .apply(&model.model, phase, "task", &mut settings)
                .unwrap();
            assert_eq!(
                settings.user_image_limit,
                reserve / USER_IMAGE_CONTEXT_TOKENS
            );
            assert_eq!(
                settings.max_output_tokens as usize,
                window - 4 - future_fusion::CONTEXT_MARGIN - reserve
            );
        }
    }

    #[test]
    fn low_context_models_keep_report_room_with_the_images_that_fit() {
        let config = custom_config();
        let mut catalog = catalog(1_000_000, "fixture");
        for model in catalog["models"].as_array_mut().unwrap() {
            model["supportsImages"] = json!(true);
        }
        catalog["models"][1]["contextWindow"] = json!(50_000);
        catalog["models"][2]["contextWindow"] = json!(40_000);
        let budget = Budget::new(&catalog, &config, 1000, 4).unwrap();
        assert!(budget.participant_bytes >= MIN_REPORT_BYTES);
        assert!(budget.mapping_bytes >= MIN_REPORT_BYTES);
        for (model, limit) in config.panel.iter().zip([4, 1, 0]) {
            let mut settings = super::super::attempt_settings(&config, model, None, 0).unwrap();
            budget
                .apply(&model.model, "analyzing", "task", &mut settings)
                .unwrap();
            assert_eq!(settings.user_image_limit, limit);
        }
    }

    #[test]
    fn image_limit_shrinks_when_a_later_prompt_takes_more_context() {
        let config = custom_config();
        let window = 131_072;
        let mut catalog = catalog(window, "fixture");
        catalog["models"][0]["supportsImages"] = json!(true);
        let budget = Budget::new(&catalog, &config, 1000, 4).unwrap();
        let model = &config.moderator;
        let mut settings = super::super::attempt_settings(&config, model, None, 0).unwrap();
        let room = window - future_fusion::CONTEXT_MARGIN - MIN_GENERATION_TOKENS;
        let prompt = "X".repeat(room + 1);
        for limit in [4, 2, 0] {
            let len = room - USER_IMAGE_CONTEXT_TOKENS * limit;
            budget
                .apply(&model.model, "synthesizing", &prompt[..len], &mut settings)
                .unwrap();
            assert_eq!(settings.user_image_limit, limit);
            assert_eq!(settings.max_output_tokens as usize, MIN_GENERATION_TOKENS);
        }
        assert!(serde_json::to_value(&settings)
            .unwrap()
            .get("user_image_limit")
            .is_none());
        assert!(budget
            .apply(&model.model, "synthesizing", &prompt, &mut settings)
            .is_err());
    }
}
