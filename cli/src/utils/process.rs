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

#[cfg(test)]
mod tests {
    use super::*;

    /// Cross-platform "print to stdout" one-liner. `#[cfg]` (not `cfg!`) so
    /// the off-platform branch is not compiled into this target's binary.
    #[cfg(windows)]
    fn shell_cmd(script: &str) -> (String, Vec<String>) {
        (
            "cmd".to_string(),
            vec!["/C".to_string(), script.to_string()],
        )
    }

    /// Unix half of [`shell_cmd`]. Absolute path: other tests mutate PATH.
    #[cfg(not(windows))]
    fn shell_cmd(script: &str) -> (String, Vec<String>) {
        (
            "/bin/sh".to_string(),
            vec!["-c".to_string(), script.to_string()],
        )
    }

    /// Print to stderr then exit 3 (cmd.exe separates commands with `&`).
    #[cfg(windows)]
    const STDERR_EXIT3_SCRIPT: &str = "echo oops 1>&2 & exit 3";
    /// Unix half of [`STDERR_EXIT3_SCRIPT`].
    #[cfg(not(windows))]
    const STDERR_EXIT3_SCRIPT: &str = "echo oops 1>&2; exit 3";

    /// Exit 2 (with some stdout noise on Unix).
    #[cfg(windows)]
    const EXIT2_SCRIPT: &str = "exit 2";
    /// Unix half of [`EXIT2_SCRIPT`].
    #[cfg(not(windows))]
    const EXIT2_SCRIPT: &str = "echo hidden; exit 2";

    /// Print the working directory.
    #[cfg(windows)]
    const PWD_SCRIPT: &str = "cd";
    /// Unix half of [`PWD_SCRIPT`].
    #[cfg(not(windows))]
    const PWD_SCRIPT: &str = "pwd";

    #[tokio::test]
    async fn run_process_captures_stdout_and_code() {
        let (cmd, args) = shell_cmd("echo hello");
        let result = run_process(&cmd, &args, &SpawnOptions::default())
            .await
            .expect("spawn");
        assert_eq!(result.code, 0);
        assert_eq!(result.stdout.trim(), "hello");
        assert!(result.stderr.is_empty());
    }

    #[tokio::test]
    async fn run_process_captures_stderr_and_failure_code() {
        let (cmd, args) = shell_cmd(STDERR_EXIT3_SCRIPT);
        let result = run_process(&cmd, &args, &SpawnOptions::default())
            .await
            .expect("spawn");
        assert_eq!(result.code, 3);
        assert_eq!(result.stderr.trim(), "oops");
    }

    #[tokio::test]
    async fn run_process_honors_cwd() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (cmd, args) = shell_cmd(PWD_SCRIPT);
        let options = SpawnOptions {
            cwd: Some(dir.path().to_path_buf()),
        };
        let result = run_process(&cmd, &args, &options).await.expect("spawn");
        assert_eq!(result.code, 0);
        let reported = std::path::PathBuf::from(result.stdout.trim());
        // macOS tempdirs are /var symlinks to /private/var — canonicalize.
        let reported = reported.canonicalize().unwrap_or(reported);
        let expected = dir
            .path()
            .canonicalize()
            .unwrap_or_else(|_| dir.path().to_path_buf());
        assert_eq!(reported, expected);
    }

    #[tokio::test]
    async fn run_process_spawn_error_is_err() {
        let err = run_process(
            "definitely-not-a-real-binary-xyz",
            &[],
            &SpawnOptions::default(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn run_inherited_process_returns_code_only() {
        let (cmd, args) = shell_cmd(EXIT2_SCRIPT);
        let result = run_inherited_process(&cmd, &args).await.expect("spawn");
        assert_eq!(result.code, 2);
        assert!(result.stdout.is_empty());
        assert!(result.stderr.is_empty());
    }

    #[tokio::test]
    async fn run_inherited_process_spawn_error_is_err() {
        let err = run_inherited_process("definitely-not-a-real-binary-xyz", &[])
            .await
            .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn format_process_output_combinations() {
        let make = |stdout: &str, stderr: &str| ServiceResult {
            code: 0,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        };
        assert_eq!(format_process_output(&make("", "")), "");
        assert_eq!(format_process_output(&make("  out \n", "")), "out");
        assert_eq!(format_process_output(&make("", " err\n")), "err");
        assert_eq!(format_process_output(&make("out", "err")), "out\nerr");
    }
}
