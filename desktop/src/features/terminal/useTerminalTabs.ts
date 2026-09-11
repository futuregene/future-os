import type { TerminalTab, TerminalTabsState } from "./tabs";
/**
 * Tab controller for a conversation's terminal panel.
 *
 * The server owns the shells; this hook owns which tabs the user sees for the
 * active conversation, keeps them in `localStorage` so a reload or a panel
 * collapse does not lose the view, and reconciles against the server list when
 * the panel opens. It never creates a shell on its own initiative except for
 * the documented "open the panel with no tabs" case.
 */
import type { CwdPolicy, TerminalInfo } from "./types";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  createTerminal,
  listTerminals,
  removeTerminal,
  TerminalApiError,
} from "./client";
import {
  clearTabsState,
  loadTabsState,
  nextTitleNumber,
  saveTabsState,
  selectTabAfterClose,

} from "./tabs";

/** Default label for a tab; the number is stable across restarts. */
export function defaultTitle(titleNumber: number): string {
  return `Terminal ${titleNumber}`;
}

export interface TerminalTabsController {
  tabs: TerminalTab[];
  activeId: string | undefined;
  activeTab: TerminalTab | undefined;
  /** The server list has been applied at least once. */
  ready: boolean;
  /**
   * Panel-level failure (the listener is unreachable); tab-level failures live
   * on the tab itself.
   */
  error: string | null;
  /** A create is in flight; the UI disables the "+" button. */
  creating: boolean;
  /** Why the last create failed, if it did: drives the fallback prompt. */
  createError: TerminalApiError | null;
  create: (policy?: CwdPolicy) => Promise<void>;
  /** Retry the last failed create with the same policy. */
  retry: () => Promise<void>;
  /** Confirm the explicit home fallback for the last failed create. */
  retryInHome: () => Promise<void>;
  dismissCreateError: () => void;
  close: (id: string) => Promise<void>;
  restart: (id: string) => Promise<void>;
  setActive: (id: string) => void;
  /** Persist view state reported by the mounted terminal. */
  save: (id: string, patch: Partial<TerminalTab>) => void;
  /** The shell exited: keep the final screen, offer a restart. */
  markExit: (id: string, exitCode: number | null) => void;
  /** The server no longer has this session. */
  markMissing: (id: string) => void;
}

function toTab(info: TerminalInfo, titleNumber: number): TerminalTab {
  return {
    id: info.id,
    title: info.title || defaultTitle(titleNumber),
    titleNumber,
    cols: info.cols || undefined,
    rows: info.rows || undefined,
    missing: false,
    ...(info.status === "exited" ? { exitCode: info.exitCode } : {}),
  };
}

/**
 * Merge the server's view of this conversation into the stored tabs.
 *
 * a stored tab the server still has keeps its screen and picks up the real
 *   exit code;
 * a stored tab the server no longer has is marked `missing` (the shell is
 *   gone — the view offers a restart) instead of silently disappearing;
 * a server session this client has never seen is adopted, so a running shell
 *   survives a reload that lost `localStorage` (the app is single-instance, so
 *   it cannot belong to another window).
 */
export function reconcileTabs(stored: TerminalTabsState, live: readonly TerminalInfo[]): TerminalTabsState {
  const liveById = new Map(live.map(info => [info.id, info]));
  const known = new Set(stored.all.map(tab => tab.id));
  const existing = stored.all.map((tab) => {
    const info = liveById.get(tab.id);
    if (!info)
      return { ...tab, missing: true };
    return {
      ...tab,
      title: info.title || tab.title,
      missing: false,
      ...(info.status === "exited" ? { exitCode: info.exitCode } : { exitCode: undefined }),
    };
  });
  const adoptedIds = new Set(known);
  const usedNumbers = new Set(existing.map(tab => tab.titleNumber).filter(number => number > 0));
  let counter = 1;
  const adopted = live
    .filter((info) => {
      if (adoptedIds.has(info.id))
        return false;
      adoptedIds.add(info.id);
      return true;
    })
    .map((info) => {
      while (usedNumbers.has(counter))
        counter += 1;
      const number = counter;
      usedNumbers.add(number);
      counter += 1;
      return toTab(info, number);
    });
  const all = [...existing, ...adopted];
  const active = stored.active && all.some(tab => tab.id === stored.active) ? stored.active : all[0]?.id;
  return { active, all };
}

export function useTerminalTabs(threadId: string | null): TerminalTabsController {
  const [state, setState] = useState<TerminalTabsState>(() => (threadId ? loadTabsState(threadId) : { all: [] }));
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<TerminalApiError | null>(null);
  const creatingRef = useRef(false);
  const lastPolicyRef = useRef<CwdPolicy>("thread");
  const stateRef = useRef(state);
  stateRef.current = state;
  const threadRef = useRef(threadId);
  threadRef.current = threadId;

  // Reload stored tabs whenever the conversation changes; the panel keeps one
  // controller per thread, so a switch must not leak the previous thread's tabs.
  useEffect(() => {
    if (!threadId) {
      setState({ all: [] });
      setReady(false);
      return;
    }
    setState(loadTabsState(threadId));
    setReady(false);
    setError(null);
    setCreateError(null);
  }, [threadId]);

  useEffect(() => {
    if (!threadId)
      return;
    let cancelled = false;
    void (async () => {
      try {
        const live = await listTerminals(threadId);
        if (cancelled || threadRef.current !== threadId)
          return;
        setState((previous) => {
          const merged = reconcileTabs(previous, live);
          saveTabsState(threadId, merged);
          return merged;
        });
        setError(null);
      }
      catch (cause) {
        if (cancelled)
          return;
        setError(cause instanceof Error ? cause.message : String(cause));
      }
      finally {
        if (!cancelled && threadRef.current === threadId)
          setReady(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [threadId]);

  const update = useCallback((mutate: (previous: TerminalTabsState) => TerminalTabsState) => {
    setState((previous) => {
      const next = mutate(previous);
      const currentThread = threadRef.current;
      if (currentThread)
        saveTabsState(currentThread, next);
      return next;
    });
  }, []);

  const create = useCallback(async (policy: CwdPolicy = "thread") => {
    const thread = threadRef.current;
    if (!thread || creatingRef.current)
      return;
    creatingRef.current = true;
    lastPolicyRef.current = policy;
    setCreating(true);
    try {
      const previous = stateRef.current;
      const titleNumber = nextTitleNumber(previous.all);
      const info = await createTerminal({ threadId: thread, title: defaultTitle(titleNumber), cwdPolicy: policy });
      update((current) => {
        const tab: TerminalTab = { ...toTab(info, titleNumber), title: defaultTitle(titleNumber) };
        return { active: tab.id, all: [...current.all, tab] };
      });
      setCreateError(null);
      setError(null);
    }
    catch (cause) {
      const apiError = cause instanceof TerminalApiError
        ? cause
        : new TerminalApiError("REQUEST_FAILED", cause instanceof Error ? cause.message : String(cause), false, 0);
      setCreateError(apiError);
    }
    finally {
      creatingRef.current = false;
      setCreating(false);
    }
  }, [update]);

  const retry = useCallback(() => create(lastPolicyRef.current), [create]);
  const retryInHome = useCallback(() => create("homeConfirmed"), [create]);

  const close = useCallback(async (id: string) => {
    update((current) => {
      const all = current.all.filter(tab => tab.id !== id);
      const active = current.active === id ? selectTabAfterClose(current.all, id) : current.active;
      return { active: active && all.some(tab => tab.id === active) ? active : all[0]?.id, all };
    });
    try {
      await removeTerminal(id);
    }
    catch {
      // The tab is gone from the UI either way: a session the server already
      // forgot (or cannot reach) must not leave a zombie tab behind.
    }
    // Drop any stored view state for the closed tab.
    const thread = threadRef.current;
    if (thread && stateRef.current.all.length === 0)
      clearTabsState(thread);
  }, [update]);

  const restart = useCallback(async (id: string) => {
    const thread = threadRef.current;
    if (!thread)
      return;
    const previous = stateRef.current;
    const tab = previous.all.find(entry => entry.id === id);
    if (!tab)
      return;
    try {
      // The old session is dead or missing; remove it first so the server does
      // not keep a poisoned entry around, then start a fresh shell with the
      // same label.
      await removeTerminal(id).catch(() => undefined);
      const info = await createTerminal({
        threadId: thread,
        title: tab.title || defaultTitle(tab.titleNumber || 1),
      });
      update(current => ({
        active: info.id,
        all: current.all.map(entry => (entry.id === id
          ? { ...toTab(info, entry.titleNumber || 1), title: entry.title || defaultTitle(entry.titleNumber || 1) }
          : entry)),
      }));
    }
    catch (cause) {
      const apiError = cause instanceof TerminalApiError
        ? cause
        : new TerminalApiError("REQUEST_FAILED", cause instanceof Error ? cause.message : String(cause), false, 0);
      setCreateError(apiError);
    }
  }, [update]);

  const setActive = useCallback((id: string) => {
    update(current => ({ ...current, active: id }));
  }, [update]);

  const save = useCallback((id: string, patch: Partial<TerminalTab>) => {
    update(current => ({
      ...current,
      all: current.all.map(tab => (tab.id === id ? { ...tab, ...patch } : tab)),
    }));
  }, [update]);

  const markExit = useCallback((id: string, exitCode: number | null) => {
    update(current => ({
      ...current,
      all: current.all.map(tab => (tab.id === id ? { ...tab, exitCode, missing: false } : tab)),
    }));
  }, [update]);

  const markMissing = useCallback((id: string) => {
    update(current => ({
      ...current,
      all: current.all.map(tab => (tab.id === id ? { ...tab, missing: true } : tab)),
    }));
  }, [update]);

  const dismissCreateError = useCallback(() => setCreateError(null), []);

  const activeTab = useMemo(
    () => state.all.find(tab => tab.id === state.active),
    [state],
  );

  return {
    tabs: state.all,
    activeId: state.active,
    activeTab,
    ready,
    error,
    creating,
    createError,
    create,
    retry,
    retryInHome,
    dismissCreateError,
    close,
    restart,
    setActive,
    save,
    markExit,
    markMissing,
  };
}
