//! Measurement instrument for the `usage`/`run` null overhead on a real
//! history page, not a shipped test: it is `#[ignore]`d and needs a dump.
//!
//! The dump is one session's `get_session_entries` reply in the payload shape
//! the phone receives, reconstructed from the live agent's protojson answer the
//! way `scripts/measure-lean-history.py` does it: parse the `…Json` string
//! fields into values, drop the absent ones, and convert protojson's
//! int64-as-string back to JSON numbers (prost hands the client an i64, so the
//! number spelling is the client-side shape).
//!
//! Run it — on a checkout with the omission rule and, for the "before" number,
//! on the base commit with only this file copied in — through the shipping
//! serializer:
//!
//! ```text
//! LEAN_USAGE_RUN_ENTRIES=/abs/path/entries-<session>.json \
//!   cargo test -p future-rpc --test measure_usage_run_nulls -- --ignored --nocapture
//! ```
//!
//! It prints one `LEAN_USAGE_RUN {...}` line: `totalBytes` of the serialized
//! entries, `savedBytes` that removing explicit-null subfields would save
//! (zero once the serializer omits them), and the null-field counts. The
//! before/after pair is exact arithmetic: `before = after + savedBefore`.
use future_rpc::message::SessionEntryPayload;
use serde_json::{json, Value};

#[test]
#[ignore = "measurement: needs LEAN_USAGE_RUN_ENTRIES"]
fn measure_real_usage_run_nulls() {
    let path = std::env::var("LEAN_USAGE_RUN_ENTRIES").expect("LEAN_USAGE_RUN_ENTRIES");
    let raw = std::fs::read_to_string(path).expect("entries readable");
    let entries: Vec<SessionEntryPayload> =
        serde_json::from_str(&raw).expect("entries are the payload shape");

    let mut total_bytes = 0usize;
    let mut saved_bytes = 0usize;
    let mut usage_objects = 0usize;
    let mut run_objects = 0usize;
    let mut null_fields: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();

    for entry in &entries {
        let bytes = serde_json::to_vec(entry).expect("serializes");
        total_bytes += bytes.len();
        // Locate the null subfields on the serialized shape, then remove them
        // and re-serialize: the length difference is exactly the bytes the
        // omission rule saves, with no hand-computed constant.
        let mut value = serde_json::to_value(entry).expect("value");
        let rehydrated = serde_json::to_vec(&value).expect("serializes");
        assert_eq!(
            rehydrated.len(),
            bytes.len(),
            "entry {} must round-trip byte-for-byte before the saving is counted",
            entry.id
        );
        for field in ["usage", "run"] {
            let Some(object) = value.get_mut(field).and_then(Value::as_object_mut) else {
                continue;
            };
            if field == "usage" {
                usage_objects += 1;
            } else {
                run_objects += 1;
            }
            object.retain(|key, value| {
                if value.is_null() {
                    *null_fields.entry(format!("{field}.{key}")).or_default() += 1;
                    false
                } else {
                    true
                }
            });
        }
        saved_bytes += bytes.len() - serde_json::to_vec(&value).expect("serializes").len();
    }

    println!(
        "LEAN_USAGE_RUN {}",
        json!({
            "entries": entries.len(),
            "totalBytes": total_bytes,
            "savedBytes": saved_bytes,
            "usageObjects": usage_objects,
            "runObjects": run_objects,
            "nullFields": null_fields,
        })
    );
}
