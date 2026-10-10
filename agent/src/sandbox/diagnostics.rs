//! Shared text hints for routing and describing permission failures.
//! These are untrusted command diagnostics, never proof of a kernel denial.
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PermissionDiagnostic {
    OperationNotPermitted,
    PermissionDenied,
    ReadOnlyFilesystem,
    DeviceBusy,
}

pub(crate) fn permission_diagnostic(text: &str) -> Option<PermissionDiagnostic> {
    // Match structured Node error codes even when the OS message is localized.
    // A bare errno name in a filename or program output is not enough.
    static EPERM: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"(?i)\b(?:error\s*:\s*eperm\s*:|code\s*:\s*['"]eperm['"])"#).unwrap()
    });
    static EACCES: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"(?i)\b(?:error\s*:\s*eacces\s*:|code\s*:\s*['"]eacces['"])"#).unwrap()
    });
    let lower = text.to_ascii_lowercase();
    if lower.contains("operation not permitted") || EPERM.is_match(text) {
        Some(PermissionDiagnostic::OperationNotPermitted)
    } else if lower.contains("permission denied") || EACCES.is_match(text) {
        Some(PermissionDiagnostic::PermissionDenied)
    } else if lower.contains("read-only file system") {
        Some(PermissionDiagnostic::ReadOnlyFilesystem)
    } else if lower.contains("device or resource busy") {
        Some(PermissionDiagnostic::DeviceBusy)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_codes_and_message_case_are_recognized_without_bare_errno_matches() {
        for text in [
            "touch: '/outside/file': Operation not permitted",
            "Error: EPERM: operation not permitted, open '/outside/file'",
            "Error: EPERM: 本地化错误, open '/outside/file'",
            "  code: 'EPERM',",
            "OPERATION NOT PERMITTED",
        ] {
            assert_eq!(
                permission_diagnostic(text),
                Some(PermissionDiagnostic::OperationNotPermitted)
            );
        }
        for text in [
            "Error: EACCES: 本地化错误",
            "code: \"EACCES\"",
            "permission denied",
        ] {
            assert_eq!(
                permission_diagnostic(text),
                Some(PermissionDiagnostic::PermissionDenied)
            );
        }
        for text in [
            "cat: /tmp/EPERM: No such file or directory",
            "EPERM",
            "EACCES",
            "Could not resolve host",
        ] {
            assert_eq!(permission_diagnostic(text), None);
        }
    }
}
