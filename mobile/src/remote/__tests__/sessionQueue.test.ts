import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import {
  cancelSessionQueuedRun,
  fetchSessionQueue,
  isParkedSessionQueueRow,
  QUEUE_POLL_MS,
  releaseSessionQueuedRun,
  useSessionQueue,
  type SessionQueueRow,
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
    const rows = await fetchSessionQueue(client, "session-1");
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
    expect(await fetchSessionQueue(clientFor({ queued: [{ text: "no id" }] }), "s")).toEqual([]);
    expect(await fetchSessionQueue(clientFor({}), "s")).toEqual([]);
    expect(await fetchSessionQueue(clientFor(null), "s")).toEqual([]);
  });

  test("a draft with no session asks nothing at all", async () => {
    const client = clientFor({ queued: [] });
    expect(await fetchSessionQueue(client, "  ")).toEqual([]);
    expect(client.requestRetry).not.toHaveBeenCalled();
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
    read: () => Promise<SessionQueueRow[]>;
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

  beforeEach(() => jest.useFakeTimers());
  afterEach(() => {
    const mounted = tree;
    if (mounted) act(() => mounted.unmount());
    tree = undefined;
    jest.useRealTimers();
  });

  const mount = async (read: () => Promise<SessionQueueRow[]>, sessionId = "s1", live = false) => {
    const element = createElement(Panel, { enabled: true, live, read, sessionId });
    await act(async () => {
      if (tree) tree.update(element);
      else tree = create(element);
    });
    await act(async () => { await Promise.resolve(); });
  };

  test("the rows are read once when the panel opens on an idle conversation", async () => {
    const read = jest.fn(async () => [row("run_a")]);
    await mount(read);
    expect(view.rows.map(item => item.runId)).toEqual(["run_a"]);
    expect(read).toHaveBeenCalledTimes(1);
    // Waiting rows keep it reading: one can start or be parked with no event.
    await act(async () => { jest.advanceTimersByTime(QUEUE_POLL_MS); });
    expect(read).toHaveBeenCalledTimes(2);
  });

  test("an empty queue on an idle conversation costs no timer", async () => {
    const read = jest.fn(async () => []);
    await mount(read);
    expect(read).toHaveBeenCalledTimes(1);
    await act(async () => { jest.advanceTimersByTime(QUEUE_POLL_MS * 2); });
    expect(read).toHaveBeenCalledTimes(1);
  });

  test("a live run keeps reading, and a refresh reads immediately", async () => {
    const read = jest.fn(async () => []);
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
    const read = jest.fn(async () => [row("run_a")]);
    await mount(read);
    read.mockRejectedValueOnce(new Error("not_connected"));
    await act(async () => { jest.advanceTimersByTime(QUEUE_POLL_MS); });
    expect(view.rows.map(item => item.runId)).toEqual(["run_a"]);
  });

  test("another conversation's rows are never handed to this one", async () => {
    const resolvers: ((rows: SessionQueueRow[]) => void)[] = [];
    const read = jest.fn(() => new Promise<SessionQueueRow[]>(yes => { resolvers.push(yes); }));
    await mount(read, "s1");
    // The reader moves on while the first conversation's read is still in flight.
    await act(async () => {
      tree!.update(createElement(Panel, { enabled: true, live: false, read, sessionId: "s2" }));
    });
    // The read that belonged to the conversation the reader left lands now.
    await act(async () => { resolvers[0]!([row("run_a")]); });
    await act(async () => { await Promise.resolve(); });
    expect(view.rows).toEqual([]);
  });
});
