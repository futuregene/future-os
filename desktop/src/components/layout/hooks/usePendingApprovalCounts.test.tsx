// @vitest-environment jsdom
import type { StoredApprovalRequest } from "../../../integrations/storage/threadStore";
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../../test/renderHook";
import { usePendingApprovalCounts } from "./usePendingApprovalCounts";

const mocks = vi.hoisted(() => ({
  handler: null as null | (() => void),
  list: vi.fn(),
  unlisten: vi.fn(),
}));

vi.mock("../../../integrations/storage/threadStore", () => ({
  listPendingApprovalRequests: (...args: unknown[]) => mocks.list(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, handler: () => void) => {
    if (event === "approvals-updated")
      mocks.handler = handler;
    return Promise.resolve(mocks.unlisten);
  },
}));

function approval(id: string, threadId: string): StoredApprovalRequest {
  return {
    id,
    threadId,
    kind: "shell",
    status: "pending",
    title: id,
    createdAt: 0,
    updatedAt: 0,
    reviewer: "user",
    decisionScope: "once",
    decisionSource: "desktop",
  };
}

describe("usePendingApprovalCounts", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    mocks.handler = null;
    mocks.list.mockReset().mockResolvedValue([]);
    mocks.unlisten.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("counts pending approvals per thread", async () => {
    mocks.list.mockResolvedValue([
      approval("a1", "t1"),
      approval("a2", "t1"),
      approval("a3", "t2"),
    ]);
    const hook = renderHook(() => usePendingApprovalCounts());
    await flushAsync();

    expect(hook.current.get("t1")).toBe(2);
    expect(hook.current.get("t2")).toBe(1);
    expect(hook.current.get("t3")).toBeUndefined();
    expect(hook.current.size).toBe(2);
    hook.unmount();
  });

  it("reloads when the backend pushes approvals-updated", async () => {
    const hook = renderHook(() => usePendingApprovalCounts());
    await flushAsync();
    // Initial load + the poll's immediate tick.
    expect(mocks.list).toHaveBeenCalledTimes(2);
    expect(hook.current.size).toBe(0);

    mocks.list.mockResolvedValue([approval("a1", "t9")]);
    await act(async () => {
      mocks.handler?.();
      await Promise.resolve();
    });

    expect(mocks.list).toHaveBeenCalledTimes(3);
    expect(hook.current.get("t9")).toBe(1);
    hook.unmount();
  });

  it("polls slowly as a lost-push backstop and stops on unmount", async () => {
    const hook = renderHook(() => usePendingApprovalCounts());
    await flushAsync();
    expect(mocks.list).toHaveBeenCalledTimes(2);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(15_000);
    });
    expect(mocks.list).toHaveBeenCalledTimes(3);

    hook.unmount();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });
    expect(mocks.list).toHaveBeenCalledTimes(3);
  });

  it("detaches the push listener on unmount", async () => {
    const hook = renderHook(() => usePendingApprovalCounts());
    await flushAsync();

    hook.unmount();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(mocks.unlisten).toHaveBeenCalledTimes(1);
  });

  it("keeps the same map when a poll returns the same rows in order", async () => {
    mocks.list.mockResolvedValue([approval("a1", "t1")]);
    const hook = renderHook(() => usePendingApprovalCounts());
    await flushAsync();
    const first = hook.current;

    mocks.list.mockResolvedValue([{ ...approval("a1", "t1") }]);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(15_000);
    });

    // Structurally equal rows are dropped by `isEqual`, so the memoised map is
    // reused and the sidebar does not re-render every 15s.
    expect(hook.current).toBe(first);
    hook.unmount();
  });

  it("picks up a changed row set even when the length is unchanged", async () => {
    mocks.list.mockResolvedValue([approval("a1", "t1")]);
    const hook = renderHook(() => usePendingApprovalCounts());
    await flushAsync();

    mocks.list.mockResolvedValue([approval("a2", "t2")]);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(15_000);
    });

    expect(hook.current.get("t1")).toBeUndefined();
    expect(hook.current.get("t2")).toBe(1);
    hook.unmount();
  });

  it("keeps the previous counts when a poll fails", async () => {
    mocks.list.mockResolvedValue([approval("a1", "t1")]);
    const hook = renderHook(() => usePendingApprovalCounts());
    await flushAsync();
    expect(hook.current.size).toBe(1);

    mocks.list.mockRejectedValue(new Error("agent down"));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(15_000);
    });

    // Silent-retry semantics: a failed backstop poll must not blank the badges.
    expect(hook.current.get("t1")).toBe(1);
    hook.unmount();
  });
});
