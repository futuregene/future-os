use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 复用根目录 proto/future.proto（与 channels/build.rs 一致）
    let proto_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("proto");

    tonic_build::configure()
        .build_server(false)
        .build_client(true)
        .compile_protos(&[proto_dir.join("future.proto")], &[proto_dir])?;

    Ok(())
}
