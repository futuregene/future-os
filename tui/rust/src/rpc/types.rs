//! RPC types for FutureAgent communication — 1:1 port of `tui/src/rpc/types.ts`.
//!
//! P1 scope: only the `ModelInfo` surface used by the components layer
//! (`ScopedModelsSelector`). The full RPC command/response/event model is a
//! P2 concern (app layer + grpc client).

/// Port of `ModelInfo` from `rpc/types.ts` (from get_available_models).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    pub id: String,
    /// Display name (was "name").
    pub label: String,
    pub provider: String,
    /// Was "image".
    pub supports_images: bool,
    /// Default thinking level for this model.
    pub thinking_level: String,
    pub context_window: u64,
    pub is_default: bool,
}

impl ModelInfo {
    /// `provider/id` when the provider is non-empty, else bare `id`
    /// (mirrors `item.provider ? \`${item.provider}/${item.id}\` : item.id` —
    /// an empty provider string is falsy in JS).
    pub fn full_id(&self) -> String {
        if self.provider.is_empty() {
            self.id.clone()
        } else {
            format!("{}/{}", self.provider, self.id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(provider: &str, id: &str) -> ModelInfo {
        ModelInfo {
            id: id.into(),
            label: id.into(),
            provider: provider.into(),
            supports_images: false,
            thinking_level: "off".into(),
            context_window: 128_000,
            is_default: false,
        }
    }

    #[test]
    fn full_id_prepends_provider() {
        assert_eq!(model("openai", "gpt-4o").full_id(), "openai/gpt-4o");
    }

    #[test]
    fn full_id_with_empty_provider_is_bare_id() {
        assert_eq!(model("", "local-model").full_id(), "local-model");
    }
}
