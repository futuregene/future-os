import {
  collapsedPromptHeight,
  isLongPrompt,
  PROMPT_COLLAPSE_LINES,
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
    // The budget is what bounds the row, not the line count: 16 *source* lines
    // of this prompt would be 16 lines, but one 4 000-character paragraph is one
    // source line and would sail past it.
    expect(head.length).toBeLessThanOrEqual(800);
    expect(head.split("\n").length).toBeLessThanOrEqual(PROMPT_COLLAPSE_LINES);
  });

  test("a single giant line is cut by characters", () => {
    // The shape the reporting conversation actually has: one multi-kilobyte
    // paragraph, where a line-based cut would keep everything.
    const text = `${"Coverage residual lines, measured and recorded. ".repeat(300)}`;
    expect(text.split("\n")).toHaveLength(1);
    const { head, truncated } = promptPreview(text);
    expect(truncated).toBe(true);
    expect(head.length).toBeLessThanOrEqual(800);
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
  test("the budget is the shared line count, plus the bubble's own padding", () => {
    // 16 lines + 2 × 12dp padding: the cap has to include the padding or the
    // capped box would show one line less than it promises.
    expect(collapsedPromptHeight(22, 915)).toBe(22 * PROMPT_COLLAPSE_LINES + 24);
  });

  test("a short window with large type caps at half of it", () => {
    // 16 × 30 = 480dp would be most of a short phone's 640dp window; a row that
    // fills the viewport is the shape this module exists to prevent.
    expect(collapsedPromptHeight(30, 640)).toBe(320);
  });

  test("the cap grows with the reader's text size until the window bounds it", () => {
    expect(collapsedPromptHeight(25.3, 915)).toBeGreaterThan(collapsedPromptHeight(22, 915));
  });
});
