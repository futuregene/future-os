# Design: Filter Image Generation Models from TUI

## Problem

TUI currently displays image generation models (text→image) in the model selector, which confuses users who expect only conversational/text generation models. Image generation models should be filtered out.

## Solution

Parse the `modality` field from Future server API responses to detect output modalities. Filter models whose output contains "image" before sending to TUI.

## Implementation

### 1. Agent: Store Output Modality (agent/src/models/mod.rs)

Add `output: Vec<String>` field to `Model` struct:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub api: String,
    pub base_url: String,
    pub api_key: String,
    pub reasoning: bool,
    pub input: Vec<String>,
    pub output: Vec<String>,  // NEW
    pub context_window: i32,
    pub max_tokens: i32,
    pub cost: Cost,
    pub compat: HashMap<String, serde_json::Value>,
    pub thinking_level_map: HashMap<String, serde_json::Value>,
    pub headers: HashMap<String, String>,
}
```

### 2. Parse Output from Modality String (agent/src/models/mod.rs)

Update `convert_future_model()` to extract output side:

```rust
fn convert_future_model(entry: FutureModelEntry, base_url: &str) -> Model {
    let supported_params = entry.supported_parameters.unwrap_or_default();
    let reasoning = supported_params
        .iter()
        .any(|p| p == "reasoning" || p == "include_reasoning");

    let (input, output) = entry
        .architecture
        .as_ref()
        .and_then(|a| a.modality.as_ref())
        .map(|m| {
            let parts: Vec<&str> = m.split("->").collect();
            let input_str = parts.first().unwrap_or(&"text");
            let output_str = parts.get(1).unwrap_or(&"text");
            
            let input: Vec<String> = input_str
                .split('+')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            
            let output: Vec<String> = output_str
                .split('+')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            
            (input, output)
        })
        .unwrap_or_else(|| (vec!["text".to_string()], vec!["text".to_string()]));

    // ... rest of function
    
    Model {
        id: entry.id,
        name: name.clone(),
        provider: "future".to_string(),
        api: "openai-completions".to_string(),
        base_url: base_url.to_string(),
        api_key: String::new(),
        reasoning,
        input,
        output,  // NEW
        context_window,
        max_tokens: 16384,
        cost: Cost { ... },
        compat: HashMap::new(),
        thinking_level_map: HashMap::new(),
        headers: HashMap::new(),
    }
}
```

### 3. Update builtin_models() Mapping (agent/src/models/mod.rs)

Add output field to generated model conversion:

```rust
pub fn builtin_models() -> Vec<Model> {
    crate::models::generated::init_builtin_models()
        .into_iter()
        .map(|m| Model {
            id: m.id,
            name: m.name,
            provider: m.provider,
            api: m.api,
            base_url: m.base_url,
            api_key: String::new(),
            reasoning: m.reasoning,
            input: m.input,
            output: vec!["text".to_string()],  // NEW - default to text
            context_window: m.context_window,
            max_tokens: m.max_tokens,
            cost: Cost { ... },
            compat: serde_json::from_str(&m.compat_json).unwrap_or_default(),
            thinking_level_map: serde_json::from_str(&m.tlm_json).unwrap_or_default(),
            headers: serde_json::from_str(&m.headers_json).unwrap_or_default(),
        })
        .collect()
}
```

### 4. Filter Image Generation Models (agent/src/rpc/commands.rs)

Update `list_models_response()` to filter:

```rust
fn list_models_response(id: &str) -> String {
    let registry = crate::models::Registry::new();
    let auth = crate::AuthStore::load();

    // Always return all available models.  Scoping / defaults are client-side.
    let mut models: Vec<crate::models::Model> = registry
        .all_models()
        .into_iter()
        .filter(|model| !model.api_key.is_empty() || auth.get(&model.provider).is_some())
        .filter(|model| !model.output.iter().any(|o| o == "image"))  // NEW: Filter image generation
        .collect();

    models.sort_by(|left, right| {
        left.provider
            .cmp(&right.provider)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
    models.dedup_by(|left, right| left.id == right.id && left.provider == right.provider);

    // ... rest of function
}
```

### 5. Update Generated Model Struct (agent/src/models/generated/mod.rs)

No changes needed - output will default to `["text"]` for built-in models.

### 6. Regenerate Model Catalog (Optional)

Run `make generate-models` to ensure all built-in models have correct fields.

## Testing

1. Start agent: `cd future-os && make run-agent`
2. List models from TUI: `cd future-os && make run-tui --list-models`
3. Verify no image generation models appear (e.g., DALL-E, Stable Diffusion, Flux, etc.)
4. Verify text generation models still appear normally

## Backward Compatibility

- `output` field defaults to `["text"]` for models without explicit output modality
- No breaking changes to API or data structures
- Future server API already provides `modality` field with output information

## Edge Cases

1. **Missing modality field**: Defaults to `text→text`, no filtering
2. **Models with no `->` separator**: Treated as input-only, output defaults to `["text"]`
3. **Multiple output modalities** (e.g., `text→text+image`): Filtered if "image" is in output list
4. **User-defined models in models.json**: Will use default `output: ["text"]` unless explicitly set

## Files Modified

1. `future-os/agent/src/models/mod.rs` - Add output field, parse modality, update builtin mapping
2. `future-os/agent/src/rpc/commands.rs` - Filter image generation models in list_models_response
