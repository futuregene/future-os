import { act } from "react";
// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { useOverlayLayer } from "../../components/ui/overlayStack";
import { renderHook } from "../../test/renderHook";
import { MIN_PANEL_HEIGHT, useTerminalPanel } from "./useTerminalPanel";

const PREFS_KEY = "future.terminal.panel.v1";

beforeEach(() => {
  localStorage.clear();
});

afterEach(() => {
  localStorage.clear();
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
