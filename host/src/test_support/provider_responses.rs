//! Original provider catalog/snapshot responses shared by both real gRPC fixtures.
use serde_json::json;

pub fn models() -> serde_json::Value {
    json!({
        "models": [],
        "defaultModel": "",
        "isScoped": false,
        "builtinProviders": {
            "deepseek": {
                "name": "DeepSeek",
                "modelCount": 3,
                "baseUrl": "https://api.deepseek.com/v1",
            },
            "azure-openai-responses": {
                "name": "Azure OpenAI Responses",
                "modelCount": 1,
                "baseUrl": "https://YOUR_RESOURCE.openai.azure.com/openai",
            },
            // Filtered out of the GUI catalog: FutureGene is presented
            // separately, and an empty id is invalid.
            "future": {
                "name": "Future",
                "modelCount": 9,
                "baseUrl": "https://future-os.cn/api/v1",
            },
            "": { "name": "", "modelCount": 0, "baseUrl": "" },
            // A pre-display-name agent entry: the GUI falls back to the id.
            "noname": {
                "name": "",
                "modelCount": 1,
                "baseUrl": "https://noname.example.com/v1",
            },
        },
    })
}

pub fn providers() -> serde_json::Value {
    json!({
        "builtin": [
            {
                "id": "future",
                "name": "Future",
                "baseUrl": "https://future-os.cn/api/v1",
                "hasApiKey": false,
                "modelCount": 9,
                "requiresBaseUrl": false
            },
            {
                "id": "deepseek",
                "name": "DeepSeek",
                "baseUrl": "https://api.deepseek.com/v1",
                "hasApiKey": false,
                "modelCount": 3,
                "requiresBaseUrl": false
            },
            {
                "id": "azure-openai-responses",
                "name": "Azure OpenAI Responses",
                "baseUrl": "https://YOUR_RESOURCE.openai.azure.com/openai",
                "hasApiKey": false,
                "modelCount": 1,
                "requiresBaseUrl": true
            }
        ],
        "custom": []
    })
}

/// Provider tests use the canonical RPC fixture, with the same response profile
/// previously supplied by the Remote fixture. Acquire HOME before this guard,
/// matching canonical TestHome/MockAgentGuard ordering.
#[cfg(test)]
pub(crate) fn mock_provider_agent() -> super::agent::MockAgentGuard {
    let agent = super::agent::mock_agent();
    agent.default_data("list_models", models());
    agent.default_data("list_providers", providers());
    agent
}
