import { applyStreamEvents, emptyTimeline } from "../timeline";
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
