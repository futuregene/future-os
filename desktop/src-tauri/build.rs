fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "cargo:rustc-env=FUTURE_TARGET_TRIPLE={}",
        std::env::var("TARGET")?
    );
    // Inject the build version (see scripts/version.mjs) as a compile-time env.
    let base = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
    let version = std::env::var("FUTURE_VERSION")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| local_dev_version(&base));
    println!("cargo:rustc-env=FUTURE_VERSION={version}");
    println!("cargo:rerun-if-env-changed=FUTURE_VERSION");

    #[cfg(feature = "gui")]
    build_gui()?;

    Ok(())
}

#[cfg(feature = "gui")]
fn build_gui() -> Result<(), Box<dyn std::error::Error>> {
    // The CLI is a workspace binary in the same target directory. tauri-build
    // copies externalBin into that directory and could overwrite it with a
    // stale sidecar while Cargo is linking it. Stage the CLI after compilation
    // in scripts/build/build-desktop.mjs; keep the original config for the
    // application and bundler (this override is local to the build script).
    let original_config = std::env::var_os("TAURI_CONFIG");
    let mut config: serde_json::Value = original_config
        .as_ref()
        .map(|value| serde_json::from_str(&value.to_string_lossy()))
        .transpose()?
        .unwrap_or_else(|| serde_json::json!({}));
    config["bundle"]["externalBin"] = serde_json::json!([]);
    std::env::set_var("TAURI_CONFIG", serde_json::to_string(&config)?);
    // Tauri embeds its default common-controls v6 manifest only into the bin
    // (via `resource.lib` + `rustc-link-arg-bins`), which leaves `cargo test`
    // lib-test binaries without one. Disable Tauri's bin-only manifest and
    // embed the same dependency ourselves for every target instead (see
    // `embed_common_controls_manifest`).
    let result = tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest()),
    );
    match original_config {
        Some(value) => std::env::set_var("TAURI_CONFIG", value),
        None => std::env::remove_var("TAURI_CONFIG"),
    }
    result?;
    embed_common_controls_manifest()?;
    reserve_platform_stack()?;

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
#[cfg(feature = "gui")]
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
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    Ok(())
}

/// Reserve the stack every platform but Windows gives a process by default.
///
/// Windows sizes the main thread's stack from the PE header, and `link.exe`
/// defaults to a 1 MB reserve where Linux and macOS reserve 8 MB. Tauri builds
/// an async command's future on the thread that received the IPC message — the
/// main thread — before moving it into the spawned task, so one dispatch needs
/// several copies of that future at once: `tauri`'s resolver holds two, and the
/// move into the task's heap allocation copies it twice more. The desktop's
/// largest command future — `agent_prompt`, 143,736 bytes, because the whole
/// prompt pipeline is inlined into it — therefore asked for a 575,856-byte
/// frame. Together with the dispatcher's own frame that overran 1 MB, and the
/// faulting instruction was the stack probe in the guard page: the process died
/// with `STATUS_STACK_OVERFLOW` (0xc00000fd) the moment a prompt was sent, with
/// no window, no log line and no chance to report anything (`desktop.log` stops
/// at the last write the process managed to flush).
///
/// Reserving what the other platforms reserve leaves the dispatcher ~7x the
/// headroom it needs for the futures this app ships today, and covers the
/// commands whose futures are merely large (several run 100-135 KB) rather than
/// relying on every one of them staying small. The reserve is address space,
/// not memory: the commit stays at 64 KB and the stack grows into the reserve
/// on demand, exactly as it does on Linux and macOS.
#[cfg(feature = "gui")]
fn reserve_platform_stack() -> Result<(), Box<dyn std::error::Error>> {
    if !std::env::var("TARGET")?.contains("windows") {
        return Ok(());
    }

    println!("cargo:rustc-link-arg=/STACK:8388608,65536");
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
