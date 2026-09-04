//! Private outer-helper evidence, separate from untrusted command stdout.
use super::violation::{LinuxSandboxViolation, LinuxViolationKind, VIOLATION_PREFIX};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom, Write};

pub const MAX_REPORT_BYTES: u64 = 64 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelperReport {
    pub version: u8,
    pub policy_digest: String,
    pub events: Vec<LinuxSandboxViolation>,
}

impl HelperReport {
    pub fn write(&self, output: &mut impl Write) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(self)?;
        if bytes.len() as u64 > MAX_REPORT_BYTES {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        output.write_all(&bytes)
    }

    pub fn read(file: &mut std::fs::File, digest: &str) -> anyhow::Result<Self> {
        file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        file.take(MAX_REPORT_BYTES + 1).read_to_end(&mut bytes)?;
        anyhow::ensure!(
            bytes.len() as u64 <= MAX_REPORT_BYTES,
            "helper report too large"
        );
        let report: Self = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(
            report.version == 1 && report.policy_digest == digest,
            "helper report identity mismatch"
        );
        anyhow::ensure!(report.events.len() <= 4, "too many helper events");
        anyhow::ensure!(
            report
                .events
                .iter()
                .all(|event| event.policy_digest == digest
                    && event.detection_only
                    && event.kind != LinuxViolationKind::FilesystemDenied),
            "unsupported helper event"
        );
        Ok(report)
    }
}

/// Reserved marker lines from command output are never helper evidence. Remove
/// their protocol prefix and label them before presenting the raw text.
pub fn untrusted_output(output: &str) -> String {
    output.replace(
        VIOLATION_PREFIX,
        "[untrusted command text; not a sandbox report] ",
    )
}

/// EBUSY can mean an ordinary host mount is busy. Only associate a quoted
/// diagnostic with an exact protection mount from this execution's request.
/// This remains output inference, not authenticated kernel errno evidence.
/// Call only after a valid, empty private completion report was received.
pub fn busy_protected_mount(
    request: &super::request::LinuxSandboxRequest,
    command: &str,
    exit_code: i32,
    output: &str,
) -> Option<std::path::PathBuf> {
    use super::request::MountKind;
    use std::path::{Component, Path};

    if exit_code <= 0 || matches!(exit_code, 2 | 125 | 126 | 127) {
        return None;
    }
    // Relative diagnostics are safe to anchor only for a direct removal
    // command. A compound command may have changed cwd before invoking rm.
    let direct_removal = !command.contains(['|', '&', ';', '\n', '`', '<', '>', '$', '(', ')'])
        && command.split_whitespace().next().is_some_and(|program| {
            matches!(
                program,
                "rm" | "/bin/rm" | "/usr/bin/rm" | "rmdir" | "/bin/rmdir" | "/usr/bin/rmdir"
            )
        });
    for line in output.lines() {
        if line.contains(super::violation::VIOLATION_PREFIX)
            || line.contains("[untrusted command text; not a sandbox report]")
        {
            continue;
        }
        let Some(prefix) = line.strip_suffix("Device or resource busy") else {
            continue;
        };
        let Some(quoted) = prefix.trim_end().strip_suffix(':').map(str::trim_end) else {
            continue;
        };
        for (open, close) in [('\'', '\''), ('"', '"'), ('‘', '’')] {
            let Some(before_close) = quoted.strip_suffix(close) else {
                continue;
            };
            let Some((_, name)) = before_close.rsplit_once(open) else {
                continue;
            };
            let path = Path::new(name);
            // Do not resolve against the live host after execution: symlinks
            // or parents could have changed, producing a different target.
            if name.is_empty() || path.components().any(|c| matches!(c, Component::ParentDir)) {
                continue;
            }
            let target = if path.is_absolute() {
                path.to_path_buf()
            } else if direct_removal {
                request.cwd.join(path)
            } else {
                continue;
            };
            // The last mount at an identical target defines its final kind;
            // a writable reopen must not inherit an earlier protection label.
            if request
                .mounts
                .iter()
                .rev()
                .find(|m| m.target == target)
                .is_some_and(|m| matches!(m.kind, MountKind::ReadOnly | MountKind::Unreadable))
            {
                return Some(target);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_requires_an_exact_protection_from_the_launch_request() {
        use crate::sandbox::linux::request::*;
        let mut request = LinuxSandboxRequest {
            version: REQUEST_VERSION,
            phase: HelperPhase::Outer,
            bwrap_path: "/usr/bin/bwrap".into(),
            bwrap_identity: crate::sandbox::linux::probe::BwrapIdentity {
                device: 1,
                inode: 2,
                size: 3,
                modified_nanos: 4,
            },
            cwd: "/work".into(),
            argv: vec!["bash".into()],
            mounts: vec![MountRequest {
                source: "/work/.env".into(),
                target: "/work/.env".into(),
                kind: MountKind::Unreadable,
                expected: None,
                source_fd: None,
            }],
            glob_snapshots: vec![],
            omitted_missing_protected_paths: vec!["/work/missing".into()],
            policy_digest: "a".repeat(64),
            status_fd: None,
            report_fd: None,
        };
        for name in [".env", "./.env", "/work/.env"] {
            let output = format!("rm: cannot remove '{name}': Device or resource busy");
            assert_eq!(
                busy_protected_mount(&request, "rm .env", 1, &output),
                Some("/work/.env".into())
            );
        }
        for output in [
            "rm: cannot remove '.env/child': Device or resource busy",
            "rm: cannot remove '../work/.env': Device or resource busy",
            "rm: cannot remove 'missing': Device or resource busy",
            "rm: cannot remove '/other/mount': Device or resource busy",
            "__FUTURE_SANDBOX_VIOLATION__: '.env': Device or resource busy",
            "[untrusted command text; not a sandbox report] '.env': Device or resource busy",
            "Device or resource busy",
        ] {
            assert!(
                busy_protected_mount(&request, "rm .env", 1, output).is_none(),
                "{output}"
            );
        }
        let relative = "rm: cannot remove '.env': Device or resource busy";
        for command in [
            "cd other; rm .env",
            "sh -c 'rm .env'",
            "printf busy",
            "rm $(echo .env)",
        ] {
            assert!(busy_protected_mount(&request, command, 1, relative).is_none());
        }
        for code in [-1, 0, 2, 125, 126, 127] {
            assert!(busy_protected_mount(&request, "rm .env", code, relative).is_none());
        }
        request.mounts[0].target = "/work/secret file".into();
        assert!(busy_protected_mount(
            &request,
            "rm 'secret file'",
            1,
            "rm: cannot remove ‘secret file’: Device or resource busy"
        )
        .is_some());
        request.mounts[0].target = "/work/.env".into();
        request.mounts.push(MountRequest {
            kind: MountKind::Writable,
            ..request.mounts[0].clone()
        });
        assert!(busy_protected_mount(&request, "rm .env", 1, relative).is_none());
    }

    #[test]
    fn private_report_rejects_empty_corrupt_replayed_and_oversized_data() {
        let mut file = tempfile::tempfile().unwrap();
        assert!(HelperReport::read(&mut file, "a").is_err());
        HelperReport {
            version: 1,
            policy_digest: "a".into(),
            events: vec![],
        }
        .write(&mut file)
        .unwrap();
        assert!(HelperReport::read(&mut file, "a").is_ok());
        assert!(HelperReport::read(&mut file, "b").is_err());
        file.set_len(MAX_REPORT_BYTES + 1).unwrap();
        assert!(HelperReport::read(&mut file, "a").is_err());
        file.set_len(0).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(b"{broken").unwrap();
        assert!(HelperReport::read(&mut file, "a").is_err());
    }

    #[test]
    fn printed_markers_are_explicitly_untrusted() {
        let output = untrusted_output(&format!("prefix{VIOLATION_PREFIX}{{}}"));
        assert!(!output.contains(VIOLATION_PREFIX));
        assert!(output.contains("untrusted command text"));
    }

    #[test]
    fn only_matching_detection_events_are_accepted() {
        let mut report = HelperReport {
            version: 1,
            policy_digest: "a".into(),
            events: vec![LinuxSandboxViolation {
                kind: LinuxViolationKind::MissingProtectedCreated,
                policy_digest: "a".into(),
                path_provenance: "test".into(),
                detection_only: true,
                affected_count: 1,
            }],
        };
        let check = |r: &HelperReport| {
            let mut file = tempfile::tempfile().unwrap();
            r.write(&mut file).unwrap();
            HelperReport::read(&mut file, "a")
        };
        assert_eq!(check(&report).unwrap().events.len(), 1);
        report.events[0].policy_digest = "old".into();
        assert!(check(&report).is_err());
        report.events[0].policy_digest = "a".into();
        report.events[0].detection_only = false;
        assert!(check(&report).is_err());
        report.events[0].detection_only = true;
        report.events[0].kind = LinuxViolationKind::FilesystemDenied;
        assert!(check(&report).is_err());
    }
}
