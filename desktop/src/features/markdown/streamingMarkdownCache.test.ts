import { parseFutureMarkdown } from "@future-os/markdown";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createStreamingMarkdownProjector, splitStreamingMarkdown } from "./streamingMarkdownBlocks";

vi.mock("@future-os/markdown", async (original) => {
  const actual = await original<typeof import("@future-os/markdown")>();
  return { ...actual, parseFutureMarkdown: vi.fn(actual.parseFutureMarkdown) };
});
afterEach(() => vi.mocked(parseFutureMarkdown).mockClear());

describe("streaming Markdown cache ownership", () => {
  it("does not admit ephemeral live documents to the shared static parse cache", () => {
    const text = "- unique live cache fixture\n- **last**";
    const live = createStreamingMarkdownProjector()(text, true)[0]!.document;
    const settled = parseFutureMarkdown(text);
    expect(settled).not.toBe(live);
    expect(settled).toEqual(live);
    expect(parseFutureMarkdown(text)).toBe(settled);
  });

  it.each([
    "- list finalization\n- **last**",
    "## Mixed finalization\n\nparagraph\n\n```js\nconst x = 1;\n```",
    "```text\nunfinished code finalization",
  ])("does not parse identical source again for live/settled transitions: %s", (text) => {
    const project = createStreamingMarkdownProjector();
    const first = project(text, true);
    vi.mocked(parseFutureMarkdown).mockClear();
    const settled = project(text, false);
    const resumed = project(text, true);
    expect(parseFutureMarkdown).not.toHaveBeenCalled();
    expect(settled.every(block => !block.live)).toBe(true);
    expect(resumed[resumed.length - 1]?.live).toBe(true);
    expect(settled.map(block => block.document)).toEqual(first.map(block => block.document));
    expect(settled[0]?.document).toBe(first[0]?.document);
  });

  it("converts existing mdast children instead of reparsing every ordinary block", () => {
    const text = "## Reuse unique tree\n\nparagraph **bold**\n\n- list\n\n```text\ncode\n```";
    const blocks = createStreamingMarkdownProjector()(text, true);
    expect(blocks).toHaveLength(4);
    expect(vi.mocked(parseFutureMarkdown).mock.calls.every(call => call[1] !== undefined)).toBe(true);
  });

  it.each([
    "## Heading\n\nparagraph *emphasis* and `code`\n\n> quote\n>\n> - nested list\n\n```js\nconst x = 1;\n```",
    "first\n\nsecond\n---\n\n| A | B |\n| --- | --- |\n| x | y |\n\nlast",
    "[outside][id]\n\n- item\n\n  [id]: https://example.com\n\nTail",
    "before\n\n<!-- multi\nline -->\n\n\\[\nx^2\n\\]\n\nend",
  ])("matches isolated-block parsing at every mixed-source prefix: %s", (source) => {
    const project = createStreamingMarkdownProjector();
    for (let length = 0; length <= source.length; length++) {
      const text = source.slice(0, length);
      const actual = project(text, true).map(block => block.document);
      // Bypass the cache so the oracle cannot reuse a candidate's cached AST.
      const expected = splitStreamingMarkdown(text, true).map(block => parseFutureMarkdown(block.content, undefined, false));
      expect(actual).toEqual(expected);
    }
  });

  it.each([
    "[outside][id]\n\n> [id]: https://example.com\n\nTail",
    "Outside[^id]\n\n[^id]: a footnote\n\nTail",
    "[outside][id]\n\n[id]: https://example.com\n\nTail",
    "# Heading\n\n\\[\nx^2 + y^2\n\\]\n\n> quoted\n\n- [x] task",
  ])("preserves existing block rendering for definitions and mixed syntax: %s", (text) => {
    const expected = splitStreamingMarkdown(text, true).map(block => parseFutureMarkdown(block.content, undefined, false));
    const actual = createStreamingMarkdownProjector()(text, true).map(block => block.document);
    expect(actual).toEqual(expected);
  });
});
