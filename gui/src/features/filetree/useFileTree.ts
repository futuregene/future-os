import type { DirEntry } from "../../integrations/storage/files";
import { useCallback, useEffect, useReducer } from "react";
import { listDirectory } from "../../integrations/storage/files";

/**
 * Per-root tree state. Held in a module-level cache (below) so it survives the
 * panel unmounting on a tab switch, the 1.5s context poll re-rendering, and
 * switching threads within the same workspace — the user's expanded folders and
 * a manual refresh never collapse the tree. Keyed by the absolute root path.
 */
interface TreeState {
  /** Absolute paths of directories the user has expanded. */
  expanded: Set<string>;
  /** Loaded children per directory path (the lazy-load cache). */
  children: Map<string, DirEntry[]>;
  /** Directory paths with an in-flight `listDirectory`. */
  loading: Set<string>;
  /** Directory paths whose last load failed. */
  errored: Set<string>;
}

const cache = new Map<string, TreeState>();

function stateFor(root: string): TreeState {
  let state = cache.get(root);
  if (!state) {
    state = { expanded: new Set(), children: new Map(), loading: new Set(), errored: new Set() };
    cache.set(root, state);
  }
  return state;
}

export interface FileTree {
  /** Entries directly under the root, or null until the first load resolves. */
  rootEntries: DirEntry[] | null;
  rootLoading: boolean;
  rootErrored: boolean;
  isExpanded: (path: string) => boolean;
  childrenOf: (path: string) => DirEntry[] | null;
  isLoading: (path: string) => boolean;
  isErrored: (path: string) => boolean;
  /** Expand/collapse a directory; lazy-loads its children on first expand. */
  toggle: (path: string) => void;
  /** (Re)load a single directory's children — used to retry a failed load. */
  reload: (path: string) => void;
  /** Re-read the root and every expanded directory, preserving expansion. */
  refresh: () => Promise<void>;
}

/** A dotfile — hidden by default, revealed by the "show hidden" toggle. */
function isHidden(name: string): boolean {
  return name.startsWith(".");
}

/**
 * Lazy-loading file tree rooted at `root` (a workspace directory). Each level
 * loads on expand and is cached; expansion state persists across remounts via
 * the module-level cache. `showHidden` filters dotfiles from the returned views
 * (a display concern — the cache keeps every entry, so toggling never refetches).
 * Pass `null` for `root` when there's no active workspace.
 */
export function useFileTree(root: string | null, showHidden = false): FileTree {
  const [, bump] = useReducer((tick: number) => tick + 1, 0);
  const state = root ? stateFor(root) : null;
  const visible = (entries: DirEntry[]) =>
    showHidden ? entries : entries.filter(entry => !isHidden(entry.name));

  const load = useCallback(async (path: string) => {
    if (!state)
      return;
    state.loading.add(path);
    state.errored.delete(path);
    bump();
    try {
      const entries = await listDirectory(path);
      state.children.set(path, entries);
    }
    catch {
      state.errored.add(path);
    }
    finally {
      state.loading.delete(path);
      bump();
    }
  }, [state]);

  const toggle = useCallback((path: string) => {
    if (!state)
      return;
    if (state.expanded.has(path)) {
      state.expanded.delete(path);
    }
    else {
      state.expanded.add(path);
      if (!state.children.has(path) && !state.loading.has(path))
        void load(path);
    }
    bump();
  }, [state, load]);

  const refresh = useCallback(async () => {
    if (!state || !root)
      return;
    // Re-read the root plus every expanded directory (even ones hidden under a
    // collapsed ancestor — keeps their cache warm so re-expanding is instant).
    const targets = new Set<string>([root, ...state.expanded]);
    await Promise.all([...targets].map(path => load(path)));
  }, [state, root, load]);

  // On mount, and whenever the root changes: show any cached tree immediately
  // (rendered synchronously from the cache below), then revalidate the root and
  // every expanded directory in the background. So opening the tab always
  // converges to the current on-disk state — a warm root updates in place with
  // no blank, a cold root shows the loading state until its first read lands.
  useEffect(() => {
    if (root)
      void refresh();
  }, [root, refresh]);

  const rawRoot = root ? state?.children.get(root) ?? null : null;
  return {
    rootEntries: rawRoot ? visible(rawRoot) : null,
    rootLoading: root ? state?.loading.has(root) ?? false : false,
    rootErrored: root ? state?.errored.has(root) ?? false : false,
    isExpanded: path => state?.expanded.has(path) ?? false,
    childrenOf: (path) => {
      const raw = state?.children.get(path);
      return raw ? visible(raw) : null;
    },
    isLoading: path => state?.loading.has(path) ?? false,
    isErrored: path => state?.errored.has(path) ?? false,
    toggle,
    reload: path => void load(path),
    refresh,
  };
}
