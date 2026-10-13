import { describe, expect, test } from "vitest";
import type { ReplayEvent, ReplayPage } from "./replay";
import { readRunReplay, tailCoversRange } from "./replay";

/**
 * The read lane's wire contract.
 *
 * A remote client walks a run's pages, and every rule here is a fact about the
 * host's replies rather than about either client — which is why they moved out of
 * the phone's suite when the desktop started reading the same feed. The cases are
 * the ones that were learned the hard way: a snapshot that lies about its
 * boundary, a tail that does not reach its watermark, a window that moves under
 * the read, a cursor that would loop forever.
 */

/** A reader over a scripted sequence of pages. */
function pages(pages: ReplayPage[]): { readPage: (command: any) => Promise<ReplayPage>; calls: any[] } {
  const calls: any[] = [];
  let index = 0;
  return {
    calls,
    readPage: async (command) => {
      calls.push(command);
      const page = pages[index];
      index += 1;
      return page ?? {};
    },
  };
}

/** A wire event as the agent's JSON payloads spell it: no `data`, no run id. */
function wire(type: string, idx: number): { type: string; idx: number } {
  return { type, idx };
}

/** The normalized form of one of those wire events. */
function event(type: string, idx: number): ReplayEvent {
  return { type, data: "", runId: "", idx };
}

function read(
  reader: (command: any) => Promise<ReplayPage>,
  sinceIdx: number,
  isCurrent: () => boolean = () => true,
) {
  return readRunReplay({ readPage: reader, sessionId: "s1", runId: "r1", sinceIdx, isCurrent });
}

describe("readRunReplay", () => {
  test("merges a single page and returns its events, normalized", async () => {
    const { readPage, calls } = pages([
      { events: [wire("agent_start", 0), wire("agent_end", 1)] },
    ]);
    const result = await read(readPage, 0);
    expect(result.events).toEqual([event("agent_start", 0), event("agent_end", 1)]);
    expect(result.projection).toBeUndefined();
    expect(result.truncated).toBeUndefined();
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({
      type: "get_events_since",
      sessionId: "s1",
      runId: "r1",
      sinceIdx: 0,
      limit: 1000,
      offset: 0,
    });
    // A tail read never asks for a snapshot: the host folds "the run from its
    // beginning", and that is not what this call is.
    expect(calls[0]).not.toHaveProperty("preferSnapshot");
  });

  test("loops paginated pages until hasMore is false", async () => {
    const { readPage, calls } = pages([
      { events: [wire("a", 0)], hasMore: true, nextOffset: 1 },
      { events: [wire("b", 1)], hasMore: true, nextOffset: 2 },
      { events: [wire("c", 2)] },
    ]);
    const result = await read(readPage, 0);
    expect(result.events.map((item) => item.idx)).toEqual([0, 1, 2]);
    expect(calls).toHaveLength(3);
    expect(calls[1]).toMatchObject({ offset: 1 });
    expect(calls[2]).toMatchObject({ offset: 2 });
  });

  test("stops looping when nextOffset is not a number", async () => {
    const { readPage, calls } = pages([{ events: [wire("a", 0)], hasMore: true }]);
    const result = await read(readPage, 0);
    expect(result.events).toHaveLength(1);
    expect(calls).toHaveLength(1);
  });

  test("stops looping when nextOffset does not advance", async () => {
    const { readPage, calls } = pages([{ events: [wire("a", 0)], hasMore: true, nextOffset: 0 }]);
    const result = await read(readPage, 0);
    expect(result.events).toHaveLength(1);
    expect(calls).toHaveLength(1);
  });

  test("handles a page with no events array", async () => {
    const { readPage, calls } = pages([{}]);
    const result = await read(readPage, 0);
    expect(result.events).toEqual([]);
    expect(calls).toHaveLength(1);
  });

  test("carries a legacy projection reply through unchanged", async () => {
    const projection = { runId: "r1", cursor: 3, events: [wire("agent_end", 3)] };
    const { readPage } = pages([{ events: [], projection }]);
    const result = await read(readPage, 0);
    expect(result.projection).toEqual({ ...projection, events: [event("agent_end", 3)] });
    expect(result.events).toEqual([]);
  });

  test("marks the envelope truncated when any page is truncated", async () => {
    const { readPage } = pages([
      { events: [wire("a", 0)], hasMore: true, nextOffset: 1, truncated: true },
      { events: [wire("b", 1)] },
    ]);
    const result = await read(readPage, 0);
    expect(result.truncated).toBe(true);
  });

  test("a whole-run read whose events are contiguous establishes the prefix", async () => {
    const { readPage } = pages([{ events: [wire("agent_start", 0), wire("agent_end", 1)], watermark: 1 }]);
    const result = await read(readPage, -1);
    expect(result.completePrefix).toBe(true);
  });

  test("a tail read never claims the prefix, however complete it looks", async () => {
    // The events are contiguous from the requested index, but this client never
    // asked for the run's beginning: what came before `since` is unproven.
    const { readPage } = pages([{ events: [wire("text_chunk", 5), wire("agent_end", 6)], watermark: 6 }]);
    const result = await read(readPage, 4);
    expect(result.completePrefix).toBe(false);
  });

  test("a page that names another run is refused", async () => {
    const { readPage } = pages([
      { events: [{ type: "text_chunk", idx: 1, runId: "r2" }], watermark: 1 },
    ]);
    await expect(read(readPage, 0)).rejects.toThrow("replay_run_changed");
  });

  test("cold bootstrap restores the folded prefix and reads only the fixed-window suffix", async () => {
    const snapshot = { runId: "r1", cursor: 1000, events: [wire("agent_start", 0), wire("text_chunk", 1000)] };
    const { readPage, calls } = pages([
      { runSnapshot: true, projection: snapshot, events: [], watermark: 1000 },
      { events: [wire("text_chunk", 1001)], watermark: 1002, nextSinceIdx: 1001, hasMore: true },
      { events: [wire("agent_end", 1002)], watermark: 1002 },
    ]);
    const result = await read(readPage, -1);
    expect(calls[0]).toMatchObject({ preferSnapshot: true, sinceIdx: -1 });
    expect(calls[1]).toMatchObject({ sinceIdx: 1000, offset: 0 });
    expect(calls[1]).not.toHaveProperty("preferSnapshot");
    // The snapshot is immutable, so the tail's watermark is pinned *after* it —
    // otherwise the tail would be read against a window that has already moved.
    expect(calls[1]).not.toHaveProperty("replayUntilIdx");
    expect(calls[2]).toMatchObject({ sinceIdx: 1001, replayUntilIdx: 1002 });
    expect(result).toEqual({
      events: [],
      watermark: 1002,
      projection: {
        ...snapshot,
        cursor: 1002,
        events: [
          event("agent_start", 0),
          event("text_chunk", 1000),
          event("text_chunk", 1001),
          event("agent_end", 1002),
        ],
      },
      completePrefix: true,
    });
    expect(snapshot.cursor).toBe(1000); // never mutate the captured response
  });

  test("an empty snapshot tail keeps the snapshot boundary, and warm reads never ask for a snapshot", async () => {
    const snapshot = { runId: "r1", cursor: 1000, events: [wire("agent_start", 0), wire("text_chunk", 1000)] };
    const { readPage, calls } = pages([
      { runSnapshot: true, projection: snapshot, events: [], watermark: 1000 },
      { events: [], watermark: 1000 },
      { events: [], watermark: 1000 },
    ]);
    const result = await read(readPage, -1);
    expect(result.projection).toEqual({ ...snapshot, events: [event("agent_start", 0), event("text_chunk", 1000)] });
    expect(result.watermark).toBe(1000);
    const warm = await read(readPage, 1000);
    expect(warm.events).toEqual([]);
    expect(warm.watermark).toBe(1000);
    expect(calls[2]).not.toHaveProperty("preferSnapshot");
  });

  test.each([
    { runId: "other", cursor: 1, events: [wire("agent_start", 0)] },
    { runId: "r1", cursor: 2, events: [wire("agent_start", 0)] },
    { runId: "r1", cursor: 1, events: [wire("agent_start", 2)] },
    { runId: "r1", cursor: 1, events: [wire("agent_start", 1), wire("text_chunk", 0)] },
    { runId: "r1", cursor: 1, events: [{ type: "agent_start", idx: 0, runId: "other" }] },
    { runId: "r1", cursor: 1, events: [] },
  ])("invalid snapshot identity/boundaries never seed a tail", async (projection) => {
    const { readPage, calls } = pages([{ runSnapshot: true, projection, watermark: 1 }]);
    await expect(read(readPage, -1)).rejects.toThrow("replay_projection_invalid");
    expect(calls).toHaveLength(1);
  });

  test.each([
    { events: [wire("text_chunk", 3)], watermark: 3 },
    { events: [], watermark: 0 },
    { events: [wire("text_chunk", 2)] },
    { events: [{ type: "text_chunk", idx: 2, runId: "other" }], watermark: 2 },
    { events: [wire("text_chunk", 2)], watermark: 2, truncated: true },
  ])("invalid tail rejects the entire snapshot transaction", async (tail) => {
    const { readPage } = pages([
      { runSnapshot: true, projection: { runId: "r1", cursor: 1, events: [wire("agent_start", 0)] }, watermark: 1 },
      tail,
    ]);
    // A bootstrap is one transaction: a prefix with an unprovable tail is not
    // half-applied, it is refused.
    await expect(read(readPage, -1)).rejects.toMatchObject({
      message: expect.stringMatching(/replay_(prefix_invalid|run_changed)/),
    });
  });

  test("watermarked pages advance event cursors while pinning the first page watermark", async () => {
    const { readPage, calls } = pages([
      { events: [wire("a", 4)], hasMore: true, nextSinceIdx: 4, watermark: 9 },
      { events: [wire("b", 9)], hasMore: false, watermark: 9 },
    ]);
    const result = await read(readPage, -1);
    expect(result.events.map((item) => item.idx)).toEqual([4, 9]);
    expect(calls[1]).toMatchObject({ sinceIdx: 4, offset: 0, replayUntilIdx: 9 });
  });

  test("a hidden or replaced lane stops after the in-flight page instead of draining the tail", async () => {
    let visible = true;
    let finish!: (value: ReplayPage) => void;
    const calls: any[] = [];
    const readPage = (command: any) => {
      calls.push(command);
      return new Promise<ReplayPage>((resolve) => {
        finish = resolve;
      });
    };
    const loading = read(readPage, -1, () => visible);
    const rejected = expect(loading).rejects.toThrow("stale_sync_lane");
    visible = false;
    finish({ events: [wire("a", 99)], hasMore: true, nextSinceIdx: 99, watermark: 9999 });
    await rejected;
    expect(calls).toHaveLength(1);
  });

  test("a lane that goes stale between the page read and its merge stops the replay", async () => {
    // The page itself was read while the lane was still current — the reader owns
    // its own checks around the request (`requestReadPage` on the phone) — and
    // this loop re-checks before it merges, because that is the last gate before
    // those events reach a timeline.
    let checks = 0;
    const { readPage, calls } = pages([{ events: [wire("a", 4)], hasMore: false, watermark: 4 }]);
    await expect(read(readPage, -1, () => ++checks <= 1)).rejects.toThrow("stale_sync_lane");
    expect(calls).toHaveLength(1);
  });

  test("a tail page that repeats a snapshot after it was pinned is refused", async () => {
    const snapshot = { runId: "r1", cursor: 1000, events: [wire("agent_start", 0), wire("text_chunk", 1000)] };
    const { readPage } = pages([
      { runSnapshot: true, projection: snapshot, events: [], watermark: 1000 },
      {
        projection: { runId: "r1", cursor: 1001, events: [wire("agent_end", 1001)] },
        events: [wire("agent_end", 1001)],
        watermark: 1001,
      },
    ]);
    // The snapshot boundary was already pinned; a differently-shaped projection
    // arriving after it would silently reinterpret what was folded.
    await expect(read(readPage, -1)).rejects.toThrow("replay_snapshot_tail_replaced");
  });

  test("single and empty replay pages retain their watermark for incremental integrity checks", async () => {
    const { readPage } = pages([
      { events: [wire("a", 4)], hasMore: false, watermark: 4 },
      { events: [], hasMore: false, watermark: 4 },
    ]);
    expect((await read(readPage, 3)).watermark).toBe(4);
    const empty = await read(readPage, 4);
    expect(empty.events).toEqual([]);
    expect(empty.watermark).toBe(4);
  });

  test("an already obsolete replay does not issue any request", async () => {
    const { readPage, calls } = pages([]);
    await expect(read(readPage, -1, () => false)).rejects.toThrow("stale_sync_lane");
    expect(calls).toHaveLength(0);
  });

  test("a changed replay window rejects the partial result so sync can retry from durable state", async () => {
    const { readPage } = pages([
      { events: [wire("a", 4)], hasMore: true, nextSinceIdx: 4, watermark: 9 },
      { events: [wire("b", 10)], hasMore: false, watermark: 10 },
    ]);
    await expect(read(readPage, -1)).rejects.toThrow("replay_window_changed");
  });

  test("a cursor that does not advance rejects the page instead of looping forever", async () => {
    // The host says there is more, but hands back the cursor it was given: taking
    // it would re-request the same page forever.
    const { readPage, calls } = pages([
      { events: [wire("a", 5)], hasMore: true, nextSinceIdx: 5, watermark: 5 },
    ]);
    await expect(read(readPage, 5)).rejects.toThrow("replay_cursor_stalled");
    expect(calls).toHaveLength(1);
  });
});

describe("tailCoversRange", () => {
  test("a peer that trims nothing must be one event per index", () => {
    expect(tailCoversRange([wire("a", 1), wire("b", 2)], 0, 2, undefined)).toBe(true);
    // A hole with no statement of the raw count is a failed read, not a trim.
    expect(tailCoversRange([wire("a", 1), wire("b", 3)], 0, 2, undefined)).toBe(false);
  });

  test("a peer that states its pre-trim count is checked by count and order", () => {
    // Two arrived of the three the range held: the third was trimmed, and the
    // count is what proves the range still reached the watermark.
    expect(tailCoversRange([wire("a", 1), wire("b", 3)], 0, 3, 3)).toBe(true);
    // A count that does not reach the watermark is a short read.
    expect(tailCoversRange([wire("a", 1), wire("b", 3)], 0, 9, 3)).toBe(false);
    // Out-of-order events are refused however the count reads.
    expect(tailCoversRange([wire("a", 3), wire("b", 1)], 0, 3, 3)).toBe(false);
  });

  test("a range beyond the watermark is refused", () => {
    expect(tailCoversRange([wire("a", 1), wire("b", 4)], 0, 3, 2)).toBe(false);
  });

  test("an empty tail is judged by the peer's count, not by its emptiness", () => {
    expect(tailCoversRange([], 3, 3, 0)).toBe(true);
    expect(tailCoversRange([], 3, undefined, undefined)).toBe(true);
    // A range the peer says held one source event and delivered nothing of is a
    // complete read of a *trimmed* feed: the one event was a delta the client
    // asked not to be sent. Counting arrivals here would report a failed read on
    // every thinking-only page.
    expect(tailCoversRange([], 3, 4, 1)).toBe(true);
    // Without that statement, nothing arrived and nothing is claimed: one event
    // per index was expected and none came.
    expect(tailCoversRange([], 3, 4, undefined)).toBe(false);
  });

  test("an unnumbered feed cannot be verified by counting arrivals", () => {
    // `rawEvents` is the peer's statement; without it the events themselves must
    // line up one per index, and a feed that omits indices cannot be counted.
    expect(tailCoversRange([{ idx: undefined }], 0, 1, undefined)).toBe(false);
  });
});
