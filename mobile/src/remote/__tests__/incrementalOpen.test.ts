import { SyncEngine, type ReplayResult } from "../syncEngine";
import { emptyTimeline, timelineFromEntries } from "../timeline";
import type { StreamEvent } from "../types";

const engines: SyncEngine[] = [];
const event = (type: string, idx: number, data: unknown = {}, runId = "r"): StreamEvent =>
  ({ type, idx, runId, data: JSON.stringify(data) });
const text = (idx: number, value: string) => event("text_chunk", idx, { text: value });
const prefix = () => [event("agent_start", 0, { started_at_ms: 1000 }), text(1, "prefix")];

function harness() {
  let active = "r";
  let visible = true;
  let history = emptyTimeline();
  let journal = prefix();
  const requestHistory = jest.fn(async () => history);
  const fetchReplay = jest.fn(async (_session: string, run: string, since: number): Promise<ReplayResult> => ({
    events: journal.filter(e => e.runId === run && e.idx! > since).map(e => ({ ...e })),
    watermark: journal.filter(e => e.runId === run).at(-1)?.idx ?? -1,
  }));
  const onSyncStatus = jest.fn();
  const onTiming = jest.fn();
  const engine = new SyncEngine({
    isSessionVisible: () => visible,
    requestGetState: async () => ({ activeRun: active ? { runId: active } : null }),
    requestHistory, fetchReplay, onSyncStatus, onTiming,
  });
  engines.push(engine);
  return {
    engine, fetchReplay, requestHistory, onSyncStatus, onTiming,
    active: (value: string) => { active = value; },
    visible: (value: boolean) => { visible = value; },
    history: (value: ReturnType<typeof emptyTimeline>) => { history = value; },
    journal: (value: StreamEvent[]) => { journal = value; },
    async open() { await engine.open("s"); await jest.advanceTimersByTimeAsync(0); },
    assistant() { return engine.timelineFor("s")?.items.filter(i => i.kind === "message" && i.role === "assistant"); },
  };
}

beforeEach(() => jest.useFakeTimers());
afterEach(() => { engines.splice(0).forEach(engine => engine.clear()); jest.useRealTimers(); });

test.each(["open", "reconnect"] as const)("%s resumes a proven active run and matches a cold rebuild including tools", async reason => {
  const h = harness();
  const initial = [...prefix(),
    event("thinking_start", 2), event("thinking_delta", 3, { text: "reason" }),
    event("thinking_end", 4), event("tool_start", 5, { tool_name: "shell", tool_call_id: "t", tool_args: { command: "echo ok" } }),
  ];
  h.journal(initial);
  await h.open();
  const cached = h.engine.timelineFor("s");
  const complete = [...initial, event("tool_end", 6, { tool_name: "shell", tool_call_id: "t", text: "ok", exit_code: 0 }), text(7, " tail")];
  h.journal(complete);
  h.engine.restart("s", reason);
  await jest.advanceTimersByTimeAsync(0);
  expect(h.fetchReplay.mock.calls.map(call => call[2])).toEqual([-1, 5]);
  expect(h.onTiming.mock.calls[0][0].replayPlan).toEqual({
    mode: "full", sinceIdx: -1, cachedHighWater: -1, prefixDecision: "no-cache",
  });
  expect(h.onTiming.mock.calls.at(-1)?.[0].replayPlan).toEqual({
    mode: "incremental", sinceIdx: 5, cachedHighWater: 5, prefixDecision: "reused",
  });
  expect(h.requestHistory).toHaveBeenCalledTimes(2);
  expect(h.engine.cursorFor("s").get("r")).toEqual({ highWater: 7, prefixComplete: true });
  expect(h.onSyncStatus).toHaveBeenLastCalledWith("s", "idle");
  expect(h.engine.streamingFor("s")).toBe(true);
  expect(cached?.items[0]).toMatchObject({ text: "prefix" }); // replay fork did not mutate the cache

  const cold = harness();
  cold.journal(complete);
  await cold.open();
  expect(h.engine.timelineFor("s")?.items).toEqual(cold.engine.timelineFor("s")?.items);
});

test("warm delta keeps refreshed user attachments and replaces a durable partial assistant mirror", async () => {
  const h = harness();
  await h.open();
  h.history(timelineFromEntries([
    { id: "u", kind: "user", role: "user", runId: "r", createdAtMs: 1, blocks: [{ kind: "text", text: "question" }], metadata: { attachments: [{ path: "file.pdf", name: "file.pdf", kind: "file" }] } },
    { id: "partial", kind: "assistant", role: "assistant", runId: "r", createdAtMs: 2, blocks: [{ kind: "text", text: "partial durable mirror" }] },
  ]));
  h.journal([...prefix(), text(2, " tail")]);
  await h.open();
  expect(h.fetchReplay.mock.calls.at(-1)?.[2]).toBe(1);
  expect(h.assistant()).toEqual([expect.objectContaining({ text: "prefix tail" })]);
  expect(h.engine.timelineFor("s")?.items.find(i => i.kind === "message" && i.role === "user"))
    .toMatchObject({ attachments: [expect.objectContaining({ name: "file.pdf" })] });
});

test("empty valid tail preserves the prefix and the following live delta appends exactly once", async () => {
  const h = harness();
  await h.open();
  await h.open();
  expect(h.fetchReplay.mock.calls.map(call => call[2])).toEqual([-1, 1]);
  h.engine.event("s", text(2, " live"));
  h.engine.event("s", text(2, " live"));
  await jest.advanceTimersByTimeAsync(80);
  expect(h.assistant()).toEqual([expect.objectContaining({ text: "prefix live" })]);
});

test.each([
  ["missing projector", "missing-projector"], ["missing assistant", "missing-assistant"],
  ["incomplete cursor", "prefix-incomplete"], ["truncated content", "truncated"],
  ["not streaming", "not-streaming"],
])("%s prevents prefix reuse", async (missing, decision) => {
  const h = harness();
  await h.open();
  if (missing === "incomplete cursor") h.engine.cursorFor("s").set("r", { highWater: 1, prefixComplete: false });
  else h.engine.mutate("s", cached => ({ ...cached,
    ...(missing === "missing projector" ? { liveRuns: new Map() } : {}),
    ...(missing === "missing assistant" ? { items: [] } : {}),
    ...(missing === "not streaming" ? { streaming: false } : {}),
    ...(missing === "truncated content" ? { items: [...cached.items, { kind: "notice" as const, id: "truncated", runId: "r", tone: "danger" as const, text: "truncated" }] } : {}),
  }));
  await jest.advanceTimersByTimeAsync(0);
  await h.open();
  expect(h.fetchReplay.mock.calls.at(-1)?.[2]).toBe(-1);
  expect(h.onTiming.mock.calls.at(-1)?.[0].replayPlan).toEqual({
    mode: "full", sinceIdx: -1, cachedHighWater: 1, prefixDecision: decision,
  });
  expect(h.assistant()).toEqual([expect.objectContaining({ text: "prefix" })]);
});

test.each(["open", "reconnect"] as const)("%s refreshes a run that ended while hidden instead of treating it as active", async reason => {
  const h = harness();
  await h.open();
  h.active("");
  h.journal([...prefix(), text(2, " final"), event("agent_end", 3, { duration_ms: 1200, usage: { output_tokens: 42 } })]);
  h.engine.restart("s", reason);
  await jest.advanceTimersByTimeAsync(0);
  expect(h.fetchReplay.mock.calls.at(-1)?.[2]).toBe(-1);
  expect(h.onTiming.mock.calls.at(-1)?.[0].replayPlan).toEqual({
    mode: "full", sinceIdx: -1, cachedHighWater: 1, prefixDecision: "run-inactive-or-changed",
  });
  expect(h.assistant()).toEqual([expect.objectContaining({ text: "prefix final", durationMs: 1200, outputTokens: 42, streaming: false })]);
});

test("durable completion discovered after get_state wins over the cached active prefix", async () => {
  const h = harness();
  await h.open();
  // get_state still reports r active, but the following history read already
  // contains its final reply; the event journal is no longer available.
  h.history(timelineFromEntries([
    { id: "u", kind: "user", role: "user", runId: "r", createdAtMs: 1, blocks: [{ kind: "text", text: "question" }] },
    { id: "a", kind: "assistant", role: "assistant", runId: "r", createdAtMs: 2, blocks: [{ kind: "text", text: "durable final" }], run: { status: "completed", durationMs: 1200 }, usage: { outputTokens: 42 } },
  ]));
  h.journal([]);
  await h.open();
  expect(h.fetchReplay.mock.calls.at(-1)?.[2]).toBe(-1);
  expect(h.assistant()).toEqual([expect.objectContaining({ text: "durable final", durationMs: 1200, outputTokens: 42 })]);
  expect(h.engine.streamingFor("s")).toBe(false);
  expect(h.onSyncStatus).toHaveBeenLastCalledWith("s", "idle");
});

test("a new active run refreshes history and gets its full prefix", async () => {
  const h = harness();
  await h.open();
  h.active("new");
  h.history(timelineFromEntries([
    { id: "u", kind: "user", role: "user", runId: "r", createdAtMs: 1, blocks: [{ kind: "text", text: "old question" }] },
    { id: "a", kind: "assistant", role: "assistant", runId: "r", createdAtMs: 2, blocks: [{ kind: "text", text: "old final" }] },
    { id: "new-u", kind: "user", role: "user", runId: "new", createdAtMs: 3, blocks: [{ kind: "text", text: "new question" }] },
  ]));
  h.journal([event("agent_start", 0, {}, "new"), event("text_chunk", 1, { text: "new prefix" }, "new")]);
  await h.open();
  expect(h.fetchReplay.mock.calls.at(-1)?.slice(1, 3)).toEqual(["new", -1]);
  expect(h.assistant()?.map(i => i.kind === "message" ? i.text : "")).toEqual(["old final", "new prefix"]);
});

test("a snapshot notification while hidden invalidates the cache without eager network work", async () => {
  const h = harness();
  await h.open();
  h.visible(false);
  h.engine.reconcile("s", "resend", "r");
  expect(h.fetchReplay).toHaveBeenCalledTimes(1);
  h.visible(true);
  h.journal([event("agent_start", 0), text(1, "replacement")]);
  await h.open();
  expect(h.fetchReplay.mock.calls.at(-1)?.[2]).toBe(-1);
  expect(h.onTiming.mock.calls.at(-1)?.[0].replayPlan).toEqual({
    mode: "full", sinceIdx: -1, cachedHighWater: 1, prefixDecision: "baseline-untrusted",
  });
  expect(h.assistant()).toEqual([expect.objectContaining({ text: "replacement" })]);
});

test("switching away during a large warm replay never partially advances the committed cache", async () => {
  const h = harness();
  await h.open();
  const cached = h.engine.timelineFor("s");
  h.journal([...prefix(), ...Array.from({ length: 10_000 }, (_, i) => text(i + 2, " discarded"))]);
  await h.engine.open("s");
  setTimeout(() => h.visible(false), 0);
  await jest.runAllTimersAsync();
  expect(h.engine.timelineFor("s")).toBe(cached);
  expect(h.engine.cursorFor("s").get("r")?.highWater).toBe(1);
  h.visible(true);
  h.journal([...prefix(), text(2, " kept")]);
  await h.open();
  expect(h.fetchReplay.mock.calls.map(call => call[2])).toEqual([-1, 1, 1]);
  expect(h.assistant()).toEqual([expect.objectContaining({ text: "prefix kept" })]);
});

test("a snapshot notification during a tail fences the old reply and survives an immediate reopen", async () => {
  const h = harness();
  await h.open();
  let finish!: (result: ReplayResult) => void;
  h.fetchReplay.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  await h.open();
  h.engine.reconcile("s", "resend", "r");
  h.journal([event("agent_start", 0), text(1, "replacement")]);
  await h.open();
  finish({ events: [text(2, " stale")].map(e => ({ ...e })), watermark: 2 });
  await jest.advanceTimersByTimeAsync(0);
  expect(h.fetchReplay.mock.calls.map(call => call[2])).toEqual([-1, 1, -1]);
  expect(h.assistant()).toEqual([expect.objectContaining({ text: "replacement" })]);
  expect(h.onSyncStatus).toHaveBeenLastCalledWith("s", "idle");
});

test.each([
  { label: "gap", result: { events: [text(3, "bad")], watermark: 3 } },
  { label: "missing suffix", result: { events: [text(2, "partial")], watermark: 3 } },
  { label: "rewound watermark", result: { events: [], watermark: 0 } },
  { label: "truncation without snapshot", result: { events: [text(2, "partial")], truncated: true } },
  { label: "wrong run", result: { events: [event("text_chunk", 2, { text: "wrong" }, "other")] } },
])("invalid incremental reply ($label) keeps the cache and retries a full prefix", async ({ result }) => {
  const h = harness();
  await h.open();
  h.journal([...prefix(), text(2, " recovered")]);
  h.fetchReplay.mockResolvedValueOnce(result as ReplayResult);
  await h.open();
  expect(h.assistant()).toEqual([expect.objectContaining({ text: "prefix" })]);
  expect(h.engine.cursorFor("s").get("r")?.highWater).toBe(1);
  expect(h.onSyncStatus).toHaveBeenLastCalledWith("s", "retrying");
  await jest.advanceTimersByTimeAsync(500);
  expect(h.fetchReplay.mock.calls.map(call => call[2])).toEqual([-1, 1, -1]);
  expect(h.assistant()).toEqual([expect.objectContaining({ text: "prefix recovered" })]);
  expect(h.onSyncStatus).toHaveBeenLastCalledWith("s", "idle");
});

test("a transient tail failure preserves the proven prefix for the retry", async () => {
  const h = harness();
  await h.open();
  h.journal([...prefix(), text(2, " recovered")]);
  h.fetchReplay.mockRejectedValueOnce(new Error("network timeout"));
  await h.open();
  await jest.advanceTimersByTimeAsync(500);
  expect(h.fetchReplay.mock.calls.map(call => call[2])).toEqual([-1, 1, 1]);
  expect(h.assistant()).toEqual([expect.objectContaining({ text: "prefix recovered" })]);
});

test("a replacement projection rewinds cursor and dedup together, then accepts new live events", async () => {
  const h = harness();
  h.journal([...prefix(), text(2, " old"), text(3, " old")]);
  await h.open();
  h.fetchReplay.mockResolvedValueOnce({ events: [], truncated: true, projection: {
    run_id: "r", cursor: 1, events: [event("agent_start", 0), text(1, "replacement")].map(e => ({ ...e })),
  } });
  await h.open();
  expect(h.engine.cursorFor("s").get("r")).toEqual({ highWater: 1, prefixComplete: true });
  h.engine.event("s", text(2, " new"));
  await jest.advanceTimersByTimeAsync(80);
  expect(h.assistant()).toEqual([expect.objectContaining({ text: "replacement new" })]);
});

test("a watermarked full reply with a missing prefix cannot establish an incremental baseline", async () => {
  const h = harness();
  h.fetchReplay.mockResolvedValueOnce({ events: [text(1, "missing start")].map(e => ({ ...e })), watermark: 1 });
  await h.open();
  expect(h.engine.cursorFor("s").has("r")).toBe(false);
  expect(h.onSyncStatus).toHaveBeenLastCalledWith("s", "retrying");
  await jest.advanceTimersByTimeAsync(500);
  expect(h.fetchReplay.mock.calls.map(call => call[2])).toEqual([-1, -1]);
  expect(h.assistant()).toEqual([expect.objectContaining({ text: "prefix" })]);
});

test("a legacy unwatermarked partial replay remains ineligible for warm reuse", async () => {
  const h = harness();
  h.fetchReplay.mockResolvedValueOnce({ events: [text(5, "partial")].map(e => ({ ...e })) });
  await h.open();
  expect(h.engine.cursorFor("s").get("r")?.prefixComplete).toBe(false);
  await h.open();
  expect(h.fetchReplay.mock.calls.map(call => call[2])).toEqual([-1, -1]);
  expect(h.engine.cursorFor("s").get("r")).toEqual({ highWater: 1, prefixComplete: true });
});

test.each(["wrong run", "invalid cursor", "incomplete snapshot"])("a projection with %s falls back without corrupting the prefix", async invalid => {
  const h = harness();
  await h.open();
  h.fetchReplay.mockResolvedValueOnce({ events: [], watermark: invalid === "incomplete snapshot" ? 3 : 1,
    projection: { runId: invalid === "wrong run" ? "other" : "r", cursor: invalid === "invalid cursor" ? -1 : 1,
      events: prefix().map(e => ({ type: e.type, idx: e.idx, data: e.data })) },
  });
  await h.open();
  expect(h.assistant()).toEqual([expect.objectContaining({ text: "prefix" })]);
  expect(h.onSyncStatus).toHaveBeenLastCalledWith("s", "retrying");
  await jest.advanceTimersByTimeAsync(500);
  expect(h.fetchReplay.mock.calls.map(call => call[2])).toEqual([-1, 1, -1]);
  expect(h.onSyncStatus).toHaveBeenLastCalledWith("s", "idle");
});

test("queued live events overlapping a warm replay are deduplicated, including agent_end", async () => {
  const h = harness();
  await h.open();
  let finish!: (result: ReplayResult) => void;
  h.fetchReplay.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  await h.open();
  const tail = [text(2, " final"), event("agent_end", 3, { duration_ms: 1200, usage: { output_tokens: 9 } })];
  h.journal([...prefix(), ...tail]);
  h.active("");
  tail.forEach(e => h.engine.event("s", e));
  finish({ events: tail.map(e => ({ ...e })), watermark: 3 });
  await jest.advanceTimersByTimeAsync(80);
  expect(h.assistant()).toEqual([expect.objectContaining({ text: "prefix final", streaming: false, durationMs: 1200, outputTokens: 9 })]);
  expect(h.engine.cursorFor("s").get("r")?.highWater).toBe(3);
  expect(h.onSyncStatus).toHaveBeenLastCalledWith("s", "idle");
});
