use crate::remote::protocol::IncomingCmd;
use crate::remote::services::ReplySink;
use serde_json::{json, Value};

use super::{missing_session, qualified_model_id, reply, reply_unit};

pub(super) async fn execute(cmd: &IncomingCmd, sink: &dyn ReplySink) {
    match cmd.cmd_type.as_str() {
        "get_state" => match crate::agent_bridge::get_session_state(cmd.session_id.clone()).await {
            Ok(data) => reply(sink, true, data, None).await,
            Err(error) if missing_session(&error) => reply(sink, true, json!({}), None).await,
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
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
