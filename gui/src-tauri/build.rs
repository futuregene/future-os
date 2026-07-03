fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Inject the build version (see scripts/version.mjs) as a compile-time env
    // so code can read it via env!("FUTURE_VERSION"). Falls back to a local dev
    // marker for a bare `cargo build` where FUTURE_VERSION isn't set.
    let base = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
    // Treat an empty FUTURE_VERSION as unset (matches scripts/version.mjs), so a
    // failed `$(shell …)` in the Makefile can't inject an empty version string.
    let version = std::env::var("FUTURE_VERSION")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| format!("{base}-dev.local"));
    println!("cargo:rustc-env=FUTURE_VERSION={version}");
    println!("cargo:rerun-if-env-changed=FUTURE_VERSION");

    tauri_build::build();

    let proto_path = std::path::Path::new("../../proto/future.proto");
    println!("cargo:rerun-if-changed={}", proto_path.display());
    tonic_build::configure()
        .build_server(false)
        .compile_protos(&[proto_path], &[proto_path.parent().unwrap()])?;

    Ok(())
}
