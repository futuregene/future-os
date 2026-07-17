//! Rewrite `future tools call ... --args '...'` to `--stdin` on Windows.
//!
//! On Windows, when the `future` CLI is invoked through its npm-generated
//! `future.cmd` wrapper, cmd.exe strips double quotes from arguments. The
//! CLI's `parseCmdObject` fallback can't distinguish commas inside string
//! values from JSON field separators when quotes are gone, so JSON args
//! containing values like `"diagnosis, treatment, and prognosis"` break.
//!
//! This module detects such commands and rewrites them to pipe the JSON
//! through stdin via a temp file, bypassing cmd.exe argument processing.

/// On Windows: if `command` contains `future tools call ... --args '...'`
/// with JSON string values that contain commas (which cmd.exe will corrupt),
/// rewrite it to use `--stdin` via a temp file. Returns the rewritten command
/// (or `None` if no rewrite is needed).
#[cfg(target_os = "windows")]
pub fn rewrite_future_tools_args(command: &str) -> Option<String> {
    rewrite_impl(command).ok().flatten()
}

/// No-op on non-Windows — shell quoting handles this correctly.
#[cfg(not(target_os = "windows"))]
pub fn rewrite_future_tools_args(_command: &str) -> Option<String> {
    None
}

#[cfg(target_os = "windows")]
use anyhow::Result;

#[cfg(target_os = "windows")]
use std::path::PathBuf;

#[cfg(target_os = "windows")]
fn rewrite_impl(command: &str) -> Result<Option<String>> {
    // Only rewrite standalone `future tools call ...` commands (no pipes, chains).
    // Multi-command pipelines make argument rewriting fragile.
    let trimmed = command.trim();
    if !trimmed.starts_with("future tools call ") && !trimmed.starts_with("future.exe tools call ")
    {
        return Ok(None);
    }
    if contains_shell_operators(trimmed) {
        return Ok(None);
    }

    // Find `--args` followed by a single-quoted JSON blob.
    let args_flag_pos = match find_flag(trimmed, "--args") {
        Some(pos) => pos,
        None => return Ok(None),
    };

    // Extract the `--args` value: single-quoted JSON.
    let after_args = trimmed[args_flag_pos + "--args".len()..].trim_start();
    let json_str = match extract_single_quoted(after_args) {
        Some(s) => s,
        None => return Ok(None),
    };

    // Parse the JSON and check if any string values contain commas.
    let parsed: serde_json::Value = serde_json::from_str(json_str)?;
    if !json_values_contain_commas(&parsed) {
        return Ok(None);
    }

    // Write JSON to a temp file.
    let tmp_path = write_temp_json(json_str)?;

    // Build the rewritten command: replace `--args '...'` with `--stdin`
    // and append `< temp_file`.
    let prefix = &trimmed[..args_flag_pos];
    // Find where the --args value ends (closing single quote).
    let value_start = after_args.find('\'').unwrap_or(0);
    let value_end = after_args[value_start..]
        .find('\'')
        .map(|p| value_start + p + 1)
        .unwrap_or(after_args.len());
    let suffix = after_args[value_end..].trim_start();

    let rewritten = format!(
        "{}--stdin {} < \"{}\"",
        prefix.trim_end(),
        suffix,
        tmp_path.display()
    );

    Ok(Some(rewritten))
}

#[cfg(target_os = "windows")]
fn find_flag(command: &str, flag: &str) -> Option<usize> {
    // Search for the flag as a whole word (preceded by space or at start).
    let mut search_from = 0;
    loop {
        let pos = command[search_from..].find(flag)?;
        let abs = search_from + pos;
        let before_ok = abs == 0 || command.as_bytes()[abs - 1] == b' ';
        let after = abs + flag.len();
        let after_ok = after >= command.len() || command.as_bytes()[after] == b' ';
        if before_ok && after_ok {
            return Some(abs);
        }
        search_from = abs + 1;
    }
}

#[cfg(target_os = "windows")]
fn extract_single_quoted(s: &str) -> Option<&str> {
    let s = s.trim_start();
    if !s.starts_with('\'') {
        return None;
    }
    let inner = &s[1..];
    // Find the matching closing single quote, handling potential escaping.
    // In practice, JSON never contains single quotes in keys/values, so
    // a simple scan for the next single quote is sufficient.
    let end = inner.find('\'')?;
    Some(&inner[..end])
}

#[cfg(target_os = "windows")]
fn json_values_contain_commas(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(s) => s.contains(','),
        serde_json::Value::Array(arr) => arr.iter().any(json_values_contain_commas),
        serde_json::Value::Object(obj) => obj.values().any(json_values_contain_commas),
        _ => false,
    }
}

#[cfg(target_os = "windows")]
fn contains_shell_operators(command: &str) -> bool {
    // Check for pipe, redirect, chain operators, or command separators.
    // These make the command non-trivial to rewrite safely.
    let operators = ["|", ">", ">>", "<", "&&", "||", "&", ";"];
    let bytes = command.as_bytes();
    operators.iter().any(|op| {
        // Find operator as a standalone token (outside quotes).
        let mut search_from = 0;
        while let Some(pos) = command[search_from..].find(op) {
            let abs = search_from + pos;
            // Simple check: not inside single quotes.
            let single_quotes_before = command[..abs].chars().filter(|&c| c == '\'').count();
            if single_quotes_before % 2 == 0 {
                // For multi-char operators, check word boundary.
                let before_ok = abs == 0 || bytes[abs - 1].is_ascii_whitespace();
                let after = abs + op.len();
                let after_ok = after >= bytes.len() || bytes[after].is_ascii_whitespace();
                if before_ok && after_ok {
                    return true;
                }
            }
            search_from = abs + 1;
        }
        false
    })
}

#[cfg(target_os = "windows")]
fn write_temp_json(json: &str) -> Result<PathBuf> {
    let mut path = std::env::temp_dir();
    path.push(format!("future-tool-args-{}.json", uuid::Uuid::new_v4()));
    std::fs::write(&path, json)?;
    Ok(path)
}

#[cfg(test)]
#[cfg(target_os = "windows")]
mod tests {
    use super::*;

    #[test]
    fn test_no_rewrite_for_simple_json() {
        let cmd = "future tools call search_paper --args '{\"queries\": [\"test\"]}'";
        assert!(rewrite_future_tools_args(cmd).is_none());
    }

    #[test]
    fn test_rewrite_for_comma_in_value() {
        let cmd = "future tools call search_paper --args '{\"info\": \"a, b, c\"}'";
        let rewritten = rewrite_future_tools_args(cmd).unwrap();
        assert!(rewritten.contains("--stdin"));
        assert!(rewritten.contains(" < \""));
        assert!(!rewritten.contains("--args"));
        // suffix (--output etc.) should be preserved
        assert!(rewritten.starts_with("future tools call search_paper --stdin"));
    }

    #[test]
    fn test_no_rewrite_for_piped_command() {
        let cmd = "echo hello | future tools call search_paper --args '{\"info\": \"a, b\"}'";
        assert!(rewrite_future_tools_args(cmd).is_none());
    }

    #[test]
    fn test_no_rewrite_without_args() {
        let cmd = "future tools call search_paper --stdin";
        assert!(rewrite_future_tools_args(cmd).is_none());
    }

    #[test]
    fn test_rewrite_preserves_trailing_flags() {
        let cmd = "future tools call search_paper --args '{\"info\": \"a, b\"}' --timeout 60";
        let rewritten = rewrite_future_tools_args(cmd).unwrap();
        assert!(rewritten.contains("--timeout 60"));
        assert!(rewritten.contains(" < \""));
    }
}
