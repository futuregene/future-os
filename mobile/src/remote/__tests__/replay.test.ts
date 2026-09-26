import { fetchEventsSince, tailCoversRange, type EventsPage } from "../replay";
import type { RemoteClient } from "../client";
import type { ReplayEventWire } from "../timeline";

/** Build a fake client that returns a scripted sequence of pages. */
function clientReturning(pages: EventsPage[]): { client: RemoteClient; request: jest.Mock } {
  const request = jest.fn();
  for (const page of pages) request.mockResolvedValueOnce({ data: page });
  return { client: { requestRetry: request } as unknown as RemoteClient, request };
}

function event(type: string, idx: number): ReplayEventWire {
  return { type, idx };
}

describe("fetchEventsSince", () => {
  test("merges a single page and returns its events", async () => {
    const { client, request } = clientReturning([
      { events: [event("agent_start", 0), event("agent_end", 1)] },
    ]);
    const result = await fetchEventsSince(client, "s1", "r1", 0);
    expect(result.events).toEqual([event("agent_start", 0), event("agent_end", 1)]);
    expect(result.projection).toBeUndefined();
    expect(result.truncated).toBeUndefined();
    expect(request).toHaveBeenCalledTimes(1);
    expect(request).toHaveBeenCalledWith(
      { type: "get_events_since", sessionId: "s1", runId: "r1", sinceIdx: 0, limit: 1000, offset: 0, chunkedRead: true },
      "s1",
    );
  });

  test("loops paginated pages until hasMore is false", async () => {
    const { client, request } = clientReturning([
      { events: [event("a", 0)], hasMore: true, nextOffset: 1 },
      { events: [event("b", 1)], hasMore: true, nextOffset: 2 },
      { events: [event("c", 2)] },
    ]);
    const result = await fetchEventsSince(client, "s1", "r1", 0);
    expect((result.events ?? []).map((e) => e.idx)).toEqual([0, 1, 2]);
    expect(request).toHaveBeenCalledTimes(3);
    expect(request.mock.calls[1][0]).toMatchObject({ offset: 1 });
    expect(request.mock.calls[2][0]).toMatchObject({ offset: 2 });
  });

  test("carries the first page's projection through unchanged", async () => {
    const projection = { run_id: "r1", cursor: 3, events: [event("agent_end", 3)] };
    const { client } = clientReturning([{ events: [], projection }]);
    const result = await fetchEventsSince(client, "s1", "r1", 0);
    expect(result.projection).toBe(projection);
  });

  test("marks the envelope truncated when any page is truncated", async () => {
    const { client } = clientReturning([
      { events: [event("a", 0)], hasMore: true, nextOffset: 1, truncated: true },
      { events: [event("b", 1)] },
    ]);
    const result = await fetchEventsSince(client, "s1", "r1", 0);
    expect(result.truncated).toBe(true);
  });

  test("stops looping when nextOffset is not a number", async () => {
    const { client, request } = clientReturning([{ events: [event("a", 0)], hasMore: true }]);
    const result = await fetchEventsSince(client, "s1", "r1", 0);
    expect(result.events).toHaveLength(1);
    expect(request).toHaveBeenCalledTimes(1);
  });

  test("stops looping when nextOffset does not advance", async () => {
    const { client, request } = clientReturning([
      { events: [event("a", 0)], hasMore: true, nextOffset: 0 },
    ]);
    const result = await fetchEventsSince(client, "s1", "r1", 0);
    expect(result.events).toHaveLength(1);
    expect(request).toHaveBeenCalledTimes(1);
  });

  test("handles a page with no events array", async () => {
    const { client, request } = clientReturning([{}]);
    const result = await fetchEventsSince(client, "s1", "r1", 0);
    expect(result.events).toEqual([]);
    expect(request).toHaveBeenCalledTimes(1);
  });
});

test("cold bootstrap restores folded prefix and reads only the fixed-window suffix", async () => {
  const snapshot = { runId: "r", cursor: 1000, events: [event("agent_start", 0), event("text_chunk", 1000)] };
  const { client, request } = clientReturning([
    { runSnapshot: true, projection: snapshot, events: [], watermark: 1000 },
    { events: [event("text_chunk", 1001)], watermark: 1002, nextSinceIdx: 1001, hasMore: true },
    { events: [event("agent_end", 1002)], watermark: 1002 },
  ]);
  const result = await fetchEventsSince(client, "s", "r", -1);
  expect(request.mock.calls[0][0]).toMatchObject({ preferSnapshot: true, sinceIdx: -1 });
  expect(request.mock.calls[1][0]).toMatchObject({ sinceIdx: 1000, offset: 0 });
  expect(request.mock.calls[1][0]).not.toHaveProperty("preferSnapshot");
  expect(request.mock.calls[1][0]).not.toHaveProperty("replayUntilIdx");
  expect(request.mock.calls[2][0]).toMatchObject({ sinceIdx: 1001, replayUntilIdx: 1002 });
  expect(result).toEqual({ events: [], watermark: 1002, projection: {
    ...snapshot, cursor: 1002, events: [...snapshot.events, event("text_chunk", 1001), event("agent_end", 1002)],
  } });
  expect(snapshot.cursor).toBe(1000); // never mutate the captured response
});

test("empty snapshot tail keeps the snapshot boundary, and warm reads never ask for a snapshot", async () => {
  const snapshot = { runId: "r", cursor: 1000, events: [event("agent_start", 0), event("text_chunk", 1000)] };
  const { client, request } = clientReturning([
    { runSnapshot: true, projection: snapshot, events: [], watermark: 1000 },
    { events: [], watermark: 1000 },
    { events: [], watermark: 1000 },
  ]);
  const result = await fetchEventsSince(client, "s", "r", -1);
  expect(result.projection).toEqual(snapshot);
  expect(result.watermark).toBe(1000);
  expect(await fetchEventsSince(client, "s", "r", 1000)).toEqual({ events: [], watermark: 1000 });
  expect(request.mock.calls[2][0]).not.toHaveProperty("preferSnapshot");
});

test.each([
  { runId: "other", cursor: 1, events: [event("agent_start", 0)] },
  { runId: "r", cursor: 2, events: [event("agent_start", 0)] },
  { runId: "r", cursor: 1, events: [event("agent_start", 2)] },
  { runId: "r", cursor: 1, events: [event("agent_start", 1), event("text_chunk", 0)] },
  { runId: "r", cursor: 1, events: [{ ...event("agent_start", 0), runId: "other" }] },
  { runId: "r", cursor: 1, events: [] },
])("invalid snapshot identity/boundaries never seed a tail", async projection => {
  const { client, request } = clientReturning([{ runSnapshot: true, projection, watermark: 1 }]);
  await expect(fetchEventsSince(client, "s", "r", -1)).rejects.toThrow("replay_projection_invalid");
  expect(request).toHaveBeenCalledTimes(1);
});

test.each([
  { events: [event("text_chunk", 3)], watermark: 3 },
  { events: [], watermark: 0 },
  { events: [event("text_chunk", 2)] },
  { events: [{ ...event("text_chunk", 2), runId: "other" }], watermark: 2 },
  { events: [event("text_chunk", 2)], watermark: 2, truncated: true },
])("invalid tail rejects the entire snapshot transaction", async tail => {
  const { client } = clientReturning([
    { runSnapshot: true, projection: { runId: "r", cursor: 1, events: [event("agent_start", 0)] }, watermark: 1 }, tail,
  ]);
  await expect(fetchEventsSince(client, "s", "r", -1)).rejects.toThrow("replay_prefix_invalid");
});

test("new Desktop replay advances event cursors while pinning the first page watermark", async () => {
  const { client, request } = clientReturning([
    { events: [event("a", 4)], hasMore: true, nextSinceIdx: 4, watermark: 9 },
    { events: [event("b", 9)], hasMore: false, watermark: 9 },
  ]);
  const result = await fetchEventsSince(client, "s", "r", -1);
  expect(result.events?.map((e) => e.idx)).toEqual([4, 9]);
  expect(request.mock.calls[1][0]).toMatchObject({ sinceIdx: 4, offset: 0, replayUntilIdx: 9 });
});

test("a hidden or replaced lane stops after the in-flight page instead of draining the tail", async () => {
  let visible = true;
  let finish!: (value: { data: EventsPage }) => void;
  const request = jest.fn(() => new Promise<{ data: EventsPage }>(resolve => { finish = resolve; }));
  const client = { requestRetry: request } as unknown as RemoteClient;
  const loading = fetchEventsSince(client, "s", "r", -1, () => visible);
  const rejected = expect(loading).rejects.toThrow("stale_sync_lane");
  visible = false;
  finish({ data: { events: [event("a", 99)], hasMore: true, nextSinceIdx: 99, watermark: 9999 } });
  await rejected;
  expect(request).toHaveBeenCalledTimes(1);
});

test("a lane that goes stale between the page read and its merge stops the replay", async () => {
  // The page itself was read while the lane was still current. That is the
  // reader's own three checks (the loop's pre-check, then the two around the
  // request); the merge is the last gate before those events reach the
  // timeline, so a lane that moved on in between must not be written to.
  let checks = 0;
  const { client, request } = clientReturning([
    { events: [event("a", 4)], hasMore: false, watermark: 4 },
  ]);
  await expect(fetchEventsSince(client, "s", "r", -1, () => ++checks <= 3))
    .rejects.toThrow("stale_sync_lane");
  expect(request).toHaveBeenCalledTimes(1);
});

test("a tail page that repeats a snapshot after it was pinned is refused", async () => {
  const snapshot = { runId: "r", cursor: 1000, events: [event("agent_start", 0), event("text_chunk", 1000)] };
  const { client } = clientReturning([
    { runSnapshot: true, projection: snapshot, events: [], watermark: 1000 },
    {
      projection: { runId: "r", cursor: 1001, events: [event("agent_end", 1001)] },
      events: [event("agent_end", 1001)],
      watermark: 1001,
    },
  ]);
  // The snapshot boundary was already pinned; appending a differently-shaped
  // projection would silently reinterpret history the caller already folded.
  await expect(fetchEventsSince(client, "s", "r", -1)).rejects.toThrow("replay_snapshot_tail_replaced");
});

test("single and empty replay pages retain their watermark for incremental integrity checks", async () => {
  const { client } = clientReturning([
    { events: [event("a", 4)], hasMore: false, watermark: 4 },
    { events: [], hasMore: false, watermark: 4 },
  ]);
  expect((await fetchEventsSince(client, "s", "r", 3)).watermark).toBe(4);
  expect(await fetchEventsSince(client, "s", "r", 4)).toEqual({ events: [], watermark: 4 });
});

test("an already obsolete replay does not issue any request", async () => {
  const { client, request } = clientReturning([]);
  await expect(fetchEventsSince(client, "s", "r", -1, () => false)).rejects.toThrow("stale_sync_lane");
  expect(request).not.toHaveBeenCalled();
});

test("a changed replay window rejects the partial result so sync can retry from durable state", async () => {
  const { client } = clientReturning([
    { events: [event("a", 4)], hasMore: true, nextSinceIdx: 4, watermark: 9 },
    { events: [event("b", 10)], hasMore: false, watermark: 10 },
  ]);
  await expect(fetchEventsSince(client, "s", "r", -1)).rejects.toThrow("replay_window_changed");
});

test("a cursor that does not advance rejects the page instead of looping forever", async () => {
  // The desktop says there is more, but hands back the cursor it was given:
  // taking it would re-request the same page forever. Reject so the caller
  // retries from durable state rather than spinning.
  const { client, request } = clientReturning([
    { events: [event("a", 5)], hasMore: true, nextSinceIdx: 5, watermark: 5 },
  ]);
  await expect(fetchEventsSince(client, "s", "r", 5)).rejects.toThrow("replay_cursor_stalled");
  expect(request).toHaveBeenCalledTimes(1);
});

describe("tailCoversRange", () => {
  test("a peer that trims nothing must be one event per index", () => {
    expect(tailCoversRange([event("a", 1), event("b", 2)], 0, 2, undefined)).toBe(true);
    // A hole with no statement of the raw count is a failed read, not a trim.
    expect(tailCoversRange([event("a", 1), event("b", 3)], 0, 2, undefined)).toBe(false);
    expect(tailCoversRange([event("a", 1), event("b", 3)], 0, 3, undefined)).toBe(false);
    // …and a short tail that does not reach the watermark is still a failure.
    expect(tailCoversRange([event("a", 1)], 0, 2, undefined)).toBe(false);
  });

  test("a stated raw count keeps the watermark check exact across holes", () => {
    // The range 1..5 held five events; two survived (the rest are trimmed).
    expect(tailCoversRange([event("a", 1), event("e", 5)], 0, 5, 5)).toBe(true);
    // A count that does not reach the pinned watermark is still rejected.
    expect(tailCoversRange([event("a", 1), event("e", 5)], 0, 6, 5)).toBe(false);
    // As is a reordered or duplicated survivor.
    expect(tailCoversRange([event("a", 5), event("b", 5)], 0, 5, 5)).toBe(false);
    expect(tailCoversRange([event("a", 3), event("b", 2)], 0, 5, 5)).toBe(false);
    // And an event past the window.
    expect(tailCoversRange([event("a", 6)], 0, 5, 5)).toBe(false);
  });
});

test("a trimmed snapshot tail is accepted on its raw count, and rejected without one", async () => {
  const snapshot = { runId: "r", cursor: 1000, events: [event("agent_start", 0)] };
  const trimmedTail: EventsPage = {
    // The trim dropped idx 1001-1004 (reasoning and argument deltas) and left
    // the terminal event. Before the fix this failed `watermark === boundary +
    // events.length` every time, so every reconcile retried forever and the
    // phone showed a sync notice it could never clear.
    events: [event("agent_end", 1005)],
    watermark: 1005,
    rawEvents: 5,
  };
  const { client } = clientReturning([
    { runSnapshot: true, projection: snapshot, events: [], watermark: 1000 },
    trimmedTail,
  ]);
  const result = await fetchEventsSince(client, "s", "r", -1);
  expect(result.projection?.cursor).toBe(1005);
  expect(result.projection?.events).toEqual([event("agent_start", 0), event("agent_end", 1005)]);

  const { client: strict } = clientReturning([
    { runSnapshot: true, projection: snapshot, events: [], watermark: 1000 },
    { ...trimmedTail, rawEvents: undefined },
  ]);
  await expect(fetchEventsSince(strict, "s", "r", -1)).rejects.toThrow("replay_prefix_invalid");
});
