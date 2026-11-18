import { describe, expect, it } from "vitest";
import {
  CHOICE_CONTEXT_VISIBLE_LINES,
  choiceContextFolds,
  estimateContextLines,
} from "./choiceContext";

describe("estimateContextLines", () => {
  it("counts a CJK character as two Latin widths", () => {
    // 60 CJK characters are 120 units: one line of a 120-unit column, two of 60.
    const text = "森".repeat(60);
    expect(estimateContextLines(text, 120)).toBe(1);
    expect(estimateContextLines(text, 60)).toBe(2);
    expect(estimateContextLines("a".repeat(120), 120)).toBe(1);
  });

  it("breaks on explicit newlines and counts an empty one as a line", () => {
    expect(estimateContextLines("甲\n乙", 120)).toBe(2);
    expect(estimateContextLines("甲\n\n乙", 120)).toBe(3);
  });

  it("rounds a partly filled line up", () => {
    expect(estimateContextLines("a".repeat(121), 120)).toBe(2);
  });

  it("never divides by a column of zero units", () => {
    // Degenerate input must not produce Infinity or NaN: the caller's column is
    // a constant, but a future renderer could compute it and get 0.
    expect(estimateContextLines("甲", 0)).toBeGreaterThan(0);
    expect(Number.isFinite(estimateContextLines("甲", 0))).toBe(true);
  });
});

describe("choiceContextFolds", () => {
  // A desktop card (120 units a line) and a phone card (48) read the same text
  // very differently — which is why the caller passes its own column.
  const fourLinesOnDesktop = "森".repeat(CHOICE_CONTEXT_VISIBLE_LINES * 60);

  it("leaves a context that fits in the visible lines alone", () => {
    expect(choiceContextFolds(fourLinesOnDesktop, 120)).toBe(false);
  });

  it("does not fold for a single hidden line", () => {
    // Five desktop lines: folding would trade a button for one line.
    expect(choiceContextFolds("森".repeat(5 * 60), 120)).toBe(false);
  });

  it("folds once two lines would be hidden", () => {
    expect(choiceContextFolds("森".repeat(6 * 60), 120)).toBe(true);
  });

  it("folds the same text on a phone, where it is already tall", () => {
    expect(estimateContextLines(fourLinesOnDesktop, 120)).toBe(CHOICE_CONTEXT_VISIBLE_LINES);
    expect(estimateContextLines(fourLinesOnDesktop, 48)).toBeGreaterThan(CHOICE_CONTEXT_VISIBLE_LINES);
    expect(choiceContextFolds(fourLinesOnDesktop, 48)).toBe(true);
  });
});
