fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Inject the build version (see scripts/version.mjs) as a compile-time env.
    let base = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
    let version = std::env::var("FUTURE_VERSION")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| local_dev_version(&base));
    println!("cargo:rustc-env=FUTURE_VERSION={version}");
    println!("cargo:rerun-if-env-changed=FUTURE_VERSION");

    ensure_placeholder_sidecars_for_non_release_builds()?;
    tauri_build::build();

    // Agent gRPC bindings come from the future-rpc crate (single codegen
    // owner; typed-RPC milestone) — nothing to generate here.

    Ok(())
}

fn ensure_placeholder_sidecars_for_non_release_builds() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("PROFILE").ok().as_deref() == Some("release") {
        return Ok(());
    }

    let target = std::env::var("TARGET")?;
    let ext = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    let binaries_dir = std::path::Path::new("binaries");
    std::fs::create_dir_all(binaries_dir)?;

    for bin in ["future-agent", "future"] {
        let path = binaries_dir.join(format!("{bin}-{target}{ext}"));
        if !path.exists() {
            std::fs::File::create(path)?;
        }
    }

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
