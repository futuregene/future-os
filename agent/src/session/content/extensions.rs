//! Lossless extension fields share the same scoped content store as text.
//! JSON pointers retain namespaced provider state and unknown future fields.
use anyhow::{Context, Result};
use serde_json::{json, Value};

pub(super) const INLINE_EXTENSION_BYTES: usize = 4 * 1024;

pub(super) struct Field {
    pub pointer: String,
    pub body: String,
    pub json: bool,
    pub preview: Option<String>,
}

pub(super) fn json_placeholder(value: &Value) -> Value {
    match value {
        Value::Object(_) => json!({}),
        Value::Array(_) => json!([]),
        _ => Value::Null,
    }
}

fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

fn consider(fields: &mut Vec<Field>, pointer: String, value: &Value, required: bool) {
    let (body, json) = match value {
        Value::String(text) => (text.clone(), false),
        value => (value.to_string(), true),
    };
    if body.len() > INLINE_EXTENSION_BYTES || (required && !body.is_empty()) {
        let preview = (pointer == "/content/error").then(|| body.chars().take(512).collect());
        fields.push(Field {
            pointer,
            body,
            json,
            preview,
        });
    }
}

pub(super) fn fields(entry: &Value) -> Vec<Field> {
    let mut fields = Vec::new();
    if let Some(blocks) = entry["content"].as_array() {
        for (ordinal, block) in blocks.iter().enumerate() {
            if let Some(object) = block.as_object() {
                let kind = block["type"].as_str().unwrap_or("");
                for (key, value) in object {
                    // Relational body columns already own these known fields.
                    if key == "type"
                        || (key == "text" && value.is_string())
                        || (kind == "tool_result"
                            && (key == "tool_call_id"
                                || key == "is_error"
                                || (key == "content" && value.is_string())))
                        || (kind == "tool_call" && matches!(key.as_str(), "id" | "name" | "args"))
                    {
                        continue;
                    }
                    consider(
                        &mut fields,
                        format!("/content/{ordinal}/{}", escape(key)),
                        value,
                        false,
                    );
                }
            } else {
                consider(&mut fields, format!("/content/{ordinal}"), block, false);
            }
        }
    } else if let Some(object) = entry["content"].as_object() {
        for (key, value) in object {
            if entry["type"] == "session_info" && super::metadata::inline_control(key) {
                continue;
            }
            // Keep fields used by relational state/indexing directly available.
            if matches!(
                key.as_str(),
                "run_id" | "state" | "model" | "epoch" | "run_sequence"
            ) {
                continue;
            }
            consider(
                &mut fields,
                format!("/content/{}", escape(key)),
                value,
                entry["type"] == "run_terminal" && key == "error",
            );
        }
    }
    if let Some(object) = entry.as_object() {
        for (key, value) in object {
            if matches!(
                key.as_str(),
                "id" | "type" | "role" | "timestamp" | "content"
            ) {
                continue;
            }
            if key == "meta" {
                if let Some(metadata) = value.as_object() {
                    for (key, value) in metadata {
                        if matches!(key.as_str(), "run_id" | "execution_cwd" | "shell_result") {
                            continue;
                        }
                        consider(&mut fields, format!("/meta/{}", escape(key)), value, false);
                    }
                    continue;
                }
            }
            consider(&mut fields, format!("/{}", escape(key)), value, false);
        }
    }
    fields
}

/// Pure SQL adapter: apply only the extension ranges selected by the view.
/// Database access and field authorization remain outside this function.
pub(super) fn restore(document: &str, patches: &str) -> Result<String> {
    let mut document: Value = serde_json::from_str(document)?;
    let patches: Vec<Value> = serde_json::from_str(patches)?;
    for patch in patches {
        let field = patch["field"]
            .as_str()
            .context("content patch pointer missing")?;
        let body = patch["body"]
            .as_str()
            .context("content patch body missing")?;
        let value = if patch["encoding"] == "json" || field == "/meta/shell_result" {
            serde_json::from_str(body)?
        } else {
            json!(body)
        };
        *document
            .pointer_mut(field)
            .context("content patch does not name an existing field")? = value;
    }
    Ok(document.to_string())
}
