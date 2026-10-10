//! Host projection of results. Never transmit arbitrary stdout/file bodies.
use crate::sandbox::diagnostics::{permission_diagnostic, PermissionDiagnostic};
use serde_json::{json, Value};

pub(super) fn result_summary(tool_name: &str, is_error: bool, content: &str) -> Value {
    // Inspect only a small tail for fixed diagnostic categories. None of the
    // matching text or user-controlled values is returned to the provider.
    let mut start = content.len().saturating_sub(2_048);
    while !content.is_char_boundary(start) {
        start += 1;
    }
    let tail = content[start..].trim_end();
    let permission = permission_diagnostic(tail);
    let diagnostic = if !is_error {
        None
    } else if matches!(
        permission,
        Some(PermissionDiagnostic::PermissionDenied | PermissionDiagnostic::OperationNotPermitted)
    ) {
        Some("permission_denied")
    } else if permission == Some(PermissionDiagnostic::ReadOnlyFilesystem) {
        Some("read_only_filesystem")
    } else if tail.contains("No such file or directory") || tail.contains("cannot find the path") {
        Some("not_found")
    } else if tail.contains("timed out") || tail.contains("timeout") {
        Some("timeout")
    } else {
        Some("other_error")
    };
    let exit_code = if tool_name == "shell" {
        tail.rsplit_once("[exit: ")
            .and_then(|(_, suffix)| suffix.strip_suffix(']'))
            .and_then(|code| code.parse::<i32>().ok())
    } else {
        None
    };
    json!({"result_status":if is_error {"error"} else {"completed"},
        "content_policy":"body_omitted","output_bytes":content.len(),
        "diagnostic_category":diagnostic,"exit_code":exit_code})
}
