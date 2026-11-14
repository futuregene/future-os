import { act } from "react";
// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { useOverlayLayer } from "../../components/ui/overlayStack";
import { renderHook } from "../../test/renderHook";
import { MIN_PANEL_HEIGHT, MIN_PANEL_WIDTH, useTerminalPanel } from "./useTerminalPanel";

const PREFS_KEY = "future.terminal.panel.v1";

// The panel derives its height from `window.innerHeight` (a share of it, clamped
// to a ceiling of `innerHeight - HEADER_RESERVE`). The resize test below shrinks
// the window to 300px, and `window` is process-global, so without a baseline the
// next test to run inherits a 60px ceiling — which clamps its stored 300px height
// up to MIN_PANEL_HEIGHT. Captured once, before any test, and restored around
// every test.
const BASE_VIEWPORT = window.innerHeight;
const BASE_VIEWPORT_WIDTH = window.innerWidth;

beforeEach(() => {
  localStorage.clear();
  window.innerHeight = BASE_VIEWPORT;
  window.innerWidth = BASE_VIEWPORT_WIDTH;
});

afterEach(() => {
  localStorage.clear();
  window.innerHeight = BASE_VIEWPORT;
  window.innerWidth = BASE_VIEWPORT_WIDTH;
});

describe("useTerminalPanel", () => {
  it("starts closed and remembers open per conversation", () => {
    const harness = renderHook(() => useTerminalPanel("thread-1"));
    expect(harness.current.open).toBe(false);
    act(() => harness.current.setOpen(true));
    expect(harness.current.open).toBe(true);

    // A different conversation has its own panel state.
    const other = renderHook(() => useTerminalPanel("thread-2"));
    expect(other.current.open).toBe(false);
    other.unmount();
    harness.unmount();

    // …and the first conversation's choice is persisted.
    const restored = renderHook(() => useTerminalPanel("thread-1"));
    expect(restored.current.open).toBe(true);
    restored.unmount();
  });

  it("keeps the panel closed when there is no conversation", () => {
    localStorage.setItem(PREFS_KEY, JSON.stringify({ open: { "thread-1": true }, height: 400 }));
    const harness = renderHook(() => useTerminalPanel(null));
    expect(harness.current.enabled).toBe(false);
    expect(harness.current.open).toBe(false);
    act(() => harness.current.setOpen(true));
    expect(harness.current.open).toBe(false);
    harness.unmount();
  });

  it("clamps the height to the usable range", () => {
    localStorage.setItem(PREFS_KEY, JSON.stringify({ height: 5 }));
    const harness = renderHook(() => useTerminalPanel("thread-1"));
    expect(harness.current.height).toBe(MIN_PANEL_HEIGHT);

    act(() => harness.current.setHeight(10 ** 6));
    expect(harness.current.height).toBe(harness.current.maxHeight);

    act(() => harness.current.setHeight(320));
    expect(harness.current.height).toBe(320);
    expect(JSON.parse(localStorage.getItem(PREFS_KEY) ?? "{}").height).toBe(320);
    harness.unmount();
  });

  it("toggles with Ctrl+J and stops behind a dialog", () => {
    localStorage.setItem(PREFS_KEY, JSON.stringify({ open: { "thread-1": true } }));
    const harness = renderHook(() => useTerminalPanel("thread-1"));
    expect(harness.current.open).toBe(true);

    // Ctrl+J collapses it.
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "j", ctrlKey: true }));
    });
    expect(harness.current.open).toBe(false);

    // Shift/Cmd variants are not the shortcut.
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "j", ctrlKey: true, shiftKey: true }));
    });
    expect(harness.current.open).toBe(false);

    // Plain "j" types into whatever has focus.
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "j" }));
    });
    expect(harness.current.open).toBe(false);
    harness.unmount();
  });

  it("docks to the bottom until the reader says otherwise, and remembers the choice", () => {
    const harness = renderHook(() => useTerminalPanel("thread-1"));
    expect(harness.current.dock).toBe("bottom");
    act(() => harness.current.setDock("right"));
    expect(harness.current.dock).toBe("right");

    // Where the panel sits is a window preference, unlike whether it is open:
    // another conversation comes back on the edge the reader chose.
    const other = renderHook(() => useTerminalPanel("thread-2"));
    expect(other.current.dock).toBe("right");
    act(() => other.current.toggleDock());
    expect(other.current.dock).toBe("bottom");
    expect(JSON.parse(localStorage.getItem(PREFS_KEY) ?? "{}").dock).toBe("bottom");
    other.unmount();
    harness.unmount();
  });

  it("treats an unknown dock as the bottom, so a corrupt value cannot lose the panel", () => {
    localStorage.setItem(PREFS_KEY, JSON.stringify({ dock: "sideways" }));
    const harness = renderHook(() => useTerminalPanel("thread-1"));
    expect(harness.current.dock).toBe("bottom");
    harness.unmount();
  });

  it("keeps a right docked panel inside the room the shell leaves", () => {
    window.innerWidth = 1600;
    localStorage.setItem(PREFS_KEY, JSON.stringify({ width: 100 }));
    const harness = renderHook(() => useTerminalPanel("thread-1", { reservedWidth: 600 }));
    // Below the floor, and above the ceiling the shell's other columns leave.
    expect(harness.current.width).toBe(MIN_PANEL_WIDTH);
    expect(harness.current.maxWidth).toBe(1600 - 600);
    act(() => harness.current.setWidth(10 ** 6));
    expect(harness.current.width).toBe(harness.current.maxWidth);

    act(() => harness.current.setWidth(500));
    expect(harness.current.width).toBe(500);
    expect(JSON.parse(localStorage.getItem(PREFS_KEY) ?? "{}").width).toBe(500);

    // A shrunk window takes the room back instead of overflowing the layout.
    act(() => {
      window.innerWidth = 800;
      window.dispatchEvent(new Event("resize"));
    });
    expect(harness.current.width).toBe(Math.max(MIN_PANEL_WIDTH, 800 - 600));
    harness.unmount();
  });

  it("falls back to the bottom — and hides the choice — when the window cannot host a column", () => {
    // 1024px window with 800px spoken for: 224px are left, below the column's
    // floor, so docking right would squeeze the conversation instead.
    window.innerWidth = 1024;
    localStorage.setItem(PREFS_KEY, JSON.stringify({ dock: "right" }));
    const harness = renderHook(() => useTerminalPanel("thread-1", { reservedWidth: 800 }));
    expect(harness.current.canDockRight).toBe(false);
    expect(harness.current.dock).toBe("bottom");

    // The preference is kept, not overwritten: a wider window brings the column
    // back where the reader put it.
    act(() => {
      window.innerWidth = 1600;
      window.dispatchEvent(new Event("resize"));
    });
    expect(harness.current.canDockRight).toBe(true);
    expect(harness.current.dock).toBe("right");
    expect(JSON.parse(localStorage.getItem(PREFS_KEY) ?? "{}").dock).toBe("right");
    harness.unmount();
  });

  it("maximizes until it is collapsed, and never comes back maximized", () => {
    localStorage.setItem(PREFS_KEY, JSON.stringify({ open: { "thread-1": true } }));
    const harness = renderHook(() => useTerminalPanel("thread-1"));
    expect(harness.current.maximized).toBe(false);
    act(() => harness.current.toggleMaximized());
    expect(harness.current.maximized).toBe(true);

    // Closing the panel hides the conversation, so it is also the way back to
    // it: reopening gives the docked layout, not the whole window again.
    act(() => harness.current.setOpen(false));
    expect(harness.current.maximized).toBe(false);
    act(() => harness.current.setOpen(true));
    expect(harness.current.maximized).toBe(false);

    // A way of looking at the terminal, not a preference to store.
    expect(JSON.parse(localStorage.getItem(PREFS_KEY) ?? "{}").maximized).toBeUndefined();
    harness.unmount();
  });

  it("cannot maximize without a conversation", () => {
    const harness = renderHook(() => useTerminalPanel(null));
    act(() => harness.current.toggleMaximized());
    expect(harness.current.maximized).toBe(false);
    harness.unmount();
  });

  it("labels the shortcut per platform", () => {
    const harness = renderHook(() => useTerminalPanel("thread-1"));
    // jsdom reports a non-macOS user agent, so the app's Ctrl convention.
    expect(harness.current.shortcut).toBe("Ctrl+J");
    harness.unmount();
  });

  it("survives corrupt preferences", () => {
    localStorage.setItem(PREFS_KEY, "{oops");
    const harness = renderHook(() => useTerminalPanel("thread-1"));
    expect(harness.current.open).toBe(false);
    expect(harness.current.height).toBeGreaterThanOrEqual(MIN_PANEL_HEIGHT);
    harness.unmount();
  });

  it("hands the keyboard back to the composer when it collapses", () => {
    // Collapsing removes the panel from the layout, so focus left on a hidden
    // terminal would swallow the next keystrokes. Both routes into a collapse
    // (the shortcut and the panel's own ✕) go through the controller.
    localStorage.setItem(PREFS_KEY, JSON.stringify({ open: { "thread-1": true } }));
    const focus: string[] = [];
    const onFocus = () => focus.push("composer");
    window.addEventListener("futureos:focus-composer", onFocus);
    const harness = renderHook(() => useTerminalPanel("thread-1"));
    try {
      expect(harness.current.open).toBe(true);

      // Ctrl+J.
      act(() => {
        window.dispatchEvent(new KeyboardEvent("keydown", { key: "j", ctrlKey: true }));
      });
      expect(harness.current.open).toBe(false);
      expect(focus).toHaveLength(1);

      // The ✕ in the panel header.
      act(() => harness.current.setOpen(true));
      expect(focus).toHaveLength(1);
      act(() => harness.current.setOpen(false));
      expect(focus).toHaveLength(2);

      // Expanding gives the terminal the keyboard, not the composer.
      act(() => harness.current.toggle());
      expect(harness.current.open).toBe(true);
      expect(focus).toHaveLength(2);
    }
    finally {
      window.removeEventListener("futureos:focus-composer", onFocus);
      harness.unmount();
    }
  });

  it("ignores the shortcut while a dialog owns the keyboard", () => {
    localStorage.setItem(PREFS_KEY, JSON.stringify({ open: { "thread-1": true } }));
    const behindDialog = renderHook(() => {
      useOverlayLayer(true);
      return useTerminalPanel("thread-1");
    });
    expect(behindDialog.current.open).toBe(true);
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "j", ctrlKey: true }));
    });
    // The modal keeps the panel exactly as it was.
    expect(behindDialog.current.open).toBe(true);
    behindDialog.unmount();

    // Without the dialog the same keystroke collapses it.
    const clear = renderHook(() => useTerminalPanel("thread-1"));
    expect(clear.current.open).toBe(true);
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "j", ctrlKey: true }));
    });
    expect(clear.current.open).toBe(false);
    clear.unmount();
  });

  it("ignores the shortcut with no conversation, without writing a preference", () => {
    const harness = renderHook(() => useTerminalPanel(null));
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "j", ctrlKey: true }));
    });
    expect(harness.current.enabled).toBe(false);
    expect(harness.current.open).toBe(false);
    expect(localStorage.getItem(PREFS_KEY)).toBeNull();
    harness.unmount();
  });

  it("recovers from a stored preference document that is valid JSON but not an object", () => {
    localStorage.setItem(PREFS_KEY, "5");
    const scalar = renderHook(() => useTerminalPanel("thread-1"));
    expect(scalar.current.open).toBe(false);
    expect(scalar.current.height).toBeGreaterThanOrEqual(MIN_PANEL_HEIGHT);
    scalar.unmount();

    // An `open` map of non-boolean values is ignored entry by entry.
    localStorage.setItem(PREFS_KEY, JSON.stringify({ height: 300, open: { "thread-1": "yes", "thread-2": false } }));
    const mixed = renderHook(() => useTerminalPanel("thread-1"));
    expect(mixed.current.open).toBe(false);
    expect(mixed.current.height).toBe(300);
    mixed.unmount();
  });

  it("clamps the height against the viewport and follows a window resize", () => {
    localStorage.setItem(PREFS_KEY, JSON.stringify({ height: 700 }));
    const harness = renderHook(() => useTerminalPanel("thread-1"));
    const initialMax = harness.current.maxHeight;
    expect(harness.current.height).toBe(Math.min(700, initialMax));

    // A shorter window lowers the ceiling and the panel follows it down.
    act(() => {
      window.innerHeight = 300;
      window.dispatchEvent(new Event("resize"));
    });
    expect(harness.current.maxHeight).toBeLessThan(initialMax);
    expect(harness.current.height).toBe(harness.current.maxHeight);
    harness.unmount();
  });
});
