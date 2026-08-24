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
    // Tauri embeds its default common-controls v6 manifest only into the bin
    // (via `resource.lib` + `rustc-link-arg-bins`), which leaves `cargo test`
    // lib-test binaries without one. Disable Tauri's bin-only manifest and
    // embed the same dependency ourselves for every target instead (see
    // `embed_common_controls_manifest`).
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest()),
    )?;
    embed_common_controls_manifest()?;

    // Agent gRPC bindings come from the future-rpc crate (single codegen
    // owner; typed-RPC milestone) — nothing to generate here.

    Ok(())
}

/// Embed a `Microsoft.Windows.Common-Controls` v6 manifest into every target.
///
/// `tauri-plugin-dialog` (via `rfd`) links `TaskDialogIndirect`, which only
/// exists in comctl32 v6. Tauri's default manifest is attached only to the bin
/// target, so a `cargo test` lib-test binary loads comctl32 v5 and aborts with
/// STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139) before any test runs. We therefore
/// disable Tauri's bin-only manifest above and provide the same dependency here
/// for all targets (bin, lib, lib-test, integration tests) with no duplication.
fn embed_common_controls_manifest() -> Result<(), Box<dyn std::error::Error>> {
    let target = std::env::var("TARGET")?;
    if !target.contains("windows") {
        return Ok(());
    }

    let out_dir = std::env::var("OUT_DIR")?;
    let manifest = std::path::Path::new(&out_dir).join("common-controls-v6.manifest");
    std::fs::write(
        &manifest,
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity
        type="win32"
        name="Microsoft.Windows.Common-Controls"
        version="6.0.0.0"
        processorArchitecture="*"
        publicKeyToken="6595b64144ccf1df"
        language="*"
      />
    </dependentAssembly>
  </dependency>
</assembly>
"#,
    )?;
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg=/MANIFESTINPUT:{}",
        manifest.display()
    );
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

    // The unified `future` CLI is the only sidecar — `future agent` runs the
    // embedded agent, so a separate future-agent binary is no longer bundled
    // (see agent_supervisor.rs).
    let path = binaries_dir.join(format!("future-{target}{ext}"));
    if !path.exists() {
        std::fs::File::create(path)?;
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
