import { retainedSize } from "../retainedSize";
import { SyncEngine } from "../syncEngine";
import { emptyTimeline } from "../timeline";
import { createRunProjector } from "@future-os/thread-projection";

test("Map/Set payloads are counted without serializing or following object cycles", () => {
  const graph = { set: new Set(["x".repeat(100000)]), map: new Map([["id", "y".repeat(100000)]]) };
  expect(retainedSize(graph)).toBeGreaterThan(400000);
  const cycle: { self?: unknown } = {};
  cycle.self = cycle;
  expect(retainedSize(cycle)).toBeLessThan(1000);
});

test("cache eviction includes closure-owned projector text even with an empty display snapshot", async () => {
  const projector = createRunProjector();
  projector.append({ id: "e", runId: "r", eventType: "text_chunk", sequence: 0, createdAt: 0,
    payload: JSON.stringify({ text: "x".repeat(100000) }) });
  expect(projector.estimatedBytes()).toBeGreaterThan(200000);
  expect(projector.fork().estimatedBytes()).toBe(projector.estimatedBytes());
  const engine = new SyncEngine({ requestGetState: async () => ({}), requestHistory: async () => emptyTimeline(), fetchReplay: async () => ({ events: [] }) });
  try {
    await new Promise<void>(resolve => {
      const unsubscribe = engine.subscribe(() => { unsubscribe(); resolve(); });
      engine.mutate("cold", () => ({ ...emptyTimeline(), liveRuns: new Map([["r", {
        projector, assistantId: "a", startedAt: 0, streaming: true, failed: false,
      }]]) }));
    });
    expect(engine.pruneCache("hot", 8, 64 * 1024)).toEqual(["cold"]);
  } finally { engine.clear(); }
});
