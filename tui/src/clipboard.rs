//! Plain-text clipboard output. No terminal frame or markdown decoration is
//! involved; platform failures are returned for the notice row.

use std::io::Write;
use std::process::{Command, Stdio};

pub fn copy_text(text: &str) -> Result<(), String> {
    let (program, args) = platform_command();
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not start {program}: {error}"))?;
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(text.as_bytes())
        .map_err(|error| format!("could not write the clipboard: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{program}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[cfg(target_os = "windows")]
fn platform_command() -> (&'static str, &'static [&'static str]) {
    (
        "powershell",
        &["-NoProfile", "-STA", "-Command", "$input | Set-Clipboard"],
    )
}
#[cfg(target_os = "macos")]
fn platform_command() -> (&'static str, &'static [&'static str]) {
    ("pbcopy", &[])
}
#[cfg(all(unix, not(target_os = "macos")))]
fn platform_command() -> (&'static str, &'static [&'static str]) {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        ("wl-copy", &[])
    } else {
        ("xclip", &["-selection", "clipboard"])
    }
}

/// Assistant prose and fenced code are content; ANSI cursor/frame sequences
/// and decorative tool output must never reach this boundary.
pub fn clean_assistant_text(text: &str) -> String {
    crate::utils::strip_ansi_codes(text).trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn copied_answer_is_content_not_terminal_chrome() {
        assert_eq!(
            clean_assistant_text("\x1b[31manswer\n```rust\nfn x() {}\n```\n\x1b[0m"),
            "answer\n```rust\nfn x() {}\n```"
        );
    }
}
