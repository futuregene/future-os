//! Read-only access to the consultations attached to an ordinary Agent run.
use crate::{
    rpc::{grpc_addr, RunClient},
    Output,
};

pub const HELP:&str="future fusion — inspect multi-model consultations\n\nUsage:\n  future run '/fusion <task>'\n  future fusion show <parent-session-id> [--run <id> --tool <id>] [--step <id>] [--offset <bytes>] [--json]\n\nWithout --run/--tool, shows the latest consultation in this session.\n--step reads an original child transcript in bounded pages. No model is called.\nStop the parent with future session abort <parent-session-id>.";

pub async fn fusion(command: Option<&str>, args: &[String], out: &Output) -> Result<(), String> {
    if command.is_none()
        || matches!(command, Some("--help" | "-h"))
        || args.iter().any(|s| s == "--help" || s == "-h")
    {
        out.log(HELP);
        return Ok(());
    }
    if command != Some("show") {
        return Err("Use future fusion show; see future fusion --help".into());
    }
    let session = args
        .first()
        .filter(|s| !s.starts_with('-'))
        .ok_or("Parent session ID is required")?;
    let mut run = String::new();
    let mut tool = String::new();
    let mut step = String::new();
    let mut offset = 0;
    let mut i = 1;
    while i < args.len() {
        let flag = &args[i];
        if flag == "--json" {
            i += 1;
            continue;
        }
        let value = args
            .get(i + 1)
            .ok_or_else(|| format!("{flag} requires a value"))?;
        match flag.as_str() {
            "--run" => run = value.clone(),
            "--tool" => tool = value.clone(),
            "--step" => step = value.clone(),
            "--offset" => offset = value.parse::<i64>().map_err(|_| "Invalid byte offset")?,
            _ => return Err(format!("Unknown argument: {flag}")),
        }
        i += 2;
    }
    if run.is_empty() != tool.is_empty() {
        return Err("Provide --run and --tool together".into());
    }
    if offset < 0 {
        return Err("Offset must be nonnegative".into());
    }
    let data = RunClient::new(&grpc_addr())
        .fusion_discussion(session, &run, &tool, &step, offset)
        .await?;
    out.log(&serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?);
    Ok(())
}
