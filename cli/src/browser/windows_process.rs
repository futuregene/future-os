//! Windows detached launcher — port of `cli/src/browser/windows-process.ts`.
//!
//! Launches a GUI process through PowerShell's Start-Process so Chrome does
//! not inherit the agent/CLI stdout pipe (which would keep a shell tool
//! waiting for EOF).

use base64::Engine;

/// `launchWindowsDetached(executable, args)` — resolve when Start-Process
/// itself exits (not when the browser exits).
pub async fn launch_windows_detached(executable: &str, args: &[String]) -> Result<(), String> {
    let script = build_start_process_script(executable, args);
    let encoded = encode_utf16le_base64(&script);

    let mut cmd = tokio::process::Command::new("powershell.exe");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-NoLogo",
        "-EncodedCommand",
        &encoded,
    ])
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null());

    let status = match cmd.status().await {
        Ok(s) => s,
        Err(e) => return Err(format!("Failed to launch browser through PowerShell: {e}")),
    };

    let code = status.code();
    if code == Some(0) {
        return Ok(());
    }
    let detail = match code {
        Some(c) => format!("exit code {c}"),
        None => "signal terminated".to_string(),
    };
    Err(format!("PowerShell failed to launch browser ({detail})."))
}

/// `buildStartProcessScript(executable, args)`.
///
/// `-ArgumentList` is omitted when there are no arguments: PowerShell validates
/// the switch as non-empty, so passing `''` fails the whole script
/// ("The argument is null or empty") and the launch reports a launcher failure
/// for a browser that would have started fine. Found by the empty-argv test.
pub fn build_start_process_script(executable: &str, args: &[String]) -> String {
    let argument_line = args
        .iter()
        .map(|a| quote_windows_command_line_argument(a))
        .collect::<Vec<_>>()
        .join(" ");
    let argument_list = if argument_line.is_empty() {
        String::new()
    } else {
        format!(
            " -ArgumentList {}",
            quote_powershell_literal(&argument_line)
        )
    };
    format!(
        "$ErrorActionPreference = 'Stop'; $process = Start-Process -FilePath {}{} -WindowStyle Normal -PassThru; if ($null -eq $process) {{ throw 'Start-Process did not return a process.' }}",
        quote_powershell_literal(executable),
        argument_list,
    )
}

/// `quoteWindowsCommandLineArgument(value)` — CommandLineToArgvW rules.
pub fn quote_windows_command_line_argument(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_string();
    }
    if !value.chars().any(|c| c.is_whitespace() || c == '"') {
        return value.to_string();
    }

    let mut result = String::from("\"");
    let mut backslashes: usize = 0;

    for ch in value.chars() {
        match ch {
            '\\' => {
                backslashes += 1;
                continue;
            }
            '"' => {
                result.push_str(&"\\".repeat(backslashes * 2 + 1));
                result.push('"');
            }
            _ => {
                result.push_str(&"\\".repeat(backslashes));
                result.push(ch);
            }
        }
        backslashes = 0;
    }

    result.push_str(&"\\".repeat(backslashes * 2));
    result.push('"');
    result
}

/// `quotePowerShellLiteral(value)` — single-quoted with doubled quotes.
fn quote_powershell_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// UTF-16LE base64 encoding (PowerShell -EncodedCommand).
fn encode_utf16le_base64(script: &str) -> String {
    let utf16: Vec<u16> = script.encode_utf16().collect();
    let bytes: Vec<u8> = utf16.iter().flat_map(|u| u.to_le_bytes()).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_argv_values_using_windows_rules() {
        assert_eq!(quote_windows_command_line_argument("a\tb"), "\"a\tb\"");
        assert_eq!(quote_windows_command_line_argument("a\nb"), "\"a\nb\"");
        assert_eq!(
            quote_windows_command_line_argument("--no-first-run"),
            "--no-first-run"
        );
        assert_eq!(
            quote_windows_command_line_argument("--user-data-dir=C:\\Users\\Ace User\\profile"),
            "\"--user-data-dir=C:\\Users\\Ace User\\profile\""
        );
        assert_eq!(quote_windows_command_line_argument("a\"b"), "\"a\\\"b\"");
    }

    #[test]
    fn builds_start_process_script() {
        let script = build_start_process_script(
            "C:\\Program Files\\Browser's App\\chrome.exe",
            &["--flag".to_string(), "value with spaces".to_string()],
        );
        assert!(script
            .contains("Start-Process -FilePath 'C:\\Program Files\\Browser''s App\\chrome.exe'"));
        assert!(script.contains("-ArgumentList '--flag \"value with spaces\"'"));
        assert!(script.contains("-WindowStyle Normal -PassThru"));
        assert!(!script.contains("cmd /c"));
    }

    /// The empty-argv script must not carry `-ArgumentList ''`: PowerShell 5.1
    /// rejects an empty value for that switch ("The argument is null or empty"),
    /// which would make every argument-less launch fail. The rest of the script
    /// is unchanged.
    #[test]
    fn empty_argv_omits_the_argument_list_switch() {
        let script = build_start_process_script("C:\\chrome.exe", &[]);
        assert!(
            !script.contains("-ArgumentList"),
            "an empty argument list is omitted, not passed as '': {script}"
        );
        assert!(
            script
                .contains("Start-Process -FilePath 'C:\\chrome.exe' -WindowStyle Normal -PassThru"),
            "the surrounding script is intact: {script}"
        );
        // A non-empty argv still gets exactly one switch, quoted as before.
        let one = build_start_process_script("C:\\chrome.exe", &["--flag".to_string()]);
        assert_eq!(one.matches("-ArgumentList").count(), 1, "{one}");
        assert!(one.contains("-ArgumentList '--flag'"), "{one}");
    }

    #[test]
    fn quotes_empty_and_trailing_backslash_args() {
        // Empty → explicit empty quotes.
        assert_eq!(quote_windows_command_line_argument(""), "\"\"");
        // Trailing backslashes double before the closing quote.
        assert_eq!(
            quote_windows_command_line_argument("C:\\dir with space\\"),
            "\"C:\\dir with space\\\\\""
        );
        // Backslash runs before a literal quote: 2n+1 rule.
        assert_eq!(
            quote_windows_command_line_argument("a\\\\\"b c"),
            "\"a\\\\\\\\\\\"b c\""
        );
    }

    #[test]
    fn encodes_utf16le_base64() {
        // "A" in UTF-16LE is [0x41, 0x00] → base64 "QQA=".
        assert_eq!(encode_utf16le_base64("A"), "QQA=");
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn launch_fails_without_powershell() {
        let _guard = crate::test_env::lock_env().await;
        // PATH without any powershell.exe → spawn error surfaces.
        let dir = tempfile::tempdir().expect("tempdir");
        let _env =
            crate::test_env::EnvGuard::set(&[("PATH", dir.path().as_os_str().to_os_string())]);
        let err = launch_windows_detached("chrome.exe", &["--flag".to_string()])
            .await
            .unwrap_err();
        assert!(err.contains("Failed to launch browser through PowerShell"));
    }

    /// Install a fake `powershell.exe` (a POSIX shell script) into a temp dir
    /// and point PATH at it exclusively. Unix-only: on Windows the real
    /// PowerShell would actually launch processes.
    #[cfg(not(windows))]
    async fn with_fake_powershell(
        script_body: &str,
    ) -> (tempfile::TempDir, crate::test_env::EnvGuard) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let bin = dir.path().join("powershell.exe");
        std::fs::write(&bin, format!("#!/bin/sh\n{script_body}\n")).expect("write");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let guard =
            crate::test_env::EnvGuard::set(&[("PATH", dir.path().as_os_str().to_os_string())]);
        (dir, guard)
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn launch_ok_when_powershell_exits_zero() {
        let _guard = crate::test_env::lock_env().await;
        let (_dir, _env) = with_fake_powershell("exit 0").await;
        launch_windows_detached("chrome.exe", &["--flag".to_string()])
            .await
            .expect("launch ok");
    }

    /// The success arm on the real platform: PowerShell's own exit code is what
    /// `launch_windows_detached` returns, and a `Start-Process` script that
    /// runs to completion exits 0 — the launched program's own exit code is not
    /// PowerShell's. An absolute path to a real Windows binary that exits at
    /// once and opens no window pins `Ok(())` without a browser being started.
    ///
    /// Windows-only: it needs the real PowerShell (the unix tests above use a
    /// fake one, so this arm is otherwise uncovered *here* while covered on
    /// unix — the platform red line only cuts one way).
    #[cfg(windows)]
    #[tokio::test]
    async fn launch_ok_when_powershell_exits_zero() {
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
        let whence = format!("{system_root}\\System32\\where.exe");
        assert!(
            std::path::Path::new(&whence).is_file(),
            "the probe binary exists: {whence}"
        );
        launch_windows_detached(&whence, &[])
            .await
            .unwrap_or_else(|e| panic!("Start-Process of {whence} exits 0: {e}"));
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn launch_err_on_nonzero_exit() {
        let _guard = crate::test_env::lock_env().await;
        let (_dir, _env) = with_fake_powershell("exit 3").await;
        let err = launch_windows_detached("chrome.exe", &[])
            .await
            .unwrap_err();
        assert!(err.contains("exit code 3"), "unexpected: {err}");
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn launch_err_on_signal_termination() {
        let _guard = crate::test_env::lock_env().await;
        let (_dir, _env) = with_fake_powershell("kill -TERM $$").await;
        let err = launch_windows_detached("chrome.exe", &[])
            .await
            .unwrap_err();
        assert!(err.contains("signal terminated"), "unexpected: {err}");
    }

    /// The Windows half of the launcher: the real `powershell.exe` is used, but
    /// nothing is ever started.
    ///
    /// `Start-Process -FilePath <missing>` fails under the script's own
    /// `$ErrorActionPreference = 'Stop'`, so PowerShell exits non-zero and the
    /// launcher reports the exit code — the arm the unix fake-PowerShell tests
    /// reach, with no browser, no window and no child left running.
    ///
    /// The call is bounded by an inner timeout: a PowerShell that never returns
    /// used to take the whole test process down (a flat spawn from the harness
    /// once hung past 60 s and then exited `0xffffffff`), and a bounded wait
    /// keeps that from being anyone else's problem. A timeout here is a test
    /// failure with a message that says so, not a hung suite.
    #[cfg(windows)]
    #[tokio::test]
    async fn launch_reports_a_powershell_failure_without_starting_anything() {
        let _guard = crate::test_env::lock_env().await;
        let missing = format!(
            "C:\\future-clitui-does-not-exist-{}.exe",
            std::process::id()
        );
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            launch_windows_detached(&missing, &["--flag".to_string()]),
        )
        .await
        .expect("PowerShell must answer within 30s")
        .expect_err("a missing executable cannot be started");
        assert!(
            result.contains("PowerShell failed to launch browser"),
            "{result}"
        );
        assert!(result.contains("exit code"), "{result}");

        // An empty argument list is rejected by Start-Process's own parameter
        // validation, which is also a non-zero exit rather than a silent no-op.
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            launch_windows_detached(&missing, &[]),
        )
        .await
        .expect("PowerShell must answer within 30s")
        .expect_err("an empty -ArgumentList cannot start anything either");
        assert!(result.contains("exit code"), "{result}");
    }
}
