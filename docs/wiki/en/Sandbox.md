# Approvals and sandboxing

FutureOS can ask before file access and restrict shell commands, but **the desktop
starts in Unrestricted (`off`) mode**. Choose **Settings → General → Approval
mode**, or the shield in the message composer, before giving an untrusted task
access to your files. Availability is checked on the machine running the agent.

## Modes and defaults

| Mode | Behavior |
|---|---|
| **Unrestricted** (`off`, desktop default) | No approval rules or OS sandbox. Commands run with your user privileges. |
| **Manual** (`manual`) | Path rules decide Allow/Ask/Deny; ordinary reads and workspace/temp writes can run without asking. Sensitive paths and external writes may ask or be denied. Shell commands ask except for the read-only allowlist. |
| **Sandboxed** (`sandbox`) | File tools follow the same path rules. Shell commands run in the available OS backend, usually without a pre-execution prompt. |

Fresh agent sessions use legacy permission level `all`. TUI, CLI and channel
clients do not independently enable the desktop sandbox policy for fresh sessions;
continuing an existing session can retain its policy. `future run --permission`
is a separate setting: `all` is unrestricted, `workspace` enables approval gating,
and `none` denies **all** tool calls. It does not select an OS sandbox.

## Platform support

| Platform | Backend | Important limits |
|---|---|---|
| macOS | Seatbelt (`sandbox-exec`) | Filesystem rules; network remains open. |
| Native Linux | System Bubblewrap ≥ 0.9.0 | Snapshot-based filesystem mounts; network open, no seccomp filter. WSL is outside the supported validation scope. |
| Windows | Unelevated restricted token + filesystem ACLs | Write protection only; shell reads/network remain open. Existing ACLs and parent deletion rights can weaken protection. |

Linux protects existing matching paths at command launch. A protected name that
does not exist yet, or a new glob match created during a command, may only be
detected afterward. Detection does not block creation, roll back changes or prove
which process created a file. Complex overlapping rules may fail to prepare.
Platform availability does not mean every distribution/architecture has completed
release and independent security validation.

## Linux setup and diagnostics

Install Bubblewrap from your distribution's trusted package source:

```bash
# Debian / Ubuntu
sudo apt update
sudo apt install bubblewrap
# Fedora: sudo dnf install bubblewrap
future agent --probe-sandbox
future doctor
```

FutureOS does not bundle or download Bubblewrap. An older distribution package
may be below 0.9.0; use a supported, trusted upgrade rather than an arbitrary
binary. The version floor is compatibility, not a guarantee of all upstream
security fixes. Completely quit and restart FutureOS after installing or fixing it.

The Linux option stays visible but disabled while checking or unavailable.
Settings shows the reason, a suggested action and a diagnostic code:

| Code | What to check |
|---|---|
| `binary_missing` | Install the system package and make its directory available on PATH. |
| `path_rejected` / `binary_invalid` | Use a trusted, executable, root-owned system binary, not a workspace copy. |
| `version_too_old` / `version_unreadable` / `required_feature_missing` | Upgrade or repair the distribution package. |
| `user_namespace_disabled` / `proc_mount_restricted` | Ask the administrator whether host/container policy supports the required namespaces/mounts. Do not bypass organizational security policy. |
| `probe_timeout` / `probe_failed` | Inspect `future doctor` and local logs. |
| `probe_transport_error` | Restore the app's connection to the agent; this is not permanent platform incompatibility. |

A definite unavailable result falls back to Manual; a transient connection error
preserves the setting. If preparing an actual command fails, that call fails:
it is not silently rerun unrestricted. The agent may request explicit approval
for a necessary alternative.

## Approval cards and saved rules

A card can offer **Allow once**, **Deny**, or a saved path rule for this workspace
or chat. Review the actual command/path and scope. Cmd/Ctrl+Enter approves;
Esc denies, or closes an open rule editor first. Pending approvals have no timeout.

Rules are stored in `~/.future/approval_rule.json` and the workspace's
`.future/approval_rule.json`. Built-in protections precede user rules. Ordinary
reads are not all intercepted; allowing a rule does not make its contents trusted.

On macOS/Linux, escalation approves the **whole command outside the sandbox once**,
not just one path. A retry can repeat earlier side effects. Windows instead asks
for explicit write access to an existing file or directory subtree and keeps the
command restricted; creating/replacing files may require approval of the parent
directory subtree.

## What this does not guarantee

Sandboxing is not a network filter, an encrypted vault, a rollback system or a
complete prompt-injection defense. Agent `auth.json` currently has a hard-deny
exception for CLI-based skills and may be readable by arbitrary shell commands;
do not assume the sandbox isolates all credentials. Saved rules and Unrestricted
mode allow actions without another prompt. Inspect outputs and changes, and keep
secrets out of untrusted tasks.

See [[Using FutureOS|Using-FutureOS]], [[Settings]], [[CLI]] and [[FAQ]]. The
repository's SECURITY.md and platform design documents describe further boundaries
and candidate-specific validation evidence.
