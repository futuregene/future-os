import { describe, expect, it } from "vitest";
import { applyContentRange, contentRangeStart } from "./contentRange";
import { createRunProjector } from "./liveApply";
import { newCursor, nextEvent } from "./runCursor";
import { tailCoversRange } from "./replay";

const data = (offset: number, text: string, first = 1) => ({
  content_key: "b", content_offset: offset,
  content_bytes: [...text].reduce((n, char) => n + (char.codePointAt(0)! <= 0x7F ? 1 : char.codePointAt(0)! <= 0x7FF ? 2 : char.codePointAt(0)! <= 0xFFFF ? 3 : 4), 0),
  content_first_idx: first, text,
});

describe("canonical content ranges", () => {
  it("does not accept an oversized body as canonical cursor overlap authority", () => {
    expect(contentRangeStart(JSON.stringify(data(0, "x")))).toBe(1);
    expect(contentRangeStart(JSON.stringify(data(0, "x".repeat(65536))))).toBeUndefined();
    expect(contentRangeStart('{"text":"legacy"}')).toBeUndefined();
  });
  it("trims an applied UTF-8 prefix and rejects a real gap", () => {
    const offsets = new Map<string, number>();
    applyContentRange(data(0, "你"), "text_chunk", offsets);
    expect(applyContentRange(data(0, "你好"), "text_chunk", offsets)).toMatchObject({ text: "好", content_offset: 3, content_bytes: 3 });
    expect(() => applyContentRange(data(7, "x"), "text_chunk", offsets)).toThrow("missing prefix");
  });
  it("keeps overlap state when an incremental projector is forked", () => {
    const projector = createRunProjector();
    projector.append({ id: "1", runId: "r", eventType: "text_chunk", payload: JSON.stringify(data(0, "你")), sequence: 1, createdAt: 0 });
    const fork = projector.fork();
    fork.append({ id: "3", runId: "r", eventType: "text_chunk", payload: JSON.stringify(data(0, "你好")), sequence: 3, createdAt: 0 });
    expect(fork.snapshot().content).toBe("你好");
    expect(projector.snapshot().content).toBe("你");
  });
  it("accepts covered cursor overlap and proves canonical replay coverage", () => {
    const cursor = newCursor();
    expect(nextEvent(cursor, "r", 0).kind).toBe("apply");
    expect(nextEvent(cursor, "r", 1, undefined, { data: JSON.stringify(data(0, "你")) }).kind).toBe("apply");
    const merged = JSON.stringify(data(0, "你好"));
    expect(nextEvent(cursor, "r", 3, undefined, { data: merged }).kind).toBe("apply");
    expect(tailCoversRange([{ idx: 3, data: merged }], 1, 3, undefined)).toBe(true);
    expect(tailCoversRange([{ idx: 3, data: JSON.stringify(data(0, "好", 3)) }], 1, 3, undefined)).toBe(false);
    expect(nextEvent(newCursor(), "cold", 3, undefined, { data: merged })).toEqual({ kind: "gap", fromIdx: -1 });
  });
});
