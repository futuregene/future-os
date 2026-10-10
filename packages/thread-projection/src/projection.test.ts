import { describe, expect, it } from "vitest";
import { entriesToMessages, entriesToTurns, turnsToMessages } from "./projection";
import type { SessionEntry, MessageBlock } from "./events";
import type { MessageSegment } from "./model";

const BASE = 1_700_000_000_000;

function entry(
  id: string,
  role: SessionEntry["role"],
  extra: Partial<SessionEntry> = {},
): SessionEntry {
  return {
    blocks: [],
    createdAtMs: BASE,
    id,
    kind: role,
    role,
    ...extra,
  };
}

function textBlock(text: string): MessageBlock {
  return { kind: "text", text };
}

function toolCall(toolCallId: string, name: string, args: unknown): MessageBlock {
  return { arguments: args, kind: "tool_call", name, toolCallId };
}

function turnOf(entries: SessionEntry[]) {
  const nodes = entriesToTurns(entries);
  expect(nodes).toHaveLength(1);
  const node = nodes[0]!;
  if (node.kind !== "turn") throw new Error("expected a turn node");
  return node.turn;
}

describe("entriesToTurns — exchanges", () => {
  it("projects an exchange with ordered thinking, activity and text segments", () => {
    const turn = turnOf([
      entry("u1", "user", { blocks: [textBlock("what is here?")], runId: "r1" }),
      entry("a1", "assistant", {
        blocks: [
          { kind: "reasoning", text: "considering" },
          toolCall("t1", "read", { path: "/repo/a.md" }),
          { kind: "text", text: "it is a file" },
        ],
        runId: "r1",
      }),
      entry("t1r", "tool", { blocks: [{ isError: false, kind: "tool_result", toolCallId: "t1" }] }),
    ]);

    expect(turn.user).toMatchObject({
      content: "what is here?",
      id: "m_u1",
      role: "user",
      sourceEntryId: "u1",
    });
    expect(turn.assistant).toMatchObject({
      content: "it is a file",
      id: "m_a1",
      role: "assistant",
      runId: "r1",
    });
    expect(turn.assistant?.segments).toEqual<MessageSegment[]>([
      { id: "seg_a1_0", kind: "thinking", text: "considering" },
      {
        id: "seg_a1_1",
        item: { detail: "/repo/a.md", id: "t1", kind: "read", status: "completed", target: "/repo/a.md" },
        kind: "activity",
      },
      { id: "seg_a1_2", kind: "text", text: "it is a file" },
    ]);
    expect(turn.identitySource).toBe("canonical");
    expect(turn.runId).toBe("r1");
    expect(turn.key).toBe("m_u1");
  });

  it("collapses completed same-kind tool bursts, counting shell calls and unique edit targets", () => {
    const turn = turnOf([
      entry("u1", "user", { blocks: [textBlock("go")] }),
      entry("a1", "assistant", {
        blocks: [
          toolCall("s1", "shell", { command: "ls" }),
          toolCall("s2", "shell", { command: "pwd" }),
          toolCall("s3", "bash", { command: "date" }),
          toolCall("e1", "edit", { path: "/repo/a.md" }),
          toolCall("e2", "edit", { path: "/repo/a.md" }),
          toolCall("e3", "edit", { path: "/repo/b.md" }),
        ],
      }),
    ]);

    const activity = (turn.assistant?.segments ?? []).filter((segment) => segment.kind === "activity");
    expect(activity).toHaveLength(2);
    expect(activity[0]).toMatchObject({
      item: { count: 3, id: "collapsed_s1", kind: "shell", status: "completed" },
    });
    expect((activity[0] as { item: { children: unknown[] } }).item.children).toHaveLength(3);
    expect(activity[1]).toMatchObject({ item: { count: 2, id: "collapsed_e1", kind: "edit" } });
  });

  it("breaks a collapse on a whitespace-only text or a failed tool result", () => {
    const turn = turnOf([
      entry("u1", "user", { blocks: [textBlock("go")] }),
      entry("a1", "assistant", {
        blocks: [
          toolCall("e1", "edit", { path: "a.md" }),
          toolCall("e2", "edit", { path: "b.md" }),
        ],
      }),
      entry("a2", "assistant", { blocks: [{ kind: "text", text: "   " }, textBlock("done")] }),
      entry("e2r", "tool", { blocks: [{ isError: true, kind: "tool_result", toolCallId: "e2" }] }),
    ]);

    const segments = turn.assistant?.segments ?? [];
    // e1 is alone after e2 failed, and the whitespace text block produced nothing.
    expect(segments.map((segment) => segment.kind)).toEqual(["activity", "activity", "text"]);
    expect(segments[0]).toMatchObject({ item: { id: "e1", status: "completed" } });
    expect(segments[1]).toMatchObject({ item: { id: "e2", status: "failed" } });
  });

  it("ignores a tool result that matches no pending call and tool entries without a turn", () => {
    const orphanResult = entry("t9", "tool", { blocks: [{ kind: "tool_result", toolCallId: "missing" }] });
    expect(entriesToTurns([orphanResult])).toEqual([]);

    const turn = turnOf([
      entry("u1", "user", { blocks: [textBlock("go")] }),
      entry("a1", "assistant", { blocks: [toolCall("t1", "read", { path: "a.md" })] }),
      orphanResult,
    ]);
    expect(turn.assistant?.segments).toHaveLength(1);
  });

  it("flushes the previous exchange when a new user entry arrives", () => {
    const entries = [
      entry("u1", "user", { blocks: [textBlock("one")], runId: "r1" }),
      entry("a1", "assistant", { blocks: [textBlock("done one")], runId: "r1" }),
      entry("u2", "user", { blocks: [textBlock("two")], runId: "r2" }),
      entry("a2", "assistant", { blocks: [textBlock("done two")], runId: "r2" }),
    ];
    const nodes = entriesToTurns(entries);
    expect(nodes.map((node) => node.kind)).toEqual(["turn", "turn"]);
    expect(nodes.map((node) => (node.kind === "turn" ? node.turn.key : ""))).toEqual(["m_u1", "m_u2"]);
    expect(entriesToMessages(entries)).toHaveLength(4);
  });

  it("ignores tool-entry blocks that are not results", () => {
    const turn = turnOf([
      entry("u1", "user", { blocks: [textBlock("go")] }),
      entry("a1", "assistant", { blocks: [toolCall("t1", "read", { path: "a.md" })] }),
      entry("tr", "tool", {
        blocks: [{ kind: "text", text: "not a result" }, { isError: true, kind: "tool_result", toolCallId: "t1" }],
      }),
    ]);
    expect(turn.assistant?.segments[0]).toMatchObject({ item: { id: "t1", status: "failed" } });
  });

  it("drops an assistant turn that never saw a user entry", () => {
    expect(entriesToTurns([entry("a1", "assistant", { blocks: [textBlock("orphan")] })])).toEqual([]);
  });

  it("falls back to a generated id for entries without one", () => {
    const turn = turnOf([entry("", "user", { blocks: [textBlock("hi")] })]);
    expect(turn.user.id).toMatch(/^m_ep_\d+_\d+$/);
    expect(turn.user.sourceEntryId).toBe("");
  });
});

describe("entriesToTurns — attachments and entry text", () => {
  it("keeps only usable attachments and defaults the kind", () => {
    const turn = turnOf([
      entry("u1", "user", {
        blocks: [textBlock("see attached")],
        metadata: {
          attachments: [
            { name: "shot.png", kind: "image", path: "/tmp/shot.png", thumbnail: "/tmp/t.png" },
            { name: "no-path.txt", path: "" },
            { path: "/tmp/nameless.txt" },
            { name: "typed.txt", path: "/tmp/typed.txt" },
          ],
        },
      }),
    ]);
    expect(turn.user.attachments).toEqual([
      { kind: "image", name: "shot.png", path: "/tmp/shot.png", thumbnail: "/tmp/t.png" },
      { kind: "file", name: "typed.txt", path: "/tmp/typed.txt", thumbnail: null },
    ]);
  });

  it("omits attachments for empty, non-array and absent metadata", () => {
    expect(turnOf([entry("u1", "user", { metadata: { attachments: [] } })]).user.attachments).toBeUndefined();
    expect(turnOf([entry("u2", "user", { metadata: { attachments: "nope" } })]).user.attachments).toBeUndefined();
    expect(turnOf([entry("u3", "user", { metadata: null })]).user.attachments).toBeUndefined();
    expect(turnOf([entry("u4", "user")]).user.attachments).toBeUndefined();
  });

  it("names a divider after the current time when its timestamp is unusable", () => {
    const nodes = entriesToTurns([
      entry("cp1", "compaction", {
        checkpoint: { checkpointId: "cp1", phase: "standalone", schemaVersion: 2 },
        createdAtMs: Number.NaN,
      }),
    ]);
    const standalone = nodes[0]!;
    if (standalone.kind !== "standalone") throw new Error("expected standalone");
    expect(Number.isNaN(Date.parse(standalone.message.createdAt))).toBe(false);
    expect(standalone.message.id).toBe("m_cp1");
  });

  it("names a marker-only snapshot after the entry when the checkpoint has no id", () => {
    const turn = turnOf([
      entry("u1", "user", { blocks: [textBlock("go")] }),
      entry("cp1", "compaction", { checkpoint: { phase: "pre_turn", schemaVersion: 2 } }),
    ]);
    expect(turn.assistant).toMatchObject({ content: "", id: "m_cp1", status: "complete" });
    expect(turn.assistant?.segments?.[0]).toMatchObject({ id: "seg_cp1_compaction", kind: "compaction" });
  });

  it("ignores entries whose role is neither user, assistant nor tool", () => {
    const turn = turnOf([
      entry("s1", "system", { blocks: [textBlock("system note")] }),
      entry("u1", "user", { blocks: [textBlock("go")] }),
      entry("s2", "system", { blocks: [textBlock("more")] }),
      entry("a1", "assistant", { blocks: [textBlock("done")] }),
      entry("s3", "system", { blocks: [textBlock("trailing")] }),
    ]);
    expect(turn.user.content).toBe("go");
    expect(turn.assistant?.content).toBe("done");
    expect(turn.assistant?.segments).toEqual([{ id: "seg_a1_0", kind: "text", text: "done" }]);
  });

  it("treats a text block with no text as empty content", () => {
    const turn = turnOf([entry("u1", "user", { blocks: [{ kind: "text" }] })]);
    expect(turn.user.content).toBe("");
  });

  it("models a tool call that carries neither a name nor an id", () => {
    const turn = turnOf([
      entry("u1", "user", { blocks: [textBlock("go")] }),
      entry("a1", "assistant", { blocks: [{ arguments: { path: "a.md" }, kind: "tool_call" }, textBlock("done")] }),
    ]);
    // An unnamed tool is treated as a shell command and the segment id stands in
    // for the missing tool-call id.
    expect(turn.assistant?.segments?.[0]).toEqual({
      id: "seg_a1_0",
      item: { detail: undefined, id: "seg_a1_0", kind: "shell", status: "completed", target: undefined },
      kind: "activity",
    });
  });

  it("uses only the first text block of a user entry but all of an assistant's", () => {
    const turn = turnOf([
      entry("u1", "user", { blocks: [textBlock("first"), textBlock("second")] }),
      entry("a1", "assistant", { blocks: [textBlock("a"), textBlock("b")] }),
    ]);
    expect(turn.user.content).toBe("first");
    expect(turn.assistant?.content).toBe("b");
  });

  it("falls back to the current time when the entry has no usable timestamp", () => {
    const turn = turnOf([entry("u1", "user", { createdAtMs: Number.NaN })]);
    expect(Number.isNaN(Date.parse(turn.user.createdAt))).toBe(false);
  });
});

describe("entriesToTurns — run outcome", () => {
  it("carries the settled run status, error and duration onto the assistant reply", () => {
    const turn = turnOf([
      entry("u1", "user", { blocks: [textBlock("go")], runId: "r1" }),
      entry("a1", "assistant", {
        blocks: [textBlock("partial")],
        run: { durationMs: 1200, error: "boom", status: "failed" },
        runId: "r1",
        usage: { cacheReadTokens: 3, inputTokens: 10, outputTokens: 5 },
      }),
    ]);
    expect(turn.outcome).toEqual({ durationMs: 1200, error: "boom", status: "failed" });
    expect(turn.assistant).toMatchObject({
      cacheReadTokens: 3,
      durationMs: 1200,
      inputTokens: 10,
      outputTokens: 5,
      runError: "boom",
      status: "failed",
    });
  });

  it("ignores a blank error, a negative duration and a run with no status", () => {
    const turn = turnOf([
      entry("u1", "user", { blocks: [textBlock("go")], runId: "r1" }),
      entry("a1", "assistant", {
        blocks: [textBlock("ok")],
        run: { durationMs: -1, error: "  ", status: "completed" },
        runId: "r1",
      }),
      entry("a2", "assistant", { blocks: [textBlock("more")], run: { status: null }, runId: "r1" }),
    ]);
    expect(turn.outcome).toEqual({ status: "completed" });
  });

  it("refuses an outcome whose run id is not the exchange's and reports the conflict", () => {
    const turn = turnOf([
      entry("u1", "user", { blocks: [textBlock("go")], runId: "r1" }),
      entry("a1", "assistant", { blocks: [textBlock("ok")], run: { status: "completed" }, runId: "other" }),
    ]);
    expect(turn.outcome).toBeUndefined();
    // `foldRunOutcome` refuses the mismatched status, and the turn reports the
    // conflict rather than attributing either id.
    expect(turn.identitySource).toBe("conflict");
    expect(turn.runId).toBeUndefined();
  });

  it("reports a canonical conflict and drops the run identity", () => {
    const turn = turnOf([
      entry("u1", "user", { blocks: [textBlock("go")], runId: "r1" }),
      entry("a1", "assistant", {
        blocks: [textBlock("ok")],
        run: { status: "completed" },
        runId: "r2",
      }),
    ]);
    expect(turn.identitySource).toBe("conflict");
    expect(turn.runId).toBeUndefined();
    expect(turn.outcome).toBeUndefined();
    expect(turn.assistant?.runId).toBeUndefined();
  });

  it("keeps a terminal run that has no reply text", () => {
    const failed = turnOf([
      entry("u1", "user", { blocks: [textBlock("go")], runId: "r1" }),
      entry("a1", "assistant", { blocks: [], run: { status: "failed" }, runId: "r1" }),
    ]);
    expect(failed.assistant).toMatchObject({ content: "", id: "m_a1", status: "failed" });
    expect(failed.assistant?.segments).toBeUndefined();

    const stopped = turnOf([
      entry("u2", "user", { blocks: [textBlock("go")], runId: "r2" }),
      entry("a2", "assistant", { blocks: [], run: { status: "cancelled" }, runId: "r2" }),
    ]);
    expect(stopped.assistant).toMatchObject({ id: "m_a2", status: "complete", stopped: true });
  });

  it("synthesizes an id for an aborted run with no assistant entry", () => {
    const turn = turnOf([
      entry("u1", "user", { blocks: [textBlock("go")], run: { status: "interrupted" }, runId: "r1" }),
    ]);
    expect(turn.assistant).toMatchObject({ content: "", id: "failed_r1", status: "failed" });
  });

  it("synthesizes a stopped id when the outcome is not a failure", () => {
    const turn = turnOf([
      entry("u1", "user", { blocks: [textBlock("go")], run: { status: "cancelled" }, runId: "r1" }),
    ]);
    expect(turn.assistant).toMatchObject({ id: "stopped_r1", stopped: true });
  });

  it("stores usage and duration without any text", () => {
    const turn = turnOf([
      entry("u1", "user", { blocks: [textBlock("go")] }),
      entry("a1", "assistant", {
        blocks: [],
        run: { durationMs: 42 },
        usage: { outputTokens: 7 },
      }),
    ]);
    expect(turn.assistant).toMatchObject({ content: "", durationMs: 42, outputTokens: 7 });
    expect(turn.assistant?.segments).toBeUndefined();
    expect(turn.assistant?.id).toBe("m_a1");
  });

  it("omits the assistant entirely for an unfinished exchange", () => {
    const turn = turnOf([entry("u1", "user", { blocks: [textBlock("still streaming")] })]);
    expect(turn.assistant).toBeUndefined();
    expect(turn.identitySource).toBe("legacy");
  });
});

describe("entriesToTurns — compaction dividers", () => {
  it("keeps an automatic in-turn checkpoint inline in the running reply", () => {
    const turn = turnOf([
      entry("u1", "user", { blocks: [textBlock("go")], runId: "r1" }),
      entry("cp1", "compaction", {
        checkpoint: { checkpointId: "cp1", phase: "pre_turn", schemaVersion: 2, tokensAfter: 20, tokensBefore: 100, trigger: "auto" },
        kind: "compaction",
      }),
      entry("a1", "assistant", { blocks: [textBlock("after")] }),
    ]);
    expect(turn.assistant?.segments).toEqual<MessageSegment[]>([
      { checkpointId: "cp1", id: "seg_cp1_compaction", kind: "compaction", tokensAfter: 20, tokensBefore: 100, trigger: "auto" },
      { id: "seg_a1_0", kind: "text", text: "after" },
    ]);
  });

  it("treats a newer checkpoint schema as in-turn too", () => {
    const turn = turnOf([
      entry("u1", "user", { blocks: [textBlock("go")] }),
      entry("cp", "compaction", { checkpoint: { checkpointId: "cp9", schemaVersion: 3 } }),
    ]);
    expect(turn.assistant?.segments?.[0]).toMatchObject({ checkpointId: "cp9", kind: "compaction" });
  });

  it("names a marker-only snapshot after the checkpoint", () => {
    const turn = turnOf([
      entry("u1", "user", { blocks: [textBlock("go")] }),
      entry("cp1", "compaction", {
        checkpoint: { checkpointId: "cp1", phase: "mid_turn", schemaVersion: 2 },
      }),
    ]);
    expect(turn.assistant).toMatchObject({ content: "", id: "m_cp1", status: "complete" });
  });

  it("pushes a standalone divider as its own node for every non-in-turn shape", () => {
    const cases: Array<[string, SessionEntry]> = [
      ["standalone phase", entry("c1", "compaction", { checkpoint: { phase: "standalone", schemaVersion: 2 } })],
      ["manual trigger", entry("c2", "compaction", { checkpoint: { schemaVersion: 2, trigger: "manual" } })],
      ["legacy schema", entry("c3", "compaction", { checkpoint: { schemaVersion: 1 } })],
      ["no checkpoint", entry("c4", "compaction", {})],
    ];
    for (const [label, divider] of cases) {
      const nodes = entriesToTurns([entry("u1", "user", { blocks: [textBlock("go")] }), divider]);
      expect(nodes.map((node) => node.kind), label).toEqual(["turn", "standalone"]);
      const standalone = nodes[1]!;
      if (standalone.kind !== "standalone") throw new Error("expected standalone");
      expect(standalone.message.role, label).toBe("assistant");
      expect(standalone.message.segments?.[0]?.kind, label).toBe("compaction");
    }
  });

  it("accepts a legacy '[Context compaction:' user entry as a divider", () => {
    const nodes = entriesToTurns([
      entry("u1", "user", { blocks: [textBlock("[Context compaction: summary]")] }),
    ]);
    expect(nodes).toHaveLength(1);
    expect(nodes[0]!.kind).toBe("standalone");
    const standalone = nodes[0]!;
    if (standalone.kind !== "standalone") throw new Error("expected standalone");
    expect(standalone.message.id).toBe("m_u1");
  });

  it("omits zero/negative token counts and an absent trigger", () => {
    const nodes = entriesToTurns([
      entry("c1", "compaction", { checkpoint: { phase: "standalone", schemaVersion: 2, tokensAfter: 0, tokensBefore: -1 } }),
    ]);
    const standalone = nodes[0]!;
    if (standalone.kind !== "standalone") throw new Error("expected standalone");
    expect(standalone.message.segments?.[0]).toEqual({ id: "seg_c1_compaction", kind: "compaction" });
  });
});

describe("turnsToMessages / entriesToMessages", () => {
  it("flattens turns and standalone messages in order", () => {
    const nodes = entriesToTurns([
      entry("u1", "user", { blocks: [textBlock("go")] }),
      entry("a1", "assistant", { blocks: [textBlock("done")] }),
      entry("c1", "compaction", { checkpoint: { phase: "standalone", schemaVersion: 2 } }),
    ]);
    expect(turnsToMessages(nodes).map((message) => message.id)).toEqual(["m_u1", "m_a1", "m_c1"]);
    expect(entriesToMessages([
      entry("u1", "user", { blocks: [textBlock("go")] }),
      entry("a1", "assistant", { blocks: [textBlock("done")] }),
    ]).map((message) => message.role)).toEqual(["user", "assistant"]);
  });

  it("returns nothing for no entries", () => {
    expect(entriesToMessages([])).toEqual([]);
    expect(turnsToMessages([])).toEqual([]);
  });
});
