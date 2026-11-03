import {
  collapsedPromptHeight,
  isLongPrompt,
  PROMPT_COLLAPSE_LINES,
  PROMPT_PREVIEW_LINES,
  promptPreview,
} from "../messages/promptCollapse";

const lines = (count: number) => Array.from({ length: count }, () => "one line of prompt").join("\n");

describe("which prompts are previewed", () => {
  test("a prompt at the shared line budget is short", () => {
    expect(isLongPrompt(lines(PROMPT_COLLAPSE_LINES))).toBe(false);
    expect(isLongPrompt("a short question")).toBe(false);
  });

  test("one line past the budget is long", () => {
    expect(isLongPrompt(lines(PROMPT_COLLAPSE_LINES + 1))).toBe(true);
  });

  test("a single unbroken line is long once it would wrap past the budget", () => {
    // 10 000 characters in one line is hundreds of wrapped phone lines; the line
    // count on its own cannot see it, which is why the character clause exists.
    expect(isLongPrompt("x".repeat(10_000))).toBe(false);
    expect(isLongPrompt("x".repeat(10_001))).toBe(true);
  });
});

describe("the head the bubble draws", () => {
  test("a short prompt is handed over whole", () => {
    const text = "a short question";
    expect(promptPreview(text)).toEqual({ head: text, truncated: false });
  });

  test("a long prompt is cut to a head the row can afford", () => {
    const text = lines(40);
    const { head, truncated } = promptPreview(text);
    expect(truncated).toBe(true);
    expect(head.length).toBeLessThan(text.length);
    expect(text.startsWith(head)).toBe(true);
    // The budget is what bounds the row, not the line count: one 4 000-character
    // paragraph is one source line and would sail past it.
    expect(head.length).toBeLessThanOrEqual(200);
    expect(head.split("\n").length).toBeLessThanOrEqual(PROMPT_PREVIEW_LINES);
  });

  test("a preview of CJK prose is a few rendered lines, not the 27 a 720-character head wraps to", () => {
    // The shape the reporting phone failed on (2026-10-29): a 720-character
    // preview wraps to ~27 lines of CJK at a 350 dp column — ~630 dp, which the
    // device drew as an accent fill with no text. The head is now budgeted in
    // rendered lines, so what the bubble draws is what the bubble promises.
    const prompt = Array.from({ length: 40 }, (_, index) => `第 ${index} 段：这一步把结论与证据分开记录，避免把推测写成已经核实的事实。`).join("\n");
    const { head } = promptPreview(prompt);
    // ~26 CJK glyphs per 350 dp line at the default size.
    expect(Math.ceil(head.length / 26)).toBeLessThanOrEqual(PROMPT_PREVIEW_LINES);
  });

  test("a single giant line is cut by characters", () => {
    // The shape the reporting conversation actually has: one multi-kilobyte
    // paragraph, where a line-based cut would keep everything.
    const text = `${"Coverage residual lines, measured and recorded. ".repeat(300)}`;
    expect(text.split("\n")).toHaveLength(1);
    const { head, truncated } = promptPreview(text);
    expect(truncated).toBe(true);
    expect(head.length).toBeLessThanOrEqual(200);
    expect(text.startsWith(head)).toBe(true);
  });

  test("the cut stops at a block boundary, never mid-block", () => {
    // Blocks are separated by a blank line; stopping there keeps a preview out
    // of the middle of a table or a fence.
    const blocks = Array.from({ length: 30 }, (_, i) => `Sentence about the plan ${i}.`);
    const text = blocks.join("\n\n");
    const { head } = promptPreview(text);
    const rendered = head.split("\n\n");
    expect(rendered.length).toBeGreaterThan(1);
    // Every painted block is a whole block of the source, in order.
    rendered.forEach((part, index) => expect(part).toBe(blocks[index]));
  });

  test("the head never carries trailing whitespace", () => {
    const text = `${"line of prompt\n".repeat(40)}\n\n\n`;
    const { head, truncated } = promptPreview(text);
    expect(truncated).toBe(true);
    expect(head).toBe(head.trimEnd());
    expect(text.startsWith(head)).toBe(true);
  });
});

describe("how tall the preview bubble is", () => {
  test("the budget is the preview's own line count, plus the bubble's padding", () => {
    // 8 lines + 2 × 12dp padding: the cap has to include the padding or the
    // capped box would show one line less than it promises.
    expect(collapsedPromptHeight(22, 915)).toBe(22 * PROMPT_PREVIEW_LINES + 24);
  });

  test("a short window with large type caps at half of it", () => {
    // 8 × 30 = 240dp + padding would be most of a short phone's 480dp window; a
    // row that fills the viewport is the shape this module exists to prevent.
    expect(collapsedPromptHeight(30, 480)).toBe(240);
  });

  test("the cap grows with the reader's text size until the window bounds it", () => {
    expect(collapsedPromptHeight(25.3, 915)).toBeGreaterThan(collapsedPromptHeight(22, 915));
  });
});
