//! Per-connection permission to receive canonical ranges and referenced bodies.
//! No declaration is inherited by a replacement connection or another device.
pub use future_remote_protocol::SEGMENTED_CONTENT_FEATURE;
pub const UPGRADE_REQUIRED: &str = "client_upgrade_required: this conversation uses segmented content; update the remote client before opening or continuing it";

tokio::task_local! { static DECLARED: bool; }

pub async fn in_request<F: std::future::Future>(declared: bool, future: F) -> F::Output {
    DECLARED.scope(declared, future).await
}

/// None is an internal adapter call with no remote connection. Production
/// dispatch always installs Some(true/false), including undeclared old peers.
pub fn declared() -> Option<bool> {
    DECLARED.try_with(|declared| *declared).ok()
}

pub fn canonical_data(data: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(data).is_ok_and(|value| value["content_format"] == 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn declaration_is_scoped_to_the_connection_and_does_not_escape() {
        assert_eq!(declared(), None);
        in_request(false, async {
            assert_eq!(declared(), Some(false));
            in_request(true, async {
                assert_eq!(declared(), Some(true));
            })
            .await;
            assert_eq!(declared(), Some(false));
        })
        .await;
        assert_eq!(declared(), None);
        assert!(canonical_data(r#"{"content_format":2,"text":""}"#));
        assert!(!canonical_data(r#"{"text":"legacy"}"#));
    }
}
