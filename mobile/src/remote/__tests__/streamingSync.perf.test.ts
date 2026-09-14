import type { RemoteClient } from "../client";
import { fetchEventsSince } from "../replay";
import { SyncEngine } from "../syncEngine";
import { emptyTimeline } from "../timeline";
import type { RemoteCommand } from "../types";

// Controlled network-latency benchmark, not device/UI timings. Exercise the
// production sync + pagination + projection code with the desktop's default
// 100-event / 512-KiB page contract. Run before and after pagination changes.
test.each([20, 150])("streaming open: 10,000 events at %i ms RPC latency", async latency => {
  jest.useFakeTimers();
  const events = [
    { type: "agent_start", runId: "r", idx: 0, data: "{}" },
    ...Array.from({ length: 9999 }, (_, i) => ({
      type: "text_chunk", runId: "r", idx: i + 1, data: JSON.stringify({ text: "token " }),
    })),
  ];
  const delay = <T,>(value: T) => new Promise<T>(resolve => setTimeout(() => resolve(value), latency));
  let replayRequests = 0;
  let replayEvents = 0;
  const requestRetry = jest.fn(async (command: RemoteCommand) => {
    replayRequests++;
    const since = command.sinceIdx ?? -1;
    const watermark = command.replayUntilIdx ?? events.length - 1;
    const page = [];
    let bytes = 0;
    for (const event of events.slice(since + 1, watermark + 1)) {
      const size = JSON.stringify(event).length;
      if (page.length && (page.length >= (command.limit || 100) || bytes + size > 512 * 1024)) break;
      page.push(event);
      bytes += size;
    }
    replayEvents += page.length;
    const nextSinceIdx = page.at(-1)?.idx ?? since;
    return delay({ success: true, data: { events: page, watermark, nextSinceIdx, hasMore: nextSinceIdx < watermark } });
  });
  const client = { requestRetry } as unknown as RemoteClient;
  let syncedAt = 0;
  let firstPaintAt = 0;
  let start = Date.now();
  const engine = new SyncEngine({
    requestGetState: () => delay({ activeRun: { runId: "r" } }),
    requestHistory: () => delay(emptyTimeline()),
    fetchReplay: async (session, run, since, current) => {
      const result = await fetchEventsSince(client, session, run, since, current);
      return { ...result, events: result.events ?? [] };
    },
    onSyncStatus: (_session, status) => { if (status === "idle") syncedAt = Date.now() - start; },
  });
  engine.subscribe(() => { firstPaintAt ||= Date.now() - start; });
  try {
    const opened = engine.open("s");
    await jest.runAllTimersAsync();
    await opened;
    expect(engine.timelineFor("s")?.items[0]).toMatchObject({ text: "token ".repeat(9999) });
    expect(engine.cursorFor("s").get("r")?.highWater).toBe(9999);
    expect(engine.streamingFor("s")).toBe(true);
    expect(syncedAt).toBeGreaterThan(0);
    expect(replayEvents).toBe(10_000);
    expect(replayRequests).toBe(10);
    expect(syncedAt).toBeLessThanOrEqual(latency * 12 + 20);
    console.warn(JSON.stringify({ benchmark: "streaming-open", rpcLatencyMs: latency,
      replayRequests, replayEvents, firstPaintMs: firstPaintAt, syncedMs: syncedAt }));

    events.push(...Array.from({ length: 200 }, (_, i) => ({
      type: "text_chunk", runId: "r", idx: 10_000 + i, data: JSON.stringify({ text: "tail " }),
    })));
    for (const mode of ["warm-incremental", "cold-full-comparison"]) {
      if (mode === "cold-full-comparison") engine.clear();
      replayRequests = 0;
      replayEvents = 0;
      syncedAt = 0;
      start = Date.now();
      const reopened = engine.open("s");
      await jest.runAllTimersAsync();
      await reopened;
      expect(engine.timelineFor("s")?.items[0]).toMatchObject({ text: "token ".repeat(9999) + "tail ".repeat(200) });
      expect(engine.cursorFor("s").get("r")?.highWater).toBe(10_199);
      expect(replayEvents).toBe(mode === "warm-incremental" ? 200 : 10_200);
      expect(replayRequests).toBe(mode === "warm-incremental" ? 1 : 11);
      expect(engine.streamingFor("s")).toBe(true);
      console.warn(JSON.stringify({ benchmark: mode, rpcLatencyMs: latency, replayRequests, replayEvents, syncedMs: syncedAt }));
    }
  } finally {
    engine.clear();
    jest.useRealTimers();
  }
});
