import { applyReplayEvents, applyStreamEvent, applyStreamEvents, emptyTimeline, timelineFromEntries } from "../projection";
import type { StreamEvent } from "../types";

function event(type: string, runId: string, idx: number, data: object = {}): StreamEvent {
  return { type, runId, idx, data: JSON.stringify(data) };
}
function compact(type: string, operation: string, run: string, idx: number): StreamEvent {
  return event(type, run, idx, { operation_id: operation, checkpoint_id: `cp-${operation}`, trigger: "manual", phase: "standalone" });
}
const oldRun = [event("agent_start", "old", 0), event("text_chunk", "old", 1, { text: "finished reply" }), event("agent_end", "old", 2)];
const projectors = {
  single: async (initial: ReturnType<typeof emptyTimeline>, events: StreamEvent[]) => events.reduce(applyStreamEvent, initial),
  batch: async (initial: ReturnType<typeof emptyTimeline>, events: StreamEvent[]) => applyStreamEvents(initial, events),
  replay: applyReplayEvents,
};

describe.each(Object.entries(projectors))("%s compaction projection", (_name, apply) => {
  test("standalone started/committed renders a divider without reviving the old reply", async () => {
    const before = await apply(emptyTimeline(), oldRun);
    const started = await apply(before, [compact("compaction_started", "one", "old", 3)]);
    expect(started.streaming).toBe(false);
    expect(started.compacting).toBe(true);
    expect(started.items).toHaveLength(2);
    expect(started.items[1]).toMatchObject({ streaming: false, segments: [{ kind: "compaction", status: "running" }] });
    const after = await apply(started, [compact("compaction_committed", "one", "old", 4)]);
    expect(after.streaming).toBe(false);
    expect(after.compacting).toBe(false);
    expect(after.items).toHaveLength(2);
    expect(after.items[0]).toEqual(before.items[0]);
    expect(after.items[1]).toMatchObject({ id: "m_cp-one", streaming: false, segments: [{ kind: "compaction", status: "completed" }] });
    expect(after.liveRuns?.size).toBe(1);
  });

  test.each(["compaction_failed", "compaction_unchanged"])("%s settles only its operation, not a reply", async terminal => {
    const initial = await apply(emptyTimeline(), oldRun);
    const after = await apply(initial, [compact("compaction_started", "one", "old", 3), compact(terminal, "one", "old", 4)]);
    expect(after.streaming).toBe(false);
    expect(after.compacting).toBe(false);
    if (terminal === "compaction_unchanged") expect(after.items).toEqual(initial.items);
    else expect(after.items[1]).toMatchObject({ streaming: false, segments: [{ status: "failed" }] });
  });

  test("two compactions across run idx resets stay separate and in chronological order", async () => {
    const result = await apply(emptyTimeline(), [
      ...oldRun, compact("compaction_started", "one", "old", 3), compact("compaction_committed", "one", "old", 4),
      event("user_message", "new", 0, { text: "next" }), event("agent_start", "new", 1),
      event("text_chunk", "new", 2, { text: "second reply" }), event("agent_end", "new", 3),
      compact("compaction_started", "two", "new", 4), compact("compaction_committed", "two", "new", 5),
    ]);
    expect(result.items.map(item => item.id)).toEqual(["assistant:old", "m_cp-one", "user:new", "assistant:new", "m_cp-two"]);
    expect(result.streaming).toBe(false);
  });

  test("a late compaction from an older run cannot clear another active run", async () => {
    const before = await apply(emptyTimeline(), [...oldRun, event("agent_start", "live", 0), event("text_chunk", "live", 1, { text: "working" })]);
    const after = await apply(before, [compact("compaction_committed", "one", "old", 3)]);
    expect(after.streaming).toBe(true);
    expect(after.currentRunId).toBe("live");
    expect(after.items.slice(0, 2)).toEqual(before.items);
  });

  test("terminal replay deduplicates the same checkpoint loaded from history", async () => {
    const history = timelineFromEntries([{ id: "entry", role: "system", kind: "compaction", createdAtMs: 0, blocks: [],
      checkpoint: { checkpointId: "cp-one", trigger: "manual", phase: "standalone" } }]);
    const after = await apply(history, [compact("compaction_started", "one", "old", 3), compact("compaction_committed", "one", "old", 4)]);
    expect(after.items).toHaveLength(1);
    expect(after.items[0]).toMatchObject({ id: "m_cp-one", streaming: false });
    expect(after.streaming).toBe(false);
  });

  test("automatic mid-turn compaction remains inside the active reply", async () => {
    const after = await apply(emptyTimeline(), [event("agent_start", "live", 0),
      event("compaction_started", "live", 1, { operation_id: "auto", phase: "mid_turn" }),
      event("compaction_committed", "live", 2, { operation_id: "auto", checkpoint_id: "auto", phase: "mid_turn" }),
      event("text_chunk", "live", 3, { text: "continued" }),
    ]);
    expect(after.items).toHaveLength(1);
    expect(after.items[0]).toMatchObject({ streaming: true, segments: [{ kind: "compaction" }, { kind: "text", text: "continued" }] });
    expect(after.streaming).toBe(true);
  });

  test("a checkpoint that opened the turn is drawn once, not also from durable history", async () => {
    // The checkpoint commits before any reply entry of that exchange is saved,
    // so durable history renders the turn as a divider-only row. The run's own
    // replay carries the same checkpoint — both drew it, one after the other.
    const history = timelineFromEntries([
      { id: "u1", role: "user", kind: "user", createdAtMs: 0, blocks: [{ kind: "text", text: "go" }] },
      {
        id: "cp-entry",
        role: "system",
        kind: "compaction",
        createdAtMs: 1,
        blocks: [],
        checkpoint: {
          schemaVersion: 3,
          checkpointId: "cp-auto",
          phase: "pre_turn",
          trigger: "automatic",
          tokensBefore: 903_386,
        },
      },
    ]);
    expect(history.items).toHaveLength(2);

    const after = await apply(history, [
      event("agent_start", "live", 0),
      event("compaction_started", "live", 1, { operation_id: "auto", phase: "pre_turn" }),
      event("compaction_committed", "live", 2, {
        operation_id: "auto",
        checkpoint_id: "cp-auto",
        phase: "pre_turn",
        tokens_before: 903_386,
      }),
      event("text_chunk", "live", 3, { text: "continued" }),
    ]);

    const dividers = after.items.flatMap(item =>
      item.kind === "message" ? (item.segments ?? []).filter(segment => segment.kind === "compaction") : []);
    expect(dividers).toHaveLength(1);
    expect(after.items.map(item => item.id)).toEqual(["m_u1", "assistant:live"]);
  });

  test("a durable divider for another checkpoint is not superseded by this run", async () => {
    const history = timelineFromEntries([
      { id: "u1", role: "user", kind: "user", createdAtMs: 0, blocks: [{ kind: "text", text: "go" }] },
      {
        id: "cp-old",
        role: "system",
        kind: "compaction",
        createdAtMs: 1,
        blocks: [],
        checkpoint: { schemaVersion: 3, checkpointId: "cp-other", phase: "pre_turn", trigger: "automatic" },
      },
    ]);

    const after = await apply(history, [
      event("agent_start", "live", 0),
      event("compaction_started", "live", 1, { operation_id: "auto", phase: "pre_turn" }),
      event("compaction_committed", "live", 2, {
        operation_id: "auto",
        checkpoint_id: "cp-auto",
        phase: "pre_turn",
        tokens_before: 903_386,
      }),
    ]);

    expect(after.items.flatMap(item =>
      item.kind === "message" ? (item.segments ?? []).filter(segment => segment.kind === "compaction") : [],
    )).toHaveLength(2);
  });
});
