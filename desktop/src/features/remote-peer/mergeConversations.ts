/**
 * The merged conversation list: this machine's threads and every paired remote
 * desktop's sessions, in one list, ordered by the same rule everywhere.
 *
 * Two properties matter and are the reason this is a pure module rather than
 * inline JSX:
 *
 * 1. **Every row knows its source.** A row that cannot name its host would be
 *    routed to the wrong machine on click, so the source is part of the row
 *    identity, not a presentation detail.
 * 2. **The order must be one order.** Local threads already rank by
 *    `pinned, COALESCE(last_message_at, updated_at, created_at)` on the host;
 *    remote sessions carry `lastMessageAt` from that same coalesce. Ranking
 *    them together with a different rule would make the merged list disagree
 *    with the per-machine lists the user can also see.
 * 3. **A heading ranks by its own place, not its rows'.** A workspace group is
 *    ordered by the workspace's own key (when *it* was last touched), which is
 *    what the local list already does — ordering groups by their conversations
 *    made a group jump whenever one of its rows moved, and made the merged list
 *    disagree with the list of this machine alone.
 *
 * A remote session without a timestamp (an older host) sorts below every
 * timestamped row rather than at the top: "unknown age" must not masquerade as
 * "just now". Equal timestamps — the same message arriving on two machines, or
 * two workspaces touched in the same millisecond — put this machine first and
 * then a host by its device id, so the order is total instead of whatever order
 * the rows happened to be merged in.
 */

/** A session as the host's `list_sessions` reports it, plus our stamp. */
export interface RemoteSessionRow {
  sessionId: string;
  threadId?: string;
  title: string;
  mode?: "chat" | "workspace";
  workspaceId?: string;
  /** The workspace's name on that host; absent on a host that predates it. */
  workspaceName?: string | null;
  /**
   * Whether the workspace this conversation sits in is pinned *on that host*.
   *
   * Carried per row rather than as a workspace list because it is what the
   * merged list orders groups by, and a group exists only where a row does. A
   * host that predates the field says nothing, and "unknown" must not order a
   * group as pinned.
   */
  workspacePinned?: boolean | null;
  /**
   * When that workspace itself was last touched on its own machine.
   *
   * The host's `COALESCE(last_opened_at, updated_at)` — the key its own
   * workspace list is ordered by, which is what keeps a group's place
   * independent of the conversations under it. Absent on a host that predates
   * the field, and an absent key must not pretend to be "just now".
   */
  workspaceOrderedAt?: number | null;
  parentSessionId?: string | null;
  pinned?: boolean;
  streaming?: boolean;
  status?: string;
  /** Unix millis. Absent on a host that predates the field. */
  lastMessageAt?: number | null;
}

export interface RemoteCatalog {
  /** The paired host this snapshot came from. */
  desktopId: string;
  sessions: RemoteSessionRow[];
}

/**
 * One workspace on another machine, addressed the way that host names it.
 *
 * `desktopId` and `workspaceId` are what a command needs; `name` is what the user
 * sees, and only what a dialog opens with — a workspace is renamed by id, because
 * the name is the thing being changed.
 */
export interface RemoteWorkspaceTarget {
  desktopId: string;
  name: string;
  workspaceId: string;
}

/** One row of the merged list. */
export interface MergedConversation {
  /** Unique across machines: the same session id on two hosts is two rows. */
  key: string;
  /** `null` for a local thread. */
  desktopId: string | null;
  /** The local thread id, or the remote session id. */
  id: string;
  /**
   * The *host* thread id for a remote row; `null` for local rows.
   *
   * Distinct from `id` on purpose: a remote row is addressed by its session id
   * for anything session-scoped (prompt, abort, rename, reading history), but
   * the host's pin and delete routes take a **thread** id. Dropping this in the
   * merge would make those two actions unaddressable, and guessing that the two
   * ids are interchangeable would pin or delete the wrong record.
   */
  threadId: string | null;
  title: string;
  pinned: boolean;
  streaming: boolean;
  /** Unix millis, or `null` when the source has no timestamp. */
  lastMessageAt: number | null;
  mode: "chat" | "workspace";
  /** The remote workspace this session belongs to; `null` for local rows. */
  workspaceId: string | null;
  /**
   * The workspace's name, for a group heading.
   *
   * Local rows take it from this machine's workspace store; remote rows from the
   * host's catalogue. Absent when neither knows it — a heading is then the
   * machine itself, which is still truthful, where an id would be noise.
   */
  workspaceName?: string | null;
  /**
   * Whether the workspace this row sits in is pinned at its source.
   *
   * Per row, because a group is only as pinned as the rows in it and the pin is
   * the *owner's*: a workspace on another machine is pinned on that machine, and
   * the group leads this list because the host said so.
   */
  workspacePinned?: boolean;
  /**
   * When that workspace was last touched *at its source*.
   *
   * The group's own place in the list — never its conversations': a workspace's
   * rank must not move because a conversation inside it did. This machine's
   * value comes from this machine's workspace store, a host's from that host's
   * own report, and both are the same key (`last_opened_at`, else `updated_at`)
   * their own workspace list is ordered by.
   */
  workspaceOrderedAt?: number | null;
  /**
   * The conversation this one was forked from, as a row key in this list.
   *
   * Resolved by the merge rather than carried over: a parent that is not a row
   * here (another machine's filter, a deleted conversation) leaves its child a
   * root, which is what the tree needs to know. Local rows are not resolved
   * here — the rail builds this machine's tree from its own store and passes
   * each row's place in.
   */
  parentKey?: string | null;
}

export interface LocalThreadLike {
  id: string;
  title: string;
  mode: "chat" | "workspace";
  pinned: boolean;
  status: "active" | "archived" | "deleted";
  workspaceId: string;
  workspacePinned?: boolean;
  /** This machine's own key for the workspace, as its store reports it. */
  workspaceOrderedAt?: number | null;
  lastMessageAt?: number | null;
  updatedAt: number;
  createdAt: number;
}

/** Which machines the list shows. `null` desktopId means this machine. */
export type DeviceFilter = { kind: "all" } | { kind: "device"; desktopId: string | null };

export function rowKey(desktopId: string | null, id: string): string {
  return `${desktopId ?? "local"}::${id}`;
}

/**
 * The local thread's timestamp, using the host's own coalesce so a thread that
 * has never been messaged still sorts by when it was last touched.
 */
export function localTimestamp(thread: LocalThreadLike): number {
  return thread.lastMessageAt ?? thread.updatedAt ?? thread.createdAt;
}

/**
 * Build the merged list.
 *
 * `catalogs` may contain several hosts; a host that is not connected simply has
 * no entry, and its rows disappear rather than going stale-but-visible. Rows
 * are de-duplicated by `(desktopId, id)`, so a poll that re-reads the same
 * snapshot cannot double a row.
 */
export function mergeConversations(
  threads: LocalThreadLike[],
  catalogs: RemoteCatalog[],
  filter: DeviceFilter,
  /** This machine's workspace names, for the group headings. */
  workspaceNames: ReadonlyMap<string, string> = new Map(),
): MergedConversation[] {
  const rows: MergedConversation[] = [];
  /**
   * Each remote row's parent, held until every row is known.
   *
   * A fork is drawn under the turn it came from, and that is a question about
   * the list rather than about one host's rows: the parent is only addressable
   * if it is a row here, which can only be answered once the merge is complete.
   */
  const forks: Array<{ parentSessionId: string; row: MergedConversation }> = [];

  const wantsLocal = filter.kind === "all" || filter.desktopId === null;
  const wantsHost = (desktopId: string) =>
    filter.kind === "all"
    || (filter.kind === "device" && filter.desktopId === desktopId);

  if (wantsLocal) {
    for (const thread of threads) {
      // Deleted threads are not conversations the user can open; archived ones
      // are still listed (the rail has a restore action for them).
      if (thread.status === "deleted")
        continue;
      rows.push({
        key: rowKey(null, thread.id),
        desktopId: null,
        id: thread.id,
        // A local row's thread id *is* its id: the local store addresses
        // threads by that id everywhere, so there is nothing to translate.
        threadId: thread.id,
        title: thread.title,
        pinned: thread.pinned,
        streaming: false,
        lastMessageAt: localTimestamp(thread),
        mode: thread.mode,
        workspaceId: thread.workspaceId,
        workspaceName: workspaceNames.get(thread.workspaceId) ?? null,
        // A local row's workspace pin is passed in with its thread (the rail
        // reads it from this machine's store); nothing is looked up here.
        workspacePinned: thread.workspacePinned === true,
        workspaceOrderedAt: thread.workspaceOrderedAt ?? null,
        // This machine's tree is the rail's (its rows arrive as built nodes), so
        // the merged list only carries a *remote* row's parent.
        parentKey: null,
      });
    }
  }

  const seen = new Set(rows.map(row => row.key));
  for (const catalog of catalogs) {
    if (!wantsHost(catalog.desktopId))
      continue;
    for (const session of catalog.sessions) {
      const key = rowKey(catalog.desktopId, session.sessionId);
      if (seen.has(key))
        continue;
      seen.add(key);
      const row: MergedConversation = {
        key,
        desktopId: catalog.desktopId,
        id: session.sessionId,
        // Absent only on a host that predates the catalogue's thread id. The
        // row is still readable and promptable; pin and delete stay disabled
        // rather than falling back to the session id, which the host's pin and
        // delete routes do not accept.
        threadId: session.threadId ?? null,
        title: session.title,
        pinned: session.pinned === true,
        streaming: session.streaming === true,
        lastMessageAt: typeof session.lastMessageAt === "number" ? session.lastMessageAt : null,
        mode: session.mode === "workspace" ? "workspace" : "chat",
        workspaceId: session.workspaceId ?? null,
        workspaceName: session.workspaceName ?? null,
        workspacePinned: session.workspacePinned === true,
        workspaceOrderedAt: typeof session.workspaceOrderedAt === "number"
          ? session.workspaceOrderedAt
          : null,
        parentKey: null,
      };
      if (session.parentSessionId)
        forks.push({ parentSessionId: session.parentSessionId, row });
      rows.push(row);
    }
  }

  for (const { parentSessionId, row } of forks) {
    // A session id is only meaningful on the host that issued it, so the parent
    // is looked for on this row's own machine — the same id exists on another.
    const parent = rowKey(row.desktopId, parentSessionId);
    if (parent !== row.key && seen.has(parent))
      row.parentKey = parent;
  }

  return rows.sort(compareConversations);
}

/**
 * Pinned first, then most recent. Ties break on the machine and then on the row
 * key so the order is stable across polls: a list that reshuffles equal
 * timestamps flickers, and a flickering list is how a user misses the message
 * they were reading. Two machines can hold the same session id with the same
 * timestamp, which is why the machine is compared before the key: this machine
 * first, then a host by its desktop id.
 */
export function compareConversations(a: MergedConversation, b: MergedConversation): number {
  if (a.pinned !== b.pinned)
    return a.pinned ? -1 : 1;
  const at = a.lastMessageAt ?? Number.NEGATIVE_INFINITY;
  const bt = b.lastMessageAt ?? Number.NEGATIVE_INFINITY;
  if (at !== bt)
    return bt - at;
  return compareSource(a.desktopId, b.desktopId) || compareText(a.key, b.key);
}

/**
 * Which machine's row leads when two rows are otherwise equal.
 *
 * This machine (`null`) first, then a host by its device id. Every order in this
 * module ends on this, so "the same timestamp" has one answer rather than
 * whichever order the rows were merged in.
 */
export function compareSource(a: string | null, b: string | null): number {
  if (a === b)
    return 0;
  if (a === null)
    return -1;
  if (b === null)
    return 1;
  return compareText(a, b);
}

/** A total order on text for the ties above (`localeCompare` is not stable enough). */
function compareText(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

/** A heading in the merged list, with the rows under it. */
export interface MergedGroup {
  key: string;
  /** `null` for this machine. */
  desktopId: string | null;
  kind: "chat" | "workspace";
  /** The workspace's name, or `null` for the chat section. */
  label: string | null;
  /** The workspace this group is, for this machine's own groups. */
  workspaceId: string | null;
  /**
   * Whether the workspace's owner pinned it, so the group leads the list.
   *
   * A workspace on another machine is pinned *there*: the pin is read from that
   * host's own report. What is asserted here is the rule, not the source.
   */
  pinned: boolean;
  /**
   * When the workspace itself was last touched, at its source.
   *
   * What the group is ordered by. `null` for a source that does not report one
   * (an older host, or a group of chats): an unknown place sorts after every
   * known one rather than pretending to be current.
   */
  orderedAt: number | null;
  rows: MergedConversation[];
}

/**
 * The merged list's headings: pinned, then workspaces, then chats.
 *
 * The same shape the local list has, because a user who pairs a machine and then
 * cannot find their workspaces has lost something they had before the feature
 * existed. A workspace is identified by `(machine, workspace id)`, not by name: a
 * group called "future-os" exists on two machines at once, and merging them would
 * put another machine's rows under this machine's heading.
 *
 * **A group's place is the workspace's own, never its rows'.** Ordering groups by
 * their newest conversation reads as reasonable — the list is ordered by time —
 * and is wrong in the two ways that matter: the group jumps while the reader is
 * working in it (a reply in the third workspace moves that group to the top and
 * back when the catalogue is next read), and the merged list then disagrees with
 * the list of this machine alone, which orders its workspaces by when *they* were
 * last touched (`COALESCE(last_opened_at, updated_at)`, the same coalesce the
 * host reports per row). So the key is the workspace's, from whichever machine
 * owns it, and the rows' recency only orders rows inside a group.
 */
export function groupMergedRows(
  rows: MergedConversation[],
): {
  chats: MergedConversation[];
  pinned: MergedConversation[];
  workspaces: MergedGroup[];
} {
  const pinned = rows.filter(row => row.pinned);
  const rest = rows.filter(row => !row.pinned);

  const byKey = new Map<string, MergedGroup>();
  for (const row of rest) {
    const machine = row.desktopId ?? "local";
    // A chat-mode conversation has no workspace of its own in this list's shape:
    // the local rail shows one chat section for all of them, and a heading per
    // workspace would be a heading for something the row is not in.
    const grouped = row.mode === "workspace";
    // This machine's groups are keyed by the workspace id *alone*, which is the
    // key the local list already folds by: the same workspace then folds and
    // unfolds together in both lists, instead of two fold states disagreeing
    // about one workspace depending on which list is drawn. A remote workspace
    // needs the machine in the key — two machines can hold the same workspace id.
    const key = grouped
      ? (row.desktopId === null ? (row.workspaceId ?? "") : `${machine}::${row.workspaceId ?? ""}`)
      : "chat";
    const existing = byKey.get(key);
    const group: MergedGroup = existing ?? {
      key,
      // A chat section has no machine: it holds every machine's chats, which is
      // the point of it — the section is where a reader looks for conversations
      // rather than for a place on a disk.
      desktopId: grouped ? row.desktopId : null,
      kind: grouped ? "workspace" : "chat",
      label: row.workspaceName ?? null,
      workspaceId: grouped ? row.workspaceId : null,
      pinned: grouped && row.workspacePinned === true,
      orderedAt: grouped ? (row.workspaceOrderedAt ?? null) : null,
      rows: [],
    };
    if (!existing)
      byKey.set(key, group);
    // A host that predates the workspace *name* must not blank a heading a newer
    // row in the same group supplied — and the same holds for its pin and its
    // place: one row reporting it is the group's answer.
    group.label ??= row.workspaceName ?? null;
    group.pinned ||= grouped && row.workspacePinned === true;
    group.orderedAt ??= row.workspaceOrderedAt ?? null;
    group.rows.push(row);
  }

  const groups = [...byKey.values()];
  groups.sort((a, b) => {
    // Pinned groups first, as in the local list; then the workspace's own place,
    // newest first, so a group holds its rank while the reader works in it.
    if (a.pinned !== b.pinned)
      return a.pinned ? -1 : 1;
    const at = a.orderedAt ?? Number.NEGATIVE_INFINITY;
    const bt = b.orderedAt ?? Number.NEGATIVE_INFINITY;
    if (at !== bt)
      return bt - at;
    return compareSource(a.desktopId, b.desktopId) || compareText(a.key, b.key);
  });

  const chats = groups.find(group => group.kind === "chat");
  return {
    pinned,
    workspaces: groups.filter(group => group.kind === "workspace"),
    // One section, every machine's conversations in it, in the same order the
    // rows were merged in. Grouping these by machine put a heading between two
    // conversations that belong to the same hour, and made the one list a user
    // scans for "what is current" into one list per machine to scan separately.
    chats: chats ? chats.rows.slice().sort(compareConversations) : [],
  };
}
