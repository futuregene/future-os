import type { RunEvent } from "@future-os/thread-projection";
import { createRunProjector } from "@future-os/thread-projection";
import { describe, expect, it } from "vitest";

function event(sequence: number, eventType: string, payload: unknown = {}): RunEvent {
  return { id: `e${sequence}`, runId: "r", sequence, eventType, payload: JSON.stringify(payload), createdAt: 0 };
}

describe("deferred projector snapshots", () => {
  it("append has the same arrival-order semantics as ingest of single events", () => {
    const events = [
      event(0, "agent_start"),
      event(5, "text_chunk", { text: "new" }),
      event(2, "text_chunk", { text: "old" }),
      event(5, "text_chunk", { text: "duplicate" }),
      event(6, "usage", { usage: { completion_tokens: 10 } }),
      event(7, "agent_end", { state: "cancelled", usage: { completion_tokens: 20 } }),
    ];
    const single = createRunProjector({ preferEndTokens: true });
    const deferred = createRunProjector({ preferEndTokens: true });
    for (const item of events) {
      single.ingest([item]);
      deferred.append(item);
    }
    expect(deferred.lastSequence).toBe(single.lastSequence);
    expect(deferred.snapshot()).toEqual(single.snapshot());
  });

  it("fork isolates open text and thinking slots", () => {
    for (const type of ["text_chunk", "thinking_delta"]) {
      const original = createRunProjector();
      original.append(event(1, type, { text: "prefix" }));
      const fork = original.fork();
      fork.append(event(2, type, { text: " fork" }));
      expect(original.lastSequence).toBe(1);
      original.append(event(2, type, { text: " original" }));
      const key = type === "text_chunk" ? "content" : "thinking";
      expect(original.snapshot()[key]).toBe("prefix original");
      expect(fork.snapshot()[key]).toBe("prefix fork");
    }
  });

  it("fork isolates tools, compaction, usage, retry and terminal flags", () => {
    const original = createRunProjector({ preferEndTokens: true });
    original.ingest([
      event(0, "tool_start", { tool_name: "shell", tool_call_id: "t", tool_args: { command: "echo hi" } }),
      event(1, "compaction_started", { operation_id: "op" }),
      event(2, "usage", { usage: { completion_tokens: 10 } }),
      event(3, "stream_retry", { attempt: 1, maxRetries: 3, delayMs: 100 }),
    ]);
    const before = original.snapshot();
    const fork = original.fork();
    fork.ingest([
      event(4, "tool_end", { tool_name: "shell", tool_call_id: "t", text: "done" }),
      event(5, "compaction_committed", { operation_id: "op", checkpoint_id: "cp", tokens_before: 100 }),
      event(6, "agent_end", { state: "cancelled", usage: { completion_tokens: 50 } }),
    ]);
    expect(original.snapshot()).toEqual(before);
    expect(original.lastSequence).toBe(3);
    expect(fork.snapshot()).toMatchObject({ outputTokens: 50, stopped: true });
    expect(fork.snapshot().reconnecting).toBeUndefined();
  });
});
