import {
  applyStreamEvent,
  appendUserMessage,
  commitAcknowledgedUserMessage,
  dropSupersededCompactionDividers,
  emptyTimeline,
  foldLiveCompactionPlaceholdersIntoHistory,
  markApprovalDecision,
  mergeHistoryAttachments,
  normalizeReplayEvents,
  stripRunItems,
  timelineFromEntries,
  timelineFromHistory,
  timelineFromProjection,
} from "../timeline";
import { messageToItems } from "../projection";
import type { HistoryEntry } from "../types";

describe("history reducer", () => {
  test("keeps optimistic attachment chips for an attachment-only prompt", () => {
    const timeline = appendUserMessage(emptyTimeline(), "", [
      { path: "file:///photo.jpg", name: "photo.jpg", kind: "image" },
    ]);
    expect(timeline.items[0]).toMatchObject({
      kind: "message",
      role: "user",
      text: "",
      attachments: [{ name: "photo.jpg", kind: "image" }],
    });
  });

  test("skips tool-call-only messages whose content is omitted on the wire", () => {
    const timeline = timelineFromHistory([
      { role: "user", blocks: [{ kind: "text", text: "hi" }] },
      // Assistant tool-call messages serialize without a content field.
      { role: "assistant", blocks: [] },
      { role: "tool", blocks: [] },
      { role: "assistant", blocks: [{ kind: "text", text: "done" }] },
    ]);
    expect(timeline.items).toEqual([
      expect.objectContaining({ kind: "message", role: "user", text: "hi" }),
      expect.objectContaining({ kind: "message", role: "assistant", text: "done" }),
    ]);
  });

  test("a message that is neither a user bubble nor an assistant reply renders nothing", () => {
    // System/tool rows travel in the same projection envelope; they have no
    // bubble in this UI and must not appear as an empty one.
    expect(messageToItems({ id: "sys", role: "system", content: "you are an agent" } as never))
      .toEqual([]);
    // An assistant row with no text, no segments and no outcome carries nothing
    // to say: not even a placeholder bubble.
    expect(messageToItems({ id: "a", role: "assistant", content: "", segments: [] } as never))
      .toEqual([]);
    // …but an outcome without text still has to be visible (a failed or stopped
    // turn is exactly the case the user needs to see).
    expect(messageToItems({ id: "a", role: "assistant", content: "", segments: [], stopped: true } as never))
      .toHaveLength(1);
    expect(messageToItems({ id: "a", role: "assistant", content: "", segments: [], durationMs: 12 } as never))
      .toHaveLength(1);
  });

  test("a live bubble that already carries its attachments keeps them", () => {
    const live = appendUserMessage(emptyTimeline(), "look", [
      { path: "file:///live.jpg", name: "live.jpg", kind: "image" },
    ]);
    const durable = appendUserMessage(emptyTimeline(), "look", [
      { path: "file:///durable.jpg", name: "durable.jpg", kind: "file" },
    ]);
    const merged = mergeHistoryAttachments(live, durable);
    // The live row's chips came from the composer the user actually used;
    // replacing them with the durable copy would swap a photo for a file.
    expect(merged.items[0]).toMatchObject({ attachments: [{ name: "live.jpg" }] });
  });

  test("a settled live placeholder for a durable checkpoint is folded away", () => {
    const divider = (id: string, status: "running" | "completed") => ({
      id,
      kind: "message" as const,
      role: "assistant" as const,
      text: "",
      segments: [{ id: `seg_${id}`, kind: "compaction" as const, checkpointId: "cp-1", status }],
    });
    // The live placeholder never got its terminal, so it still carries the
    // operation id while the durable divider carries the checkpoint id. When
    // both rows exist, the placeholder must go — including any copy of it that
    // the durable window still holds.
    const live = [divider("compaction:op-1", "completed")];
    const history = [divider("compaction:op-1", "completed"), divider("m_cp-1", "completed")];
    const folded = foldLiveCompactionPlaceholdersIntoHistory(history, live);
    expect(folded.live).toEqual([]);
    expect(folded.history.map(item => item.id)).toEqual(["m_cp-1"]);
    // A placeholder whose terminal is still running is not an alias: the durable
    // copy of that same row stays, because only a settled placeholder proves the
    // running marker is stale.
    const running = foldLiveCompactionPlaceholdersIntoHistory(
      [divider("compaction:op-2", "running")],
      [divider("compaction:op-2", "running")],
    );
    expect(running.live).toEqual([]);
    expect(running.history.map(item => item.id)).toEqual(["compaction:op-2"]);
  });
});

describe("entry reducer", () => {
  test("enriches text-only live user messages from durable attachments", () => {
    const live = appendUserMessage(emptyTimeline(), "check this");
    const durable = timelineFromEntries([
      {
        id: "e1",
        kind: "user",
        role: "user",
        createdAtMs: 0,
        blocks: [{ kind: "text", text: "check this" }],
        metadata: { attachments: [{ path: "/tmp/a.png", name: "a.png", kind: "image" }] },
      },
    ]);

    expect(mergeHistoryAttachments(live, durable).items[0]).toMatchObject({
      attachments: [{ path: "/tmp/a.png", name: "a.png", kind: "image" }],
    });
  });

  test("matches repeated live prompts to the latest durable attachment", () => {
    const live = appendUserMessage(emptyTimeline(), "same prompt");
    const durable = timelineFromEntries([
      {
        id: "old",
        kind: "user",
        role: "user",
        createdAtMs: 0,
        blocks: [{ kind: "text", text: "same prompt" }],
        metadata: { attachments: [{ path: "/tmp/old.png", name: "old.png", kind: "image" }] },
      },
      {
        id: "new",
        kind: "user",
        role: "user",
        createdAtMs: 0,
        blocks: [{ kind: "text", text: "same prompt" }],
        metadata: { attachments: [{ path: "/tmp/new.png", name: "new.png", kind: "image" }] },
      },
    ]);

    expect(mergeHistoryAttachments(live, durable).items[0]).toMatchObject({
      attachments: [{ path: "/tmp/new.png", name: "new.png", kind: "image" }],
    });
  });

  test("projects user/assistant entries and carries attachments", () => {
    const timeline = timelineFromEntries([
      {
        id: "e1",
        kind: "user",
        role: "user",
        createdAtMs: 0,
        blocks: [{ kind: "text", text: "check this" }],
        metadata: {
          attachments: [
            { path: "/tmp/a.png", name: "a.png", kind: "image" },
            { path: "/tmp/b.pdf", name: "b.pdf", kind: "file" },
          ],
        },
      },
      {
        id: "e2",
        kind: "assistant",
        role: "assistant",
        createdAtMs: 0,
        blocks: [{ kind: "text", text: "looks good" }],
      },
      {
        id: "e3",
        kind: "tool",
        role: "tool",
        createdAtMs: 0,
        blocks: [
          { kind: "tool_result", toolCallId: "unmatched", text: "tool output", isError: false },
        ],
      },
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

  test("projects authoritative run outcomes from remote history", () => {
    const timeline = timelineFromEntries([
      {
        id: "u1",
        kind: "user",
        role: "user",
        createdAtMs: 0,
        blocks: [{ kind: "text", text: "try" }],
        runId: "run-failed",
      },
      {
        id: "a1",
        kind: "assistant",
        role: "assistant",
        createdAtMs: 0,
        blocks: [{ kind: "text", text: "partial" }],
        runId: "run-failed",
        run: { status: "failed" },
      },
    ]);
    expect(timeline.items[1]).toMatchObject({
      kind: "message",
      role: "assistant",
      runId: "run-failed",
      failed: true,
    });
  });

  /**
   * A lean history page carries reasoning blocks with no body (the desktop trims
   * them for a client that declared `lean_events_v1`). The row still has to
   * appear — it is the only indication that the model reasoned — so the segment
   * is produced from the block's presence, not from its text. The full-feed case
   * in the same assertion is what keeps the rendered body working.
   */
  test("a reasoning block produces a thinking row with or without a body", () => {
    const user: HistoryEntry = {
      id: "u1", kind: "user", role: "user", createdAtMs: 0,
      blocks: [{ kind: "text", text: "question" }],
    };
    const withText = timelineFromEntries([
      user,
      {
        id: "a1",
        kind: "assistant",
        role: "assistant",
        createdAtMs: 1,
        blocks: [
          { kind: "reasoning", text: "considered the options" },
          { kind: "text", text: "answer" },
        ],
      },
    ]);
    const full = withText.items.find(item => item.kind === "message" && item.role === "assistant");
    if (!full || full.kind !== "message") throw new Error("reply bubble missing");
    expect(full.segments?.map(segment => segment.kind)).toEqual(["thinking", "text"]);
    expect(full.segments?.[0]).toMatchObject({ kind: "thinking", text: "considered the options" });

    // The same entry as a lean page delivers it: the block is there, the body is
    // not. Both an absent key and an empty string count as "no body".
    for (const block of [{ kind: "reasoning" }, { kind: "reasoning", text: "" }]) {
      const lean = timelineFromEntries([
        user,
        {
          id: "a1",
          kind: "assistant",
          role: "assistant",
          createdAtMs: 1,
          blocks: [block, { kind: "text", text: "answer" }],
        },
      ]);
      const reply = lean.items.find(item => item.kind === "message" && item.role === "assistant");
      if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
      expect(reply.segments?.map(segment => segment.kind)).toEqual(["thinking", "text"]);
      expect(reply.segments?.[0]).toMatchObject({ kind: "thinking", text: "" });
      // And the visible answer is unaffected either way.
      expect(reply.segments?.[1]).toMatchObject({ kind: "text", text: "answer" });
    }
  });

  /**
   * The argument list a lean page delivers holds only the file-path keys, and a
   * shell call's arguments are dropped whole, so the row must still render its
   * label without inventing a target — and keep the identity that lets it fetch
   * the command when opened. This is the client half of the desktop's
   * `path`/`file_path`/`filePath` trim; if the two ever disagree, tool rows go
   * blank on a real phone with nothing failing here.
   */
  test("a trimmed argument list still renders the tool row's label", () => {
    // Every key the desktop keeps has to be one this derivation can actually
    // use, or the trim silently strands it and a real phone shows a blank row.
    // All four spellings are listed on purpose: an alias that only one side
    // knows about is exactly the drift this test exists to catch.
    const cases: { name: string; arguments: Record<string, unknown>; target: string }[] = [
      { name: "shell", arguments: { command: "ls -la /tmp" }, target: "ls -la /tmp" },
      { name: "read", arguments: { path: "/a/b.txt" }, target: "/a/b.txt" },
      { name: "read", arguments: { file_path: "/a/b.txt" }, target: "/a/b.txt" },
      { name: "read", arguments: { filePath: "/a/b.txt" }, target: "/a/b.txt" },
      // A tool name the client does not know is treated as shell.
      { name: "future_tool", arguments: { command: "do the thing" }, target: "do the thing" },
    ];
    for (const { name, arguments: args, target } of cases) {
      const timeline = timelineFromEntries([
        {
          id: "u1", kind: "user", role: "user", createdAtMs: 0,
          blocks: [{ kind: "text", text: "go" }],
        },
        {
          id: "a1",
          kind: "assistant",
          role: "assistant",
          createdAtMs: 1,
          runId: "r9",
          blocks: [
            { kind: "tool_call", name, toolCallId: "c1", arguments: args },
          ],
        },
      ]);
      const reply = timeline.items.find(item => item.kind === "message" && item.role === "assistant");
      if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
      const tool = reply.segments?.find(segment => segment.kind === "tool");
      if (!tool || tool.kind !== "tool") throw new Error(`tool row missing for ${name}`);
      expect(tool.tool.detail).toBe(target);
      // Identity comes along for every row: it is what an on-open fetch needs,
      // and the page's `runId` is where it comes from.
      expect(tool.tool).toMatchObject({ toolCallId: "c1", runId: "r9" });
    }
  });

  /**
   * The lean shape of a shell row: no arguments at all, so no target — but the
   * call identity and run survive, which is the whole contract the on-open
   * fetch depends on. A row that lost them would be permanently blank.
   */
  test("a shell row trimmed of its arguments keeps the identity to fetch them", () => {
    const timeline = timelineFromEntries([
      {
        id: "u1", kind: "user", role: "user", createdAtMs: 0, runId: "r9",
        blocks: [{ kind: "text", text: "go" }],
      },
      {
        id: "a1", kind: "assistant", role: "assistant", createdAtMs: 1, runId: "r9",
        blocks: [
          { kind: "tool_call", name: "shell", toolCallId: "c1" },
          { kind: "tool_call", name: "read", toolCallId: "c2", arguments: { path: "/a/b" } },
        ],
      },
    ]);
    const reply = timeline.items.find(item => item.kind === "message" && item.role === "assistant");
    if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
    const tools = reply.segments?.filter(segment => segment.kind === "tool") ?? [];
    const shell = tools[0];
    if (!shell || shell.kind !== "tool") throw new Error("shell row missing");
    expect(shell.tool).toMatchObject({ name: "shell", toolCallId: "c1", runId: "r9" });
    expect(shell.tool.detail).toBeUndefined();
    // The file row still carries its target from the page.
    const file = tools[1];
    if (!file || file.kind !== "tool") throw new Error("file row missing");
    expect(file.tool).toMatchObject({
      name: "read",
      detail: "/a/b",
      toolCallId: "c2",
      runId: "r9",
    });
  });

  test("projects a durable checkpoint from reloaded history", () => {
    const state = timelineFromEntries([
      {
        id: "checkpoint-entry",
        kind: "compaction",
        role: "system",
        createdAtMs: 0,
        blocks: [{ kind: "text", text: "" }],
        checkpoint: {
          schemaVersion: 2,
          checkpointId: "cp-history",
          tokensBefore: 190_000,
          tokensAfter: 20_000,
          trigger: "manual",
        },
      },
    ]);
    const divider = state.items.find(
      item =>
        item.kind === "message" && item.segments?.some(segment => segment.kind === "compaction"),
    );
    expect(divider).toMatchObject({
      kind: "message",
      id: "m_cp-history",
      segments: [
        {
          id: "seg_cp-history_compaction",
          kind: "compaction",
          tokensBefore: 190_000,
          tokensAfter: 20_000,
          trigger: "manual",
        },
      ],
    });
  });

  test.each([
    [2, undefined],
    [2, "pre_turn"],
    [2, "mid_turn"],
    // v3 is the schema the agent writes today; a literal `=== 2` gate here
    // dropped every entry after an in-turn checkpoint.
    [3, undefined],
    [3, "pre_turn"],
    [3, "mid_turn"],
  ])(
    "preserves a reply across an in-turn checkpoint (schema=%s phase=%s)",
    (schemaVersion, phase) => {
      const entries: HistoryEntry[] = [
        {
          id: "u1", kind: "user", role: "user", createdAtMs: 0, runId: "r1",
          blocks: [{ kind: "text", text: "continue" }],
        },
        {
          id: "a1", kind: "assistant", role: "assistant", createdAtMs: 1, runId: "r1",
          blocks: [
            { kind: "text", text: "before compression" },
            { kind: "tool_call", name: "shell", toolCallId: "tool1", arguments: { command: "test" } },
          ],
        },
        {
          id: "cp-entry", kind: "compaction", role: "system", createdAtMs: 2, blocks: [],
          checkpoint: { schemaVersion, checkpointId: "cp1", tokensBefore: 903_386, trigger: "automatic", phase },
        },
        {
          id: "t1", kind: "tool", role: "tool", createdAtMs: 3,
          blocks: [{ kind: "tool_result", toolCallId: "tool1", text: "failed", isError: true }],
        },
        {
          id: "a2", kind: "assistant", role: "assistant", createdAtMs: 4, runId: "r1",
          blocks: [{ kind: "text", text: "after compression" }],
          usage: { outputTokens: 85 }, run: { status: "completed", durationMs: 14_000 },
        },
        {
          id: "u2", kind: "user", role: "user", createdAtMs: 5, runId: "r2",
          blocks: [{ kind: "text", text: "next question" }],
        },
      ];
      const timeline = timelineFromEntries(entries);
      expect(timeline.items.map(item => item.id)).toEqual(["m_u1", "m_a2", "m_u2"]);
      expect(timeline.items[1]).toMatchObject({
        role: "assistant", runId: "r1", text: "before compression\n\nafter compression",
        outputTokens: 85, durationMs: 14_000,
        segments: [
          { kind: "text", text: "before compression" },
          { kind: "tool", tool: { status: "failed" } },
          { kind: "compaction", tokensBefore: 903_386 },
          { kind: "text", text: "after compression" },
        ],
      });
      expect(timelineFromEntries(entries).items).toEqual(timeline.items);
    },
  );

  test.each([undefined, "pre_turn"])(
    "keeps the first reply after pre-turn compression (phase=%s)",
    phase => {
      const entries: HistoryEntry[] = [
        {
          id: "u1", kind: "user", role: "user", createdAtMs: 0, runId: "r1",
          blocks: [{ kind: "text", text: "continue" }],
        },
        {
          id: "cp-entry", kind: "compaction", role: "system", createdAtMs: 1, blocks: [],
          checkpoint: { schemaVersion: 2, checkpointId: "cp1", trigger: "automatic", phase },
        },
      ];
      const pending = timelineFromEntries(entries);
      expect(pending.items.map(item => item.id)).toEqual(["m_u1", "m_cp1"]);
      expect(timelineFromEntries(entries).items).toEqual(pending.items);
      const settled = timelineFromEntries([...entries, {
        id: "a1", kind: "assistant", role: "assistant", createdAtMs: 2, runId: "r1",
        blocks: [{ kind: "text", text: "the missing reply" }],
        run: { status: "completed", durationMs: 100 },
      }]);
      expect(settled.items).toHaveLength(2);
      expect(settled.items[1]).toMatchObject({
        id: "m_a1", runId: "r1", text: "the missing reply",
        segments: [{ kind: "compaction" }, { kind: "text", text: "the missing reply" }],
      });
    },
  );

  test("keeps repeated checkpoints, tool boundaries and the terminal failure in one turn", () => {
    const checkpoint = (id: string): HistoryEntry => ({
      id, kind: "compaction", role: "system", createdAtMs: 1, blocks: [],
      checkpoint: { schemaVersion: 2, checkpointId: id, phase: "mid_turn" },
    });
    const tool = (id: string): HistoryEntry => ({
      id, kind: "assistant", role: "assistant", createdAtMs: 1, runId: "r1",
      blocks: [{ kind: "tool_call", name: "shell", toolCallId: id, arguments: { command: "test" } }],
    });
    const state = timelineFromEntries([
      {
        id: "u1", kind: "user", role: "user", createdAtMs: 0, runId: "r1",
        blocks: [{ kind: "text", text: "continue" }],
        run: { status: "failed", error: "interrupted", durationMs: 100 },
      },
      checkpoint("cp1"), tool("t1"), tool("t2"), checkpoint("cp2"), tool("t3"),
    ]);
    expect(state.items).toHaveLength(2);
    expect(state.items[1]).toMatchObject({
      runId: "r1", failed: true, error: "interrupted", durationMs: 100,
      segments: [
        { kind: "compaction", id: "seg_cp1_compaction" },
        { kind: "tool", tool: { count: 2 } },
        { kind: "compaction", id: "seg_cp2_compaction" },
        { kind: "tool" },
      ],
    });
  });

  test.each([undefined, "standalone"])(
    "keeps manual compression between turns standalone (phase=%s)",
    phase => {
      const state = timelineFromEntries([
        { id: "u1", kind: "user", role: "user", createdAtMs: 0, blocks: [{ kind: "text", text: "first" }] },
        { id: "a1", kind: "assistant", role: "assistant", createdAtMs: 1, blocks: [{ kind: "text", text: "reply" }] },
        {
          id: "cp-entry", kind: "compaction", role: "system", createdAtMs: 2, blocks: [],
          checkpoint: { schemaVersion: 2, checkpointId: "cp1", trigger: "manual", phase },
        },
        { id: "u2", kind: "user", role: "user", createdAtMs: 3, blocks: [{ kind: "text", text: "next" }] },
        { id: "a2", kind: "assistant", role: "assistant", createdAtMs: 4, blocks: [{ kind: "text", text: "next reply" }] },
      ]);
      expect(state.items.map(item => item.id)).toEqual(["m_u1", "m_a1", "m_cp1", "m_u2", "m_a2"]);
    },
  );

  test("keeps a v3 manual checkpoint between turns standalone", () => {
    const state = timelineFromEntries([
      { id: "u1", kind: "user", role: "user", createdAtMs: 0, blocks: [{ kind: "text", text: "first" }] },
      { id: "a1", kind: "assistant", role: "assistant", createdAtMs: 1, blocks: [{ kind: "text", text: "reply" }] },
      {
        id: "cp-entry", kind: "compaction", role: "system", createdAtMs: 2, blocks: [],
        checkpoint: { schemaVersion: 3, checkpointId: "cp1", trigger: "manual", phase: "standalone" },
      },
      { id: "u2", kind: "user", role: "user", createdAtMs: 3, blocks: [{ kind: "text", text: "next" }] },
      { id: "a2", kind: "assistant", role: "assistant", createdAtMs: 4, blocks: [{ kind: "text", text: "next reply" }] },
    ]);
    expect(state.items.map(item => item.id)).toEqual(["m_u1", "m_a1", "m_cp1", "m_u2", "m_a2"]);
  });

  test("keeps attachment-only user entries and drops malformed attachments", () => {
    const timeline = timelineFromEntries([
      {
        id: "e1",
        kind: "user",
        role: "user",
        createdAtMs: 0,
        blocks: [{ kind: "text", text: "" }],
        metadata: {
          attachments: [{ path: "/tmp/a.png", name: "a.png" }, { name: "no-path" } as never],
        },
      },
      {
        id: "e2",
        kind: "user",
        role: "user",
        createdAtMs: 0,
        blocks: [{ kind: "text", text: "" }],
      },
    ]);
    expect(timeline.items).toHaveLength(1);
    expect(timeline.items[0]).toMatchObject({
      attachments: [{ path: "/tmp/a.png", name: "a.png" }],
    });
  });

  test("projects thinking and tool rows inline in the reply per exchange (D2)", () => {
    // History must read like the live transcript (desktop entryProjection
    // parity): the run's thinking/tool rows and streamed text render inline
    // inside the reply bubble, in stream order.
    const timeline = timelineFromEntries([
      {
        id: "u1",
        kind: "user",
        role: "user",
        createdAtMs: 0,
        blocks: [{ kind: "text", text: "check this" }],
      },
      {
        id: "a1",
        kind: "assistant",
        role: "assistant",
        createdAtMs: 0,
        blocks: [
          { kind: "reasoning", text: "reasoning…" },
          { kind: "text", text: "interim analysis" },
          ...[{ id: "call_0", function: { name: "read", arguments: { path: "/tmp/x" } } }].map(
            call => ({
              kind: "tool_call",
              toolCallId: call.id,
              name: call.function.name,
              arguments: call.function.arguments,
            }),
          ),
        ],
      },
      {
        id: "t1",
        kind: "tool",
        role: "tool",
        createdAtMs: 0,
        blocks: [{ kind: "tool_result", toolCallId: "call_0", text: "ok", isError: false }],
      },
      {
        id: "a2",
        kind: "assistant",
        role: "assistant",
        createdAtMs: 0,
        blocks: [{ kind: "text", text: "done" }],
        runId: "run-9",
        usage: { inputTokens: 34, outputTokens: 12, cacheReadTokens: 21 },
        run: { durationMs: 3400 },
      },
      {
        id: "u2",
        kind: "user",
        role: "user",
        createdAtMs: 0,
        blocks: [{ kind: "text", text: "thanks" }],
      },
    ]);
    expect(timeline.items.map(item => item.kind)).toEqual(["message", "message", "message"]);
    const reply = timeline.items[1];
    if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
    expect(reply).toMatchObject({
      role: "assistant",
      text: "interim analysis\n\ndone",
      runId: "run-9",
      durationMs: 3400,
      outputTokens: 12,
      inputTokens: 34,
      cacheReadTokens: 21,
    });
    expect(reply.segments).toEqual([
      { id: expect.any(String), kind: "thinking", text: "reasoning…" },
      { id: expect.any(String), kind: "text", text: "interim analysis" },
      {
        id: expect.any(String),
        kind: "tool",
        tool: {
          name: "read",
          status: "completed",
          complete: true,
          detail: "/tmp/x",
          // The call's identity rides the row, so a target the page omitted can
          // be fetched on open (this entry carries no runId, so none is set).
          toolCallId: "call_0",
        },
      },
      { id: expect.any(String), kind: "text", text: "done" },
    ]);
    // A reply-less run (empty assistant entry) renders nothing extra.
    const divider = timelineFromEntries([
      {
        id: "u3",
        kind: "user",
        role: "user",
        createdAtMs: 0,
        blocks: [{ kind: "text", text: "next" }],
      },
      {
        id: "a3",
        kind: "assistant",
        role: "assistant",
        createdAtMs: 0,
        blocks: [{ kind: "text", text: "" }],
      },
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
    expect(timeline.items.map(item => item.kind)).toEqual(["message"]);
    const reply = timeline.items[0];
    if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
    expect(reply).toMatchObject({ kind: "message", role: "assistant", text: "answer" });
    // The tool row renders inline inside the bubble, in stream order.
    expect(reply.segments).toEqual([
      {
        id: expect.any(String),
        kind: "tool",
        tool: {
          name: "read",
          status: "completed",
          complete: true,
          toolCallId: "t1",
          runId: "run-1",
        },
      },
      { id: expect.any(String), kind: "text", text: "answer" },
    ]);
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

  test("dropSupersededCompactionDividers drops only the durable copy of a shared checkpoint", () => {
    const divider = (id: string, checkpointId?: string) => ({
      id,
      kind: "message" as const,
      role: "assistant" as const,
      text: "",
      ...(checkpointId
        ? { segments: [{ id: `seg_${checkpointId}_compaction`, kind: "compaction" as const, checkpointId }] }
        : { segments: [{ id: "seg", kind: "compaction" as const }] }),
    });
    const live = divider("assistant:live", "cp-1");
    const history = [
      divider("m_cp-1", "cp-1"),
      divider("m_cp-old", "cp-old"),
      divider("m_cp-anonymous"),
      { id: "u1", kind: "message" as const, role: "user" as const, text: "hi" },
    ];

    expect(dropSupersededCompactionDividers(history, [live]).map(item => item.id))
      .toEqual(["m_cp-old", "m_cp-anonymous", "u1"]);
    // No identity to compare on either side — nothing is superseded.
    expect(dropSupersededCompactionDividers(history, [divider("assistant:live")]))
      .toEqual(history);
    // The live row is a divider-only row too; it must never drop itself.
    expect(dropSupersededCompactionDividers([live], [live]).map(item => item.id))
      .toEqual(["assistant:live"]);
  });
});

describe("user message mirror", () => {
  test("commits a user message only when the prompt acknowledgement is applied", () => {
    const pending = emptyTimeline();
    expect(pending.items).toEqual([]);

    const accepted = commitAcknowledgedUserMessage(pending, {
      id: "local:prompt-1",
      runId: "run-1",
      text: "check this",
    });
    expect(accepted.items).toEqual([
      expect.objectContaining({
        id: "local:prompt-1",
        kind: "message",
        role: "user",
        runId: "run-1",
        text: "check this",
      }),
    ]);
  });

  test("pairs persisted tool results by id and trusts explicit error status", () => {
    const timeline = timelineFromEntries([
      {
        id: "u1",
        kind: "user",
        role: "user",
        createdAtMs: 0,
        blocks: [{ kind: "text", text: "run tools" }],
      },
      {
        id: "a1",
        kind: "assistant",
        role: "assistant",
        createdAtMs: 0,
        blocks: [
          { kind: "text", text: "done" },
          ...[
            { id: "call-read", function: { name: "read", arguments: { path: "/tmp/x" } } },
            { id: "call-shell", function: { name: "shell", arguments: { command: "true" } } },
          ].map(call => ({
            kind: "tool_call",
            toolCallId: call.id,
            name: call.function.name,
            arguments: call.function.arguments,
          })),
        ],
      },
      {
        id: "t2",
        kind: "tool",
        role: "tool",
        createdAtMs: 0,
        blocks: [{ kind: "tool_result", toolCallId: "call-shell", text: "ok", isError: true }],
      },
      {
        id: "t1",
        kind: "tool",
        role: "tool",
        createdAtMs: 0,
        blocks: [
          {
            kind: "tool_result",
            toolCallId: "call-read",
            text: "Error: legacy-looking text",
            isError: false,
          },
        ],
      },
    ]);
    const reply = timeline.items.find(item => item.kind === "message" && item.role === "assistant");
    if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
    const tools = reply.segments?.filter(segment => segment.kind === "tool") ?? [];
    expect(tools.map(segment => (segment.kind === "tool" ? segment.tool.status : null))).toEqual([
      "completed",
      "failed",
    ]);
  });

  test("acknowledgement enriches an early live mirror instead of duplicating it", () => {
    const mirrored = applyStreamEvent(emptyTimeline(), {
      type: "user_message",
      runId: "run-1",
      data: JSON.stringify({ text: "check this" }),
    });
    const accepted = commitAcknowledgedUserMessage(mirrored, {
      id: "local:prompt-1",
      runId: "run-1",
      text: "check this",
      attachments: [{ path: "file:///photo.jpg", name: "photo.jpg", kind: "image" }],
    });

    expect(accepted.items).toHaveLength(1);
    expect(accepted.items[0]).toMatchObject({
      kind: "message",
      role: "user",
      runId: "run-1",
      text: "check this",
      attachments: [{ name: "photo.jpg", kind: "image" }],
    });
  });

  test("replaying the same acknowledgement is idempotent", () => {
    const input = {
      id: "local:prompt-1",
      runId: "run-1",
      text: "check this",
    };
    const first = commitAcknowledgedUserMessage(emptyTimeline(), input);
    const replayed = commitAcknowledgedUserMessage(first, input);

    expect(replayed.items).toHaveLength(1);
    expect(replayed.items[0]).toMatchObject(input);
  });

  test("user_message from another device appends a user bubble", () => {
    const state = applyStreamEvent(emptyTimeline(), {
      type: "user_message",
      data: JSON.stringify({ text: "check this" }),
    });
    expect(state.items).toHaveLength(1);
    expect(state.items[0]).toMatchObject({ kind: "message", role: "user", text: "check this" });
  });

  test("events deduplicate by identity while identical prompts remain separate", () => {
    const event = {
      type: "user_message",
      data: JSON.stringify({ text: "check this", entry_id: "u1", run_id: "r1" }),
    };
    const first = applyStreamEvent(emptyTimeline(), event);
    const repeat = applyStreamEvent(first, event);
    expect(repeat.items).toHaveLength(1);
    const next = applyStreamEvent(repeat, {
      type: "user_message",
      data: JSON.stringify({ text: "check this", entry_id: "u2", run_id: "r2" }),
    });
    expect(next.items.map(item => item.id)).toEqual(["m_u1", "m_u2"]);
    const durable = timelineFromEntries([
      {
        id: "u1",
        kind: "user",
        role: "user",
        createdAtMs: 0,
        runId: "r1",
        blocks: [{ kind: "text", text: "check this" }],
      },
    ]);
    expect(applyStreamEvent(durable, event).items).toHaveLength(1);
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

  test("consumes a text_chunk truncation marker as a friendly notice instead of dropping it", () => {
    const timeline = applyStreamEvent(emptyTimeline(), {
      type: "text_chunk",
      data: JSON.stringify({ _truncated: true, bytes: 1200000 }),
      runId: "run-1",
      idx: 5,
    });
    expect(timeline.items).toEqual([
      expect.objectContaining({
        kind: "notice",
        tone: "warning",
        text: "truncated",
        runId: "run-1",
      }),
    ]);
    // A second marker for the same run must not duplicate the notice.
    const again = applyStreamEvent(timeline, {
      type: "text_chunk",
      data: JSON.stringify({ _truncated: true, bytes: 1200000 }),
      runId: "run-1",
      idx: 6,
    });
    expect(again.items).toHaveLength(1);
  });

  test("never renders the truncation marker's raw JSON as an error", () => {
    const timeline = applyStreamEvent(emptyTimeline(), {
      type: "error",
      data: JSON.stringify({ _truncated: true, bytes: 1200000 }),
      runId: "run-1",
      idx: 7,
    });
    expect(timeline.items[0]).toMatchObject({
      kind: "notice",
      tone: "danger",
      text: "truncated",
      runId: "run-1",
    });
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
    // Chronological order: the reasoning renders inline inside the bubble.
    expect(host.segments).toEqual([
      { id: expect.any(String), kind: "thinking", text: "reasoning…" },
    ]);
    expect(thinking.items.map(item => item.kind)).toEqual(["message"]);

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
    expect(text.items.map(item => item.kind)).toEqual(["message"]);
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
    expect(state.items.map(item => item.kind)).toEqual(["message"]);
    const answer = state.items[0];
    if (!answer || answer.kind !== "message") throw new Error("assistant message missing");
    expect(answer).toMatchObject({ text: "answer", streaming: true });
    // D2: the reasoning and tool work render inline inside the bubble, in the
    // chronological order the agent produced them.
    expect(answer.segments?.map(segment => segment.kind)).toEqual(["thinking", "tool", "text"]);
  });

  /**
   * The lean live lane drops a shell call's arguments, so the row has no target
   * to show until it is opened. What keeps it from being permanently blank is
   * the identity the activity carries from the event: `get_tool_call_args` is
   * answered by (session, run, call), and the row passes the run it streamed
   * under. A row that lost either would render a dead affordance.
   */
  test("a live shell row keeps the identity to fetch a command the lane dropped", () => {
    let state = applyStreamEvent(emptyTimeline(), {
      type: "agent_start",
      data: "{}",
      runId: "run-7",
      idx: 0,
    });
    state = applyStreamEvent(state, {
      type: "tool_start",
      data: JSON.stringify({ tool_id: "t1", tool_name: "shell", phase: "execution" }),
      runId: "run-7",
      idx: 1,
    });
    state = applyStreamEvent(state, {
      type: "tool_end",
      data: JSON.stringify({ tool_id: "t1", exit_code: 0 }),
      runId: "run-7",
      idx: 2,
    });
    const reply = state.items.find(item => item.kind === "message" && item.role === "assistant");
    if (!reply || reply.kind !== "message") throw new Error("assistant message missing");
    const tool = reply.segments?.find(segment => segment.kind === "tool");
    if (!tool || tool.kind !== "tool") throw new Error("tool row missing");
    expect(tool.tool).toMatchObject({
      name: "shell",
      complete: true,
      toolCallId: "t1",
      runId: "run-7",
    });
    expect(tool.tool.detail).toBeUndefined();
  });

  test("keeps tool rows in stream order when text streams before the first tool call", () => {
    // Regression: a model may stream an interim remark ahead of its first tool
    // call; the tool row must sit between the two text blocks inside the
    // bubble (desktop shows the final answer last).
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
    const answer = state.items.find(item => item.kind === "message" && item.role === "assistant");
    if (!answer || answer.kind !== "message") throw new Error("assistant message missing");
    // The copyable text joins the bubble's text blocks; the tool row sits
    // between them inside the bubble (stream order).
    expect(answer.text).toBe("interim \n\nanswer");
    expect(answer.segments?.map(segment => segment.kind)).toEqual(["text", "tool", "text"]);
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
    const shell = state.items.find(item => item.kind === "message");
    if (!shell || shell.kind !== "message") throw new Error("shell bubble missing");
    expect(shell.segments).toEqual([
      {
        id: expect.any(String),
        kind: "tool",
        tool: {
          name: "shell",
          status: "running",
          complete: false,
          detail: "ls -la",
          // The call's identity travels with the row on the live lane too: it is
          // what an on-open fetch of a dropped target is answered by.
          toolCallId: "t1",
          runId: "run-1",
        },
      },
    ]);
    state = applyStreamEvent(state, {
      type: "tool_start",
      data: JSON.stringify({ tool_id: "t2", tool_name: "read", tool_args: '{"path":"/tmp/x"}' }),
      runId: "run-1",
      idx: 1,
    });
    const reply = state.items.find(item => item.kind === "message");
    if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
    expect(reply.segments).toEqual([
      {
        id: expect.any(String),
        kind: "tool",
        tool: {
          name: "shell",
          status: "running",
          complete: false,
          detail: "ls -la",
          toolCallId: "t1",
          runId: "run-1",
        },
      },
      {
        id: expect.any(String),
        kind: "tool",
        tool: {
          name: "read",
          status: "running",
          complete: false,
          detail: "/tmp/x",
          toolCallId: "t2",
          runId: "run-1",
        },
      },
    ]);
  });

  test("agent_end leaves the thinking slice inline in the settled reply", () => {
    let state = applyStreamEvent(emptyTimeline(), {
      type: "thinking_delta",
      data: JSON.stringify({ text: "hmm" }),
      runId: "run-1",
      idx: 0,
    });
    state = applyStreamEvent(state, { type: "agent_end", data: "{}", runId: "run-1", idx: 1 });
    const reply = state.items.find(item => item.kind === "message" && item.role === "assistant");
    if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
    // The thinking slice stays inline in the bubble; the settled reply keeps it.
    expect(reply.segments).toEqual([{ id: expect.any(String), kind: "thinking", text: "hmm" }]);
    expect(reply.streaming).toBe(false);
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

describe("shared-projection semantic flags", () => {
  test("a shell exit-code marks only the tool row failed (G1)", () => {
    let state = applyStreamEvent(emptyTimeline(), {
      type: "tool_start",
      data: JSON.stringify({
        tool_id: "t1",
        tool_name: "shell",
        tool_args: { command: "future nosuch" },
      }),
      runId: "run-1",
      idx: 0,
    });
    state = applyStreamEvent(state, {
      type: "tool_end",
      data: JSON.stringify({
        tool_id: "t1",
        tool_name: "shell",
        text: "bash: future: command not found\n\n[exit: 127]",
      }),
      runId: "run-1",
      idx: 1,
    });
    const reply = state.items.find(item => item.kind === "message");
    if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
    expect(reply.failed).toBeUndefined();
    const toolSegment = reply.segments?.find(segment => segment.kind === "tool");
    expect(toolSegment && toolSegment.kind === "tool" && toolSegment.tool.status).toBe("failed");
  });

  test("a bare grep exit-1 is a soft fail, not a tool failure (G1 exemption)", () => {
    let state = applyStreamEvent(emptyTimeline(), {
      type: "tool_start",
      data: JSON.stringify({
        tool_id: "t1",
        tool_name: "shell",
        tool_args: { command: "grep foo file" },
      }),
      runId: "run-1",
      idx: 0,
    });
    state = applyStreamEvent(state, {
      type: "tool_end",
      data: JSON.stringify({ tool_id: "t1", tool_name: "shell", text: "[exit: 1]" }),
      runId: "run-1",
      idx: 1,
    });
    const reply = state.items.find(item => item.kind === "message");
    if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
    expect(reply.failed).toBeUndefined();
  });

  /**
   * The lean feed (declared as `lean_events_v1`) sends no captured tool output,
   * so the `[exit: N]` footer these two tests above rely on is simply absent.
   * The agent's structured outcome has to carry the verdict on its own —
   * otherwise every failing command would render as completed.
   */
  test("a structured exit code fails the row with no output text at all", () => {
    let state = applyStreamEvent(emptyTimeline(), {
      type: "tool_start",
      data: JSON.stringify({
        tool_id: "t1",
        tool_name: "shell",
        tool_args: { command: "future nosuch" },
      }),
      runId: "run-1",
      idx: 0,
    });
    state = applyStreamEvent(state, {
      type: "tool_end",
      data: JSON.stringify({ tool_id: "t1", tool_name: "shell", exit_code: 127 }),
      runId: "run-1",
      idx: 1,
    });
    const reply = state.items.find(item => item.kind === "message");
    if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
    const toolSegment = reply.segments?.find(segment => segment.kind === "tool");
    expect(toolSegment && toolSegment.kind === "tool" && toolSegment.tool.status).toBe("failed");
  });

  test("a zero structured exit code completes the row", () => {
    let state = applyStreamEvent(emptyTimeline(), {
      type: "tool_start",
      data: JSON.stringify({ tool_id: "t1", tool_name: "shell", tool_args: { command: "ls" } }),
      runId: "run-1",
      idx: 0,
    });
    state = applyStreamEvent(state, {
      type: "tool_end",
      data: JSON.stringify({ tool_id: "t1", tool_name: "shell", exit_code: 0 }),
      runId: "run-1",
      idx: 1,
    });
    const reply = state.items.find(item => item.kind === "message");
    if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
    const toolSegment = reply.segments?.find(segment => segment.kind === "tool");
    expect(toolSegment && toolSegment.kind === "tool" && toolSegment.tool.status).toBe("completed");
  });

  test("the soft-fail exemption survives the structured path", () => {
    const run = (end: Record<string, unknown>) => {
      let state = applyStreamEvent(emptyTimeline(), {
        type: "tool_start",
        data: JSON.stringify({
          tool_id: "t1",
          tool_name: "shell",
          tool_args: { command: "grep foo file" },
        }),
        runId: "run-1",
        idx: 0,
      });
      state = applyStreamEvent(state, {
        type: "tool_end",
        data: JSON.stringify({ tool_id: "t1", tool_name: "shell", ...end }),
        runId: "run-1",
        idx: 1,
      });
      const reply = state.items.find(item => item.kind === "message");
      if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
      const segment = reply.segments?.find(candidate => candidate.kind === "tool");
      return segment && segment.kind === "tool" ? segment.tool.status : undefined;
    };
    // Bare `grep` exits 1 when it simply finds nothing — not a failure.
    expect(run({ exit_code: 1 })).toBe("completed");
    // Any other code, or a piped command whose code is ambiguous, is a failure.
    expect(run({ exit_code: 2 })).toBe("failed");
    // The agent's own verdict is honoured when it is present.
    expect(run({ exit_code: 1, is_soft_fail: true })).toBe("completed");
  });

  /**
   * The same verdicts on the lane as it now ships: the shell command is not on
   * the wire at all, so a row's exemption can only come from the agent's own
   * `is_soft_fail`. That is the whole signal in practice — over 20,332 real
   * shell results, every `exit_code: 1` without the flag was a genuine failure
   * (a piped/`&&` chain or a program outside the soft-fail set), and the bare
   * soft-fail case carries the flag. A row whose command the lane dropped and
   * whose outcome lacks the flag therefore reads as failed: the documented
   * price of not shipping the command, reachable only from an agent predating
   * the flag (see `FETCHED_LABEL_TOOLS` in the desktop's lean trim).
   */
  test("a lean shell row's verdict comes from the agent, not from a command", () => {
    const run = (end: Record<string, unknown>) => {
      let state = applyStreamEvent(emptyTimeline(), {
        type: "tool_start",
        data: JSON.stringify({ tool_id: "t1", tool_name: "shell", phase: "execution" }),
        runId: "run-1",
        idx: 0,
      });
      state = applyStreamEvent(state, {
        type: "tool_end",
        data: JSON.stringify({ tool_id: "t1", tool_name: "shell", ...end }),
        runId: "run-1",
        idx: 1,
      });
      const reply = state.items.find(item => item.kind === "message");
      if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
      const segment = reply.segments?.find(candidate => candidate.kind === "tool");
      return segment && segment.kind === "tool" ? segment.tool.status : undefined;
    };
    // The agent judged it a soft failure: no command needed.
    expect(run({ exit_code: 1, is_soft_fail: true })).toBe("completed");
    // No flag and no command: the exemption cannot be evaluated, and a real
    // failure must not be hidden. This is also every real exit-1 row.
    expect(run({ exit_code: 1 })).toBe("failed");
    expect(run({ exit_code: 2 })).toBe("failed");
    expect(run({ exit_code: 0 })).toBe("completed");
  });

  /**
   * The other two halves of the lean contract: a reasoning row has to exist from
   * its boundary alone (the deltas never arrive), and a tool's target has to come
   * from `tool_start`'s complete arguments, since `tool_delta` is not sent.
   */
  test("a reasoning row is projected from its boundary alone", () => {
    let state = applyStreamEvent(emptyTimeline(), {
      type: "thinking_start",
      data: JSON.stringify({ block_id: "b1" }),
      runId: "run-1",
      idx: 0,
    });
    state = applyStreamEvent(state, {
      type: "thinking_end",
      data: JSON.stringify({ block_id: "b1" }),
      runId: "run-1",
      idx: 1,
    });
    state = applyStreamEvent(state, {
      type: "text_chunk",
      data: JSON.stringify({ text: "answer" }),
      runId: "run-1",
      idx: 2,
    });
    const reply = state.items.find(item => item.kind === "message");
    if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
    expect(reply.segments?.map(segment => segment.kind)).toEqual(["thinking", "text"]);
    const thinking = reply.segments?.[0];
    expect(thinking && thinking.kind === "thinking" && thinking.text).toBe("");
  });

  /**
   * A lean feed opens a reasoning block *after* work has already started: the
   * tool calls streamed first, then the model thinks again. The block carries no
   * body (its deltas are not published), so its boundary is the only thing that
   * can render the row — the tool run's "hop over whitespace-only text" must not
   * mistake the empty reasoning slot for that whitespace and swallow it, nor may
   * it glue the tools on either side into one burst.
   */
  test("a reasoning row after a tool call survives with no body (lean feed)", () => {
    const leanRow = (events: [string, Record<string, unknown>][]) => {
      let state = applyStreamEvent(emptyTimeline(), {
        type: "agent_start",
        data: "{}",
        runId: "run-1",
        idx: 0,
      });
      events.forEach(([type, data], index) => {
        state = applyStreamEvent(state, {
          type,
          data: JSON.stringify(data),
          runId: "run-1",
          idx: index + 1,
        });
      });
      const reply = state.items.find(item => item.kind === "message" && item.role === "assistant");
      if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
      return reply.segments?.map(segment => segment.kind);
    };
    const read = (id: string) => ([
      "tool_start",
      { tool_id: id, tool_name: "read", tool_args: { path: "/tmp/a" } },
    ] as [string, Record<string, unknown>]);
    const readEnd = (id: string) => ([
      "tool_end",
      { tool_id: id, tool_name: "read", exit_code: 0 },
    ] as [string, Record<string, unknown>]);
    const thinking = (blockId: string) => ([
      "thinking_start",
      { type: "thinking_start", block_id: blockId },
    ] as [string, Record<string, unknown>]);

    // Tool, then a fresh reasoning block, then nothing else yet (the live tail).
    expect(leanRow([read("t1"), readEnd("t1"), thinking("b2")])).toEqual(["tool", "thinking"]);
    // The reasoning boundary also separates two tool calls into their own rows.
    expect(leanRow([
      read("t1"), readEnd("t1"), thinking("b2"), read("t2"), readEnd("t2"),
    ])).toEqual(["tool", "thinking", "tool"]);
  });

  test("a tool target comes from tool_start, with no argument stream", () => {
    let state = applyStreamEvent(emptyTimeline(), {
      type: "tool_start",
      data: JSON.stringify({
        tool_id: "t1",
        tool_name: "shell",
        tool_args: { command: "ls -la /tmp" },
      }),
      runId: "run-1",
      idx: 0,
    });
    state = applyStreamEvent(state, {
      type: "tool_end",
      data: JSON.stringify({ tool_id: "t1", tool_name: "shell", exit_code: 0 }),
      runId: "run-1",
      idx: 1,
    });
    const reply = state.items.find(item => item.kind === "message");
    if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
    const toolSegment = reply.segments?.find(segment => segment.kind === "tool");
    if (!toolSegment || toolSegment.kind !== "tool") throw new Error("tool row missing");
    // The mobile bubble renders the row's text from `detail`; the projection
    // takes it from `tool_start`'s complete `tool_args`, which is why dropping
    // the argument stream does not cost the row its label.
    expect(toolSegment.tool.detail).toBe("ls -la /tmp");
    expect(toolSegment.tool.status).toBe("completed");
  });

  test("a cancelled run marks the bubble stopped (G15)", () => {
    let state = applyStreamEvent(emptyTimeline(), {
      type: "text_chunk",
      data: JSON.stringify({ text: "partial" }),
      runId: "run-1",
      idx: 0,
    });
    state = applyStreamEvent(state, {
      type: "agent_end",
      data: JSON.stringify({ state: "cancelled" }),
      runId: "run-1",
      idx: 1,
    });
    const reply = state.items.find(item => item.kind === "message" && item.role === "assistant");
    if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
    expect(reply.stopped).toBe(true);
    expect(reply.truncated).toBeUndefined();
  });

  test("an incomplete stream marks the bubble truncated (G13)", () => {
    let state = applyStreamEvent(emptyTimeline(), {
      type: "text_chunk",
      data: JSON.stringify({ text: "cut off" }),
      runId: "run-1",
      idx: 0,
    });
    state = applyStreamEvent(state, {
      type: "agent_end",
      data: JSON.stringify({ reason: "incomplete" }),
      runId: "run-1",
      idx: 1,
    });
    const reply = state.items.find(item => item.kind === "message" && item.role === "assistant");
    if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
    expect(reply.truncated).toBe(true);
    expect(reply.failed).toBe(true);
    expect(reply.stopped).toBeUndefined();
  });

  test("an agent error marks the run failed", () => {
    let state = applyStreamEvent(emptyTimeline(), {
      type: "text_chunk",
      data: JSON.stringify({ text: "partial" }),
      runId: "run-1",
      idx: 0,
    });
    state = applyStreamEvent(state, {
      type: "agent_end",
      data: JSON.stringify({ state: "error", error: "provider failed" }),
      runId: "run-1",
      idx: 1,
    });
    const reply = state.items.find(item => item.kind === "message" && item.role === "assistant");
    if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
    expect(reply.failed).toBe(true);
    expect(reply.stopped).toBeUndefined();
  });

  test("a clean agent_end is neither stopped nor truncated", () => {
    let state = applyStreamEvent(emptyTimeline(), {
      type: "text_chunk",
      data: JSON.stringify({ text: "full" }),
      runId: "run-1",
      idx: 0,
    });
    state = applyStreamEvent(state, { type: "agent_end", data: "{}", runId: "run-1", idx: 1 });
    const reply = state.items.find(item => item.kind === "message" && item.role === "assistant");
    if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
    expect(reply.stopped).toBeUndefined();
    expect(reply.truncated).toBeUndefined();
  });

  test("a compaction_end renders an inline divider segment (G3)", () => {
    let state = applyStreamEvent(emptyTimeline(), {
      type: "compaction_end",
      data: JSON.stringify({ tokens_before: 190_000, aborted: false }),
      runId: "run-1",
      idx: 0,
    });
    state = applyStreamEvent(state, {
      type: "text_chunk",
      data: JSON.stringify({ text: "Continuing." }),
      runId: "run-1",
      idx: 1,
    });
    const reply = state.items.find(item => item.kind === "message");
    if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
    expect(reply.segments).toEqual([
      { id: expect.any(String), kind: "compaction", tokensBefore: 190_000 },
      { id: expect.any(String), kind: "text", text: "Continuing." },
    ]);
  });

  test("a durable compaction_committed renders one checkpoint divider", () => {
    const committed = {
      type: "compaction_committed",
      data: JSON.stringify({ checkpoint_id: "cp-1", tokens_before: 190_000 }),
      runId: "run-1",
      idx: 0,
    };
    let state = applyStreamEvent(emptyTimeline(), committed);
    // Replayed delivery of the same durable checkpoint must be idempotent.
    state = applyStreamEvent(state, { ...committed, idx: 1 });
    const reply = state.items.find(item => item.kind === "message");
    if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
    expect(reply.segments?.filter(segment => segment.kind === "compaction")).toEqual([
      { id: "cp-1", kind: "compaction", checkpointId: "cp-1", tokensBefore: 190_000 },
    ]);
  });

  test("a standalone compaction becomes its own divider and never revives a finished run", () => {
    // A settled run, then a manual compaction. The Agent stamps the compaction
    // with the session's last run id, so keying it by that run would re-open
    // the finished reply (and leave the composer on "generating" forever,
    // because no agent_end follows a standalone compaction).
    let state = applyStreamEvent(emptyTimeline(), {
      type: "text_chunk",
      data: JSON.stringify({ text: "previous reply" }),
      runId: "run-old",
      idx: 0,
    });
    state = applyStreamEvent(state, { type: "agent_end", data: "{}", runId: "run-old", idx: 1 });
    expect(state.streaming).toBe(false);

    state = applyStreamEvent(state, {
      type: "compaction_started",
      data: JSON.stringify({ operation_id: "cmp-manual", trigger: "manual", phase: "standalone" }),
      runId: "run-old",
      idx: 2,
    });    state = applyStreamEvent(state, {
      type: "compaction_committed",
      data: JSON.stringify({
        operation_id: "cmp-manual",
        checkpoint_id: "cp-manual",
        tokens_before: 33_064,
        trigger: "manual",
        phase: "standalone",
      }),
      runId: "run-old",
      idx: 3,
    });

    expect(state.streaming).toBe(false);
    const messages = state.items.filter(item => item.kind === "message");
    // The finished reply is untouched: no second, empty streaming bubble.
    expect(messages.map(item => item.id)).toEqual(["assistant:run-old", "m_cp-manual"]);
    expect(messages[0]).toMatchObject({ streaming: false, text: "previous reply" });
    expect(messages[1]).toMatchObject({
      streaming: false,
      text: "",
      segments: [{ id: "seg_cp-manual_compaction", kind: "compaction", tokensBefore: 33_064, trigger: "manual", status: "completed" }],
    });
    expect(messages[1]?.runId).toBeUndefined();
  });

  test("a mid-turn compaction stays inside the running reply", () => {
    let state = applyStreamEvent(emptyTimeline(), {
      type: "agent_start",
      data: "{}",
      runId: "run-live",
      idx: 0,
    });
    state = applyStreamEvent(state, {
      type: "compaction_started",
      data: JSON.stringify({ operation_id: "cmp-auto", trigger: "automatic", phase: "mid_turn" }),
      runId: "run-live",
      idx: 1,
    });
    state = applyStreamEvent(state, {
      type: "compaction_committed",
      data: JSON.stringify({ operation_id: "cmp-auto", checkpoint_id: "cp-auto", tokens_before: 120_000 }),
      runId: "run-live",
      idx: 2,
    });
    state = applyStreamEvent(state, {
      type: "text_chunk",
      data: JSON.stringify({ text: "continuing" }),
      runId: "run-live",
      idx: 3,
    });

    expect(state.streaming).toBe(true);
    expect(state.items.filter(item => item.kind === "message")).toHaveLength(1);
    expect(state.items[0]).toMatchObject({
      id: "assistant:run-live",
      streaming: true,
      segments: [
        { id: "cp-auto", kind: "compaction", tokensBefore: 120_000 },
        { id: expect.any(String), kind: "text", text: "continuing" },
      ],
    });
  });

  test("a compaction with nothing to show leaves no placeholder behind", () => {
    let state = applyStreamEvent(emptyTimeline(), {
      type: "compaction_started",
      data: JSON.stringify({ operation_id: "cmp-none", trigger: "manual", phase: "standalone" }),
      idx: 0,
    });
    // The running placeholder is the immediate feedback for a manual compaction.
    expect(state.items).toHaveLength(1);
    expect(state.compacting).toBe(true);
    state = applyStreamEvent(state, {
      type: "compaction_unchanged",
      data: JSON.stringify({ operation_id: "cmp-none", reused: true, phase: "standalone" }),
      idx: 1,
    });
    expect(state.streaming).toBe(false);
    expect(state.compacting).toBe(false);
    expect(state.items).toEqual([]);
  });

  test("compaction lifecycle replaces running state and retains failures", () => {
    let state = applyStreamEvent(emptyTimeline(), {
      type: "compaction_started",
      data: JSON.stringify({ operation_id: "cmp-1", trigger: "automatic", phase: "pre_turn" }),
      runId: "run-1",
      idx: 0,
    });
    let reply = state.items.find(item => item.kind === "message");
    if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
    expect(reply.segments).toEqual([
      { id: "cmp-1", kind: "compaction", status: "running", trigger: "automatic" },
    ]);

    state = applyStreamEvent(state, {
      type: "compaction_committed",
      data: JSON.stringify({ operation_id: "cmp-1", checkpoint_id: "cp-1", tokens_before: 42_000 }),
      runId: "run-1",
      idx: 1,
    });
    reply = state.items.find(item => item.kind === "message");
    if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
    expect(reply.segments).toEqual([
      { id: "cp-1", kind: "compaction", checkpointId: "cp-1", tokensBefore: 42_000, trigger: "automatic" },
    ]);

    let failedState = applyStreamEvent(emptyTimeline(), {
      type: "compaction_started",
      data: JSON.stringify({ operation_id: "cmp-2" }),
      runId: "run-2",
      idx: 0,
    });
    failedState = applyStreamEvent(failedState, {
      type: "compaction_failed",
      data: JSON.stringify({ operation_id: "cmp-2", error: "summary failed" }),
      runId: "run-2",
      idx: 1,
    });
    const failedReply = failedState.items.find(item => item.kind === "message");
    if (!failedReply || failedReply.kind !== "message") throw new Error("reply bubble missing");
    expect(failedReply.segments).toEqual([
      { id: "cmp-2", kind: "compaction", status: "failed", error: "summary failed" },
    ]);

    let interruptedState = applyStreamEvent(emptyTimeline(), {
      type: "compaction_started",
      data: JSON.stringify({ operation_id: "cmp-3" }),
      runId: "run-3",
      idx: 0,
    });
    interruptedState = applyStreamEvent(interruptedState, {
      type: "agent_end",
      data: JSON.stringify({ state: "cancelled" }),
      runId: "run-3",
      idx: 1,
    });
    const interruptedReply = interruptedState.items.find(item => item.kind === "message");
    if (!interruptedReply || interruptedReply.kind !== "message") {
      throw new Error("reply bubble missing");
    }
    expect(interruptedReply.segments).toEqual([
      {
        id: "cmp-3",
        kind: "compaction",
        status: "failed",
        error: "compaction interrupted before completion",
      },
    ]);
  });

  test("the settled totals prefer agent_end usage over the late-join partial sum", () => {
    let state = applyStreamEvent(emptyTimeline(), {
      type: "text_chunk",
      data: JSON.stringify({ text: "tail" }),
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
      data: JSON.stringify({ usage: { output_tokens: 2965 } }),
      runId: "run-1",
      idx: 2000,
    });
    const reply = state.items.find(item => item.kind === "message");
    if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
    expect(reply.outputTokens).toBe(2965);
  });
});

describe("approval decisions", () => {
  test("a repeated approval_request with the same id does not duplicate the card", () => {
    let state = applyStreamEvent(emptyTimeline(), {
      type: "approval_request",
      data: JSON.stringify({ approval_request_id: "approval-1", title: "Write file" }),
      runId: "run-1",
      idx: 1,
    });
    state = applyStreamEvent(state, {
      type: "approval_request",
      data: JSON.stringify({ approval_request_id: "approval-1", title: "Write file" }),
      runId: "run-1",
      idx: 2,
    });
    expect(state.items.filter(item => item.kind === "approval")).toHaveLength(1);
  });

  test("markApprovalDecision stamps a decision only on the matching approval", () => {
    const state = applyStreamEvent(emptyTimeline(), {
      type: "approval_request",
      data: JSON.stringify({ approval_request_id: "approval-1" }),
      runId: "run-1",
      idx: 1,
    });
    const decided = markApprovalDecision(state, "approval-1", "approved");
    expect(decided.items[0]).toMatchObject({ kind: "approval", decision: "approved" });
    const untouched = markApprovalDecision(state, "approval-other", "rejected");
    expect(untouched.items[0]).not.toHaveProperty("decision");
  });
});

describe("stream event edge cases", () => {
  test("malformed event JSON degrades to an empty payload", () => {
    const state = applyStreamEvent(emptyTimeline(), {
      type: "user_message",
      data: "not json{",
      runId: "run-1",
      idx: 1,
    });
    // The unparseable payload yields no text, so no user bubble is appended.
    expect(state.items).toEqual([]);
  });

  test("a burst of same-kind completed tools collapses into a summary row with children", () => {
    let state = applyStreamEvent(emptyTimeline(), {
      type: "agent_start",
      data: "{}",
      runId: "run-1",
      idx: 0,
    });
    state = applyStreamEvent(state, {
      type: "tool_start",
      data: JSON.stringify({ tool_id: "t1", tool_name: "read", tool_args: { path: "/tmp/a" } }),
      runId: "run-1",
      idx: 1,
    });
    state = applyStreamEvent(state, {
      type: "tool_end",
      data: JSON.stringify({ tool_id: "t1", tool_name: "read" }),
      runId: "run-1",
      idx: 2,
    });
    state = applyStreamEvent(state, {
      type: "tool_start",
      data: JSON.stringify({ tool_id: "t2", tool_name: "read", tool_args: { path: "/tmp/b" } }),
      runId: "run-1",
      idx: 3,
    });
    state = applyStreamEvent(state, {
      type: "tool_end",
      data: JSON.stringify({ tool_id: "t2", tool_name: "read" }),
      runId: "run-1",
      idx: 4,
    });
    const reply = state.items.find(item => item.kind === "message" && item.role === "assistant");
    if (!reply || reply.kind !== "message") throw new Error("reply bubble missing");
    const toolSegment = reply.segments?.find(segment => segment.kind === "tool");
    expect(toolSegment && toolSegment.kind === "tool").toBe(true);
    if (toolSegment?.kind === "tool") {
      expect(toolSegment.tool.count).toBe(2);
      expect(toolSegment.tool.children?.map(child => child.name)).toEqual(["read", "read"]);
    }
  });
});

describe("run failure parity with the desktop", () => {
  test("an error event settles the run and pins the raw error on the assistant bubble", () => {
    let state = applyStreamEvent(emptyTimeline(), {
      type: "agent_start",
      data: "{}",
      runId: "run-1",
      idx: 0,
    });
    state = applyStreamEvent(state, {
      type: "error",
      data: JSON.stringify({ error: "Authentication failed (401). Check your API key." }),
      runId: "run-1",
      idx: 1,
    });
    // The failure lives on the assistant bubble (friendly text at render time),
    // not as a separate red notice.
    expect(state.items.filter(item => item.kind === "notice")).toHaveLength(0);
    const reply = state.items.find(item => item.kind === "message" && item.role === "assistant");
    if (!reply || reply.kind !== "message") throw new Error("assistant bubble missing");
    expect(reply.failed).toBe(true);
    expect(reply.streaming).toBe(false);
    expect(reply.error).toBe("Authentication failed (401). Check your API key.");
    expect(reply.text).toBe("");
  });

  test("an error event without any run id still surfaces as a danger notice", () => {
    const state = applyStreamEvent(emptyTimeline(), {
      type: "error",
      data: JSON.stringify({ error: "boom" }),
      idx: 0,
    });
    expect(state.items[0]).toMatchObject({ kind: "notice", tone: "danger", text: "boom" });
  });

  test("history reload rebuilds the failure bubble for a run with no assistant entry", () => {
    const timeline = timelineFromEntries([
      {
        id: "e1",
        kind: "user",
        role: "user",
        createdAtMs: 0,
        blocks: [{ kind: "text", text: "poem.txt 里面内容是什么" }],
        runId: "run-1",
        run: {
          status: "failed",
          error: "Authentication failed (401). Check your API key.",
          durationMs: 15_000,
        },
      },
    ]);
    expect(timeline.items).toEqual([
      expect.objectContaining({ kind: "message", role: "user" }),
      expect.objectContaining({
        id: "failed_run-1",
        kind: "message",
        role: "assistant",
        failed: true,
        error: "Authentication failed (401). Check your API key.",
        durationMs: 15_000,
      }),
    ]);
  });

  test("history reload anchors an empty-content failed run's bubble at the tail", () => {
    // A failed run whose user entry carries no text and no attachments leaves
    // no user item to anchor behind, so its failure bubble is appended unanchored.
    const timeline = timelineFromEntries([
      {
        id: "e1",
        kind: "user",
        role: "user",
        createdAtMs: 0,
        blocks: [{ kind: "text", text: "" }],
        runId: "run-1",
        run: { status: "failed" },
      },
    ]);
    expect(timeline.items).toEqual([
      expect.objectContaining({
        id: "failed_run-1",
        kind: "message",
        role: "assistant",
        failed: true,
      }),
    ]);
  });

  test("an error event before any other run event still produces the failed bubble", () => {
    const state = applyStreamEvent(emptyTimeline(), {
      type: "error",
      data: JSON.stringify({ error: "boom" }),
      runId: "run-1",
      idx: 0,
    });
    const reply = state.items.find(item => item.kind === "message" && item.role === "assistant");
    if (!reply || reply.kind !== "message") throw new Error("assistant bubble missing");
    expect(reply.failed).toBe(true);
    expect(reply.error).toBe("boom");
    expect(reply.streaming).toBe(false);
  });

  test("history reload keeps a partial assistant reply instead of a synthesized bubble", () => {
    const timeline = timelineFromEntries([
      {
        id: "e1",
        kind: "user",
        role: "user",
        createdAtMs: 0,
        blocks: [{ kind: "text", text: "hi" }],
        runId: "run-1",
        run: { status: "failed" },
      },
      {
        id: "e2",
        kind: "assistant",
        role: "assistant",
        createdAtMs: 0,
        blocks: [{ kind: "text", text: "partial" }],
        runId: "run-1",
        run: { status: "failed", error: "error decoding response body" },
      },
    ]);
    const assistants = timeline.items.filter(
      item => item.kind === "message" && item.role === "assistant",
    );
    expect(assistants).toHaveLength(1);
    expect(assistants[0]).toMatchObject({
      failed: true,
      text: "partial",
      error: "error decoding response body",
    });
  });
});
