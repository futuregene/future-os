import { act } from "react";
// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from "vitest";
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
});
