//! Redact literal secrets while preserving shell operators and expansions.
//! Text redaction alone is not a permission to transmit arbitrary tool output.
use serde_json::{json, Value};
use std::sync::LazyLock;

pub(super) fn redact_value(value: &Value) -> Value {
    match value {
        Value::String(s) => json!(redact(s)),
        Value::Array(v) => Value::Array(v.iter().map(redact_value).collect()),
        Value::Object(v) => Value::Object(
            v.iter()
                .map(|(k, v)| (k.clone(), redact_value(v)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

pub(super) fn redact(text: &str) -> String {
    static SECRET: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"(?i)(bearer\s+|(?:[a-z0-9]+[_-])*(?:api[_-]?key|access[_-]?key(?:[_-]?id)?|secret(?:[_-]?access[_-]?key)?|secret[_-]?key|token|password|passwd)["']?\s*[=:]\s*["']?)[^\s'";&|<>()$`\\]+"#).unwrap()
    });
    // Never match across command operators, quotes, escapes or expansions.
    // Unquoted '&' is a shell operator; the remaining parameter assignments
    // are independently filtered by SECRET above. Do not mask executable text.
    static QUERY: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"(https?://[^\s?'";&|<>()$`\\]+)\?[^\s'";&|<>()$`\\]+"#).unwrap()
    });
    static PRIVATE_KEY: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----",
        )
        .unwrap()
    });
    static URL_AUTH: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"((?:https?|postgres(?:ql)?|mysql|redis|mongodb)://)[^/\s:@'";&|<>()$`\\]+:[^/\s@'";&|<>()$`\\]+@"#).unwrap()
    });
    let text = PRIVATE_KEY.replace_all(text, "[REDACTED PRIVATE KEY]");
    let text = URL_AUTH.replace_all(&text, "${1}[REDACTED]@");
    QUERY
        .replace_all(
            &SECRET.replace_all(&text, "${1}[REDACTED]"),
            "${1}?[REDACTED]",
        )
        .into_owned()
}
