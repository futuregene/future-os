import { describe, expect, it } from "vitest";
import {
  buildAssistantRunProjection,
  createRunProjector,
  isSoftExit,
  nonZeroExitCode,
} from "./liveApply";
import type { AssistantRunProjection } from "./liveApply";
import type { RunEvent } from "./events";

let counter = 0;

function event(eventType: string, payload?: unknown, overrides: Partial<RunEvent> = {}): RunEvent {
  counter += 1;
  return {
    createdAt: counter,
    eventType,
    id: `e${counter}`,
    payload: payload === undefined ? null : JSON.stringify(payload),
    runId: "r1",
    sequence: counter,
    ...overrides,
  };
}

/** Compact, assertion-friendly view of the ordered segments. */
function shape(projection: AssistantRunProjection): string[] {
  return projection.segments.map((segment) => {
    if (segment.kind === "text") return `text(${segment.text})`;
    if (segment.kind === "thinking") return `thinking(${segment.text})`;
    if (segment.kind === "compaction") {
      return `compaction(${segment.id},${segment.status ?? "completed"}${segment.error ? `,${segment.error}` : ""})`;
    }
    return `activity(${segment.item.kind},${segment.item.status}${segment.item.count ? `x${segment.item.count}` : ""})`;
  });
}

describe("text and thinking", () => {
  it("keeps reasoning and answer text in production order", () => {
    const projection = buildAssistantRunProjection([
      event("thinking_start", { blockId: "b1" }),
      event("thinking_delta", { blockId: "b1", text: "why " }),
      event("thinking_delta", { blockId: "b1", text: "not" }),
      event("thinking_end", { blockId: "b1" }),
      event("text_chunk", { text: "answer" }),
    ]);
    expect(shape(projection)).toEqual(["thinking(why not)", "text(answer)"]);
    expect(projection.thinking).toBe("why not");
    expect(projection.content).toBe("answer");
    expect(projection.thinkingActive).toBe(false);
    expect(projection.truncated).toBe(false);
    expect(projection.stopped).toBe(false);
  });

  it("reads text from `text`, `delta` and `content` spellings and ignores other payloads", () => {
    const projection = buildAssistantRunProjection([
      event("text_chunk", { delta: "a" }),
      event("text_chunk", { content: "b" }),
      event("text_chunk", { text: "c" }),
      event("text_chunk", 42),
      event("text_chunk", "raw string"),
    ]);
    expect(projection.content).toBe("abc");
  });

  it("reports thinkingActive only while nothing visible has arrived", () => {
    const thinking = buildAssistantRunProjection([event("thinking_start", { block_id: "b1" })]);
    expect(thinking.thinkingActive).toBe(true);
    expect(thinking.segments).toEqual([{ id: "thinking_0", kind: "thinking", text: "" }]);

    const withTool = buildAssistantRunProjection([
      event("thinking_start", { blockId: "b1" }),
      event("toolcall_start", { tool_name: "read", tool_args: { path: "a.md" } }),
      event("thinking_delta", { blockId: "b1", text: "still reasoning" }),
    ]);
    expect(withTool.thinkingActive).toBe(false);

    // Whitespace-only text does not count as visible work.
    const whitespace = buildAssistantRunProjection([
      event("thinking_start", { blockId: "b1" }),
      event("text_chunk", { text: "   " }),
    ]);
    expect(whitespace.thinkingActive).toBe(true);
  });

  it("opens a reasoning slot lazily for a delta without a start", () => {
    const projection = buildAssistantRunProjection([event("thinking_delta", { blockId: "b9", text: "lazy" })]);
    expect(shape(projection)).toEqual(["thinking(lazy)"]);
    expect(projection.thinking).toBe("lazy");
  });

  it("annexes a late delta to its original block", () => {
    const projection = buildAssistantRunProjection([
      event("thinking_start", { blockId: "b1" }),
      event("thinking_start", { blockId: "b2" }),
      event("thinking_delta", { blockId: "b1", text: "late" }),
      event("thinking_end", { blockId: "b2" }),
    ]);
    expect(projection.thinking).toBe("late");
    // b1 is still active, so the model is not done reasoning.
    expect(projection.thinkingActive).toBe(true);
    expect(shape(projection)).toEqual(["thinking(late)", "thinking()"]);
  });

  it("keeps the open block when a different block id ends", () => {
    const projection = buildAssistantRunProjection([
      event("thinking_start", { blockId: "b1" }),
      event("thinking_delta", { blockId: "b1", text: "one" }),
      event("thinking_start", { blockId: "b2" }),
      event("thinking_end", { blockId: "b1" }),
      event("thinking_delta", { blockId: "b1", text: "two" }),
    ]);
    // b1 and b2 are separate blocks joined by a blank line; the late delta
    // rejoined b1 rather than being appended to the open b2.
    expect(projection.thinking).toBe("one\n\ntwo");
  });

  it("clears every active block when thinking_end carries no id", () => {
    const projection = buildAssistantRunProjection([
      event("thinking_start", { blockId: "b1" }),
      event("thinking_start", { blockId: "b2" }),
      event("thinking_end"),
      event("text_chunk", { text: "done" }),
    ]);
    expect(projection.thinkingActive).toBe(false);
    expect(projection.thinking).toBe("");
  });

  it("opts out of the end-token preference when the run reported nothing", () => {
    // `preferEndTokens` takes `agentEndOutput || usageOutputSum`; with neither
    // present the reported total is a plain 0 rather than NaN/undefined.
    const projector = createRunProjector({ preferEndTokens: true });
    expect(projector.ingest([event("agent_end", {})]).outputTokens).toBe(0);
    expect(projector.ingest([event("usage", { usage: { completion_tokens: 2 } })]).outputTokens).toBe(2);
  });

  it("ignores a delta with no text", () => {
    const projection = buildAssistantRunProjection([
      event("thinking_start", { blockId: "b1" }),
      event("thinking_delta", { blockId: "b1" }),
    ]);
    expect(projection.thinking).toBe("");
  });

  it("opens an anonymous reasoning block when no block id is given", () => {
    const projection = buildAssistantRunProjection([
      event("thinking_start", {}),
      event("thinking_delta", { text: "reasoning" }),
      event("text_chunk", { text: "answer" }),
    ]);
    expect(shape(projection)).toEqual(["thinking(reasoning)", "text(answer)"]);
    expect(projection.thinking).toBe("reasoning");
  });

  it("lazily opens an anonymous reasoning block for a bare delta", () => {
    // Neither a `thinking_start` nor a block id: the delta opens the block.
    // `thinkingActive` stays false on purpose — it is driven by the
    // `thinking_start` lifecycle flag, not by a block's presence.
    const projection = buildAssistantRunProjection([event("thinking_delta", { text: "lazy" })]);
    expect(shape(projection)).toEqual(["thinking(lazy)"]);
    expect(projection.thinking).toBe("lazy");
    expect(projection.thinkingActive).toBe(false);
  });

  it("leaves a completed reply alone when the run ends", () => {
    // `agent_end` walks the slots to fail a pending compaction divider; a slot
    // that is not a running divider must be left as it is.
    const projection = buildAssistantRunProjection([
      event("text_chunk", { text: "done" }),
      event("agent_end", {}),
    ]);
    expect(shape(projection)).toEqual(["text(done)"]);
    expect(projection.truncated).toBe(false);
  });
});

describe("usage and terminal events", () => {
  it("sums per-call usage and prefers it over the agent_end total", () => {
    const projection = buildAssistantRunProjection([
      event("usage", { usage: { completion_tokens: 3 } }),
      event("usage", { output_tokens: 4 }),
      event("usage", {}),
      event("agent_end", { usage: { completion_tokens: 100 } }),
    ]);
    expect(projection.outputTokens).toBe(7);
  });

  it("prefers the agent_end total when the client opted in (late joiners)", () => {
    const projector = createRunProjector({ preferEndTokens: true });
    const projection = projector.ingest([
      event("usage", { usage: { completion_tokens: 3 } }),
      event("agent_end", { usage: { completion_tokens: 100 } }),
    ]);
    expect(projection.outputTokens).toBe(100);
  });

  it("falls back to the agent_end total when no usage event was streamed", () => {
    const projection = buildAssistantRunProjection([event("agent_end", { output_tokens: 9 })]);
    expect(projection.outputTokens).toBe(9);
  });

  it("flags a truncated stream, a cancelled run and neither for an empty end", () => {
    expect(buildAssistantRunProjection([event("agent_end", { reason: "incomplete" })]).truncated).toBe(true);
    expect(buildAssistantRunProjection([event("agent_end", { state: "cancelled" })]).stopped).toBe(true);
    expect(buildAssistantRunProjection([event("agent_end", null)]).truncated).toBe(false);
    expect(buildAssistantRunProjection([event("agent_end", { reason: "stop" })]).stopped).toBe(false);
  });

  it("reports and clears a transient reconnect state", () => {
    const withRetry = buildAssistantRunProjection([
      event("stream_retry", { attempt: 2, delayMs: 500, maxRetries: 3 }),
      event("agent_end", {}),
    ]);
    expect(withRetry.reconnecting).toBeUndefined();

    const midRetry = buildAssistantRunProjection([
      event("stream_retry", { attempt: 2, delayMs: 500, maxRetries: 3 }),
    ]);
    expect(midRetry.reconnecting).toEqual({ attempt: 2, delayMs: 500, maxRetries: 3 });

    const resumed = buildAssistantRunProjection([
      event("stream_retry", { attempt: 1, delayMs: 0, maxRetries: 1 }),
      event("stream_resumed"),
    ]);
    expect(resumed.reconnecting).toBeUndefined();

    const errored = buildAssistantRunProjection([
      event("stream_retry", { attempt: 1, delayMs: 0, maxRetries: 1 }),
      event("error", "boom"),
    ]);
    expect(errored.reconnecting).toBeUndefined();
  });

  it.each<[unknown]>([
    [{}],
    [null],
    [{ attempt: 0, delayMs: 0, maxRetries: 1 }],
    [{ attempt: 2, delayMs: 0, maxRetries: 1 }],
    [{ attempt: 1, delayMs: -1, maxRetries: 1 }],
    [{ attempt: 1.5, delayMs: 0, maxRetries: 2 }],
    [{ attempt: 1, delayMs: 0, maxRetries: 2.5 }],
  ])("ignores a malformed stream_retry payload %j", (payload) => {
    expect(buildAssistantRunProjection([event("stream_retry", payload)]).reconnecting).toBeUndefined();
  });
});

describe("compaction lifecycle", () => {
  it("tracks a pending divider from start to commit", () => {
    const projection = buildAssistantRunProjection([
      event("compaction_started", { operation_id: "op1", trigger: "auto" }),
      event("compaction_committed", {
        checkpoint_id: "cp1",
        operation_id: "op1",
        tokens_after: 20,
        tokens_before: 100,
        trigger: "auto",
      }),
    ]);
    expect(shape(projection)).toEqual(["compaction(cp1,completed)"]);
    expect(projection.segments[0]).toEqual({
      checkpointId: "cp1",
      id: "cp1",
      kind: "compaction",
      tokensAfter: 20,
      tokensBefore: 100,
      trigger: "auto",
    });
  });

  it("names a started divider without an operation id and keeps it running", () => {
    const projection = buildAssistantRunProjection([event("compaction_started", { trigger: "manual" })]);
    expect(shape(projection)).toEqual([`compaction(${projection.segments[0]!.id},running)`]);
    expect(projection.segments[0]).toMatchObject({ id: expect.stringMatching(/^pending_r1_\d+$/) });
    expect(projection.segments[0]).toMatchObject({ trigger: "manual" });
    // A repeated start for the same operation is ignored.
    const repeated = buildAssistantRunProjection([
      event("compaction_started", { operation_id: "op1" }),
      event("compaction_started", { operation_id: "op1" }),
    ]);
    expect(repeated.segments).toHaveLength(1);
  });

  it("creates a divider for a commit without a preceding start", () => {
    const projection = buildAssistantRunProjection([
      event("compaction_end", { checkpoint_id: "cp2", tokensAfter: 5, tokensBefore: 50 }),
    ]);
    expect(shape(projection)).toEqual(["compaction(cp2,completed)"]);
    // A second commit for the same checkpoint is a no-op.
    const duplicate = buildAssistantRunProjection([
      event("compaction_end", { checkpoint_id: "cp2" }),
      event("compaction_end", { checkpoint_id: "cp2" }),
    ]);
    expect(duplicate.segments).toHaveLength(1);
  });

  it("falls back to a legacy divider for a commit with no payload", () => {
    const projection = buildAssistantRunProjection([event("compaction_end")]);
    expect(projection.segments).toHaveLength(1);
    expect(projection.segments[0]).toMatchObject({ id: expect.stringMatching(/^legacy_r1_\d+$/), kind: "compaction" });
    // A completed divider omits the status key entirely.
    expect(projection.segments[0]).not.toHaveProperty("status");
  });

  it("ignores an aborted commit", () => {
    const projection = buildAssistantRunProjection([
      event("compaction_started", { operation_id: "op1" }),
      event("compaction_committed", { aborted: true, operation_id: "op1" }),
    ]);
    expect(shape(projection)).toEqual(["compaction(op1,running)"]);
  });

  it("reads the camelCase token spellings too", () => {
    const projection = buildAssistantRunProjection([
      event("compaction_end", { checkpoint_id: "cp3", tokensAfter: 7, tokensBefore: 70 }),
    ]);
    expect(projection.segments[0]).toMatchObject({ tokensAfter: 7, tokensBefore: 70 });
  });

  it("removes a pending divider when compaction reports no change", () => {
    const projection = buildAssistantRunProjection([
      event("compaction_started", { operation_id: "op1" }),
      event("compaction_unchanged", { operation_id: "op1" }),
    ]);
    expect(projection.segments).toEqual([]);
    // Unknown and malformed payloads leave other slots alone.
    const untouched = buildAssistantRunProjection([
      event("compaction_started", { operation_id: "op1" }),
      event("compaction_unchanged", { operation_id: "other" }),
      event("compaction_unchanged", null),
    ]);
    expect(shape(untouched)).toEqual(["compaction(op1,running)"]);
  });

  it("marks the pending divider failed on compaction_failed", () => {
    const projection = buildAssistantRunProjection([
      event("compaction_started", { operation_id: "op1" }),
      event("compaction_failed", { error: "no room", operation_id: "op1" }),
    ]);
    expect(shape(projection)).toEqual(["compaction(op1,failed,no room)"]);
  });

  it("pushes a failed divider when there was no pending one", () => {
    const withError = buildAssistantRunProjection([event("compaction_failed", { error: "boom", trigger: "manual" })]);
    const withoutError = buildAssistantRunProjection([event("compaction_failed")]);
    expect(withError.segments[0]).toMatchObject({
      error: "boom",
      id: expect.stringMatching(/^failed_r1_\d+$/),
      status: "failed",
      trigger: "manual",
    });
    expect(withoutError.segments[0]).toMatchObject({
      id: expect.stringMatching(/^failed_r1_\d+$/),
      status: "failed",
    });
    expect(withoutError.segments[0]).not.toHaveProperty("error");
  });

  it("completes a pending divider that reports neither a checkpoint id nor a trigger", () => {
    const projection = buildAssistantRunProjection([
      event("compaction_started", { operation_id: "op1", trigger: "auto" }),
      event("compaction_committed", { operation_id: "op1", tokens_after: 4, tokens_before: 40 }),
    ]);
    expect(projection.segments).toHaveLength(1);
    expect(projection.segments[0]).toMatchObject({
      // Without a checkpoint id the divider is re-identified under the legacy
      // scheme, and the operation id is what linked it to the pending slot.
      id: expect.stringMatching(/^legacy_r1_\d+$/),
      tokensAfter: 4,
      tokensBefore: 40,
      // The trigger survives from the start event when the commit omits it.
      trigger: "auto",
    });
    // A completed divider omits the status key entirely.
    expect(projection.segments[0]).not.toHaveProperty("status");
    expect(projection.segments[0]).not.toHaveProperty("checkpointId");
    expect(projection.segments[0]).not.toHaveProperty("error");
  });

  it("carries the trigger of a commit that had no pending divider", () => {
    const projection = buildAssistantRunProjection([
      event("compaction_committed", { checkpoint_id: "cp5", tokens_after: 3, tokens_before: 30, trigger: "manual" }),
    ]);
    expect(projection.segments[0]).toMatchObject({ checkpointId: "cp5", id: "cp5", trigger: "manual" });
  });

  it("marks a still-running divider as interrupted when the run ends", () => {
    const projection = buildAssistantRunProjection([
      event("compaction_started", { operation_id: "op1" }),
      event("agent_end", {}),
    ]);
    expect(shape(projection)).toEqual(["compaction(op1,failed,compaction interrupted before completion)"]);
  });

  it("breaks a tool run when a divider sits between the calls", () => {
    const projection = buildAssistantRunProjection([
      event("toolcall_start", { tool_args: { path: "a.md" }, tool_id: "t1", tool_name: "read" }),
      event("tool_end", { tool_id: "t1" }),
      event("compaction_end", { checkpoint_id: "cp1" }),
      event("toolcall_start", { tool_args: { path: "b.md" }, tool_id: "t2", tool_name: "read" }),
      event("tool_end", { tool_id: "t2" }),
    ]);
    expect(shape(projection)).toEqual(["activity(read,completed)", "compaction(cp1,completed)", "activity(read,completed)"]);
  });
});

describe("tool activity", () => {
  it("streams a tool's target from partial arguments and completes it", () => {
    const projector = createRunProjector();
    projector.append(event("toolcall_start", { tool_name: "write", tool_id: "t1" }));
    projector.append(event("toolcall_delta", { text: '{"path": "docs/a.md", "content": "half' }));
    expect(shape(projector.snapshot())).toEqual(["activity(write,running)"]);
    expect(projector.snapshot().segments[0]).toMatchObject({ item: { detail: "docs/a.md", target: "docs/a.md" } });

    const finished = projector.ingest([event("tool_end", { tool_id: "t1" })]);
    expect(shape(finished)).toEqual(["activity(write,completed)"]);
  });

  it("replaces the accumulated arguments when a delta is a snapshot", () => {
    const projector = createRunProjector();
    projector.append(event("tool_start", { tool_args: { path: "a.md" }, tool_name: "read", tool_id: "t1" }));
    projector.append(event("tool_delta", { snapshot: true, text: '{"path":"b.md"}' }));
    expect(projector.snapshot().segments[0]).toMatchObject({ item: { target: "b.md" } });
  });

  it("finds the target in a partial argument string and ignores an invalid escape", () => {
    const projector = createRunProjector();
    projector.append(event("tool_start", { tool_name: "read", tool_id: "t1" }));
    // Incomplete JSON: the path is complete even though the object is not.
    projector.append(event("tool_delta", { text: '{"path": "docs/a.md", "content": "' }));
    expect(projector.snapshot().segments[0]).toMatchObject({ item: { target: "docs/a.md" } });

    const bad = createRunProjector();
    bad.append(event("tool_start", { tool_name: "shell", tool_id: "t2" }));
    bad.append(event("tool_delta", { text: String.raw`{"command": "a\qb"}` }));
    expect(bad.snapshot().segments[0]).toMatchObject({ item: { kind: "shell" } });
  });

  it("ignores deltas without an active tool and unknown tool starts", () => {
    const projection = buildAssistantRunProjection([
      event("tool_delta", { text: "orphan" }),
      event("toolcall_start", { tool_name: "thinking" }),
      event("toolcall_start", 42),
      event("toolcall_start", { tool_name: "edit" }),
      event("tool_delta", { text: "{}" }),
    ]);
    expect(shape(projection)).toEqual(["activity(edit,running)"]);
  });

  it("uses the tool id, its aliases, or a synthesized id", () => {
    const byAlias = buildAssistantRunProjection([
      event("tool_start", { toolID: "x1", tool_name: "read" }),
      event("tool_end", { tool_call_id: "x1" }),
    ]);
    expect(shape(byAlias)).toEqual(["activity(read,completed)"]);

    const synthesized = buildAssistantRunProjection([event("tool_start", { tool_name: "read" })]);
    expect(synthesized.segments[0]).toMatchObject({ item: { id: expect.stringMatching(/^read_\d+$/) } });
  });

  it("completes a tool by explicit id even when the result carries no tool name", () => {
    const projection = buildAssistantRunProjection([
      event("tool_start", { tool_args: { path: "a.md" }, tool_name: "edit", tool_id: "t1" }),
      event("tool_end", { tool_id: "t1" }),
    ]);
    expect(shape(projection)).toEqual(["activity(edit,completed)"]);
    expect(projection.segments[0]).toMatchObject({ item: { target: "a.md" } });

    // A result that names no known tool and no tracked id is dropped.
    expect(shape(buildAssistantRunProjection([event("tool_end", { tool_id: "ghost" })]))).toEqual([]);
    expect(shape(buildAssistantRunProjection([event("tool_end", 42)]))).toEqual([]);
  });

  it("completes the latest running tool of the same kind when no id is given", () => {
    const projection = buildAssistantRunProjection([
      event("tool_start", { tool_args: { path: "a.md" }, tool_name: "read" }),
      event("tool_start", { tool_args: { path: "b.md" }, tool_name: "read" }),
      event("tool_end", { tool_name: "read" }),
    ]);
    expect(shape(projection)).toEqual(["activity(read,running)", "activity(read,completed)"]);
  });

  it("keeps a single slot for a tool that reports its start twice", () => {
    const projection = buildAssistantRunProjection([
      event("tool_start", { tool_args: { path: "a.md" }, tool_id: "t1", tool_name: "read" }),
      event("toolcall_start", { tool_args: { path: "b.md" }, tool_id: "t1", tool_name: "read" }),
    ]);
    expect(projection.segments).toHaveLength(1);
    expect(projection.segments[0]).toMatchObject({ item: { id: "t1", target: "b.md" } });
  });

  it("clears the active tool only when the result belongs to it", () => {
    // Both results omit the tool name, so each is resolved by its explicit id:
    // the first is for the older tool (not the active one), the second for it.
    // The first fails, which also keeps the two rows from collapsing, so both
    // statuses stay observable.
    const projection = buildAssistantRunProjection([
      event("tool_start", { tool_args: { path: "a.md" }, tool_id: "t1", tool_name: "read" }),
      event("tool_start", { tool_args: { path: "b.md" }, tool_id: "t2", tool_name: "read" }),
      event("tool_end", { error: "boom", tool_id: "t1" }),
      event("tool_end", { tool_id: "t2" }),
    ]);
    expect(shape(projection)).toEqual(["activity(read,failed)", "activity(read,completed)"]);
    expect(
      projection.segments.map((segment) =>
        segment.kind === "activity" ? [segment.item.id, segment.item.status] : null,
      ),
    ).toEqual([["t1", "failed"], ["t2", "completed"]]);
  });

  it("fails a named result that arrives with no preceding start", () => {
    const projection = buildAssistantRunProjection([
      event("tool_end", { error: "boom", tool_args: { command: "ls" }, tool_id: "t9", tool_name: "shell" }),
    ]);
    expect(shape(projection)).toEqual(["activity(shell,failed)"]);
    expect(projection.segments[0]).toMatchObject({ item: { id: "t9" } });
  });

  it("slots a result that never had a start", () => {
    const projection = buildAssistantRunProjection([
      event("tool_end", { tool_args: { path: "a.md" }, tool_id: "t9", tool_name: "read" }),
    ]);
    expect(shape(projection)).toEqual(["activity(read,completed)"]);
    expect(projection.segments[0]).toMatchObject({ item: { id: "t9" } });
  });

  it("marks a tool failed from an error field, a structured exit code or an output footer", () => {
    const failedFromError = buildAssistantRunProjection([
      event("tool_start", { tool_name: "shell", tool_id: "t1" }),
      event("tool_end", { error: "command failed", tool_id: "t1" }),
    ]);
    expect(shape(failedFromError)).toEqual(["activity(shell,failed)"]);

    const failedFromExit = buildAssistantRunProjection([
      event("tool_start", { tool_name: "shell", tool_id: "t2" }),
      event("tool_end", { exitCode: 2, tool_id: "t2" }),
    ]);
    expect(shape(failedFromExit)).toEqual(["activity(shell,failed)"]);

    const failedFromFooter = buildAssistantRunProjection([
      event("tool_start", { tool_args: { command: "npm test" }, tool_name: "shell", tool_id: "t3" }),
      event("tool_end", { text: "output\n[exit: 1]", tool_id: "t3" }),
    ]);
    expect(shape(failedFromFooter)).toEqual(["activity(shell,failed)"]);
  });

  it.each<[Record<string, unknown>, string]>([
    [{ exit_code: 0 }, "completed"],
    [{ exitCode: 0 }, "completed"],
    [{ is_soft_fail: true, exit_code: 1 }, "completed"],
    [{ exitCode: 1, isSoftFail: true }, "completed"],
    [{ errorText: "   " }, "completed"],
    [{ text: "no footer here" }, "completed"],
    [{ text: "done\n[exit: 0]" }, "completed"],
    [{ result: "x\n[exit: 3]" }, "failed"],
  ])("classifies a tool result payload %j as %s", (payload, expected) => {
    const projection = buildAssistantRunProjection([
      event("tool_start", { tool_args: { command: "npm test" }, tool_name: "shell", tool_id: "t1" }),
      event("tool_end", { ...payload, tool_id: "t1" }),
    ]);
    expect(shape(projection)).toEqual([`activity(shell,${expected})`]);
  });

  it("exempts a bare soft-fail command's exit 1 from failure", () => {
    const soft = buildAssistantRunProjection([
      event("tool_start", { tool_args: { command: "grep -q none ." }, tool_name: "shell", tool_id: "t1" }),
      event("tool_end", { exit_code: 1, tool_id: "t1" }),
    ]);
    expect(shape(soft)).toEqual(["activity(shell,completed)"]);
  });

  it("keeps the streamed target when the result carries none", () => {
    const projection = buildAssistantRunProjection([
      event("tool_start", { tool_args: { path: "a.md" }, tool_name: "edit", tool_id: "t1" }),
      event("tool_end", { tool_name: "edit", tool_id: "t1" }),
    ]);
    expect(projection.segments[0]).toMatchObject({ item: { detail: "a.md", target: "a.md" } });
  });

  it("collapses a burst of completed same-kind calls in the flat activity list", () => {
    const projection = buildAssistantRunProjection([
      event("tool_start", { tool_args: { command: "ls" }, tool_name: "shell", tool_id: "s1" }),
      event("tool_end", { tool_id: "s1" }),
      event("tool_start", { tool_args: { command: "pwd" }, tool_name: "shell", tool_id: "s2" }),
      event("tool_end", { tool_id: "s2" }),
      event("tool_start", { tool_args: { path: "a.md" }, tool_name: "edit", tool_id: "e1" }),
      event("tool_end", { tool_id: "e1" }),
      event("tool_start", { tool_args: { path: "a.md" }, tool_name: "edit", tool_id: "e2" }),
      event("tool_end", { tool_id: "e2" }),
    ]);
    const [shellGroup, editGroup] = projection.activityItems;
    expect(projection.activityItems.map((item) => [item.kind, item.count])).toEqual([["shell", 2], ["edit", 1]]);
    expect(shellGroup?.id).toMatch(/^shell_\d+_group$/);
    expect(editGroup?.id).toMatch(/^edit_\d+_group$/);
    expect(shellGroup?.children).toHaveLength(2);
    expect(editGroup?.children).toHaveLength(1);
  });

  it("hops whitespace-only text between tool calls but stops at real prose", () => {
    const hopped = buildAssistantRunProjection([
      event("tool_start", { tool_args: { path: "a.md" }, tool_id: "t1", tool_name: "read" }),
      event("tool_end", { tool_id: "t1" }),
      event("text_chunk", { text: "   " }),
      event("tool_start", { tool_args: { path: "b.md" }, tool_id: "t2", tool_name: "read" }),
      event("tool_end", { tool_id: "t2" }),
    ]);
    expect(shape(hopped)).toEqual(["activity(read,completedx2)"]);

    const broken = buildAssistantRunProjection([
      event("tool_start", { tool_args: { path: "a.md" }, tool_id: "t1", tool_name: "read" }),
      event("tool_end", { tool_id: "t1" }),
      event("text_chunk", { text: "wait, let me look again" }),
      event("tool_start", { tool_args: { path: "b.md" }, tool_id: "t2", tool_name: "read" }),
      event("tool_end", { tool_id: "t2" }),
    ]);
    expect(shape(broken)).toEqual(["activity(read,completed)", "text(wait, let me look again)", "activity(read,completed)"]);
  });

  it("keeps a single completed tool as its own item", () => {
    const projection = buildAssistantRunProjection([
      event("tool_start", { tool_args: { path: "a.md" }, tool_name: "read", tool_id: "t1" }),
      event("tool_end", { tool_id: "t1" }),
    ]);
    expect(projection.activityItems).toEqual([
      { detail: "a.md", id: "t1", kind: "read", status: "completed", target: "a.md" },
    ]);
  });
});

describe("projector lifecycle", () => {
  it("skips events that are not newer than the watermark", () => {
    const projector = createRunProjector();
    projector.append(event("text_chunk", { text: "a" }, { sequence: 1 }));
    projector.append(event("text_chunk", { text: "b" }, { sequence: 0 }));
    expect(projector.snapshot().content).toBe("a");
    expect(projector.lastSequence).toBe(1);

    projector.append(event("text_chunk", { text: "c" }, { sequence: 2 }));
    expect(projector.snapshot().content).toBe("ac");
    expect(projector.lastSequence).toBe(2);
  });

  it("ingests an unsorted batch and processes two events that share a sequence", () => {
    const first = event("text_chunk", { text: "a" }, { sequence: 1 });
    const second = event("text_chunk", { text: "b" }, { sequence: 1 });
    const third = event("text_chunk", { text: "c" }, { sequence: 2 });
    const projection = createRunProjector().ingest([third, second, first]);
    // The sort is stable, so the two sequence-1 events keep their input order;
    // both are processed because the pre-batch watermark was -1.
    expect(projection.content).toBe("bac");
  });

  it("computes a byte estimate for every kind of retained slot", () => {
    const projector = createRunProjector();
    projector.append(event("text_chunk", { text: "some answer text" }, { sequence: 1 }));
    projector.append(event("thinking_start", { blockId: "b1" }, { sequence: 2 }));
    projector.append(event("thinking_delta", { blockId: "b1", text: "re" }, { sequence: 3 }));
    projector.append(event("tool_start", { tool_args: { command: "ls -la" }, tool_id: "t1", tool_name: "shell" }, { sequence: 4 }));
    projector.append(event("compaction_end", { checkpoint_id: "cp1" }, { sequence: 5 }));
    const bytes = projector.estimatedBytes();
    expect(Number.isFinite(bytes)).toBe(true);
    expect(bytes).toBeGreaterThan(1024);
  });

  it("ingesting an empty batch leaves the watermark alone", () => {
    const projector = createRunProjector();
    expect(projector.ingest([]).content).toBe("");
    expect(projector.lastSequence).toBe(-1);
  });

  it("skips the overlapping events of a re-fetched batch", () => {
    const projector = createRunProjector();
    projector.ingest([
      event("text_chunk", { text: "a" }, { sequence: 1 }),
      event("text_chunk", { text: "b" }, { sequence: 2 }),
    ]);
    const projection = projector.ingest([
      event("text_chunk", { text: "X" }, { sequence: 2 }),
      event("text_chunk", { text: "c" }, { sequence: 3 }),
    ]);
    expect(projection.content).toBe("abc");
  });

  it("ignores an event whose payload is not JSON", () => {
    const projection = buildAssistantRunProjection([
      event("text_chunk", { text: "kept" }),
      event("text_chunk", undefined, { payload: "{not json" }),
    ]);
    expect(projection.content).toBe("kept");
  });

  it("forks a projector that already tracks tools", () => {
    const projector = createRunProjector();
    projector.append(event("tool_start", { tool_args: { path: "a.md" }, tool_id: "t1", tool_name: "read" }, { sequence: 1 }));
    projector.append(event("text_chunk", { text: "see " }, { sequence: 2 }));
    const fork = projector.fork();
    expect(fork.snapshot().segments.map((segment) => segment.kind)).toEqual(["activity", "text"]);

    // The fork owns its own tool map and argument buffer.
    fork.append(event("tool_delta", { text: '{"path":"b.md"}' }, { sequence: 3 }));
    expect(fork.snapshot().segments[0]).toMatchObject({ item: { target: "b.md" } });
    expect(projector.snapshot().segments[0]).toMatchObject({ item: { target: "a.md" } });
  });

  it("forks an independent accumulator", () => {
    const projector = createRunProjector();
    projector.append(event("text_chunk", { text: "base" }, { sequence: 1 }));
    projector.append(event("thinking_start", { blockId: "b1" }, { sequence: 2 }));
    const fork = projector.fork();

    fork.append(event("text_chunk", { text: "-forked" }, { sequence: 3 }));
    expect(fork.snapshot().content).toBe("base-forked");
    expect(projector.snapshot().content).toBe("base");
    // The fork carries the parent's slots (base text, open thinking) and opens
    // its own text slot for the appended chunk.
    expect(fork.snapshot().segments.map((segment) => segment.kind)).toEqual(["text", "thinking", "text"]);
    expect(fork.lastSequence).toBe(3);
  });

  it("estimates a conservative byte size that grows with retained content", () => {
    const empty = createRunProjector();
    const before = empty.estimatedBytes();
    empty.append(event("text_chunk", { text: "x".repeat(500) }));
    expect(empty.estimatedBytes()).toBeGreaterThan(before);

    const withTool = createRunProjector();
    withTool.append(event("tool_start", { tool_args: { path: "a.md" }, tool_name: "read", tool_id: "some-long-tool-id" }));
    expect(withTool.estimatedBytes()).toBeGreaterThan(256);
  });

  it("estimates the size of a retained compaction divider", () => {
    // A compaction slot carries numeric fields, so the size walk must skip them
    // instead of adding `NaN`/throwing on `value.length`.
    const projector = createRunProjector();
    projector.append(event("compaction_end", { checkpoint_id: "cp1", tokens_before: 100 }, { sequence: 1 }));
    expect(projector.estimatedBytes()).toBeGreaterThan(256);
    expect(Number.isFinite(projector.estimatedBytes())).toBe(true);
  });

  it("forks a projector that carries an active reconnect state", () => {
    const projector = createRunProjector();
    projector.append(event("stream_retry", { attempt: 1, delayMs: 10, maxRetries: 2 }, { sequence: 1 }));
    const fork = projector.fork();
    expect(fork.snapshot().reconnecting).toEqual({ attempt: 1, delayMs: 10, maxRetries: 2 });
    // The two accumulators own separate state objects.
    expect(fork.snapshot().reconnecting).not.toBe(projector.snapshot().reconnecting);

    fork.append(event("stream_resumed", undefined, { sequence: 2 }));
    expect(fork.snapshot().reconnecting).toBeUndefined();
    expect(projector.snapshot().reconnecting).toEqual({ attempt: 1, delayMs: 10, maxRetries: 2 });
  });

  it("falls back to the default option when no options are given", () => {
    expect(createRunProjector().snapshot().outputTokens).toBe(0);
  });
});

describe("assistant id synthesis", () => {
  it("returns an empty snapshot for a projector that saw no events", () => {
    const projection = createRunProjector().snapshot();
    expect(projection).toMatchObject({ content: "", outputTokens: 0, stopped: false, thinkingActive: false, truncated: false });
    expect(projection.segments).toEqual([]);
    expect(projection.activityItems).toEqual([]);
    expect(projection.thinking).toBe("");
    expect(projection.reconnecting).toBeUndefined();
  });
});

describe("nonZeroExitCode", () => {
  it.each<[string | undefined, number | null]>([
    [undefined, null],
    ["", null],
    ["no footer", null],
    ["done\n[exit: 0]", null],
    ["done\n[exit: 1]", 1],
    ["done\n[exit: -9]", -9],
    ["[exit: 7]   ", 7],
    ["[exit: 1] trailing", null],
  ])("reads %j as %j", (output, expected) => {
    expect(nonZeroExitCode(output)).toBe(expected);
  });
});

describe("isSoftExit", () => {
  it.each<[number, string | undefined, boolean]>([
    [0, "grep x", false],
    [2, "grep x", false],
    [1, undefined, false],
    [1, "", false],
    [1, " \t ", false],
    [1, "grep x", true],
    [1, "  grep -q x", true],
    [1, "C:\\tools\\rg.EXE pattern", true],
    [1, "/usr/bin/diff a b", true],
    [1, "findstr x", true],
    [1, "test -f a", true],
    [1, "[ -f a ]", true],
    [1, "   ", false],
    [1, "grep x | head", false],
    [1, "grep x && echo y", false],
    [1, "grep x; echo y", false],
    [1, "grep $(echo x)", false],
    [1, "grep x > out", false],
    [1, "npm test", false],
  ])("isSoftExit(%i, %j) === %s", (exitCode, command, expected) => {
    expect(isSoftExit(exitCode, command)).toBe(expected);
  });
});
