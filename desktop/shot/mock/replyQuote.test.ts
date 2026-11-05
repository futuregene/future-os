import { describe, expect, it } from "vitest";
import { QUOTED_PROMPT_LIMIT, quotedPrompt } from "./replyQuote";

describe("quotedPrompt", () => {
  // #1534: the demo reply echoed `prompt.slice(0, 16)`, so this 19-character
  // request was quoted up to "并" and the reply read as if the rest of the
  // sentence had never been sent.
  it("quotes the whole request instead of its first characters", () => {
    const prompt = "帮我把今天的文献整理成综述初稿并核对引用";
    expect(quotedPrompt(prompt)).toBe(prompt);
  });

  it("abbreviates a long request with an ellipsis rather than a silent cut", () => {
    const prompt = "整理".repeat(QUOTED_PROMPT_LIMIT);
    const quoted = quotedPrompt(prompt);
    expect(quoted.endsWith("…")).toBe(true);
    // The limit counts the reader's characters, the ellipsis is on top of them.
    expect(Array.from(quoted)).toHaveLength(QUOTED_PROMPT_LIMIT + 1);
  });

  it("never splits a surrogate pair at the cut", () => {
    const prompt = "🙂".repeat(QUOTED_PROMPT_LIMIT + 5);
    expect(quotedPrompt(prompt)).toBe(`${"🙂".repeat(QUOTED_PROMPT_LIMIT)}…`);
  });

  it("trims surrounding whitespace so the quote starts at the reader's words", () => {
    expect(quotedPrompt("  帮我把引用核对一遍 \n")).toBe("帮我把引用核对一遍");
  });
});
