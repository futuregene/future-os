//! Preserve JSON arguments when PowerShell launches an npm-generated .cmd shim.
//!
//! cmd.exe can strip JSON quotes. Transport affected arguments through stdin
//! instead, using an ASCII-only base64 literal decoded inside PowerShell. No
//! temporary file is created, so cancellation/failure cannot leak argument files.

#[cfg(windows)]
pub fn rewrite_future_tools_args(command: &str) -> Option<String> {
    rewrite_impl(command)
}

#[cfg(not(windows))]
pub fn rewrite_future_tools_args(_command: &str) -> Option<String> {
    None
}

#[cfg(any(windows, test))]
fn rewrite_impl(command: &str) -> Option<String> {
    use base64::Engine;
    let trimmed = command.trim();
    if !trimmed.starts_with("future tools call ") && !trimmed.starts_with("future.exe tools call ")
    {
        return None;
    }
    // Deliberately do not rewrite compound shell programs. Operators adjacent
    // to words are operators too; whitespace is not a security boundary.
    let mut quoted = false;
    for c in trimmed.chars() {
        if c == '\'' {
            quoted = !quoted;
        }
        if !quoted && matches!(c, '|' | '>' | '<' | '&' | ';' | '\n' | '\r' | '`') {
            return None;
        }
    }
    let args_pos = trimmed.find(" --args ")?;
    let prefix = &trimmed[..args_pos];
    let value = trimmed[args_pos + " --args ".len()..].trim_start();
    let inner = value.strip_prefix('\'')?;
    let mut chars = inner.char_indices().peekable();
    let mut json = String::new();
    let mut end = None;
    while let Some((index, ch)) = chars.next() {
        if ch == '\'' {
            if chars.peek().is_some_and(|(_, next)| *next == '\'') {
                chars.next(); // PowerShell escapes a single quote by doubling it
                json.push('\'');
            } else {
                end = Some(index + 1);
                break;
            }
        } else {
            json.push(ch);
        }
    }
    let suffix = inner[end?..].trim();
    let parsed: serde_json::Value = serde_json::from_str(&json).ok()?;
    if !json_values_contain_commas(&parsed) {
        return None;
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(json.as_bytes());
    // Windows PowerShell 5 defaults native stdin to ASCII; explicitly choose
    // UTF-8 for Unicode JSON. The enclosing shell is a per-command process.
    Some(format!(
        "$OutputEncoding = New-Object System.Text.UTF8Encoding($false); [System.Text.Encoding]::UTF8.GetString([System.Convert]::FromBase64String('{encoded}')) | {prefix} --stdin {suffix}"
    ))
}

#[cfg(any(windows, test))]
fn json_values_contain_commas(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(s) => s.contains(','),
        serde_json::Value::Array(a) => a.iter().any(json_values_contain_commas),
        serde_json::Value::Object(o) => o.values().any(json_values_contain_commas),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn rewrite_preserves_json_and_trailing_flags_without_temp_files() {
        let json = r#"{"info":"中文, patient's text, b"}"#;
        let quoted = json.replace('\'', "''");
        let rewritten = rewrite_impl(&format!(
            "future tools call search_paper --args '{quoted}' --timeout 60"
        ))
        .unwrap();
        assert!(rewritten.ends_with("| future tools call search_paper --stdin --timeout 60"));
        assert!(!rewritten.contains("--args"));
        assert!(!rewritten.contains(" < "));
        assert!(!rewritten.contains("future-tool-args-"));
        let encoded = rewritten
            .split("FromBase64String('")
            .nth(1)
            .unwrap()
            .split('\'')
            .next()
            .unwrap();
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .unwrap(),
            json.as_bytes()
        );
    }

    #[test]
    fn simple_json_and_compound_programs_are_not_rewritten() {
        for command in [
            "future tools call x --args '{\"query\":\"test\"}'",
            "future tools call x --stdin",
            "echo x | future tools call x --args '{\"x\":\"a,b\"}'",
            "future tools call x --args '{\"x\":\"a,b\"}';echo no",
            "future tools call x --args '{\"x\":\"a,b\"}'&&echo no",
            "future tools call x --args '{\"x\":\"a,b\"}'\necho no",
        ] {
            assert!(rewrite_impl(command).is_none(), "{command}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn powershell_executes_rewritten_stdin_with_unicode() {
        // Use a local function instead of a real Future CLI/network call.
        let json = r#"{"info":"中文, a, b"}"#;
        let rewritten = rewrite_impl(&format!("future tools call x --args '{json}'")).unwrap();
        let script =
            format!("function future {{ process {{ [Console]::WriteLine($_) }} }}; {rewritten}");
        let (program, args) = crate::sandbox::shell_invocation(&script);
        let output = std::process::Command::new(program)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["info"], "中文, a, b");
    }
}
