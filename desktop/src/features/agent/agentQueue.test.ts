// @vitest-environment jsdom
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../test/renderHook";
import {
  cancelAgentQueuedRun,
  fetchAgentQueue,
  isParkedRow,
  releaseAgentQueuedRun,
  useAgentQueue,
} from "./agentQueue";

const mocks = vi.hoisted(() => ({ invokeCommand: vi.fn() }));

vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: (...args: unknown[]) => mocks.invokeCommand(...args),
}));

/** One row as `session_queue` answers it (the Host's normalised shape). */
function row(over: Record<string, unknown> = {}) {
  return {
    runId: "run-1",
    position: 1,
    acceptedAt: "2026-11-03T10:00:00Z",
    text: "waiting here",
    state: "queued",
    ...over,
  };
}

beforeEach(() => {
  mocks.invokeCommand.mockReset();
  mocks.invokeCommand.mockResolvedValue({ activeRunId: "run-0", queued: [], recentTerminalAcks: [] });
});

describe("fetchAgentQueue", () => {
  it("reads the conversation's rows from the Agent, in the Agent's order", async () => {
    mocks.invokeCommand.mockResolvedValue({
      activeRunId: "run-0",
      queued: [row(), row({ runId: "run-2", position: 2, text: "second", state: "stale:stale" })],
      recentTerminalAcks: [],
    });

    const rows = await fetchAgentQueue("thread-1");

    expect(mocks.invokeCommand).toHaveBeenCalledWith("session_queue", { threadId: "thread-1" });
    expect(rows.map(entry => entry.runId)).toEqual(["run-1", "run-2"]);
    expect(rows[1]!.state).toBe("stale:stale");
    expect(isParkedRow(rows[0]!)).toBe(false);
    expect(isParkedRow(rows[1]!)).toBe(true);
  });

  it("drops rows without a run id instead of rendering an untargetable one", async () => {
    // A row the panel cannot address is a row it cannot remove or start, so it
    // must not be drawn at all.
    mocks.invokeCommand.mockResolvedValue({
      queued: [{ position: 1, text: "no id" }, row()],
    });

    const rows = await fetchAgentQueue("thread-1");

    expect(rows).toHaveLength(1);
    expect(rows[0]!.runId).toBe("run-1");
  });

  it("is empty for a conversation with no queue, not an error", async () => {
    mocks.invokeCommand.mockResolvedValue({});

    await expect(fetchAgentQueue("thread-1")).resolves.toEqual([]);
  });
});

describe("queue mutations", () => {
  it("addresses the row by run id through the Agent's own commands", async () => {
    await cancelAgentQueuedRun("run-9");
    await releaseAgentQueuedRun("run-9");

    expect(mocks.invokeCommand.mock.calls).toEqual([
      ["cancel_queued_run", { runId: "run-9" }],
      ["release_queued_run", { runId: "run-9" }],
    ]);
  });
});

describe("useAgentQueue", () => {
  it("reads on mount and again when the view asks for a refresh", async () => {
    mocks.invokeCommand.mockResolvedValue({ queued: [row()] });
    const harness = renderHook(() => useAgentQueue("thread-1", false));
    await act(async () => {
      await Promise.resolve();
    });
    expect(harness.current.rows.map(entry => entry.runId)).toEqual(["run-1"]);

    mocks.invokeCommand.mockResolvedValue({ queued: [] });
    act(() => harness.current.refresh());
    await act(async () => {
      await Promise.resolve();
    });
    expect(harness.current.rows).toEqual([]);
    harness.unmount();
  });

  it("keeps the last rows when a read fails", async () => {
    mocks.invokeCommand.mockResolvedValue({ queued: [row()] });
    const harness = renderHook(() => useAgentQueue("thread-1", false));
    await act(async () => {
      await Promise.resolve();
    });

    mocks.invokeCommand.mockRejectedValue(new Error("agent offline"));
    act(() => harness.current.refresh());
    await act(async () => {
      await Promise.resolve();
    });
    // A read failure is not a queue emptying: the panel keeps what it showed.
    expect(harness.current.rows.map(entry => entry.runId)).toEqual(["run-1"]);
    harness.unmount();
  });

  it("does not answer a conversation the view already left", async () => {
    // The read is in flight when the view switches conversation; its answer must
    // not land in the next conversation's panel.
    const pending: ((value: unknown) => void)[] = [];
    mocks.invokeCommand.mockImplementation(() => new Promise((resolve) => {
      pending.push(resolve);
    }));
    let threadId = "thread-1";
    const harness = renderHook(() => useAgentQueue(threadId, false));
    const firstRead = pending[0]!;

    threadId = "thread-2";
    harness.rerender();
    await act(async () => {
      // The answer to thread-1's read arrives after the switch.
      firstRead({ queued: [row({ runId: "run-from-1" })] });
      await Promise.resolve();
    });

    expect(harness.current.rows).toEqual([]);
    harness.unmount();
  });
});
