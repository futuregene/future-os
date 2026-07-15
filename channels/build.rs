use std::path::PathBuf;
use std::process::Command;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("proto");
    let feishu_proto_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proto");
    let generated_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/generated");

    let gprc_rs = generated_dir.join("proto.rs");
    let feishu_ws_rs = generated_dir.join("feishu_ws.rs");

    // If generated files already exist, skip proto compilation.
    // Proto files change rarely; re-running protoc every build is wasteful
    // and fails when protoc isn't installed (e.g. fresh WSL without
    // protobuf-compiler).  The generated files are checked into git.
    if gprc_rs.exists() && feishu_ws_rs.exists() {
        return Ok(());
    }

    if !has_protoc() {
        panic!(
            "Could not find `protoc`, and {:?} does not exist. \
             Install protobuf with `sudo apt install protobuf-compiler` (Linux) \
             or `brew install protobuf` (macOS), or set PROTOC.",
            generated_dir
        );
    }

    println!("cargo:warning=Generating proto code (run `make generate-proto` to update checked-in files)");

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
