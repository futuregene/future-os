use std::env;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("proto");

    let feishu_proto_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proto");

    // Compile future.proto for gRPC client
    tonic_build::configure()
        .build_server(false)
        .build_client(true)
        .compile_protos(&[proto_dir.join("future.proto")], &[proto_dir])?;

    // Compile feishu_ws.proto for WebSocket frames
    prost_build::compile_protos(
        &[feishu_proto_dir.join("feishu_ws.proto")],
        &[feishu_proto_dir],
    )?;

    Ok(())
}
