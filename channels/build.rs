// build.rs — Proto code generation for FutureChannel.
//
// Proto code generation happens via `make generate-proto`, NOT here.
// The generated files (src/generated/*.rs) are checked into git so normal
// builds never need protoc.

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Proto regeneration is opt-in via `make generate-proto` (sets the
    // REGENERATE_PROTO env var).  Skip it on normal builds so protoc is
    // never required to compile the channel bridge.
    if std::env::var("REGENERATE_PROTO").is_err() {
        return Ok(());
    }

    let proto_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("proto");
    let feishu_proto_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proto");
    let generated_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/generated");

    // Compile future.proto for gRPC client
    tonic_build::configure()
        .build_server(false)
        .build_client(true)
        .out_dir(&generated_dir)
        .compile_protos(&[proto_dir.join("future.proto")], &[proto_dir])?;

    // Compile feishu_ws.proto for WebSocket frames
    prost_build::Config::new()
        .out_dir(&generated_dir)
        .compile_protos(
            &[feishu_proto_dir.join("feishu_ws.proto")],
            &[feishu_proto_dir],
        )?;

    Ok(())
}
