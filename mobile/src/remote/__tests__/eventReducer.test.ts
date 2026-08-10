import {
  applyStreamEvent,
  emptyTimeline,
  normalizeReplayEvents,
  stripRunItems,
  timelineFromEntries,
  timelineFromHistory,
  timelineFromProjection,
} from "../eventReducer";

describe("history reducer", () => {
  test("skips tool-call-only messages whose content is omitted on the wire", () => {
    const timeline = timelineFromHistory([
      { role: "user", content: "hi" },
      // Assistant tool-call messages serialize without a content field.
      { role: "assistant" },
      { role: "tool", content: null },
      { role: "assistant", content: "done" },
    ]);
    expect(timeline.items).toEqual([
      expect.objectContaining({ kind: "message", role: "user", text: "hi" }),
      expect.objectContaining({ kind: "message", role: "assistant", text: "done" }),
    ]);
  });
});

describe("entry reducer", () => {
  test("projects user/assistant entries and carries attachments", () => {
    const timeline = timelineFromEntries([
      {
        id: "e1",
        role: "user",
        content: "check this",
        meta: {
          attachments: [
            { path: "/tmp/a.png", name: "a.png", kind: "image" },
            { path: "/tmp/b.pdf", name: "b.pdf", kind: "file" },
          ],
        },
      },
      { id: "e2", role: "assistant", content: "looks good" },
      { id: "e3", role: "tool", content: "tool output" },
    ]);
    expect(timeline.items).toEqual([
      expect.objectContaining({
        kind: "message",
        role: "user",
        text: "check this",
        attachments: [
          { path: "/tmp/a.png", name: "a.png", kind: "image" },
          { path: "/tmp/b.pdf", name: "b.pdf", kind: "file" },
        ],
      }),
      expect.objectContaining({ kind: "message", role: "assistant", text: "looks good" }),
    ]);
  });

  test("keeps attachment-only user entries and drops malformed attachments", () => {
    const timeline = timelineFromEntries([
      {
        id: "e1",
        role: "user",
        content: "",
        meta: {
          attachments: [{ path: "/tmp/a.png", name: "a.png" }, { name: "no-path" } as never],
        },
      },
      { id: "e2", role: "user", content: "" },
    ]);
    expect(timeline.items).toHaveLength(1);
    expect(timeline.items[0]).toMatchObject({
      attachments: [{ path: "/tmp/a.png", name: "a.png" }],
    });
  });

  test("projects thinking and tool rows above the merged reply per exchange", () => {
    // History must read like the live transcript (desktop entryProjection
    // parity): user bubble, then the run's thinking/tool rows, then the merged
    // reply with its run stats.
    const timeline = timelineFromEntries([
      { id: "u1", role: "user", content: "check this" },
      {
        id: "a1",
        role: "assistant",
        content: "interim analysis",
        thinking: "reasoning…",
        tool_calls: [{ id: "call_0", function: { name: "read", arguments: { path: "/tmp/x" } } }],
      },
      { id: "t1", role: "tool", content: "ok" },
      {
        id: "a2",
        role: "assistant",
        content: "done",
        meta: { run_id: "run-9" },
        output_tokens: 12,
        duration_ms: 3400,
      },
      { id: "u2", role: "user", content: "thanks" },
    ]);
    expect(timeline.items.map(item => item.kind)).toEqual([
      "message",
      "thinking",
      "tool",
      "message",
      "message",
    ]);
    expect(timeline.items[1]).toMatchObject({
      kind: "thinking",
      text: "reasoning…",
      complete: true,
    });
    expect(timeline.items[2]).toMatchObject({
      kind: "tool",
      name: "read",
      complete: true,
      detail: "/tmp/x",
    });
    const reply = timeline.items[3];
    if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
    expect(reply).toMatchObject({
      role: "assistant",
      text: "interim analysis\n\ndone",
      runId: "run-9",
      durationMs: 3400,
      outputTokens: 12,
    });
    // A reply-less run (empty assistant entry) renders nothing extra.
    const divider = timelineFromEntries([
      { id: "u3", role: "user", content: "next" },
      { id: "a3", role: "assistant", content: "" },
    ]);
    expect(divider.items.map(item => item.kind)).toEqual(["message"]);
  });
});

describe("projection reducer", () => {
  test("folds a run projection into a timeline like the live stream", () => {
    // The projection replaces the run's partial persisted entries wholesale,
    // so folding it reproduces the same transcript as the live events.
    const timeline = timelineFromProjection([
      { type: "agent_start", data: "{}", runId: "run-1", idx: 0 },
      {
        type: "tool_start",
        data: JSON.stringify({ tool_id: "t1", tool_name: "read" }),
        runId: "run-1",
        idx: 1,
      },
      {
        type: "tool_end",
        data: JSON.stringify({ tool_id: "t1" }),
        runId: "run-1",
        idx: 2,
      },
      { type: "text_chunk", data: JSON.stringify({ text: "answer" }), runId: "run-1", idx: 3 },
      { type: "agent_end", data: "{}", runId: "run-1", idx: 4 },
    ]);
    expect(timeline.items.map(item => item.kind)).toEqual(["tool", "message"]);
    const reply = timeline.items.find(item => item.kind === "message");
    expect(reply).toMatchObject({ kind: "message", role: "assistant", text: "answer" });
    expect(timeline.streaming).toBe(false);
  });

  test("empty projection is an empty timeline", () => {
    const timeline = timelineFromProjection([]);
    expect(timeline.items).toEqual([]);
    expect(timeline.streaming).toBe(false);
  });

  test("stripRunItems drops a run's items but keeps user bubbles and other runs", () => {
    const base = {
      items: [
        { id: "u1", kind: "message" as const, role: "user" as const, text: "hi" },
        {
          id: "a1",
          kind: "message" as const,
          role: "assistant" as const,
          text: "run-1 reply",
          runId: "run-1",
        },
        {
          id: "t1",
          kind: "tool" as const,
          toolId: "t1",
          name: "read" as const,
          complete: true,
          runId: "run-1",
        },
        {
          id: "a2",
          kind: "message" as const,
          role: "assistant" as const,
          text: "run-2 reply",
          runId: "run-2",
        },
      ],
      seenEvents: new Set<string>(),
      currentRunId: null,
      streaming: false,
    };
    const stripped = stripRunItems(base, "run-1");
    expect(stripped.items.map(item => item.id)).toEqual(["u1", "a2"]);
  });
});

describe("user message mirror", () => {
  test("user_message from another device appends a user bubble", () => {
    const state = applyStreamEvent(emptyTimeline(), {
      type: "user_message",
      data: JSON.stringify({ text: "check this" }),
    });
    expect(state.items).toHaveLength(1);
    expect(state.items[0]).toMatchObject({ kind: "message", role: "user", text: "check this" });
  });

  test("a repeat of the last user bubble's text is deduped (own optimistic send)", () => {
    const first = applyStreamEvent(emptyTimeline(), {
      type: "user_message",
      data: JSON.stringify({ text: "check this" }),
    });
    const repeat = applyStreamEvent(first, {
      type: "user_message",
      data: JSON.stringify({ text: "check this" }),
    });
    expect(repeat.items).toHaveLength(1);
    // A genuinely new prompt still lands.
    const next = applyStreamEvent(repeat, {
      type: "user_message",
      data: JSON.stringify({ text: "and this" }),
    });
    expect(next.items.map(item => item.kind === "message" && item.text)).toEqual([
      "check this",
      "and this",
    ]);
  });
});

describe("replay event normalization", () => {
  test("maps snake_case run_id to the camelCase StreamEvent shape", () => {
    const events = normalizeReplayEvents([
      { type: "agent_start", data: "{}", run_id: "run-1", idx: 0 },
      { type: "text_chunk", data: '{"text":"hi"}', run_id: "run-1", idx: 1 },
    ]);
    expect(events).toEqual([
      { type: "agent_start", data: "{}", runId: "run-1", idx: 0 },
      { type: "text_chunk", data: '{"text":"hi"}', runId: "run-1", idx: 1 },
    ]);
  });

  test("drops malformed entries and tolerates missing run_id/idx", () => {
    const events = normalizeReplayEvents([
      null as never,
      { type: "agent_start", data: "{}" },
      42 as never,
    ]);
    expect(events).toEqual([{ type: "agent_start", data: "{}", runId: "", idx: undefined }]);
  });

  test("empty or undefined input yields no events", () => {
    expect(normalizeReplayEvents(undefined)).toEqual([]);
    expect(normalizeReplayEvents(null)).toEqual([]);
    expect(normalizeReplayEvents([])).toEqual([]);
  });
});

describe("stream event reducer", () => {
  test("deduplicates and appends text chunks by run", () => {
    const first = applyStreamEvent(emptyTimeline(), {
      type: "text_chunk",
      data: JSON.stringify({ text: "hello" }),
      runId: "run-1",
      idx: 1,
    });
    const duplicate = applyStreamEvent(first, {
      type: "text_chunk",
      data: JSON.stringify({ text: "hello" }),
      runId: "run-1",
      idx: 1,
    });
    const second = applyStreamEvent(duplicate, {
      type: "text_chunk",
      data: JSON.stringify({ text: " world" }),
      runId: "run-1",
      idx: 2,
    });
    expect(second.items).toHaveLength(1);
    expect(second.items[0]).toMatchObject({ kind: "message", text: "hello world" });
  });

  test("tracks streaming and approval state", () => {
    const started = applyStreamEvent(emptyTimeline(), {
      type: "agent_start",
      data: "{}",
      runId: "run-1",
      idx: 0,
    });
    const approval = applyStreamEvent(started, {
      type: "approval_request",
      data: JSON.stringify({ approval_request_id: "approval-1", title: "Write file" }),
      runId: "run-1",
      idx: 1,
    });
    const ended = applyStreamEvent(approval, {
      type: "agent_end",
      data: "{}",
      runId: "run-1",
      idx: 2,
    });
    expect(started.streaming).toBe(true);
    expect(approval.items.find(item => item.kind === "approval")).toMatchObject({
      kind: "approval",
    });
    expect(ended.streaming).toBe(false);
  });

  test("marks the assistant streaming while running and settles it with a duration on end", () => {
    const started = applyStreamEvent(emptyTimeline(), {
      type: "agent_start",
      data: "{}",
      runId: "run-1",
      idx: 0,
    });
    const placeholder = started.items.find(
      item => item.kind === "message" && item.role === "assistant",
    );
    if (!placeholder || placeholder.kind !== "message")
      throw new Error("streaming assistant placeholder was not created");
    expect(placeholder.streaming).toBe(true);
    expect(typeof placeholder.startedAt).toBe("number");

    const text = applyStreamEvent(started, {
      type: "text_chunk",
      data: JSON.stringify({ text: "done" }),
      runId: "run-1",
      idx: 1,
    });
    const streaming = text.items.find(item => item.kind === "message" && item.role === "assistant");
    expect(streaming && streaming.kind === "message" && streaming.streaming).toBe(true);

    const ended = applyStreamEvent(text, {
      type: "agent_end",
      data: "{}",
      runId: "run-1",
      idx: 2,
    });
    const settled = ended.items.find(item => item.kind === "message" && item.role === "assistant");
    if (!settled || settled.kind !== "message") throw new Error("assistant message missing");
    expect(settled.streaming).toBe(false);
    expect(settled.durationMs).toEqual(expect.any(Number));
  });

  test("anchors the live timer to the agent_start started_at_ms, not the receipt time", () => {
    const runStart = 1_750_000_000_000; // fixed epoch-ms, far from Date.now()
    const state = applyStreamEvent(emptyTimeline(), {
      type: "agent_start",
      data: JSON.stringify({ started_at_ms: runStart }),
      runId: "run-1",
      idx: 0,
    });
    const placeholder = state.items.find(
      item => item.kind === "message" && item.role === "assistant",
    );
    if (!placeholder || placeholder.kind !== "message")
      throw new Error("streaming assistant placeholder was not created");
    expect(placeholder.startedAt).toBe(runStart);
  });

  test("creates the streaming assistant host on thinking_delta when agent_start was missed", () => {
    // Late join mid-think: the run's event-ring tail starts with thinking
    // deltas, so the generating indicator still needs a host bubble.
    const thinking = applyStreamEvent(emptyTimeline(), {
      type: "thinking_delta",
      data: JSON.stringify({ text: "reasoning…" }),
      runId: "run-1",
      idx: 7,
    });
    const host = thinking.items.find(item => item.kind === "message" && item.role === "assistant");
    if (!host || host.kind !== "message") throw new Error("assistant host bubble missing");
    expect(host.streaming).toBe(true);
    expect(host.text).toBe("");
    // Chronological order: reasoning first, the answer bubble below it.
    expect(thinking.items.map(item => item.kind)).toEqual(["thinking", "message"]);

    // The reply text merges into that same bubble — no duplicate assistant row.
    const text = applyStreamEvent(thinking, {
      type: "text_chunk",
      data: JSON.stringify({ text: "answer" }),
      runId: "run-1",
      idx: 8,
    });
    const assistants = text.items.filter(
      item => item.kind === "message" && item.role === "assistant",
    );
    expect(assistants).toHaveLength(1);
    expect(assistants[0]).toMatchObject({ text: "answer", streaming: true });
    expect(text.items.map(item => item.kind)).toEqual(["thinking", "message"]);
  });

  test("keeps thinking and tool rows above the answer bubble in event order", () => {
    // From-start flow: the placeholder exists from agent_start, so secondary
    // items must insert *before* it — otherwise the answer would render above
    // its own reasoning (desktop renders inline, chronologically).
    let state = applyStreamEvent(emptyTimeline(), {
      type: "agent_start",
      data: "{}",
      runId: "run-1",
      idx: 0,
    });
    state = applyStreamEvent(state, {
      type: "thinking_delta",
      data: JSON.stringify({ text: "reasoning…" }),
      runId: "run-1",
      idx: 1,
    });
    state = applyStreamEvent(state, {
      type: "tool_start",
      data: JSON.stringify({ tool_id: "t1", tool_name: "read" }),
      runId: "run-1",
      idx: 2,
    });
    state = applyStreamEvent(state, {
      type: "text_chunk",
      data: JSON.stringify({ text: "answer" }),
      runId: "run-1",
      idx: 3,
    });
    expect(state.items.map(item => item.kind)).toEqual(["thinking", "tool", "message"]);
    const answer = state.items[2];
    if (!answer || answer.kind !== "message") throw new Error("assistant message missing");
    expect(answer).toMatchObject({ text: "answer", streaming: true });
  });

  test("keeps tool rows above the reply when text streams before the first tool call", () => {
    // Regression: a model may stream an interim remark ahead of its first tool
    // call; the merged reply bubble must still settle below the run's tool
    // rows, not above them (the desktop shows the final answer last).
    let state = applyStreamEvent(emptyTimeline(), {
      type: "agent_start",
      data: "{}",
      runId: "run-1",
      idx: 0,
    });
    state = applyStreamEvent(state, {
      type: "text_chunk",
      data: JSON.stringify({ text: "interim " }),
      runId: "run-1",
      idx: 1,
    });
    state = applyStreamEvent(state, {
      type: "tool_start",
      data: JSON.stringify({ tool_id: "t1", tool_name: "read" }),
      runId: "run-1",
      idx: 2,
    });
    state = applyStreamEvent(state, {
      type: "tool_end",
      data: JSON.stringify({ tool_id: "t1" }),
      runId: "run-1",
      idx: 3,
    });
    state = applyStreamEvent(state, {
      type: "text_chunk",
      data: JSON.stringify({ text: "answer" }),
      runId: "run-1",
      idx: 4,
    });
    expect(state.items.map(item => item.kind)).toEqual(["tool", "message"]);
    const answer = state.items[1];
    if (!answer || answer.kind !== "message") throw new Error("assistant message missing");
    expect(answer.text).toBe("interim answer");
  });

  test("settle prefers the authoritative agent_end totals over partial late-join stats", () => {
    // Regression: a client that joined seconds before the end used to stamp a
    // receipt-clock duration and keep only the tail's accumulated usage.
    let state = applyStreamEvent(emptyTimeline(), {
      type: "text_chunk",
      data: JSON.stringify({ text: "tail of the reply" }),
      runId: "run-1",
      idx: 1998,
    });
    state = applyStreamEvent(state, {
      type: "usage",
      data: JSON.stringify({ usage: { completion_tokens: 273 } }),
      runId: "run-1",
      idx: 1999,
    });
    state = applyStreamEvent(state, {
      type: "agent_end",
      data: JSON.stringify({ duration_ms: 85_000, usage: { output_tokens: 2965 } }),
      runId: "run-1",
      idx: 2000,
    });
    const settled = state.items.find(item => item.kind === "message" && item.role === "assistant");
    if (!settled || settled.kind !== "message") throw new Error("assistant message missing");
    expect(settled.streaming).toBe(false);
    expect(settled.durationMs).toBe(85_000);
    expect(settled.outputTokens).toBe(2965);
  });

  test("tool rows carry the call target from tool_args (object or JSON string)", () => {
    let state = applyStreamEvent(emptyTimeline(), {
      type: "tool_start",
      data: JSON.stringify({ tool_id: "t1", tool_name: "shell", tool_args: { command: "ls -la" } }),
      runId: "run-1",
      idx: 0,
    });
    expect(state.items[0]).toMatchObject({ kind: "tool", detail: "ls -la" });
    state = applyStreamEvent(state, {
      type: "tool_start",
      data: JSON.stringify({ tool_id: "t2", tool_name: "read", tool_args: '{"path":"/tmp/x"}' }),
      runId: "run-1",
      idx: 1,
    });
    expect(state.items[1]).toMatchObject({ kind: "tool", detail: "/tmp/x" });
  });

  test("agent_end closes an unfinished thinking row", () => {
    let state = applyStreamEvent(emptyTimeline(), {
      type: "thinking_delta",
      data: JSON.stringify({ text: "hmm" }),
      runId: "run-1",
      idx: 0,
    });
    state = applyStreamEvent(state, { type: "agent_end", data: "{}", runId: "run-1", idx: 1 });
    const thinking = state.items.find(item => item.kind === "thinking");
    if (!thinking || thinking.kind !== "thinking") throw new Error("thinking row missing");
    expect(thinking.complete).toBe(true);
  });

  test("settle falls back to receipt-clock duration and accumulated usage on older agents", () => {
    let state = applyStreamEvent(emptyTimeline(), {
      type: "agent_start",
      data: "{}",
      runId: "run-1",
      idx: 0,
    });
    state = applyStreamEvent(state, {
      type: "usage",
      data: JSON.stringify({ usage: { completion_tokens: 42 } }),
      runId: "run-1",
      idx: 1,
    });
    state = applyStreamEvent(state, {
      type: "agent_end",
      data: "{}",
      runId: "run-1",
      idx: 2,
    });
    const settled = state.items.find(item => item.kind === "message" && item.role === "assistant");
    if (!settled || settled.kind !== "message") throw new Error("assistant message missing");
    expect(settled.durationMs).toEqual(expect.any(Number));
    expect(settled.outputTokens).toBe(42);
  });
});
