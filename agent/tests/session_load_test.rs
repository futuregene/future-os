#[test]
fn lenient_timestamp_parses_no_tz() {
    // Without timezone — should now be repaired
    let json = r#"{"id":"t","type":"u","timestamp":"2026-07-17T12:44:27.161975"}"#;
    let entry: future_agent::session::SessionEntry = serde_json::from_str(json).unwrap();
    eprintln!("no-tz: OK ts={}", entry.timestamp);
    
    // With timezone — should still work
    let json2 = r#"{"id":"t","type":"u","timestamp":"2026-07-17T12:44:27.161975+08:00"}"#;
    let entry2: future_agent::session::SessionEntry = serde_json::from_str(json2).unwrap();
    eprintln!("with-tz: OK ts={}", entry2.timestamp);
    
    // Bare date format
    let json3 = r#"{"id":"t","type":"u","timestamp":"2026-07-17"}"#;
    match serde_json::from_str::<future_agent::session::SessionEntry>(json3) {
        Ok(e) => eprintln!("bare-date: OK (repaired to {})", e.timestamp),
        Err(e) => eprintln!("bare-date: ERR {}", e),
    }
}
