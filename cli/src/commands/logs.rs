//! Read-only discovery of component diagnostics. No runtime profile controls.
use crate::Output;
use future_logging::{Component, Profile};

const COMPONENTS: [Component; 6] = [
    Component::Desktop,
    Component::Agent,
    Component::Host,
    Component::Channel,
    Component::Tui,
    Component::Cli,
];

pub const HELP: &str = "future logs — locate Rust component diagnostics

Usage:
  future logs status [--json]
  future logs path --component desktop|agent|host|channel|tui|cli

Paths use the effective FUTURE_HOME. Status reports files and this CLI build's
default profile; it does not claim to inspect another running binary's filter.
Overlapping processes may have a PID-suffixed file beside the canonical file.
RUST_LOG is a startup override; runtime diagnostic switching is not provided.";

pub fn run(args: &[String], out: &Output) -> Result<(), String> {
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
    {
        out.log(HELP);
        return Ok(());
    }
    let root = future_rpc::home::future_home();
    match args.first().map(String::as_str).unwrap_or("status") {
        "path" if args.len() == 3 && args[1] == "--component" => {
            let component = COMPONENTS
                .iter()
                .find(|component| component.name() == args[2])
                .ok_or_else(|| {
                    "Unknown log component; use desktop, agent, host, channel, tui or cli"
                        .to_string()
                })?;
            out.log(&component.path(&root).display().to_string());
        }
        "status" if args.len() <= 1 || (args.len() == 2 && args[1] == "--json") => {
            let files: Vec<_> = COMPONENTS.iter().map(|component| {
                let path = component.path(&root);
                serde_json::json!({"component":component.name(),"path":path,"exists":path.is_file()})
            }).collect();
            let profile = Profile::for_version(crate::version::VERSION);
            if args.iter().any(|arg| arg == "--json") {
                out.log(
                    &serde_json::to_string_pretty(&serde_json::json!({
                        "defaultProfile": format!("{profile:?}").to_lowercase(),
                        "version": crate::version::VERSION, "files":files,
                    }))
                    .map_err(|error| error.to_string())?,
                );
            } else {
                out.log(&format!(
                    "Build default: {profile:?} (RUST_LOG may override individual processes)"
                ));
                for file in files {
                    out.log(&format!(
                        "{}  {}  {}",
                        file["component"].as_str().unwrap(),
                        if file["exists"] == true {
                            "present"
                        } else {
                            "absent"
                        },
                        file["path"].as_str().unwrap()
                    ));
                }
            }
        }
        _ => return Err(format!("Invalid logs arguments\n{HELP}")),
    }
    Ok(())
}
