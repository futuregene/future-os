# Workspace & Conversation Pagination and Archive Design

> Status: design finalized, not yet implemented. This document consolidates the
> conclusions of the design discussion and serves as the basis for implementation.
> Primary scope: the desktop app (`desktop/` + `host/`), with notes on the remote
> catalogue, mobile and TUI/CLI.

[中文](workspace-thread-archive-and-paging.zh-CN.md)

## 1. Background and current state

As users accumulate conversations and workspaces, the current "load everything,
reload everything" list model becomes a performance and usability bottleneck.

| Layer | Current state | Key locations |
|---|---|---|
| Data model | `threads` already has `status('active'/'archived'/'deleted')` + `archived_at`; `workspaces` has no archive column (`cleanup_status` is the temporary-workspace disk-cleanup state machine, unrelated to archiving) | `host/src/store/schema.rs` |
| Store | `list_threads()` returns every non-deleted thread (including archived), no pagination; `archive_thread`/`restore_thread` exist; `list_workspaces()` returns everything | `host/src/store/threads.rs`, `workspaces.rs` |
| Commands | Tauri commands `archive_thread`/`restore_thread` registered; no paged list commands | `desktop/src-tauri/src/commands/threads.rs` |
| Frontend loading | `useThreadStore` loads everything at bootstrap and `refreshStore` reloads everything after each mutation; push events (`thread-activity-updated`, …) patch rows | `desktop/src/components/layout/hooks/useThreadStore.ts` |
| Frontend rendering | Rail tree filters `status === "active"` client-side; the recent view has a client-side window (`RECENT_PAGE_SIZE = 30` + "show more"); search is client-side filtering | `ActivityRail.tsx`, `recentOrder.ts` |
| Archive UI | Half-built: the row menu shows "restore" when `archived`, but active rows have no "archive" action and no surface lists archived conversations | `ActivityRailMenus.tsx` |
| Message history | Cursor-pagination precedent exists: `get_session_entries_page(before, limit)` with a `historySource` fence | `host/src/application/threads.rs` |
| Deletion | Already tree-cascading (`thread_delete_closure`, deleting a parent takes its descendants) | `host/src/store/threads.rs` |
| Tree rules | A parent missing from the list (including archived) promotes its children to roots; pinned rows are always roots; chains deeper than three levels flatten | `desktop/src/components/layout/threadTree.ts` |
| Agent | `list_sessions` RPC enumerates everything; no pagination, no archive concept | `agent/src/rpc/commands/session_lifecycle.rs` |

Conclusion: the archive backend skeleton exists; what is missing is the UI loop,
workspace archiving, and list pagination. Pagination is not just SQL — the
frontend's load-everything/reload-everything model must change with it.

## 2. Goals and non-goals

**Goals**

1. **All three list kinds paginate**: the workspace list, conversations inside a
   workspace, and temporary conversations (the chat section). Startup and refresh
   load only what the first screen needs.
2. A complete archive loop: archive action → gone from the default list →
   browsable, searchable, viewable in detail, restorable.
3. Workspaces can be archived (hidden); their conversations are not loaded while
   archived.
4. Archiving is non-destructive: history, the Agent session, and the ability to
   continue the conversation after restore are all preserved.

**Non-goals (this iteration)**

- Automatic archiving (e.g. N days idle, or archive-on-run-completion).
- Agent-session-level archiving or pagination (TUI/CLI keep full visibility).
- Full list virtualization (paging caps the data volume; existing windowed
  rendering suffices for now).
- Paginating the merged remote list (remote catalogues are full snapshots and
  stay client-merged).

## 3. Archive semantics

### 3.1 States and units

| State | Meaning | Default list | History / session | Reversible |
|---|---|---|---|---|
| active | Normal | Shown | Intact | — |
| archived | Shelved by the user (archive *is* hide; no separate "hidden" state) | Hidden | Fully preserved; continuable after restore | One-click restore |
| deleted | Existing hard-delete flow | Gone | Deleted (stop runs, remove JSONL, clean directories) | Irreversible |

**The archive unit is the fork tree** (a root conversation plus all its
descendants), mirroring deletion's tree cascade (`thread_delete_closure`). A tree
is either entirely visible or entirely archived — mixed states such as
"parent archived, child active" cannot arise, so the cross-status matrix from
earlier drafts disappears: no promote-to-root display rules or lineage
annotations are needed.

Restore is tree-level too: restoring any member of an archived tree brings the
whole tree back. Lineage is lossless (`parent_session_id` is never touched by
archiving), and the visible tree is always rebuilt from the currently visible set.

### 3.2 One precondition everywhere: nothing active in scope

**Every archive requires that nothing in its target scope is active; otherwise it
is refused, naming the offenders.**

| Operation | Precondition | When violated |
|---|---|---|
| Archive one conversation tree | No member active | Refuse, naming the active members |
| Archive a workspace | No conversation tree in it active | Refuse, naming the active conversations (first few + total count) |
| Batch archive (selection mode) | Judged per tree | Inactive trees archived; active ones skipped and reported per tree |

- **Activity** is the union of: non-terminal `runs` rows in the store
  (`running` / `queued` / `waiting_approval`) ∪ the agent's streaming snapshot
  (`list_streaming_sessions`, an in-memory scan that covers runs driven by other
  clients). `waiting_approval` must count as active: archiving a
  approval-waiting conversation would hide its badge and stall the run silently.
- **Stale run rows are settled first**: a crashed non-terminal run row must not
  block archiving forever. The activity check first runs the existing convergence
  path (`settle_interrupted_run_from_agent`), then judges.
- **Refusals are actionable**: "「X」is running / waiting for approval — let it
  finish or stop it first."
- **No "stop and archive" button**: stopping a run belongs in the conversation
  view, where the user can see what it is doing. Archive stays pure — it only
  flips visibility markers and never touches runs.
- **"Archive the inactive ones first" is covered by selection mode**: batch
  archive skips active trees with a per-tree report. The workspace archive dialog
  keeps a single refuse-and-explain outcome.

### 3.3 Side effects and invariants

- **Archiving clears pinning** (for every member of the tree); restore does not
  bring pins back. The archived list is ordered by `archived_at` descending.
- Archiving never stops runs, never rewrites `parent_session_id`, and never
  touches the agent session or its history.
- **Workspace archiving = a container marker + JOIN-hiding at query time**; it
  does not cascade into member threads' `status`. Restoring the workspace brings
  back exactly what was hidden, and conversations archived individually before
  stay archived.
- **Invariant**: GUI actions never produce "an active conversation inside an
  archived scope".
- **Container auto-wake**: when an automatic path (TUI/CLI sessions matched by
  cwd, `reconcile_thread_workspace`, imports) lands a new conversation in an
  archived workspace, the workspace is automatically restored. This is the only
  sanctioned revival — new work in the directory means the user is using it
  again; without it, a running conversation could end up JOIN-hidden.
- **Cross-client activity does not revive archived conversations**: when another
  client keeps running in an archived session, the archived view shows an
  unread/running indicator, but the conversation stays archived.
- **Approvals remain actionable**: even for an archived conversation, a pending
  approval raised by another client stays visible and decidable — safety surfaces
  are never hidden.
- **No archive-on-completion**: "inactive" is not a one-way event (finished →
  auto-archived → another client continues → running again), and it races with
  restore. If the need proves real, the shape is an explicit opt-in checkbox in
  the archive dialog (a persisted pending flag + a run-completion hook), never
  the default.

## 4. Pagination design

### 4.1 Scope: all three list kinds paginate

1. **The workspace list** (`kind='user'`; `temporary` workspaces are containers
   of chat conversations, not standalone list items).
2. **Conversations inside a workspace** (`mode='workspace'`; one paged stream per
   workspace group).
3. **Temporary conversations** (`mode='chat'`; one paged stream for the chat
   section).

The recent view's "earlier" stretch and the archived view paginate as well
(see 4.4).

### 4.2 Keyset cursors, not offsets

The list is continuously reordered by `last_message_at` pushes; offset paging
would duplicate or drop rows. The repo already has a cursor precedent
(`get_session_entries_page`).

- Sort key: `(pinned DESC, activity_ts DESC, id DESC)`, where
  `activity_ts = COALESCE(last_message_at, updated_at, created_at)` — identical
  to the current `list_threads` order, so the UI change is invisible.
- **Pinned rows come from a separate small unpaged query** (pins are hand-picked
  shortcuts and naturally few); paging applies only to the unpinned remainder,
  matching the existing global pinned section.
- The cursor encodes `(activity_ts, id)`; the server compares row values:
  `WHERE pinned = 0 AND (activity_ts, id) < (?, ?)`. Cursors are opaque and need
  no fencing (keyset tolerates concurrent reordering).

### 4.3 Page by roots, then fetch descendants

Row-level paging has a subtle failure: a fork child's `activity_ts` is usually
newer than its parent's, so a page could contain the child while its parent sits
in a later page, and the tree cannot be drawn. Therefore:

1. Cursor paging applies only to **root conversations** (rows with no
   `parent_session_id`, or whose parent is outside the current scope/status set);
2. After fetching a page of roots, one small query fetches **all active
   descendants** of those roots (mapping `parent_session_id` back through agent
   session ids with an IN lookup).

**One page = several complete trees**, matching the rail keyboard navigation's
existing block concept (a block = one fork tree). Page size counts roots
(default 50); the actual row count is slightly larger (each root brings its
descendants) but bounded. Tree-level archiving guarantees "a visible root's
descendants are all visible", so the descendant query needs no status filtering.

### 4.4 Per-view loading behaviour

- **Tree view**: a collapsed workspace **does not load its conversations**
  (collapse becomes real lazy loading, not just visual); expanding fetches the
  first page, and so does a group that starts expanded. Each workspace group and
  the chat section offer "load more" at the bottom.
- **Recent view**: the active stretch comes from `list_active_threads` (unpaged,
  naturally small); the earlier stretch uses the `recent` scope's global cursor
  stream, replacing the client-side `recentLimit` window;
  `RECENT_PAGE_SIZE = 30` stays as the page size. Every page reports the scope's
  own root count (`ThreadsPage.total_roots`), so the control under the list names
  the real remainder and one click loads all of it — a cursor alone cannot say
  how much is left, and naming the loaded part was the defect.
- **Archived view**: flat, `archived_at DESC`, 50 trees per page.
- **Workspace list**: read whole at startup, not paged, and the section does not
  fold. The list holds user workspaces only (a chat's temporary workspace never
  appears in it), so it is one or two requests; a "load more workspaces" under
  the last group hid workspaces behind a click the reader had no reason to make.
  Pinned workspaces are also fetched separately in full.

### 4.5 Event-driven updates replace full reloads (a prerequisite)

With paging, `refreshStore`'s full reload must go first — otherwise every
mutation would wipe the paging state.

- A new **`store-changed` push** carries row-level patches
  `{kind: thread|workspace, op: created|updated|archived|restored|deleted, record}`.
  Every store-mutating command publishes after committing (commands already
  return the record, so the patch and the return value share one source), over
  the existing `crate::events::Event` + Tauri event bridge
  (`thread-activity-updated` is the precedent).
- The frontend reducer inserts/moves/removes rows by sort key; changes outside
  the loaded window leave the window alone (they arrive naturally on the next
  page fetch).
- Mutation return values patch directly + push patches + the existing 30-second
  reconciliation poll stays.
- Unread and pending-approval counts (`usePendingApprovalCounts`) remain
  independent small unpaged queries; they do not paginate with the list.

### 4.6 Server-side search

With paging, client-side search necessarily misses rows. Replace it with:

```
search_threads(query) -> { active: [...], archived: [...] }
```

Semantics unchanged (substring match on title or workspace name, escaped `LIKE`),
results partitioned — which makes archived conversations findable for the first
time. Clicking an archived result opens the read-only detail directly, without
switching views first. The merged remote list keeps its client-side filtering
(remote catalogues are full snapshots).

## 5. Data model changes

Following the migration rules in `host/src/store/schema.rs` (the `SCHEMA`
constant plus immutable `VERSIONED_MIGRATIONS` entries):

```sql
-- New workspaces column (new VERSIONED_MIGRATIONS entry + SCHEMA + ADDED_COLUMNS)
ALTER TABLE workspaces ADD COLUMN archived_at INTEGER;

-- Cursor-paging indexes (expression indexes)
CREATE INDEX idx_threads_page ON threads(
  status, workspace_id, pinned,
  COALESCE(last_message_at, updated_at, created_at), id
);
CREATE INDEX idx_workspaces_page ON workspaces(
  archived_at, pinned, COALESCE(last_opened_at, updated_at), id
);
```

`threads` needs no structural change (`status`/`archived_at` already exist). The
existing `idx_threads_recent` overlaps the new index; evaluate keeping only one
during migration.

`workspaces.last_opened_at` has exactly **one writer**: `store::mark_thread_opened`.
Opening a conversation records its user workspace as the one last used (a chat's
`temporary` container is not a list item and is left alone). Without that writer
the key is just the workspace's creation time, and the cursor degenerates into
"oldest-created last" — the folder used every day sinks past the first page.

## 6. Backend interfaces (host layer)

```rust
// Paged queries
list_threads_page({ scope, cursor, limit }) -> { threads, next_cursor }
// scope: chat | workspace(id) | recent | archived; limit clamped 1..=100, default 50 (roots)
list_workspaces_page({ scope: active | archived, cursor, limit }) -> { workspaces, next_cursor }

// Archive (tree-level; the existing single-row archive_thread is upgraded to tree semantics)
archive_thread_tree(thread_id) -> ThreadRecord        // activity check + closure archive
restore_thread_tree(thread_id) -> ThreadRecord
batch_archive_threads(ids) -> { archived_count, skipped: [{id, reason}] }
archive_workspace(workspace_id) / restore_workspace(workspace_id)  // kind='user' only

// Helpers
list_active_threads() -> Vec<ThreadRecord>   // non-terminal runs + streaming; recent active stretch & badges
search_threads(query) -> { active, archived }

// Events
store-changed: { kind: thread|workspace, op: created|updated|archived|restored|deleted, record }
```

The archive closure reuses the delete closure's traversal
(`thread_delete_closure` walks the real `parent_session_id` lineage, unaffected
by the UI's three-level flattening, with cycle protection), filtered to
`status != 'deleted'`.

## 7. Frontend design

### 7.1 Store rework (`useThreadStore`)

- Bootstrap: concurrently fetch pinned rows in full + the first chat-scope page +
  the first workspaces page + `list_active_threads`; other scopes load on demand.
- Each scope holds `{rows, nextCursor, loading}`; paging appends.
- `refreshStore` retires, replaced by mutation-return patches + `store-changed`
  patches + polling reconciliation.
- The active slot tolerates "the open conversation is an archived one outside
  the loaded window" (fetched on demand via `get_thread`); reselection treats
  "viewing an archived detail" as a valid state instead of bouncing the user back
  to the active list.

### 7.2 Archive entry: a small icon button beside the view switch

The archive entry is not a third switch segment but a standalone small icon
button **on the same row as `RailViewSwitch`**, visually a notch quieter:

```
[ Tree | Recent ]          [🗃]
 └─ primary view switch     └─ archive entry (icon-only)
```

- `Archive` icon at size-3.5, button around size-6, `text-ink-muted`, background
  only on hover — quiet grey at rest, pushed to the row's end with `ml-auto` so
  it reads as separate from the switch.
- No count badge; the count lives in the tooltip ("Archived · 42").
- While the archived view is active the button shows its active style
  (`bg-surface-subtle text-ink`) and neither switch segment is highlighted.
- View state stays with `useRailView` (`view: "tree" | "recent" | "archived"`),
  persisted as today; add the `rail.view.archived` keyboard command.

### 7.3 The archived view

Two sections, matching the two archive units:

- **Archived workspaces**: a heading row (name, archived time, conversation
  count, restore button), expandable to browse member conversations (those
  threads are still `status='active'`, merely JOIN-hidden, and open in detail the
  same way).
- **Archived conversation trees**: flat by tree, `archived_at` descending. Rows
  show title, owning workspace, archived time, member count ("3 conversations"),
  and cross-client activity indicators while archived (unread dot / running
  indicator — visible, but never reviving).

Row actions: restore tree, delete tree, rename. Clicking a row opens the
read-only detail.

### 7.4 Read-only detail

Clicking an archived row opens the conversation in the center pane as usual, but
read-only:

| Capability | Behaviour while archived |
|---|---|
| Message history | Fully readable; `get_session_entries_page` paging works unchanged (the read path never checks `status`) |
| Composer | Disabled, replaced by a banner: "This conversation is archived · restore to continue **[Restore]**" |
| Pin / model switching / other input affordances | Disabled |
| Rename | Allowed (archives need organizing too; uses the existing rename command) |
| Approvals | Still actionable (pending approvals from other clients are never hidden) |
| Cross-client activity | Running/unread indicators shown; no auto-revival |

The banner's **Restore** completes the loop: find → inspect → confirm → one-click
restore → composer unlocks. Leaving the archived view returns the active slot to
the previous active conversation; opening an archived detail is transient and not
persisted.

### 7.5 First-archive guide

Modelled on the existing `SkillIntroBubble` + `skillAttention` pulse:

1. Archive succeeds → the archived rows **collapse and fade out** (150–200ms);
2. The archive icon button **pulses 2–3 times** (~1.5s) and a pointing coachmark
   bubble appears next to it:

   > **Archived conversations live here**
   > View details or restore anytime.
   > [View archived]　[Got it]

3. The bubble auto-dismisses after 8s or on "Got it"; `archiveGuideSeen` is
   persisted in app settings (the same `changeSettings` path as
   `skillIntroDismissed`) and shown only once;
4. "View archived" switches to the archived view, scrolling the just-archived
   tree into view with a brief highlight.

Edge cases: with the rail collapsed (icon-only), degrade to a lightweight toast
("Archived — view it in the archive area [View]") and still mark seen; a first
archive that is a workspace or a batch uses the same generic copy; subsequent
archives show no bubble or pulse, but **the row-exit animation stays** (it is the
baseline feedback for every archive); under `prefers-reduced-motion` the pulse
and slide animations are disabled while the bubble still appears.

### 7.6 Menus and batch

- Conversation row menu gains "Archive": single-node trees archive immediately;
  multi-node trees get a lightweight confirmation ("Archives the whole tree — N
  conversations"), whose copy on a child row states that the root is included.
  Activity is verified in the backend; menu items are not pre-disabled on the
  client, and refusals return the offending members.
- Workspace header menu gains "Archive workspace" (`kind='user'` only).
- The selection-mode toolbar gains "Archive selected": selections are collapsed
  into trees first (selecting one member selects the whole tree, deduplicated),
  each tree is checked, qualifying trees archive as a unit, and skipped trees are
  reported with reasons.

## 8. Impact on other clients

| Client | Phase 1 | Phase 2 (as needed) |
|---|---|---|
| Remote catalogue (host publisher) | Session payload gains an `archived` flag (read from app.db; no agent change) | Mobile filters archived by default + a browse entry |
| Mobile | Unchanged | Same as above |
| TUI / CLI | Unchanged (`list_sessions` stays fully visible, fitting power-user tools) | `list_sessions` gains `cursor/limit`; TUI `/resume` paginates |
| Agent | Unchanged | Same as above, a pure performance optimization (`summary_rows` is already an indexed query; thousands of sessions are fine) |

## 9. Phased delivery

| Phase | Scope | Acceptance |
|---|---|---|
| **P1 Archive loop** | Tree archive + workspace archive (schema migration) + archived view + read-only detail + first-archive guide | Archive → hidden → searchable → detail view → restore → history intact and continuable; migration idempotent on old databases |
| **P2 Pagination** | `store-changed` events + store rework + cursor paging for the conversation scopes + collapse lazy-loading + recent-view rework (its own `recent` stream, true remainder) + the workspace list read whole | With 1000 conversations, startup fetches only pinned + the chat page; the recent view pages the whole store 30 at a time; mutations no longer trigger full reloads |
| **P3 Search & propagation** | Server-side search (with archived partition) + remote-catalogue `archived` flag | Unloaded rows are findable |
| **P4 As needed** | Agent `list_sessions` paging, TUI/mobile, render virtualization, opt-in auto-archive | — |

P1 ships independently (archiving does not depend on paging); P2 carries the
performance gains; P3 depends on P2's store shape.

## 10. Edge cases

| Case | Handling |
|---|---|
| Stale run rows from a crash blocking archive | Settle via `settle_interrupted_run_from_agent` before the activity check |
| Cycles in fork lineage | Reuse `thread_delete_closure`'s cycle protection; `buildTree` breaks edges defensively as well |
| Forks spanning workspaces | The closure traversal covers them naturally; the confirmation dialog's count is honest |
| Another client keeps running in an archived conversation | No revival; activity indicator in the archived view; approvals stay actionable |
| TUI/CLI starts a session in an archived workspace's directory | The workspace auto-restores (container wake) |
| First archive with the rail collapsed | Guide degrades to a toast; no layout change |
| `prefers-reduced-motion` | Animations off; bubble still shown |
| Opening an archived detail outside the loaded window | `get_thread` on demand; active slot and reselection tolerate it |
| Deleting from the archived view | Same as the active list: tree cascade + confirmation + stop-first |

## 11. Open questions

1. **Restoring a single conversation out of an archived tree**: not in phase 1
   (restore is tree-level). If needed, add an explicit "detach from the archived
   tree" action — the existing tree code already renders mixed states correctly.
2. **Archive-on-completion**: only ever as an explicit opt-in (see 3.3).
3. **Should TUI/CLI respect archiving**: not in phase 1 (archiving is a GUI
   organization-layer concept and is not pushed down into agent sessions,
   avoiding two-way synchronization).
4. **An "archive suggestions" list for old conversations**: later, built on the
   `managed_cleanup` scheduling machinery — suggestions for N-days-idle
   conversations, never automatic archiving.
