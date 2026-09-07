import type { StoredThread } from "../../integrations/storage/threadStore";

export interface ThreadTreeNode {
  thread: StoredThread;
  children: ThreadTreeNode[];
}

/**
 * Build once before section grouping: descendants follow their root's section.
 * Pinned children become roots in the pinned section. Missing/archived parents
 * become roots too. Break malformed cycles and flatten depth > 3 to level 3,
 * without losing a conversation or changing the Agent's authoritative lineage.
 */
export function buildThreadTree(threads: StoredThread[]): ThreadTreeNode[] {
  const bySession = new Map(threads.filter(t => t.agentSessionId).map(t => [t.agentSessionId, t]));
  const parents = new Map<string, string>();
  for (const thread of threads) {
    const parent = thread.parentSessionId ? bySession.get(thread.parentSessionId) : undefined;
    if (!thread.pinned && parent && parent.id !== thread.id)
      parents.set(thread.id, parent.id);
  }
  // Break one edge per cycle, including cycles disconnected from every root.
  const resolved = new Set<string>();
  for (const thread of threads) {
    const path = new Set<string>();
    let id: string | undefined = thread.id;
    while (id && !resolved.has(id)) {
      path.add(id);
      const parent = parents.get(id);
      if (parent && path.has(parent)) {
        parents.delete(id);
        break;
      }
      id = parent;
    }
    for (const visited of path) resolved.add(visited);
  }
  const nodes = new Map(threads.map(thread => [thread.id, { thread, children: [] } as ThreadTreeNode]));
  const roots: ThreadTreeNode[] = [];
  for (const thread of threads) {
    const node = nodes.get(thread.id)!;
    const ancestors: string[] = [];
    let parent = parents.get(thread.id);
    while (parent) {
      ancestors.push(parent);
      parent = parents.get(parent);
    }
    // The last ancestor is level 1; attach to level 2 at most.
    const displayParent = ancestors.length > 1 ? ancestors[ancestors.length - 2] : ancestors[0];
    if (displayParent)
      nodes.get(displayParent)!.children.push(node);
    else
      roots.push(node);
  }
  return roots;
}

export interface ThreadTreeRow {
  thread: StoredThread;
  depth: number;
  hasChildren: boolean;
}

export function visibleThreadRows(nodes: ThreadTreeNode[], expanded: Set<string>, depth = 0): ThreadTreeRow[] {
  return nodes.flatMap(node => [
    { thread: node.thread, depth, hasChildren: node.children.length > 0 },
    ...(expanded.has(node.thread.id) ? visibleThreadRows(node.children, expanded, depth + 1) : []),
  ]);
}
