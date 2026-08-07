// build.rs — Proto code generation for FutureChannel.
//
// The agent gRPC contract (future.proto) is generated once in the future-rpc
// crate; this crate consumes it via that dependency. Only the Feishu
// WebSocket frame codec (feishu_ws.proto) is generated here.
//
// Code generation happens via `make generate-proto`, NOT on normal builds:
// the generated file (src/generated/feishu_ws.rs) is checked into git, so
// protoc is never required to compile the channel bridge.

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-env-changed=REGENERATE_PROTO");
    // Re-run when the proto source changes, so `make generate-proto` after a
    // proto edit is not skipped by Cargo's build-script cache (the env var
    // alone is unchanged between two REGENERATE_PROTO=1 runs).
    println!("cargo:rerun-if-changed=proto/feishu_ws.proto");
    // Regeneration is opt-in via `make generate-proto` (sets the
    // REGENERATE_PROTO env var).  Skip it on normal builds so protoc is
    // never required to compile the channel bridge.
    if std::env::var("REGENERATE_PROTO").is_err() {
        return Ok(());
    }

    let feishu_proto_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proto");
    let generated_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/generated");

    // Compile feishu_ws.proto for WebSocket frames
    prost_build::Config::new()
        .out_dir(&generated_dir)
        .compile_protos(
            &[feishu_proto_dir.join("feishu_ws.proto")],
            &[feishu_proto_dir],
        )?;

    Ok(())
}
