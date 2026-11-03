//! Persistence projection, applied only after the complete review computation.
//! Diagnostic removal must never change the classifier input or verdict.
use serde_json::Value;

pub fn for_storage(mut value: Value, diagnostics: bool) -> Value {
    if !diagnostics {
        if let Some(object) = value.as_object_mut() {
            for field in [
                "confidence",
                "probabilities",
                "model",
                "provider_request_id",
                "input_context",
                "action",
                "attempt",
                "prompt_version",
                "reason_catalog_version",
                "policy_version",
                "duration_ms",
            ] {
                object.remove(field);
            }
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn production_retains_the_verdict_and_identity_without_diagnostics() {
        let value = json!({"assessment_id":"a","approval_request_id":"q","tool_call_id":"t","reviewer":"model","status":"rejected","effective":{"risk":"high","authorization":"low"},"reported":{"reason_code":"scope_mismatch"},"error_code":null,"confidence":{"risk":0.99},"probabilities":{"risk":{}},"input_context":{"sources":[]},"action":{"command":"pwd"},"policy_version":2});
        assert_eq!(for_storage(value.clone(), true), value);
        let lean = for_storage(value, false);
        assert_eq!(lean["status"], "rejected");
        assert_eq!(lean["reported"]["reason_code"], "scope_mismatch");
        for field in [
            "confidence",
            "probabilities",
            "input_context",
            "action",
            "policy_version",
        ] {
            assert!(lean.get(field).is_none());
        }
        assert!(crate::encode::event_payload("approval_assessment", &lean.to_string()).is_none());
    }
}
