import type { AgentMessage } from "@future-os/thread-projection";
import type { StoredRun, StoredRunEvent } from "../../integrations/storage/threadStore";
import { describe, expect, it } from "vitest";
import { applyRecoveredEvents, applyRunMetadata, deriveRenderFields, mergeStreamingPreview, patchMessage, recoverFailedRuns, runDurationMs, streamingBubbleBase } from "./threadRunProjection";

function message(id: string, patch: Partial<AgentMessage> = {}): AgentMessage {
  return {
    id,
    role: "assistant",
    authorKey: "author.researchCopilot",
    content: "",
    createdAt: "2026-01-01T00:00:00.000Z",
    ...patch,
  };
}

function events(list: Array<[string, Record<string, unknown>]>): StoredRunEvent[] {
  return list.map(([eventType, payload], index) => ({
    id: `e${index}`,
    runId: "r1",
    eventType,
    payload: JSON.stringify(payload),
    sequence: index,
    createdAt: index,
  }));
}

function applyPatch(
  state: AgentMessage[],
  ...args: Parameters<typeof patchMessage> extends [unknown, ...infer Rest] ? Rest : never
): AgentMessage[] {
  let next = state;
  patchMessage((action) => {
    next = typeof action === "function" ? action(next) : action;
  }, ...args);
  return next;
}

describe("patchMessage", () => {
  it("patches only the matching message and leaves others untouched", () => {
    const state = applyPatch(
      [message("a", { content: "one" }), message("b", { content: "two" })],
      "b",
      { content: "patched" },
    );
    expect(state.map(m => m.content)).toEqual(["one", "patched"]);
  });

  it("supports a functional patch derived from the current message", () => {
    const state = applyPatch(
      [message("a", { content: "x", outputTokens: 5 })],
      "a",
      prev => ({ outputTokens: (prev.outputTokens ?? 0) + 1 }),
    );
    expect(state[0]?.outputTokens).toBe(6);
  });

  it("is a no-op when no id matches", () => {
    const state = applyPatch([message("a", { content: "x" })], "missing", { content: "y" });
    expect(state[0]?.content).toBe("x");
  });
});

describe("runDurationMs", () => {
  it("uses persisted start/end when both present and ordered", () => {
    const run = { startedAt: 1000, endedAt: 3500 } as StoredRun;
    expect(runDurationMs(run)).toBe(2500);
  });

  it("ignores an inverted end/start and falls back to null without a fallback anchor", () => {
    const run = { startedAt: 3000, endedAt: 1000 } as StoredRun;
    expect(runDurationMs(run)).toBeNull();
  });

  it("returns null when nothing is known", () => {
    expect(runDurationMs(null)).toBeNull();
    expect(runDurationMs(undefined)).toBeNull();
  });

  it("falls back to elapsed-since-anchor while the run is still settling", () => {
    expect(runDurationMs(null, Date.now())).toBeGreaterThanOrEqual(0);
  });
});

function user(id: string, patch: Partial<AgentMessage> = {}): AgentMessage {
  return message(id, { role: "user", authorKey: "author.you", status: "complete", ...patch });
}

function assistant(id: string, patch: Partial<AgentMessage> = {}): AgentMessage {
  return message(id, { status: "complete", ...patch });
}

function run(id: string, patch: Partial<StoredRun> = {}): StoredRun {
  return {
    id,
    threadId: "t1",
    status: "completed",
    createdAt: 0,
    updatedAt: 0,
    ...patch,
  } as StoredRun;
}

describe("applyRunMetadata", () => {
  it("uses canonical run ids instead of positional or timestamp alignment", () => {
    const result = applyRunMetadata([
      user("u1"),
      assistant("a1", { runId: "r-old", createdAt: "2030-01-01T00:00:00.000Z" }),
      user("u2"),
      assistant("a2", { runId: "r-new", createdAt: "2020-01-01T00:00:00.000Z" }),
    ], [
      run("r-new", { status: "failed", modelId: "new-model" }),
      run("r-old", { status: "completed", modelId: "old-model" }),
    ]);

    expect(result[1]).toMatchObject({ id: "a1", runId: "r-old", status: "complete", modelId: "old-model" });
    expect(result[3]).toMatchObject({ id: "a2", runId: "r-new", status: "failed", modelId: "new-model" });
  });

  it("marks the most recent exchange failed when its run failed", () => {
    const messages = [
      user("u1"),
      assistant("a1"),
      user("u2"),
      assistant("a2"),
    ];
    // Newest run first (created_at DESC): a2 ↔ r2 (failed), a1 ↔ r1.
    const result = applyRunMetadata(messages, [
      run("r2", { status: "failed", modelId: "m-2" }),
      run("r1", { status: "completed", modelId: "m-1" }),
    ]);
    expect(result[3]).toMatchObject({ id: "a2", runId: "r2", status: "failed", modelId: "m-2", stopped: false });
    expect(result[1]).toMatchObject({ id: "a1", runId: "r1", status: "complete", modelId: "m-1", stopped: false });
  });

  it("marks a cancelled run's exchange as stopped without failing it", () => {
    const result = applyRunMetadata([user("u1"), assistant("a1")], [run("r1", { status: "cancelled" })]);
    expect(result[1]).toMatchObject({ id: "a1", runId: "r1", status: "complete", stopped: true });
  });

  it("aligns from the newest end and ignores extra older runs", () => {
    // One exchange, two runs: only the newest run pairs with the exchange.
    const result = applyRunMetadata([user("u1"), assistant("a1")], [
      run("r-new", { status: "failed" }),
      run("r-old", { status: "completed" }),
    ]);
    expect(result[1]).toMatchObject({ id: "a1", runId: "r-new", status: "failed" });
  });

  it("leaves older exchanges untouched when there are fewer runs than exchanges", () => {
    const result = applyRunMetadata([
      user("u1"),
      assistant("a1"),
      user("u2"),
      assistant("a2"),
    ], [run("r2", { status: "failed" })]);
    // Newest exchange pairs with the only run; the older exchange keeps its defaults.
    expect(result[3]).toMatchObject({ id: "a2", runId: "r2", status: "failed" });
    expect(result[1]?.runId).toBeUndefined();
    expect(result[1]?.status).toBe("complete");
  });

  it("does not consume a run slot for a compaction divider", () => {
    const divider = assistant("div", { content: "", segments: [{ id: "s", kind: "compaction" }] });
    const result = applyRunMetadata([
      user("u1"),
      assistant("a1"),
      divider,
      user("u2"),
      assistant("a2"),
    ], [
      run("r2", { status: "failed" }),
      run("r1", { status: "completed" }),
    ]);
    expect(result[4]).toMatchObject({ id: "a2", runId: "r2", status: "failed" });
    expect(result[1]).toMatchObject({ id: "a1", runId: "r1" });
    // The divider stays a plain complete marker with no run attached.
    expect(result[2]?.runId).toBeUndefined();
    expect(result[2]?.status).toBe("complete");
  });

  it("stamps an aborted (empty) exchange with the run's end time — the stop time", () => {
    const stopMs = Date.parse("2026-07-01T10:00:06.000Z");
    const result = applyRunMetadata(
      [
        user("u1", { createdAt: "2026-07-01T10:00:00.000Z" }),
        // The exchange's projected time falls inside the run's window (as real
        // session-derived exchanges do) so window matching can pair them.
        assistant("a1", { content: "", createdAt: "2026-07-01T10:00:01.000Z" }),
      ],
      [run("r1", { status: "cancelled", startedAt: stopMs - 6000, endedAt: stopMs })],
    );
    expect(result[1]?.createdAt).toBe(new Date(stopMs).toISOString());
    expect(result[1]?.stopped).toBe(true);
  });

  it("does not stamp a run that failed before any assistant entry onto the previous exchange", () => {
    // r2 (402 insufficient credit) died before the agent saved an entry: the
    // projected history ends with u2. Positional newest-first pairing would
    // stamp r2 onto a1 — the previous, successful exchange — and mislabel it as
    // failed. Window matching excludes the orphan instead.
    const result = applyRunMetadata([
      user("u1", { createdAt: "2026-07-01T10:00:00.000Z" }),
      assistant("a1", { content: "answer", createdAt: "2026-07-01T10:00:05.000Z" }),
      user("u2", { createdAt: "2026-07-01T10:05:00.000Z" }),
    ], [
      run("r2", {
        status: "failed",
        startedAt: Date.parse("2026-07-01T10:05:00.000Z"),
        endedAt: Date.parse("2026-07-01T10:05:02.000Z"),
      }),
      run("r1", {
        status: "completed",
        startedAt: Date.parse("2026-07-01T10:00:00.000Z"),
        endedAt: Date.parse("2026-07-01T10:00:06.000Z"),
      }),
    ]);
    expect(result[1]).toMatchObject({ id: "a1", runId: "r1", status: "complete" });
  });

  it("keeps later exchanges aligned when a middle run left no assistant entry", () => {
    // Exchange 2's run failed without an entry; exchange 3 succeeded afterwards.
    const result = applyRunMetadata([
      user("u1", { createdAt: "2026-07-01T10:00:00.000Z" }),
      assistant("a1", { content: "one", createdAt: "2026-07-01T10:00:05.000Z" }),
      user("u2", { createdAt: "2026-07-01T10:05:00.000Z" }),
      user("u3", { createdAt: "2026-07-01T10:10:00.000Z" }),
      assistant("a3", { content: "three", createdAt: "2026-07-01T10:10:05.000Z" }),
    ], [
      run("r3", {
        status: "completed",
        startedAt: Date.parse("2026-07-01T10:10:00.000Z"),
        endedAt: Date.parse("2026-07-01T10:10:06.000Z"),
      }),
      run("r2", {
        status: "failed",
        startedAt: Date.parse("2026-07-01T10:05:00.000Z"),
        endedAt: Date.parse("2026-07-01T10:05:02.000Z"),
      }),
      run("r1", {
        status: "completed",
        startedAt: Date.parse("2026-07-01T10:00:00.000Z"),
        endedAt: Date.parse("2026-07-01T10:00:06.000Z"),
      }),
    ]);
    expect(result[4]).toMatchObject({ id: "a3", runId: "r3", status: "complete" });
    expect(result[1]).toMatchObject({ id: "a1", runId: "r1", status: "complete" });
  });

  it("falls back to positional pairing when no run window matches any exchange (legacy timestamps)", () => {
    const result = applyRunMetadata([
      user("u1"),
      assistant("a1"),
    ], [
      run("r1", { status: "failed", startedAt: 1000, endedAt: 2000 }),
    ]);
    // a1's 2026 timestamp is outside r1's window, but with no window matches at
    // all the positional fallback must still stamp the run.
    expect(result[1]).toMatchObject({ id: "a1", runId: "r1", status: "failed" });
  });

  it("keeps a completed exchange's own reply time rather than restamping it", () => {
    const replyTs = "2026-07-01T10:00:07.000Z";
    const result = applyRunMetadata(
      [user("u1"), assistant("a1", { content: "answer", createdAt: replyTs })],
      [run("r1", { status: "completed", endedAt: 999 })],
    );
    expect(result[1]?.createdAt).toBe(replyTs);
  });

  it("returns messages unchanged when there are no runs", () => {
    const messages = [user("u1"), assistant("a1")];
    expect(applyRunMetadata(messages, [])).toBe(messages);
  });

  it("leaves the in-flight exchange unstamped when its mid-run partial entry is persisted", () => {
    // The agent's save_callback persists each completed LLM call mid-run, so a
    // reload during streaming surfaces a partial assistant entry for the
    // ACTIVE run's exchange. Exchanges then outnumber settled runs — stamping the
    // newest settled run onto the partial entry misaligns every pairing and
    // defeats streamingBubbleBase's dedup (the frozen partial renders next to
    // the growing live bubble).
    const result = applyRunMetadata([
      user("u1"),
      assistant("a1", { content: "first answer" }),
      user("u2"),
      assistant("a2-partial", { content: "ABC" }),
    ], [
      run("r2", { status: "running", createdAt: 2 }),
      run("r1", { status: "completed", createdAt: 1, modelId: "m-1" }),
    ]);
    // The in-flight exchange keeps no runId — the streaming bubble owns it.
    expect(result[3]?.runId).toBeUndefined();
    // The settled run pairs with its real owner.
    expect(result[1]).toMatchObject({ id: "a1", runId: "r1", modelId: "m-1" });
  });

  it("strips the run id from a mid-run partial stamped by the active run", () => {
    // PR #66's save_closure stamps run_id on EVERY persisted assistant message,
    // so a partial entry saved while the run streams now carries the active
    // run's id. Leaving that stamp makes the live-bubble guards treat the run
    // as settled — after a mid-run reload the streaming bubble is suppressed and
    // the user sees a frozen "complete" partial (the core acceptance scenario:
    // "switch to a running conversation and it keeps growing"). applyRunMetadata
    // must strip the stamp so the entry re-enters in-flight handling.
    const result = applyRunMetadata([
      user("u1"),
      assistant("a1", { content: "first answer" }),
      user("u2"),
      assistant("a2-partial", { content: "ABC", runId: "r2" }),
    ], [
      run("r2", { status: "running", createdAt: 2 }),
      run("r1", { status: "completed", createdAt: 1, modelId: "m-1" }),
    ]);
    // The active run's partial loses its stamp — the streaming bubble owns it.
    expect(result[3]?.runId).toBeUndefined();
    // The settled run still binds its real owner exactly.
    expect(result[1]).toMatchObject({ id: "a1", runId: "r1", modelId: "m-1" });

    // End-to-end: with the stamp stripped, the live bubble for the active run
    // survives a reload instead of being suppressed by the persisted partial.
    const base = streamingBubbleBase(result, "r2", "stream_r2", "ABCDE");
    expect(base).not.toBeNull();
    expect(base!.some(message => message.id === "a2-partial")).toBe(false);
  });

  it("still stamps the newest exchange when the active run has no persisted entry yet", () => {
    // Exchanges == settled runs here: the active run's first LLM call hasn't
    // completed, so its exchange has no entry on disk and the newest assistant
    // exchange belongs to the last settled run.
    const result = applyRunMetadata([
      user("u1"),
      assistant("a1", { content: "answer" }),
      user("u2"),
    ], [
      run("r2", { status: "running", createdAt: 2 }),
      run("r1", { status: "completed", createdAt: 1 }),
    ]);
    expect(result[1]).toMatchObject({ id: "a1", runId: "r1" });
  });

  it("keeps an existing agent-recorded durationMs over the run's wall-clock", () => {
    const result = applyRunMetadata(
      [user("u1"), assistant("a1", { durationMs: 1234 })],
      [run("r1", { startedAt: 1000, endedAt: 9000 })],
    );
    expect(result[1]?.durationMs).toBe(1234);
  });
});

describe("recoverFailedRuns", () => {
  const r1Window = {
    startedAt: Date.parse("2026-07-01T10:00:00.000Z"),
    endedAt: Date.parse("2026-07-01T10:00:06.000Z"),
  };
  const r2Window = {
    startedAt: Date.parse("2026-07-01T10:05:00.000Z"),
    endedAt: Date.parse("2026-07-01T10:05:02.000Z"),
  };

  it("appends a failure bubble for a run that failed before any assistant entry", () => {
    const result = recoverFailedRuns([
      user("u1", { createdAt: "2026-07-01T10:00:00.000Z" }),
      assistant("a1", { content: "answer", createdAt: "2026-07-01T10:00:05.000Z" }),
      user("u2", { createdAt: "2026-07-01T10:05:00.000Z" }),
    ], [
      run("r2", { status: "failed", errorMessage: "API request failed (HTTP 402). insufficient credit", ...r2Window }),
      run("r1", { status: "completed", ...r1Window }),
    ]);
    expect(result).toHaveLength(4);
    const bubble = result[3]!;
    expect(bubble).toMatchObject({
      id: "failed_r2",
      role: "assistant",
      runId: "r2",
      status: "failed",
      createdAt: new Date(r2Window.endedAt).toISOString(),
    });
    expect(bubble.content.trim()).not.toBe("");
  });

  it("inserts the bubble at its chronological position when a later exchange succeeded", () => {
    const result = recoverFailedRuns([
      user("u1", { createdAt: "2026-07-01T10:00:00.000Z" }),
      assistant("a1", { content: "one", createdAt: "2026-07-01T10:00:05.000Z", runId: "r1" }),
      user("u2", { createdAt: "2026-07-01T10:05:00.000Z" }),
      user("u3", { createdAt: "2026-07-01T10:10:00.000Z" }),
      assistant("a3", { content: "three", createdAt: "2026-07-01T10:10:05.000Z", runId: "r3" }),
    ], [
      run("r3", {
        status: "completed",
        startedAt: Date.parse("2026-07-01T10:10:00.000Z"),
        endedAt: Date.parse("2026-07-01T10:10:06.000Z"),
      }),
      run("r2", { status: "failed", errorMessage: "boom", ...r2Window }),
      run("r1", { status: "completed", ...r1Window }),
    ]);
    expect(result.map(m => m.id)).toEqual(["u1", "a1", "u2", "failed_r2", "u3", "a3"]);
  });

  it("appends a failure bubble when the FIRST run of the session failed (no assistant entry at all)", () => {
    // The "prompt acknowledgement omitted run_id" case: the run failed before
    // the agent saved anything — the user's message is the only exchange inside the
    // run's window, so the trust guard must not require an assistant reply.
    const result = recoverFailedRuns([
      user("u1", { createdAt: "2026-07-01T10:00:00.000Z" }),
    ], [
      run("r1", { status: "failed", errorMessage: "Future Agent prompt acknowledgement omitted run_id.", ...r1Window }),
    ]);
    expect(result).toHaveLength(2);
    expect(result[1]).toMatchObject({
      id: "failed_r1",
      role: "assistant",
      runId: "r1",
      status: "failed",
    });
    expect(result[1]!.content.trim()).not.toBe("");
  });

  it("leaves messages unchanged when the failed run already owns a projected exchange", () => {
    const messages = [
      user("u1", { createdAt: "2026-07-01T10:00:00.000Z" }),
      assistant("a1", { content: "partial", createdAt: "2026-07-01T10:00:05.000Z", runId: "r1", status: "failed" }),
    ];
    const result = recoverFailedRuns(messages, [
      run("r1", { status: "failed", errorMessage: "boom", ...r1Window }),
    ]);
    expect(result).toBe(messages);
  });

  it("ignores failed runs without a usable start time (legacy rows)", () => {
    const messages = [user("u1"), assistant("a1", { content: "answer" })];
    const result = recoverFailedRuns(messages, [
      run("r1", { status: "failed", errorMessage: "boom" }),
    ]);
    expect(result).toBe(messages);
  });

  it("ignores completed and cancelled runs", () => {
    const messages = [user("u1", { createdAt: "2026-07-01T10:00:00.000Z" })];
    const result = recoverFailedRuns(messages, [
      run("r2", { status: "completed", ...r2Window }),
      run("r1", { status: "cancelled", ...r1Window }),
    ]);
    expect(result).toBe(messages);
  });

  it("skips recovery entirely when no run window matches any exchange (legacy timestamps)", () => {
    // Legacy session entries have no real timestamps (the agent backfills
    // load-time `now`), so every exchange sits after every run — window matching is
    // meaningless and bubbles would land at the wrong end of history.
    const messages = [
      user("u1", { createdAt: "2026-07-20T09:00:00.000Z" }),
      assistant("a1", { content: "answer", createdAt: "2026-07-20T09:00:01.000Z" }),
    ];
    const result = recoverFailedRuns(messages, [
      run("r1", { status: "failed", errorMessage: "boom", ...r1Window }),
    ]);
    expect(result).toBe(messages);
  });
});

describe("applyRecoveredEvents", () => {
  it("fills an empty aborted exchange with the streamed partial text", () => {
    const messages = [
      user("u1"),
      assistant("a1", { content: "", runId: "r1", stopped: true }),
    ];
    const result = applyRecoveredEvents(
      messages,
      new Map([["r1", events([["text_chunk", { text: "half a poem" }]])]]),
    );
    expect(result[1]?.content).toBe("half a poem");
    expect(result[1]?.segments).toBeDefined();
    // Recovery doesn't touch the stopped marker the run metadata set.
    expect(result[1]?.stopped).toBe(true);
  });

  it("leaves an exchange that already has content untouched", () => {
    const messages = [user("u1"), assistant("a1", { content: "final answer", runId: "r1" })];
    const result = applyRecoveredEvents(
      messages,
      new Map([["r1", events([["text_chunk", { text: "something else" }]])]]),
    );
    expect(result[1]?.content).toBe("final answer");
  });

  it("leaves an exchange with segments untouched (tool activity already projected)", () => {
    const withSegments = assistant("a1", {
      content: "",
      runId: "r1",
      segments: [{ id: "s", kind: "text", text: "kept" }],
    });
    const result = applyRecoveredEvents(
      [user("u1"), withSegments],
      new Map([["r1", events([["text_chunk", { text: "ignored" }]])]]),
    );
    expect(result[1]?.segments).toEqual([{ id: "s", kind: "text", text: "kept" }]);
    expect(result[1]?.content).toBe("");
  });

  it("leaves an empty exchange untouched when its events carried no text", () => {
    const messages = [user("u1"), assistant("a1", { content: "", runId: "r1" })];
    const result = applyRecoveredEvents(messages, new Map([["r1", events([])]]));
    expect(result[1]?.content).toBe("");
  });

  it("ignores exchanges without a runId", () => {
    const messages = [user("u1"), assistant("a1", { content: "" })];
    const result = applyRecoveredEvents(messages, new Map([["r1", events([["text_chunk", { text: "x" }]])]]));
    expect(result[1]?.content).toBe("");
  });
});

describe("deriveRenderFields", () => {
  it("prefers event-derived content and segments when the events carried text", () => {
    const result = deriveRenderFields(
      events([["text_chunk", { text: "Hello" }]]),
      "fallback",
    );
    expect(result.content).toBe("Hello");
    expect(result.segments).toBeDefined();
  });

  it("falls back to the stored reply when events carried no assistant text", () => {
    const result = deriveRenderFields(events([]), "stored reply");
    expect(result.content).toBe("stored reply");
    expect(result.segments).toBeUndefined();
  });
});

describe("streamingBubbleBase", () => {
  const RUN = "r1";
  const BUBBLE = `stream_${RUN}`;

  it("returns null when a settled run's persisted message already carries the runId", () => {
    const current = [user("u1"), assistant("a1", { runId: RUN, content: "done" })];
    expect(streamingBubbleBase(current, RUN, BUBBLE, "done")).toBeNull();
  });

  it("ignores the bubble itself when checking the runId guard", () => {
    const current = [user("u1"), assistant(BUBBLE, { runId: RUN, content: "live" })];
    const base = streamingBubbleBase(current, RUN, BUBBLE, "live");
    expect(base).toBe(current);
  });

  it("drops the mid-run persisted entry that duplicates the live projection (short snapshot)", () => {
    // The reported bug: persisted "Hello wor" (< 80 chars) failed the old
    // includes(content[:80]) guard, so the bubble was inserted alongside it.
    const persisted = assistant("a-partial", { content: "Hello wor" });
    const current = [user("u1"), assistant("a1", { content: "earlier reply" }), user("u2"), persisted];
    const base = streamingBubbleBase(current, RUN, BUBBLE, "Hello world, how are you?");
    expect(base?.some(m => m.id === "a-partial")).toBe(false);
    expect(base?.some(m => m.id === "a1")).toBe(true);
    expect(base?.some(m => m.id === "u2")).toBe(true);
  });

  it("drops the persisted entry of a multi-call exchange (finalText is the last call's text)", () => {
    // Two LLM calls persisted separately; entriesToMessages keeps only the last
    // call's text as content, which is a substring (not prefix) of the live projection.
    const persisted = assistant("a-partial", { content: "second call text" });
    const current = [user("u1"), persisted];
    const base = streamingBubbleBase(current, RUN, BUBBLE, "first call text second call text and more");
    expect(base?.some(m => m.id === "a-partial")).toBe(false);
  });

  it("keeps an earlier exchange's reply even when the new stream starts alike", () => {
    const earlier = assistant("a1", { content: "OK" });
    const current = [user("u1"), earlier, user("u2")];
    const base = streamingBubbleBase(current, RUN, BUBBLE, "OK, let me help with that");
    expect(base?.some(m => m.id === "a1")).toBe(true);
  });

  it("returns null when another exchange's persisted reply already covers the live text", () => {
    const earlier = assistant("a1", { content: "Hello world, how are you today?" });
    const current = [user("u1"), earlier, user("u2")];
    // u2's exchange has no persisted entry; u1's reply happens to contain the head.
    expect(streamingBubbleBase(current, RUN, BUBBLE, "Hello world, how")).toBeNull();
  });

  it("does not prefix-suppress an earlier exchange that carries a canonical runId", () => {
    // N9 / 5.4E: the prefix heuristic is legacy-only. A settled canonical exchange
    // (runId present) whose reply shares a head with a repeated question's live
    // text ("continue" / "yes" / deterministic output) must NOT kill the new
    // bubble — that prefix match was the mis-kill. The runId guard at the top
    // already covers the settled-reload race for canonical data.
    const earlier = assistant("a1", { runId: "r-prev", content: "Hello world, how are you today?" });
    const current = [user("u1"), earlier, user("u2")];
    const base = streamingBubbleBase(current, RUN, BUBBLE, "Hello world, how");
    expect(base).not.toBeNull();
    expect(base?.some(m => m.id === "a1")).toBe(true);
  });

  it("returns the list unchanged when the in-flight exchange has no persisted entry", () => {
    const current = [user("u1"), assistant("a1", { content: "previous reply" }), user("u2")];
    const base = streamingBubbleBase(current, RUN, BUBBLE, "brand new stream");
    expect(base).toBe(current);
  });

  it("returns the list unchanged when live content is empty (thinking-only so far)", () => {
    const persisted = assistant("a-partial", { content: "partial text" });
    const current = [user("u1"), persisted];
    // Same-exchange persisted entry is always dropped — the bubble will fill in
    // as events arrive.
    const base = streamingBubbleBase(current, RUN, BUBBLE, "");
    expect(base?.some(m => m.id === "a-partial")).toBe(false);
  });

  it("drops the same-exchange persisted entry even when it carries another run's id", () => {
    // Defense in depth: a runId that is NOT the active run's (e.g. a stale
    // misaligned stamp) must not shield the in-flight exchange's mid-run snapshot
    // from the dedup — the bubble replaces it either way.
    const persisted = assistant("a-partial", { content: "ABC", runId: "r-old-settled" });
    const current = [user("u1"), assistant("a1", { content: "earlier reply" }), user("u2"), persisted];
    const base = streamingBubbleBase(current, RUN, BUBBLE, "ABC");
    expect(base?.some(m => m.id === "a-partial")).toBe(false);
    expect(base?.some(m => m.id === "a1")).toBe(true);
  });

  it("drops the same-exchange snapshot that has no text yet (thinking/tools-only)", () => {
    // Mid-run the exchange may have produced only thinking + tool calls: the
    // persisted entry's `content` is empty but its segments still render, so
    // leaving it in place duplicates the live bubble's thinking/activity.
    const persisted = assistant("a-partial", {
      content: "",
      segments: [
        { id: "s1", kind: "thinking", text: "Let me think…" },
        { id: "s2", kind: "activity", item: { id: "t1", kind: "read", status: "completed", target: "/a.ts" } },
      ],
    });
    const current = [user("u1"), assistant("a1", { content: "earlier reply" }), user("u2"), persisted];
    const base = streamingBubbleBase(current, RUN, BUBBLE, "");
    expect(base?.some(m => m.id === "a-partial")).toBe(false);
    expect(base?.some(m => m.id === "a1")).toBe(true);
  });

  it("keeps a compaction divider sitting at the head of the in-flight exchange", () => {
    // A divider is a marker, not a reply snapshot — the bubble must be
    // appended AFTER it, never replace it.
    const divider = assistant("div", { content: "", segments: [{ id: "s", kind: "compaction" }] });
    const current = [user("u1"), divider];
    const base = streamingBubbleBase(current, RUN, BUBBLE, "live text");
    expect(base?.some(m => m.id === "div")).toBe(true);
  });
});

describe("mergeStreamingPreview", () => {
  it("replaces a persisted mid-run snapshot when switching back to an active thread", () => {
    const previous = assistant("a1", { content: "earlier reply", runId: "r-old" });
    const partial = assistant("a-partial", {
      content: "好的，根据李白《静夜思》诗意来创作——",
    });
    const preview = assistant("stream_r1", {
      content: "好的，根据李白《静夜思》诗意来创作——",
      runId: "r1",
      status: "streaming",
    });

    const result = mergeStreamingPreview([
      user("u1"),
      previous,
      user("u2"),
      partial,
    ], preview);

    expect(result.map(message => message.id)).toEqual(["u1", "a1", "u2", "stream_r1"]);
    expect(result.filter(message => message.role === "assistant" && message.content === preview.content)).toHaveLength(1);
  });

  it("does not resurrect a preview after the run's persisted reply has settled", () => {
    const settled = assistant("a2", { content: "done", runId: "r1" });
    const preview = assistant("stream_r1", {
      content: "done",
      runId: "r1",
      status: "streaming",
    });

    const current = [user("u1"), settled];
    expect(mergeStreamingPreview(current, preview)).toBe(current);
  });
});
