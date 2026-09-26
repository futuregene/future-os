// @vitest-environment jsdom
import type { StoredThread } from "../../../integrations/storage/threadStore";
import { act } from "react";
import { describe, expect, it, vi } from "vitest";
import { renderHook } from "../../../test/renderHook";
import { useRailSelection } from "./useRailSelection";

function thread(id: string): StoredThread {
  return { id, agentSessionId: id, title: id, mode: "chat", workspaceId: id, status: "active", pinned: false, readonly: false, createdAt: 0, updatedAt: 0 };
}

describe("rail selection", () => {
  it("keeps selection scoped and removes IDs that disappear externally", () => {
    const first = thread("first");
    const second = thread("second");
    let visibleThreads = [first, second];
    let threadScopes = new Map([[first.id, "chat"], [second.id, "chat"]]);
    const onBatchDeleteThreads = vi.fn();
    const h = renderHook(() => useRailSelection({
      onBatchDeleteThreads,
      onSelectThread: vi.fn(),
      threadScopes,
      visibleThreads,
    }));

    act(() => h.current.enterSelectionMode("chat"));
    act(() => h.current.toggleThreadSelection(first));
    expect([...h.current.selectedThreadIds]).toEqual([first.id]);

    visibleThreads = [second];
    threadScopes = new Map([[second.id, "chat"]]);
    h.rerender();
    expect(h.current.selectedThreadIds.size).toBe(0);

    act(() => h.current.selectAll());
    act(() => h.current.deleteSelected());
    expect(onBatchDeleteThreads).toHaveBeenCalledWith([second]);
    expect(h.current.selectionMode).toBe(false);
    h.unmount();
  });

  it("never sweeps a pinned thread into a batch selection", () => {
    const plain = thread("plain");
    const pinned = { ...thread("pinned"), pinned: true };
    const onBatchDeleteThreads = vi.fn();
    const h = renderHook(() => useRailSelection({
      onBatchDeleteThreads,
      onSelectThread: vi.fn(),
      threadScopes: new Map([[plain.id, "chat"], [pinned.id, "chat"]]),
      visibleThreads: [plain, pinned],
    }));

    act(() => h.current.enterSelectionMode("chat"));
    // The pinned thread sits in the pinned section, not in this group: no
    // checkbox, no row toggle, and select-all skips it.
    expect(h.current.isThreadInScope(pinned)).toBe(false);
    act(() => h.current.handleRowSelect(pinned));
    act(() => h.current.selectAll());
    expect([...h.current.selectedThreadIds]).toEqual([plain.id]);
    act(() => h.current.deleteSelected());
    expect(onBatchDeleteThreads).toHaveBeenCalledWith([plain]);
    h.unmount();
  });

  it("only Escape leaves selection mode, and nothing is bound once it is off", () => {
    const item = thread("item");
    const h = renderHook(() => useRailSelection({
      onBatchDeleteThreads: vi.fn(),
      onSelectThread: vi.fn(),
      threadScopes: new Map([[item.id, "chat"]]),
      visibleThreads: [item],
    }));

    // While selection mode is off there is no listener at all, so Escape is just
    // a normal key (e.g. it must not close anything on the user's behalf).
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(h.current.selectionMode).toBe(false);

    act(() => h.current.enterSelectionMode("chat"));
    expect(h.current.selectionMode).toBe(true);

    // A non-Escape key must not end the batch — only Escape does.
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "a" }));
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown" }));
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Backspace" }));
    });
    expect(h.current.selectionMode).toBe(true);

    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(h.current.selectionMode).toBe(false);

    // And the listener is gone again: a later Escape is inert.
    act(() => h.current.enterSelectionMode("chat"));
    act(() => h.current.exitSelectionMode());
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(h.current.selectionMode).toBe(false);
    h.unmount();
  });

  it("routes row activation to selection without opening the thread", () => {
    const item = thread("item");
    const onSelectThread = vi.fn();
    const h = renderHook(() => useRailSelection({
      onBatchDeleteThreads: vi.fn(),
      onSelectThread,
      threadScopes: new Map([[item.id, "chat"]]),
      visibleThreads: [item],
    }));

    act(() => h.current.handleRowSelect(item));
    expect(onSelectThread).toHaveBeenCalledWith(item);
    act(() => h.current.enterSelectionMode("chat"));
    act(() => h.current.handleRowSelect(item));
    expect(onSelectThread).toHaveBeenCalledTimes(1);
    expect(h.current.selectedThreadIds.has(item.id)).toBe(true);
    h.unmount();
  });
});
