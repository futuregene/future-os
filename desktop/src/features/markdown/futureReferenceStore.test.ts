import { act } from "react";
// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../test/renderHook";

import {
  peekFutureReference,
  queueFutureReferenceLoad,
  useFutureReference,
  useFutureReferences,
} from "./futureReferenceStore";

const resolveMock = vi.fn<(workspaceId: string, refs: unknown[]) => Promise<Array<Record<string, unknown>>>>();

vi.mock("../../integrations/storage/markdownReferences", () => ({
  resolveMarkdownReferences: (workspaceId: string, refs: unknown[]) => resolveMock(workspaceId, refs),
}));

type Listener = (event: { payload: { runId?: string; status?: string } }) => void;
let terminalListener: Listener | null = null;
const listenMock = vi.fn((_name: string, handler: Listener) => {
  terminalListener = handler;
  return Promise.resolve(() => {});
});

vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, handler: Listener) => listenMock(name, handler),
}));

beforeEach(() => {
  vi.useFakeTimers();
  // Records persist across tests (module-level cache) — each test uses its own
  // workspace id; only the call counts reset.
  resolveMock.mockClear();
});

afterEach(() => {
  vi.useRealTimers();
});

const RUN = { targetType: "run" as const, targetId: "run-1" };

/** Advance fake time in 1s steps until the resolve call count reaches n. */
async function advanceUntilCalls(expected: number, maxMs: number) {
  for (let elapsed = 0; elapsed < maxMs; elapsed += 1_000) {
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    await flush();
    if (resolveMock.mock.calls.length >= expected)
      return;
  }
  throw new Error(`timed out waiting for ${expected} resolve calls`);
}

function resolvedRecord(targetId = "run-1") {
  return { targetType: "run", targetId, status: "resolved", data: null };
}

/** Flush the 0ms batching timer + the (multi-tick) resolve promise chain. */
async function flush() {
  // The async advance variant awaits timer callbacks' promise chains; two
  // rounds cover flush-then-store (and any re-queued 0ms flush).
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
    await vi.advanceTimersByTimeAsync(0);
  });
}

describe("futureReferenceStore", () => {
  it("batches multiple queued loads into one pending flush", async () => {
    resolveMock.mockResolvedValue([resolvedRecord("run-batch")]);
    queueFutureReferenceLoad("w-batch", [
      { targetType: "run", targetId: "run-batch" },
      { targetType: "run", targetId: "run-batch-2" },
    ]);
    // A second synchronous queue hits the already-pending flush guard.
    queueFutureReferenceLoad("w-batch", [{ targetType: "run", targetId: "run-batch" }]);
    await flush();
    expect(resolveMock).toHaveBeenCalledTimes(1);
  });

  it("ignores empty load requests", () => {
    queueFutureReferenceLoad("w1", []);
    expect(resolveMock).not.toHaveBeenCalled();
  });

  it("resolves, caches, and skips re-resolving resolved records", async () => {
    resolveMock.mockResolvedValue([resolvedRecord()]);
    queueFutureReferenceLoad("w1", [RUN]);
    await flush();
    expect(resolveMock).toHaveBeenCalledTimes(1);
    expect(peekFutureReference("w1", RUN)).toMatchObject({ status: "resolved" });
    // Resolved records never re-resolve.
    queueFutureReferenceLoad("w1", [RUN]);
    await flush();
    expect(resolveMock).toHaveBeenCalledTimes(1);
  });

  it("returns undefined for a null workspace and unknown records", () => {
    expect(peekFutureReference(null, RUN)).toBeUndefined();
    expect(peekFutureReference("w1", { targetType: "run", targetId: "nope" })).toBeUndefined();
  });

  it("marks failures and retries on an escalating backoff until parked", async () => {
    resolveMock.mockImplementation((_w: string, refs: any[]) =>
      Promise.resolve(refs.map((r: any) => ({ ...r, status: "missing", data: undefined }))));
    queueFutureReferenceLoad("w2", [RUN]);
    await flush();
    expect(resolveMock).toHaveBeenCalledTimes(1);
    expect(peekFutureReference("w2", RUN)).toMatchObject({ status: "missing" });

    // Not due yet: an explicit re-queue is ignored.
    queueFutureReferenceLoad("w2", [RUN]);
    await flush();
    expect(resolveMock).toHaveBeenCalledTimes(1);

    // A second record with a later deadline: the first sweep re-queues w2 but
    // skips w2b (not due) and re-arms.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    queueFutureReferenceLoad("w2b", [{ targetType: "run", targetId: "run-2" }]);
    await flush();
    expect(resolveMock).toHaveBeenCalledTimes(2);

    // Advance until each retry lands (deadline-boundary agnostic).
    await advanceUntilCalls(3, 25_000); // w2 first retry (~30s mark)
    await advanceUntilCalls(4, 15_000); // w2b first retry (~40s mark)
    await advanceUntilCalls(5, 70_000); // w2 second retry (60s backoff) → parked
    await advanceUntilCalls(6, 70_000); // w2b second retry → parked

    // Parked: no further IPC even after a long wait + explicit requeue.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(600_000);
    });
    await flush();
    queueFutureReferenceLoad("w2", [RUN]);
    queueFutureReferenceLoad("w2b", [{ targetType: "run", targetId: "run-2" }]);
    await flush();
    expect(resolveMock).toHaveBeenCalledTimes(6);
  });

  it("stores a failed record when the resolve IPC rejects", async () => {
    resolveMock.mockRejectedValue(new Error("ipc down"));
    queueFutureReferenceLoad("w3", [RUN]);
    await flush();
    expect(peekFutureReference("w3", RUN)).toMatchObject({ status: "failed", error: "ipc down" });
  });

  it("loads via the useFutureReferences hook only with a workspace and references", async () => {
    resolveMock.mockResolvedValue([resolvedRecord("run-hook")]);
    const hookRef = { source: "inline" as const, targetType: "run" as const, targetId: "run-hook", view: "chip" as const };
    let workspaceId: string | null = null;
    const h = renderHook(() => useFutureReferences(workspaceId, [hookRef]));
    await flush();
    expect(resolveMock).not.toHaveBeenCalled();
    workspaceId = "w4";
    h.rerender();
    await flush();
    expect(resolveMock).toHaveBeenCalledWith("w4", [{ targetType: "run", targetId: "run-hook" }]);
    h.unmount();
  });

  it("useFutureReference subscribes to cache updates", async () => {
    resolveMock.mockResolvedValue([resolvedRecord("run-sub")]);
    const subRef = { targetType: "run" as const, targetId: "run-sub" };
    const h = renderHook(() => useFutureReference("w5", subRef));
    expect(h.current).toBeUndefined();
    queueFutureReferenceLoad("w5", [subRef]);
    await flush();
    expect(h.current).toMatchObject({ status: "resolved" });
    h.unmount();
  });

  it("installs the terminal-run listener once and re-resolves settled runs in place", async () => {
    resolveMock.mockResolvedValue([resolvedRecord("run-term")]);
    queueFutureReferenceLoad("w6", [{ targetType: "run", targetId: "run-term" }]);
    await flush();
    const listenCalls = listenMock.mock.calls.length;
    queueFutureReferenceLoad("w6b", [{ targetType: "run", targetId: "run-other" }]);
    await flush();
    expect(listenMock.mock.calls.length).toBe(listenCalls);

    // Non-terminal payloads are ignored.
    const callsBefore = resolveMock.mock.calls.length;
    terminalListener!({ payload: { runId: "run-term", status: "running" } });
    terminalListener!({ payload: { status: "completed" } });
    await flush();
    expect(resolveMock.mock.calls.length).toBe(callsBefore);

    // Terminal status re-resolves exactly that run's records.
    resolveMock.mockResolvedValue([resolvedRecord("run-term")]);
    await act(async () => {
      terminalListener!({ payload: { runId: "run-term", status: "completed" } });
      await Promise.resolve();
    });
    await flush();
    expect(resolveMock).toHaveBeenCalledWith("w6", [{ targetType: "run", targetId: "run-term" }]);
  });

  it("prunes records past the 1000-entry cap", async () => {
    const many = Array.from({ length: 1005 }, (_, i) => ({ targetType: "artifact" as const, targetId: `art-${i}` }));
    resolveMock.mockResolvedValue(
      many.map(r => ({ targetType: r.targetType, targetId: r.targetId, status: "resolved", data: null })),
    );
    queueFutureReferenceLoad("w7", many);
    await flush();
    // Oldest entries evicted: art-0..art-4 gone, the rest present.
    expect(peekFutureReference("w7", { targetType: "artifact", targetId: "art-0" })).toBeUndefined();
    expect(peekFutureReference("w7", { targetType: "artifact", targetId: "art-5" })).toBeDefined();
  });
});
