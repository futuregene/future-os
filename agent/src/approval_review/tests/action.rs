use super::*;

#[test]
fn streaming_digest_preserves_the_canonical_binding_format() {
    let action = prepare_review_action(
        "shell",
        "call",
        &json!({"env":{"Z":"last","A":"first"},"command":"pwd"}),
        &json!({}),
        &json!({"execution":"outside_sandbox_once"}),
        "/project",
    );
    assert_eq!(
        action.action_digest,
        "sha256:f70fcb5166c8341bf3ffa9a92a584598b3f968cae80a31026e2ab885b6439c8b"
    );
}

#[test]
fn redaction_preserves_shell_operators_quotes_and_expansions() {
    use crate::approval_review::redaction::redact;
    for suffix in [
        ";printf${IFS}SECOND_COMMAND",
        "&&printf SECOND_COMMAND",
        "|printf SECOND_COMMAND",
        "$(printf SECOND_COMMAND)",
        "`printf SECOND_COMMAND`",
        "\\;printf SECOND_COMMAND",
        "';printf SECOND_COMMAND",
        "\";printf SECOND_COMMAND",
    ] {
        let input = format!("curl https://example.test/?q=private{suffix}");
        assert_eq!(
            redact(&input),
            format!("curl https://example.test/?[REDACTED]{suffix}")
        );
    }
    for input in [
        "AWS_SECRET_ACCESS_KEY=fixture-value;printf next",
        "AWS_ACCESS_KEY_ID=fixture-value;printf next",
        "{\"api_key\":\"fixture-value\"}",
        "postgres://user:fixture-value@db.test/table",
        "Authorization: Bearer fixture-value",
        "-----BEGIN PRIVATE KEY-----\nfixture-value\n-----END PRIVATE KEY-----",
    ] {
        assert!(!redact(input).contains("fixture-value"), "{input}");
    }
}

#[test]
fn denial_buckets_bind_confirmed_targets_or_executable_parameters() {
    let boundary = json!({"execution":"outside_sandbox_once"});
    let bucket = |id: &str, args: Value, shape: Value| {
        prepare_review_action("shell", id, &args, &shape, &boundary, "/project").denial_bucket_key
    };
    let shape = json!({"category":"sandbox_escalation"});
    let a = bucket(
        "a",
        json!({"command":"printf first","justification":"first reason"}),
        shape.clone(),
    );
    assert_eq!(
        a,
        bucket(
            "b",
            json!({"command":"printf first","justification":"new reason","failure_summary":"different error"}),
            shape.clone()
        )
    );
    assert_ne!(
        a,
        bucket("c", json!({"command":"printf second"}), shape.clone())
    );
    assert_ne!(
        bucket(
            "a",
            json!({"command":"write"}),
            json!({"paths":["/outside/a.txt"]})
        ),
        bucket(
            "b",
            json!({"command":"write"}),
            json!({"paths":["/outside/b.txt"]})
        )
    );
    let diagnostic_shape = json!({"category":"sandbox_escalation","blocked_paths":["~/outside/a"],"diagnostic_paths":["/outside/a"]});
    let action = prepare_review_action(
        "shell",
        "a",
        &json!({"command":"printf first"}),
        &diagnostic_shape,
        &boundary,
        "/project",
    );
    assert_eq!(action.facts["targets"], json!([]));
    assert_eq!(action.facts["diagnostic_paths"], json!(["/outside/a"]));
    assert_eq!(a, action.denial_bucket_key);
}
#[test]
fn digest_covers_exact_execution_but_audit_redacts_secrets() {
    let boundary = json!({"cwd":"/project","execution":"outside_sandbox_once"});
    let raw = json!({"command":"curl https://host/path?token=hidden -H 'Authorization: Bearer hidden'","env":{"API_KEY":"never-send"}});
    let (facts, digest, _) = action(
        "shell",
        "call1",
        &raw,
        &json!({"category":"process"}),
        &boundary,
        "/project",
    );
    assert!(!facts.to_string().contains("hidden"));
    assert!(!facts.to_string().contains("never-send"));
    let mut other = raw.clone();
    other["env"]["API_KEY"] = json!("changed");
    assert_ne!(
        digest,
        action("shell", "call1", &other, &json!({}), &boundary, "/project").1
    );
    let reversed = json!({"env":{"API_KEY":"never-send"},"command":raw["command"]});
    assert_eq!(
        digest,
        action(
            "shell",
            "call1",
            &reversed,
            &json!({}),
            &boundary,
            "/project"
        )
        .1
    );
}
