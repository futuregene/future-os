# Security Policy

FutureOS is a local-first AI agent. This document describes the trust model,
the current state of our security controls (including honest gaps), and how to
report a vulnerability.

## Trust Model

- **Local-first.** All sessions, project data, and configuration live on your
  device under `~/.future/`. Nothing is uploaded to a FutureOS server.
- **Localhost-only backend.** The agent gRPC service binds to
  `127.0.0.1:50051`. It does not accept inbound network connections by
  default.
- **Credential storage.** API keys live in `~/.future/agent/auth.json` on your
  local disk. They are transmitted only to the corresponding model provider's
  endpoint — never anywhere else.

## Tool Execution Safety

The agent's tool set is deliberately minimal: `read`, `write`, `edit`,
`shell`.

- **Approval gating (default).** Every tool call requires your explicit
  approval before execution (`/approve` / `/reject` in the TUI, or the
  equivalent control in other clients). Nothing writes to your filesystem or
  executes a shell command silently unless you have explicitly configured an
  auto-approve policy.
- **Sandbox tiers.** Three levels: `off` / `manual` / `sandbox`.
  - `off` — no approval, no sandbox; everything runs.
  - `manual` — approval rules on; shell commands ask before running (default).
  - `sandbox` — approval rules on; shell commands run inside the OS sandbox
    where one is available: macOS Seatbelt, or the Windows unelevated
    restricted-token sandbox.
  - ⚠️ **Known gaps:** Linux has no OS-level sandbox yet and relies on approval
    gating alone. The Windows sandbox is a first version that enforces **write
    protection only** (shell reads and network remain open), so it is not yet
    equivalent to macOS Seatbelt. Closing these gaps is on the roadmap —
    contributions welcome.
- **Channel bridge.** The IM bridge (Feishu / DingTalk) connects the same
  local agent to chat platforms. Messages you send the bot can drive the same
  approval-gated tools; treat bot configuration (app credentials, allowed
  chats) with the same care as shell access.

## Scope Notes

- **Prompt injection.** Like every LLM agent, FutureOS cannot fully prevent a
  malicious web page, document, or tool result from attempting to steer the
  model. Approval gating is the mitigation: injected instructions still cannot
  act without your consent. Review approval prompts carefully, especially
  after the agent has browsed untrusted content.
- **Skills.** Built-in skills ship from the
  [future-skills](https://github.com/futuregene/future-skills) repository.
  Third-party skills execute with the same tool permissions as the agent
  itself — only install skills you trust.

## Supported Versions

Security fixes are applied to the latest release on `main`. We do not
maintain patched branches for older versions — please stay on the newest
release.

## Reporting a Vulnerability

**Please do not open a public GitHub issue for security reports.**

Preferred channel: [GitHub Security Advisories](https://github.com/futuregene/future-os/security/advisories/new)
(“Report a vulnerability” on the repository's Security tab).

We commit to:

- Acknowledging your report within **72 hours**
- Keeping you informed as we investigate and remediate
- Crediting reporters in release notes (unless you prefer anonymity)

When reporting, please include: affected version/commit, platform, a
description of the issue and its impact, and reproduction steps or a
proof-of-concept if available.
