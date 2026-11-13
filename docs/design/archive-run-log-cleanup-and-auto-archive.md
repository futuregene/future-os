# Conversation Archiving: Run-Log Cleanup and Automatic Archiving

> Status: the run-log cleanup design is substantially settled; automatic archiving is a design proposal awaiting review. Implementation has not started.
> Date: 2026-11-13.
> Scope: Desktop (`desktop/`), the shared host (`host/`), and Agent history storage (`agent/`). Remote conversations follow the owning host's rules.
> Related: [Workspace & Conversation Pagination and Archive Design](workspace-thread-archive-and-paging.md).

[中文](archive-run-log-cleanup-and-auto-archive.zh-CN.md)

## 1. Terminology and current behavior

### 1.1 Two different kinds of archive

1. **Run archiving**: the “Archive finished” action sets `runs.archived_at` and hides terminal runs from the Runs panel. It does not delete history and has no “unarchive run” action. This proposal leaves it unchanged.
2. **Conversation archiving**: archives an entire conversation fork tree, makes it read-only, and allows the user to restore it and continue talking. Run-log cleanup and idle automatic archiving both operate at this layer.

Workspace archiving remains container-level hiding. This proposal does not change fork lineage, workspace wake-up, approval visibility, or read-only/restore behavior.

### 1.2 Where the data lives

| Location | Contents | Effect of this proposal |
|---|---|---|
| `runs` in `~/.future/app/app.db` | Run identity, conversation association, status, timestamps, errors, and `archived_at` | Retained, with cleanup-completion state added |
| Conversation bodies in `~/.future/agent/agent.db` | User messages, assistant prose, `tool_call` records, tool results, and associated content bodies | Prose and commands retained; selected outputs removed |
| Stream replay data in the same database | `stream_*`; legacy formats also use `run_events`, `run_snapshots`, and related records | Replay data for affected runs removed |
| Workspace files, Artifacts, and Review | Source files and other independent products | Untouched |
| Diagnostic logs such as `~/.future/agent/logs/agent.log` | Agent diagnostics, not the source of the run-detail panel | Out of scope |

“Run logs” in this document means **the raw output, error details, and replay shown in the right-hand panel after opening a command in the transcript**. It does not mean the command title, assistant prose, or specifically `agent.log`.

## 2. Goals and non-goals

### 2.1 Goals

- Reduce the long-term retention of unneeded run details when a conversation is archived.
- Preserve user prompts, assistant answers, command titles, command order, and completion status across archiving.
- Show “Run logs cleared” when a command in an archived conversation is opened, rather than a blank panel, an error, or a claim that the command never ran.
- Allow a restored conversation to keep its readable prose and start new turns and tool calls normally.
- Add an archive-cleanup setting enabled by default, explain it before the first actual cleanup, and provide a direct link to the setting.
- Add an independent, opt-in idle-archive setting based on meaningful conversation activity rather than creation time.

### 2.2 Non-goals

- Restoring cleared output or guaranteeing a lossless continuation of an old failed run from its original stderr.
- Changing the existing “Archive finished” behavior.
- Deleting source files, undoing command side effects, or cleaning the Git worktree.
- Automatically archiving workspace containers in v1; only their conversation trees are candidates.
- Promising secure erasure or removal of every copy for privacy purposes. Compaction summaries, legacy migration sources, independent backups, and existing exports have separate retention contracts.
- Removing Fusion reports, Artifacts, or other result-oriented content in v1. Only Shell command output is removed from conversation bodies.

## 3. Clear run logs when archiving

### 3.1 Retention boundary

| Data | State after archive cleanup |
|---|---|
| User prompts, assistant prose, and attachments | Preserved unchanged |
| Session identity, cwd, model, permissions, and other session records | Preserved unchanged |
| “Ran” rows, command text, tool kind, and execution order | Preserved unchanged |
| Shell `tool_call` records and arguments such as `command` | Preserved |
| `runId`, `toolCallId`, original success/failure status, and timestamps | Preserved |
| Shell stdout, stderr, raw output, and detailed `shell_result` | Removed |
| Stream events and replay snapshots for affected runs | Removed |
| Other conversation prose, source files, Artifacts, and Review | Preserved |
| A durable completion marker such as `logs_purged_at` | Added |

Commands and full call arguments remain. This feature therefore **removes execution output and replay details**, not all tool history, and is not sensitive-data erasure. Code embedded in Write/Edit arguments is not removed by this feature; source files on disk are also unaffected.

### 3.2 Model history: keep the call, empty the result

A Shell `tool_result` must not simply be deleted while its `tool_call` remains. Providers require properly paired calls and results, and the existing “tool execution lost” repair placeholder could make a completed command appear interrupted.

The recommended rewrite unit is:

1. Keep the original `tool_call`, arguments, and identity.
2. Keep the corresponding `tool_result` record shell, replacing its original output with a fixed “cleared on archive” placeholder.
3. Preserve the result's `tool_call_id`, `is_error`, timestamp, and run association so names, counts, and completion states do not change.
4. Remove large output fields, detailed Shell facts, and content references holding the original output.
5. Retire the affected run's replay stream so deleted output cannot return through replay.
6. Reload the Agent's in-memory history after the rewrite so the next request uses the rewritten version.

The placeholder exists only to preserve protocol validity and status. The detail UI must not present it as real output.

The Agent remains the sole history writer. The existing session-reset planner, atomic rewrite, and content-reclamation ideas can be reused, but its long-lived full `session-backups` policy cannot be copied directly: doing so would retain complete copies of the output and defeat the cleanup's purpose.

### 3.3 Settings and defaults

| Setting | Default | Meaning |
|---|---|---|
| Automatically clear run logs when archiving | **On** | Applies the boundary in §3.1 to manual and automatic conversation archiving |
| First run-log cleanup notice seen | Not seen | Recorded once per installation, not once per conversation |

Suggested setting description:

> Preserve conversation prose, command titles, and generated files while clearing command output and run replay. You can restore the conversation and continue talking, but cleared run logs cannot be recovered.

### 3.4 First-use notice and settings link

Because cleanup is enabled by default, the first actual cleanup after the feature ships must be explained. The notice appears only for the **first archive that would really clear logs**. A conversation with nothing to clean does not show an irrelevant warning or consume the one-time guide.

Suggested copy:

> **Archive this conversation?**
>
> Prompts, assistant answers, command titles, and generated files will be preserved. You can restore the conversation later and continue talking.
>
> This archive will clear the output and run replay shown when a command is opened. This cannot be undone. The feature is enabled by default and can be changed in Archive Settings.

Actions:

- **View Archive Settings**: navigate to the setting without archiving;
- **Cancel**: remain in the current conversation;
- **Archive and Clear Run Logs**: execute according to the current setting.

After returning from Settings, the copy and primary action follow the latest preference: if cleanup was disabled, the next action is an ordinary archive. The notice is global and shown once, not once per fork tree. Conversations archived before this feature ships are not cleaned retroactively.

### 3.5 Presentation after archiving

- The original “Ran” row and command text stay in place. A subdued “logs cleared” state may be added, but raw output is never shown.
- Opening the command shows:

  > **Run logs cleared**
  >
  > This command's output and run replay were deleted when the conversation was archived. The command record, conversation prose, and generated files remain. Deleted logs cannot be recovered.

- `tools.length === 0` must not be interpreted as proof of cleanup: a run may never have had tools, and a failed cleanup may also leave an incomplete state.
- `logs_purged_at` is written only after that run was actually cleaned. Legacy archives, pending cleanup, failed cleanup, and new runs created after restore must all reflect their real state.

### 3.6 Cleanup execution and restore

- Conversation archiving and Agent history rewriting cross storage boundaries and cannot be assumed to share one database transaction.
- Cleanup may run as a retryable background task. Failure must not trap the conversation in its archived state or prevent restore and further conversation.
- The UI must not claim completion while cleanup is pending. Partial success is recorded per run rather than marking the whole fork tree complete.
- Restore brings back visibility and the composer, not old logs. New turns, tool calls, and details are stored normally and are handled according to the setting on the next archive.
- If the user restores and immediately continues, retained tool calls and results must still form valid provider history; the frontend must not be responsible for hiding protocol errors.

## 4. Automatically archive idle conversations

### 4.1 Settings and default policy

| Setting | Proposed default | Meaning |
|---|---|---|
| Automatically archive idle conversations | **Off** | Background archiving starts only after explicit opt-in |
| Idle period | **30 days** | Offer 7 / 30 / 90 days and a custom integer, suggested range 1–365 |
| Scan frequency | **Once per day** | No higher-frequency or cron configuration in v1 |

Automatic archiving and run-log cleanup are independent switches: the former decides **when to shelve**, the latter decides **what to delete when shelving**.

### 4.2 Archive unit and scope

- The unit is the **complete conversation fork tree**, exactly as for manual archiving, not an individual descendant.
- Both temporary chats and conversations inside workspaces are covered.
- **Workspace containers are not automatically archived in v1.** A workspace can contain trees with very different activity and its directory may still be actively used outside FutureOS. Hiding the whole container remains an explicit user action.
- Already archived or deleted objects are skipped; archived runs are not candidates.

### 4.3 Time basis: last meaningful conversation activity

Creation time is not the primary test. A conversation created years ago but used yesterday is still active.

For each member:

```text
member_activity = max(
  last_message_at (or created_at when there is no message),
  restored_at (if the member was restored)
)
```

For the whole tree:

```text
tree_activity = max(member_activity across every member)
```

A tree becomes a candidate only when `now - tree_activity >= N × 24 hours`.

Rules:

1. **Meaningful activity** includes new prompts, assistant replies, new runs, and real message activity from other clients. `last_message_at` already covers the main cases.
2. **Restore counts as activity** and starts a fresh N-day period. This prevents a just-restored old conversation from being archived again on the next scan. A dedicated `restored_at` can be added rather than overloading the broad `updated_at` field.
3. **An empty conversation with no messages** uses `created_at`, allowing long-unused empty conversations to be tidied.
4. **Opening, renaming, changing the model, or ordinary metadata edits do not count as conversation activity.** `updated_at` is too broad to be the archive clock.
5. **The currently foregrounded conversation is skipped for that scan**, so it does not become read-only under the reader. Opening it does not refresh long-term activity, so it can still be archived after it is closed and a later scan runs.
6. **Recent activity in any descendant protects the entire tree**, preventing an old-looking parent from being archived while its derived work is still active.

### 4.4 Permanent or per-scan exclusions

- Any pinned member in the tree. Pinning is an explicit request to keep a shortcut; v1 has no “archive pinned trees too” option.
- Any `running`, `queued`, or `waiting_approval` run, or an Agent snapshot that still reports streaming.
- The conversation tree currently open in the foreground.
- Any scan attempted before startup recovery and run-state reconciliation have completed.
- A tree that receives a new message or run after candidate selection but before the archive commit.

Automatic archiving must reuse the manual activity check and revalidate activity and state immediately before committing, rather than trusting a stale snapshot captured at the start of the daily scan.

### 4.5 Scan cadence and time semantics

Recommended schedule:

1. Scan after application startup, once Store and Agent state have settled.
2. Scan every 24 hours while the application remains running.
3. Do no work while the application is closed; calculate eligibility from the current time on the next launch.
4. Process candidates in batches, revalidating each batch and allowing user operations to interleave.

“30 days” means **at least thirty 24-hour periods of idleness**, not a precisely timed local-calendar reminder. A daily scan means archiving can actually happen between days 30 and 31, and later still if the application was closed. Hourly scans are unnecessary for day-level organization and would add background work and notification noise.

### 4.6 Interaction with run-log cleanup

Automatic archiving follows the archive-cleanup setting in force when it runs:

- Cleanup on: archive in the background, then clear run logs by the same rules.
- Cleanup off: archive without deleting logs.

The automatic-archive settings page must expose that interaction directly:

> Automatic archiving follows the current run-log cleanup setting. Current: **Clear run logs when archiving (on)**. [View Archive Settings]

Enabling automatic archiving authorizes subsequent background execution under that policy; there is no confirmation dialog for every tree. Users who do not want background deletion must disable cleanup or leave automatic archiving off.

The first completed automatic archive produces one summary notification, not a dialog per item:

> Automatically archived 3 conversations idle for more than 30 days and cleared their run logs according to your settings. [View archived]

No notification is emitted when nothing was archived. Cleanup failure does not undo the archive; details show the real state and cleanup can be retried.

### 4.7 Restore, remote ownership, and idempotency

- Restoring a conversation starts a fresh N-day idle period through `restored_at`; cleared logs do not return.
- New turns and runs after restore update `last_message_at` normally and drive the next eligibility calculation.
- A remote conversation is automatically archived only by the host that owns its Store and Agent. Remote control clients do not each run the policy, avoiding multi-device races.
- Activity from another client after archiving follows the existing rule and does not automatically restore the conversation; its timestamps still inform later decisions.
- A tree that is already archived produces no mutation, notification, or duplicate placeholder on later scans.

## 5. Suggested settings information architecture

A “Conversations and Archive” settings section can present, in order:

1. **Automatically clear run logs when archiving** — on by default.
2. **Automatically archive idle conversations** — off by default.
3. **Idle period** — editable only when automatic archiving is on; 30 days by default.
4. The cleanup interaction and a “View Archive Settings” link.
5. A plain-language policy summary, such as “Unpinned conversations idle for more than 30 days are checked once per day.”

Changing the idle period affects later candidate calculations. It need not force an immediate scan and never restores an already archived conversation.

## 6. Edge cases

| Case | Behavior |
|---|---|
| An old archive still has its logs | Does not claim to be cleaned and is not deleted retroactively |
| Cleanup fails or only partially completes | Restore remains available; state is recorded per run and cleanup can be retried |
| A command originally had no output | Cleanup may complete, but the UI must not invent output that never existed |
| A run never called a tool | No “Ran” placeholder is invented; an empty tool list is not proof of cleanup |
| Old creation time but recent replies | Not archived; last meaningful activity wins |
| An empty conversation was never prompted | Creation time is used, so it can be archived after N days |
| An old conversation is only opened or renamed | Long-term activity is unchanged; a foreground tree is skipped for that scan |
| Old parent fork, recent child | The entire tree is protected |
| A very old archived conversation is restored | A fresh N-day period starts at restore |
| The application is reopened after 60 days away | Scan after startup reconciliation and archive only currently eligible trees |
| Automatic archiving on, log cleanup off | Conversations are shelved without deleting logs |
| Manual and automatic archiving target the same tree | State is rechecked before commit; an already archived tree is an idempotent no-op |
| A workspace still contains one active tree | Other eligible trees may be archived independently; the workspace container is not automatically archived |

## 7. Acceptance criteria

### 7.1 Run-log cleanup

1. The default setting produces the first-use explanation before the first real cleanup; all three actions work and the settings link lands on the correct control.
2. User messages, assistant prose, command text, command counts, and original completion states are unchanged by archiving.
3. Opening a cleaned command shows only the explanation and never returns raw stdout, stderr, or replay.
4. Deleted output cannot reappear through a reload, run-detail query, or old event cursor.
5. Source files and independent Artifacts are unchanged.
6. After restore and an application restart, a new turn is accepted by the provider without tool-pairing errors and new tools run normally.
7. Legacy archives, pending cleanup, failed cleanup, successful cleanup, and new runs have distinct and truthful UI states.
8. Repeated archive and cleanup operations create no duplicate placeholders and never delete new prose.

### 7.2 Automatic archiving

1. It is off by default; background archiving starts only after opt-in, with a 30-day default threshold.
2. A 29-day candidate is retained; an eligible 30-day candidate is archived on the next daily scan, with no exact-time guarantee.
3. An old conversation with recent messages, runs, or a restore is retained.
4. Renaming does not extend its life; a foreground tree is skipped; an empty conversation uses creation time.
5. The newest activity in a fork tree protects the whole tree; pinned, active, approval-waiting, and foreground trees are skipped.
6. No work occurs while offline, restart catches up by rule, and repeated scans do not duplicate notifications.
7. Automatic archiving follows the cleanup setting in force at execution time, and that interaction is visible on the settings page.
8. Restore starts a fresh idle period and preserves continued conversation; cleared logs remain deleted.
9. Workspace containers are not automatically archived, and remote clients do not duplicate the owning host's job.

## 8. Possible follow-ups

- A separate “automatically archive idle workspaces” setting, allowed only when every tree meets the same idle rule and the workspace has no pins, active work, or recent opens. It must not share the v1 conversation switch.
- An “archive suggestions only” mode that presents candidates for bulk confirmation instead of archiving in the background.
- Persistent exclusions for selected workspaces or conversations, so pinning is not the only way to express an exception.
- If the goal later changes from storage reduction to privacy erasure, define a separate retention policy for command arguments, compaction summaries, backups, legacy sources, and exports.
