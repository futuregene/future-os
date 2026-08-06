// build.rs — Proto code generation for the FutureOS loop control plane.
//
// Generates the FutureAgent gRPC client from the repo's canonical
// proto/future.proto into OUT_DIR. Requires protoc on PATH (same requirement
// as `make generate-proto`); generation happens on every build (unlike
// channels, which pins generated files).

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=../../proto/future.proto");

    let proto_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("proto");

    tonic_build::configure()
        .build_server(false)
        .build_client(true)
        .compile_protos(&[proto_dir.join("future.proto")], &[proto_dir])?;

    Ok(())
}
