import {
  collapsedPromptHeight,
  isLongPrompt,
  PROMPT_COLLAPSE_LINES,
} from "../messages/promptCollapse";

const lines = (count: number) => Array.from({ length: count }, () => "one line of prompt").join("\n");

describe("which prompts are collapsed", () => {
  test("a prompt at the shared line budget is short", () => {
    expect(isLongPrompt(lines(PROMPT_COLLAPSE_LINES))).toBe(false);
    expect(isLongPrompt("a short question")).toBe(false);
  });

  test("one line past the budget is long", () => {
    expect(isLongPrompt(lines(PROMPT_COLLAPSE_LINES + 1))).toBe(true);
  });

  test("a single unbroken line is long once it would wrap past the budget", () => {
    // 2 400 characters in one line is 170 wrapped phone lines; the line count on
    // its own cannot see it, which is why the character clause exists.
    expect(isLongPrompt("x".repeat(10_000))).toBe(false);
    expect(isLongPrompt("x".repeat(10_001))).toBe(true);
  });
});

describe("how tall a collapsed prompt is", () => {
  test("the budget is the shared line count, plus the bubble's own padding", () => {
    // 16 lines + 2 × 12dp padding: the cap has to include the padding or the
    // clipped box would show one line less than it promises.
    expect(collapsedPromptHeight(22, 915)).toBe(22 * PROMPT_COLLAPSE_LINES + 24);
  });

  test("a short window with large type caps at half of it", () => {
    // 16 × 30 = 480dp would be most of a short phone's 640dp window; a row that
    // fills the viewport is the shape this module exists to prevent.
    expect(collapsedPromptHeight(30, 640)).toBe(320);
  });

  test("the cap grows with the reader's text size until the window bounds it", () => {
    const medium = collapsedPromptHeight(22, 915);
    const large = collapsedPromptHeight(25.3, 915);
    expect(large).toBeGreaterThan(medium);
  });
});
