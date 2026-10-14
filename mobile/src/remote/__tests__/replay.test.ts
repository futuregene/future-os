import type { RemoteClient } from "../client";
import type { EventsPage } from "../replay";
import { fetchEventsSince } from "../replay";

/**
 * The phone's read of a run's log — its own half of it.
 *
 * Walking a run's pages, validating a snapshot bootstrap and proving a tail
 * reaches its watermark all belong to the shared read
 * (`@future-os/thread-projection`: `readRunReplay`), and are tested there with
 * the desktop's client reading the same feed. What is the phone's is the
 * *transport*: the pages come back through `requestReadPage`, which asks to be
 * chunked and reassembles an oversized reply before the loop sees it. That wiring
 * is what this file asserts — one page at a time, with the cursor the loop hands
 * it, and the shared result coming back unchanged.
 */

/** Build a fake client that returns a scripted sequence of pages. */
function clientReturning(pages: EventsPage[]): { client: RemoteClient; request: jest.Mock } {
  const request = jest.fn();
  for (const page of pages) request.mockResolvedValueOnce({ data: page });
  return { client: { requestRetry: request } as unknown as RemoteClient, request };
}

describe("fetchEventsSince", () => {
  test("reads one page through the chunking request layer, and normalizes what it returns", async () => {
    const { client, request } = clientReturning([
      { events: [{ type: "agent_start", idx: 0, data: "{}" }] },
    ]);

    const result = await fetchEventsSince(client, "s1", "r1", 0);

    // The shared shape, with the wire's spellings resolved: a caller reads
    // `type`/`data`/`runId`/`idx`, never `run_id`.
    expect(result.events).toEqual([
      { type: "agent_start", data: "{}", runId: "", idx: 0 },
    ]);
    expect(result.projection).toBeUndefined();
    expect(result.completePrefix).toBe(false);
    expect(request).toHaveBeenCalledTimes(1);
    expect(request).toHaveBeenCalledWith(
      { type: "get_events_since", sessionId: "s1", runId: "r1", sinceIdx: 0, limit: 1000, offset: 0, chunkedRead: true },
      "s1",
    );
  });

  test("carries a snapshot bootstrap through as one whole-run projection", async () => {
    const { client, request } = clientReturning([
      {
        runSnapshot: true,
        events: [],
        watermark: 4,
        projection: {
          runId: "r1",
          cursor: 4,
          events: [
            { type: "agent_start", idx: 0, data: JSON.stringify({ started_at_ms: 1_700_000_000_000 }) },
            { type: "text_chunk", idx: 4, data: JSON.stringify({ text: "the reply" }) },
          ],
        },
      },
      { events: [{ type: "agent_end", idx: 5, data: "{}" }], watermark: 5 },
    ]);

    const result = await fetchEventsSince(client, "s1", "r1", -1);

    // The folded prefix and the tail that arrived while it was read are one
    // coherent log, and the raw deltas are not returned beside it (appending
    // them to folded text would draw the same words twice).
    expect(result.events).toEqual([]);
    expect(result.projection?.events.map(event => event.type))
      .toEqual(["agent_start", "text_chunk", "agent_end"]);
    expect(result.projection?.cursor).toBe(5);
    expect(result.completePrefix).toBe(true);
    // The bootstrap is asked for once, on the first page of a whole-run read.
    expect(request.mock.calls[0][0]).toMatchObject({ preferSnapshot: true, sinceIdx: -1 });
    expect(request.mock.calls[1][0]).not.toHaveProperty("preferSnapshot");
  });

  test("stops without asking when the lane it was read for is already gone", async () => {
    const { client, request } = clientReturning([]);
    await expect(fetchEventsSince(client, "s1", "r1", -1, () => false))
      .rejects.toThrow("stale_sync_lane");
    expect(request).not.toHaveBeenCalled();
  });
});
