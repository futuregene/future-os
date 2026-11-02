# Data backup and recovery

[中文](data-backup.zh-CN.md)

`future data` is the offline, coordinated backup and recovery path for a local
FutureOS home. It copies the Agent/Desktop SQLite databases and optional Fusion database and the managed file trees that
carry conversations, attachments, review repositories and Agent credentials,
verifies every copied byte with SHA-256, and can put that set back at the same
path. It is not a live backup: it refuses to run while a Desktop backend or an
Agent owns the home, and it never stops a process for you.

`<home>` below means the FutureOS home — normally `~/.future`
(`%USERPROFILE%\.future\` on Windows), or the absolute directory named by
`FUTURE_HOME`. The commands act on whichever home the current process resolves,
so the environment that runs them must match the environment of the process you
stopped. There is no `--home` flag. [Directory layout](directory-layout.md) maps
everything else that lives there.

## Commands

| Command | Arguments | What it does |
| --- | --- | --- |
| `future data backup <new-directory>` | exactly one path | Takes a verified backup into a directory that must not exist yet. |
| `future data verify <backup-directory>` | exactly one path | Re-checks an existing backup. Read-only; no locks, no Agent. |
| `future data restore <backup-directory> --yes` | path and the literal `--yes` | Restores the backup to the home it was taken from. |
| `future data resume` | none | Finishes an interrupted restore switch. |
| `future data rollback` | none | Puts the pre-restore files back after an interrupted restore. |
| `future data --help`, `-h` | none | Prints the same usage text. |

There are no other flags: any other argument combination fails with
`Invalid data command.` and the usage text, and the process exits with status 1.
`--yes` is not a prompt bypass — it is the required second argument; without it
the command is invalid.

## What a backup contains

The backup is a closed set, not a mirror of the home. These roots are all it
collects; anything outside them is simply not in the backup.

| Path under `<home>` | Contents |
| --- | --- |
| `agent/agent.db` | Agent store: sessions, runs, events, skill inventory (SQLite snapshot). |
| `app/app.db` | Desktop store: workspaces, threads, runs, approvals (SQLite snapshot). |
| `fusion/fusion.db`, `fusion/snapshots` | Consultation metadata (SQLite snapshot) and immutable file evidence. Full child transcripts are in `agent/agent.db`. |
| `app/images` | Per-thread image tree (`thumb/`, `origin/`). |
| `app/review` | Per-workspace shadow git review repositories. |
| `workspaces/chat` | Per-thread chat workspaces. |
| `agent/sessions`, `agent/run-events` | Retained legacy JSONL migration sources. |
| `agent/session-backups` | Pre-reset history snapshots written by `future session reset`. |
| `agent/settings.json`, `agent/models.json`, `agent/auth.json` | Agent configuration, including provider credentials. |

All present databases are copied as standalone snapshots, not as files plus WAL: each
is opened read-only and dumped through SQLite's snapshot API while a
`BEGIN IMMEDIATE` transaction on the live database blocks other writers, so the
snapshots and the managed-file inventory cannot straddle a commit. A
database's `-wal`/`-shm` sidecars are never copied and are rejected if they
appear next to a database inside a backup.

Managed files are copied byte for byte, written owner-only (`0600`), then
re-hashed; a file that changes during the copy aborts the backup. Symbolic links
and special files are refused rather than followed. The manifest
`manifest.json` at the backup root records the format, id, source home, the
directory list and one `{path, bytes, sha256}` record per file.

**A backup contains credentials.** `agent/auth.json` is in it, and the archive is
plain files, not encrypted. Store it on encrypted media and keep it private.

## What is not included

Everything else under the home, for example:

- `channels/config.json` and per-channel state, TUI settings, keybindings and
  logs, the scheduled-task ledger `tasks/tasks.db`;
- `remote_pairings.json`, `remote_pairing.json`, `remote_peers.json`,
  `app/remote-previews/`, `app/relay-root-certs.pem`;
- installed skills (`agent/skills/`), browser state (`agent/browser/`), agent
  logs, `run/`;
- user workspace directories outside `workspaces/chat` — a user-selected project
  directory is never copied.

Those are configuration or rebuildable state. Copy them separately if a recovery
needs them.

## Before you start: stop every writer

The command takes the same two locks the processes themselves hold, and waits
for neither:

- `<home>/app/desktop.lock` — held for the whole lifetime of the Desktop backend,
  whether that is the GUI or a foreground `future host`.
- `<home>/agent/agent-instance.lock` — held by the Agent. When Desktop or
  `future host` runs the embedded Agent, stopping that process releases both;
  a standalone `future agent` must be stopped as well. A TUI never holds it:
  the TUI only attaches to an Agent and never starts one.

So: quit the Desktop app and stop any `future host`/`future agent` in a
terminal (TUIs need no stopping — they hold no Agent). A held lock is reported
for what it is — `Desktop is running; quit it before data maintenance` or
`Agent is running; stop it before data maintenance`.

The same check distinguishes a process from a file system that cannot lock at
all (some network shares and FUSE mounts). That case says so and tells you to
move the home to a local disk; quitting a process will not help.

`future data verify` is the exception: it only reads a backup directory and does
not touch the home or its locks, so it runs while everything is live.

## Take a backup

```sh
future data backup /absolute/path/outside/the/home/backup-2026-10-23
```

- The destination **must not exist**. Backups are never overwritten; a retry
  needs a new directory. Its parent is created if necessary.
- The destination must be **outside the home**. The check compares canonicalized
  paths, so a symlink back into the home is caught.
- The backup is staged in a `.future-backup-*` directory beside the destination,
  verified there, and published by a single rename — a failed run leaves no
  half-written backup at the destination.
- An existing incomplete restore blocks a new backup; resolve it first (below).
- Success prints `Verified backup: <destination>`.

A backup is a full copy every time; there is no incremental mode. Size the
destination for the managed trees plus both databases.

## Verify a backup

```sh
future data verify /absolute/path/outside/the/home/backup-2026-10-23
```

Verification reads `manifest.json` and fails unless:

- the format marker and version are current;
- every listed file exists with the recorded size and SHA-256, and no listed
  directory is missing;
- the backup contains no file or directory the manifest does not list — an
  omitted record must not turn present history into absent data;
- no `-wal`/`-shm`/`-journal` sidecar sits next to either database;
- both databases pass `PRAGMA quick_check`, the Desktop database header
  validates, and the Agent database reports the expected `application_id` and a
  supported schema version.

Success prints `Verified backup <id>: <N> files`. Keep the backup directory
intact: adding, removing or opening files in it (which can create sidecars)
invalidates it, and the next verify or restore will refuse it.

## Restore to the same home

```sh
future data restore /absolute/path/outside/the/home/backup-2026-10-23 --yes
```

Stop the writers first, exactly as for `backup`. Then the command:

1. verifies the backup, and refuses unless the home recorded in the manifest is
   the same canonical path as the current home — a restore cannot relocate
   history, because saved working-directory paths and workspace identities keep
   their original meaning;
2. stages a copy of the whole set into `<home>/.restore-work-<id>/incoming/`,
   re-verifies it there, and rotates the restore generation marker in both
   databases (message, run and session ids are untouched);
3. writes the journal and the fence `<home>/.restore-incomplete.json`;
4. switches each managed root by rename: the current copy moves to
   `<home>/.restore-work-<id>/previous/`, the restored copy moves into place. A
   previous WAL/SHM file moves aside with its database so it cannot override the
   restored one;
5. verifies the installed file set, then clears the fence by renaming it into
   the work directory.

Only the enumerated managed roots move. Instance locks, skill directories,
channel/TUI configuration and other files in the home are left as they are, so
restoring an older backup does not roll back the rest of the home.

Success prints `Restore completed. Previous files are retained for rollback inspection.`
The pre-restore files stay under
`<home>/.restore-work-<id>/previous/`; nothing deletes that directory
automatically. Inspect it after a recovery, then remove it yourself when you no
longer need the old copy.

Because the restore replaces both databases, anything written after the backup
was taken is gone. Take a fresh backup before restoring if the current state
might matter.

## Interrupted restore: resume and rollback

A restore spans many renames and can be killed at any point — power loss, a
crashed terminal, a closed laptop. It is built to survive that:

- The fence `<home>/.restore-incomplete.json` is written **before** the first
  original moves, so a half-switched home is never mistaken for a healthy one.
- `future data resume` continues the switch from wherever it stopped, re-checking
  each remaining replacement before it moves another original.
- `future data rollback` undoes it: restored roots go back to `incoming/`, the
  retained originals go back to their home paths, and the rollback only
  completes after every preimage digest still matches.
- The direction is persisted in the journal, so `resume` after a rollback has
  started finishes the rollback instead of re-applying the backup.

Both commands need the same two locks — stop the writers first — and both are
safe to repeat: every rename boundary is resumable, and a repeated run continues
rather than starting over. `resume` or `rollback` without a pending restore fails
because there is no journal to finish. Success prints
`Recovery operation completed; startup is available again.`

Do not delete the work directory or edit the fence to make a stuck restore go
away. A missing or modified preimage is reported by path and deliberately keeps
the fence in place; deleting the fence only hides an inconsistent home.

## The startup fence

While `.restore-incomplete.json` exists in the home, every component that would
open the data refuses to start and names the fix (`future data resume` or
`future data rollback`):

- the Desktop backend, GUI or headless, and its store connections;
- the Agent on startup and when it opens its database, including the skills
  registry;
- clients that would otherwise start a local Agent.

The fence is not a per-command check that can be worked around by another entry
point: it is the home's own state. A successful `resume`/`rollback` clears it by
renaming the file into the restore work directory, so the operation is recorded
rather than erased.

## Troubleshooting

| Message | Meaning | Action |
| --- | --- | --- |
| `Desktop is running; quit it before data maintenance` | Something holds `app/desktop.lock`: the GUI, or a `future host` in another terminal or user session. | Quit it. If no window exists, find the foreground host process and stop it with Ctrl+C. |
| `Agent is running; stop it before data maintenance` | Something holds `agent/agent-instance.lock`. | Stop the Agent. `agent/agent-instance.json` names the pid, host, executable and home of the process that published the lock. |
| `this file system will not lock <path> …` | The home is on a share or mount without working file locks. | Move the home to a local disk; maintenance cannot verify that no writer is running there. |
| `backup destination already exists` | Backups are never overwritten. | Choose a new destination. |
| `place complete backups outside the FutureOS home` | The destination resolves inside the home. | Move it outside. |
| `restore must target the original FutureOS home; managed working-directory paths are retained` | The home path differs from the one in the manifest. | Restore on the machine and absolute path the backup came from; a relocated home needs a manual migration, not this command. |
| `A FutureOS data restore is incomplete. Run 'future data resume' or 'future data rollback' …` | The fence is present. | Finish or roll the restore back; nothing else will start. |
| `managed file changed during backup` | A writer was still active while files were being copied. | Stop every writer and take a new backup. |
| `backup inventory disagrees with its manifest`, `backup file verification failed`, `unexpected SQLite sidecar` | The backup directory was modified after it was written, or copied incompletely. | Re-copy it in full, or take a new backup; do not "repair" it in place. |
| `restore preimage changed or is missing: <path>; startup remains fenced` | The retained pre-restore copy of that root was altered or deleted. | Restore the preimage from your own copy if you have one, then `rollback`; otherwise the work directory is the only evidence of the previous state. |

## See also

- [Directory layout](directory-layout.md) — every path under the home.
- [Offline coordinated backup and restore](../architecture/sqlite-migration.md#offline-coordinated-backup-and-restore)
  — the storage-contract view, including migration snapshots under
  `app/backups/` and `agent/backups/`.
- [Desktop Agent lifecycle and recovery](desktop-agent.md) — who owns the home
  and its locks.
