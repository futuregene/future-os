import type { StoredRunEvent } from "../../integrations/storage/threadStore";
import { describe, expect, it } from "vitest";
import { buildAssistantRunProjection, createRunProjector, isSoftExit, nonZeroExitCode } from "./agentActivity";

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

function read(id: string, path: string) {
  return [{ tool_name: "read", tool_id: id, tool_args: { path } }] as const;
}
function edit(id: string, path: string) {
  return [{ tool_name: "edit", tool_id: id, tool_args: { file_path: path } }] as const;
}

describe("buildAssistantRunProjection segments", () => {
  it("interleaves text and tool activity in chronological order", () => {
    const projection = buildAssistantRunProjection(
      events([
        ["text_chunk", { text: "First. " }],
        ["tool_start", read("t1", "/a.ts")[0]],
        ["tool_end", read("t1", "/a.ts")[0]],
        ["text_chunk", { text: "Second." }],
      ]),
    );

    expect(projection.segments.map(s => s.kind)).toEqual(["text", "activity", "text"]);
    expect(projection.segments[0]).toMatchObject({ kind: "text", text: "First. " });
    expect(projection.segments[1]).toMatchObject({ kind: "activity" });
    expect(projection.segments[2]).toMatchObject({ kind: "text", text: "Second." });
    expect(projection.content).toBe("First. Second.");
  });

  it("surfaces a compaction marker inline, carrying the pre-compaction token count", () => {
    const projection = buildAssistantRunProjection(
      events([
        ["compaction_start", { reason: "auto" }],
        ["compaction_end", { tokens_before: 190000, summary: "…", aborted: false, reason: "auto" }],
        ["text_chunk", { text: "Continuing." }],
      ]),
    );

    expect(projection.segments.map(s => s.kind)).toEqual(["compaction", "text"]);
    expect(projection.segments[0]).toMatchObject({ kind: "compaction", tokensBefore: 190000 });
    // The marker must not leak into the copyable/rendered answer text.
    expect(projection.content).toBe("Continuing.");
  });

  it("omits the token count for the retry-path compaction (tokens_before 0) and skips aborted ones", () => {
    const retryPath = buildAssistantRunProjection(
      events([["compaction_end", { tokens_before: 0, summary: "", aborted: false, reason: "auto" }]]),
    );
    expect(retryPath.segments).toHaveLength(1);
    expect(retryPath.segments[0]).toMatchObject({ kind: "compaction" });
    expect(retryPath.segments[0]).not.toHaveProperty("tokensBefore", 0);

    const aborted = buildAssistantRunProjection(
      events([["compaction_end", { tokens_before: 5, aborted: true, reason: "auto" }]]),
    );
    expect(aborted.segments).toHaveLength(0);
  });

  it("collapses a run of adjacent same-kind tools into one grouped line", () => {
    const projection = buildAssistantRunProjection(
      events([
        ["tool_start", edit("t1", "/a.ts")[0]],
        ["tool_end", edit("t1", "/a.ts")[0]],
        ["tool_start", edit("t2", "/b.ts")[0]],
        ["tool_end", edit("t2", "/b.ts")[0]],
      ]),
    );

    expect(projection.segments).toHaveLength(1);
    const segment = projection.segments[0]!;
    expect(segment.kind).toBe("activity");
    if (segment.kind === "activity") {
      expect(segment.item.kind).toBe("edit");
      expect(segment.item.count).toBe(2);
    }
  });

  it("keeps tools separate when real prose sits between them", () => {
    const projection = buildAssistantRunProjection(
      events([
        ["tool_start", edit("t1", "/a.ts")[0]],
        ["tool_end", edit("t1", "/a.ts")[0]],
        ["text_chunk", { text: "then I checked the result" }],
        ["tool_start", edit("t2", "/b.ts")[0]],
        ["tool_end", edit("t2", "/b.ts")[0]],
      ]),
    );

    expect(projection.segments.map(s => s.kind)).toEqual(["activity", "text", "activity"]);
    // Two separate edits, not the collapsed "edited 2 files" line.
    for (const segment of projection.segments) {
      if (segment.kind === "activity") {
        expect(segment.item.count).toBeUndefined();
      }
    }
  });

  it("treats whitespace-only text between tools as non-breaking", () => {
    const projection = buildAssistantRunProjection(
      events([
        ["tool_start", edit("t1", "/a.ts")[0]],
        ["tool_end", edit("t1", "/a.ts")[0]],
        ["text_chunk", { text: "\n\n" }],
        ["tool_start", edit("t2", "/b.ts")[0]],
        ["tool_end", edit("t2", "/b.ts")[0]],
      ]),
    );

    expect(projection.segments).toHaveLength(1);
    const segment = projection.segments[0]!;
    expect(segment.kind === "activity" && segment.item.count).toBe(2);
  });

  it("still produces activity segments for a tool-only exchange (no text)", () => {
    const projection = buildAssistantRunProjection(
      events([
        ["tool_start", read("t1", "/a.ts")[0]],
        ["tool_end", read("t1", "/a.ts")[0]],
      ]),
    );

    expect(projection.content.trim()).toBe("");
    expect(projection.segments).toHaveLength(1);
    expect(projection.segments[0]!.kind).toBe("activity");
  });
});

describe("buildAssistantRunProjection thinking", () => {
  it("accumulates thinking_delta text between start/end", () => {
    const projection = buildAssistantRunProjection(
      events([
        ["thinking_start", {}],
        ["thinking_delta", { text: "Let me " }],
        ["thinking_delta", { text: "reason." }],
        ["thinking_end", {}],
        ["text_chunk", { text: "Answer." }],
      ]),
    );

    expect(projection.thinking).toBe("Let me reason.");
    expect(projection.content).toBe("Answer.");
  });

  it("separates distinct thinking blocks with a blank line", () => {
    const projection = buildAssistantRunProjection(
      events([
        ["thinking_start", {}],
        ["thinking_delta", { text: "First." }],
        ["thinking_end", {}],
        ["thinking_start", {}],
        ["thinking_delta", { text: "Second." }],
        ["thinking_end", {}],
      ]),
    );

    expect(projection.thinking).toBe("First.\n\nSecond.");
  });

  it("is empty when there is no thinking", () => {
    const projection = buildAssistantRunProjection(events([["text_chunk", { text: "Hi." }]]));
    expect(projection.thinking).toBe("");
  });

  it("places thinking inline in the timeline, not hoisted to the top", () => {
    const projection = buildAssistantRunProjection(
      events([
        ["text_chunk", { text: "Let me check. " }],
        ["thinking_start", {}],
        ["thinking_delta", { text: "The file is under attachments." }],
        ["thinking_end", {}],
        ["tool_start", read("t1", "/a.pdf")[0]],
        ["tool_end", read("t1", "/a.pdf")[0]],
        ["text_chunk", { text: "Done." }],
      ]),
    );

    expect(projection.segments.map(s => s.kind)).toEqual(["text", "thinking", "activity", "text"]);
    const thinkingSegment = projection.segments[1]!;
    expect(thinkingSegment).toMatchObject({ kind: "thinking", text: "The file is under attachments." });
  });

  it("flags thinkingActive while reasoning with nothing else visible, and injects no top activity line", () => {
    const projection = buildAssistantRunProjection(
      events([
        ["thinking_start", {}],
        ["thinking_delta", { text: "Working on it." }],
      ]),
    );

    expect(projection.thinkingActive).toBe(true);
    // The old top-of-message "thinking" activity line is gone.
    expect(projection.segments.some(s => s.kind === "activity")).toBe(false);
  });

  it("clears thinkingActive once answer text appears", () => {
    const projection = buildAssistantRunProjection(
      events([
        ["thinking_start", {}],
        ["thinking_delta", { text: "Hmm." }],
        ["thinking_end", {}],
        ["text_chunk", { text: "Answer." }],
      ]),
    );

    expect(projection.thinkingActive).toBe(false);
  });

  it("clears thinkingActive once tool work is visible", () => {
    const projection = buildAssistantRunProjection(
      events([
        ["thinking_start", {}],
        ["thinking_delta", { text: "Let me look." }],
        ["tool_start", read("t1", "/a.pdf")[0]],
      ]),
    );

    expect(projection.thinkingActive).toBe(false);
  });
});

describe("buildAssistantRunProjection output tokens", () => {
  // Real shape emitted by the agent's gRPC StreamEvent: usage nested under `usage`
  // with `completion_tokens` (mirrors how the TUI reads it).
  it("sums completion tokens across every per-call usage event", () => {
    const projection = buildAssistantRunProjection(
      events([
        ["text_chunk", { text: "thinking" }],
        ["usage", { type: "usage", usage: { prompt_tokens: 1200, completion_tokens: 40, total_tokens: 1240 } }],
        ["tool_start", read("t1", "/a.ts")[0]],
        ["tool_end", read("t1", "/a.ts")[0]],
        ["usage", { type: "usage", usage: { prompt_tokens: 1800, completion_tokens: 110, total_tokens: 1910 } }],
        ["agent_end", { type: "agent_end" }],
      ]),
    );

    // 40 + 110 generated across the two exchanges.
    expect(projection.outputTokens).toBe(150);
  });

  it("falls back to agent_end usage when no per-call usage was streamed", () => {
    const projection = buildAssistantRunProjection(
      events([
        ["text_chunk", { text: "done" }],
        ["agent_end", { type: "agent_end", usage: { completion_tokens: 64 } }],
      ]),
    );

    expect(projection.outputTokens).toBe(64);
  });

  it("tolerates a flat output_tokens shape", () => {
    const projection = buildAssistantRunProjection(
      events([["usage", { output_tokens: 27 }]]),
    );

    expect(projection.outputTokens).toBe(27);
  });

  it("reports zero when the provider returned no usage", () => {
    const projection = buildAssistantRunProjection(
      events([["text_chunk", { text: "hi" }]]),
    );

    expect(projection.outputTokens).toBe(0);
  });
});

// The agent appends the exit code as a "[exit: N]" footer on the LAST line, not
// a "[exit code: N]" prefix. Parsing the wrong shape silently dropped every
// failure to exit 0 → a failed shell command rendered as "completed".
describe("nonZeroExitCode", () => {
  it("reads the [exit: N] footer even with leading output and blank lines", () => {
    // The exact real-world capture: command-not-found on bash (macOS/Linux).
    expect(nonZeroExitCode("bash: future: command not found\n\n[exit: 127]")).toBe(127);
  });

  it("detects the Windows form of the same failure (PowerShell exit 1)", () => {
    // On Windows the wrapper reports command-not-found via $Error as exit 1, not
    // 127 — the footer parser keys on any non-zero, so both platforms are caught.
    expect(nonZeroExitCode(
      "future : The term 'future' is not recognized as the name of a cmdlet.\n[exit: 1]",
    )).toBe(1);
  });

  it("returns null for exit 0 and for output with no footer", () => {
    expect(nonZeroExitCode("all good\n[exit: 0]")).toBeNull();
    expect(nonZeroExitCode("no footer here")).toBeNull();
    expect(nonZeroExitCode(undefined)).toBeNull();
    // A "[exit code: N]" prefix is the OLD format and must NOT be mistaken for one.
    expect(nonZeroExitCode("[exit code: 127]\nfutre: command not found")).toBeNull();
  });

  it("keeps the soft-fail exemption keyed to bare grep/findstr exit 1", () => {
    // findstr is the Windows no-match case (native, exit 1) — exempt.
    expect(isSoftExit(1, "findstr foo bar.txt")).toBe(true);
    expect(isSoftExit(1, "grep foo file")).toBe(true);
    // A real command-not-found (exit 1, first token not a soft-fail program) stays a failure.
    expect(isSoftExit(1, "future tools call parse_doc")).toBe(false);
  });
});

describe("createRunProjector incremental ingestion", () => {
  const full = events([
    ["thinking_start", {}],
    ["thinking_delta", { text: "reasoning " }],
    ["thinking_delta", { text: "more" }],
    ["thinking_end", {}],
    ["text_chunk", { text: "First. " }],
    ["tool_start", read("t1", "/a.ts")[0]],
    ["toolcall_delta", { text: "{\"path\":\"/a" }],
    ["toolcall_delta", { text: ".ts\"}" }],
    ["tool_end", read("t1", "/a.ts")[0]],
    ["text_chunk", { text: "Second." }],
    ["usage", { usage: { completion_tokens: 42 } }],
    ["agent_end", { usage: { completion_tokens: 99 } }],
  ]);

  it("matches the one-shot projection when fed in chunks", () => {
    const expected = buildAssistantRunProjection(full);
    const projector = createRunProjector();

    // Feed in uneven chunks, as the 220ms incremental poll would deliver them.
    projector.ingest(full.slice(0, 3));
    projector.ingest(full.slice(3, 6));
    const projection = projector.ingest(full.slice(6));

    expect(projection).toEqual(expected);
    expect(projector.lastSequence).toBe(full.length - 1);
  });

  it("skips already-ingested events in overlapping batches", () => {
    const expected = buildAssistantRunProjection(full);
    const projector = createRunProjector();

    projector.ingest(full.slice(0, 6));
    // Overlapping redelivery (events 4-7) must not double-apply text/tools.
    const projection = projector.ingest(full.slice(4));

    expect(projection).toEqual(expected);
  });

  it("processes two events sharing one sequence within a single batch", () => {
    const projector = createRunProjector();
    const batch = events([
      ["text_chunk", { text: "a" }],
      ["text_chunk", { text: "b" }],
    ]).map(event => ({ ...event, sequence: 7 }));

    const projection = projector.ingest(batch);

    expect(projection.content).toBe("ab");
    expect(projector.lastSequence).toBe(7);
    // ...but a later batch re-delivering that watermark is deduped.
    expect(projector.ingest(batch).content).toBe("ab");
  });

  it("returns a stable empty snapshot before any event lands", () => {
    const projector = createRunProjector();
    const projection = projector.ingest([]);

    expect(projection.content).toBe("");
    expect(projection.segments).toHaveLength(0);
    expect(projector.lastSequence).toBe(-1);
  });
});

describe("buildAssistantRunProjection edge branches", () => {
  it("opens a thinking block lazily for a delta without a start", () => {
    const projection = buildAssistantRunProjection(
      events([["thinking_delta", { text: "musing" }]]),
    );
    expect(projection.thinking).toBe("musing");
    expect(projection.segments[0]).toMatchObject({ kind: "thinking" });
  });

  it("ignores a tool delta with no active tool call", () => {
    const projection = buildAssistantRunProjection(
      events([
        ["tool_delta", { text: "{\"path\":" }],
        ["text_chunk", { text: "done" }],
      ]),
    );
    expect(projection.content).toBe("done");
  });

  it("ignores a tool end whose payload names no known tool", () => {
    const projection = buildAssistantRunProjection(
      events([
        ["tool_end", { tool_name: "mystery" }],
        ["text_chunk", { text: "done" }],
      ]),
    );
    expect(projection.activityItems).toHaveLength(0);
  });

  it("slots a tool result that arrives without a start", () => {
    const projection = buildAssistantRunProjection(
      events([["tool_end", { tool_name: "read", tool_id: "orphan", tool_args: { path: "/a.ts" } }]]),
    );
    expect(projection.activityItems[0]).toMatchObject({ id: "orphan", status: "completed" });
  });

  it("matches an id-less tool end to the latest running tool of its kind", () => {
    const projection = buildAssistantRunProjection(
      events([
        ["tool_start", { tool_name: "read", tool_id: "r1", tool_args: { path: "/a.ts" } }],
        ["tool_start", { tool_name: "read", tool_id: "r2", tool_args: { path: "/b.ts" } }],
        ["tool_end", { tool_name: "read" }],
      ]),
    );
    // The later (higher-order) running tool settles first.
    const byId = Object.fromEntries(projection.activityItems.map(i => [i.id, i.status]));
    expect(byId).toEqual({ r1: "running", r2: "completed" });
  });

  it("surfaces the streaming args target from partial JSON", () => {
    const projection = buildAssistantRunProjection(
      events([
        ["tool_start", { tool_name: "edit", tool_id: "e1", tool_args: {} }],
        ["toolcall_delta", { text: "{\"file_path\": \"/partial.ts\"" }],
        ["tool_end", { tool_name: "edit", tool_id: "e1" }],
      ]),
    );
    expect(projection.activityItems[0]).toMatchObject({ target: "/partial.ts" });
  });

  it("tolerates an invalid JSON string field in partial args", () => {
    const projection = buildAssistantRunProjection(
      events([
        ["tool_start", { tool_name: "edit", tool_id: "e1", tool_args: {} }],
        ["toolcall_delta", { text: "{\"path\": \"/bad\\q.ts\"" }],
        ["tool_end", { tool_name: "edit", tool_id: "e1" }],
      ]),
    );
    expect(projection.activityItems[0]?.status).toBe("completed");
  });

  it("breaks a collapsible tool run at a compaction marker", () => {
    const projection = buildAssistantRunProjection(
      events([
        ["tool_start", edit("t1", "/a.ts")[0]],
        ["tool_end", edit("t1", "/a.ts")[0]],
        ["tool_start", edit("t2", "/b.ts")[0]],
        ["tool_end", edit("t2", "/b.ts")[0]],
        ["compaction_end", { tokens_before: 100, aborted: false }],
        ["tool_start", edit("t3", "/c.ts")[0]],
        ["tool_end", edit("t3", "/c.ts")[0]],
      ]),
    );
    // The first pair collapses; the post-compaction tool stays separate.
    const kinds = projection.segments.map(s => s.kind);
    expect(kinds).toEqual(["activity", "compaction", "activity"]);
  });

  it("handles non-record and malformed event payloads", () => {
    const raw: StoredRunEvent[] = [
      { id: "e0", runId: "r1", eventType: "usage", payload: "5", sequence: 0, createdAt: 0 },
      { id: "e1", runId: "r1", eventType: "compaction_end", payload: "\"x\"", sequence: 1, createdAt: 1 },
      { id: "e2", runId: "r1", eventType: "tool_start", payload: "5", sequence: 2, createdAt: 2 },
      { id: "e3", runId: "r1", eventType: "text_chunk", payload: "5", sequence: 3, createdAt: 3 },
      { id: "e4", runId: "r1", eventType: "text_chunk", payload: null, sequence: 4, createdAt: 4 },
      { id: "e5", runId: "r1", eventType: "text_chunk", payload: "not json", sequence: 5, createdAt: 5 },
      { id: "e6", runId: "r1", eventType: "text_chunk", payload: "{\"text\": \"ok\"}", sequence: 6, createdAt: 6 },
    ];
    const projection = buildAssistantRunProjection(raw);
    expect(projection.content).toBe("ok");
  });

  it("marks a tool failed by its error field", () => {
    const projection = buildAssistantRunProjection(
      events([
        ["tool_start", { tool_name: "read", tool_id: "r1", tool_args: { path: "/a" } }],
        ["tool_end", { tool_name: "read", tool_id: "r1", error: "permission denied" }],
      ]),
    );
    expect(projection.activityItems[0]?.status).toBe("failed");
  });

  it("marks a shell tool failed by a non-zero exit footer", () => {
    const projection = buildAssistantRunProjection(
      events([
        ["tool_start", { tool_name: "shell", tool_id: "s1", tool_args: { command: "make" } }],
        ["tool_end", { tool_name: "shell", tool_id: "s1", text: "boom\n[exit: 2]" }],
      ]),
    );
    expect(projection.activityItems[0]?.status).toBe("failed");
  });
});
