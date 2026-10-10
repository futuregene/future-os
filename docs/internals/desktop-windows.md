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
  (`future-desktop.exe`), CLI/agent (`future.exe`), and retired standalone Agent
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
- Explicit in-app updates (`/UPDATE`) close exact-path executables automatically.
  This lets a new installer repair older releases that launched NSIS without
  first stopping their Agent; unrelated installations are never touched.
- After the preflight succeeds, remove both current and retired sidecar names
  before Tauri copies the new desktop. Before replacement, preserve the current
  and legacy executables in NSIS temporary storage. A copy/extraction failure
  restores that exact set; a failed first install removes partial executables.
- After copying, run bounded `futureos.exe --help` and `future.exe --version`
  probes. They return before GUI/Agent initialization while still exercising the
  Windows loader. A missing DLL, corrupt file, wrong architecture, non-zero exit
  or timeout aborts installation and restores the previous executables.

The in-app Windows updater downloads and verifies the complete NSIS package
before it stops the Desktop-owned Agent. It then launches the installer, because
Tauri exits the desktop process directly on Windows and does not emit the normal
`RunEvent::Exit` cleanup path. Keep this ordering aligned with the fail-before-copy
installer preflight: downloads must not interrupt work, while installation must
never begin with the bundled sidecar still running. The updater's
`on_before_exit` hook runs only after download, signature verification, and
package extraction succeed, immediately before NSIS launch. Update checks have a
30-second deadline and installation requests have a 15-minute deadline. Portable
Windows builds do not run the NSIS in-place updater because they have no
registered installation target.

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
