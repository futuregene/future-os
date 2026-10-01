// build.rs — Build-time version and build-metadata injection for FutureAgent.
//
// Proto code generation lives in the future-rpc crate (single codegen owner);
// `make generate-proto` regenerates it there. The agent consumes the
// generated types through the future-rpc dependency.
//
// The build-identity emission below mirrors `cli/build.rs`. The two crates can
// be built with different versions (the CLI pins a `0.0.2` dev core, the agent
// uses its own `CARGO_PKG_VERSION`) and either can be rebuilt alone, so each
// embeds its own facts rather than sharing one build script.

use std::path::Path;
use std::process::Command;

fn main() {
    emit_build_info();
}

/// Inject the build version (see `scripts/version.mjs`) plus the structured
/// build identity the RPC reports, all as compile-time env vars. CI/`make` set
/// FUTURE_VERSION (tag release or online hash); a bare `cargo build` does not,
/// so we mirror version.mjs's local scheme here from git directly.
fn emit_build_info() {
    let base = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
    // Treat an empty FUTURE_VERSION as unset (matches scripts/version.mjs), so a
    // failed `$(shell …)` in the Makefile can't inject an empty version string.
    let version = std::env::var("FUTURE_VERSION")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| local_dev_version(&base));
    println!("cargo:rustc-env=FUTURE_VERSION={version}");

    // The version string is not enough to identify the code: a release tag
    // (`1.2.3`) and a coordinated build (`0.0.2-<run>+test`) carry no commit at
    // all, so the commit is embedded separately. A client can then compare a
    // *running* Agent against a source checkout, which is what "is the agent I
    // am talking to the code I am reading?" requires.
    println!(
        "cargo:rustc-env=FUTURE_GIT_COMMIT={}",
        git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_string())
    );
    println!(
        "cargo:rustc-env=FUTURE_GIT_COMMIT_SHORT={}",
        git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".to_string())
    );
    println!(
        "cargo:rustc-env=FUTURE_GIT_DIRTY={}",
        if git_status_is_dirty() { "1" } else { "0" }
    );
    println!(
        "cargo:rustc-env=FUTURE_BUILD_TARGET={}",
        std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string())
    );
    println!(
        "cargo:rustc-env=FUTURE_BUILD_PROFILE={}",
        std::env::var("PROFILE").unwrap_or_else(|_| "unknown".to_string())
    );

    println!("cargo:rerun-if-env-changed=FUTURE_VERSION");
    // Without a watch on the git ref, Cargo would rerun this script only on the
    // env change above and a rebuild after committing would keep reporting the
    // previous commit. See cli/build.rs for why `--git-path` resolves both a
    // checkout and a worktree, and why only existing paths are emitted.
    let mut candidates = vec!["HEAD".to_string(), "packed-refs".to_string()];
    if let Some(reference) = git(&["symbolic-ref", "--quiet", "HEAD"]) {
        candidates.push(reference);
    }
    for candidate in candidates {
        if let Some(path) = git(&["rev-parse", "--git-path", &candidate]) {
            if Path::new(&path).exists() {
                println!("cargo:rerun-if-changed={path}");
            }
        }
    }
}

/// Local dev version from git, mirroring `scripts/version.mjs`:
/// `<base>-<short-hash>+local` (`+local.dirty` when the tree has uncommitted
/// changes). Falls back to `unknown` outside a git checkout. Only used when
/// FUTURE_VERSION isn't injected (bare `cargo build`).
fn local_dev_version(base: &str) -> String {
    let hash = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    format!(
        "{base}-{hash}+local{}",
        if git_status_is_dirty() { ".dirty" } else { "" }
    )
}

fn git(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

fn git_status_is_dirty() -> bool {
    Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .is_some_and(|o| o.status.success() && !o.stdout.is_empty())
}
