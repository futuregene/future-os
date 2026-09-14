import { SyncEngine, type ReplayResult } from "../syncEngine";
import { timelineFromEntries } from "../timeline";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(yes => { resolve = yes; });
  return { promise, resolve };
}
const history = timelineFromEntries([
  { id: "u", kind: "user", role: "user", createdAtMs: 1, blocks: [{ kind: "text", text: "question" }] },
]);
const replay: ReplayResult = { events: [
  { type: "agent_start", runId: "r", idx: 0, data: "{}" },
  { type: "text_chunk", runId: "r", idx: 1, data: JSON.stringify({ text: "latest" }) },
] };

beforeEach(() => jest.useFakeTimers());
afterEach(() => jest.useRealTimers());

test("reports state/history/replay timings without treating a failed attempt as synced", async () => {
  const delay = <T,>(value: T, ms: number) => new Promise<T>(resolve => setTimeout(() => resolve(value), ms));
  const onTiming = jest.fn();
  let fail = true;
  const engine = new SyncEngine({
    requestGetState: () => delay({ activeRun: { runId: "r" } }, 20),
    requestHistory: () => delay(history, 30),
    fetchReplay: async () => {
      await delay(null, 100);
      if (fail) throw new Error("offline");
      return replay;
    },
    onTiming,
  });
  try {
    const opened = engine.open("s");
    await jest.advanceTimersByTimeAsync(150);
    await opened;
    expect(onTiming).toHaveBeenLastCalledWith({
      sessionId: "s", runId: "r", reason: "open", attempt: 1,
      elapsedMs: 150, stagesMs: { get_state: 20, history: 30, replay: 100 }, outcome: "failure",
    });
    fail = false;
    await jest.advanceTimersByTimeAsync(650);
    expect(onTiming).toHaveBeenLastCalledWith({
      sessionId: "s", runId: "r", reason: "open", attempt: 2,
      elapsedMs: 150, stagesMs: { get_state: 20, history: 30, replay: 100 }, outcome: "success",
    });
    expect(engine.streamingFor("s")).toBe(true);
  } finally { engine.clear(); }
});

test("readable initial history does not finish syncing while replay is pending", async () => {
  const pending = deferred<ReplayResult>();
  const onSyncStatus = jest.fn();
  const engine = new SyncEngine({
    requestGetState: async () => ({ activeRun: { runId: "r" } }),
    requestHistory: async () => history,
    fetchReplay: () => pending.promise,
    onSyncStatus,
  });
  try {
    await engine.open("s");
    await jest.advanceTimersByTimeAsync(0);
    expect(engine.timelineFor("s")?.items).toHaveLength(1);
    expect(onSyncStatus.mock.calls).toEqual([["s", "syncing"]]);
    pending.resolve(replay);
    await jest.advanceTimersByTimeAsync(0);
    expect(onSyncStatus).toHaveBeenLastCalledWith("s", "idle");
    // Caught up is different from the model finishing generation.
    expect(engine.streamingFor("s")).toBe(true);
  } finally { engine.clear(); }
});

test("warm opens retain content and stale replay cannot clear a newer sync indicator", async () => {
  const onSyncStatus = jest.fn();
  const fetchReplay = jest.fn(async () => replay);
  const engine = new SyncEngine({
    requestGetState: async () => ({ activeRun: { runId: "r" } }),
    requestHistory: async () => history,
    fetchReplay, onSyncStatus,
  });
  try {
    await engine.open("s");
    await jest.advanceTimersByTimeAsync(0);
    const cached = engine.timelineFor("s");
    const older = deferred<ReplayResult>();
    const newer = deferred<ReplayResult>();
    fetchReplay.mockReturnValueOnce(older.promise).mockReturnValueOnce(newer.promise);
    onSyncStatus.mockClear();
    await engine.open("s");
    await jest.advanceTimersByTimeAsync(0);
    expect(engine.timelineFor("s")).toBe(cached);
    await engine.open("s");
    await jest.advanceTimersByTimeAsync(0);
    older.resolve(replay);
    await jest.advanceTimersByTimeAsync(0);
    expect(onSyncStatus.mock.calls).toEqual([["s", "syncing"], ["s", "syncing"]]);
    newer.resolve(replay);
    await jest.advanceTimersByTimeAsync(0);
    expect(onSyncStatus).toHaveBeenLastCalledWith("s", "idle");
  } finally { engine.clear(); }
});

test("replay failures remain visibly retrying until automatic recovery succeeds", async () => {
  const onSyncStatus = jest.fn();
  const pending = deferred<ReplayResult>();
  const fetchReplay = jest.fn<Promise<ReplayResult>, []>()
    .mockRejectedValueOnce(new Error("offline"))
    .mockReturnValueOnce(pending.promise);
  const engine = new SyncEngine({
    requestGetState: async () => ({ activeRun: { runId: "r" } }),
    requestHistory: async () => history,
    fetchReplay, onSyncStatus,
  });
  try {
    await engine.open("s");
    await jest.advanceTimersByTimeAsync(0);
    expect(onSyncStatus).toHaveBeenLastCalledWith("s", "retrying");
    expect(engine.timelineFor("s")?.items).toHaveLength(1);
    await jest.advanceTimersByTimeAsync(500);
    expect(onSyncStatus).toHaveBeenLastCalledWith("s", "syncing");
    pending.resolve(replay);
    await jest.advanceTimersByTimeAsync(0);
    expect(onSyncStatus).toHaveBeenLastCalledWith("s", "idle");
  } finally { engine.clear(); }
});
