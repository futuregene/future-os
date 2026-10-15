# Desktop Agent lifecycle and recovery

[中文](desktop-agent.zh-CN.md)

GUI and `futureos-headless` run the complete Agent in their own process by default.
CLI, TUI, mobile and other clients retain the same RPC interface and existing
configuration, database, session and credential formats. The packaged `future`
CLI remains available for skills, maintenance and independent `future agent`
use; it is no longer the process required to start Desktop's default Agent.

## Startup and external services

Desktop checks external Agent identity, compatibility and readiness for the
current user, data root and endpoint first. It attaches to a compatible service.
Only an absent default endpoint and an acquired instance lock authorize embedded
startup. Incompatible versions/protocol/storage contracts or an unavailable
lock owner produce an error without starting another Agent, deleting its lock
or opening a second business database.

Explicit `FUTURE_AGENT_GRPC_ADDR` connections and
`FUTURE_DESKTOP_AGENT_MODE=external` are attach-only. Failure never authorizes
takeover. For development, run `future agent` in another terminal before starting
Desktop in external mode. Default `auto` supports both compatible external and
embedded services; there is no force-embedded bypass of an external owner.

CLI/TUI clients must use the same user, FUTURE_HOME and endpoint settings to
connect to an embedded Agent. For an independently managed service, exit Desktop,
start `future agent` separately, then reopen Desktop. Changing ports cannot
bypass the instance lock for the same data root.

## Exit, restart and updates

- Exiting an embedded owner stops every client's active, queued and approval
  work, including CLI/TUI conversations. GUI prompts when work exists or activity
  cannot be established. Confirmed force quit performs cleanup before exit;
  other clients observe cancellation or disconnection.
- Attached external Agents survive. Desktop cancels its own submitted runs and
  closes its remote entry; other clients and the service continue running.
- Headless Ctrl+C and Unix SIGTERM/SIGHUP perform bounded shutdown. Credentials
  and completed pairing remain available for the next launch.
- Updates download, verify signatures and confirm interruption before full
  cleanup. Failed cleanup forbids installation and another Agent in the same
  process. Restart likewise requires the previous service to finish cleanup.
- OS force termination, crashes and development-tool interruption can bypass
  coordinated shutdown. Existing transaction recovery and permission GC apply
  on startup. Never delete databases, WAL, lock files or capability metadata as
  a startup repair.

On Windows an independent observer waits for Desktop exit before running the
installer. Installation failure restarts the old program only after complete
restoration; incomplete rollback preserves diagnostics for repair with a trusted
full installer. Replace portable files as a complete set to avoid mixing CLI and
Desktop versions. See [Windows installer preflight](../internals/desktop-windows.md).

## Troubleshooting and logs

The default data root is `~/.future`; a valid absolute FUTURE_HOME replaces the
whole root. Embedded host logs are at `<root>/app/logs/desktop.log`. Independent
CLI Agent file logs remain at `<root>/agent/logs/agent.log` when enabled. Windows
update recovery uses `<root>/app/logs/update-recovery.log`.

| Symptom | Action |
| --- | --- |
| External version incompatibility | Check the reported version/protocol; exit or upgrade the external service, then reopen Desktop |
| Held lock, unavailable service | Identify GUI/headless/independent owners of this data root; exit normally or reboot if necessary; never delete the lock |
| Explicit endpoint unreachable | Check its address, service and user environment; remove an unwanted explicit override before reopening |
| CLI missing but Agent works | Restore a matching CLI beside Desktop or on absolute PATH; skills/maintenance require it, embedded hosting does not require another Agent process |
| Cleanup failure prevents update | Preserve diagnostics; exit related clients and retry; if persistent, reboot and use the full installer |
| Installation failure without automatic reopen | Inspect recovery/installer logs and repair completely; do not launch mixed binaries or an incomplete rollback |

Report version, platform, service mode, error and relevant log excerpts. Remove
API keys and pairing links. `future doctor` helps diagnose the environment but
never authorizes bypassing instance ownership.

## Development entry points

Use `scripts/dev/start-desktop-macos.sh` or `scripts/dev/start-desktop-linux.sh`;
on Windows use `scripts/dev/start-desktop-windows.bat`, delegating to PowerShell.
These build/stage CLI tools and start Tauri; Desktop selects its service. The
iOS/Android launchers manage mobile development only, without a Desktop Agent.

`DRY_RUN=1` prints configuration only; `BUILD_CLI=0` skips CLI compilation;
`RUN_CHECKS=1` runs static checks without unit tests. DESKTOP_DEV_PORT selects the
Vite port: an occupied port fails without choosing another or killing its owner.
Relative CARGO_TARGET_DIR values resolve against the repository root and are
exported as absolute paths.

On macOS/Linux, the launcher checks `sccache` selected through `RUSTC_WRAPPER`,
`RUSTC_WORKSPACE_WRAPPER` or their `CARGO_BUILD_*` environment aliases before
building. A failed/missing cache or a probe exceeding 10 seconds is bypassed for
this launch only if direct rustc succeeds. CLI, optional Cargo checks and Tauri
inherit the bypass. Healthy caches and other wrappers remain enabled. Original
diagnostics are saved to `.logs/rust-cache-preflight.log`; no cache files, shared
servers or global configuration are changed. Real compiler/build failures retain
the existing error behavior; the launcher never retries an entire Desktop session.
This preflight does not parse Cargo TOML configuration or recover cache failures
that begin after the probe. For a wrapper configured only in Cargo TOML, run
`RUSTC_WRAPPER='' RUSTC_WORKSPACE_WRAPPER='' ./scripts/dev/start-desktop-macos.sh`
(use the Linux entry point there) to bypass wrappers explicitly for one launch.
The Windows launcher does not currently perform this cache preflight.

Quit from the app for coordinated cleanup. Development-script Ctrl+C interrupts
the toolchain. Unix cleanup targets only this launch's process group and can
force termination after its deadline; it does not guarantee normal app cleanup,
scan/stop external Agents or edit databases/locks. The actual headless binary
handles Ctrl+C directly; see [headless mode](desktop-headless.md).

## Change notes and acceptance

Desktop's default host changes from a child process to a complete embedded
service. External connections, independent CLI/TUI, tool capabilities and storage
formats remain. Exit now interrupts other clients of an embedded owner too;
users should understand that before upgrading.

Three-platform native lifecycle CI, offline Windows update fixtures, macOS ad-hoc
packaging and Linux/Windows daily unsigned package checks have passing records.
Release signing/notarization, real installation/upgrades/rollback, complete GUI
tool interaction, native Linux Bubblewrap and performance comparisons remain
separate acceptance. Code cleanup is not release approval. Evidence and open
checks are in the [development plan](../architecture/embedded-agent-development.md).
