use super::*;

/// Preserve status outside the output budget and below the transcript's 100K-byte cap.
pub(super) fn text(result: &ShellResult) -> String {
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
