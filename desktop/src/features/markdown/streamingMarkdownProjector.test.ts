import { parseFutureMarkdown } from "@future-os/markdown";
import { describe, expect, it } from "vitest";
import { createStreamingMarkdownProjector } from "./streamingMarkdownBlocks";

function assertEquivalent(project: ReturnType<typeof createStreamingMarkdownProjector>, text: string, live = true) {
  const blocks = project(text, live);
  expect(blocks.map(block => block.content).join("")).toBe(text);
  expect(blocks.flatMap(block => block.document!.nodes)).toEqual(parseFutureMarkdown(text).nodes);
  expect(blocks.every(block => block.live === live)).toBe(blocks.length <= 1 || !live);
  return blocks;
}

describe("incremental single-table projection", () => {
  it.each([
    "| A | B |\n| --- | :---: |\n| first | second |\n| **next** | `a\\|b` |\n| end | done |",
    "A | B\n--- | ---\none | two\nthree | four\n",
    " | A | B |\n | --- | --- |\n | one | two |\n   | three | four |\n | five | six |",
    "| A | B |\r\n| --- | --- |\r\n| one | two |\r\n| three | four |\r\n",
    "| A | B |\r| --- | --- |\r| one | two |\r| three | four |\r",
    "| A | B |\n| --- | --- |\n| 中文 | café |\n| $x^2$ | \\(y\\) |\n",
    "| A | B |\n| --- | --- |\n| escaped \\| pipe | **strong** |\n| [url](https://example.com) | ![x](https://example.com/x.png) |",
    "| A | B |\n| --- | --- |\n| [x][id] | y |\n\n[id]: https://example.com",
    "| A | B |\n| --- | --- |\n| [file](./a.txt) | y |\n| z | next |",
    "| A | B |\n| --- | --- |\n| one | two |\n\nParagraph after table.",
    "| A | B |\n| --- | --- |\n| one | two |\n# Heading\n\n- list",
    "| A | B |\n| --- | --- |\n| one | two |\n\n```js\nconst a = 1;\n```",
    "| A | B |\n| --- | --- |\n| one | two |\n\n| C | D |\n| --- | --- |\n| three | four |",
    "| A | B |\n| --- | --- |\n| first | second |\n\n\n",
    "| one |\n| --- |\n| two |\n| three | extra |\n| last |",
  ])("matches the canonical parser at every partial prefix: %s", (source) => {
    const project = createStreamingMarkdownProjector();
    for (let length = 0; length <= source.length; length++) {
      const text = source.slice(0, length);
      const blocks = project(text, true);
      expect(blocks.map(block => block.content).join("")).toBe(text);
      expect(blocks.flatMap(block => block.document!.nodes)).toEqual(parseFutureMarkdown(text).nodes);
    }
    const final = project(source, false);
    expect(final.every(block => !block.live)).toBe(true);
    expect(final.flatMap(block => block.document!.nodes)).toEqual(parseFutureMarkdown(source).nodes);
  });

  it("retains completed row objects and reparses a growing last row", () => {
    const project = createStreamingMarkdownProjector();
    const text = "| A | B |\n| --- | --- |\n| stable | row |\n| growing | **bo";
    const before = assertEquivalent(project, text)[0]!.document!.nodes[0]!;
    const after = assertEquivalent(project, `${text}ld** |\n| next | row |`)[0]!.document!.nodes[0]!;
    expect(before.type).toBe("table");
    expect(after.type).toBe("table");
    if (before.type === "table" && after.type === "table") {
      expect(after.rows[0]).toBe(before.rows[0]);
      expect(after.rows[1]).not.toBe(before.rows[1]);
    }
  });

  it("invalidates checkpoints on replacement and reuses a settled table safely", () => {
    const project = createStreamingMarkdownProjector();
    const first = "| A | B |\n| --- | --- |\n| one | two |";
    assertEquivalent(project, first);
    assertEquivalent(project, first, false);
    assertEquivalent(project, "replacement");
    assertEquivalent(project, first);
    assertEquivalent(project, `${first}\n| next | row |`);
    assertEquivalent(project, "");
    assertEquivalent(project, first);
  });

  it("matches full parsing over many batched row appends", () => {
    const project = createStreamingMarkdownProjector();
    let text = "| A | B |\n| --- | --- |\n| initial | row |";
    for (let batch = 0; batch < 30; batch++) {
      text += `\n| batch ${batch} | **value** |\n| more | \\(x_${batch}\\) |`;
      assertEquivalent(project, text);
    }
  });
});
