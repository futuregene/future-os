# Security Policy

FutureOS is a local-first AI agent. This document describes its trust model,
current controls and limitations, and vulnerability reporting.

## Trust model and data flow

- **Local persistence, not offline-only processing.** Sessions and configuration
  are stored locally under `~/.future/`; loop state is project-local by default.
  Prompts, selected attachments and tool results included in context are sent to
  the configured model provider. FutureOS-hosted models and online tools use
  FutureOS services; other providers/tools use their respective endpoints.
- **Optional remote surfaces.** Enabling Remote sends commands, conversation events
  and requested files through the configured NATS relay. Mobile requires TLS
  WebSocket (`wss://`); desktop-to-NATS transport follows deployment configuration
  and is not unconditionally TLS-enforced. Feishu/DingTalk messages and replies
  also pass through those platforms. Local storage does not imply end-to-end
  encryption or that no data leaves the device.
- **Per-user local backend.** The agent defaults to Unix-domain sockets on
  macOS/Linux (private directory and peer-UID checks) or a current-user-only
  Windows named pipe. Unix honors `FUTURE_AGENT_SOCKET`; Linux otherwise uses
  `$XDG_RUNTIME_DIR/future/agent.sock` when set, with
  `~/.future/run/agent.sock` as fallback/macOS default. `--grpc-addr` explicitly
  enables TCP. Do not expose the agent's plain TCP service to an untrusted network;
  use an authenticated secure tunnel if remote access is needed.
- **Credentials.** Provider keys are stored locally, normally in
  `~/.future/agent/auth.json`; the legacy `agent-app/auth.json` location is also
  read. Provider configuration can contain keys too. Treat these files, backups,
  logs and `future auth credential` output as sensitive. Local storage is not an
  encrypted credential vault.

## Tool execution safety

The core tools are `read`, `write`, `edit` and `shell`.

**Protection is configurable, not enabled for every call by default.** Fresh
agent sessions default to permission level `all`; the desktop defaults to
Unrestricted (`off`). TUI/CLI/channel clients do not independently enable a
sandbox policy for fresh sessions. Continuing an existing session may retain its
policy. An enum's `manual` default is not the application's effective default.

Desktop Settings → General, or the composer shield, selects:

- `off` — no approval rules or OS sandbox.
- `manual` — path rules decide Allow/Ask/Deny. Ordinary reads and workspace/temp
  writes can be allowed without a prompt; sensitive paths and external writes
  can ask or be denied. Shell commands ask unless matched by the read-only
  allowlist. Approved shell commands run with the current user's privileges.
- `sandbox` — path rules remain active; shell commands use an available OS
  backend. They do not all ask before execution. macOS uses Seatbelt, native
  Linux uses system Bubblewrap, Windows uses unelevated restricted-token write
  protection. A definitive unavailable probe falls back to manual approval;
  initialization failure for a real sandboxed command fails that call rather
  than silently running it unrestricted.

Separate legacy permission levels are `all` (unrestricted), `workspace`
(approval-gated access), and `none` (deny all tool calls). They are not aliases
for the three sandbox tiers. See the [sandbox guide](docs/wiki/en/Sandbox.md).

### Known boundaries

- **Network is open.** These backends do not provide domain/network filtering.
  Linux currently has no seccomp filter. Neither shell allowlisting nor a sandbox
  proves a command safe, prevents all exfiltration, or undoes side effects.
- **Linux snapshot limits.** Requires trusted system Bubblewrap ≥ 0.9.0 and usable
  user namespaces. Existing protected paths are mounted with restrictions;
  missing protected paths and new glob matches during a command are only checked
  afterward, not dynamically blocked or rolled back. Complex overlapping rules
  can fail preparation. Implementation availability is not certification of all
  distributions/architectures; see [Linux boundaries and validation](desktop/DEV_MD/SANDBOX/LINUX.md).
- **Windows write protection only.** Shell reads/network remain open. Existing
  ACLs and parent-directory deletion rights can weaken the write boundary.
  Additional access is approved for concrete file/subtree write capabilities;
  Windows does not use macOS/Linux whole-command unsandboxing. See
  [Windows boundaries](desktop/DEV_MD/SANDBOX/WINDOWS.md).
- **Credential exception.** `auth.json` is currently omitted from the sandbox's
  built-in hard-deny list so official CLI-based skills can authenticate. This can
  also expose it to arbitrary shell reads; it is not per-binary trust or a secure
  credential channel. Other write rules still apply; `models.json` remains denied
  by the built-in policy. Do not assume sandboxing isolates all agent credentials.
- **Approvals have scope.** macOS/Linux escalation authorizes the whole command
  outside the sandbox once. Review it for unrelated actions and repeat side
  effects. Saved rules authorize future matching actions without another prompt.
- **Same-user trust boundary.** These controls do not defend against an attacker
  already controlling the agent process or the user's host account.

## Prompt injection, skills and channels

Untrusted pages, documents, skills and tool results may try to steer the model.
Approvals and sandboxing reduce risk when enabled; they do not guarantee that
injected instructions cannot act. Review tool activity, narrow permissions and
avoid exposing secrets to untrusted tasks.

Skills ship from [future-skills](https://github.com/futuregene/future-skills) and
execute through the session's tool permissions; install only skills you trust.
Channel conversations can drive the local agent with their configured permissions
(`all` by default). Restrict bot membership/allowlists and credentials as carefully
as shell access; do not assume the desktop's approval setting applies to every
client or fresh session.

## Supported versions

Security fixes are applied to the latest release on `main`. We do not maintain
patched branches for older versions; stay on the newest release.

## Reporting a vulnerability

**Do not open a public GitHub issue for security reports.** Use
[GitHub Security Advisories](https://github.com/futuregene/future-os/security/advisories/new).

We commit to acknowledging reports within **72 hours**, keeping you informed as
we investigate and remediate, and crediting reporters unless they prefer anonymity.

Include the affected version/commit, platform, issue and impact, and reproduction
steps or a proof of concept if available. Do not include live credentials.
