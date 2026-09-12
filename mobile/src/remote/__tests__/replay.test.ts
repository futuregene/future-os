import { fetchEventsSince, type EventsPage } from "../replay";
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
      { type: "get_events_since", sessionId: "s1", runId: "r1", sinceIdx: 0, offset: 0 },
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
