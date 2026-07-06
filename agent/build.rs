// build.rs - Compile proto files for FutureAgent
//
// This build script compiles the proto files from the proto/ directory.
// The proto files are in a separate repository at ../proto/

use std::path::PathBuf;
use std::process::Command;

fn main() {
    emit_build_version();

    // Find the proto directory (sibling to agent)
    let proto_dir = PathBuf::from("../proto");

    if !proto_dir.exists() {
        println!(
            "cargo:warning=Proto directory not found at {:?}, skipping proto compilation",
            proto_dir
        );
        // Still tell cargo to rerun if build.rs changes
        return;
    }

    // Get list of proto files
    let proto_files: Vec<_> = std::fs::read_dir(&proto_dir)
        .expect("Failed to read proto directory")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "proto"))
        .map(|e| e.path())
        .collect();

    if proto_files.is_empty() {
        println!("cargo:warning=No proto files found in {:?}", proto_dir);
        return;
    }

    println!("cargo:rerun-if-changed=../proto");

    let generated = PathBuf::from("src/grpc/generated/proto.rs");

    // If the generated file already exists, skip proto compilation.
    // Proto files change rarely; re-running protoc every build is wasteful and
    // can fail in sandboxed environments where prost-build can't write temp files.
    if generated.exists() {
        println!(
            "cargo:warning=Generated proto at {:?} already exists; skipping proto compilation",
            generated
        );
        return;
    }

    if !has_protoc() {
        panic!(
            "Could not find `protoc`, and {:?} does not exist. Install protobuf with `brew install protobuf` or set PROTOC.",
            generated
        );
    }

    // Compile with tonic_build
    tonic_build::configure()
        .build_server(true)
        .build_client(false)
        .out_dir("src/grpc/generated")
        .compile_protos(&proto_files, &[&proto_dir])
        .expect("Failed to compile proto files");
}

/// Inject the build version (see `scripts/version.mjs`) as a compile-time env so
/// code can read it via `env!("FUTURE_VERSION")`. CI/`make` set FUTURE_VERSION
/// (tag release or online hash); a bare `cargo build` does not, so we mirror
/// version.mjs's local scheme here from git directly.
fn emit_build_version() {
    let base = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
    // Treat an empty FUTURE_VERSION as unset (matches scripts/version.mjs), so a
    // failed `$(shell …)` in the Makefile can't inject an empty version string.
    let version = std::env::var("FUTURE_VERSION")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| local_dev_version(&base));
    println!("cargo:rustc-env=FUTURE_VERSION={version}");
    println!("cargo:rerun-if-env-changed=FUTURE_VERSION");
}

/// Local dev version from git, mirroring `scripts/version.mjs`:
/// `<base>-<short-hash>+local` (`+local.dirty` when the tree has uncommitted
/// changes). Falls back to `unknown` outside a git checkout. Only used when
/// FUTURE_VERSION isn't injected (bare `cargo build`).
fn local_dev_version(base: &str) -> String {
    let git = |args: &[&str]| {
        Command::new("git")
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

fn has_protoc() -> bool {
    if std::env::var_os("PROTOC").is_some() {
        return true;
    }

    Command::new("protoc")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}
