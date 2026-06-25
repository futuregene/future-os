fn main() -> Result<(), Box<dyn std::error::Error>> {
    tauri_build::build();

    let proto_path = std::path::Path::new("../../proto/future.proto");
    println!("cargo:rerun-if-changed={}", proto_path.display());
    tonic_build::configure()
        .build_server(false)
        .compile_protos(&[proto_path], &[proto_path.parent().unwrap()])?;

    Ok(())
}
