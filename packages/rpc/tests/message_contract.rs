//! Contract tests for the canonical history projection (`future_rpc::message`).
//!
//! These functions are the boundary the phone and the desktop app read a session
//! through, and they had **no tests at all** before this file: every assertion here
//! is about a rename, a default, or a state mapping that a client depends on, so a
//! silent regression would show up as a wrong number or a blank field on a device
//! rather than as a failing build.
//!
//! Nothing here reaches the network, a filesystem or the agent: the whole module is
//! pure `Value` → `Value`, which is why the boundary cases can be exact.

use future_rpc::message::{
    checkpoint_metadata, run_terminal, session_metadata, ContextMessage, MessageBlock,
    MessageUsage, SessionEntryPayload,
};
use serde_json::{json, Value};

/// `from_model` is the single conversion point from the internal model/import
/// vocabulary to the canonical block. Each arm maps a different set of fields, so
/// each is asserted separately - a wrong arm shows up as a missing field, not an error.
#[test]
fn model_blocks_convert_each_kind_and_keep_unknown_ones_opaque() {
    // `text` and `reasoning` share an arm: both carry payload in `text`.
    for kind in ["text", "reasoning"] {
        let block = MessageBlock::from_model(&json!({ "type": kind, "text": "hello" }));
        assert_eq!(block.kind, kind);
        assert_eq!(block.text.as_deref(), Some("hello"));
        assert!(block.data.is_none(), "{kind} is a known kind, not opaque");
    }

    // A text block with no `text` is still that kind, just empty - the client
    // renders an empty bubble rather than an opaque payload.
    let empty = MessageBlock::from_model(&json!({ "type": "text" }));
    assert_eq!(empty.kind, "text");
    assert!(empty.text.is_none());

    let call = MessageBlock::from_model(&json!({
        "type": "tool_call",
        "id": "call-1",
        "name": "shell",
        "args": { "cmd": "ls" },
    }));
    assert_eq!(call.kind, "tool_call");
    assert_eq!(call.tool_call_id.as_deref(), Some("call-1"));
    assert_eq!(call.name.as_deref(), Some("shell"));
    assert_eq!(
        call.arguments,
        Some(json!({ "cmd": "ls" })),
        "args are kept whole"
    );

    // `args` absent: the arguments field must be None, not `null`, so
    // `skip_serializing_if` still omits it.
    let call_no_args = MessageBlock::from_model(&json!({ "type": "tool_call", "id": "c2" }));
    assert!(call_no_args.arguments.is_none());
    assert!(
        !serde_json::to_value(&call_no_args)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("arguments"),
        "an absent argument payload must be omitted, not written as null"
    );

    let result = MessageBlock::from_model(&json!({
        "type": "tool_result",
        "tool_call_id": "call-1",
        "content": "ok",
        "is_error": true,
    }));
    assert_eq!(result.kind, "tool_result");
    assert_eq!(result.tool_call_id.as_deref(), Some("call-1"));
    assert_eq!(result.text.as_deref(), Some("ok"));
    assert_eq!(result.is_error, Some(true));

    // `is_error` defaults to false rather than to absent: a tool result that does
    // not say is a success, and a client must not render it as an error.
    let ok_result = MessageBlock::from_model(&json!({
        "type": "tool_result",
        "tool_call_id": "c",
        "content": "fine",
    }));
    assert_eq!(ok_result.is_error, Some(false));

    // `image_url` is RENAMED to the canonical `image` kind.
    let image = MessageBlock::from_model(&json!({
        "type": "image_url",
        "image_url": { "url": "https://example.test/a.png" },
    }));
    assert_eq!(
        image.kind, "image",
        "the wire kind is `image`, not `image_url`"
    );
    assert_eq!(
        image.image_url.as_deref(),
        Some("https://example.test/a.png")
    );

    // ...but an image with no resolvable url falls back to opaque `data`, so the
    // payload is never dropped.
    let broken_image = MessageBlock::from_model(&json!({
        "type": "image_url",
        "image_url": {},
    }));
    assert_eq!(broken_image.kind, "image");
    assert!(broken_image.image_url.is_none());
    assert_eq!(
        broken_image.data,
        Some(json!({ "type": "image_url", "image_url": {} })),
        "an unreadable image must be preserved opaquely, not lost"
    );

    // Unknown provider blocks stay lossless - this is the forward-compat promise.
    let vendor = MessageBlock::from_model(&json!({ "type": "vendor_thing", "payload": [1, 2] }));
    assert_eq!(vendor.kind, "vendor_thing");
    assert_eq!(
        vendor.data,
        Some(json!({ "type": "vendor_thing", "payload": [1, 2] }))
    );

    // A block with no `type` at all is `opaque`, not an empty kind.
    let anonymous = MessageBlock::from_model(&json!({ "payload": true }));
    assert_eq!(anonymous.kind, "opaque");
    assert_eq!(anonymous.data, Some(json!({ "payload": true })));

    // `provider_metadata` rides along on EVERY kind (it is set after the match).
    for raw in [
        json!({ "type": "text", "text": "t", "provider_metadata": { "cache": "hit" } }),
        json!({ "type": "tool_call", "id": "i", "provider_metadata": { "cache": "hit" } }),
        json!({ "type": "vendor", "provider_metadata": { "cache": "hit" } }),
    ] {
        let block = MessageBlock::from_model(&raw);
        assert_eq!(
            block.provider_metadata,
            Some(json!({ "cache": "hit" })),
            "provider metadata must survive on every kind: {raw}"
        );
    }
}

/// `session_metadata` renames the journalled vocabulary to the public one and folds
/// the token counters into one `usage` object. The renames are what the phone reads,
/// so a dropped entry shows up as a blank settings row.
#[test]
fn session_metadata_renames_every_field_and_folds_usage() {
    let input = json!({
        "tokens_in": 11,
        "tokens_out": 22,
        "tokens_cache_r": 33,
        "tokens_cache_w": 44,
        "total_cost": 1.25,
        "session_name": "my session",
        "parent_session_id": "parent-1",
        "thinking_level": "high",
        "last_prompt_tokens": 7,
        "auto_compaction": true,
        "created_by": "user",
        "creator_id": "u-1",
        "source_meta": { "origin": "cli" },
        "unrelated": "kept",
    });
    let out = session_metadata(&input);

    // Every journalled key is consumed, not duplicated beside its public name.
    for old in [
        "tokens_in",
        "tokens_out",
        "tokens_cache_r",
        "tokens_cache_w",
        "total_cost",
        "session_name",
        "parent_session_id",
        "thinking_level",
        "last_prompt_tokens",
        "auto_compaction",
        "created_by",
        "creator_id",
        "source_meta",
    ] {
        assert!(
            out.get(old).is_none(),
            "`{old}` must be renamed, not left: {out}"
        );
    }

    assert_eq!(out["sessionName"], "my session");
    assert_eq!(out["parentSessionId"], "parent-1");
    assert_eq!(out["thinkingLevel"], "high");
    assert_eq!(out["lastPromptTokens"], 7);
    assert_eq!(out["autoCompactionEnabled"], true);
    assert_eq!(out["createdBy"], "user");
    assert_eq!(out["creatorId"], "u-1");
    assert_eq!(out["sourceMeta"], json!({ "origin": "cli" }));

    // Unknown keys are preserved: this projection is additive, not a whitelist.
    assert_eq!(out["unrelated"], "kept");

    let usage = &out["usage"];
    assert_eq!(usage["inputTokens"], 11);
    assert_eq!(usage["outputTokens"], 22);
    assert_eq!(usage["cacheReadTokens"], 33);
    assert_eq!(usage["cacheWriteTokens"], 44);
    assert_eq!(usage["costCny"], 1.25);
    // The per-category split is derived live from the resolved model's prices and
    // is deliberately NOT journalled; a replay must not freeze a past rate.
    for field in [
        "costInputCny",
        "costOutputCny",
        "costCacheReadCny",
        "costCacheWriteCny",
    ] {
        assert_eq!(
            usage[field], 0.0,
            "`{field}` must not be journalled: {usage}"
        );
    }

    // The empty metadata object a fresh session has: zeros, never nulls - a client
    // that shows `0 tok` is right, one that shows `null` is broken.
    let blank = session_metadata(&json!({}));
    assert_eq!(blank["usage"]["inputTokens"], 0);
    assert_eq!(blank["usage"]["outputTokens"], 0);
    assert_eq!(blank["usage"]["costCny"], 0.0);

    // A wrongly-typed counter falls back to 0 rather than leaking a float/string
    // into a field the client formats as an integer.
    let wrong_type = session_metadata(&json!({ "tokens_in": "many", "total_cost": "1.25" }));
    assert_eq!(wrong_type["usage"]["inputTokens"], 0);
    assert_eq!(wrong_type["usage"]["costCny"], 0.0);

    // A non-object metadata is returned untouched instead of panicking - this is
    // the defensive arm for a malformed journal line.
    for passthrough in [json!(null), json!("text"), json!([1, 2]), json!(7)] {
        assert_eq!(
            session_metadata(&passthrough),
            passthrough,
            "a non-object metadata must pass through unchanged"
        );
    }
}

/// `checkpoint_metadata` renames the compaction checkpoint vocabulary. Its arm is
/// distinct from the session one (different keys, and it tolerates a non-object
/// without an early return), so it needs its own assertions.
#[test]
fn checkpoint_metadata_renames_compaction_fields() {
    let input = json!({
        "schema_version": 2,
        "checkpoint_id": "ck-1",
        "cutoff_entry_id": "e-9",
        "tokens_before": 100,
        "tokens_after": 40,
        "algorithm_version": 3,
        "covered_entry_ids": ["e-1", "e-2"],
        "covered_from_entry_id": "e-1",
        "model_state": { "summary": "s" },
        "extra": true,
    });
    let out = checkpoint_metadata(&input);
    assert_eq!(out["schemaVersion"], 2);
    assert_eq!(out["checkpointId"], "ck-1");
    assert_eq!(out["cutoffEntryId"], "e-9");
    assert_eq!(out["tokensBefore"], 100);
    assert_eq!(out["tokensAfter"], 40);
    assert_eq!(out["algorithmVersion"], 3);
    assert_eq!(out["coveredEntryIds"], json!(["e-1", "e-2"]));
    assert_eq!(out["coveredFromEntryId"], "e-1");
    assert_eq!(out["modelState"], json!({ "summary": "s" }));
    assert_eq!(out["extra"], true, "unknown checkpoint keys are preserved");
    for old in [
        "schema_version",
        "checkpoint_id",
        "cutoff_entry_id",
        "tokens_before",
        "tokens_after",
        "algorithm_version",
        "covered_entry_ids",
        "covered_from_entry_id",
        "model_state",
    ] {
        assert!(out.get(old).is_none(), "`{old}` must be renamed: {out}");
    }

    // The non-object arm returns the value as-is (the `if let` is not an early return).
    assert_eq!(checkpoint_metadata(&json!("plain")), json!("plain"));
}

/// `run_terminal` maps the journalled run state onto the three values a client
/// renders, and pulls the usage counters out of the run record.
#[test]
fn run_terminal_maps_states_and_extracts_usage() {
    let cases = [
        ("error", "failed"),
        ("incomplete", "interrupted"),
        ("interrupted_by_restart", "interrupted"),
        // A state the client does not know is passed through, so a new state name
        // reaches the UI instead of being flattened to "unknown".
        ("completed", "completed"),
        ("cancelled", "cancelled"),
    ];
    for (state, want) in cases {
        let out = run_terminal(&json!({
            "run_id": "run-1",
            "state": state,
            "run_duration_ms": 1500,
            "input_tokens": 5,
            "run_tokens": 6,
            "cache_read_tokens": 7,
            "cache_write_tokens": 8,
        }));
        assert_eq!(
            out["status"], want,
            "state `{state}` must map to `{want}` but got {}",
            out["status"]
        );
        assert_eq!(out["runId"], "run-1");
        assert_eq!(out["durationMs"], 1500);
        assert_eq!(out["usage"]["inputTokens"], 5);
        assert_eq!(
            out["usage"]["outputTokens"], 6,
            "output comes from `run_tokens`"
        );
        assert_eq!(out["usage"]["cacheReadTokens"], 7);
        assert_eq!(out["usage"]["cacheWriteTokens"], 8);
    }

    // No state at all is `unknown`, not an empty string a client would render as blank.
    let bare = run_terminal(&json!({}));
    assert_eq!(bare["status"], "unknown");
    assert_eq!(bare["runId"], Value::Null);
    assert_eq!(bare["durationMs"], Value::Null);
    assert_eq!(bare["error"], Value::Null);
    assert_eq!(
        bare["usage"]["inputTokens"],
        Value::Null,
        "an absent counter stays null"
    );

    // An EMPTY error string is null, not "": a client that tests truthiness would
    // otherwise render an empty error banner for a successful run.
    assert_eq!(run_terminal(&json!({ "error": "" }))["error"], Value::Null);
    assert_eq!(run_terminal(&json!({ "error": "boom" }))["error"], "boom");
    // A non-string error is null too (the journal can hold a structured error).
    assert_eq!(
        run_terminal(&json!({ "error": { "kind": "io" } }))["error"],
        Value::Null
    );
}

/// The optional fields of a projected entry must be OMITTED when unset. The doc
/// comment states the reason (absent optionals are 2.4-3.0% of a real history page
/// of `null`s), and the promise is that a consumer's `entry.usage?.x` cannot tell the
/// difference - so both directions are pinned here.
#[test]
fn entry_payload_omits_unset_optionals_but_still_decodes_explicit_nulls() {
    let entry = SessionEntryPayload {
        id: "e-1".into(),
        kind: "message".into(),
        role: "assistant".into(),
        created_at_ms: 1_700_000_000_000,
        run_id: None,
        blocks: vec![],
        metadata: None,
        usage: None,
        run: None,
        session: None,
        checkpoint: None,
    };
    let serialized = serde_json::to_value(&entry).unwrap();
    for field in ["runId", "metadata", "usage", "run", "session", "checkpoint"] {
        assert!(
            !serialized.as_object().unwrap().contains_key(field),
            "an unset `{field}` must be omitted, not written as null: {serialized}"
        );
    }

    // A peer (or a cached page) that still spells them out as `null` decodes the
    // same way, so the wire change is backward compatible.
    let with_nulls = json!({
        "id": "e-1",
        "kind": "message",
        "role": "assistant",
        "createdAtMs": 1_700_000_000_000i64,
        "runId": Value::Null,
        "blocks": [],
        "metadata": Value::Null,
        "usage": Value::Null,
        "run": Value::Null,
        "session": Value::Null,
        "checkpoint": Value::Null,
    });
    let decoded: SessionEntryPayload = serde_json::from_value(with_nulls).unwrap();
    assert!(decoded.run_id.is_none());
    assert!(decoded.usage.is_none());
    assert!(decoded.checkpoint.is_none());

    // A present usage round-trips through the same camelCase names the client reads.
    let usage = MessageUsage {
        input_tokens: Some(1),
        output_tokens: Some(2),
        cache_read_tokens: None,
        cache_write_tokens: None,
    };
    let encoded = serde_json::to_value(&usage).unwrap();
    assert_eq!(encoded["inputTokens"], 1);
    assert_eq!(encoded["outputTokens"], 2);
}

/// `MessageBlock` and the projection structs are `deny_unknown_fields`: a peer
/// sending a field this build does not know is a HARD ERROR rather than a silently
/// dropped value. That is the deliberate contract (the payload has no room for an
/// unknown field to hide), so pin it.
#[test]
fn unknown_fields_are_refused_on_the_canonical_structs() {
    let err = serde_json::from_value::<MessageBlock>(json!({
        "kind": "text",
        "text": "hi",
        "surprise": 1,
    }))
    .expect_err("an unknown block field must be refused");
    assert!(
        err.to_string().contains("surprise"),
        "the error must name the offending field: {err}"
    );

    let err = serde_json::from_value::<MessageUsage>(json!({
        "inputTokens": 1,
        "unexpected": 2,
    }))
    .expect_err("an unknown usage field must be refused");
    assert!(err.to_string().contains("unexpected"), "{err}");
}

/// `ContextMessage` is the shape sent back to the agent on the next turn, and the
/// same block conversion feeds it - so a tool call must arrive with both halves
/// (id + arguments) intact rather than as an opaque blob.
#[test]
fn context_message_keeps_both_halves_of_a_tool_exchange() {
    let ctx = ContextMessage {
        role: "assistant".into(),
        run_id: Some("run-1".into()),
        blocks: vec![
            MessageBlock::from_model(&json!({
                "type": "tool_call",
                "id": "call-1",
                "name": "read_file",
                "args": { "path": "a.txt" },
            })),
            MessageBlock::from_model(&json!({
                "type": "tool_result",
                "tool_call_id": "call-1",
                "content": "contents",
                "is_error": false,
            })),
        ],
        metadata: Some(json!({ "thinking_level": "high" })),
    };
    let value = serde_json::to_value(&ctx).unwrap();
    assert_eq!(value["role"], "assistant");
    assert_eq!(value["runId"], "run-1");
    let blocks = value["blocks"].as_array().unwrap();
    assert_eq!(blocks[0]["toolCallId"], "call-1");
    assert_eq!(blocks[0]["arguments"], json!({ "path": "a.txt" }));
    assert_eq!(
        blocks[1]["toolCallId"], "call-1",
        "the result must reference the same call id as the call"
    );
    assert_eq!(blocks[1]["text"], "contents");
}
