import { createStreamingMarkdownParser, parseFutureMarkdown } from "@future-os/markdown";
import { describe, expect, it, vi } from "vitest";
import * as parser from "../../../../packages/markdown/src/parseFutureMarkdown";

const fixtures = [
  "# Heading\n\nFirst **paragraph**.\n\nSecond *paragraph* with `code`.\n",
  "Heading\n=======\n\nparagraph\n---\n\nlast",
  "- one\n- two\n\n  more\n\n- three\n\nEnd",
  "> quote\n>\n> next\n\nparagraph\n\n    code\n    more",
  "before\n\n```js\nconst x = 1;\n\nconst y = 2;\n```\n\nafter",
  "before\n\n~~~\ncode\n\n~~~\n\nafter",
  "  # indented\n\n  paragraph\n\n    code\n\n  tail",
  "before\n\n| A | B |\n|---|:---:|\n| x | y |\n| **z** | `a\\|b` |\n\nafter",
  "A | B\n--- | ---\nx | y\none | two\n",
  " | A | B |\n | --- | --- |\n | x | y |\n   | z | w |",
  "| A | B |\r\n|---|---|\r\n| x | y |\r\n| z | w |",
  "| A | B |\r|---|---|\r| x | y |\r| z | w |",
  "| A | B |\n|---|---|\n| 中文 | café |\n| $x$ | \\(y\\) |\n# next",
  "[old][ref]\n\ntext\n\n[ref]: https://example.com",
  "[old][two words]\n\ntext\n\n[two\nwords]: /target",
  "| A | B |\n|---|---|\n| [link](./file.md) | y |\n\n![x](pic.png)",
  "before\n\n- [x] task\n- [ ] other\n\nafter",
  "before\n\n$$\nx^2\n$$\n\nafter \\[y\\]",
  "before\n\n<div>\ninside\n</div>\n\nafter",
  "before\n\n<!-- comment\n\nend -->\n\nafter",
  "before\n\n***\n\nafter\n\n| One |\n| --- |\n| two | extra |",
];

describe("shared streaming markdown parser", () => {
  it.each(fixtures)("matches full parsing at every partial prefix: %s", (source) => {
    const project = createStreamingMarkdownParser();
    for (let length = 0; length <= source.length; length++) {
      const text = source.slice(0, length);
      expect(project(text, true)).toEqual(parseFutureMarkdown(text));
    }
    expect(project(source, false)).toEqual(parseFutureMarkdown(source));
  });

  it("reuses completed blocks and parses only the growing tail", () => {
    const project = createStreamingMarkdownParser();
    let text = `${Array.from({ length: 200 }, (_, n) => `Paragraph ${n}: **stable content**.\n\n`).join("")}tail`;
    const before = project(text, true);
    const parse = vi.spyOn(parser, "parseMdast");
    try {
      for (let i = 0; i < 100; i++) {
        text += "x";
        const next = project(text, true);
        expect(next.nodes[0]).toBe(before.nodes[0]);
      }
      expect(Math.max(...parse.mock.calls.map(([raw]) => raw.length))).toBeLessThan(256);
      expect(project(text, true)).toEqual(parseFutureMarkdown(text));
    }
    finally { parse.mockRestore(); }
  });

  it("keeps completed table row identities with preceding prose", () => {
    const project = createStreamingMarkdownParser();
    let text = "# Report\n\n| A | B |\n|---|---|\n| stable | row |\n| growing | **bo";
    const before = project(text, true);
    text += "ld** |\n| next | row |";
    const after = project(text, true);
    expect(after).toEqual(parseFutureMarkdown(text));
    expect(after.nodes[0]).toBe(before.nodes[0]);
    const first = before.nodes[1];
    const second = after.nodes[1];
    expect(first?.type).toBe("table");
    if (first?.type === "table" && second?.type === "table") {
      expect(second.rows[0]).toBe(first.rows[0]);
      expect(second.rows[1]).not.toBe(first.rows[1]);
    }
  });

  it("invalidates on replacement, reference definitions, clear and finalization", () => {
    const project = createStreamingMarkdownParser();
    for (const text of ["a\n\nb", "a\n\nb more", "replacement", "", "[a][id]\n\nmore", "[a][id]\n\nmore\n\n[id]: /x"])
      expect(project(text, true)).toEqual(parseFutureMarkdown(text));
    const final = "## done\n\n**complete**";
    expect(project(final, false)).toBe(parseFutureMarkdown(final));
  });

  it("does not pollute the settled parse cache with transient prefixes", () => {
    const settled = parseFutureMarkdown("settled cache sentinel");
    const project = createStreamingMarkdownParser();
    for (let i = 0; i < 600; i++) project(`stream ${i}`, true);
    expect(parseFutureMarkdown("settled cache sentinel")).toBe(settled);
  });

  it.each(["prose", "table"])("bounds parser work on 100 growing %s frames", (workload) => {
    let text = workload === "prose"
      ? `${Array.from({ length: 200 }, (_, n) => `Stable paragraph ${n}: **content**.\n\n`).join("")}tail`
      : `# Benchmark\n\n| A | B |\n|---|---|\n${Array.from({ length: 200 }, (_, n) => `| row ${n} | value |\n`).join("").trimEnd()}`;
    const frames = [text];
    for (let i = 0; i < 100; i++) {
      text += workload === "prose" ? "x" : `\n| next ${i} | **value** |`;
      frames.push(text);
    }
    const baselineStart = performance.now();
    for (const frame of frames) parseFutureMarkdown(frame, undefined, false);
    const baselineMs = performance.now() - baselineStart;
    const project = createStreamingMarkdownParser();
    const parse = vi.spyOn(parser, "parseMdast");
    try {
      const start = performance.now();
      for (const frame of frames) project(frame, true);
      const incrementalMs = performance.now() - start;
      const fullChars = frames.reduce((total, frame) => total + frame.length, 0);
      const incrementalChars = parse.mock.calls.reduce((total, [raw]) => total + raw.length, 0);
      expect(incrementalChars).toBeLessThan(fullChars / 10);
      expect(project(text, true)).toEqual(parseFutureMarkdown(text));
      console.warn(JSON.stringify({ workload, frames: frames.length, fullChars, incrementalChars, baselineMs: Math.round(baselineMs), incrementalMs: Math.round(incrementalMs) }));
    }
    finally { parse.mockRestore(); }
  }, 30_000);

  it("keeps the canonical deep-nesting fallback for the entire document", () => {
    const project = createStreamingMarkdownParser();
    project("prefix\n\n> ", true);
    const text = `prefix\n\n${"> ".repeat(70)}deep`;
    expect(project(text, true)).toEqual(parseFutureMarkdown(text));
  });
});
