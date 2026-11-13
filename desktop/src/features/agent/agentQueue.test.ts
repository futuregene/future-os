// @vitest-environment jsdom
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../test/renderHook";
import {
  agentQueueNoticeReason,
  cancelAgentQueuedRun,
  editAgentQueuedRun,
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

    const { rows } = await fetchAgentQueue("thread-1");

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

    const { rows } = await fetchAgentQueue("thread-1");

    expect(rows).toHaveLength(1);
    expect(rows[0]!.runId).toBe("run-1");
  });

  it("is empty for a conversation with no queue, not an error", async () => {
    mocks.invokeCommand.mockResolvedValue({});

    await expect(fetchAgentQueue("thread-1")).resolves.toEqual({ acks: [], rows: [] });
  });

  /**
   * A waiting message's files.
   *
   * They are on this machine (the Agent runs here), which is what makes them both
   * openable from the panel and re-sendable by an edit. Without them an edit sent
   * the words alone and the photos the message carried were gone.
   */
  it("reads the files a waiting message carries, and tells absence from none", async () => {
    mocks.invokeCommand.mockResolvedValue({
      queued: [
        row({
          attachments: [
            { kind: "image", name: "shot.png", path: "/tmp/shot.png", size: 4096 },
            // No path: nothing to open and nothing to name again, so it is not
            // drawn as a chip that would do nothing.
            { name: "nameless.png" },
          ],
        }),
        row({ runId: "run-empty", attachments: [] }),
        row({ runId: "run-old" }),
      ],
    });

    const { rows } = await fetchAgentQueue("thread-1");

    expect(rows[0]!.attachments).toEqual([
      { kind: "image", name: "shot.png", path: "/tmp/shot.png", size: 4096 },
    ]);
    expect(rows[1]!.attachments).toEqual([]);
    // An Agent older than the field says nothing; the editor then warns that a
    // save sends the text alone instead of claiming the message carried no files.
    expect(rows[2]!.attachments).toBeUndefined();
  });

  /**
   * The other half of the answer: rows that left the queue without running.
   * Reading only `queued` is what let a cancelled or merged message disappear
   * with nobody told.
   */
  it("reads why rows left the queue, and skips ones it cannot explain", async () => {
    mocks.invokeCommand.mockResolvedValue({
      queued: [],
      recentTerminalAcks: [
        { reason: "merged", runId: "run-1", state: "terminal" },
        { reason: "cancelled", runId: "run-2", state: "terminal" },
        { reason: "", runId: "run-3", state: "terminal" },
        { reason: "superseded", state: "terminal" },
      ],
    });

    const { acks } = await fetchAgentQueue("thread-1");

    expect(acks).toEqual([
      { reason: "merged", runId: "run-1", state: "terminal" },
      { reason: "cancelled", runId: "run-2", state: "terminal" },
    ]);
  });

  /** A reason this client has no sentence for is named, not dropped or invented. */
  it("maps a reason to its sentence, and an unknown one to the catch-all", () => {
    expect(agentQueueNoticeReason("merged")).toBe("merged");
    expect(agentQueueNoticeReason("cancelled")).toBe("cancelled");
    expect(agentQueueNoticeReason("superseded")).toBe("superseded");
    expect(agentQueueNoticeReason("session_deleted")).toBe("session_deleted");
    expect(agentQueueNoticeReason("something_new")).toBe("other");
  });
});

describe("queue mutations", () => {
  it("addresses the row by its conversation and run id through the Agent's own commands", async () => {
    await cancelAgentQueuedRun("thread-1", "run-9");
    await releaseAgentQueuedRun("thread-1", "run-9");
    await editAgentQueuedRun("thread-1", "run-9", "corrected");

    expect(mocks.invokeCommand.mock.calls).toEqual([
      ["cancel_queued_run", { runId: "run-9", threadId: "thread-1" }],
      ["release_queued_run", { runId: "run-9", threadId: "thread-1" }],
      // The edit is its own command, not a cancel plus a send: the row keeps the
      // place the reader gave it.
      ["edit_queued_run", { message: "corrected", runId: "run-9", threadId: "thread-1" }],
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

  /**
   * A row that leaves the queue without running — folded into the turn ahead,
   * taken back on another device, or lost with a deleted conversation — is not
   * announced by the Agent as an event. Its terminal ack is the only trace, and
   * reading it is what stops the message from vanishing unremarked.
   */
  it("says what became of a row that left the queue after the panel opened", async () => {
    mocks.invokeCommand.mockResolvedValue({ queued: [row()], recentTerminalAcks: [] });
    const harness = renderHook(() => useAgentQueue("thread-1", true));
    await act(async () => {
      await Promise.resolve();
    });
    expect(harness.current.notice).toBeNull();

    mocks.invokeCommand.mockResolvedValue({
      queued: [],
      recentTerminalAcks: [{ reason: "merged", runId: "run-1", state: "terminal" }],
    });
    act(() => harness.current.refresh());
    await act(async () => {
      await Promise.resolve();
    });

    expect(harness.current.rows).toEqual([]);
    expect(harness.current.notice).toEqual({ count: 1, reason: "merged" });
    harness.unmount();
  });

  /** The Agent's window holds older acks: those are not news for this view. */
  it("does not announce acks that predate the panel, or say the same thing twice", async () => {
    const oldAck = { reason: "cancelled", runId: "run-old", state: "terminal" };
    mocks.invokeCommand.mockResolvedValue({ queued: [], recentTerminalAcks: [oldAck] });
    const harness = renderHook(() => useAgentQueue("thread-1", true));
    await act(async () => {
      await Promise.resolve();
    });
    expect(harness.current.notice).toBeNull();

    mocks.invokeCommand.mockResolvedValue({
      queued: [],
      recentTerminalAcks: [oldAck, { reason: "session_deleted", runId: "run-new", state: "terminal" }],
    });
    act(() => harness.current.refresh());
    await act(async () => {
      await Promise.resolve();
    });
    expect(harness.current.notice).toEqual({ count: 1, reason: "session_deleted" });

    const shown = harness.current.notice;
    act(() => harness.current.refresh());
    await act(async () => {
      await Promise.resolve();
    });
    expect(harness.current.notice).toBe(shown);
    harness.unmount();
  });

  /** Removing a row is this window's own act: its ack would read as a fault. */
  it("does not announce a removal this window made itself", async () => {
    mocks.invokeCommand.mockResolvedValue({ queued: [row()], recentTerminalAcks: [] });
    const harness = renderHook(() => useAgentQueue("thread-1", true));
    await act(async () => {
      await Promise.resolve();
    });

    act(() => harness.current.forget("run-1"));
    mocks.invokeCommand.mockResolvedValue({
      queued: [],
      recentTerminalAcks: [{ reason: "cancelled", runId: "run-1", state: "terminal" }],
    });
    act(() => harness.current.refresh());
    await act(async () => {
      await Promise.resolve();
    });

    expect(harness.current.rows).toEqual([]);
    expect(harness.current.notice).toBeNull();
    harness.unmount();
  });
});
