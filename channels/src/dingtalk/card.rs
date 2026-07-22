//! DingTalk AI Card streaming response.
//! Mirrors the OpenClaw connector's card.ts flow:
//!   create → INPUTING → streaming → FINISHED

use anyhow::Result;
use serde_json::json;
use tracing::{info, warn};

/// AI Card template ID (same as OpenClaw).
const CARD_TEMPLATE_ID: &str = "02fcf2f4-5e02-4a85-b672-46d1f715543e.schema";

/// AI Card flow states.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CardStatus {
    Inputing,
    Finished,
}

impl CardStatus {
    fn as_str(&self) -> &'static str {
        match self {
            CardStatus::Inputing => "INPUTING",
            CardStatus::Finished => "FINISHED",
        }
    }
}

/// An active AI Card instance.
pub struct AiCard {
    pub card_instance_id: String,
    pub access_token: String,
    pub inputing_started: bool,
}

/// Create an AI Card instance for a conversation.
pub async fn create_ai_card(
    domain: &str,
    token: &str,
    client_id: &str,
    target: &CardTarget,
) -> Result<AiCard> {
    let card_instance_id = format!(
        "card_{}_{}",
        std::time::UNIX_EPOCH
            .elapsed()
            .unwrap_or_default()
            .as_millis(),
        unique_id()
    );
    let api = format!("https://{}/v1.0/card/instances", domain);

    let client = crate::tls::http_client();

    // Step 1: Create card instance
    let create_body = json!({
        "cardTemplateId": CARD_TEMPLATE_ID,
        "outTrackId": card_instance_id,
        "cardData": {
            "cardParamMap": {
                "config": r#"{"autoLayout": true}"#,
            }
        },
        "callbackType": "STREAM",
        "imGroupOpenSpaceModel": {"supportForward": true},
        "imRobotOpenSpaceModel": {"supportForward": true},
    });

    client
        .post(&api)
        .header("x-acs-dingtalk-access-token", token)
        .header("Content-Type", "application/json")
        .json(&create_body)
        .send()
        .await?;
    info!("[DING CARD] created {}", card_instance_id);

    // Step 2: Deliver card
    let deliver_api = format!("{}/deliver", api);
    let deliver_body = match target {
        CardTarget::User { user_id } => json!({
            "outTrackId": card_instance_id,
            "robotCode": client_id,
            "imRobotOpenSpaceModel": {"supportForward": true},
            "userId": user_id,
        }),
        CardTarget::Group {
            open_conversation_id,
        } => json!({
            "outTrackId": card_instance_id,
            "robotCode": client_id,
            "imGroupOpenSpaceModel": {"supportForward": true},
            "openConversationId": open_conversation_id,
        }),
    };

    client
        .post(&deliver_api)
        .header("x-acs-dingtalk-access-token", token)
        .header("Content-Type", "application/json")
        .json(&deliver_body)
        .send()
        .await?;
    info!("[DING CARD] delivered {}", card_instance_id);

    Ok(AiCard {
        card_instance_id,
        access_token: token.to_string(),
        inputing_started: false,
    })
}

/// Stream content to an AI Card (automatically sets INPUTING on first call).
pub async fn stream_ai_card(
    domain: &str,
    card: &mut AiCard,
    content: &str,
    finished: bool,
) -> Result<()> {
    let api = format!("https://{}/v1.0/card", domain);
    let client = crate::tls::http_client();

    // Set INPUTING state on first call
    if !card.inputing_started {
        let status_body = json!({
            "outTrackId": card.card_instance_id,
            "cardData": {
                "cardParamMap": {
                    "flowStatus": CardStatus::Inputing.as_str(),
                    "msgContent": normalize_content(content),
                    "staticMsgContent": "",
                    "sys_full_json_obj": r#"{"order": ["msgContent"]}"#,
                    "config": r#"{"autoLayout": true}"#,
                },
            },
        });

        client
            .put(format!("{}/instances", api))
            .header("x-acs-dingtalk-access-token", &card.access_token)
            .header("Content-Type", "application/json")
            .json(&status_body)
            .send()
            .await?;
        card.inputing_started = true;
    }

    // Stream content update
    let stream_content = if finished {
        normalize_content(content)
    } else {
        normalize_content(content)
            .trim_end_matches('\n')
            .to_string()
    };

    let stream_body = json!({
        "outTrackId": card.card_instance_id,
        "guid": format!("{}_{}",
            std::time::UNIX_EPOCH.elapsed().unwrap_or_default().as_millis(),
            unique_id()
        ),
        "key": "msgContent",
        "content": stream_content,
        "isFull": true,
        "isFinalize": finished,
        "isError": false,
    });

    client
        .put(format!("{}/streaming", api))
        .header("x-acs-dingtalk-access-token", &card.access_token)
        .header("Content-Type", "application/json")
        .json(&stream_body)
        .send()
        .await?;

    if finished {
        // Set FINISHED state
        let finish_body = json!({
            "outTrackId": card.card_instance_id,
            "cardData": {
                "cardParamMap": {
                    "flowStatus": CardStatus::Finished.as_str(),
                    "msgContent": normalize_content(content),
                    "staticMsgContent": "",
                    "sys_full_json_obj": r#"{"order": ["msgContent"]}"#,
                    "config": r#"{"autoLayout": true}"#,
                },
            },
            "cardUpdateOptions": {"updateCardDataByKey": true},
        });

        client
            .put(format!("{}/instances", api))
            .header("x-acs-dingtalk-access-token", &card.access_token)
            .header("Content-Type", "application/json")
            .json(&finish_body)
            .send()
            .await?;
    }

    Ok(())
}

/// Close/cleanup a card that failed to create or was interrupted.
pub async fn close_ai_card(domain: &str, card: &AiCard, error_msg: &str) {
    let api = format!("https://{}/v1.0/card/instances", domain);
    let client = crate::tls::http_client();

    let body = json!({
        "outTrackId": card.card_instance_id,
        "cardData": {
            "cardParamMap": {
                "flowStatus": CardStatus::Finished.as_str(),
                "msgContent": format!("Error: {}", error_msg),
                "staticMsgContent": "",
                "sys_full_json_obj": r#"{"order": ["msgContent"]}"#,
                "config": r#"{"autoLayout": true}"#,
            },
        },
        "cardUpdateOptions": {"updateCardDataByKey": true},
    });

    if let Err(e) = client
        .put(&api)
        .header("x-acs-dingtalk-access-token", &card.access_token)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
    {
        warn!("[DING CARD] close failed: {}", e);
    }
}

/// Target for card delivery.
pub enum CardTarget {
    User { user_id: String },
    Group { open_conversation_id: String },
}

/// Simple unique ID without external crate dependency.
fn unique_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!("{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// Normalize markdown content for AI Card display.
fn normalize_content(s: &str) -> String {
    // AI Card has a text length limit. Keep it reasonable.
    if s.len() > 20000 {
        format!("{}...", &s[..19950])
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── CardStatus ─────────────────────────────────────────────────────────

    #[test]
    fn card_status_as_str() {
        assert_eq!(CardStatus::Inputing.as_str(), "INPUTING");
        assert_eq!(CardStatus::Finished.as_str(), "FINISHED");
    }

    #[test]
    fn card_status_equality() {
        assert_eq!(CardStatus::Inputing, CardStatus::Inputing);
        assert_ne!(CardStatus::Inputing, CardStatus::Finished);
    }

    // ─── normalize_content ──────────────────────────────────────────────────

    #[test]
    fn normalize_short_content_unchanged() {
        let s = "hello world";
        assert_eq!(normalize_content(s), s);
    }

    #[test]
    fn normalize_empty_string() {
        assert_eq!(normalize_content(""), "");
    }

    #[test]
    fn normalize_exact_limit() {
        let s = "x".repeat(20000);
        assert_eq!(normalize_content(&s), s);
    }

    #[test]
    fn normalize_over_limit_truncates() {
        let s = "x".repeat(25000);
        let result = normalize_content(&s);
        assert!(result.ends_with("..."));
        assert_eq!(result.len(), 19950 + 3);
    }

    #[test]
    fn normalize_unicode_content() {
        let s = "你好世界".repeat(100);
        let result = normalize_content(&s);
        assert!(!result.is_empty());
    }

    // ─── unique_id ──────────────────────────────────────────────────────────

    #[test]
    fn unique_id_increments() {
        let a = unique_id();
        let b = unique_id();
        assert_ne!(a, b);
        let a_num: u64 = a.parse().unwrap();
        let b_num: u64 = b.parse().unwrap();
        assert!(b_num > a_num);
    }

    #[test]
    fn unique_id_is_numeric_string() {
        let id = unique_id();
        assert!(id.parse::<u64>().is_ok());
    }

    // ─── AiCard struct ──────────────────────────────────────────────────────

    #[test]
    fn ai_card_fields() {
        let card = AiCard {
            card_instance_id: "card_123".to_string(),
            access_token: "token_abc".to_string(),
            inputing_started: false,
        };
        assert_eq!(card.card_instance_id, "card_123");
        assert!(!card.inputing_started);
    }

    // ─── CardTarget ─────────────────────────────────────────────────────────

    #[test]
    fn card_target_user() {
        let target = CardTarget::User {
            user_id: "user_123".to_string(),
        };
        match target {
            CardTarget::User { user_id } => assert_eq!(user_id, "user_123"),
            _ => panic!("expected User target"),
        }
    }

    #[test]
    fn card_target_group() {
        let target = CardTarget::Group {
            open_conversation_id: "conv_456".to_string(),
        };
        match target {
            CardTarget::Group {
                open_conversation_id,
            } => assert_eq!(open_conversation_id, "conv_456"),
            _ => panic!("expected Group target"),
        }
    }
}
