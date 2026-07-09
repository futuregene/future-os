import type { MessageSegment } from "./agentThreadTypes";
import { describe, expect, it } from "vitest";
import { sliceSegments, textBudget } from "./usePacedReveal";

function text(id: string, t: string): MessageSegment {
  return { kind: "text", id, text: t };
}

function tool(id: string): MessageSegment {
  return { kind: "activity", id, item: { id, kind: "bash", status: "completed" } };
}

describe("textBudget", () => {
  it("sums only text-segment lengths", () => {
    expect(textBudget([text("a", "hello"), tool("t"), text("b", "world")])).toBe(10);
  });
});

describe("sliceSegments", () => {
  it("truncates the live text segment to the revealed budget", () => {
    const result = sliceSegments([text("a", "hello world")], 5);
    expect(result).toEqual([text("a", "hello")]);
  });

  it("reveals earlier full text before spending budget on later text", () => {
    const segments = [text("a", "abc"), text("b", "def")];
    expect(sliceSegments(segments, 4)).toEqual([text("a", "abc"), text("b", "d")]);
  });

  it("gates a trailing non-text segment until preceding text is fully revealed", () => {
    const segments = [text("a", "abc"), tool("t")];
    // Text only partly revealed: the tool row must not appear yet.
    expect(sliceSegments(segments, 2)).toEqual([text("a", "ab")]);
    // Text fully revealed: the tool row appears.
    expect(sliceSegments(segments, 3)).toEqual([text("a", "abc"), tool("t")]);
  });

  it("passes a leading non-text segment through immediately (zero-width gate)", () => {
    const segments = [tool("t"), text("a", "abc")];
    expect(sliceSegments(segments, 1)).toEqual([tool("t"), text("a", "a")]);
  });

  it("returns the whole timeline once the budget covers all text", () => {
    const segments = [text("a", "abc"), tool("t"), text("b", "de")];
    expect(sliceSegments(segments, 5)).toEqual(segments);
  });
});
