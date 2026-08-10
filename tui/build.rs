// build.rs — version injection for the Rust TUI.
//
// Version injection mirrors scripts/version.mjs (the single source of truth
// for FutureOS build versioning) exactly like cli/rust does, so
// `future-tui --version` prints the same string the TypeScript TUI prints for
// the same checkout/CI environment.
//
// Proto code generation is NOT owned here: future-rpc is the single proto
// codegen owner (PR #112) and the TUI consumes `future_rpc::proto` as a
// dependency (see Cargo.toml). No protoc needed for any build.

use std::process::Command;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    emit_build_version();
    Ok(())
}

/// Inject the display version (scripts/version.mjs `resolveVersion`, ported
/// 1:1) as a compile-time env so code can read it via `env!("FUTURE_TUI_VERSION")`.
fn emit_build_version() {
    let version = resolve_version();
    println!("cargo:rustc-env=FUTURE_TUI_VERSION={version}");
    // `cfg(coverage)` is set by cargo-llvm-cov; used for test-only
    // substitutions of process-death paths (see terminal_posix.rs).
    println!("cargo:rustc-check-cfg=cfg(coverage)");
    println!("cargo:rerun-if-env-changed=FUTURE_VERSION");
    println!("cargo:rerun-if-env-changed=GITHUB_REF");
    println!("cargo:rerun-if-env-changed=GITHUB_ACTIONS");
    println!("cargo:rerun-if-env-changed=CI");
}

/// Port of scripts/version.mjs `resolveVersion()`:
///   - FUTURE_VERSION env override wins (trimmed, empty treated as unset)
///   - release tag `refs/tags/vX.Y.Z` → `X.Y.Z`
///   - dev build `0.0.<commit-count>-<hash>`; `+local` (and `.dirty`) appended
///     for local builds (no GITHUB_ACTIONS/CI), matching the TS TUI exactly.
fn resolve_version() -> String {
    if let Ok(v) = std::env::var("FUTURE_VERSION") {
        let v = v.trim();
        if !v.is_empty() {
            return v.to_string();
        }
    }
    if let Ok(reference) = std::env::var("GITHUB_REF") {
        // refs/tags/vX.Y.Z → X.Y.Z
        if let Some(stripped) = reference.strip_prefix("refs/tags/v") {
            if is_semver_core(stripped) {
                return stripped.to_string();
            }
        }
    }
    let count = git(&["rev-list", "--count", "HEAD"])
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "0".to_string());
    let hash = git(&["rev-parse", "--short", "HEAD"])
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    let ci = std::env::var("GITHUB_ACTIONS").is_ok() || std::env::var("CI").is_ok();
    if ci {
        return format!("0.0.{count}-{hash}");
    }
    let dirty = git_status_porcelain_nonempty();
    format!(
        "0.0.{count}-{hash}+local{}",
        if dirty { ".dirty" } else { "" }
    )
}

fn is_semver_core(s: &str) -> bool {
    let mut parts = s.split('.');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(a), Some(b), Some(c), None)
            if !a.is_empty() && !b.is_empty() && !c.is_empty()
                && a.chars().all(|ch| ch.is_ascii_digit())
                && b.chars().all(|ch| ch.is_ascii_digit())
                && c.chars().all(|ch| ch.is_ascii_digit())
    )
}

fn git(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn git_status_porcelain_nonempty() -> bool {
    Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .is_some_and(|o| !o.stdout.is_empty())
}
