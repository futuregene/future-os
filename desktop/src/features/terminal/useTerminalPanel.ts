/**
 * Panel-level state for the embedded terminal: whether it is open for the
 * active conversation, how tall it is, and the global toggle shortcut.
 *
 * Height is a window-level preference (one panel per window); "open" is
 * remembered per conversation so switching threads restores each thread's own
 * panel, which is what the design asks for and what the tab store assumes.
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import { hasOpenOverlay } from "../../components/ui/overlayStack";
import { isPanelToggleShortcut, panelToggleShortcutLabel } from "./shortcut";

const PREFS_KEY = "future.terminal.panel.v1";
/** Default share of the viewport. */
const DEFAULT_RATIO = 0.35;
/** Usable minimum: below this the tab strip and one prompt line do not fit. */
export const MIN_PANEL_HEIGHT = 160;
/** Leave room for the header and a couple of messages above the panel. */
const HEADER_RESERVE = 240;

interface PanelPrefs {
  height?: number;
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
    const open: Record<string, boolean> = {};
    if (typeof record.open === "object" && record.open !== null) {
      for (const [threadId, value] of Object.entries(record.open as Record<string, unknown>)) {
        if (typeof value === "boolean")
          open[threadId] = value;
      }
    }
    return { height, open };
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
  height: number;
  maxHeight: number;
  /** The panel is available at all (a conversation is open). */
  enabled: boolean;
  toggle: () => void;
  setOpen: (open: boolean) => void;
  setHeight: (height: number) => void;
  /** Shortcut label for tooltips, platform-correct. */
  shortcut: string;
}

export function useTerminalPanel(threadId: string | null): TerminalPanelController {
  const [prefs, setPrefs] = useState<PanelPrefs>(() => loadPrefs());
  const [viewport, setViewport] = useState(() => (typeof window === "undefined" ? 900 : window.innerHeight));

  useEffect(() => {
    const update = () => setViewport(window.innerHeight);
    window.addEventListener("resize", update);
    return () => window.removeEventListener("resize", update);
  }, []);

  const maxHeight = Math.max(MIN_PANEL_HEIGHT, viewport - HEADER_RESERVE);
  const height = useMemo(() => {
    const stored = prefs.height ?? Math.round(viewport * DEFAULT_RATIO);
    return Math.min(Math.max(stored, MIN_PANEL_HEIGHT), maxHeight);
  }, [prefs.height, viewport, maxHeight]);

  const enabled = Boolean(threadId);
  const open = enabled && threadId ? prefs.open?.[threadId] === true : false;

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
  }, [threadId, update]);

  const toggle = useCallback(() => setOpen(!open), [open, setOpen]);

  const setHeight = useCallback((next: number) => {
    update(previous => ({ ...previous, height: next }));
  }, [update]);

  // Global shortcut, so it also works while the composer (or anything else in
  // the window) has focus. Ctrl+J on Windows/Linux, Cmd+J on macOS — the
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
    height,
    maxHeight,
    enabled,
    toggle,
    setOpen,
    setHeight,
    shortcut: panelToggleShortcutLabel(),
  };
}
