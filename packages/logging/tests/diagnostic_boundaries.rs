#[path = "support/diagnostic_audit.rs"]
mod audit;

#[test]
fn production_diagnostics_use_tracing_and_keep_protocol_streams_intact() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let report = audit::collect(root);
    assert!(report.modules > 300);
    audit::check(&report).unwrap();
}

#[test]
fn auditor_finds_nested_output_but_ignores_test_code_and_string_literals() {
    let root = tempfile::tempdir().unwrap();
    for path in [
        "agent/src/lib.rs",
        "agent/src/main.rs",
        "host/src/lib.rs",
        "desktop/src-tauri/src/lib.rs",
        "desktop/src-tauri/src/main.rs",
    ] {
        let path = root.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }
    std::fs::write(
        root.path().join("agent/src/lib.rs"),
        r#"
        #[cfg(test)]
        mod tests { fn fixture() { eprintln!("test fixture"); } }
        #[cfg(any(test, feature="test-support"))]
        fn support() { eprintln!("test support"); }
        pub fn worker() {
            tracing::info!("literal eprintln!(\"not a call\")");
            tokio::select! { _ = wait() => { console::eprintln!("diagnostic bypass"); } }
            let _ = writeln!(std::io::stderr().lock(), "direct stderr bypass");
        }
    "#,
    )
    .unwrap();
    let report = audit::collect(root.path());
    assert_eq!(report.rows.len(), 2);
    assert_eq!(report.rows[0]["macro"], "console::eprintln");
    assert_eq!(report.rows[1]["macro"], "writeln");
    assert_eq!(report.rows[0]["tokens"], "\"diagnostic bypass\"");
    assert!(audit::check(&report)
        .unwrap_err()
        .contains("Diagnostic bypasses tracing"));
}
