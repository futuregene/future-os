// @vitest-environment jsdom
import type { RemoteQueueRow } from "./remoteQueue";
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../test/renderHook";

/**
 * A paired computer's queue, as this client reads and changes it.
 *
 * The rows belong to *that* machine's Agent — the whole reason the queue moved
 * out of this app — so what is asserted here is the wire: the command, the lane it
 * rides, and what the answer is read as. The composer's own behaviour (which
 * control asks for what) is `RemoteComposer.test.tsx`'s.
 */
const request = vi.fn<(desktopId: string, command: Record<string, unknown>, lane: string) => Promise<unknown>>();
vi.mock("./remotePeerClient", () => ({
  requestRemotePeer: (...args: Parameters<typeof request>) => request(...args),
}));

const {
  QUEUED_SEND_POLICY,
  cancelRemoteQueuedRun,
  getRemoteQueue,
  isParkedRemoteQueueRow,
  parseRemoteQueueRows,
  releaseRemoteQueuedRun,
  useRemoteQueue,
} = await import("./remoteQueue");

beforeEach(() => {
  request.mockReset();
});

describe("what a queued remote send names", () => {
  /** The same policy the local GUI path uses, and the reason it is that one. */
  it("uses the policy a session folds across a turn boundary", () => {
    // `enqueue_if_busy` would hold the rows as separate turns; `coalescing`
    // joins the consecutive ones into a single message, which is the behaviour
    // the clients used to implement in their own memory.
    expect(QUEUED_SEND_POLICY).toBe("enqueue_coalescing");
  });
});

describe("getRemoteQueue", () => {
  it("reads the conversation's own queue, on its lane", async () => {
    request.mockResolvedValue({
      activeRunId: "run_live",
      queued: [
        { acceptedAt: "2026-11-03T10:00:00Z", position: 1, runId: "run_1", state: "queued", text: "first" },
      ],
      recentTerminalAcks: [],
    });

    await expect(getRemoteQueue("desktop_a", "sess_1")).resolves.toEqual([
      { acceptedAt: "2026-11-03T10:00:00Z", position: 1, runId: "run_1", state: "queued", text: "first" },
    ]);
    expect(request).toHaveBeenCalledWith(
      "desktop_a",
      { sessionId: "sess_1", type: "get_queue" },
      "sess_1",
    );
  });

  /**
   * A row this client could neither remove nor start would be a message with no
   * way back out, so it is dropped rather than drawn.
   */
  it("drops a row it could not address and keeps the rest", async () => {
    request.mockResolvedValue({ queued: [{ position: 1, text: "no id" }, { runId: "run_2", text: "kept" }] });

    const rows = await getRemoteQueue("desktop_a", "sess_1");

    expect(rows.map(row => row.text)).toEqual(["kept"]);
    // A host that says nothing about a row's state means the ordinary one.
    expect(rows[0]!.state).toBe("queued");
  });

  it("reads an empty or unusable answer as an empty queue", async () => {
    request.mockResolvedValue(null);
    await expect(getRemoteQueue("desktop_a", "sess_1")).resolves.toEqual([]);
    expect(parseRemoteQueueRows(undefined)).toEqual([]);
    expect(parseRemoteQueueRows({ queued: "not a list" })).toEqual([]);
    // Both spellings of the rows are accepted: the reply itself, or its `queued`.
    expect(parseRemoteQueueRows([{ runId: "run_3", state: "stale:stale" }])[0]!.state).toBe("stale:stale");
  });
});

describe("the queue's commands", () => {
  it("cancels and releases by run id, on the conversation's lane", async () => {
    request.mockResolvedValue(undefined);

    await cancelRemoteQueuedRun("desktop_a", "sess_1", "run_7");
    expect(request).toHaveBeenLastCalledWith(
      "desktop_a",
      { runId: "run_7", sessionId: "sess_1", type: "cancel_queued_run" },
      "sess_1",
    );

    await releaseRemoteQueuedRun("desktop_a", "sess_1", "run_7");
    expect(request).toHaveBeenLastCalledWith(
      "desktop_a",
      { runId: "run_7", sessionId: "sess_1", type: "release_queued_run" },
      "sess_1",
    );
  });

  it("tells a parked row from one that will run by itself", () => {
    const row = (state: string): RemoteQueueRow => ({ acceptedAt: "", position: 1, runId: "run_1", state, text: "" });
    expect(isParkedRemoteQueueRow(row("stale:stale"))).toBe(true);
    // The age could not be read: still parked, just for a reason this client
    // cannot spell out.
    expect(isParkedRemoteQueueRow(row("stale:unknown_age"))).toBe(true);
    expect(isParkedRemoteQueueRow(row("queued"))).toBe(false);
  });
});

describe("useRemoteQueue", () => {
  it("reads that host's queue when the view opens", async () => {
    request.mockResolvedValue({ queued: [{ runId: "run_1", text: "waiting" }] });

    const hook = renderHook(() => useRemoteQueue("desktop_a", "sess_1", true));
    await flushAsync();

    expect(request).toHaveBeenCalledWith("desktop_a", { sessionId: "sess_1", type: "get_queue" }, "sess_1");
    expect(hook.current.rows.map(row => row.text)).toEqual(["waiting"]);
    hook.unmount();
  });

  /**
   * A send accepted elsewhere — this window's own, or a phone's — is not an
   * event the host announces, so the panel is re-read on request rather than
   * waiting for a poll that is not running on an idle conversation.
   */
  it("reads again when asked", async () => {
    request.mockResolvedValue({ queued: [] });
    const hook = renderHook(() => useRemoteQueue("desktop_a", "sess_1", false));
    await flushAsync();
    expect(hook.current.rows).toEqual([]);

    request.mockResolvedValue({ queued: [{ runId: "run_2", text: "just queued" }] });
    act(() => hook.current.refresh());
    await flushAsync();

    expect(hook.current.rows.map(row => row.text)).toEqual(["just queued"]);
    hook.unmount();
  });

  /** A draft has no conversation on that host, so there is no queue to ask about. */
  it("does not ask about a conversation that does not exist yet", async () => {
    const hook = renderHook(() => useRemoteQueue("desktop_a", "", true));
    await flushAsync();

    expect(request).not.toHaveBeenCalled();
    expect(hook.current.rows).toEqual([]);
    hook.unmount();
  });
});
