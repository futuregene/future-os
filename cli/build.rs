// build.rs — Build-time version and build-metadata injection for the Rust CLI.
//
// Version injection mirrors scripts/version.mjs — the single source of truth
// for FutureOS build versioning — so `future --version` prints exactly what
// the TypeScript CLI prints for the same checkout/CI environment.
//
// It also injects the *structured* build identity (`FUTURE_GIT_COMMIT` and
// friends) that `future version --json` reports. The display version only
// carries a short hash for dev builds: a release tag (`v1.2.3`) and a
// coordinated test/nightly build (`0.0.2-<run>+test`) both omit the commit
// entirely, so the commit has to be embedded independently of it. Both values
// come from one `GitFacts` collection, so they can never disagree about which
// commit this binary was built from.
//
// Proto code generation is NOT owned here: future-rpc is the single proto
// codegen owner (PR #112) and the CLI consumes `future_rpc::proto` as a
// crate dependency.

use std::path::Path;
use std::process::Command;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    emit_build_info();
    Ok(())
}

/// Everything the build knows about the commit it came from, gathered once.
struct GitFacts {
    /// Full 40-hex object name, or `unknown` outside a git checkout.
    commit: String,
    /// Abbreviated object name, or `unknown`.
    short: String,
    /// Uncommitted changes (untracked included) at build time.
    dirty: bool,
    /// Paths whose change means "the commit this binary reports is stale", so
    /// Cargo reruns this script after a commit. Without them Cargo would only
    /// rerun on the env vars below (a `rerun-if-*` directive suppresses the
    /// default "any package file changed" rule), and a rebuild after committing
    /// would keep reporting the previous commit.
    watch: Vec<String>,
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

fn collect_git_facts() -> GitFacts {
    let commit = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    let short = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".to_string());

    // Paths whose change means the reported commit is stale. `--git-path`
    // resolves a normal checkout, a worktree (where `HEAD` lives per-worktree
    // and refs in the common dir) and a missing packed-refs alike, and its
    // output is relative to the build script's cwd — which is the package root,
    // the same base Cargo resolves `rerun-if-changed` against. Only existing
    // paths are emitted: Cargo treats a missing one as perpetually changed.
    let mut watch = Vec::new();
    let mut candidates = vec!["HEAD".to_string(), "packed-refs".to_string()];
    if let Some(reference) = git(&["symbolic-ref", "--quiet", "HEAD"]) {
        candidates.push(reference);
    }
    for candidate in candidates {
        if let Some(path) = git(&["rev-parse", "--git-path", &candidate]) {
            if Path::new(&path).exists() {
                watch.push(path);
            }
        }
    }
    GitFacts {
        commit,
        short,
        dirty: git_status_is_dirty(),
        watch,
    }
}

/// Uncommitted changes, untracked included. `false` outside a git checkout.
fn git_status_is_dirty() -> bool {
    Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .is_some_and(|o| o.status.success() && !o.stdout.is_empty())
}

/// Inject the display version (scripts/version.mjs `resolveVersion`, ported
/// 1:1) plus the structured build identity, all as compile-time env vars.
fn emit_build_info() {
    let facts = collect_git_facts();
    let version = resolve_version(&facts);

    println!("cargo:rustc-env=FUTURE_CLI_VERSION={version}");
    println!("cargo:rustc-env=FUTURE_GIT_COMMIT={}", facts.commit);
    println!("cargo:rustc-env=FUTURE_GIT_COMMIT_SHORT={}", facts.short);
    // "1"/"0" rather than true/false: the value is consumed by a `== "1"` test
    // in the JSON surface, where a typo in a boolean literal would silently
    // read as false.
    println!(
        "cargo:rustc-env=FUTURE_GIT_DIRTY={}",
        if facts.dirty { "1" } else { "0" }
    );
    // Cargo sets TARGET/PROFILE for build scripts. The triple answers "which
    // platform artifact is this", which a bug report otherwise has to guess.
    println!(
        "cargo:rustc-env=FUTURE_BUILD_TARGET={}",
        std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string())
    );
    println!(
        "cargo:rustc-env=FUTURE_BUILD_PROFILE={}",
        std::env::var("PROFILE").unwrap_or_else(|_| "unknown".to_string())
    );

    println!("cargo:rerun-if-env-changed=FUTURE_VERSION");
    println!("cargo:rerun-if-env-changed=GITHUB_REF");
    println!("cargo:rerun-if-env-changed=GITHUB_ACTIONS");
    println!("cargo:rerun-if-env-changed=CI");
    for path in &facts.watch {
        println!("cargo:rerun-if-changed={path}");
    }
}

/// Port of scripts/version.mjs `resolveVersion()`:
///   - FUTURE_VERSION env override wins (trimmed, empty treated as unset)
///   - release tag `refs/tags/vX.Y.Z` → `X.Y.Z`
///   - dev build `0.0.2-<hash>`; `+local` (and `.dirty`) appended for local
///     builds (no GITHUB_ACTIONS/CI), matching the TS CLI exactly.
fn resolve_version(facts: &GitFacts) -> String {
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
    // Dev minor is pinned at 0.0.2 (mirroring DEV_VERSION in version.mjs). A
    // commit-count scheme is useless in CI (shallow checkout → always 1), so
    // the version never derives from the git history length.
    let hash = facts.short.as_str();
    let ci = std::env::var("GITHUB_ACTIONS").is_ok() || std::env::var("CI").is_ok();
    if ci {
        return format!("0.0.2-{hash}");
    }
    format!(
        "0.0.2-{hash}+local{}",
        if facts.dirty { ".dirty" } else { "" }
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
