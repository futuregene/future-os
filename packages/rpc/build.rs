// build.rs — Proto code generation for future-rpc.
//
// Proto code generation happens via `make generate-proto`, NOT here.
// The generated file (src/generated/proto.rs) is checked into git so normal
// builds never need protoc.
//
// future-rpc is the SINGLE owner of the generated proto code: it emits both
// the tonic server (used by the agent) and client (used by channels and the
// desktop backend) modules. The per-crate generated copies that used to live
// in agent/, channels/ and desktop/src-tauri/ are being retired onto this crate.

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Agent and RPC share the 0.0.0 Cargo release baseline. CLI/TUI display
    // versions use a different development scheme; do not compare that label
    // against the Agent built into the same checkout. Coordinated builds set
    // FUTURE_VERSION for every crate and remain strictly version checked.
    let agent_version = std::env::var("FUTURE_VERSION")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());
    println!("cargo:rustc-env=EXPECTED_AGENT_VERSION={agent_version}");
    println!("cargo:rerun-if-env-changed=FUTURE_VERSION");
    println!("cargo:rerun-if-env-changed=REGENERATE_PROTO");
    // Also re-run when the proto source changes, so `make generate-proto`
    // after a proto edit is not skipped by Cargo's build-script cache (the
    // env var alone is unchanged between two REGENERATE_PROTO=1 runs).
    println!("cargo:rerun-if-changed=proto/future.proto");
    // Proto regeneration is opt-in via `make generate-proto` (sets the
    // REGENERATE_PROTO env var).  Skip it on normal builds so protoc is
    // never required to compile the crate.
    if std::env::var("REGENERATE_PROTO").is_err() {
        return Ok(());
    }

    let proto_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proto");
    let generated_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/generated");
    std::fs::create_dir_all(&generated_dir)?;

    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .out_dir(&generated_dir)
        .compile_protos(&[proto_dir.join("future.proto")], &[proto_dir])?;

    Ok(())
}
