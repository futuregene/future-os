//! Host-owned process facts. Output text is a projection, never an input to these facts.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellStatus {
    Exited,
    LaunchFailed,
    ExecutionFailed,
    TimedOut,
    Cancelled,
    NotStarted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellAttempt {
    pub status: ShellStatus,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub output: String,
    pub output_truncated: bool,
    pub escalated: bool,
}

/// One command invocation. New calls run once; attempts also preserve historical retry records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellResult {
    pub command: String,
    pub cwd: String,
    pub duration_ms: u64,
    pub status: ShellStatus,
    pub exit_code: Option<i32>,
    pub is_soft_fail: bool,
    pub is_error: bool,
    pub attempts: Vec<ShellAttempt>,
    pub approval: Option<String>,
    pub note: Option<String>,
}

impl ShellResult {
    pub fn update_verdict(&mut self) {
        self.is_error = self.approval.as_deref().is_some_and(|approval| {
            matches!(approval, "denied" | "cancelled" | "context_invalidated")
        }) || match self.status {
            ShellStatus::Exited => self
                .exit_code
                .is_none_or(|code| code != 0 && !self.is_soft_fail),
            _ => true,
        };
    }
}

/// Preserve status outside the output budget and below the transcript's 100K-byte cap.
pub fn model_summary(result: &ShellResult) -> String {
    let output_limit = 60_000 / result.attempts.len().max(1);
    let mut text = format!(
        "[internal execution diagnostics]\nExecution status: {:?}; duration: {}ms.\n",
        result.status, result.duration_ms
    );
    for (index, attempt) in result.attempts.iter().enumerate() {
        text.push_str(&format!(
            "Attempt {} (escalated={}): [status: {:?}, duration: {}ms]\n",
            index + 1,
            attempt.escalated,
            attempt.status,
            attempt.duration_ms
        ));
        text.push_str(&bounded(&attempt.output, output_limit));
        text.push('\n');
        if let Some(code) = attempt.exit_code {
            text.push_str(&format!("[exit: {code}]\n"));
        } else {
            text.push_str("[exit: unavailable]\n");
        }
        if attempt.output_truncated || attempt.output.len() > output_limit {
            text.push_str("[output truncated in model summary; structured attempt retained]\n");
        }
    }
    if result.attempts.is_empty() {
        text.push_str("[process not started]\n");
    }
    if let Some(approval) = &result.approval {
        text.push_str(&format!("[approval: {approval}]\n"));
    }
    if let Some(note) = &result.note {
        // Successful approval notes are audit metadata, not an outstanding
        // task caveat. Keep actionable failure/context notes in model text.
        if result.is_error || result.approval.as_deref() != Some("approved") {
            text.push_str(&bounded(note, 2048));
            text.push('\n');
        }
    }
    text
}

fn bounded(text: &str, max: usize) -> String {
    text[text.ceil_char_boundary(text.len().saturating_sub(max))..].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(status: ShellStatus, exit_code: Option<i32>) -> ShellResult {
        ShellResult {
            command: "cargo test".into(),
            cwd: "/w".into(),
            duration_ms: 1,
            status,
            exit_code,
            is_soft_fail: false,
            is_error: false,
            attempts: Vec::new(),
            approval: None,
            note: None,
        }
    }

    /// A non-zero exit is the failure signal, but a soft-fail exit is a
    /// deliberate partial result and a denied approval is an error even when the
    /// command exited 0 (it never ran).
    #[test]
    fn a_denied_or_replaced_approval_is_an_error_whatever_the_exit_code() {
        for approval in ["denied", "cancelled", "context_invalidated"] {
            let mut shell = result(ShellStatus::Exited, Some(0));
            shell.approval = Some(approval.to_string());
            shell.update_verdict();
            assert!(shell.is_error, "{approval} must be reported as an error");
        }
        // An approval that was not a refusal leaves the exit code in charge.
        let mut shell = result(ShellStatus::Exited, Some(0));
        shell.approval = Some("approved".into());
        shell.update_verdict();
        assert!(!shell.is_error);
    }

    #[test]
    fn an_exited_command_follows_its_exit_code_and_soft_fail_flag() {
        let mut ok = result(ShellStatus::Exited, Some(0));
        ok.update_verdict();
        assert!(!ok.is_error);

        let mut failed = result(ShellStatus::Exited, Some(1));
        failed.update_verdict();
        assert!(failed.is_error);

        let mut soft = result(ShellStatus::Exited, Some(1));
        soft.is_soft_fail = true;
        soft.update_verdict();
        assert!(
            !soft.is_error,
            "a declared soft fail is reported by the caller, not as an error"
        );

        let mut signalled = result(ShellStatus::Exited, None);
        signalled.update_verdict();
        assert!(
            signalled.is_error,
            "an exit with no code (a signal) is not a success"
        );
    }

    #[test]
    fn every_non_exited_status_is_an_error() {
        for status in [
            ShellStatus::LaunchFailed,
            ShellStatus::ExecutionFailed,
            ShellStatus::TimedOut,
            ShellStatus::Cancelled,
            ShellStatus::NotStarted,
        ] {
            let mut shell = result(status, None);
            shell.update_verdict();
            assert!(shell.is_error, "{status:?} must be an error");
        }
    }
}
