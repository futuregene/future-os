import type { RemoteSession, RemoteWorkspace } from "../remote/types";

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
  | { kind: "workspace"; key: string; workspace: RemoteWorkspace; count: number }
  | { kind: "session"; key: string; session: RemoteSession; depth: number; hasChildren: boolean };

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
  const groups = [...workspaces];
  // Retain sessions whose workspace disappeared rather than silently hiding them.
  for (const node of roots) {
    if (node.session.mode !== "workspace") continue;
    const id = node.session.workspaceId ?? "";
    if (!groups.some(workspace => workspace.id === id)) groups.push({ id, name: id, path: "" });
  }
  const countNodes = (nodes: SessionNode[]): number =>
    nodes.reduce((total, node) => total + 1 + countNodes(node.children), 0);
  for (const workspace of groups) {
    const children = roots.filter(
      node =>
        node.session.mode === "workspace" && (node.session.workspaceId ?? "") === workspace.id,
    );
    const workspaceMatches =
      !!search && `${workspace.name} ${workspace.path}`.toLocaleLowerCase().includes(search);
    const header: CatalogRow = {
      kind: "workspace",
      key: `workspace:${workspace.id}`,
      workspace,
      count: countNodes(children),
    };
    const start = rows.length;
    rows.push(header);
    if (search || !collapsed.has(workspace.id)) append(children, 0, workspaceMatches);
    if (search && !workspaceMatches && rows.length === start + 1) rows.pop();
  }
  return rows;
}
