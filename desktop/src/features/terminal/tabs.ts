/**
 * Tab state for one conversation's terminal panel.
 *
 * The server owns the shell; this store owns only the *view*: which tabs exist,
 * which is active, and the serialized screen each tab last showed. Persisting
 * the screen is what makes closing the panel (or reloading the webview) cheap:
 * the restored view resumes from the cursor it last applied instead of asking
 * the server to re-render, and it works even after the server's 2 MiB output
 * tail has moved past that client.
 */

/** A terminal tab as the UI knows it. */
export interface TerminalTab {
  id: string;
  title: string;
  /** Stable index used for the label ("Terminal 1"); survives restarts. */
  titleNumber: number;
  /** Serialized xterm screen from the last time this tab was mounted. */
  buffer?: string;
  /** Absolute output cursor already applied to `buffer`. */
  cursor?: number;
  cols?: number;
  rows?: number;
  scrollY?: number;
  /**
   * Set when the shell exited: the tab keeps its final screen and the UI
   * offers a restart.
   */
  exitCode?: number | null;
  /**
   * Set when the server no longer has this session (app restarted, or the
   * session was evicted): the view offers to start a new shell instead.
   */
  missing?: boolean;
}

export interface TerminalTabsState {
  active?: string;
  all: TerminalTab[];
}

const STORAGE_PREFIX = "future.terminal.tabs.v1";

/** Visible for tests and for the logout/cleanup paths. */
export function tabsStorageKey(threadId: string): string {
  return `${STORAGE_PREFIX}.${threadId}`;
}

export const EMPTY_TABS: TerminalTabsState = { all: [] };

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function text(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function num(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function toTab(value: unknown): TerminalTab | undefined {
  if (!record(value))
    return undefined;
  const id = text(value.id);
  if (!id)
    return undefined;
  const title = text(value.title) ?? "";
  const titleNumber = num(value.titleNumber);
  const buffer = text(value.buffer);
  const cursor = num(value.cursor);
  const cols = num(value.cols);
  const rows = num(value.rows);
  const scrollY = num(value.scrollY);
  const exitCode = value.exitCode === null ? null : num(value.exitCode);
  const missing = value.missing === true;
  return {
    id,
    title,
    titleNumber: titleNumber && titleNumber > 0 ? titleNumber : 0,
    ...(buffer === undefined ? {} : { buffer }),
    ...(cursor === undefined ? {} : { cursor }),
    ...(cols === undefined ? {} : { cols }),
    ...(rows === undefined ? {} : { rows }),
    ...(scrollY === undefined ? {} : { scrollY }),
    ...(exitCode === undefined ? {} : { exitCode }),
    ...(missing ? { missing } : {}),
  };
}

/**
 * Validate anything that came out of storage. Persisted state is untrusted
 * input: a half-written or hand-edited entry must not break the panel.
 */
export function migrateTabsState(value: unknown): TerminalTabsState {
  if (!record(value))
    return { ...EMPTY_TABS };
  const seen = new Set<string>();
  const all = (Array.isArray(value.all) ? value.all : []).flatMap((item) => {
    const tab = toTab(item);
    if (!tab || seen.has(tab.id))
      return [];
    seen.add(tab.id);
    return [tab];
  });
  const active = text(value.active);
  return {
    active: active && seen.has(active) ? active : all[0]?.id,
    all,
  };
}

export function loadTabsState(threadId: string): TerminalTabsState {
  try {
    const raw = localStorage.getItem(tabsStorageKey(threadId));
    if (!raw)
      return { ...EMPTY_TABS };
    return migrateTabsState(JSON.parse(raw));
  }
  catch {
    return { ...EMPTY_TABS };
  }
}

export function saveTabsState(threadId: string, state: TerminalTabsState): void {
  try {
    // An empty state is removed rather than stored: it keeps a thread that was
    // opened once with no tabs from leaving an entry behind forever.
    if (state.all.length === 0)
      localStorage.removeItem(tabsStorageKey(threadId));
    else
      localStorage.setItem(tabsStorageKey(threadId), JSON.stringify(state));
  }
  catch {
    // Quota or a disabled store must not break the terminal itself; the tab
    // simply will not survive a reload.
  }
}

export function clearTabsState(threadId: string): void {
  try {
    localStorage.removeItem(tabsStorageKey(threadId));
  }
  catch {
    // See saveTabsState.
  }
}

/**
 * Smallest unused positive label index, so "Terminal 1" reappears after the
 * first tab is closed.
 */
export function nextTitleNumber(all: readonly TerminalTab[]): number {
  const used = new Set(all.map(tab => tab.titleNumber).filter(number => number > 0));
  for (let index = 1; index <= used.size + 1; index += 1) {
    if (!used.has(index))
      return index;
  }
  return used.size + 1;
}

export function selectTabAfterClose(all: readonly TerminalTab[], closingId: string): string | undefined {
  const index = all.findIndex(tab => tab.id === closingId);
  if (index === -1)
    return all[0]?.id;
  return all[index - 1]?.id ?? all[index + 1]?.id;
}

export function patchTab(state: TerminalTabsState, id: string, patch: Partial<TerminalTab>): TerminalTabsState {
  return {
    active: state.active,
    all: state.all.map(tab => (tab.id === id ? { ...tab, ...patch } : tab)),
  };
}
