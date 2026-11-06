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
  /**
   * Roots whose read failed because the directory is not there yet — a chat
   * conversation's workspace is created by its first message, so a conversation
   * nobody has messaged has no folder to list. Kept apart from `errored`: the
   * panel says an empty folder, not a read failure.
   */
  missing: Set<string>;
  /** Latest load generation per directory; prevents stale response overwrite. */
  generations: Map<string, number>;
}

const cache = new Map<string, TreeState>();
const CACHE_MAX_ROOTS = 20;

function stateFor(root: string): TreeState {
  let state = cache.get(root);
  if (!state) {
    state = {
      expanded: new Set(),
      children: new Map(),
      loading: new Set(),
      errored: new Set(),
      missing: new Set(),
      generations: new Map(),
    };
    cache.set(root, state);
  }
  else {
    // Refresh insertion order so the module cache behaves as an LRU.
    cache.delete(root);
    cache.set(root, state);
  }
  while (cache.size > CACHE_MAX_ROOTS) {
    const oldest = cache.keys().next().value;
    if (!oldest)
      break;
    cache.delete(oldest);
  }
  return state;
}

export interface FileTree {
  /** Entries directly under the root, or null until the first load resolves. */
  rootEntries: DirEntry[] | null;
  rootLoading: boolean;
  rootErrored: boolean;
  /** The root directory does not exist yet (nothing has been written to it). */
  rootMissing: boolean;
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
 * Whether a failed root read means "the directory is not there yet" rather than
 * "the read failed".
 *
 * A chat conversation's workspace directory is created by its first message, so
 * a conversation nobody has messaged has a path that does not exist — and
 * `read_dir`, and therefore the host's `list_directory`, reports that as an
 * error like any other. The panel's copy for that state is its empty card ("no
 * files yet"), so the two have to be told apart.
 *
 * The only thing the host hands back is the OS message, which is localized
 * (Windows formats its own text and keeps only the `(os error N)` suffix), so
 * ask the filesystem instead: a directory that its parent lists exists, and one
 * that it does not is missing. Everything unanswerable — a root with no parent
 * to ask, or a parent that is unreadable too — stays an error.
 */
async function isMissingDirectory(path: string): Promise<boolean> {
  const split = parentOf(path);
  if (!split)
    return false;
  try {
    const siblings = await listDirectory(split.parent);
    return !siblings.some(entry =>
      entry.name === split.name
      // A listing comes back in the filesystem's own casing, and a
      // case-insensitive volume may have normalized it: the same directory under
      // another case is present, and calling it missing would swap a real
      // failure for a false empty state.
      || entry.name.toLowerCase() === split.name.toLowerCase());
  }
  catch {
    return false;
  }
}

/** Split a root into the directory that would list it and its own name. */
function parentOf(path: string): { parent: string; name: string } | null {
  const trimmed = path.trim().replace(/[\\/]+$/, "");
  const index = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  // No separator, or one with nothing before it (`/` itself): no directory to ask.
  if (index <= 0 || index === trimmed.length - 1)
    return null;
  const parent = trimmed.slice(0, index);
  return {
    // A bare `C:` is drive-relative on Windows — the *current* directory on that
    // drive, not its root.
    parent: /^[a-z]:$/i.test(parent) ? `${parent}${trimmed.charAt(index)}` : parent,
    name: trimmed.slice(index + 1),
  };
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
    const generation = (state.generations.get(path) ?? 0) + 1;
    state.generations.set(path, generation);
    state.loading.add(path);
    state.errored.delete(path);
    state.missing.delete(path);
    bump();
    try {
      const entries = await listDirectory(path);
      if (state.generations.get(path) === generation)
        state.children.set(path, entries);
    }
    catch {
      // Only the root can be a directory that does not exist yet, and only when
      // it has nothing to show: ask its parent before calling the read a
      // failure (see `isMissingDirectory`). Keep the loading state up while the
      // parent answers, so a missing folder never flashes the danger box.
      const missing = path === root && await isMissingDirectory(path);
      if (state.generations.get(path) === generation) {
        if (missing)
          state.missing.add(path);
        else
          state.errored.add(path);
      }
    }
    finally {
      if (state.generations.get(path) === generation) {
        state.loading.delete(path);
        bump();
      }
    }
  }, [root, state]);

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
    rootMissing: root ? state?.missing.has(root) ?? false : false,
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
