fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Inject the build version (see scripts/version.mjs) as a compile-time env
    // so code can read it via env!("FUTURE_VERSION"). CI/`make` set it (the tag
    // release or online hash); a bare `tauri dev` / IDE / `cargo build` does not,
    // so we mirror version.mjs's local scheme here from git directly.
    let base = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
    // Treat an empty FUTURE_VERSION as unset (matches scripts/version.mjs), so a
    // failed `$(shell …)` in the Makefile can't inject an empty version string.
    let version = std::env::var("FUTURE_VERSION")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| local_dev_version(&base));
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

/// Local dev version from git, mirroring `scripts/version.mjs`:
/// `<base>-<short-hash>+local` (`+local.dirty` when the tree has uncommitted
/// changes). Falls back to `unknown` outside a git checkout. Only used when
/// FUTURE_VERSION isn't injected (bare `cargo build` / `tauri dev` / IDE).
fn local_dev_version(base: &str) -> String {
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
    };
    let hash = git(&["rev-parse", "--short", "HEAD"])
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    let dirty = git(&["status", "--porcelain"]).is_some_and(|o| !o.stdout.is_empty());
    format!("{base}-{hash}+local{}", if dirty { ".dirty" } else { "" })
}
