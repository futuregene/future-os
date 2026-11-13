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
  QUEUE_NOTICE_MS,
  cancelRemoteQueuedRun,
  editRemoteQueuedRun,
  getRemoteQueue,
  isParkedRemoteQueueRow,
  parseRemoteQueueAcks,
  parseRemoteQueueRows,
  releaseRemoteQueuedRun,
  remoteQueueNoticeReason,
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

    await expect(getRemoteQueue("desktop_a", "sess_1")).resolves.toEqual({
      acks: [],
      rows: [
        { acceptedAt: "2026-11-03T10:00:00Z", position: 1, runId: "run_1", state: "queued", text: "first" },
      ],
    });
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

    const { rows } = await getRemoteQueue("desktop_a", "sess_1");

    expect(rows.map(row => row.text)).toEqual(["kept"]);
    // A host that says nothing about a row's state means the ordinary one.
    expect(rows[0]!.state).toBe("queued");
  });

  /**
   * A queued message's files, so the panel can show (and keep) them.
   *
   * The host's queue used to answer with text and an acceptance time only, which
   * left an edit — a cancel and a re-send, the only kind there is — no way to name
   * the files again, and left the reader looking at a message whose photos were
   * invisible.
   */
  it("reads the files a waiting message carries", async () => {
    request.mockResolvedValue({
      queued: [
        {
          runId: "run_files",
          text: "look at this",
          attachments: [
            { kind: "image", name: "shot.png", path: "/Users/x/shot.png", size: 4096 },
            { kind: "file", name: "doc.pdf", path: "/Users/x/doc.pdf", size: 0 },
          ],
        },
        // A file with no path could not be opened or named again, so it is not
        // drawn as a chip that does nothing.
        { runId: "run_partial", attachments: [{ name: "nameless.png" }] },
        { runId: "run_text", text: "later" },
      ],
    });

    const { rows } = await getRemoteQueue("desktop_a", "sess_1");

    expect(rows[0]!.attachments).toEqual([
      { kind: "image", name: "shot.png", path: "/Users/x/shot.png", size: 4096 },
      { kind: "file", name: "doc.pdf", path: "/Users/x/doc.pdf", size: 0 },
    ]);
    expect(rows[1]!.attachments).toEqual([]);
    // Absent is absence, not "a message with no files": an older host is told
    // apart from one that reported none.
    expect(rows[2]!.attachments).toBeUndefined();
  });

  it("reads an empty or unusable answer as an empty queue", async () => {
    request.mockResolvedValue(null);
    await expect(getRemoteQueue("desktop_a", "sess_1")).resolves.toEqual({ acks: [], rows: [] });
    expect(parseRemoteQueueRows(undefined)).toEqual([]);
    expect(parseRemoteQueueRows({ queued: "not a list" })).toEqual([]);
    // Both spellings of the rows are accepted: the reply itself, or its `queued`.
    expect(parseRemoteQueueRows([{ runId: "run_3", state: "stale:stale" }])[0]!.state).toBe("stale:stale");
  });

  /**
   * The other half of the answer: rows that left the queue without running.
   * Reading only `queued` is what let a cancelled or merged message disappear
   * with nobody told.
   */
  it("reads why rows left the queue, and skips ones it cannot explain", async () => {
    request.mockResolvedValue({
      queued: [],
      recentTerminalAcks: [
        { reason: "merged", runId: "run_1", state: "terminal" },
        { reason: "session_deleted", runId: "run_2", state: "terminal" },
        { reason: "", runId: "run_3", state: "terminal" },
        { reason: "cancelled", state: "terminal" },
      ],
    });

    const { acks } = await getRemoteQueue("desktop_a", "sess_1");

    expect(acks).toEqual([
      { reason: "merged", runId: "run_1", state: "terminal" },
      { reason: "session_deleted", runId: "run_2", state: "terminal" },
    ]);
    expect(parseRemoteQueueAcks(undefined)).toEqual([]);
    expect(parseRemoteQueueAcks({ runId: "run_1" })).toEqual([]);
  });

  /** A reason this client has no sentence for is named, not dropped or invented. */
  it("maps a reason to its sentence, and an unknown one to the catch-all", () => {
    expect(remoteQueueNoticeReason("merged")).toBe("merged");
    expect(remoteQueueNoticeReason("cancelled")).toBe("cancelled");
    expect(remoteQueueNoticeReason("superseded")).toBe("superseded");
    expect(remoteQueueNoticeReason("session_deleted")).toBe("session_deleted");
    expect(remoteQueueNoticeReason("something_new")).toBe("other");
  });
});

describe("the queue's commands", () => {
  it("cancels, releases and edits by run id, on the conversation's lane", async () => {
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

    // An edit names the row and the new words: the host corrects that row where
    // it waits instead of queueing a new one behind the reader's later messages.
    await editRemoteQueuedRun("desktop_a", "sess_1", "run_7", "fixed");
    expect(request).toHaveBeenLastCalledWith(
      "desktop_a",
      { message: "fixed", runId: "run_7", sessionId: "sess_1", type: "edit_queued_run" },
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

  /**
   * A row that leaves the queue without running — folded into the turn ahead,
   * taken back on another device, or lost with a deleted conversation — is not
   * announced by the host as an event. Its terminal ack is the only trace, and
   * reading it is what stops the message from vanishing unremarked.
   */
  it("says what became of a row that left the queue after the panel opened", async () => {
    request.mockResolvedValue({ queued: [{ runId: "run_1", text: "waiting" }], recentTerminalAcks: [] });
    const hook = renderHook(() => useRemoteQueue("desktop_a", "sess_1", true));
    await flushAsync();
    expect(hook.current.notice).toBeNull();

    // Next read: the row is gone, and the host says why.
    request.mockResolvedValue({
      queued: [],
      recentTerminalAcks: [{ reason: "merged", runId: "run_1", state: "terminal" }],
    });
    act(() => hook.current.refresh());
    await flushAsync();

    expect(hook.current.rows).toEqual([]);
    expect(hook.current.notice).toEqual({ count: 1, reason: "merged" });
    hook.unmount();
  });

  /**
   * The host keeps a window of acks older than this view. Reading a conversation
   * must not greet the reader with news about a row that was merged before they
   * opened it, and a poll must not repeat a sentence already said.
   */
  it("does not announce acks that predate the panel, or say the same thing twice", async () => {
    const oldAck = { reason: "cancelled", runId: "run_old", state: "terminal" };
    request.mockResolvedValue({ queued: [], recentTerminalAcks: [oldAck] });
    const hook = renderHook(() => useRemoteQueue("desktop_a", "sess_1", true));
    await flushAsync();
    expect(hook.current.notice).toBeNull();

    // A fresh ack arrives alongside the old one; only the fresh one is news.
    request.mockResolvedValue({
      queued: [],
      recentTerminalAcks: [oldAck, { reason: "session_deleted", runId: "run_new", state: "terminal" }],
    });
    act(() => hook.current.refresh());
    await flushAsync();
    expect(hook.current.notice).toEqual({ count: 1, reason: "session_deleted" });

    // The same acks are still inside the host's window on the next read.
    const shown = hook.current.notice;
    act(() => hook.current.refresh());
    await flushAsync();
    expect(hook.current.notice).toBe(shown);
    hook.unmount();
  });

  /**
   * Removing a row is this window's own act. Its ack would otherwise come back
   * as "a message was taken out of the queue" right after the reader took it
   * out, which reads as a fault rather than as confirmation.
   */
  it("does not announce a removal this window made itself", async () => {
    request.mockResolvedValue({ queued: [{ runId: "run_1", text: "waiting" }], recentTerminalAcks: [] });
    const hook = renderHook(() => useRemoteQueue("desktop_a", "sess_1", true));
    await flushAsync();

    act(() => hook.current.forget("run_1"));
    request.mockResolvedValue({
      queued: [],
      recentTerminalAcks: [{ reason: "cancelled", runId: "run_1", state: "terminal" }],
    });
    act(() => hook.current.refresh());
    await flushAsync();

    expect(hook.current.rows).toEqual([]);
    expect(hook.current.notice).toBeNull();
    hook.unmount();
  });

  /** A notice is news, not state: it says its piece once and goes away. */
  it("takes the notice off the screen after a while", async () => {
    vi.useFakeTimers();
    try {
      request.mockResolvedValue({ queued: [{ runId: "run_1", text: "waiting" }], recentTerminalAcks: [] });
      const hook = renderHook(() => useRemoteQueue("desktop_a", "sess_1", true));
      await flushAsync();

      request.mockResolvedValue({
        queued: [],
        recentTerminalAcks: [{ reason: "merged", runId: "run_1", state: "terminal" }],
      });
      act(() => hook.current.refresh());
      await flushAsync();
      expect(hook.current.notice).not.toBeNull();

      act(() => {
        vi.advanceTimersByTime(QUEUE_NOTICE_MS);
      });
      expect(hook.current.notice).toBeNull();
      hook.unmount();
    }
    finally {
      vi.useRealTimers();
    }
  });
});
