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
    let api = format!("{}/v1.0/card/instances", super::config::base_url(domain));

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
    let api = format!("{}/v1.0/card", super::config::base_url(domain));
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
    let api = format!("{}/v1.0/card/instances", super::config::base_url(domain));
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
        assert!(matches!(target, CardTarget::User { ref user_id } if user_id == "user_123"));
    }

    #[test]
    fn card_target_group() {
        let target = CardTarget::Group {
            open_conversation_id: "conv_456".to_string(),
        };
        assert!(
            matches!(target, CardTarget::Group { ref open_conversation_id } if open_conversation_id == "conv_456")
        );
    }

    // ─── HTTP flows against the mock server ──────────────────────────────────

    use crate::test_support::{self as ts, HttpRoute};

    #[tokio::test(flavor = "multi_thread")]
    async fn create_ai_card_user_target() {
        ts::ensure_crypto_provider();
        let routes = vec![
            HttpRoute::json("/v1.0/card/instances", 200, "{}"),
            HttpRoute::json("/v1.0/card/instances/deliver", 200, "{}"),
        ];
        let (base, recorded) = ts::spawn_http(routes).await;
        let card = create_ai_card(
            &base,
            "tok",
            "robot-1",
            &CardTarget::User {
                user_id: "u-1".into(),
            },
        )
        .await
        .unwrap();
        assert!(card.card_instance_id.starts_with("card_"));
        assert_eq!(card.access_token, "tok");
        assert!(!card.inputing_started);
        let deliver = ts::requests_to(&recorded, "/v1.0/card/instances/deliver");
        assert_eq!(deliver.len(), 1);
        assert!(deliver[0].body_string().contains("\"userId\":\"u-1\""));
        assert!(deliver[0]
            .body_string()
            .contains("\"robotCode\":\"robot-1\""));
        assert_eq!(
            deliver[0].header("x-acs-dingtalk-access-token"),
            Some("tok")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn create_ai_card_group_target() {
        ts::ensure_crypto_provider();
        let routes = vec![
            HttpRoute::json("/v1.0/card/instances", 200, "{}"),
            HttpRoute::json("/v1.0/card/instances/deliver", 200, "{}"),
        ];
        let (base, recorded) = ts::spawn_http(routes).await;
        let card = create_ai_card(
            &base,
            "tok",
            "robot-1",
            &CardTarget::Group {
                open_conversation_id: "oc-1".into(),
            },
        )
        .await
        .unwrap();
        let deliver = ts::requests_to(&recorded, "/v1.0/card/instances/deliver");
        assert!(deliver[0]
            .body_string()
            .contains("\"openConversationId\":\"oc-1\""));
        drop(card);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn create_ai_card_send_failure() {
        ts::ensure_crypto_provider();
        // Nothing listening → connection refused surfaces as Err.
        let err = create_ai_card(
            "http://127.0.0.1:1",
            "tok",
            "r",
            &CardTarget::User {
                user_id: "u".into(),
            },
        )
        .await
        .err()
        .unwrap();
        assert!(!err.to_string().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stream_ai_card_inputing_then_finished() {
        ts::ensure_crypto_provider();
        let routes = vec![
            HttpRoute::json("/v1.0/card/instances", 200, "{}"),
            HttpRoute::json("/v1.0/card/streaming", 200, "{}"),
        ];
        let (base, recorded) = ts::spawn_http(routes).await;
        let mut card = AiCard {
            card_instance_id: "card_x".into(),
            access_token: "tok".into(),
            inputing_started: false,
        };
        // First call: INPUTING status PUT + streaming PUT (not finalized,
        // trailing newlines trimmed).
        stream_ai_card(&base, &mut card, "hello\n\n", false)
            .await
            .unwrap();
        assert!(card.inputing_started);
        // Second call: no INPUTING PUT; finished=true → FINISH PUT.
        stream_ai_card(&base, &mut card, "done", true)
            .await
            .unwrap();

        let instances = ts::requests_to(&recorded, "/v1.0/card/instances");
        // INPUTING once, FINISH once.
        assert_eq!(instances.len(), 2);
        assert!(instances[0].body_string().contains("INPUTING"));
        assert!(instances[1].body_string().contains("FINISHED"));
        let streams = ts::requests_to(&recorded, "/v1.0/card/streaming");
        assert_eq!(streams.len(), 2);
        assert!(streams[0].body_string().contains("\"isFinalize\":false"));
        assert!(streams[0].body_string().contains("\"content\":\"hello\""));
        assert!(streams[1].body_string().contains("\"isFinalize\":true"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stream_ai_card_finish_directly_on_fresh_card() {
        ts::ensure_crypto_provider();
        let routes = vec![
            HttpRoute::json("/v1.0/card/instances", 200, "{}"),
            HttpRoute::json("/v1.0/card/streaming", 200, "{}"),
        ];
        let (base, _) = ts::spawn_http(routes).await;
        let mut card = AiCard {
            card_instance_id: "card_y".into(),
            access_token: "tok".into(),
            inputing_started: false,
        };
        stream_ai_card(&base, &mut card, "final", true)
            .await
            .unwrap();
        assert!(card.inputing_started);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stream_ai_card_send_failure() {
        ts::ensure_crypto_provider();
        let mut card = AiCard {
            card_instance_id: "card_z".into(),
            access_token: "tok".into(),
            inputing_started: false,
        };
        assert!(stream_ai_card("http://127.0.0.1:1", &mut card, "x", false)
            .await
            .is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn close_ai_card_sends_finish_with_error() {
        ts::ensure_crypto_provider();
        let routes = vec![HttpRoute::json("/v1.0/card/instances", 200, "{}")];
        let (base, recorded) = ts::spawn_http(routes).await;
        let card = AiCard {
            card_instance_id: "card_c".into(),
            access_token: "tok".into(),
            inputing_started: true,
        };
        close_ai_card(&base, &card, "kaboom").await;
        let instances = ts::requests_to(&recorded, "/v1.0/card/instances");
        assert_eq!(instances.len(), 1);
        assert!(instances[0].body_string().contains("FINISHED"));
        assert!(instances[0].body_string().contains("kaboom"));

        // Send failure only warns (no panic).
        close_ai_card("http://127.0.0.1:1", &card, "ignored").await;
    }
}
