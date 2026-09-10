import AsyncStorage from "@react-native-async-storage/async-storage";
import { useCallback, useEffect, useRef, useState } from "react";

/**
 * Workspace groups the user folded shut in the workspace tab. Desktop parity:
 * the activity rail persists the same set, so a restart brings the list back
 * the way the user left it.
 */
const STORAGE_KEY = "futureos.mobile.collapsed-workspaces.v1";

/**
 * In-session cache. `SessionList` is unmounted whenever the user opens a
 * conversation and remounted on return, and AsyncStorage has no synchronous
 * read — without this the list would render fully expanded for a frame on
 * every return. `null` means "never hydrated in this process".
 */
let remembered: Set<string> | null = null;

/** Folded ids a synchronous render can use before storage resolves. */
export function rememberedCollapsedWorkspaces(): Set<string> {
  return remembered ?? new Set();
}

/**
 * Parse a stored value into ids. Storage is best effort: anything corrupt,
 * non-array, or holding non-string members reads as "nothing collapsed"
 * rather than throwing during render.
 */
export function parseCollapsedWorkspaces(raw: string | null): Set<string> {
  try {
    const parsed: unknown = raw === null ? [] : JSON.parse(raw);
    if (!Array.isArray(parsed)) return new Set();
    return new Set(parsed.filter((id): id is string => typeof id === "string"));
  } catch {
    return new Set();
  }
}

export interface CollapsedWorkspacesApi {
  collapsed: Set<string>;
  toggleWorkspaceCollapsed(workspaceId: string): void;
}

/**
 * Create/delete state of the workspace groups, persisted across restarts.
 * Ids of deleted workspaces are harmless — they never match a rendered group.
 */
export function useCollapsedWorkspaces(): CollapsedWorkspacesApi {
  const [collapsed, setCollapsed] = useState<Set<string>>(rememberedCollapsedWorkspaces);
  const [hydrated, setHydrated] = useState(false);
  // A toggle that lands before the storage read resolves must win over the
  // stored value, otherwise the user's tap is undone a few ms later.
  const touchedRef = useRef(false);

  useEffect(() => {
    let active = true;
    void AsyncStorage.getItem(STORAGE_KEY)
      .then(raw => {
        if (!active) return;
        setCollapsed(current => {
          if (touchedRef.current) return current;
          return parseCollapsedWorkspaces(raw);
        });
      })
      .catch(() => undefined)
      .finally(() => {
        if (active) setHydrated(true);
      });
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    remembered = collapsed;
    // Persist only after hydration: writing the initial empty set first would
    // erase what is on disk before we ever read it.
    if (!hydrated) return;
    void AsyncStorage.setItem(STORAGE_KEY, JSON.stringify([...collapsed])).catch(() => undefined);
  }, [collapsed, hydrated]);

  const toggleWorkspaceCollapsed = useCallback((workspaceId: string) => {
    touchedRef.current = true;
    setCollapsed(current => {
      const next = new Set(current);
      if (next.has(workspaceId)) next.delete(workspaceId);
      else next.add(workspaceId);
      return next;
    });
  }, []);

  return { collapsed, toggleWorkspaceCollapsed };
}
