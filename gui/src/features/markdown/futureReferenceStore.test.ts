import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeCommand = vi.fn();
const { runtimeHandlers } = vi.hoisted(() => ({
  runtimeHandlers: [] as Array<(event: { payload: Record<string, unknown> }) => void>,
}));

vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: (...args: unknown[]) => invokeCommand(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: Record<string, unknown> }) => void) => {
    if (name === "thread-runtime-updated")
      runtimeHandlers.push(handler);
    return () => {};
  }),
}));

describe("futureReferenceStore", () => {
  beforeEach(() => {
    invokeCommand.mockReset();
    runtimeHandlers.length = 0;
    vi.resetModules();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("actively retries a failed resolution after the backoff, without any re-render", async () => {
    invokeCommand.mockRejectedValueOnce(new Error("ipc down"));
    const { queueFutureReferenceLoad, peekFutureReference } = await import("./futureReferenceStore");
    const identity = { targetId: "run_retry", targetType: "run" as const };

    queueFutureReferenceLoad("ws_retry", [identity]);
    await vi.advanceTimersByTimeAsync(1);

    expect(invokeCommand).toHaveBeenCalledTimes(1);
    expect(peekFutureReference("ws_retry", identity)?.status).toBe("failed");

    invokeCommand.mockResolvedValueOnce([
      { targetType: "run", targetId: "run_retry", status: "resolved", data: { id: "run_retry" } },
    ]);
    // Nothing re-renders or re-queues: the scheduled sweep alone must heal
    // the record (the row-not-yet-committed race, a transient IPC error).
    await vi.advanceTimersByTimeAsync(31_000);

    expect(invokeCommand).toHaveBeenCalledTimes(2);
    expect(peekFutureReference("ws_retry", identity)).toMatchObject({
      status: "resolved",
      targetId: "run_retry",
    });
  });

  it("does not retry before the backoff elapses", async () => {
    invokeCommand.mockRejectedValueOnce(new Error("ipc down"));
    const { queueFutureReferenceLoad } = await import("./futureReferenceStore");

    queueFutureReferenceLoad("ws_wait", [{ targetId: "run_wait", targetType: "run" }]);
    await vi.advanceTimersByTimeAsync(1);
    expect(invokeCommand).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(29_000);
    expect(invokeCommand).toHaveBeenCalledTimes(1);
  });

  it("re-resolves a run record in place when the run settles", async () => {
    invokeCommand.mockResolvedValueOnce([
      { targetType: "run", targetId: "run_settle", status: "resolved", data: { id: "run_settle", status: "running" } },
    ]);
    const { queueFutureReferenceLoad, peekFutureReference } = await import("./futureReferenceStore");
    const identity = { targetId: "run_settle", targetType: "run" as const };

    queueFutureReferenceLoad("ws_settle", [identity]);
    await vi.advanceTimersByTimeAsync(1);
    expect(peekFutureReference("ws_settle", identity)).toMatchObject({
      status: "resolved",
      data: { status: "running" },
    });

    invokeCommand.mockResolvedValueOnce([
      { targetType: "run", targetId: "run_settle", status: "resolved", data: { id: "run_settle", status: "completed" } },
    ]);
    const handler = runtimeHandlers[runtimeHandlers.length - 1];
    expect(handler).toBeDefined();
    handler?.({ payload: { runId: "run_settle", status: "completed", threadId: "thread_1" } });
    await vi.advanceTimersByTimeAsync(1);

    expect(invokeCommand).toHaveBeenCalledTimes(2);
    expect(peekFutureReference("ws_settle", identity)).toMatchObject({
      status: "resolved",
      data: { status: "completed" },
    });
  });

  it("parks a permanently missing reference instead of retrying forever", async () => {
    invokeCommand.mockResolvedValue([
      { targetType: "run", targetId: "run_gone", status: "missing", error: "run was not found" },
    ]);
    const { queueFutureReferenceLoad, peekFutureReference } = await import("./futureReferenceStore");
    const identity = { targetId: "run_gone", targetType: "run" as const };

    queueFutureReferenceLoad("ws_gone", [identity]);
    await vi.advanceTimersByTimeAsync(1);
    expect(invokeCommand).toHaveBeenCalledTimes(1);

    // Attempt 2 after the first backoff (30s)...
    await vi.advanceTimersByTimeAsync(31_000);
    expect(invokeCommand).toHaveBeenCalledTimes(2);
    // ...attempt 3 after the escalated one (60s)...
    await vi.advanceTimersByTimeAsync(61_000);
    expect(invokeCommand).toHaveBeenCalledTimes(3);

    // ...then it parks: no further IPC no matter how long the page sits,
    // and the record keeps its terminal missing status for the chip.
    await vi.advanceTimersByTimeAsync(3_600_000);
    expect(invokeCommand).toHaveBeenCalledTimes(3);
    expect(peekFutureReference("ws_gone", identity)).toMatchObject({
      status: "missing",
      targetId: "run_gone",
    });
  });

  it("keeps resolved records final across streaming-delta re-queues", async () => {
    invokeCommand.mockResolvedValue([
      { targetType: "run", targetId: "run_final", status: "resolved", data: { id: "run_final" } },
    ]);
    const { queueFutureReferenceLoad } = await import("./futureReferenceStore");
    const identity = { targetId: "run_final", targetType: "run" as const };

    queueFutureReferenceLoad("ws_final", [identity]);
    await vi.advanceTimersByTimeAsync(1);
    expect(invokeCommand).toHaveBeenCalledTimes(1);

    // The parsed references array gets a fresh identity on every streaming
    // delta; already-resolved records must not re-fire IPC.
    queueFutureReferenceLoad("ws_final", [identity]);
    queueFutureReferenceLoad("ws_final", [identity]);
    await vi.advanceTimersByTimeAsync(31_000);
    expect(invokeCommand).toHaveBeenCalledTimes(1);
  });
});
