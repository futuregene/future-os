//! Exercise the real binary: discovery must work with no data directory and
//! must not create one. Never execute fabricated platform executables.
#[test]
fn structured_version_discovery_is_read_only() {
    let root = tempfile::tempdir().unwrap();
    let future_home = root.path().join("absent-data-root");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_future"))
        .args(["version", "--json"])
        .env("FUTURE_HOME", &future_home)
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let reply: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let capabilities: future_rpc::cli_capabilities::CliCapabilities =
        serde_json::from_value(reply["desktopCapabilities"].clone()).unwrap();
    capabilities.validate(Some("skills.init")).unwrap();
    assert!(!future_home.exists());
    assert!(reply["version"].as_str().is_some());
}
