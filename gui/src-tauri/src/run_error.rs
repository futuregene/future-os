//! Classification of raw agent error strings into structured categories the UI
//! uses to render targeted recovery hints.

/// Classify a raw error message string into a structured category that the UI
/// can use to render targeted recovery hints. Returns one of:
/// `stream_disconnected`, `command_failed`, `model_failed`, `abort_requested`,
/// `timeout`, or `unknown`.
///
/// The classification is intentionally conservative: when in doubt we return
/// `unknown` so the UI shows the generic error message without a misleading
/// category. Keep the patterns ordered from most-specific to most-generic; an
/// abort-induced timeout, for example, must be classified as `abort_requested`
/// rather than `timeout`.
pub(crate) fn classify_run_error(error: &str) -> &'static str {
    let lower = error.to_lowercase();

    // User-initiated abort wins over every other category, including timeouts
    // that may be reported as a side effect of cancellation.
    if lower.contains("interrupted")
        || lower.contains("aborted")
        || lower.contains("terminated by user")
        || lower.contains("cancelled")
        || lower.contains("canceled")
    {
        return "abort_requested";
    }

    if lower.contains("timed out") || lower.contains("timeout") {
        return "timeout";
    }

    // gRPC / transport / streaming layer failures.
    if lower.contains("unable to connect to future agent")
        || lower.contains("transport error")
        || lower.contains("broken pipe")
        || lower.contains("connection")
        || lower.contains("stream")
        || lower.contains("eof")
    {
        return "stream_disconnected";
    }

    // LLM / model-side failures. Check before generic `command failed` because
    // some providers report rate limit errors that mention "request" but are
    // model errors, not bash failures.
    if lower.contains("model")
        || lower.contains("llm")
        || lower.contains("provider")
        || lower.contains("api key")
        || lower.contains("rate limit")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
        || lower.contains("openai")
        || lower.contains("anthropic")
    {
        return "model_failed";
    }

    // Tool / shell command failures. We anchor on bash/exit-code phrasing the
    // agent itself emits to avoid mis-classifying generic "command" mentions.
    if lower.contains("bash command")
        || lower.contains("exit code")
        || lower.contains("failed to run bash")
        || lower.contains("tool execution")
    {
        return "command_failed";
    }

    "unknown"
}

#[cfg(test)]
mod tests {
    use super::classify_run_error;

    #[test]
    fn test_classify_abort_requested() {
        assert_eq!(
            classify_run_error("Bash command interrupted by abort"),
            "abort_requested"
        );
        assert_eq!(classify_run_error("Interrupted"), "abort_requested");
        assert_eq!(classify_run_error("Terminated by user."), "abort_requested");
        assert_eq!(classify_run_error("aborted"), "abort_requested");
        assert_eq!(classify_run_error("cancelled"), "abort_requested");
        assert_eq!(classify_run_error("canceled"), "abort_requested");
    }

    #[test]
    fn test_classify_timeout() {
        assert_eq!(
            classify_run_error("Bash command timed out after 60 seconds"),
            "timeout"
        );
        assert_eq!(classify_run_error("timeout"), "timeout");
        assert_eq!(classify_run_error("Timed out"), "timeout");
    }

    #[test]
    fn test_classify_stream_disconnected() {
        assert_eq!(
            classify_run_error("Unable to connect to Future Agent at 127.0.0.1:50051"),
            "stream_disconnected"
        );
        assert_eq!(
            classify_run_error("Transport error: broken pipe"),
            "stream_disconnected"
        );
        assert_eq!(
            classify_run_error("connection closed"),
            "stream_disconnected"
        );
        assert_eq!(
            classify_run_error("Stream error: unexpected EOF"),
            "stream_disconnected"
        );
    }

    #[test]
    fn test_classify_model_failed() {
        assert_eq!(
            classify_run_error("Model returned error: unauthorized"),
            "model_failed"
        );
        assert_eq!(
            classify_run_error("LLM provider failed: rate limit exceeded"),
            "model_failed"
        );
        assert_eq!(classify_run_error("api key is invalid"), "model_failed");
        assert_eq!(classify_run_error("forbidden"), "model_failed");
        assert_eq!(classify_run_error("OpenAI API error"), "model_failed");
        assert_eq!(classify_run_error("Anthropic API error"), "model_failed");
    }

    #[test]
    fn test_classify_command_failed() {
        assert_eq!(
            classify_run_error("Bash command exited with code 1"),
            "command_failed"
        );
        assert_eq!(
            classify_run_error("Failed to run bash command: no such file"),
            "command_failed"
        );
        assert_eq!(classify_run_error("exit code: 127"), "command_failed");
    }

    #[test]
    fn test_classify_unknown() {
        assert_eq!(
            classify_run_error("Something unexpected happened"),
            "unknown"
        );
        assert_eq!(classify_run_error(""), "unknown");
    }

    #[test]
    fn test_classify_abort_beats_timeout() {
        // Interrupt aborts take priority over timeout mentions
        assert_eq!(
            classify_run_error("Bash command interrupted: timed out"),
            "abort_requested"
        );
        assert_eq!(
            classify_run_error("Timed out (user aborted)"),
            "abort_requested"
        );
    }
}
