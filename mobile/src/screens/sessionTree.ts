import type { RemoteSession, RemoteWorkspace } from "../remote/types";
import type { MergedSessionRow } from "../remote/mergeSessions";
import { sessionKey } from "../remote/mergeSessions";

export interface SessionNode {
  session: RemoteSession;
  children: SessionNode[];
}

/** Desktop parity: pinned/orphan sessions are roots; break cycles without losing rows. */
export function buildSessionTree(sessions: RemoteSession[]): SessionNode[] {
  const nodes = new Map(
    sessions.map(session => [session.sessionId, { session, children: [] } as SessionNode]),
  );
  const parents = new Map<string, string>();
  for (const session of sessions) {
    const parent = session.parentSessionId;
    if (!session.pinned && parent && parent !== session.sessionId && nodes.has(parent)) {
      parents.set(session.sessionId, parent);
    }
  }
  const resolved = new Set<string>();
  for (const id of nodes.keys()) {
    const path = new Set<string>();
    let current: string | undefined = id;
    while (current && !resolved.has(current)) {
      path.add(current);
      const parent = parents.get(current);
      if (parent && path.has(parent)) {
        parents.delete(current);
        break;
      }
      current = parent;
    }
    for (const visited of path) resolved.add(visited);
  }
  const roots: SessionNode[] = [];
  for (const [id, node] of nodes) {
    const ancestors: string[] = [];
    let parent = parents.get(id);
    while (parent) {
      ancestors.push(parent);
      parent = parents.get(parent);
    }
    const displayParent = ancestors.length > 1 ? ancestors[ancestors.length - 2] : ancestors[0];
    if (displayParent) nodes.get(displayParent)!.children.push(node);
    else roots.push(node);
  }
  return roots;
}

export type CatalogRow =
  | {
    kind: "workspace";
    key: string;
    workspace: RemoteWorkspace;
    /**
     * The id the machine that owns this workspace knows it by.
     *
     * `workspace.id` is the key the list *folds* by, and a peer's carries its
     * machine so one fold cannot close two machines' same-named workspace. This
     * is the id a command has to name on the machine that owns it, which is the
     * plain workspace id in both lists — the fold key is a drawing concern.
     */
    workspaceId: string;
    count: number;
    /**
     * Which desktop the workspace lives on, in the merged list.
     *
     * Absent for the ordinary single-desktop list, where every heading is the
     * active desktop's and every action on it is addressed to that desktop.
     */
    desktopId?: string;
  }
  /**
   * A caption inside the list rather than a row in it: the recency page's two
   * stretches ("active", then the rest).
   *
   * It is a row of the same list on purpose — the FlatList scrolls it with the
   * conversations, and a sticky header would be a second scrolling structure for
   * two short captions.
   */
  | {
    kind: "section";
    key: string;
    /** What the caption says; the component translates it. */
    titleKey: "sessions.activeSection" | "sessions.earlierSection";
    /** How many conversations hang under it, as drawn. */
    count: number;
  }
  | {
    kind: "session";
    key: string;
    session: RemoteSession;
    depth: number;
    hasChildren: boolean;
    /**
     * What this row's fold is keyed by.
     *
     * A conversation on the machine the phone is talking to folds by its session
     * id — the key the single-machine list already folds by, so one conversation
     * folds the same in both lists. A peer's carries its machine, because a
     * session id is that machine's, not global: two machines can each hold a
     * conversation called `s1`, and one fold must not close both. The same rule
     * the merged list's workspace headings follow, for the same reason.
     */
    foldId: string;
    /**
     * Which desktop the session lives on, when the list is merged.
     *
     * Absent in the ordinary single-desktop list, where every row is the active
     * desktop's and a label would be noise. Present in the merged list, where a
     * row that lost its source would be opened against the wrong machine.
     */
    desktopId?: string;
    /**
     * Which stretch of the recency page a row belongs to.
     *
     * Only that page sets it: its two stretches are drawn by one row renderer,
     * and the screenshot scenario addresses rows by it (how many the window
     * draws). The other two pages have no such split.
     */
    section?: "active" | "earlier";
  };

/** One workspace's rows on one machine, as the merged list files them. */
export interface MergedWorkspaceGroup {
  /** Unique across machines: the same workspace id on two machines is two groups. */
  key: string;
  /** The machine the group belongs to, or null for the chat section. */
  desktopId: string | null;
  kind: "workspace" | "chat";
  /** The workspace's name, from whichever row in the group carried it. */
  label: string | null;
  workspaceId: string | null;
  /** Pinned *on its own machine*, which is what leads the merged list. */
  pinned: boolean;
  /** When the workspace itself was last touched at its source. */
  orderedAt: number | null;
  rows: MergedSessionRow[];
}

/**
 * The merged list's sections: pinned shortcuts, then a group per workspace, then
 * the conversations that live in no workspace.
 *
 * The shape is the desktop app's, on purpose — the same sessions are shown in
 * both, and a phone that filed them differently would make one machine's
 * conversation appear in two places depending on the device holding it. A group
 * is keyed by `(machine, workspace)`: a workspace called "future-os" exists on
 * two machines at once, and merging them would file one machine's rows under the
 * other's heading.
 *
 * Every group fact (name, pin, place) rides on its rows — the host reports them
 * per row — and one row reporting it is the group's answer: a host that predates
 * a field must not blank what a newer row in the same group supplied.
 */
export function groupMergedRows(
  rows: readonly MergedSessionRow[],
): { pinned: MergedSessionRow[]; workspaces: MergedWorkspaceGroup[]; chats: MergedSessionRow[] } {
  const pinned: MergedSessionRow[] = [];
  const byKey = new Map<string, MergedWorkspaceGroup>();
  for (const row of rows) {
    if (row.session.pinned) {
      pinned.push(row);
      continue;
    }
    // A chat-mode conversation has no workspace of its own: the list shows one
    // section for all of them, whatever machine they are on.
    const grouped = row.session.mode === "workspace";
    const workspaceId = row.session.workspaceId ?? "";
    const key = grouped ? `${row.desktopId}::${workspaceId}` : "chat";
    const existing = byKey.get(key);
    const group: MergedWorkspaceGroup = existing ?? {
      key,
      desktopId: grouped ? row.desktopId : null,
      kind: grouped ? "workspace" : "chat",
      label: null,
      workspaceId: grouped ? workspaceId : null,
      pinned: false,
      orderedAt: null,
      rows: [],
    };
    group.label ??= row.session.workspaceName ?? null;
    group.pinned ||= grouped && row.session.workspacePinned === true;
    group.orderedAt ??= row.session.workspaceOrderedAt ?? null;
    group.rows.push(row);
    if (!existing) byKey.set(key, group);
  }
  const groups = [...byKey.values()];
  groups.sort((a, b) => {
    // Pinned groups first, as in the single-machine list; then the workspace's
    // own place, newest first, so a group holds its rank while the reader works
    // in it — a conversation arriving inside a workspace must not move it.
    if (a.pinned !== b.pinned) return a.pinned ? -1 : 1;
    const at = a.orderedAt ?? Number.NEGATIVE_INFINITY;
    const bt = b.orderedAt ?? Number.NEGATIVE_INFINITY;
    if (at !== bt) return bt - at;
    if (a.desktopId !== b.desktopId)
      return (a.desktopId ?? "") < (b.desktopId ?? "") ? -1 : 1;
    return a.key < b.key ? -1 : a.key > b.key ? 1 : 0;
  });
  const chats = groups.find(group => group.kind === "chat");
  return {
    pinned,
    workspaces: groups.filter(group => group.kind === "workspace"),
    chats: chats ? chats.rows : [],
  };
}

/**
 * The merged list as rows: a heading per workspace, with its conversations under
 * it, and each conversation's forks under *that*.
 *
 * The heading carries the group key as its workspace id for a machine other than
 * the active one: a fold must not make two machines' "future-os" close together,
 * and the active machine keeps its plain id so that folding there behaves
 * exactly as it does in the single-machine list.
 *
 * `tab` decides which conversations the list is about, exactly as it does in the
 * single-machine list: the workspaces page holds the conversations that live in
 * a workspace, the conversations page holds the ones that live in none. A merged
 * list that showed both on both pages would put a machine's chats under the
 * workspace groups on the page that has no such section, and make the two pages
 * the same list.
 *
 * The tree is the single-machine list's, per machine (see `mergedTree`): a fork
 * is drawn under the conversation it came from, one level in and folded by the
 * same `+`/`−`. A merged list is not a list of roots — the machines' rows carry
 * the same parents here as they do there — and drawing them flat left a fork
 * indistinguishable from a conversation of its own, with no way to fold a
 * conversation that had thirty of them.
 */
/**
 * The archived view's rows, as one flat list of two sections.
 *
 * Deliberately not a tree: an archived workspace and an archived conversation are
 * two shelves, and the desktop's archived view draws them that way — a workspace
 * heading with its conversations under it, then the shelved conversations that
 * belong to no workspace at all. Filing them back into the normal workspace tree
 * would make "archived" look like a status of the tree rather than a place the
 * reader goes to.
 *
 * A conversation shelved out of a workspace that is still in use keeps that
 * workspace's heading: the heading is what says where a restore goes back to.
 * The heading earns its place from the shelved rows under it, so an active
 * workspace with nothing shelved is not drawn at all.
 *
 * The rows come from the same catalogue the default pages read (the host keeps
 * archived rows in it), narrowed by the caller to the archived half.
 */
export function archivedRows(
  sessions: RemoteSession[],
  workspaces: RemoteWorkspace[],
  onThisMachine: (session: RemoteSession) => boolean,
  /**
   * The fold set, shared with the default pages' workspace headings.
   *
   * One workspace folds the same whichever shelf it is sitting on: a reader who
   * closed "future-os" in the active list does not find it open again after
   * following the archive entry.
   */
  collapsed: ReadonlySet<string> = new Set(),
  /**
   * The machine these rows belong to, for a shelf that holds several.
   *
   * Stamped onto every row (the list's row renderer draws the machine's glyph
   * and routes an action by it) and into the row keys: a session id is that
   * machine's, so two machines can each hold a conversation called `s1`, and one
   * FlatList cannot hold two rows with one key.
   */
  desktopId?: string,
): CatalogRow[] {
  const rows: CatalogRow[] = [];
  const stamp = desktopId ? { desktopId } : {};
  const rowKey = (sessionId: string) => (desktopId ? sessionKey(desktopId, sessionId) : sessionId);
  const byWorkspace = new Map<string, RemoteSession[]>();
  const loose: RemoteSession[] = [];
  for (const session of sessions) {
    if (!onThisMachine(session)) continue;
    // A conversation is archived with its workspace even when its own thread is
    // still active; the host marks the row, so this is one comparison.
    if (session.archived !== true) continue;
    const workspaceId = session.mode === "workspace" ? session.workspaceId ?? "" : "";
    const workspace = workspaceId
      ? workspaces.find(item => item.id === workspaceId)
      : undefined;
    if (workspace) {
      const list = byWorkspace.get(workspace.id) ?? [];
      list.push(session);
      byWorkspace.set(workspace.id, list);
    }
    else {
      loose.push(session);
    }
  }
  // A shelved workspace keeps its heading even with nothing left inside it — its
  // one action is restore. A workspace still in the active list earns a heading
  // only because some of its conversations are shelved: that is the same
  // grouping the desktop's archived view draws, so one conversation does not
  // look like it came from nowhere.
  const archivedWorkspaces = workspaces.filter(workspace => workspace.archived === true);
  const activeShelves = workspaces.filter(workspace =>
    workspace.archived !== true && (byWorkspace.get(workspace.id)?.length ?? 0) > 0);
  for (const workspace of [...archivedWorkspaces, ...activeShelves]) {
    const children = byWorkspace.get(workspace.id) ?? [];
    rows.push({
      kind: "workspace",
      key: `archived-workspace:${desktopId ? `${desktopId}:` : ""}${workspace.id}`,
      workspace,
      workspaceId: workspace.id,
      count: children.length,
      ...stamp,
    });
    // A folded shelved workspace keeps its heading and its count, exactly as a
    // folded group does on the active pages.
    if (collapsed.has(workspace.id)) continue;
    for (const session of children) {
      rows.push({
        kind: "session",
        key: rowKey(session.sessionId),
        session,
        depth: 1,
        hasChildren: false,
        foldId: rowKey(session.sessionId),
        ...stamp,
      });
    }
  }
  for (const session of loose) {
    rows.push({
      kind: "session",
      key: rowKey(session.sessionId),
      session,
      depth: 0,
      hasChildren: false,
      foldId: rowKey(session.sessionId),
      ...stamp,
    });
  }
  return rows;
}

/**
 * The archived rows, narrowed to a search query.
 *
 * A shelved workspace survives on a match of its own name (then keeps all its
 * conversations, since the reader asked for the shelf) or because some of its
 * conversations match; a loose conversation is judged on its own name. Folded
 * shelves are already open by the time this runs — the caller drops the fold set
 * while searching — because a match hidden inside a closed group is a match the
 * reader cannot see.
 */
export function filterArchivedRows(rows: CatalogRow[], query: string): CatalogRow[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return rows;
  const matches = (title: string | undefined) =>
    (title ?? "").toLowerCase().includes(needle);
  const out: CatalogRow[] = [];
  const loose: Extract<CatalogRow, { kind: "session" }>[] = [];
  let index = 0;
  while (index < rows.length) {
    const head = rows[index];
    if (!head) break;
    if (head.kind === "session") {
      // A depth-0 row belongs to no shelf: it is one of the loose ones, and is
      // judged on its own name below.
      if (head.depth === 0) loose.push(head);
      index += 1;
      continue;
    }
    // A section caption belongs to the recency page, never to this shelf.
    if (head.kind !== "workspace") {
      index += 1;
      continue;
    }
    // The shelf's own conversations are the depth-1 rows after it, up to the
    // next heading — exactly the block `archivedRows` put there.
    const children: Extract<CatalogRow, { kind: "session" }>[] = [];
    let cursor = index + 1;
    for (; cursor < rows.length; cursor += 1) {
      const child = rows[cursor];
      if (!child || child.kind !== "session" || child.depth === 0) break;
      children.push(child);
    }
    index = cursor;
    const kept = matches(head.workspace.name)
      ? children
      : children.filter(child => matches(child.session.title));
    // A shelf with no conversation left is not drawn: a heading over nothing
    // reads as a shelf that was emptied, which is a different statement from
    // "nothing here matches".
    if (kept.length > 0) out.push(head, ...kept);
  }
  for (const row of loose) {
    if (matches(row.session.title)) out.push(row);
  }
  return out;
}

/**
 * The archived view's window: at most `limit` conversations, whole shelves.
 *
 * Paged by conversations rather than by rows because a shelf's heading is not
 * something the reader is reading — it is what the conversations under it are
 * filed under — and a window that cut between a heading and its own contents
 * would show a shelf holding nothing. So a block (a heading and its
 * conversations, or one loose conversation) is either entirely in or entirely
 * out. The first block is always drawn, however large: a page that showed
 * nothing would leave the reader no footer to open.
 */
export function pageArchivedRows(
  rows: CatalogRow[],
  limit: number,
): { rows: CatalogRow[]; more: number } {
  let shown = 0;
  let cut = 0;
  let index = 0;
  while (index < rows.length) {
    const head = rows[index];
    if (!head) break;
    let end = index + 1;
    let size = head.kind === "session" ? 1 : 0;
    for (; end < rows.length; end += 1) {
      const child = rows[end];
      if (!child || child.kind !== "session" || child.depth === 0) break;
      size += 1;
    }
    // A shelf holding no conversations is passed over rather than drawn as an
    // empty heading; it has nothing to page.
    if (size === 0 && head.kind === "workspace") {
      index = end;
      continue;
    }
    if (shown > 0 && shown + size > limit) break;
    shown += size;
    cut = end;
    index = end;
  }
  const more = rows.slice(cut).filter(row => row.kind === "session").length;
  return { rows: rows.slice(0, cut), more };
}

export function mergedRows(
  rows: readonly MergedSessionRow[],
  query: string,
  activeDesktopId: string | null,
  tab: "chat" | "workspace",
  collapsed: ReadonlySet<string> = new Set(),
  /**
   * Which conversations have their forks on screen, by session id.
   *
   * The same state the single-machine list folds by, and for the same reason:
   * it is the same fork. One conversation therefore opens and closes together
   * whichever list is drawing it, instead of a fold that depends on where the
   * reader happens to be looking.
   */
  expanded: ReadonlySet<string> = new Set(),
): CatalogRow[] {
  const search = query.trim().toLocaleLowerCase();
  const matches = (row: (typeof rows)[number]): boolean =>
    !search ||
    row.session.title.toLocaleLowerCase().includes(search) ||
    (row.session.workspaceName ?? "").toLocaleLowerCase().includes(search);
  // A session with no `mode` is a chat, as it is in the single-machine list:
  // the field was added after chat conversations existed.
  const onThisPage = (row: (typeof rows)[number]): boolean =>
    tab === "workspace"
      ? row.session.mode === "workspace"
      : row.session.mode !== "workspace";
  const listed = rows.filter(row => matches(row) && onThisPage(row));
  const { children, rootOf } = mergedTree(listed);
  const sessionRow = (
    row: (typeof rows)[number],
    depth: number,
    hasChildren: boolean,
  ): Extract<CatalogRow, { kind: "session" }> => ({
    kind: "session" as const,
    key: row.key,
    session: row.session,
    depth,
    hasChildren,
    foldId: row.desktopId === activeDesktopId ? row.session.sessionId : row.key,
    desktopId: row.desktopId,
  });
  /**
   * A conversation and the forks under it, parents before children.
   *
   * A fork whose conversation this page does not hold — one filed on the other
   * page, or hidden by the search — is a root, exactly as a conversation whose
   * parent is gone is: `mergedTree` hangs nothing under a row that is not here.
   * The fold is the single-machine list's, and a search ignores it, so a hidden
   * fork stays discoverable at the top level — how that list answers the same
   * question.
   */
  const block = (row: (typeof rows)[number], depth = 0): CatalogRow[] => {
    const forks = children.get(row.key) ?? [];
    const head = sessionRow(row, search ? 0 : depth, !search && forks.length > 0);
    // A fork sits under its conversation, which is what puts it under the same
    // heading.
    if (!search && !expanded.has(head.foldId)) return [head];
    return [head, ...forks.flatMap(fork => block(fork, depth + 1))];
  };
  /** Everything under a conversation, folded ones included — a heading counts them. */
  const subtree = (row: (typeof rows)[number]): number =>
    1 + (children.get(row.key) ?? []).reduce((total, fork) => total + subtree(fork), 0);
  // Sections and their order are decided by the *conversation* each row hangs
  // under — its root — rather than by the fork's own fields, so a fork follows
  // the conversation it came from: the forks of a pinned conversation are part
  // of that shortcut, and one forked inside a workspace is under that heading.
  // A fork is younger than its parent by construction, so filing it by its own
  // fields or time would put it above the row the reader just unfolded.
  const roots = listed.filter(
    row => (rootOf.get(row.key) ?? row).key === row.key,
  );
  const { pinned, workspaces, chats } = groupMergedRows(roots);
  const out: CatalogRow[] = [];
  // Pinned conversations are shortcuts promoted above the groups, exactly as in
  // the single-machine list.
  for (const row of pinned) out.push(...block(row));
  for (const group of workspaces) {
    const workspaceId = group.workspaceId ?? "";
    // This machine's heading folds by the workspace id alone — the key the
    // single-machine list already folds by, so one workspace folds the same in
    // both lists. A peer's needs its machine in the key: two machines can hold
    // the same workspace id, and one fold must not close both.
    const id = group.desktopId === activeDesktopId ? workspaceId : group.key;
    out.push({
      kind: "workspace",
      key: `merged:${group.key}`,
      // The group's pin is one of the facts its rows reported, and the heading
      // is what draws it: the marker reads `workspace.pinned`, and so does the
      // group menu, which offers "pin" or "unpin" off the same flag. A heading
      // built without it read as an unpinned group everywhere the group fact is
      // *shown* — no marker, and a menu that pinned an already-pinned workspace
      // again, so a pin made on this machine could not be undone in the merged
      // list — while the group was still placed as pinned.
      workspace: { id, name: group.label ?? workspaceId, path: "", pinned: group.pinned },
      workspaceId,
      count: group.rows.reduce((total, row) => total + subtree(row), 0),
      ...(group.desktopId ? { desktopId: group.desktopId } : {}),
    });
    // A folded group keeps its heading and its count and hides its rows — and a
    // search ignores folds, so a hidden conversation stays discoverable (the
    // same rule the single-machine list follows).
    if (search || !collapsed.has(id)) {
      for (const row of group.rows) out.push(...block(row));
    }
  }
  for (const row of chats) out.push(...block(row));
  return out;
}

/**
 * Where each listed row hangs, keyed by the row's own key.
 *
 * Built one machine at a time, because a parent is named by a session id that
 * is only meaningful on the machine that issued it: two machines can each hold
 * a conversation called `s1`, and one machine's row must not adopt the other's
 * fork — the same reason a merged row is addressed by `(machine, id)` at all.
 *
 * `buildSessionTree` is the single-machine list's own tree, reused rather than
 * restated: "pinned conversations are roots, a parent that is not listed makes
 * its child a root, a cycle loses one edge" is one relation, and a second
 * implementation of it would be a second answer to "whose fork is this". This
 * is only the mapping from its session-shaped nodes back to the merged rows,
 * which are what carry the machine.
 */
function mergedTree(listed: readonly MergedSessionRow[]): {
  /** A conversation's forks, in the order the catalogue lists them. */
  children: Map<string, MergedSessionRow[]>;
  /** The conversation each row hangs under, itself when it hangs under none. */
  rootOf: Map<string, MergedSessionRow>;
} {
  const byKey = new Map(listed.map(row => [row.key, row]));
  const byDesktop = new Map<string, MergedSessionRow[]>();
  for (const row of listed) {
    const machine = byDesktop.get(row.desktopId);
    if (machine) machine.push(row);
    else byDesktop.set(row.desktopId, [row]);
  }
  const children = new Map<string, MergedSessionRow[]>();
  const rootOf = new Map<string, MergedSessionRow>();
  for (const [desktopId, machineRows] of byDesktop) {
    const walk = (nodes: SessionNode[], root: MergedSessionRow | null): void => {
      for (const node of nodes) {
        const row = byKey.get(sessionKey(desktopId, node.session.sessionId));
        if (!row) continue;
        const blockRoot = root ?? row;
        rootOf.set(row.key, blockRoot);
        const forks = node.children
          .map(child => byKey.get(sessionKey(desktopId, child.session.sessionId)))
          .filter((fork): fork is MergedSessionRow => fork !== undefined);
        if (forks.length > 0) children.set(row.key, forks);
        walk(node.children, blockRoot);
      }
    };
    walk(buildSessionTree(machineRows.map(row => row.session)), null);
  }
  return { children, rootOf };
}

export function catalogRows(
  sessions: RemoteSession[],
  workspaces: RemoteWorkspace[],
  tab: "chat" | "workspace",
  collapsed: Set<string>,
  expanded: Set<string>,
  query: string,
): CatalogRow[] {
  const search = query.trim().toLocaleLowerCase();
  const roots = buildSessionTree(sessions);
  const rows: CatalogRow[] = [];
  const append = (nodes: SessionNode[], depth: number, workspaceMatches = false) => {
    for (const node of nodes) {
      // Search is flat and ignores folds, so a hidden child is still discoverable.
      const matches =
        !search || workspaceMatches || node.session.title.toLocaleLowerCase().includes(search);
      if (matches)
        rows.push({
          kind: "session",
          key: node.session.sessionId,
          session: node.session,
          depth: search ? 0 : depth,
          hasChildren: !search && node.children.length > 0,
          // One machine's list: the session id is the whole identity.
          foldId: node.session.sessionId,
        });
      if (search || expanded.has(node.session.sessionId))
        append(node.children, depth + 1, workspaceMatches);
    }
  };
  if (tab === "chat") {
    append(
      roots.filter(node => node.session.mode !== "workspace"),
      0,
    );
    return rows;
  }
  const workspaceRoots = roots.filter(node => node.session.mode === "workspace");
  const groups = [...workspaces];
  // Retain sessions whose workspace disappeared rather than silently hiding them.
  for (const node of workspaceRoots) {
    const id = node.session.workspaceId ?? "";
    if (!groups.some(workspace => workspace.id === id)) groups.push({ id, name: id, path: "" });
  }
  const matchesWorkspace = (workspace: RemoteWorkspace): boolean =>
    !!search && `${workspace.name} ${workspace.path}`.toLocaleLowerCase().includes(search);
  // The catalog already supplies recency order. Keep that order among pins,
  // but promote them (with their expandable children) above every workspace.
  // Workspace folds must not hide these shortcuts.
  for (const node of workspaceRoots) {
    if (!node.session.pinned) continue;
    const workspace = groups.find(group => group.id === (node.session.workspaceId ?? ""))!;
    append([node], 0, matchesWorkspace(workspace));
  }
  // Pinned workspaces follow those pinned conversations and lead the rest of
  // the groups; within each half the desktop's recency order is kept.
  const orderedGroups = [
    ...groups.filter(workspace => workspace.pinned),
    ...groups.filter(workspace => !workspace.pinned),
  ];
  const countNodes = (nodes: SessionNode[]): number =>
    nodes.reduce((total, node) => total + 1 + countNodes(node.children), 0);
  for (const workspace of orderedGroups) {
    const children = workspaceRoots.filter(
      node => (node.session.workspaceId ?? "") === workspace.id,
    );
    const workspaceMatches = matchesWorkspace(workspace);
    const header: CatalogRow = {
      kind: "workspace",
      key: `workspace:${workspace.id}`,
      workspace,
      workspaceId: workspace.id,
      count: countNodes(children),
    };
    const start = rows.length;
    rows.push(header);
    // Counts and workspace actions still include promoted pins; display them
    // only once, at the page top, rather than duplicating them in the group.
    if (search || !collapsed.has(workspace.id))
      append(children.filter(node => !node.session.pinned), 0, workspaceMatches);
    if (search && !workspaceMatches && rows.length === start + 1) rows.pop();
  }
  return rows;
}
