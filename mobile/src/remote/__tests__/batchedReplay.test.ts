import { applyReplayEvents, applyStreamEvent, applyStreamEvents, emptyTimeline, type TimelineState } from "../timeline";
import type { StreamEvent } from "../types";

let mockSnapshots = 0;
let mockAppends = 0;
jest.mock("@future-os/thread-projection", () => {
  const actual = jest.requireActual<typeof import("@future-os/thread-projection")>("@future-os/thread-projection");
  return { ...actual, createRunProjector: (options?: Parameters<typeof actual.createRunProjector>[0]) => {
    const projector = actual.createRunProjector(options);
    const ingest = projector.ingest;
    const snapshot = projector.snapshot;
    const append = projector.append;
    projector.ingest = events => { mockSnapshots++; return ingest(events); };
    projector.snapshot = () => { mockSnapshots++; return snapshot(); };
    projector.append = event => { mockAppends++; append(event); };
    return projector;
  } };
});

function event(type: string, idx: number, data: unknown = {}, runId = "r"): StreamEvent {
  return { type, idx, runId, data: JSON.stringify(data) };
}
function view(state: TimelineState) {
  return { items: state.items, seenEvents: state.seenEvents, currentRunId: state.currentRunId, streaming: state.streaming };
}
function workload(count: number): StreamEvent[] {
  return [event("agent_start", 0, { started_at_ms: 1000 }),
    ...Array.from({ length: count }, (_, i) => event("text_chunk", i + 1, { text: "x" })),
    event("agent_end", count + 1, { duration_ms: 1000, usage: { output_tokens: count } })];
}

beforeEach(() => {
  mockSnapshots = 0;
  mockAppends = 0;
  jest.spyOn(Date, "now").mockReturnValue(10_000);
});
afterEach(() => jest.restoreAllMocks());

test("batch folding matches single events across tools, approvals, errors, compaction and duplicates", () => {
  const events = [
    event("agent_start", 0, { started_at_ms: 1000 }),
    event("user_message", 1, { text: "question" }),
    event("thinking_start", 2), event("thinking_delta", 3, { text: "reason" }), event("thinking_end", 4),
    event("text_chunk", 5, { text: "before" }),
    event("tool_start", 6, { tool_name: "write", tool_call_id: "t", tool_args: { path: "file.txt" } }),
    event("tool_end", 7, { tool_name: "write", tool_call_id: "t", text: "done", exit_code: 0 }),
    event("approval_request", 8, { approval_request_id: "approval", tool_name: "shell" }),
    event("text_chunk", 9, { text: "after" }),
    event("text_chunk", 9, { text: "duplicate" }),
    event("text_chunk", 10, { _truncated: true }),
    event("compaction_started", 11, { operation_id: "op" }),
    event("compaction_committed", 12, { operation_id: "op", checkpoint_id: "cp", tokens_before: 100 }),
    event("usage", 13, { usage: { output_tokens: 20 } }),
    event("error", 14, { error: "failed" }),
    event("agent_end", 15, { state: "failed", duration_ms: 50, usage: { output_tokens: 30 } }),
    event("agent_start", 0, { started_at_ms: 2000 }, "other"),
    event("text_chunk", 1, { text: "another run" }, "other"),
    event("agent_end", 2, { state: "cancelled", duration_ms: 50 }, "other"),
  ];
  const expected = events.reduce(applyStreamEvent, emptyTimeline());
  expect(view(applyStreamEvents(emptyTimeline(), events))).toEqual(view(expected));
});

test("arrival order, untracked events and malformed payloads keep their old semantics", () => {
  const events: StreamEvent[] = [
    event("agent_start", 0), event("text_chunk", 5, { text: "new" }),
    event("text_chunk", 2, { text: "old" }),
    { type: "text_chunk", data: '{"text":"untracked"}' },
    { type: "error", data: "bad JSON" },
    event("ping", 6), event("agent_end", 7, { reason: "incomplete" }),
  ];
  expect(view(applyStreamEvents(emptyTimeline(), events)))
    .toEqual(view(events.reduce(applyStreamEvent, emptyTimeline())));
});

test("cooperative cancellation leaves a committed tail and its projector untouched", async () => {
  const prefix = [event("agent_start", 0), event("text_chunk", 1, { text: "prefix" })];
  const initial = prefix.reduce(applyStreamEvent, emptyTimeline());
  const before = view(initial);
  let current = true;
  let processed = 0;
  const events = Array.from({ length: 10_000 }, (_, i) => event("text_chunk", i + 2, { text: "x" }));
  const pending = applyReplayEvents(initial, events, {
    isCurrent: () => current, onEvent: () => { processed++; },
  });
  const rejected = expect(pending).rejects.toThrow("stale_sync_lane");
  setTimeout(() => { current = false; }, 0);
  await rejected;
  expect(processed).toBeGreaterThan(0);
  expect(processed).toBeLessThan(events.length);
  expect(view(initial)).toEqual(before);
  const continued = applyStreamEvent(initial, event("text_chunk", 2, { text: " kept" }));
  expect(continued.items.find(item => item.kind === "message" && item.role === "assistant"))
    .toMatchObject({ text: "prefix kept" });
});

test.each([10_000, 50_000, 100_000])("folds %i text events with one render snapshot and yields to other tasks", async count => {
  const events = workload(count);
  let processed = 0;
  let taskAt = -1;
  const start = performance.now();
  const resultPromise = applyReplayEvents(emptyTimeline(), events, { onEvent: () => { processed++; } });
  setTimeout(() => { taskAt = processed; }, 0);
  const result = await resultPromise;
  expect(taskAt).toBeGreaterThan(0);
  expect(taskAt).toBeLessThan(events.length);
  expect(mockSnapshots).toBe(1);
  expect(mockAppends).toBe(events.length);
  expect(result.seenEvents.size).toBe(events.length);
  expect(result.streaming).toBe(false);
  expect(result.items[0]).toMatchObject({ text: "x".repeat(count), outputTokens: count });
  console.warn(JSON.stringify({ events: events.length, snapshots: mockSnapshots, otherTaskAfterEvents: taskAt,
    elapsedMs: Math.round(performance.now() - start) }));
}, 30_000);

test("hundreds of interleaved tool calls retain their exact transcript with one snapshot", async () => {
  const events = [event("agent_start", 0, { started_at_ms: 1000 })];
  for (let i = 0; i < 300; i++) {
    events.push(event("tool_start", events.length, { tool_name: "shell", tool_call_id: `t${i}`, tool_args: { command: `echo ${i}` } }));
    events.push(event("tool_end", events.length, { tool_name: "shell", tool_call_id: `t${i}`, text: "done", exit_code: 0 }));
    events.push(event("text_chunk", events.length, { text: `after ${i}\n` }));
  }
  events.push(event("agent_end", events.length, { duration_ms: 1000 }));
  const expected = events.reduce(applyStreamEvent, emptyTimeline());
  mockSnapshots = 0;
  const result = await applyReplayEvents(emptyTimeline(), events);
  expect(view(result)).toEqual(view(expected));
  expect(mockSnapshots).toBe(1);
}, 30_000);

test("records a bounded baseline comparison without timing assertions", async () => {
  const events = workload(5000);
  const start = performance.now();
  const baseline = events.reduce(applyStreamEvent, emptyTimeline());
  const baselineMs = performance.now() - start;
  const baselineSnapshots = mockSnapshots;
  mockSnapshots = 0;
  const syncStart = performance.now();
  const synchronous = applyStreamEvents(emptyTimeline(), events);
  const syncBatchMs = performance.now() - syncStart;
  expect(view(synchronous)).toEqual(view(baseline));
  mockSnapshots = 0;
  const batchStart = performance.now();
  const result = await applyReplayEvents(emptyTimeline(), events);
  const batchMs = performance.now() - batchStart;
  expect(view(result)).toEqual(view(baseline));
  expect(mockSnapshots).toBe(1);
  expect(baselineSnapshots).toBe(events.length);
  console.warn(JSON.stringify({ baselineEvents: events.length, baselineSnapshots, batchSnapshots: mockSnapshots,
    baselineMs: Math.round(baselineMs), syncBatchMs: Math.round(syncBatchMs), batchMs: Math.round(batchMs) }));
});
