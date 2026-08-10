//! Object helpers — port of `cli/src/utils/object.ts`.
//!
//! The TS module operates on untyped JS values (`unknown`); the Rust port
//! operates on `serde_json::Value`, the closest equivalent for the JSON
//! payloads the CLI handles. `isNodeError` is omitted: it inspects JS `Error`
//! instances and has no analogue here (the browser subsystem, ported in P3,
//! will need a purpose-built equivalent if required).

use serde_json::{Map, Value};

/// `isRecord(value)` — JS objects (not arrays, not null).
pub fn is_record(value: &Value) -> bool {
    value.is_object()
}

/// `getRecord(value)`.
pub fn get_record(value: &Value) -> Option<&Map<String, Value>> {
    value.as_object()
}

/// `ensureRecordProperty(parent, key)` — returns the existing object at
/// `parent[key]`, or inserts a fresh `{}` and returns it.
pub fn ensure_record_property<'a>(
    parent: &'a mut Map<String, Value>,
    key: &str,
) -> &'a mut Map<String, Value> {
    if !parent.get(key).is_some_and(Value::is_object) {
        parent.insert(key.to_string(), Value::Object(Map::new()));
    }
    // Invariant: the entry exists and is an object (checked/inserted above).
    parent
        .get_mut(key)
        .and_then(Value::as_object_mut)
        .expect("entry checked or inserted above")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn record_checks() {
        assert!(is_record(&json!({"a": 1})));
        assert!(!is_record(&json!([1, 2])));
        assert!(!is_record(&json!(null)));
        assert!(!is_record(&json!("str")));
        assert!(get_record(&json!({})).is_some());
        assert!(get_record(&json!(1)).is_none());
    }

    #[test]
    fn ensure_record_property_behavior() {
        let mut parent = Map::new();
        let child = ensure_record_property(&mut parent, "future");
        child.insert(
            "base_url".to_string(),
            Value::String("https://x/api".into()),
        );
        // Existing object is returned as-is.
        let child2 = ensure_record_property(&mut parent, "future");
        assert_eq!(child2["base_url"], "https://x/api");
        // Non-object value is replaced.
        parent.insert("bad".to_string(), Value::Bool(true));
        let replaced = ensure_record_property(&mut parent, "bad");
        assert!(replaced.is_empty());
    }
}
