use serde::{Deserialize, Serialize};

pub const VIOLATION_PREFIX: &str = "__FUTURE_SANDBOX_VIOLATION__:";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinuxViolationKind {
    FilesystemDenied,
    DynamicGlobCreated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinuxSandboxViolation {
    pub kind: LinuxViolationKind,
    pub path_provenance: String,
    pub policy_digest: String,
    pub detection_only: bool,
    pub affected_count: usize,
}

pub fn marker(violation: &LinuxSandboxViolation) -> String {
    format!(
        "{VIOLATION_PREFIX}{}",
        serde_json::to_string(violation).expect("violation serializes")
    )
}

pub fn parse_marker(output: &str) -> Option<LinuxSandboxViolation> {
    output.lines().rev().find_map(|line| {
        line.strip_prefix(VIOLATION_PREFIX)
            .and_then(|json| serde_json::from_str(json).ok())
    })
}

pub fn classify(
    exit_code: i32,
    output: &str,
    policy_digest: &str,
) -> Option<LinuxSandboxViolation> {
    if let Some(violation) = parse_marker(output) {
        return Some(violation);
    }
    if exit_code == 0 || matches!(exit_code, 2 | 125 | 126 | 127) {
        return None;
    }
    let lower = output.to_ascii_lowercase();
    if lower.contains("permission denied")
        || lower.contains("operation not permitted")
        || lower.contains("read-only file system")
    {
        return Some(LinuxSandboxViolation {
            kind: LinuxViolationKind::FilesystemDenied,
            path_provenance: "command_output_inferred".into(),
            policy_digest: policy_digest.into(),
            detection_only: false,
            affected_count: 0,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_denials_but_excludes_infrastructure_and_shell_codes() {
        assert!(classify(1, "write: Read-only file system", "abc").is_some());
        assert!(classify(1, "cat: Permission denied", "abc").is_some());
        for code in [0, 2, 125, 126, 127] {
            assert!(classify(code, "Permission denied", "abc").is_none());
        }
        assert!(classify(1, "ordinary compiler error", "abc").is_none());
    }

    #[test]
    fn structured_marker_round_trips_without_a_host_path() {
        let violation = LinuxSandboxViolation {
            kind: LinuxViolationKind::DynamicGlobCreated,
            path_provenance: "glob_snapshot".into(),
            policy_digest: "a".repeat(64),
            detection_only: true,
            affected_count: 2,
        };
        assert_eq!(parse_marker(&marker(&violation)), Some(violation));
    }
}
