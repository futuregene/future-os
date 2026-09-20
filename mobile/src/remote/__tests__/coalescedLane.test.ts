import { applyStreamEvents, emptyTimeline } from "../timeline";
import { SyncEngine } from "../syncEngine";
import type { StreamEvent } from "../types";

/**
 * The coalesced lane must be invisible in the rendered conversation.
 *
 * The desktop may merge a run's consecutive text fragments into one event
 * (`event_coalescing_v1`): the merged event carries the concatenated text and
 * the newest source index. This proves the two feeds produce the *same*
 * timeline — not merely similar text — for a workload with text, thinking and
 * tool streams, including the interleaved and snapshot-fragment shapes the real
 * providers emit.
 */

function fragment(type: string, idx: number, text: string, extra: Record<string, unknown> = {}): StreamEvent {
  return { type, idx, runId: "r", data: JSON.stringify({ text, ...extra }) };
}

/** Merge the way `desktop/src-tauri/src/remote/publisher/coalesce.rs` does:
 * one stream at a time, concatenating text, keeping the newest index, and
 * declaring the covered count. */
function coalesce(events: StreamEvent[]): StreamEvent[] {
  const out: StreamEvent[] = [];
  let open: { key: string; text: string; newest: StreamEvent; count: number } | null = null;
  const keyOf = (event: StreamEvent): string | null => {
    const parsed = JSON.parse(event.data) as Record<string, unknown>;
    if (typeof parsed.text !== "string") return null;
    if (!["text_chunk", "thinking_delta", "tool_delta", "toolcall_delta"].includes(event.type)) return null;
    return `${event.type}:${String(parsed.tool_id ?? parsed.tool_call_id ?? "")}`;
  };
  const flush = () => {
    if (!open) return;
    const payload = JSON.parse(open.newest.data) as Record<string, unknown>;
    out.push({
      ...open.newest,
      data: JSON.stringify({ ...payload, text: open.text, coalescedCount: open.count }),
    });
    open = null;
  };
  for (const event of events) {
    const key = keyOf(event);
    if (key === null) {
      flush();
      out.push(event);
      continue;
    }
    const parsed = JSON.parse(event.data) as Record<string, unknown>;
    if (open && open.key !== key) flush();
    if (!open) {
      open = { key, text: parsed.text as string, newest: event, count: 1 };
    } else {
      open.text = parsed.snapshot === true ? (parsed.text as string) : open.text + (parsed.text as string);
      open.newest = event;
      open.count += 1;
    }
  }
  flush();
  return out;
}

function workload(): StreamEvent[] {
  const events: StreamEvent[] = [
    { type: "agent_start", idx: 0, runId: "r", data: JSON.stringify({ started_at_ms: 1000 }) },
    { type: "user_message", idx: 1, runId: "r", data: JSON.stringify({ text: "question" }) },
  ];
  let idx = 2;
  // A reasoning run, then visible text, in many small fragments.
  events.push({ type: "thinking_start", idx: idx++, runId: "r", data: "{}" });
  for (const piece of ["We ", "must ", "check ", "the ", "file", "."]) {
    events.push(fragment("thinking_delta", idx++, piece));
  }
  events.push({ type: "thinking_end", idx: idx++, runId: "r", data: "{}" });
  for (const piece of ["The ", "answer ", "is ", "42", "."]) {
    events.push(fragment("text_chunk", idx++, piece));
  }
  // A tool call whose arguments stream in, then the execution start and result.
  events.push({
    type: "tool_start",
    idx: idx++,
    runId: "r",
    data: JSON.stringify({ tool_name: "shell", tool_id: "t1", phase: "input", tool_args: "" }),
  });
  for (const piece of ['{"comm', 'and":', '"ls"', "}"]) {
    events.push(fragment("tool_delta", idx++, piece, { tool_id: "t1" }));
  }
  events.push({
    type: "tool_start",
    idx: idx++,
    runId: "r",
    data: JSON.stringify({ tool_name: "shell", tool_id: "t1", phase: "execution", tool_args: { command: "ls" } }),
  });
  events.push({
    type: "tool_end",
    idx: idx++,
    runId: "r",
    data: JSON.stringify({ tool_name: "shell", tool_id: "t1", text: "a.txt", exit_code: 0 }),
  });
  for (const piece of ["Then ", "it ", "finished."]) {
    events.push(fragment("text_chunk", idx++, piece));
  }
  events.push({
    type: "agent_end",
    idx: idx++,
    runId: "r",
    data: JSON.stringify({ state: "completed", duration_ms: 900, usage: { output_tokens: 30 } }),
  });
  return events;
}

test("a coalesced lane renders exactly what the raw fragments render", async () => {
  const raw = workload();
  const merged = coalesce(raw);
  // The merge must be worth doing on this workload...
  expect(merged.length).toBeLessThan(raw.length);
  // ...and must not change a single character or row.
  const fromRaw = await applyStreamEvents(emptyTimeline(), raw);
  const fromMerged = await applyStreamEvents(emptyTimeline(), merged);
  expect(JSON.stringify(fromMerged.items)).toEqual(JSON.stringify(fromRaw.items));
  expect(fromMerged.currentRunId).toEqual(fromRaw.currentRunId);
  expect(fromMerged.streaming).toEqual(fromRaw.streaming);
});

test("a coalesced lane loses no fragment and no lifecycle event", async () => {
  const raw = workload();
  const merged = coalesce(raw);
  const FRAGMENTS = ["text_chunk", "text_delta", "thinking_delta", "tool_delta", "toolcall_delta"];
  const isFragment = (event: StreamEvent) => FRAGMENTS.includes(event.type);
  // Every non-fragment event survives one-for-one and in order...
  expect(merged.filter(event => !isFragment(event)).map(event => event.type))
    .toEqual(raw.filter(event => !isFragment(event)).map(event => event.type));
  // ...and the fragments are all accounted for by the declared coverage, so a
  // client that applies the ranges has seen exactly the raw sequence.
  const covered = merged
    .filter(isFragment)
    .reduce((total, event) => {
      const parsed = JSON.parse(event.data) as { coalescedCount?: number };
      return total + (parsed.coalescedCount ?? 1);
    }, 0);
  expect(covered).toBe(raw.filter(isFragment).length);
  // The run still ends on the same index, so the cursor cannot stop short.
  const lastMerged = merged[merged.length - 1];
  const lastRaw = raw[raw.length - 1];
  expect(lastMerged).toBeDefined();
  expect(lastRaw).toBeDefined();
  expect(lastMerged?.idx).toBe(lastRaw?.idx);
});

test("interleaved tool streams still render per tool after merging", async () => {
  const events: StreamEvent[] = [
    {
      type: "tool_start",
      idx: 0,
      runId: "r",
      data: JSON.stringify({ tool_name: "read", tool_id: "a", phase: "input", tool_args: "" }),
    },
    fragment("tool_delta", 1, '{"pa', { tool_id: "a" }),
    fragment("tool_delta", 2, '{"pa', { tool_id: "b" }),
    fragment("tool_delta", 3, 'th":"x"}', { tool_id: "a" }),
    fragment("tool_delta", 4, 'th":"y"}', { tool_id: "b" }),
  ];
  const fromRaw = await applyStreamEvents(emptyTimeline(), events);
  const fromMerged = await applyStreamEvents(emptyTimeline(), coalesce(events));
  expect(JSON.stringify(fromMerged.items)).toEqual(JSON.stringify(fromRaw.items));
});

/**
 * A merged event and a reconcile can cover the same source indices: the live
 * lane holds a merge window open (100 ms) while a reconcile pins the journal
 * watermark behind it. The merged event then starts *below* what the cursor has
 * already applied, and its text cannot be trimmed — the merge hides where the
 * already-seen head ends. Appending it verbatim renders the overlap twice, and
 * at a reply's tail that is what a reader sees as a duplicated fragment and a
 * reopened code fence.
 */
class LiveLane {
  /** Durable journal: one text fragment per source index, in order. */
  texts: string[] = [];
  /** The journal head a replay may read up to (the pinned watermark). */
  head = 0;
  /** When set, the next replay reply waits on it (the in-flight reconcile). */
  gate: Promise<void> | null = null;
  /** Notified when a replay request is being served (its watermark is pinned). */
  issued: (() => void) | null = null;
  replayCalls: number[] = [];
  timeline: ReturnType<typeof emptyTimeline> | null = null;
  engine: SyncEngine;

  constructor() {
    this.engine = new SyncEngine({
      requestGetState: async () => ({ activeRun: { runId: "r" }, isCompacting: false }),
      requestHistory: async () => emptyTimeline(),
      fetchReplay: async (_sessionId, _runId, since) => {
        this.replayCalls.push(since);
        const notify = this.issued;
        this.issued = null;
        notify?.();
        // The watermark is pinned when the request is served, not when the
        // reply lands: the journal keeps growing while the reply is in flight.
        const watermark = this.head;
        const gate = this.gate;
        if (gate) {
          this.gate = null;
          await gate;
        }
        const events = this.journal()
          .filter(event => (event.idx ?? -1) > since && (event.idx ?? -1) <= watermark)
          .map(event => ({ type: event.type, data: event.data, runId: event.runId, idx: event.idx }));
        return { events, watermark };
      },
    });
    this.engine.subscribe(commit => { this.timeline = commit.timeline; });
  }

  journal(): StreamEvent[] {
    return [
      { type: "agent_start", idx: 0, runId: "r", data: "{}" },
      ...this.texts.map((text, index) => fragment("text_chunk", index + 1, text)),
    ];
  }

  text(): string {
    return (this.timeline?.items ?? [])
      .filter((item) => item.kind === "message")
      .map((item) => (item.kind === "message" ? item.text : ""))
      .join("");
  }

  async drain(): Promise<void> {
    await new Promise(resolve => setTimeout(resolve, 100));
  }
}

/** A merge of sources `from..to`, the way the desktop publishes one. */
function mergedRange(lane: LiveLane, from: number, to: number): StreamEvent {
  const text = lane.texts.slice(from - 1, to).join("");
  return { type: "text_chunk", idx: to, runId: "r", coalescedCount: to - from + 1, data: JSON.stringify({ text }) };
}

function journalText(count: number): string[] {
  return Array.from({ length: count }, (_value, index) => `<${index + 1}>`);
}

/** A gate plus its resolver, so a test can hold a reply in flight. */
function deferred(): { promise: Promise<void>; resolve: () => void } {
  let resolve!: () => void;
  const promise = new Promise<void>(r => { resolve = r; });
  return { promise, resolve };
}

test("a merged range overlapping an applied prefix is recovered, not appended twice", async () => {
  const lane = new LiveLane();
  lane.texts = journalText(12);
  lane.head = 4;
  try {
    await lane.engine.open("s");
    await lane.drain();
    expect(lane.text()).toBe("<1><2><3><4>");

    // A reconcile is in flight and pins a watermark behind the journal head;
    // the desktop meanwhile flushes one merge covering sources 5..12.
    const gate = deferred();
    const served = deferred();
    lane.gate = gate.promise;
    lane.issued = served.resolve;
    lane.head = 9;
    lane.engine.reconcile("s", "gap", "r");
    // The reply is pinned at source 9 and still in flight when the merge lands.
    await served.promise;
    lane.engine.event("s", mergedRange(lane, 5, 12));
    // The journal advances while the reply is in flight, so the merge reaches
    // past the watermark the reconcile pinned.
    lane.head = 12;
    gate.resolve();
    await lane.drain();
    await lane.drain();

    // The merged text overlaps sources 5..9, which the reconcile already
    // applied. Every fragment must appear exactly once.
    expect(lane.text()).toBe(lane.texts.join(""));
  } finally {
    lane.engine.clear();
  }
});

test("a merged range that continues the cursor needs no replay", async () => {
  const lane = new LiveLane();
  lane.texts = journalText(12);
  lane.head = 4;
  try {
    await lane.engine.open("s");
    await lane.drain();
    const before = lane.replayCalls.length;

    lane.head = 12;
    lane.engine.event("s", mergedRange(lane, 5, 12));
    await lane.drain();

    expect(lane.text()).toBe(lane.texts.join(""));
    expect(lane.replayCalls.length).toBe(before);
  } finally {
    lane.engine.clear();
  }
});

test("snapshot-style fragments replace rather than append after merging", async () => {
  const events: StreamEvent[] = [
    {
      type: "tool_start",
      idx: 0,
      runId: "r",
      data: JSON.stringify({ tool_name: "read", tool_id: "a", phase: "input", tool_args: "" }),
    },
    fragment("tool_delta", 1, '{"path":', { tool_id: "a" }),
    fragment("tool_delta", 2, '{"path":"/tmp/x"}', { tool_id: "a", snapshot: true }),
  ];
  const fromRaw = await applyStreamEvents(emptyTimeline(), events);
  const fromMerged = await applyStreamEvents(emptyTimeline(), coalesce(events));
  expect(JSON.stringify(fromMerged.items)).toEqual(JSON.stringify(fromRaw.items));
});
