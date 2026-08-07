//! Subprocess helpers — port of `cli/src/utils/process.ts`.

use crate::types::ServiceResult;
use std::process::Stdio;

/// Options mirroring Node's `SpawnOptionsWithoutStdio` (only the subset the
/// CLI actually uses today: working directory).
#[derive(Debug, Default, Clone)]
pub struct SpawnOptions {
    pub cwd: Option<std::path::PathBuf>,
}

/// `runProcess` — spawn with stdio ["ignore", "pipe", "pipe"], resolve with
/// `{ code: code ?? 1, stdout, stderr }`; a spawn error is returned as `Err`.
pub async fn run_process(
    command: &str,
    args: &[String],
    options: &SpawnOptions,
) -> Result<ServiceResult, std::io::Error> {
    let mut cmd = tokio::process::Command::new(command);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = &options.cwd {
        cmd.current_dir(cwd);
    }
    let output = cmd.output().await?;
    Ok(ServiceResult {
        code: output.status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// `runInheritedProcess` — spawn with inherited stdio; captures no output.
pub async fn run_inherited_process(
    command: &str,
    args: &[String],
) -> Result<ServiceResult, std::io::Error> {
    let status = tokio::process::Command::new(command)
        .args(args)
        .status()
        .await?;
    Ok(ServiceResult {
        code: status.code().unwrap_or(1),
        stdout: String::new(),
        stderr: String::new(),
    })
}

/// `formatProcessOutput` — trimmed stdout/stderr joined with a newline.
pub fn format_process_output(result: &ServiceResult) -> String {
    let stdout = result.stdout.trim();
    let stderr = result.stderr.trim();
    match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => String::new(),
        (false, true) => stdout.to_string(),
        (true, false) => stderr.to_string(),
        (false, false) => format!("{stdout}\n{stderr}"),
    }
}
