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

  test("a start delivered after its checkpoint is durable does not draw a second, running divider", async () => {
    // The screenshot bug: the committed divider arrived via durable history
    // (the terminal frame was lost), then the operation's own started frame
    // landed — it appended a "compacting…" divider BELOW the completed one,
    // where no terminal would ever settle it. Fold the late start into the
    // settled marker instead; its id aliases the pending identity so a late
    // terminal can still find it.
    const history = timelineFromEntries([
      { id: "cp-entry", role: "system", kind: "compaction", createdAtMs: 0, blocks: [],
        checkpoint: { checkpointId: "cp-one", trigger: "manual", phase: "standalone", tokensBefore: 13_225 } },
    ]);
    const after = await apply(history, [compact("compaction_started", "one", "old", 3)]);
    expect(after.items).toHaveLength(1);
    expect(after.items[0]).toMatchObject({
      id: "compaction:one",
      segments: [{ kind: "compaction", status: "completed", checkpointId: "cp-one", tokensBefore: 13_225 }],
    });
    // A late terminal for the same operation still settles the alias.
    const settled = await apply(after, [compact("compaction_committed", "one", "old", 4)]);
    expect(settled.items).toHaveLength(1);
    expect(settled.items[0]).toMatchObject({ id: "m_cp-one", segments: [{ status: "completed" }] });
    expect(settled.compacting).toBe(false);
  });

  test("a late start folds into a settled divider even when its terminal never arrives", async () => {
    const history = timelineFromEntries([
      { id: "cp-entry", role: "system", kind: "compaction", createdAtMs: 0, blocks: [],
        checkpoint: { checkpointId: "cp-one", trigger: "manual", phase: "standalone" } },
    ]);
    const after = await apply(history, [compact("compaction_started", "one", "old", 3)]);
    expect(after.items).toHaveLength(1);
    expect(after.items[0]).toMatchObject({ id: "compaction:one", segments: [{ status: "completed", checkpointId: "cp-one" }] });
  });

  test("repeating an operation's start never revives a divider it already settled", async () => {
    // The agent re-sends the started frame on reconnect. Once the operation's
    // marker is settled for that identity, a second start must be a no-op: it
    // may not flip the divider back to "running", or the panel would show a
    // compaction that no terminal frame can ever finish.
    const history = timelineFromEntries([
      { id: "cp-entry", role: "system", kind: "compaction", createdAtMs: 0, blocks: [],
        checkpoint: { checkpointId: "cp-one", trigger: "manual", phase: "standalone" } },
    ]);
    const once = await apply(history, [compact("compaction_started", "one", "old", 3)]);
    const twice = await apply(once, [compact("compaction_started", "one", "old", 4)]);
    expect(twice.items).toEqual(once.items);
    expect(twice.items).toHaveLength(1);
    expect(twice.items[0]).toMatchObject({ id: "compaction:one", segments: [{ status: "completed" }] });
    // The alias is still the pending identity, so a late terminal can settle it.
    const settled = await apply(twice, [compact("compaction_committed", "one", "old", 5)]);
    expect(settled.items).toHaveLength(1);
    expect(settled.items[0]).toMatchObject({ id: "m_cp-one", segments: [{ status: "completed" }] });
    expect(settled.compacting).toBe(false);
  });

  test("a genuine new compaction still draws its running divider", async () => {
    // A committed divider for an EARLIER operation must not swallow the next
    // compaction's start: the earlier divider is not the last item, so the
    // new operation's placeholder still materializes.
    const before = await apply(emptyTimeline(), [
      ...oldRun,
      compact("compaction_started", "one", "old", 3),
      compact("compaction_committed", "one", "old", 4),
      event("user_message", "new", 0, { text: "next" }),
    ]);
    const after = await apply(before, [compact("compaction_started", "two", "new", 1)]);
    expect(after.items.map(item => item.id)).toEqual(["assistant:old", "m_cp-one", "user:new", "compaction:two"]);
    expect(after.items[3]).toMatchObject({ segments: [{ status: "running" }] });
  });

  test("automatic mid-turn compaction remains inside the active reply", async () => {
    const after = await apply(emptyTimeline(), [event("agent_start", "live", 0),
      event("compaction_started", "live", 1, { operation_id: "auto", phase: "mid_turn" }),
      event("compaction_committed", "live", 2, { operation_id: "auto", checkpoint_id: "auto", phase: "mid_turn", tokens_before: 120_000, tokens_after: 18_000 }),
      event("text_chunk", "live", 3, { text: "continued" }),
    ]);
    expect(after.items).toHaveLength(1);
    expect(after.items[0]).toMatchObject({ streaming: true, segments: [{ kind: "compaction", tokensBefore: 120_000, tokensAfter: 18_000 }, { kind: "text", text: "continued" }] });
    expect(after.streaming).toBe(true);
  });

  test("a committed divider reports both token counts", async () => {
    // `tokens_before` is what the turn was about to send, `tokens_after` the
    // agent's estimate for the next one — the client renders them as a pair.
    const after = await apply(emptyTimeline(), [
      compact("compaction_started", "one", "old", 0),
      event("compaction_committed", "old", 1, {
        operation_id: "one",
        checkpoint_id: "cp-one",
        trigger: "manual",
        phase: "standalone",
        tokens_before: 190_000,
        tokens_after: 20_000,
      }),
    ]);
    expect(after.items).toMatchObject([
      { id: "m_cp-one", segments: [{ kind: "compaction", status: "completed", tokensBefore: 190_000, tokensAfter: 20_000 }] },
    ]);
  });

  test("a legacy compaction_end from a released journal keeps its before-only count", async () => {
    const after = await apply(emptyTimeline(), [
      event("agent_start", "old", 0),
      event("compaction_end", "old", 1, { tokens_before: 190_000, aborted: false }),
    ]);
    const divider = after.items.flatMap(item =>
      item.kind === "message" ? (item.segments ?? []).filter(segment => segment.kind === "compaction") : []);
    expect(divider).toHaveLength(1);
    expect(divider[0]).toMatchObject({ tokensBefore: 190_000 });
    expect(divider[0]).not.toHaveProperty("tokensAfter");
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

  test("a compaction frame without an operation or checkpoint is not a marker", async () => {
    const before = await apply(emptyTimeline(), oldRun);
    const after = await apply(before, [
      event("compaction_started", "old", 3, { trigger: "manual", phase: "standalone" }),
    ]);
    // Nothing identifies the operation, so there is no row to place and no
    // checkpoint to key later terminal frames against.
    expect(after.items).toEqual(before.items);
  });

  test("an unknown compaction-shaped frame is ignored instead of opening a divider", async () => {
    const before = await apply(emptyTimeline(), oldRun);
    const after = await apply(before, [
      event("compaction_progress", "old", 3, {
        operation_id: "one", checkpoint_id: "cp-one", trigger: "manual", phase: "standalone",
      }),
    ]);
    expect(after.items).toEqual(before.items);
  });
});

describe("replay lane fencing", () => {
  test("a replay with no events returns the timeline it was given", async () => {
    const initial = await applyReplayEvents(emptyTimeline(), oldRun);
    expect(await applyReplayEvents(initial, [])).toBe(initial);
  });

  test("a replay whose lane goes stale before or after the fold is refused", async () => {
    await expect(applyReplayEvents(emptyTimeline(), oldRun, { isCurrent: () => false }))
      .rejects.toThrow("stale_sync_lane");
    let checks = 0;
    // The lane was current for the pre-check and for the batch itself, and
    // moved on only as the finished timeline was about to be handed back.
    await expect(applyReplayEvents(emptyTimeline(), oldRun, { isCurrent: () => ++checks <= 2 }))
      .rejects.toThrow("stale_sync_lane");
  });
});
