use super::probe::BwrapIdentity;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

pub const REQUEST_VERSION: u16 = 3;
/// Maximum decoded JSON request size. The legacy base64 CLI form is accepted
/// only by the hidden test/compatibility entry point; production uses an FD.
pub const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_MOUNTS: usize = 16_384;
// Leave headroom below Linux's per-string execve limit for `bash -c`.
pub const MAX_ARG_BYTES: usize = 96 * 1024;
pub const HELPER_REQUEST_FD: i32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperPhase {
    Outer,
    Inner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MountKind {
    Writable,
    ReadOnly,
    Unreadable,
    MissingProtected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MountRequest {
    pub source: PathBuf,
    pub target: PathBuf,
    pub kind: MountKind,
    pub expected: Option<BwrapIdentity>,
    pub source_fd: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinuxSandboxRequest {
    pub version: u16,
    pub phase: HelperPhase,
    pub bwrap_path: PathBuf,
    pub bwrap_identity: BwrapIdentity,
    pub cwd: PathBuf,
    pub argv: Vec<String>,
    pub mounts: Vec<MountRequest>,
    pub glob_snapshots: Vec<super::plan::GlobSnapshot>,
    pub policy_digest: String,
    pub status_fd: Option<i32>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RequestError {
    #[error("Linux sandbox helper request is too large")]
    TooLarge,
    #[error("Linux sandbox helper request encoding is invalid")]
    InvalidEncoding,
    #[error("Linux sandbox helper request JSON is invalid: {0}")]
    InvalidJson(String),
    #[error("Linux sandbox helper request version {0} is unsupported")]
    UnsupportedVersion(u16),
    #[error("Linux sandbox helper request has too many mounts")]
    TooManyMounts,
    #[error("Linux sandbox helper request argv is empty or too large")]
    InvalidArgv,
    #[error("Linux sandbox helper request contains an unsafe path: {0}")]
    UnsafePath(PathBuf),
    #[error("Linux sandbox helper request contains an invalid file descriptor")]
    InvalidFd,
    #[error("Linux sandbox helper request contains a duplicate file descriptor")]
    DuplicateFd,
    #[error("Linux sandbox helper request contains an invalid digest")]
    InvalidDigest,
}

impl LinuxSandboxRequest {
    pub fn to_json_bytes(&self) -> Result<Vec<u8>, RequestError> {
        self.validate()?;
        let json =
            serde_json::to_vec(self).map_err(|e| RequestError::InvalidJson(e.to_string()))?;
        if json.len() > MAX_REQUEST_BYTES {
            return Err(RequestError::TooLarge);
        }
        Ok(json)
    }

    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, RequestError> {
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err(RequestError::TooLarge);
        }
        let request: Self =
            serde_json::from_slice(bytes).map_err(|e| RequestError::InvalidJson(e.to_string()))?;
        request.validate()?;
        Ok(request)
    }

    pub fn encode(&self) -> Result<String, RequestError> {
        use base64::Engine as _;
        let json = self.to_json_bytes()?;
        Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json))
    }

    pub fn decode(encoded: &str) -> Result<Self, RequestError> {
        use base64::Engine as _;
        if encoded.len() > MAX_REQUEST_BYTES.saturating_mul(2) {
            return Err(RequestError::TooLarge);
        }
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| RequestError::InvalidEncoding)?;
        Self::from_json_bytes(&bytes)
    }

    pub fn validate(&self) -> Result<(), RequestError> {
        if self.version != REQUEST_VERSION {
            return Err(RequestError::UnsupportedVersion(self.version));
        }
        validate_path(&self.bwrap_path)?;
        validate_path(&self.cwd)?;
        if self.mounts.len() > MAX_MOUNTS {
            return Err(RequestError::TooManyMounts);
        }
        if self.glob_snapshots.len() > MAX_MOUNTS
            || self.glob_snapshots.iter().any(|snapshot| {
                snapshot.matches.len() > MAX_MOUNTS
                    || !Path::new(&snapshot.pattern).is_absolute()
                    || snapshot.matches.iter().any(|path| !path.is_absolute())
            })
        {
            return Err(RequestError::TooManyMounts);
        }
        let argv_bytes = self.argv.iter().map(String::len).sum::<usize>();
        if self.argv.is_empty()
            || argv_bytes > MAX_ARG_BYTES
            || self.argv.iter().any(|arg| arg.as_bytes().contains(&0))
        {
            return Err(RequestError::InvalidArgv);
        }
        if self.policy_digest.len() != 64
            || !self
                .policy_digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(RequestError::InvalidDigest);
        }
        let mut fds = std::collections::BTreeSet::new();
        match (self.phase, self.status_fd) {
            (HelperPhase::Outer, None) => {}
            (HelperPhase::Inner, Some(fd)) if fd >= 3 => {
                fds.insert(fd);
            }
            _ => return Err(RequestError::InvalidFd),
        }
        for mount in &self.mounts {
            validate_path(&mount.source)?;
            validate_path(&mount.target)?;
            if let Some(fd) = mount.source_fd {
                if fd < 3 {
                    return Err(RequestError::InvalidFd);
                }
                if !fds.insert(fd) {
                    return Err(RequestError::DuplicateFd);
                }
            }
            match (self.phase, mount.kind) {
                (HelperPhase::Outer, _)
                    if mount.source_fd.is_some() || mount.expected.is_some() =>
                {
                    return Err(RequestError::InvalidFd)
                }
                (HelperPhase::Inner, MountKind::MissingProtected) => {
                    if mount.source_fd.is_some() || mount.expected.is_some() {
                        return Err(RequestError::InvalidFd);
                    }
                }
                (HelperPhase::Inner, _)
                    if mount.source_fd.is_none() || mount.expected.is_none() =>
                {
                    return Err(RequestError::InvalidFd)
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn validate_path(path: &Path) -> Result<(), RequestError> {
    if !path.is_absolute()
        || path.as_os_str().as_encoded_bytes().contains(&0)
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(RequestError::UnsafePath(path.to_path_buf()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> BwrapIdentity {
        BwrapIdentity {
            device: 1,
            inode: 2,
            size: 3,
            modified_nanos: 4,
        }
    }

    fn request() -> LinuxSandboxRequest {
        LinuxSandboxRequest {
            version: REQUEST_VERSION,
            phase: HelperPhase::Outer,
            bwrap_path: PathBuf::from("/usr/bin/bwrap"),
            bwrap_identity: identity(),
            cwd: PathBuf::from("/tmp/work"),
            argv: vec!["/bin/sh".into(), "-c".into(), "true".into()],
            mounts: vec![MountRequest {
                source: PathBuf::from("/tmp/work"),
                target: PathBuf::from("/tmp/work"),
                kind: MountKind::Writable,
                expected: None,
                source_fd: None,
            }],
            glob_snapshots: Vec::new(),
            policy_digest: "a".repeat(64),
            status_fd: None,
        }
    }

    #[test]
    fn round_trip_is_versioned_and_structured() {
        let request = request();
        assert_eq!(
            LinuxSandboxRequest::decode(&request.encode().unwrap()).unwrap(),
            request
        );
    }

    #[test]
    fn rejects_unknown_version_unsafe_paths_and_nul_argv() {
        let mut invalid = request();
        invalid.version += 1;
        assert!(matches!(
            invalid.validate(),
            Err(RequestError::UnsupportedVersion(_))
        ));
        let mut invalid = request();
        invalid.cwd = PathBuf::from("relative");
        assert!(matches!(
            invalid.validate(),
            Err(RequestError::UnsafePath(_))
        ));
        let mut invalid = request();
        invalid.argv.push("bad\0arg".into());
        assert_eq!(invalid.validate(), Err(RequestError::InvalidArgv));
    }

    #[test]
    fn inner_requires_unique_non_stdio_fds_and_identity() {
        let mut invalid = request();
        invalid.phase = HelperPhase::Inner;
        assert_eq!(invalid.validate(), Err(RequestError::InvalidFd));
        invalid.status_fd = Some(6);
        invalid.mounts[0].source_fd = Some(7);
        invalid.mounts[0].expected = Some(identity());
        invalid.mounts.push(invalid.mounts[0].clone());
        assert_eq!(invalid.validate(), Err(RequestError::DuplicateFd));
    }

    #[test]
    fn inner_missing_path_mount_requires_no_host_source_fd() {
        let mut request = request();
        request.phase = HelperPhase::Inner;
        request.status_fd = Some(6);
        request.mounts[0].kind = MountKind::MissingProtected;
        request.mounts[0].source_fd = None;
        request.mounts[0].expected = None;
        assert_eq!(request.validate(), Ok(()));
    }

    #[test]
    fn request_size_mount_count_and_unknown_fields_are_bounded() {
        let mut invalid = request();
        invalid.mounts = vec![invalid.mounts[0].clone(); MAX_MOUNTS + 1];
        assert_eq!(invalid.validate(), Err(RequestError::TooManyMounts));

        let oversized = "x".repeat(MAX_REQUEST_BYTES * 2 + 1);
        assert_eq!(
            LinuxSandboxRequest::decode(&oversized),
            Err(RequestError::TooLarge)
        );

        use base64::Engine as _;
        let json = serde_json::to_string(&request()).unwrap();
        let unknown = json.replacen('{', "{\"unknown\":true,", 1);
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(unknown);
        assert!(matches!(
            LinuxSandboxRequest::decode(&encoded),
            Err(RequestError::InvalidJson(_))
        ));
    }
}
