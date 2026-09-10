import { useCallback, useEffect, useState } from "react";

/** Workspace groups the user collapsed in the activity rail. */
const STORAGE_KEY = "future.collapsedWorkspaces";

/** Read the stored collapsed workspace ids; storage is best effort. */
function readCollapsedWorkspaces(): Set<string> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    const parsed: unknown = raw === null ? [] : JSON.parse(raw);
    if (!Array.isArray(parsed))
      return new Set();
    return new Set(parsed.filter(id => typeof id === "string"));
  }
  catch {
    // Unavailable or corrupt storage — start fully expanded.
    return new Set();
  }
}

/**
 * Collapse state of the activity rail's workspace groups, persisted so a
 * restart brings the rail back the way the user left it. Ids of deleted
 * workspaces are harmless: they never match a rendered group.
 */
export function useCollapsedWorkspaces() {
  const [collapsedWorkspaces, setCollapsedWorkspaces] = useState<Set<string>>(readCollapsedWorkspaces);

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify([...collapsedWorkspaces]));
    }
    catch { /* Storage is best effort. */ }
  }, [collapsedWorkspaces]);

  const toggleWorkspaceCollapsed = useCallback((workspaceId: string) => {
    setCollapsedWorkspaces((current) => {
      const next = new Set(current);
      if (next.has(workspaceId))
        next.delete(workspaceId);
      else next.add(workspaceId);
      return next;
    });
  }, []);

  return { collapsedWorkspaces, toggleWorkspaceCollapsed };
}
