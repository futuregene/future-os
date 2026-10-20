//! Isolated Windows entry fixtures, without an adjacent built CLI or shared runtime.

/// Return true in the parent after the isolated test passed, false in the child
/// that must execute the test body. No production discovery behavior is changed.
pub(crate) fn reexec(test_name: &str) -> bool {
    const CHILD: &str = "FUTUREOS_ISOLATED_ENTRY_TEST";
    if std::env::var(CHILD).as_deref() == Ok(test_name) {
        return false;
    }
    // Stabilize process-global environment while creating the child. It gets
    // its own HOME fixture and process-lifetime runtime inside the test body.
    let _home = crate::auth_store::test_support::HomeGuard::new("isolated-entry-parent");
    let directory = tempfile::tempdir().expect("isolated test executable directory");
    let executable = std::env::current_exe().expect("test executable");
    let isolated = directory.path().join(executable.file_name().unwrap());
    std::fs::copy(executable, &isolated).expect("copy test executable without adjacent CLI");
    let output = std::process::Command::new(isolated)
        .args(["--exact", test_name, "--nocapture"])
        .env(CHILD, test_name)
        .output()
        .expect("run isolated entry test");
    assert!(
        output.status.success(),
        "isolated {test_name} failed: {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("running 1 test") && stdout.contains("1 passed; 0 failed"),
        "the isolated filter must execute exactly its target: {stdout}"
    );
    true
}
