// @vitest-environment jsdom
import type { CompactContextResult } from "../../../integrations/agent/agentClient";
import type { StoredThread } from "../../../integrations/storage/threadStore";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { compactThreadContext } from "../../../integrations/agent/agentClient";
import { onFutureEvent } from "../../../lib/futureEvents";
import { renderHook } from "../../../test/renderHook";
import { useThreadCompaction } from "./useThreadCompaction";

vi.mock("../../../integrations/agent/agentClient", () => ({ compactThreadContext: vi.fn() }));
const compact = vi.mocked(compactThreadContext);
const thread = { id: "t1", agentSessionId: "s1" } as StoredThread;

beforeEach(() => {
  vi.useFakeTimers();
  compact.mockReset();
});
afterEach(() => vi.useRealTimers());

function terminal(operationId: string, eventType: string, error?: string) {
  window.dispatchEvent(new CustomEvent("future:agent-event", {
    detail: { threadId: "t1", sessionId: "s1", eventType, payload: { operation_id: operationId, error } },
  }));
}

it("matches a terminal failure that arrives before the request acknowledges, ignoring unrelated operations", async () => {
  let acknowledge!: (result: CompactContextResult) => void;
  compact.mockReturnValue(new Promise((resolve) => {
    acknowledge = resolve;
  }));
  const toast = vi.fn();
  const off = onFutureEvent("toast", toast);
  const hook = renderHook(() => useThreadCompaction(thread));
  try {
    const pending = hook.current();
    terminal("op1", "compaction_failed", "model failure");
    terminal("other", "compaction_committed");
    expect(toast).not.toHaveBeenCalled();
    acknowledge({ operationId: "op1", accepted: true });
    await pending;
    expect(toast).toHaveBeenCalledTimes(1);
    expect(toast.mock.calls[0]?.[0]).toMatchObject({ tone: "error" });
    expect(toast.mock.calls[0]?.[0].message).toContain("model failure");
    expect(vi.getTimerCount()).toBe(0);
  }
  finally {
    off();
    hook.unmount();
  }
});

it("settles cancellation while the request is still pending and handles a late rejection", async () => {
  let fail!: (error: Error) => void;
  compact.mockReturnValue(new Promise((_resolve, reject) => {
    fail = reject;
  }));
  const toast = vi.fn();
  const off = onFutureEvent("toast", toast);
  const hook = renderHook(() => useThreadCompaction(thread));
  try {
    const pending = hook.current();
    hook.unmount();
    await pending;
    fail(new Error("late request failure"));
    await Promise.resolve();
    expect(toast).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
  }
  finally {
    off();
  }
});

it("bounds the initial request with the compaction timeout", async () => {
  compact.mockReturnValue(new Promise(() => {}));
  const toast = vi.fn();
  const off = onFutureEvent("toast", toast);
  const hook = renderHook(() => useThreadCompaction(thread));
  try {
    const pending = hook.current();
    await vi.advanceTimersByTimeAsync(30 * 60 * 1000);
    await pending;
    expect(toast).toHaveBeenCalledTimes(1);
    expect(toast.mock.calls[0]?.[0]).toMatchObject({ tone: "error" });
    expect(vi.getTimerCount()).toBe(0);
  }
  finally {
    off();
    hook.unmount();
  }
});
