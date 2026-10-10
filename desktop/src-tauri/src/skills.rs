//! Unauthenticated platform guide used by the Skills onboarding UI.
//! Skill installation, discovery, and catalogue operations live in the Agent.

use crate::AppError;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

fn http_client() -> reqwest::Client {
    // `Client::builder().timeout().build()` only fails for an invalid config;
    // the default config here is constant, so a failure is an invariant break.
    crate::install_rustls_provider();
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .expect("default reqwest client config cannot fail to build")
}

/// A zh/en text pair from the platform guide config.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LocalizedText {
    #[serde(default)]
    pub zh: String,
    #[serde(default)]
    pub en: String,
}

/// `links` section of the guide config: global links shared across features.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuideLinks {
    #[serde(default)]
    pub help: String,
}

/// `skills` section of the guide config: the coach prompt and manual link that
/// power the skill-onboarding banner.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuideSkills {
    #[serde(default, alias = "coach_prompt")]
    pub coach_prompt: LocalizedText,
    #[serde(default)]
    pub manual: LocalizedText,
}

/// The platform skill-guide config (`GET /client/v1/guide`).
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillGuide {
    #[serde(default)]
    pub links: GuideLinks,
    #[serde(default)]
    pub skills: GuideSkills,
}

/// The platform skill-guide config (`GET /client/v1/guide`). Unauthenticated,
/// like the catalogue.
pub async fn get_skill_guide() -> Result<SkillGuide, AppError> {
    let url = format!(
        "{}/client/v1/guide",
        crate::future_platform::current_platform_url()
    );
    let response = http_client()
        .get(&url)
        .send()
        .await
        .map_err(|error| AppError::Message(format!("Failed to fetch skill guide: {error}")))?;
    if !response.status().is_success() {
        return Err(AppError::Message(format!(
            "Failed to fetch skill guide (HTTP {})",
            response.status().as_u16()
        )));
    }
    response
        .json()
        .await
        .map_err(|error| AppError::Message(format!("Failed to parse skill guide: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A non-2xx guide response must be reported with its status — the
    /// onboarding banner shows the failure instead of an empty guide.
    #[tokio::test]
    async fn a_non_success_guide_response_reports_its_status() {
        let _home = crate::auth_store::test_support::HomeGuard::new("skills-guide-error");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let count = stream.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..count]).contains("/client/v1/guide"));
            stream
                .write_all(
                    b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });
        crate::auth_store::set_future_base_url(&format!("{url}/api")).unwrap();

        let error = get_skill_guide()
            .await
            .expect_err("a 503 must not be read as an empty guide")
            .to_string();
        assert!(error.contains("HTTP 503"), "{error}");
    }
}
