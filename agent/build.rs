// build.rs - Compile proto files for FutureAgent
//
// This build script compiles the proto files from the proto/ directory.
// The proto files are in a separate repository at ../proto/

use std::path::PathBuf;
use std::process::Command;

fn main() {
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
