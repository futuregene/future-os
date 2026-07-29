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
});
