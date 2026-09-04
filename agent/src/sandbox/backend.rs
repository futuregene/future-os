use std::collections::BTreeMap;

/// Backend selected for a prepared shell invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellBackend {
    Plain,
    MacosSeatbelt,
    LinuxBubblewrap,
    WindowsRestricted,
}

/// Stable metadata attached to a shell process boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxBoundary {
    pub backend: ShellBackend,
    pub policy_digest: Option<String>,
}

/// A platform-neutral, fully structured process invocation.
///
/// Backend planners populate this value; the tools layer remains responsible
/// for cwd, stdio, timeout, interruption, and process-group lifecycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedShell {
    pub program: String,
    pub args: Vec<String>,
    pub env_delta: BTreeMap<String, Option<String>>,
    pub boundary: SandboxBoundary,
    /// Linux-only helper JSON. `into_command` copies it to an anonymous file
    /// and exposes that file as fixed FD 3 in the helper child; other backends
    /// must leave it unset.
    pub request_payload: Option<Vec<u8>>,
}

impl PreparedShell {
    pub fn plain(command: &str) -> Self {
        let (program, args) = super::shell_invocation(command);
        Self {
            program: program.to_string(),
            args,
            env_delta: BTreeMap::new(),
            boundary: SandboxBoundary {
                backend: ShellBackend::Plain,
                policy_digest: None,
            },
            request_payload: None,
        }
    }

    pub fn into_command(self) -> anyhow::Result<tokio::process::Command> {
        let mut command = tokio::process::Command::new(self.program);
        command.args(self.args);
        for (key, value) in self.env_delta {
            match value {
                Some(value) => {
                    command.env(key, value);
                }
                None => {
                    command.env_remove(key);
                }
            }
        }
        #[cfg(target_os = "linux")]
        if let Some(payload) = self.request_payload {
            use std::io::{Seek, Write};
            use std::os::fd::AsRawFd;
            use std::os::unix::process::CommandExt;

            let mut file = tempfile::tempfile()?;
            file.write_all(&payload)?;
            file.rewind()?;
            let source_fd = file.as_raw_fd();
            // SAFETY: the closure only performs async-signal-safe fd operations
            // in the forked child. Capturing `file` keeps the anonymous request
            // alive until spawn; FD 3 is the fixed private helper input.
            unsafe {
                command.as_std_mut().pre_exec(move || {
                    // The user's `(command) 2>&1` starts too late to capture
                    // helper/bwrap initialization errors. Share the stdout pipe
                    // at the OS boundary; no extra pipe reader or buffering is
                    // needed, and the existing timeout/output handling applies.
                    if libc::dup2(libc::STDOUT_FILENO, libc::STDERR_FILENO) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if source_fd != super::linux::request::HELPER_REQUEST_FD
                        && libc::dup2(source_fd, super::linux::request::HELPER_REQUEST_FD) < 0
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                    let flags =
                        libc::fcntl(super::linux::request::HELPER_REQUEST_FD, libc::F_GETFD);
                    if flags < 0
                        || libc::fcntl(
                            super::linux::request::HELPER_REQUEST_FD,
                            libc::F_SETFD,
                            flags & !libc::FD_CLOEXEC,
                        ) < 0
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                    let _keep_alive = &file;
                    Ok(())
                });
            }
        }
        Ok(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn helper_stderr_is_captured_before_user_shell_redirection() {
        let mut prepared = PreparedShell::plain("true");
        prepared.program = "/bin/sh".into();
        prepared.args = vec![
            "-c".into(),
            "echo 'helper initialization diagnostic' >&2; exit 125".into(),
        ];
        prepared.request_payload = Some(b"{}".to_vec());
        let output = prepared
            .into_command()
            .unwrap()
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .await
            .unwrap();
        assert_eq!(output.status.code(), Some(125));
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("helper initialization diagnostic")
        );
    }

    #[test]
    fn plain_preparation_preserves_structured_argv() {
        let prepared = PreparedShell::plain("printf 'a b'");
        assert_eq!(prepared.boundary.backend, ShellBackend::Plain);
        assert_eq!(
            prepared.args.last().map(String::as_str),
            Some("printf 'a b'")
        );
        assert!(prepared.env_delta.is_empty());
    }

    #[test]
    fn env_delta_is_applied_without_shell_interpolation() {
        let mut prepared = PreparedShell::plain("true");
        prepared
            .env_delta
            .insert("FUTURE_TEST_SET".into(), Some("a b".into()));
        prepared.env_delta.insert("FUTURE_TEST_REMOVE".into(), None);
        let command = prepared.into_command().unwrap();
        let env: Vec<_> = command.as_std().get_envs().collect();
        assert!(env.iter().any(|(key, value)| {
            *key == "FUTURE_TEST_SET" && value.and_then(|v| v.to_str()) == Some("a b")
        }));
        assert!(env
            .iter()
            .any(|(key, value)| *key == "FUTURE_TEST_REMOVE" && value.is_none()));
    }
}
