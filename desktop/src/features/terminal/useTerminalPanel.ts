/**
 * Panel-level state for the embedded terminal: whether it is open for the
 * active conversation, how big it is, which edge it docks to, whether it has
 * the whole content area, and the global toggle shortcut.
 *
 * Size, dock and the maximized state are window-level preferences (one panel per
 * window); "open" is remembered per conversation so switching threads restores
 * each thread's own panel, which is what the design asks for and what the tab
 * store assumes.
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import { hasOpenOverlay } from "../../components/ui/overlayStack";
import { emitFutureEvent } from "../../lib/futureEvents";
import { useBindingLabel } from "../keyboard/shortcuts";
import { isPanelToggleShortcut } from "./shortcut";

const PREFS_KEY = "future.terminal.panel.v1";
/** Default share of the viewport. */
const DEFAULT_RATIO = 0.35;
/** Usable minimum: below this the tab strip and one prompt line do not fit. */
export const MIN_PANEL_HEIGHT = 160;
/** Same floor on the other axis: a narrower column wraps every line twice. */
export const MIN_PANEL_WIDTH = 280;
/** Default width of a right-docked panel, beside a 384px conversation floor. */
const DEFAULT_WIDTH = 420;
/** Leave room for the header and a couple of messages above the panel. */
const HEADER_RESERVE = 240;

/**
 * Which edge the panel is attached to.
 *
 * Both are columns of the shell, not two different panels: the same tabs, the
 * same shells, the same preferences. A bottom dock takes the width of the
 * conversation and the files panel; a right dock is a column at the window's
 * edge beside them (see `AppShell`), which is why the two keep separate sizes.
 */
export type TerminalDock = "bottom" | "right";

interface PanelPrefs {
  height?: number;
  width?: number;
  dock?: TerminalDock;
  open?: Record<string, boolean>;
}

function loadPrefs(): PanelPrefs {
  try {
    const raw = localStorage.getItem(PREFS_KEY);
    if (!raw)
      return {};
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null)
      return {};
    const record = parsed as Record<string, unknown>;
    const height = typeof record.height === "number" && Number.isFinite(record.height) ? record.height : undefined;
    const width = typeof record.width === "number" && Number.isFinite(record.width) ? record.width : undefined;
    // An unknown dock is "bottom", the layout the panel has always had; a
    // corrupt value must not make the panel un-placeable.
    const dock: TerminalDock | undefined
      = record.dock === "right" ? "right" : record.dock === "bottom" ? "bottom" : undefined;
    const open: Record<string, boolean> = {};
    if (typeof record.open === "object" && record.open !== null) {
      for (const [threadId, value] of Object.entries(record.open as Record<string, unknown>)) {
        if (typeof value === "boolean")
          open[threadId] = value;
      }
    }
    return { height, width, dock, open };
  }
  catch {
    return {};
  }
}

function savePrefs(prefs: PanelPrefs): void {
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify(prefs));
  }
  catch {
    // A disabled/full storage must not break the panel; the preference is lost.
  }
}

export interface TerminalPanelController {
  open: boolean;
  /** The edge the panel is attached to *now* (see `canDockRight`). */
  dock: TerminalDock;
  /** Room for a right-hand column beside the conversation. */
  canDockRight: boolean;
  /** The panel has the whole content area instead of its docked size. */
  maximized: boolean;
  height: number;
  maxHeight: number;
  width: number;
  maxWidth: number;
  /** The panel is available at all (a conversation is open). */
  enabled: boolean;
  toggle: () => void;
  setOpen: (open: boolean) => void;
  setHeight: (height: number) => void;
  setWidth: (width: number) => void;
  setDock: (dock: TerminalDock) => void;
  /** Bottom ⇄ right, for the panel header's dock button. */
  toggleDock: () => void;
  toggleMaximized: () => void;
  /** Shortcut label for tooltips, platform-correct and live. */
  shortcut: string;
}

export interface TerminalPanelOptions {
  /**
   * Pixels of the window the shell keeps beside a right-docked panel: the left
   * rail when it is a column, the conversation's floor and the context column.
   * The panel never takes the window below them, so the conversation keeps its
   * width floor however wide the reader drags the terminal.
   *
   * Read at render time from the shell's live layout, which is why it is an
   * argument rather than something the panel measures itself.
   */
  reservedWidth?: number;
}

export function useTerminalPanel(
  threadId: string | null,
  options: TerminalPanelOptions = {},
): TerminalPanelController {
  const reservedWidth = options.reservedWidth ?? 0;
  const [prefs, setPrefs] = useState<PanelPrefs>(() => loadPrefs());
  const [viewport, setViewport] = useState(() => (typeof window === "undefined"
    ? { height: 900, width: 1440 }
    : { height: window.innerHeight, width: window.innerWidth }));
  // Session state, not a preference: a maximized panel is a way of looking at
  // the terminal, and reopening it later should give the layout it was docked in.
  const [isMaximized, setIsMaximized] = useState(false);

  useEffect(() => {
    const update = () => setViewport({ height: window.innerHeight, width: window.innerWidth });
    window.addEventListener("resize", update);
    return () => window.removeEventListener("resize", update);
  }, []);

  const maxHeight = Math.max(MIN_PANEL_HEIGHT, viewport.height - HEADER_RESERVE);
  const height = useMemo(() => {
    const stored = prefs.height ?? Math.round(viewport.height * DEFAULT_RATIO);
    return Math.min(Math.max(stored, MIN_PANEL_HEIGHT), maxHeight);
  }, [prefs.height, viewport.height, maxHeight]);

  // Room left for the column itself. A right dock is only offered when that room
  // reaches the floor: below it the conversation would go under its own floor,
  // and a terminal squeezed to a sliver helps nobody (the shell folds panels for
  // exactly this reason).
  const room = viewport.width - reservedWidth;
  const canDockRight = room >= MIN_PANEL_WIDTH;
  const maxWidth = Math.max(MIN_PANEL_WIDTH, room);
  const width = useMemo(() => {
    const stored = prefs.width ?? DEFAULT_WIDTH;
    return Math.min(Math.max(stored, MIN_PANEL_WIDTH), maxWidth);
  }, [prefs.width, maxWidth]);

  const enabled = Boolean(threadId);
  const open = enabled && threadId ? prefs.open?.[threadId] === true : false;
  // The *effective* dock, not the stored one: a preference for the right edge is
  // kept, and comes back by itself when the window can host the column again.
  const dock: TerminalDock = prefs.dock === "right" && canDockRight ? "right" : "bottom";
  const maximized = enabled && isMaximized;

  const update = useCallback((mutate: (previous: PanelPrefs) => PanelPrefs) => {
    setPrefs((previous) => {
      const next = mutate(previous);
      savePrefs(next);
      return next;
    });
  }, []);

  const setOpen = useCallback((next: boolean) => {
    if (!threadId)
      return;
    update(previous => ({ ...previous, open: { ...previous.open, [threadId]: next } }));
    // Collapsing (shortcut or the panel's own ✕) hands the keyboard back to the
    // conversation: the panel leaves the layout, so focus left on a hidden
    // terminal would swallow the next keystrokes. The composer answers this
    // event and declines while it cannot hold a caret.
    if (!next) {
      // A maximized panel hides the conversation, so closing it is also the way
      // back to that conversation: reopening gives the docked layout, not a
      // window-filling terminal nobody asked for again.
      setIsMaximized(false);
      emitFutureEvent("focus-composer", undefined);
    }
  }, [threadId, update]);

  const toggle = useCallback(() => setOpen(!open), [open, setOpen]);

  const setHeight = useCallback((next: number) => {
    update(previous => ({ ...previous, height: next }));
  }, [update]);

  const setWidth = useCallback((next: number) => {
    update(previous => ({ ...previous, width: next }));
  }, [update]);

  const setDock = useCallback((next: TerminalDock) => {
    update(previous => ({ ...previous, dock: next }));
  }, [update]);

  const toggleDock = useCallback(() => setDock(dock === "right" ? "bottom" : "right"), [dock, setDock]);

  const toggleMaximized = useCallback(() => setIsMaximized(previous => !previous), []);

  // Subscribed rather than read once: a rebinding in Settings must reach the
  // tooltip without a reload (the key itself is read at press time).
  const shortcut = useBindingLabel("terminal.toggle");

  // Global shortcut, so it also works while the composer (or anything else in
  // the window) has focus. The binding is the command table's — `Ctrl+J` on
  // Windows/Linux, `⌘J` on macOS by default, rebindable in Settings — and the
  // terminal never receives it (the design records that trade-off explicitly;
  // Enter submits a command line). Inside the terminal, `TerminalView` releases
  // the key from xterm so it reaches this listener; see `./shortcut`.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!isPanelToggleShortcut(event))
        return;
      // A modal owns the keyboard while it is open.
      if (hasOpenOverlay())
        return;
      if (!enabled)
        return;
      event.preventDefault();
      toggle();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [enabled, toggle]);

  return {
    open,
    dock,
    canDockRight,
    maximized,
    height,
    maxHeight,
    width,
    maxWidth,
    enabled,
    toggle,
    setOpen,
    setHeight,
    setWidth,
    setDock,
    toggleDock,
    toggleMaximized,
    shortcut,
  };
}
