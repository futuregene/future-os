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

A release Desktop rejects attachment when its configured Future platform is
non-production. It does not rewrite authentication or remove keys while an
external Agent owns the data root. Exit that Agent independently, then restart
Desktop so the local owner can apply the existing production policy. Development
builds retain an explicitly selected environment. Direct account, login and
skill-guide requests apply the same check even if Agent startup failed. A stopping Agent also blocks
attachment and local startup; wait for it to finish before reconnecting.

CLI/TUI clients must use the same user, FUTURE_HOME and endpoint settings to
connect to an embedded Agent. For an independently managed service, exit Desktop,
start `future agent` separately, then reopen Desktop. Changing ports cannot
bypass the instance lock for the same data root.

CLI, TUI, Desktop and Channels share the business connection gate. RPC protocol 2
requires complete version/protocol/storage and instance/data-root metadata on
commands and subscriptions. Readiness is discoverable first, but unverified
clients cannot execute business calls. Upgrade old clients together; mismatch
never authorizes another service. Database schema and file formats are unchanged.

On Unix, `<socket>.owner.lock` is an endpoint lease separate from the Agent data
lock. Its continued existence is normal: ownership is determined by the OS lock.
Do not delete it. Live or unclassifiable endpoints are preserved; only verified
stale sockets can be reclaimed. Listener cleanup removes only its own endpoint.

## Exit, restart and updates

- Exiting an embedded owner stops every client's active, queued and approval
  work, including CLI/TUI conversations. GUI prompts when work exists or activity
  cannot be established. Confirmed force quit performs cleanup before exit;
  other clients observe cancellation or disconnection.
- Attached external Agents survive. Desktop cancels its own submitted runs and
  closes its remote entry; other clients and the service continue running.
  Queued work and work whose status cannot be confirmed retain ownership until
  a terminal event or verified terminal state, so collector failures cannot
  silently omit them from exit cancellation.
- External cancellation must verify that exact run's terminal state. Timeout,
  disconnect or another run's terminal is not success. Unconfirmed runs preserve
  diagnostics and block update, reset and restart. Forcing Desktop to exit cannot
  guarantee cancellation on an unreachable external service.
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

Exit, updates, reset and restart share one stop operation. Later requests neither
repeat cleanup nor extend its deadline. Cleanup runs in the background while the
main window remains responsive and displays stages; headless reports stages to
stderr. The stop result has a twenty-second overall deadline. Timeout reports
failure and continues blocking installation/in-process restart; it does not
force-kill writers or unlock early. Normal exit offers a process-exit decision
after failure; explicitly forced quit logs failure and exits. Window close does
not interrupt an active maintenance handoff to the installer.

Exit can cancel startup during legacy-session import. Committed sessions survive,
unfinished transactions roll back, and incomplete reads are not recorded as
corrupt sources or a completed import batch. The next start resumes import.
Uninterruptible system I/O remains subject to the safety barrier; never delete
WAL or lock files to accelerate exit.

## Troubleshooting and logs

The default data root is `~/.future`; a valid absolute FUTURE_HOME replaces the
whole root. Embedded host logs are at `<root>/app/logs/desktop.log`. Independent
CLI Agent file logs remain at `<root>/agent/logs/agent.log` when enabled. Windows
update recovery uses `<root>/app/logs/update-recovery.log`.

Desktop's `app/app.db`, `app/desktop.lock`, GUI single-instance scope, chat scratch
workspaces, pairing and device identity follow this same root. The default paths
and storage formats are unchanged. Earlier versions could leave Desktop data in
the default root while Agent data used an override. Those files are left intact:
this correction does not copy, merge or move them. Select the default root to
access its Desktop data; an override selects its own isolated Desktop state.

| Symptom | Action |
| --- | --- |
| External version incompatibility | Check the reported version/protocol; exit or upgrade the external service, then reopen Desktop |
| Held lock, unavailable service | Identify GUI/headless/independent owners of this data root; exit normally or reboot if necessary; never delete the lock |
| Explicit endpoint unreachable | Check its address, service and user environment; remove an unwanted explicit override before reopening |
| CLI missing but Agent works | Restore a matching CLI beside Desktop or on absolute PATH; skills/maintenance require it, embedded hosting does not require another Agent process |
| Cleanup failure prevents update | Preserve diagnostics; exit related clients and retry; if persistent, reboot and use the full installer |
| Installation failure without automatic reopen | Inspect recovery/installer logs and repair completely; do not launch mixed binaries or an incomplete rollback |

Update settings show the selected CLI path and version. Desktop probes
`future version --json` without initializing storage and validates the Agent
contract separately from tool capabilities. Different development commit
suffixes need not be incompatible. For missing, incompatible or capability-poor
tools, correct that path or restore a matching CLI; another Agent is not a fix.

Update phases and outcomes live at
`<data-root>/app/updates/<operation-id>/result.json`; `latest.json` selects the
latest operation. Windows observer readiness confirms handoff only. The durable
outcome determines installation success. The shutdown screen also displays the
result and permits restart only after installation or verified restoration,
subject to the cleanup gates. Unknown outcomes and incomplete rollback prevent
automatic retry or relaunch. Inspect the records and recovery log, then repair
with a trusted complete installer. After manual repair, explicit confirmation
in update settings preserves the original failure rather than claiming verified
success; an active observer still blocks acknowledgement. Retry downloads,
verifies signatures and cleans up again instead of executing leftover staging.

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

All three desktop launchers recover stale sccache failures in Cargo compiler
metadata before building. Cargo can store a failed `sccache rustc -vV` in
`<target>/.rustc_info.json` and replay that failure after sccache is healthy again.
CLI and Desktop normally have separate target directories, so one can keep
failing while the other builds successfully. The launcher checks both default
directories, or the explicit CARGO_TARGET_DIR, for failed probes with an explicit
`sccache: error:` diagnostic. It renames only that metadata file to a sibling
`.sccache-failure-<id>.bak`, preserving the original error, and lets Cargo regenerate
the metadata. Recovery paths and diagnostics are logged to
`.logs/rust-cache-preflight.log`.

This does not disable sccache, clear compilation caches, stop shared servers or
change global configuration. Healthy metadata, unrelated compiler errors and
custom wrappers remain intact. An ongoing permission/service failure still
requires fixing the actual execution environment; the launcher does not conceal
it by compiling without sccache. When invoking Cargo from an agent's restricted
sandbox, grant the build the permissions needed to access the configured cache
and local cache service. The initial historical permission failure may no longer
be reproducible even while Cargo continues replaying its saved result.

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
