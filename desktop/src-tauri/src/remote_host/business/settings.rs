use crate::remote::protocol::IncomingCmd;
use crate::remote::services::ReplySink;
use serde_json::{json, Value};

use super::{missing_session, qualified_model_id, reply, reply_unit};

pub(super) async fn execute(cmd: &IncomingCmd, sink: &dyn ReplySink) {
    match cmd.cmd_type.as_str() {
        "get_desktop_settings" => {
            reply_settings(sink, crate::store::get_app_settings()).await;
        }
        "update_desktop_settings" => {
            let result = parse_settings_patch(cmd.settings.clone())
                .and_then(crate::store::update_app_settings);
            reply_settings(sink, result).await;
        }
        "list_settings_models" => match crate::agent_bridge::get_available_models().await {
            Ok(data) => reply(sink, true, data, None).await,
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
        "list_available_skills" => match crate::agent_bridge::list_available_skills().await {
            Ok(skills) => reply(sink, true, json!({ "skills": skills }), None).await,
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
        "install_skill" => {
            let result =
                crate::agent_bridge::install_skill(cmd.skill_id.clone(), cmd.version.clone()).await;
            if result.is_ok() {
                crate::agent_events::publish_invalidation("skills_changed");
            }
            reply_unit(sink, result).await;
        }
        "uninstall_skill" => match crate::agent_bridge::uninstall_skill(cmd.skill_id.clone()).await
        {
            Ok(removed) => {
                if removed {
                    crate::agent_events::publish_invalidation("skills_changed");
                }
                reply(sink, true, json!({ "removed": removed }), None).await
            }
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
        // The phone asks the desktop's agent for a recommendation. Best-effort
        // by contract (see `agent_bridge::suggest_skill`): a failure is reported
        // as "no recommendation" rather than an error, so a phone whose desktop
        // is mid-upgrade still sends its message.
        "suggest_skill" => {
            let candidates = parse_candidates(&cmd.candidates);
            match crate::agent_bridge::suggest_skill(&cmd.query, candidates).await {
                Ok(Some(skill)) => {
                    reply(
                        sink,
                        true,
                        json!({ "skill": { "name": skill.name, "description": skill.description } }),
                        None,
                    )
                    .await;
                }
                Ok(None) => reply(sink, true, json!({ "skill": Value::Null }), None).await,
                Err(error) => {
                    reply(
                        sink,
                        true,
                        json!({ "skill": Value::Null }),
                        Some(&error.to_string()),
                    )
                    .await
                }
            }
        }
        "skill_reco_today" => match crate::store::skill_reco_today() {
            Ok(today) => reply(sink, true, json!({ "today": today }), None).await,
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
        "record_skill_reco" => {
            let result = if cmd.skill_id.is_empty() {
                Err("skill_id is required".into())
            } else {
                crate::store::record_skill_reco(&cmd.skill_id, &cmd.message_hash)
            };
            reply_unit(sink, result).await;
        }
        "get_state" => {
            // A remote state request is an active view of this conversation,
            // not a background catalog check.
            let _ = crate::agent_bridge::ensure_observer_for_session(&cmd.session_id);
            match crate::agent_bridge::get_session_state(cmd.session_id.clone()).await {
                Ok(data) => reply(sink, true, data, None).await,
                Err(error) if missing_session(&error) => reply(sink, true, json!({}), None).await,
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            }
        }
        "list_models" | "get_available_models" => {
            match crate::agent_bridge::get_available_models().await {
                Ok(mut data) => match crate::store::get_app_settings() {
                    Ok(settings) => {
                        if let Some(models) = data.get_mut("models").and_then(Value::as_array_mut) {
                            let had_models = !models.is_empty();
                            models.retain(|model| {
                                let id = model["id"].as_str().unwrap_or_default();
                                let provider = model["provider"].as_str().unwrap_or_default();
                                let key = if provider.is_empty() {
                                    id.to_string()
                                } else {
                                    format!("{provider}/{id}")
                                };
                                !settings.hidden_models.contains(&key)
                            });
                            data["allModelsHidden"] = json!(had_models && models.is_empty());
                        }
                        reply(sink, true, data, None).await;
                    }
                    Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
                },
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            }
        }
        "list_skills" => match crate::agent_bridge::list_installed_skills().await {
            Ok(skills) => reply(sink, true, json!({ "skills": skills }), None).await,
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
        "set_model" => {
            reply_unit(
                sink,
                crate::agent_bridge::set_session_model(
                    cmd.session_id.clone(),
                    qualified_model_id(&cmd.model_id, &cmd.provider_id).unwrap_or_default(),
                )
                .await,
            )
            .await;
        }
        "set_thinking_level" => {
            reply_unit(
                sink,
                crate::agent_bridge::set_session_thinking_level(
                    cmd.session_id.clone(),
                    cmd.level.clone(),
                )
                .await,
            )
            .await;
        }
        "get_settings" => match crate::store::get_app_settings() {
            Ok(settings) => {
                let sandbox_available = match product_sandbox_available().await {
                    Ok(available) => available,
                    Err(error) => {
                        reply(sink, false, Value::Null, Some(&error.to_string())).await;
                        return;
                    }
                };
                reply(
                    sink,
                    true,
                    json!({
                        "approvalTier": settings.approval_tier,
                        "sandboxAvailable": sandbox_available,
                    }),
                    None,
                )
                .await;
            }
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
        "set_approval_tier" => {
            let tier = if cmd.tier == "sandbox" {
                match product_sandbox_available().await {
                    Ok(true) => cmd.tier.clone(),
                    Ok(false) => "manual".to_string(),
                    Err(error) => {
                        reply(sink, false, Value::Null, Some(&error.to_string())).await;
                        return;
                    }
                }
            } else {
                cmd.tier.clone()
            };
            match crate::store::update_app_settings(crate::store::UpdateAppSettingsInput {
                approval_tier: Some(tier),
                ..Default::default()
            }) {
                Ok(settings) => {
                    reply(
                        sink,
                        true,
                        json!({ "approvalTier": settings.approval_tier }),
                        None,
                    )
                    .await
                }
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            }
        }
        _ => unreachable!("settings handler received {}", cmd.cmd_type),
    }
}

/// A deliberately narrow write surface. A paired phone cannot change debug
/// environment, billing, language, credentials, or arbitrary stored keys here.
///
/// Provider credentials are the one exception, and they live in their own
/// module ([`super::providers`]) behind that module's own write-only contract
/// (`hasApiKey` is the only key state a phone ever sees).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SettingsPatch {
    auto_upgrade_skills: Option<bool>,
    auto_title_first_turn: Option<bool>,
    auto_connect_remote: Option<bool>,
    skill_recommend: Option<bool>,
    hidden_models: Option<Vec<String>>,
}

fn parse_settings_patch(
    value: Value,
) -> Result<crate::store::UpdateAppSettingsInput, crate::AppError> {
    let patch: SettingsPatch = serde_json::from_value(value)?;
    if patch.hidden_models.as_ref().is_some_and(|models| {
        models.len() > 10_000 || models.iter().any(|id| id.is_empty() || id.len() > 1024)
    }) {
        return Err("Invalid model visibility list".into());
    }
    Ok(crate::store::UpdateAppSettingsInput {
        auto_upgrade_skills: patch.auto_upgrade_skills,
        auto_title_first_turn: patch.auto_title_first_turn,
        auto_connect_remote: patch.auto_connect_remote,
        skill_recommend: patch.skill_recommend,
        hidden_models: patch.hidden_models,
        ..Default::default()
    })
}

/// Skill candidates from an `suggest_skill` command.
///
/// A malformed entry is skipped rather than failing the command: the candidate
/// set is the phone's guess at the catalogue, and a recommendation made from
/// the entries that did parse is better than none. An entry without a name is
/// unusable (the name is what the agent matches on), but its description may be
/// empty.
fn parse_candidates(value: &Value) -> Vec<crate::agent_bridge::SkillCandidate> {
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let name = item.get("name")?.as_str()?.trim();
            if name.is_empty() {
                return None;
            }
            Some(crate::agent_bridge::SkillCandidate {
                name: name.to_string(),
                description: item
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            })
        })
        .collect()
}

async fn reply_settings(
    sink: &dyn ReplySink,
    result: Result<crate::store::AppSettings, crate::AppError>,
) {
    match result {
        Ok(settings) => {
            reply(
                sink,
                true,
                json!({
                    "autoUpgradeSkills": settings.auto_upgrade_skills,
                    "autoTitleFirstTurn": settings.auto_title_first_turn,
                    "autoConnectRemote": settings.auto_connect_remote,
                    "hiddenModels": settings.hidden_models,
                }),
                None,
            )
            .await
        }
        Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
    }
}

async fn product_sandbox_available() -> Result<bool, crate::AppError> {
    #[cfg(target_os = "macos")]
    {
        Ok(true)
    }
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        crate::agent_bridge::probe_sandbox()
            .await
            .map(|result| result.available)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_candidates, parse_settings_patch};
    use serde_json::json;

    /// The phone's candidate set is a guess at the catalogue, so a malformed
    /// entry is skipped rather than failing the whole recommendation: the
    /// entries that parsed still describe what the user could install.
    #[test]
    fn candidates_skip_unusable_entries() {
        let parsed = parse_candidates(&json!([
            { "name": "future-web", "description": "search the web" },
            // No description is fine — the name is what the model matches on.
            { "name": "future-paper" },
            // No name is unusable.
            { "description": "anonymous" },
            // A blank name is the same as no name.
            { "name": "   ", "description": "blank" },
            "not an object",
        ]));
        assert_eq!(
            parsed.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["future-web", "future-paper"]
        );
        assert_eq!(parsed[0].description, "search the web");
        assert_eq!(parsed[1].description, "", "a missing description is empty");
    }

    #[test]
    fn candidates_tolerate_a_missing_or_wrong_typed_field() {
        // A client that never sends candidates, or sends the wrong shape, must
        // not blow up the command.
        assert!(parse_candidates(&serde_json::Value::Null).is_empty());
        assert!(parse_candidates(&json!("nope")).is_empty());
        assert!(parse_candidates(&json!({ "name": "x" })).is_empty());
    }

    /// The phone can flip the same settings the desktop UI can, including the
    /// skill-recommendation toggle.
    #[test]
    fn settings_patch_accepts_the_recommendation_toggle() {
        let patch = parse_settings_patch(json!({ "skillRecommend": false })).expect("parses");
        assert_eq!(patch.skill_recommend, Some(false));
        // Absent means "leave it alone", not "set it to false".
        let untouched = parse_settings_patch(json!({})).expect("parses");
        assert_eq!(untouched.skill_recommend, None);
    }
}
