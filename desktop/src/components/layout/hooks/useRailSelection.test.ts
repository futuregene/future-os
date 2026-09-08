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
