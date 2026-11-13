import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import {
  cancelSessionQueuedRun,
  fetchSessionQueue,
  isParkedSessionQueueRow,
  QUEUE_NOTICE_MS,
  QUEUE_POLL_MS,
  releaseSessionQueuedRun,
  sessionQueueNoticeReason,
  useSessionQueue,
  type SessionQueueRow,
  type SessionQueueSnapshot,
  type SessionQueueView,
} from "../sessionQueue";
import type { RemoteClient } from "../client";

function clientFor(reply: unknown) {
  return {
    requestRetry: jest.fn(async () => ({ data: reply, success: true })),
  } as unknown as RemoteClient & { requestRetry: jest.Mock };
}

describe("reading the session's queue", () => {
  test("a conversation's rows come back in the session's own order and shape", async () => {
    const client = clientFor({
      activeRunId: "run_live",
      queued: [
        { acceptedAt: "2026-01-01T00:00:00Z", position: 1, runId: "run_a", state: "queued", text: "first" },
        { acceptedAt: "2026-01-01T00:01:00Z", position: 2, runId: "run_b", state: "stale:stale", text: "parked" },
      ],
      recentTerminalAcks: [],
    });
    const { rows } = await fetchSessionQueue(client, "session-1");
    expect(client.requestRetry).toHaveBeenCalledWith(
      { sessionId: "session-1", type: "get_queue" },
      "session-1",
    );
    expect(rows.map(row => row.runId)).toEqual(["run_a", "run_b"]);
    expect(rows[1]).toEqual({
      acceptedAt: "2026-01-01T00:01:00Z",
      position: 2,
      runId: "run_b",
      state: "stale:stale",
      text: "parked",
    });
  });

  test("a row without a run id is not a row, and a missing list is an empty queue", async () => {
    expect((await fetchSessionQueue(clientFor({ queued: [{ text: "no id" }] }), "s")).rows).toEqual([]);
    expect(await fetchSessionQueue(clientFor({}), "s")).toEqual({ acks: [], rows: [] });
    expect(await fetchSessionQueue(clientFor(null), "s")).toEqual({ acks: [], rows: [] });
  });

  /**
   * A waiting message's files travel with the row now, which is what lets the
   * panel open one or take it off. Three states, and the difference between the
   * last two matters: no files at all is an empty list, while a desktop that does
   * not report them leaves the field absent — a client that reads both as "none"
   * would offer to add a photo and quietly drop the ones it cannot see.
   */
  test("reads the files a waiting message carries, and tells 'none' from 'not reported'", async () => {
    const client = clientFor({
      queued: [
        {
          attachments: [
            { kind: "image", name: "shot.png", path: "/Users/x/shot.png", size: 2048 },
            { kind: "", name: "", path: "/Users/x/doc.pdf" },
            { name: "no path" },
          ],
          runId: "run_files",
          text: "look",
        },
        { attachments: [], runId: "run_text", text: "later" },
        { runId: "run_old", text: "older desktop" },
      ],
    });
    const { rows } = await fetchSessionQueue(client, "session-1");
    expect(rows[0]!.attachments).toEqual([
      { kind: "image", name: "shot.png", path: "/Users/x/shot.png", size: 2048 },
      // A file with no name is still a file: the path's last segment stands in.
      { kind: "", name: "doc.pdf", path: "/Users/x/doc.pdf", size: 0 },
    ]);
    expect(rows[0]!.attachments).toHaveLength(2);
    expect(rows[1]!.attachments).toEqual([]);
    expect(rows[2]!.attachments).toBeUndefined();
  });

  test("a draft with no session asks nothing at all", async () => {
    const client = clientFor({ queued: [] });
    expect(await fetchSessionQueue(client, "  ")).toEqual({ acks: [], rows: [] });
    expect(client.requestRetry).not.toHaveBeenCalled();
  });

  /**
   * The other half of the answer: rows that left the queue without running.
   * Reading only `queued` is what let a cancelled or merged message disappear
   * with nobody told.
   */
  test("reads why rows left the queue, and skips ones it cannot explain", async () => {
    const client = clientFor({
      queued: [],
      recentTerminalAcks: [
        { reason: "merged", runId: "run_a", state: "terminal" },
        { reason: "session_deleted", runId: "run_b", state: "terminal" },
        { reason: "", runId: "run_c", state: "terminal" },
        { reason: "cancelled", state: "terminal" },
      ],
    });

    const { acks } = await fetchSessionQueue(client, "session-1");

    expect(acks).toEqual([
      { reason: "merged", runId: "run_a", state: "terminal" },
      { reason: "session_deleted", runId: "run_b", state: "terminal" },
    ]);
  });

  /** A reason this phone has no sentence for is named, not dropped or invented. */
  test("maps a reason to its sentence, and an unknown one to the catch-all", () => {
    expect(sessionQueueNoticeReason("merged")).toBe("merged");
    expect(sessionQueueNoticeReason("cancelled")).toBe("cancelled");
    expect(sessionQueueNoticeReason("superseded")).toBe("superseded");
    expect(sessionQueueNoticeReason("session_deleted")).toBe("session_deleted");
    expect(sessionQueueNoticeReason("something_new")).toBe("other");
  });

  test("a parked row is the one that waits for a person", () => {
    const row = (state: string): SessionQueueRow => ({
      acceptedAt: "", position: 1, runId: "run", state, text: "",
    });
    expect(isParkedSessionQueueRow(row("stale:stale"))).toBe(true);
    expect(isParkedSessionQueueRow(row("stale:unknown_age"))).toBe(true);
    expect(isParkedSessionQueueRow(row("queued"))).toBe(false);
  });
});

describe("changing the session's queue", () => {
  test("a cancel and a release address the session and the run", async () => {
    const client = clientFor(null);
    await cancelSessionQueuedRun(client, "session-1", "run_a");
    expect(client.requestRetry).toHaveBeenCalledWith(
      { runId: "run_a", sessionId: "session-1", type: "cancel_queued_run" },
      "session-1",
    );
    await releaseSessionQueuedRun(client, "session-1", "run_b");
    expect(client.requestRetry).toHaveBeenLastCalledWith(
      { runId: "run_b", sessionId: "session-1", type: "release_queued_run" },
      "session-1",
    );
  });

  test("a refusal reaches the caller instead of reading as success", async () => {
    const client = {
      requestRetry: jest.fn(async () => { throw new Error("session_not_on_desktop"); }),
    } as unknown as RemoteClient;
    await expect(cancelSessionQueuedRun(client, "s", "run_a")).rejects.toThrow("session_not_on_desktop");
  });
});

describe("the panel's view", () => {
  let tree: ReactTestRenderer | undefined;
  let view: SessionQueueView;

  function Panel({ read, sessionId, live, enabled }: {
    read: () => Promise<SessionQueueSnapshot>;
    sessionId: string;
    live: boolean;
    enabled: boolean;
  }) {
    view = useSessionQueue({ enabled, live, read, sessionId });
    return null;
  }

  const row = (runId: string): SessionQueueRow => ({
    acceptedAt: "", position: 1, runId, state: "queued", text: runId,
  });
  const reply = (
    rows: SessionQueueRow[],
    acks: SessionQueueSnapshot["acks"] = [],
  ): SessionQueueSnapshot => ({ acks, rows });

  beforeEach(() => jest.useFakeTimers());
  afterEach(() => {
    const mounted = tree;
    if (mounted) act(() => mounted.unmount());
    tree = undefined;
    jest.useRealTimers();
  });

  const mount = async (read: () => Promise<SessionQueueSnapshot>, sessionId = "s1", live = false) => {
    const element = createElement(Panel, { enabled: true, live, read, sessionId });
    await act(async () => {
      if (tree) tree.update(element);
      else tree = create(element);
    });
    await act(async () => { await Promise.resolve(); });
  };

  test("the rows are read once when the panel opens on an idle conversation", async () => {
    const read = jest.fn(async () => reply([row("run_a")]));
    await mount(read);
    expect(view.rows.map(item => item.runId)).toEqual(["run_a"]);
    expect(read).toHaveBeenCalledTimes(1);
    // Waiting rows keep it reading: one can start or be parked with no event.
    await act(async () => { jest.advanceTimersByTime(QUEUE_POLL_MS); });
    expect(read).toHaveBeenCalledTimes(2);
  });

  test("an empty queue on an idle conversation costs no timer", async () => {
    const read = jest.fn(async () => reply([]));
    await mount(read);
    expect(read).toHaveBeenCalledTimes(1);
    await act(async () => { jest.advanceTimersByTime(QUEUE_POLL_MS * 2); });
    expect(read).toHaveBeenCalledTimes(1);
  });

  test("a live run keeps reading, and a refresh reads immediately", async () => {
    const read = jest.fn(async () => reply([]));
    await mount(read);
    await mount(read, "s1", true);
    await act(async () => { jest.advanceTimersByTime(QUEUE_POLL_MS); });
    const afterPoll = read.mock.calls.length;
    expect(afterPoll).toBeGreaterThanOrEqual(2);
    await act(async () => { view.refresh(); });
    await act(async () => { await Promise.resolve(); });
    expect(read.mock.calls.length).toBeGreaterThan(afterPoll);
  });

  test("a read that fails leaves the last rows in place", async () => {
    const read = jest.fn(async () => reply([row("run_a")]));
    await mount(read);
    read.mockRejectedValueOnce(new Error("not_connected"));
    await act(async () => { jest.advanceTimersByTime(QUEUE_POLL_MS); });
    expect(view.rows.map(item => item.runId)).toEqual(["run_a"]);
  });

  /**
   * The poll's "nothing changed, do not re-render" comparison has to include the
   * files: a row whose text and place are the same but whose files changed (a
   * removal on another device) would otherwise keep drawing the old chips.
   */
  test("a poll that changes only a row's files still redraws it", async () => {
    const read = jest.fn(async () => reply([row("run_a")]));
    await mount(read, "s1", true);
    expect(view.rows[0]!.attachments).toBeUndefined();

    read.mockResolvedValue(reply([{
      ...row("run_a"),
      attachments: [{ kind: "image", name: "a.png", path: "/x/a.png", size: 12 }],
    }]));
    await act(async () => { jest.advanceTimersByTime(QUEUE_POLL_MS); });
    expect(view.rows[0]!.attachments).toEqual([
      { kind: "image", name: "a.png", path: "/x/a.png", size: 12 },
    ]);
  });

  test("another conversation's rows are never handed to this one", async () => {
    const resolvers: ((rows: SessionQueueSnapshot) => void)[] = [];
    const read = jest.fn(() => new Promise<SessionQueueSnapshot>(yes => { resolvers.push(yes); }));
    await mount(read, "s1");
    // The reader moves on while the first conversation's read is still in flight.
    await act(async () => {
      tree!.update(createElement(Panel, { enabled: true, live: false, read, sessionId: "s2" }));
    });
    // The read that belonged to the conversation the reader left lands now.
    await act(async () => { resolvers[0]!(reply([row("run_a")])); });
    await act(async () => { await Promise.resolve(); });
    expect(view.rows).toEqual([]);
  });

  /**
   * A row that leaves the queue without running — folded into the turn ahead,
   * taken back on the desktop, or lost with a deleted conversation — is not
   * announced as an event. Its terminal ack is the only trace, and reading it is
   * what stops the message from vanishing unremarked.
   */
  test("says what became of a row that left the queue after the screen opened", async () => {
    const read = jest.fn(async () => reply([row("run_a")]));
    await mount(read, "s1", true);
    expect(view.notice).toBeNull();

    read.mockResolvedValue(reply([], [{ reason: "merged", runId: "run_a", state: "terminal" }]));
    await act(async () => { jest.advanceTimersByTime(QUEUE_POLL_MS); });

    expect(view.rows).toEqual([]);
    expect(view.notice).toEqual({ count: 1, reason: "merged" });
  });

  /**
   * The desktop keeps a window of acks older than this screen. Opening a
   * conversation must not greet the reader with news about a row that was merged
   * before they got there, and a poll must not repeat a sentence already said.
   */
  test("does not announce acks that predate the screen, or say the same thing twice", async () => {
    const oldAck = { reason: "cancelled", runId: "run_old", state: "terminal" };
    const read = jest.fn(async () => reply([], [oldAck]));
    await mount(read, "s1", true);
    expect(view.notice).toBeNull();

    read.mockResolvedValue(reply([], [oldAck, { reason: "session_deleted", runId: "run_new", state: "terminal" }]));
    await act(async () => { jest.advanceTimersByTime(QUEUE_POLL_MS); });
    expect(view.notice).toEqual({ count: 1, reason: "session_deleted" });

    const shown = view.notice;
    await act(async () => { jest.advanceTimersByTime(QUEUE_POLL_MS); });
    expect(view.notice).toBe(shown);
  });

  /** Removing a row is this screen's own act: its ack would read as a fault. */
  test("does not announce a removal this screen made itself", async () => {
    const read = jest.fn(async () => reply([row("run_a")]));
    await mount(read, "s1", true);

    await act(async () => { view.forget("run_a"); });
    read.mockResolvedValue(reply([], [{ reason: "cancelled", runId: "run_a", state: "terminal" }]));
    await act(async () => { jest.advanceTimersByTime(QUEUE_POLL_MS); });

    expect(view.rows).toEqual([]);
    expect(view.notice).toBeNull();
  });

  /** A notice is news, not state: it says its piece once and goes. */
  test("takes the notice off the screen after a while", async () => {
    const read = jest.fn(async () => reply([row("run_a")]));
    await mount(read, "s1", true);
    read.mockResolvedValue(reply([], [{ reason: "merged", runId: "run_a", state: "terminal" }]));
    await act(async () => { jest.advanceTimersByTime(QUEUE_POLL_MS); });
    expect(view.notice).not.toBeNull();

    await act(async () => { jest.advanceTimersByTime(QUEUE_NOTICE_MS); });
    expect(view.notice).toBeNull();
  });
});
