import { SyncEngine } from "../syncEngine";
import { emptyTimeline, type TimelineState } from "../timeline";

function history(text: string): TimelineState {
  return { ...emptyTimeline(), items: [{ kind: "message", id: "u", role: "user", text }] };
}

beforeEach(() => jest.useFakeTimers());
afterEach(() => jest.useRealTimers());

test("inactive timeline caches obey LRU count and byte budgets without truncating the selected session", async () => {
  const engine = new SyncEngine({ requestGetState: async () => ({}), requestHistory: async () => history("reloaded"), fetchReplay: jest.fn() });
  try {
    for (let i = 0; i < 12; i++) { await engine.open(`s${i}`); await jest.advanceTimersByTimeAsync(0); }
    expect(engine.pruneCache("s11")).toEqual(["s0", "s1", "s2", "s3"]);
    expect(engine.timelineFor("s0")).toBeNull();
    await engine.open("s4");
    await jest.advanceTimersByTimeAsync(0);
    await engine.open("s12");
    await jest.advanceTimersByTimeAsync(0);
    expect(engine.pruneCache("s12")).toEqual(["s5"]);
    expect(engine.timelineFor("s4")).not.toBeNull();
    engine.mutate("s12", () => history("x".repeat(10_000)));
    await jest.advanceTimersByTimeAsync(0);
    engine.pruneCache("s12", 8, 1000);
    expect(engine.timelineFor("s12")?.items).toEqual(history("x".repeat(10_000)).items);
    expect(engine.timelineFor("s4")).toBeNull();
  } finally { engine.clear(); }
});

test("evicting a pending lane fences its late history result", async () => {
  let finish!: (value: TimelineState) => void;
  const old = new Promise<TimelineState>(resolve => { finish = resolve; });
  const engine = new SyncEngine({ requestGetState: async () => ({}), requestHistory: async sid => sid === "old" ? old : history("current"), fetchReplay: jest.fn() });
  try {
    await engine.open("old");
    await jest.advanceTimersByTimeAsync(0);
    await engine.open("current");
    await jest.advanceTimersByTimeAsync(0);
    expect(engine.pruneCache("current", 1)).toEqual(["old"]);
    finish(history("late"));
    await jest.advanceTimersByTimeAsync(0);
    expect(engine.timelineFor("old")).toBeNull();
    expect(engine.timelineFor("current")?.items).toEqual(history("current").items);
  } finally { engine.clear(); }
});

test("a live burst yields to navigation and stops projecting after the session is hidden", async () => {
  let visible = "s";
  const engine = new SyncEngine({
    isSessionVisible: sid => sid === visible,
    requestGetState: async () => ({}), requestHistory: async () => history("prompt"), fetchReplay: jest.fn(),
  });
  const commits = jest.fn();
  engine.subscribe(commits);
  try {
    await engine.open("s");
    await jest.advanceTimersByTimeAsync(0);
    engine.event("s", { type: "agent_start", runId: "r", idx: 0, data: "{}" });
    await jest.advanceTimersByTimeAsync(16);
    commits.mockClear();
    for (let idx = 1; idx <= 512; idx++) {
      engine.event("s", { type: "text_chunk", runId: "r", idx, data: '{"text":"x"}' });
    }
    // Simulate a back event queued behind the first live frame. It must run
    // before the whole burst drains, without stopping the remote agent.
    let textAtBack = "";
    setTimeout(() => {
      textAtBack = engine.timelineFor("s")!.items
        .filter(item => item.kind === "message" && item.role === "assistant")
        .map(item => item.kind === "message" ? item.text : "").join("");
      visible = "";
    }, 16);
    await jest.advanceTimersByTimeAsync(16);
    expect(textAtBack.length).toBeGreaterThan(0);
    expect(textAtBack.length).toBeLessThanOrEqual(64);
    const countAtBack = commits.mock.calls.length;
    const timelineAtBack = engine.timelineFor("s");
    await jest.advanceTimersByTimeAsync(1000);
    expect(commits).toHaveBeenCalledTimes(countAtBack);
    expect(engine.timelineFor("s")).toBe(timelineAtBack);
  } finally { engine.clear(); }
});

test("yielded live batches retain event order, duplicate filtering, terminal events, and mutations", async () => {
  const engine = new SyncEngine({
    requestGetState: async () => ({}), requestHistory: async () => history("prompt"), fetchReplay: async () => ({ events: [] }),
  });
  try {
    await engine.open("s");
    await jest.advanceTimersByTimeAsync(0);
    engine.event("s", { type: "agent_start", runId: "r", idx: 0, data: "{}" });
    await jest.advanceTimersByTimeAsync(16);
    const parts = Array.from({ length: 200 }, (_, i) => `${i},`);
    parts.forEach((text, i) => {
      const event = { type: "text_chunk", runId: "r", idx: i + 1, data: JSON.stringify({ text }) };
      engine.event("s", event);
      engine.event("s", event);
    });
    engine.event("s", { type: "agent_end", runId: "r", idx: 201, data: "{}" });
    const mutation = jest.fn((timeline: TimelineState) => timeline);
    engine.mutate("s", mutation);
    await jest.advanceTimersByTimeAsync(0);
    expect(mutation).not.toHaveBeenCalled();
    await jest.advanceTimersByTimeAsync(1000);
    expect(mutation).toHaveBeenCalledTimes(1);
    expect(mutation.mock.calls[0]![0].items.at(-1)).toMatchObject({ text: parts.join("") });
    expect(mutation.mock.calls[0]![0].streaming).toBe(false);
    expect(engine.cursorFor("s").get("r")?.highWater).toBe(201);
  } finally { engine.clear(); }
});

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
    expect(requestHistory.mock.calls).toEqual([["b", expect.any(Function)]]);
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
