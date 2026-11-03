# Windows installer preflight

## Visual C++ runtime

The repository's `.cargo/config.toml` enables `+crt-static` for
`x86_64-pc-windows-msvc`, embedding the CRT into the desktop and CLI/agent.
Both NSIS and portable builds use this configuration. This avoids requiring a
machine-wide Visual C++ Redistributable installation (and elevation) before
`future.exe` can start, probe the sandbox, or clean up sandbox permissions during
uninstall. Packaging builds must not override these flags with `RUSTFLAGS` or
`CARGO_ENCODED_RUSTFLAGS` that omit `+crt-static`.

For release validation on Windows, inspect **both** `future.exe` and
`futureos.exe` with `dumpbin /DEPENDENTS`: neither should import `VCRUNTIME140*.dll`
or `MSVCP140*.dll`. On a clean Windows machine without the VC++ Redistributable,
test installation, desktop/agent startup, sandbox probing, and uninstall.
Build success on a developer machine with the runtime installed is insufficient.

This changes newly built binaries only. An already installed release whose
uninstaller invokes its old dynamically linked `future.exe` may still need the
Microsoft runtime repaired before an uninstall-first upgrade can proceed.

## Process and file access checks

Tauri's default NSIS running-app check covers only the main executable. The
bundled `future.exe` can outlive the desktop and remain locked during an upgrade.
`installer-hooks.nsh` checks file access **before** Tauri replaces any executables,
and before the new uninstaller runs sandbox permission cleanup.

- Check the current desktop (`futureos.exe`), legacy desktop
  (`future-desktop.exe`), headless host (`futureos-headless.exe`), CLI/agent (`future.exe`), and retired standalone Agent
  (`future-agent.exe`) in `$INSTDIR`.
- Explain that closing the window does not necessarily stop the agent. Offer
  Yes (close this installation and continue), No (recheck after manual exit),
  or Cancel. Cancel is the default and exits without replacing executables.
- Closing requires interactive consent with a task/session interruption warning.
  Match full executable paths case-insensitively; close desktop processes first,
  then the agent. Never kill by process name alone or kill process trees.
- Probe existing files without truncating them, and create/delete a unique
  directory write probe. External locks, read-only files and permissions can
  block installation even when no matching process is found.
- A failed close offers Retry/Cancel with Task Manager and restart instructions.
  A write/check failure offers Retry/Cancel with permissions, disk space,
  security-software and default-directory guidance. There is no Ignore path.
- Generic silent (`/S`) and passive (`/P`) installs never close programs or
  display blocking dialogs: exit **32** for busy files, **5** for write/check
  failures. `scripts/release/install.ps1` translates these into recovery instructions
  rather than continuing to initialization.
- Legacy in-app updates (`/UPDATE`) close exact-path executables in this
  installation for older releases that left a sidecar running. New hosts pass
  `/FUTUREOS_CLEAN` and only check occupancy. External hosts in other directories
  block automatic updates too. Interactive Yes explicitly authorizes closing
  verified external owners using the same data root; PID, creation time,
  executable path and data root must all match.
- Under the Agent lock lease, the installer runs its newly bundled maintenance
  CLI to completely reset Windows sandbox permissions before backup/replacement.
  Cleanup failure/timeout aborts installation. Old installed CLIs need not
  support the maintenance command.
- After the preflight succeeds, remove both current and retired sidecar names
  before Tauri copies the new desktop. Before replacement, preserve the current
  and legacy executables in NSIS temporary storage. A copy/extraction failure
  restores that exact set; a failed first install removes partial executables.
- After copying, run bounded `futureos.exe --help` and `future.exe --version`
  probes. They return before GUI/Agent initialization while still exercising the
  Windows loader. A missing DLL, corrupt file, wrong architecture, non-zero exit
  or timeout aborts installation and restores the previous executables.

The updater downloads/verifies, confirms the interruption scope, then performs
fallible complete Desktop/Agent cleanup. Failure blocks installation. An
independent observer waits for the original Desktop process handle to exit,
launches `/P /R /UPDATE /FUTUREOS_CLEAN` and checks launch errors and exit codes.
Failure relaunches the old app only if all original executable hashes match and
no rollback-failure marker exists; otherwise diagnostics require repair.
Recovery logs live at `FUTURE_HOME/app/logs/update-recovery.log`. Check timeout
is thirty seconds, download timeout fifteen minutes, observer readiness ten
seconds and Desktop exit sixty seconds. Extraction is never killed on timeout.
Portable Windows builds still do not use the NSIS in-place updater.

The helper is embedded into NSIS's temporary plugin directory, not installed
with the app. Use native Windows PowerShell even from 32-bit NSIS so 64-bit
process paths can be inspected. If PowerShell cannot run, fail closed with
instructions. The normal current-user/unelevated install mode is unchanged.
Uninstall closes exact-path current and legacy processes, then verifies that
every current and retired executable can be deleted before Tauri removes the
uninstall record. Interactive removal offers Retry/Cancel; silent/passive removal
exits 32 on a lock rather than reporting success with files still installed.
Temporary executable backups also close the race between the access check and
deletion: a late deletion failure restores the pre-uninstall executable set.
This avoids `/REBOOTOK`, whose delayed-delete path requires privileges a
current-user installer does not have. Sandbox reset remains best-effort and
time-bounded, so a missing, old or broken CLI cannot hang or block removal.
Capability metadata stays outside `$INSTDIR` for a later install/repair retry.

## Offline regression

The phase-six offline hooks and update observer passed native Windows CI and
daily unsigned packaging on 2026-10-15. See [execution evidence in sections
10.10 and 10.13](../architecture/embedded-agent-development.md). Regressions use
throwaway installers and validate actual product payloads without installing
FutureOS; clean-VM real upgrades/rollback remain separate acceptance.

On Windows with NSIS installed (the Tauri NSIS cache is also detected):

```powershell
& ./scripts/tests/test-windows-installer-preflight.ps1
& ./scripts/tests/test-install.ps1
```

The test compiles the actual hooks with NSIS `/WX`, creates harmless temporary
executables, and drives only the test installer's dialogs by PID. It covers
fresh installation, English/Chinese guidance, Cancel/No/Yes, silent/passive
failure, automatic-update recovery from a running mixed install, exact-path
closing (including both legacy executable names), external locks, retry,
read-only files, uninstall with a running Agent, arbitrary cleanup failure, a
pre-sandbox mixed CLI, hung cleanup, a missing CLI, fail-closed locked uninstall,
post-copy load verification, and rollback after copy/verification failures. It
does not launch, stop, or modify the developer's real installation.
The Windows packaging workflow runs these checks before publishing artifacts.

Before releasing, also smoke-test the **full packaged installer** over the
previous release, including both reinstall and uninstall-first upgrade choices.
A previous release's uninstaller contains its original hooks, not the new ones.
No preflight can prevent another application from acquiring a lock after the
check; avoid reopening FutureOS until installation finishes.

## A windowless launch has no writable stderr

A process started by Explorer, an installer's "run now", or an auto-start entry
has no console window, and fd 2 is either absent or names a handle the process
may not write. `eprintln!` does not merely lose that line:
`std::io::stdio::print_to` panics on the failed write, which ends the task that
emitted it.

Measured 2026-11-01 in `app/logs/crash.log` of one `futureos.exe` process:
**699** records of `panicked ... failed printing to stderr: 拒绝访问 (os error 5)`,
all on `tokio-rt-worker`, one exactly **11.1 s** after every relay
`authorization violation` the remote supervisor reported. Two consequences, both
observed that day: the reason a remote desktop stopped was nowhere on disk, and a
panic could kill the task that would have gone on to rebuild the link.

Two layers answer it, and both are needed:

- **`future_agent::console::redirect_windowless_standard_streams`**, called first
  in `future_host::environment::prepare_process`: when `GetConsoleWindow()` is
  null, this launch's stdout and stderr are pointed at
  `~/.future/app/logs/desktop.stdio.log` (a fresh file past 4 MiB), so *any*
  `eprintln!` — std, a dependency, or code nobody audited — appends to a file
  instead of panicking. A launch from a terminal keeps its terminal.
- **the tolerant writers**, for a stream that dies *during* a run:
  `console::eprintln!` / `console::println!` drop a failed write,
  `logfile::eprintln_log!` also mirrors into `desktop.log`, and the remote stack
  reports through `future_remote_protocol::diagnostics`, which the host installs
  onto `environment::report`. A terminal that goes away after startup cannot be
  detected at startup, so a long-lived write must never be a plain `eprintln!`.

Rule for new code in this tree: a diagnostic emitted by a long-lived task goes
through one of those three, and `grep -rn 'eprintln!'` finding your line means it
does not.

## Correction (2026-11-02): a console is not a writable stream

The redirect described above was gated on `GetConsoleWindow()`, and that gate was
wrong: it never fired in production. Evidence from one Desktop process
(`app/logs/desktop.prev.log`, build `bbfee50e`):

```
06:22:07.572Z host starting ... pid=30816 mode=auto          # launched by a scheduled task
06:22:08.780Z WARN terminal stream is unusable; ... error=拒绝访问 (os error 5)
06:22:08.936Z Agent host finished ... phase=Failed("Embedded Agent worker panicked")
```

- The launch **had** a console window — a hidden one, created by the task's
  PowerShell — while its stderr answered `os error 5` on every write. "Console
  present" and "stream writable" are different questions, so the redirect was
  skipped exactly where it was needed.
- `desktop.stdio.log` did not exist for any launch, which is the plainest proof
  the branch never ran: the redirect creates that file unconditionally.
- What followed: a 1.4-second panic, then the watchdog restarted the Agent every
  ten minutes for **thirteen hours** (84 recoveries, 06:22:26Z to 19:28:38Z),
  reaching no user. Nothing in the UI said the Agent was dead.

Three changes answer it, and the first is the one that makes the other two
bearable:

1. **A failed write must never end the task that wrote it.** `console::write_err`
   / `write_out` degrade: the text goes to the host log, one line records that
   output is unusable, and a write that fails there too is dropped. The Agent's
   own diagnostics no longer use `eprintln!`/`println!` at all (16 production
   sites moved to the tolerant macros), so the class of panic is gone rather than
   covered.
2. **The redirect asks whether a stream has a destination, not whether a console
   exists.** `redirect_windowless_standard_streams` moves stderr/stdout to
   `app/logs/desktop.stdio.log` for the streams that have nowhere else to go, and
   leaves a stream its caller captured exactly where the caller put it (a `NUL`
   device and a closed pipe answer the same question the same way; a captured pipe
   is somebody's reader, and moving it would break `futureos | tee`). The Desktop
   goes through it at process preparation, whatever console the launch inherited.
3. **Recovery has a terminal state.** `future_rpc::supervision::RestartBudget`
   allows three launches that never reach `Attached::Stable`; the third spends the
   budget for good and no cooldown, late wake-up or `reset()` re-arms it. The
   watchdog then stops, reports one line
   (`Agent recovery gave up after N/3 attempts: <cause>; automatic retries stopped,
   restart FutureOS`), sets `phase=restart_required` with the cause on the status
   page, and shows **one** dialog per episode whose button restarts the app. A
   headless host has no dialog: it logs and reports the status.

Acceptance for this work is not "it starts from a window". It is: launched **by a
scheduled task with a hidden window**, the process must create
`app/logs/desktop.stdio.log`, keep `crash.log`'s entry count unchanged, and say so
in the `host starting` line. The unit test that covers it spawns the child **with**
a console on purpose (`console::tests::a_captured_stream_is_left_where_its_caller_put_it`);
the earlier one used `CREATE_NO_WINDOW`, which is the branch that was already
covered and the one production never used.

### Temporary convention until this build is installed

On a machine still running the old build, the only stable way to start the Desktop
is **from a terminal that stays open** (measured: `WindowsTerminal` → `powershell` →
`futureos.exe`, alive since 19:21:44 while four launches before it died within a
second). Closing that terminal makes stderr unwritable again, which re-enters the
silent restart loop. This is a workaround, not a fix: install the build that
contains the changes above and the launch path stops mattering.

### Pitfall: read `origin/dev-huichen` before implementing, not after

This work was written against a base that had already moved. By the time it was
ready, another session had replaced the redirect gate with the destination-based
one above — which is strictly better, because it leaves a captured stream where
its caller put it — and had migrated the plain `eprintln!` sites itself. The
rebase then conflicted in exactly the files that mattered, and reconciling cost
more than reading `origin` would have: one `git show origin/dev-huichen:<file>`
before starting would have shown that the gate had changed, and the change would
have been written once instead of twice.

The rule this leaves: before touching a file this repository has recently been
busy with, read the remote copy first (`git show origin/<branch>:<path>`), not the
one in the working tree, and not after the code is written.

