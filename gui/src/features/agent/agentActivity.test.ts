import type { StoredRunEvent } from "../../integrations/storage/threadStore";
import { describe, expect, it } from "vitest";
import { buildAssistantRunProjection } from "./agentActivity";

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
    const [segment] = projection.segments;
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
    const [segment] = projection.segments;
    expect(segment.kind === "activity" && segment.item.count).toBe(2);
  });

  it("still produces activity segments for a tool-only turn (no text)", () => {
    const projection = buildAssistantRunProjection(
      events([
        ["tool_start", read("t1", "/a.ts")[0]],
        ["tool_end", read("t1", "/a.ts")[0]],
      ]),
    );

    expect(projection.content.trim()).toBe("");
    expect(projection.segments).toHaveLength(1);
    expect(projection.segments[0].kind).toBe("activity");
  });
});
