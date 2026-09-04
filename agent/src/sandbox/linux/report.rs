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

#[cfg(test)]
mod tests {
    use super::*;

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
