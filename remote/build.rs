use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Reuse the repo-root proto/future.proto (same as channels/build.rs)
    let proto_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("proto");

    tonic_build::configure()
        .build_server(false)
        .build_client(true)
        .compile_protos(&[proto_dir.join("future.proto")], &[proto_dir])?;

    Ok(())
}
