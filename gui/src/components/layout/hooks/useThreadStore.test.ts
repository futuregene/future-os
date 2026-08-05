import { describe, expect, it } from "vitest";
import { reduceThreadRunStatus } from "./useThreadStore";

describe("reduceThreadRunStatus", () => {
  it("rejects an older run update after a newer run started on the same thread", () => {
    const running = reduceThreadRunStatus({}, {
      threadId: "thread-1",
      runId: "run-new",
      revision: 12,
      status: "running",
      resetProjection: false,
    });
    const staleTerminal = reduceThreadRunStatus(running, {
      threadId: "thread-1",
      runId: "run-old",
      revision: 11,
      status: "completed",
      resetProjection: false,
    }, 100);

    expect(staleTerminal).toBe(running);
    expect(staleTerminal["thread-1"]).toMatchObject({
      runId: "run-new",
      revision: 12,
      status: "running",
      endedAt: null,
    });
  });

  it("applies a newer terminal update with its run identity and revision", () => {
    const result = reduceThreadRunStatus({}, {
      threadId: "thread-1",
      runId: "run-1",
      revision: 7,
      status: "cancelled",
      resetProjection: false,
    }, 1234);

    expect(result["thread-1"]).toEqual({
      runId: "run-1",
      revision: 7,
      status: "cancelled",
      endedAt: 1234,
    });
  });

  it("keeps the terminal status when the collector's trailing push arrives after the abort", () => {
    const running = reduceThreadRunStatus({}, {
      threadId: "thread-1",
      runId: "run-1",
      revision: 1,
      status: "running",
      resetProjection: false,
    });
    const cancelled = reduceThreadRunStatus(running, {
      threadId: "thread-1",
      runId: "run-1",
      revision: 2,
      status: "cancelled",
      resetProjection: false,
    }, 500);
    // Abort race: the run row is already cancelled when the collector drains
    // the stream and emits its trailing "finalizing" with a HIGHER revision.
    const afterTrailing = reduceThreadRunStatus(cancelled, {
      threadId: "thread-1",
      runId: "run-1",
      revision: 3,
      status: "finalizing",
      resetProjection: false,
    });

    expect(afterTrailing).toBe(cancelled);
    expect(afterTrailing["thread-1"]).toMatchObject({ status: "cancelled", endedAt: 500 });
  });

  it("lets a new run replace a terminal entry on the same thread", () => {
    const cancelled = reduceThreadRunStatus({}, {
      threadId: "thread-1",
      runId: "run-1",
      revision: 2,
      status: "cancelled",
      resetProjection: false,
    });
    const nextRun = reduceThreadRunStatus(cancelled, {
      threadId: "thread-1",
      runId: "run-2",
      revision: 3,
      status: "running",
      resetProjection: false,
    });

    expect(nextRun["thread-1"]).toMatchObject({ runId: "run-2", status: "running", endedAt: null });
  });
});
