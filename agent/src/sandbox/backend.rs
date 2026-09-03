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
        }
    }

    pub fn into_command(self) -> tokio::process::Command {
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
        command
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let command = prepared.into_command();
        let env: Vec<_> = command.as_std().get_envs().collect();
        assert!(env.iter().any(|(key, value)| {
            *key == "FUTURE_TEST_SET" && value.and_then(|v| v.to_str()) == Some("a b")
        }));
        assert!(env
            .iter()
            .any(|(key, value)| *key == "FUTURE_TEST_REMOVE" && value.is_none()));
    }
}
