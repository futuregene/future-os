# Scheduled tasks

A scheduled task is a reusable piece of work: a prompt, a working directory and a
model, run on a trigger you choose. Open **Scheduled tasks** in the left panel to
create them, watch their runs, and edit or disable them.

> **Scheduled tasks run with full permission and never ask for approval.** They
> are for work you have already decided is safe to run unattended. If a task
> needs to touch untrusted content, give it a folder of its own rather than your
> whole home directory.

## Create one

Click **New scheduled task** and fill in:

- **Name** and **Prompt** — what the task does. The prompt is versioned, so an
  edit keeps the version it replaced.
- **Working directory** — the folder the task works in.
- **Model** and **Thinking level** — the models enabled on the **Models** page.
- **Conversation type** — a **Workspace** conversation filed under the working
  directory (the default), or a **Chat** conversation in its own temporary
  workspace.
- **Conversation after the run** — **Keep** the conversation (default), or
  **Delete** it once the run settles. A deleted conversation still leaves the
  run's status, summary and full answer on the run.
- **Time zone** — the clock the trigger times are read on. Leave it on this
  machine's zone to follow whichever computer runs the task; if that cannot be
  read, `Asia/Shanghai` is used.
- **Trigger** — one of:
  - **Once on a date** — a single run at a date and time.
  - **Every N minutes** — anchored to a fixed grid, so runs do not drift.
  - **Every day / Every week / Every month / Once a year** at a wall-clock time.
    Months without the chosen day run on the last day of the month, so a
    monthly task on day 31 runs on the 30th in April.
  - **Manual (legacy)** — only for a task that already is manual; a new task must
    pick a real trigger.

**Save** stores the task; **Enable** / **Disable** turns a trigger on or off
without deleting anything. **Delete** stops the task but keeps the runs and the
conversations it already created.

## Runs

The right-hand list shows the run ledger: each run's status (queued, running,
done, failed, skipped), the prompt version it used and its result summary.

- **Open** jumps to the conversation the run created.
- A run's full answer stays readable even after its conversation is deleted.

Runs are executed by the desktop app (or `future host`) while it is running. If
FutureOS is closed, a due task waits and runs when it is open again — a phone or
the CLI can queue a request, but neither executes the task.

## From the command line

```bash
future task list [--all]
future task add --name "Nightly review" --prompt "Review new issues" \
  --cwd ~/project --model <model> --daily --time 09:00
future task run "Nightly review" --wait
future task runs "Nightly review"
future task output <run-id>
```

`future task` covers the same tasks: `--every`, `--at`, `--weekly`, `--monthly`
and `--yearly` for the other triggers, `--timezone` for the clock,
`--session new|existing` and `--session-retention keep|delete` for the
conversation, and `prompt log` / `prompt apply` / `prompt revert` for the version
history. See [[CLI (future)|CLI]].

## See also

- [[Using FutureOS|Using-FutureOS]] — the app layout and approval modes.
- [[Approvals and sandboxing|Sandbox]] — what full permission means.
- [[CLI (future)|CLI]] — `future task` in full.
