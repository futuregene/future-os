//! Static audit only; does not start any application or execute unit tests.
#[path = "../tests/support/diagnostic_audit.rs"]
mod audit;

fn main() -> Result<(), String> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let report = audit::collect(root);
    audit::check(&report)?;
    println!("{} production modules checked; {} preserved output-boundary calls; no legacy diagnostic prints", report.modules, report.rows.len());
    Ok(())
}
