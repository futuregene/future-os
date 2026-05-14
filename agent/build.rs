// build.rs - Compile proto files for FutureAgent
//
// This build script compiles the proto files from the proto/ directory.
// The proto files are in a separate repository at ../proto/

use std::path::PathBuf;

fn main() {
    // Find the proto directory (sibling to agent)
    let proto_dir = PathBuf::from("../proto/proto");

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

    println!("cargo:rerun-if-changed=../proto/proto");

    // Compile with tonic_build
    tonic_build::configure()
        .build_server(true)
        .build_client(false)
        .out_dir("src/grpc/generated")
        .compile_protos(&proto_files, &[&proto_dir])
        .expect("Failed to compile proto files");
}
