import { SyncEngine } from "../syncEngine";
import { emptyTimeline, type TimelineState } from "../timeline";

function history(text: string): TimelineState {
  return { ...emptyTimeline(), items: [{ kind: "message", id: "u", role: "user", text }] };
}

beforeEach(() => jest.useFakeTimers());
afterEach(() => jest.useRealTimers());

test("one opening state request feeds controls and history without another network round trip", async () => {
  const state = { model: "provider/model" };
  const requestGetState = jest.fn(async () => {
    await new Promise(resolve => setTimeout(resolve, 350));
    return state;
  });
  const requestHistory = jest.fn(async () => {
    await new Promise(resolve => setTimeout(resolve, 350));
    return history("ready");
  });
  const engine = new SyncEngine({ requestGetState, requestHistory, fetchReplay: jest.fn() });
  const startedAt = Date.now();
  const commits: number[] = [];
  engine.subscribe(() => commits.push(Date.now() - startedAt));
  try {
    const opening = engine.open("s");
    await jest.advanceTimersByTimeAsync(350);
    await expect(opening).resolves.toEqual(state);
    expect(requestGetState).toHaveBeenCalledTimes(1);
    expect(requestHistory).toHaveBeenCalledTimes(1);
    await jest.advanceTimersByTimeAsync(350);
    expect(commits).toEqual([700]);
    expect(requestGetState).toHaveBeenCalledTimes(1);
    expect(requestHistory).toHaveBeenCalledTimes(1);
  } finally { engine.clear(); }
});

test("live deltas during an open do not enqueue a duplicate opening read", async () => {
  const requestGetState = jest.fn(async () => ({ activeRun: { runId: "r" } }));
  const requestHistory = jest.fn(async () => {
    await new Promise(resolve => setTimeout(resolve, 200));
    return history("prompt");
  });
  const engine = new SyncEngine({ requestGetState, requestHistory, fetchReplay: async () => ({ events: [
    { type: "agent_start", idx: 0, runId: "r", data: "{}" },
    { type: "text_chunk", idx: 1, runId: "r", data: '{"text":"hello"}' },
  ] }) });
  try {
    await engine.open("s");
    await jest.advanceTimersByTimeAsync(0);
    for (let i = 0; i < 100; i++) engine.event("s", { type: "text_chunk", idx: 1, runId: "r", data: '{"text":"hello"}' });
    await jest.advanceTimersByTimeAsync(1000);
    expect(requestGetState).toHaveBeenCalledTimes(1);
    expect(requestHistory).toHaveBeenCalledTimes(1);
  } finally { engine.clear(); }
});

test("ten thousand hidden deltas trigger no history, replay or timeline allocations", async () => {
  let visible = "front";
  const requestGetState = jest.fn(async () => ({}));
  const requestHistory = jest.fn(async () => history("latest"));
  const fetchReplay = jest.fn();
  const engine = new SyncEngine({
    isSessionVisible: sid => sid === visible, requestGetState, requestHistory, fetchReplay,
  });
  try {
    for (let session = 0; session < 100; session++) {
      for (let idx = 0; idx < 100; idx++) {
        engine.event(`hidden-${session}`, { type: "text_chunk", data: '{"text":"x"}', runId: "r", idx });
      }
      engine.reconcile(`hidden-${session}`, "resend");
    }
    engine.restartAll("reconnect");
    await jest.advanceTimersByTimeAsync(20);
    expect(requestGetState).not.toHaveBeenCalled();
    expect(requestHistory).not.toHaveBeenCalled();
    expect(fetchReplay).not.toHaveBeenCalled();
    for (let session = 0; session < 100; session++) expect(engine.timelineFor(`hidden-${session}`)).toBeNull();

    visible = "hidden-3";
    await engine.open(visible);
    await jest.advanceTimersByTimeAsync(20);
    expect(requestGetState).toHaveBeenCalledTimes(1);
    expect(requestHistory).toHaveBeenCalledTimes(1);
    expect(engine.timelineFor(visible)?.items).toEqual(history("latest").items);
  } finally { engine.clear(); }
});

test("reconnect only restarts the visible lane and reopening refreshes an idle cached session", async () => {
  let visible = "a";
  let text = "old";
  const requestGetState = jest.fn(async () => ({}));
  const requestHistory = jest.fn(async () => history(text));
  const engine = new SyncEngine({
    isSessionVisible: sid => sid === visible, requestGetState, requestHistory, fetchReplay: jest.fn(),
  });
  try {
    await engine.open("a");
    await jest.advanceTimersByTimeAsync(0);
    visible = "b";
    await engine.open("b");
    await jest.advanceTimersByTimeAsync(0);
    requestGetState.mockClear();
    requestHistory.mockClear();
    engine.restartAll("reconnect");
    await jest.advanceTimersByTimeAsync(0);
    expect(requestGetState.mock.calls).toEqual([["b"]]);
    expect(requestHistory.mock.calls).toEqual([["b"]]);
    text = "completed while hidden";
    visible = "a";
    await engine.open("a");
    await jest.advanceTimersByTimeAsync(0);
    expect(engine.timelineFor("a")?.items).toEqual(history(text).items);
  } finally { engine.clear(); }
});

test("late history from a hidden lane cannot overwrite a fresh reopen", async () => {
  let visible = "a";
  let finishOld!: (value: TimelineState) => void;
  const oldHistory = new Promise<TimelineState>(resolve => { finishOld = resolve; });
  const requestHistory = jest.fn(async () => history("fresh")).mockReturnValueOnce(oldHistory);
  const engine = new SyncEngine({
    isSessionVisible: sid => sid === visible,
    requestGetState: async () => ({}), requestHistory, fetchReplay: jest.fn(),
  });
  try {
    await engine.open("a");
    await jest.advanceTimersByTimeAsync(0);
    visible = "b";
    await engine.open("b");
    await jest.advanceTimersByTimeAsync(0);
    visible = "a";
    await engine.open("a");
    await jest.advanceTimersByTimeAsync(0);
    finishOld(history("obsolete"));
    await jest.advanceTimersByTimeAsync(0);
    expect(engine.timelineFor("a")?.items).toEqual(history("fresh").items);
    expect(engine.timelineFor("b")?.items).toEqual(history("fresh").items);
  } finally { engine.clear(); }
});

test("an opening request failure retries with fresh state instead of a rejected promise", async () => {
  const requestGetState = jest.fn(async () => ({})).mockRejectedValueOnce(new Error("offline"));
  const onFailure = jest.fn();
  const engine = new SyncEngine({
    requestGetState, requestHistory: async () => history("recovered"), fetchReplay: jest.fn(), onFailure,
  });
  try {
    await expect(engine.open("s")).rejects.toThrow("offline");
    await jest.advanceTimersByTimeAsync(501);
    expect(onFailure).toHaveBeenCalledTimes(1);
    expect(requestGetState).toHaveBeenCalledTimes(2);
    expect(engine.timelineFor("s")?.items).toEqual(history("recovered").items);
  } finally { engine.clear(); }
});

test("opening a background session restores its pending approval from durable replay", async () => {
  let visible = "front";
  const fetchReplay = jest.fn(async () => ({ events: [
    { type: "agent_start", runId: "r", idx: 0, data: "{}" },
    { type: "approval_request", runId: "r", idx: 1, data: JSON.stringify({ approval_request_id: "approval-1", tool_name: "shell" }) },
  ] }));
  const engine = new SyncEngine({
    isSessionVisible: sid => sid === visible,
    requestGetState: async () => ({ activeRun: { runId: "r" } }),
    requestHistory: async () => history("prompt"), fetchReplay,
  });
  try {
    engine.event("background", { type: "approval_request", runId: "r", idx: 1, data: "{}" });
    expect(fetchReplay).not.toHaveBeenCalled();
    visible = "background";
    await engine.open(visible);
    await jest.advanceTimersByTimeAsync(0);
    expect(fetchReplay).toHaveBeenCalledWith("background", "r", -1, expect.any(Function));
    expect(engine.timelineFor(visible)?.items.find(item => item.kind === "approval"))
      .toMatchObject({ payload: { approval_request_id: "approval-1" } });
  } finally { engine.clear(); }
});
