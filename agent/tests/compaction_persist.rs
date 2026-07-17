use std::io::Write;

#[test]
fn compaction_discards_history_and_records_marker() {
    let temp = tempfile::tempdir().unwrap();
    let jsonl_path = temp.path().join("test-session.jsonl");

    // ── 1. Build simulated long conversation ───────────────────────────────
    let padding = "x".repeat(10_000);
    let mut messages: Vec<future_agent::types::Message> = vec![
        future_agent::types::Message {
            role: "system".into(),
            content: Some(serde_json::json!([{"type":"text","text":"sys prompt"}])),
            ..Default::default()
        },
    ];
    for i in 0..40 {
        messages.push(future_agent::types::Message {
            role: "user".into(),
            content: Some(serde_json::json!([{"type":"text","text":format!("turn {i}: {padding}")}])),
            ..Default::default()
        });
        messages.push(future_agent::types::Message {
            role: "assistant".into(),
            content: Some(serde_json::json!([{"type":"text","text":format!("response {i}: {padding}")}])),
            ..Default::default()
        });
    }
    let estimated = future_agent::compaction::estimate_context_tokens(&messages);
    eprintln!("Built {} messages, est {} tokens", messages.len(), estimated);

    // ── 2. Compact with small window to force it ───────────────────────────
    let context_window = 50_000i32;
    let reserve = ((context_window as f64 * 0.1) as i32).max(16384);
    let keep = ((context_window as f64 * 0.2) as i32).max(reserve);
    let (compacted, result) = future_agent::compaction::compact(
        messages.clone(),
        &future_agent::compaction::CompactOptions {
            reserve_tokens: reserve,
            keep_recent_tokens: keep,
            context_window,
            tokens_before: estimated,
        },
    );
    assert!(
        result.is_some(),
        "Compaction should trigger: est={estimated} tokens, window={context_window}"
    );
    eprintln!(
        "Compacted: {} -> {} messages",
        messages.len(),
        compacted.len()
    );

    // ── 3. Verify history discarded, recent kept ───────────────────────────
    assert!(
        compacted.len() < messages.len(),
        "Compacted ({}) should be fewer than original ({})",
        compacted.len(),
        messages.len()
    );
    assert!(
        !compacted
            .iter()
            .any(|m| m.content.as_ref().map(|c| c.to_string().contains("turn 0")).unwrap_or(false)),
        "Turn 0 should be discarded"
    );
    assert!(
        compacted
            .iter()
            .any(|m| m.content.as_ref().map(|c| c.to_string().contains("turn 39")).unwrap_or(false)),
        "Turn 39 should be kept (most recent)"
    );

    // ── 4. Compaction marker is first user message ─────────────────────────
    let marker_idx = compacted.iter().position(|m| m.role == "user").unwrap();
    let text = compacted[marker_idx]
        .content
        .as_ref()
        .unwrap()
        .as_array()
        .unwrap()[0]
        .get("text")
        .unwrap()
        .as_str()
        .unwrap();
    assert!(
        text.starts_with("[Context compaction:"),
        "Compaction marker not found: {text}"
    );

    // ── 5. Simulate save path: detect marker, replace with compaction entry
    let mut entries: Vec<serde_json::Value> = compacted
        .iter()
        .map(|m| {
            serde_json::json!({
                "id": "x", "type": m.role, "role": m.role,
                "content": m.content,
                "timestamp": chrono::Local::now().to_rfc3339(),
            })
        })
        .collect();

    // Prepend session_info
    entries.insert(0, serde_json::json!({
        "id": "si", "type": "session_info", "role": "system",
        "content": {"session_name": "compaction-test"},
        "timestamp": chrono::Local::now().to_rfc3339(),
    }));

    // Debug: show entries before marker replacement
    eprintln!("Entries before marker replacement ({} total):", entries.len());
    for (i, e) in entries.iter().enumerate() {
        let r = e.get("role").and_then(|v| v.as_str()).unwrap_or("?");
        let c = e.get("content").map(|c| c.to_string()).unwrap_or_default();
        let c = if c.len() > 80 { format!("{}…", &c[..80]) } else { c };
        eprintln!("  {i:3} role={r:12} content={c}");
    }

    // Replace compaction marker with proper entry (same logic as save path)
    if let Some(idx) = entries.iter().position(|e| {
        e.get("role").and_then(|r| r.as_str()) == Some("user")
            && e.get("content")
                .and_then(|c| c.as_array())
                .and_then(|arr| arr.first())
                .and_then(|b| b.get("text"))
                .and_then(|t| t.as_str())
                .is_some_and(|t| t.starts_with("[Context compaction:"))
    }) {
        // Build clean compaction entry (matches save path format)
        let summary = entries[idx]
            .get("content")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|b| b.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("");
        entries.insert(idx + 1, serde_json::json!({
            "id": "comp-1",
            "type": "compaction",
            "role": "system",
            "content": {"summary": summary},
            "label": "compacted",
            "timestamp": chrono::Local::now().to_rfc3339(),
        }));
        entries.remove(idx);
    }

    // ── 6. Write and read back JSONL ──────────────────────────────────────
    {
        let file = std::fs::File::create(&jsonl_path).unwrap();
        let mut writer = std::io::BufWriter::new(file);
        for e in &entries {
            serde_json::to_writer(&mut writer, e).unwrap();
            writer.write_all(b"\n").unwrap();
        }
        writer.flush().unwrap();
    }

    let content = std::fs::read_to_string(&jsonl_path).unwrap();
    let read_entries: Vec<serde_json::Value> =
        content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();

    // ── 7. Verify JSONL contents ──────────────────────────────────────────
    assert!(
        read_entries
            .iter()
            .any(|e| e.get("type").and_then(|t| t.as_str()) == Some("compaction")),
        "JSONL MUST have 'compaction' entry"
    );
    assert!(
        !read_entries
            .iter()
            .any(|e| e.get("content").map(|c| c.to_string().contains("turn 0")).unwrap_or(false)),
        "Turn 0 MUST be discarded from JSONL"
    );
    assert!(
        read_entries
            .iter()
            .any(|e| e.get("content").map(|c| c.to_string().contains("turn 39")).unwrap_or(false)),
        "Turn 39 MUST be kept in JSONL"
    );

    eprintln!("\n=== ALL CHECKS PASSED ===");
    eprintln!("JSONL entries ({} total):", read_entries.len());
    for (i, e) in read_entries.iter().enumerate() {
        let t = e.get("type").and_then(|v| v.as_str()).unwrap_or("?");
        let preview = e
            .get("content")
            .map(|c| {
                let s = c.to_string();
                if s.len() > 100 { format!("{}…", &s[..100]) } else { s }
            })
            .unwrap_or_default();
        eprintln!("  {i:3} {t:15} {preview}");
    }
}
