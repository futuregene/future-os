import { describe, expect, it } from "vitest";
import { repairNestedDocumentFence } from "./fenceNesting";
import { parseFutureMarkdown } from "./parseFutureMarkdown";

const parse = (source: string) => parseFutureMarkdown(source, undefined, false);

/** The shape a model writes when it quotes a Markdown file that itself holds a
 * `futureos-choice` fence — taken from a real reply that rendered broken. */
const quotedDocument = [
  "完整的 `choice.md` 内容：",
  "",
  "```markdown",
  "# Offering Choices",
  "",
  "Some replies end by asking the user what to do next.",
  "",
  "```futureos-choice",
  "title: 接下来做哪一件？",
  "option: forest | 重画森林图",
  "option: flow | 补一张实验流程图",
  "```",
  "",
  "- `title:` — the question itself, one line.",
  "- `mode:` — `single` or `multi`.",
  "",
  "## 一个小标题",
  "",
  "Clients that cannot draw the card show it as a code block.",
  "```",
  "",
  "简言之，卡片必须是回复的最后一块。",
  "",
].join("\n");

describe("repairNestedDocumentFence", () => {
  it("reads a document fence as containing its inner fence", () => {
    const repaired = repairNestedDocumentFence(quotedDocument);
    // Only the two delimiters of the outer fence change; the swapped run keeps
    // its width so every source offset downstream stays put.
    expect(repaired).toBe(quotedDocument.replace(/```markdown\n/, "~~~markdown\n")
      .replace(/\n```\n\n简言之/, "\n~~~\n\n简言之"));
    expect(repaired.length).toBe(quotedDocument.length);
  });

  it("gives back the same string when nothing nests", () => {
    const plain = "```rust\nfn main() {}\n```\n\nafter\n";
    expect(repairNestedDocumentFence(plain)).toBe(plain);
  });

  it("leaves a non-document fence alone", () => {
    // A backtick run inside a `rust` block is text, and CommonMark already
    // reads it that way: ` ```futureos-choice ` never closes the outer fence.
    const source = "```rust\nlet a = 1;\n\n```futureos-choice\nx\n```\n\nafter\n";
    expect(repairNestedDocumentFence(source)).toBe(source);
  });

  it("leaves an explicitly nested four-backtick fence alone", () => {
    const source = "````markdown\n# T\n\n```futureos-choice\nx\n```\n````\n\nafter\n";
    expect(repairNestedDocumentFence(source)).toBe(source);
  });

  it("leaves a document fence whose quoted body has no inner fence alone", () => {
    const source = "```markdown\n# Title\n\ntext\n```\n\nafter\n";
    expect(repairNestedDocumentFence(source)).toBe(source);
  });

  it("leaves an unterminated document fence alone", () => {
    const source = "```markdown\n# Title\n\n```futureos-choice\nx\n";
    expect(repairNestedDocumentFence(source)).toBe(source);
  });

  it("stays idempotent once repaired", () => {
    const once = repairNestedDocumentFence(quotedDocument);
    expect(repairNestedDocumentFence(once)).toBe(once);
  });

  it("refuses to swap when the quoted body holds a tilde fence", () => {
    // A `~~~` run in the body would close the rewritten outer fence early, so
    // this shape keeps CommonMark's grouping rather than a wrong repair.
    const source = "```markdown\n# T\n\n~~~\nx\n~~~\n\n```futureos-choice\na\n```\n```\n";
    expect(repairNestedDocumentFence(source)).toBe(source);
  });
});

describe("a document fence that quotes another fenced block", () => {
  it("keeps the whole quoted file in one code block", () => {
    const nodes = parse(quotedDocument).nodes;
    // Before the repair this was `code`, `list`, `heading`, `paragraph` — the
    // list leaked out of the quote as untitled bullets in the reply body.
    expect(nodes.map((node) => node.type)).toEqual(["paragraph", "code", "paragraph"]);
    expect(nodes[1]).toEqual({
      code: [
        "# Offering Choices",
        "",
        "Some replies end by asking the user what to do next.",
        "",
        "```futureos-choice",
        "title: 接下来做哪一件？",
        "option: forest | 重画森林图",
        "option: flow | 补一张实验流程图",
        "```",
        "",
        "- `title:` — the question itself, one line.",
        "- `mode:` — `single` or `multi`.",
        "",
        "## 一个小标题",
        "",
        "Clients that cannot draw the card show it as a code block.",
      ].join("\n"),
      language: "markdown",
      type: "code",
    });
    expect(nodes[2]).toEqual({
      children: [{ text: "简言之，卡片必须是回复的最后一块。", type: "text" }],
      type: "paragraph",
    });
  });

  it("keeps a shell transcript below it as code, not as prose with math", () => {
    // The same failure on the real reply: once the quote leaked, the `$(…)` of a
    // shell line was read as an inline formula and the block's lines became
    // paragraphs.
    const source = [
      "```markdown",
      "# Doc",
      "",
      "```futureos-choice",
      "title: T",
      "```",
      "```",
      "",
      "```",
      '$ strings $(which future) | grep -c "Offering Choices"',
      "0",
      "```",
      "",
    ].join("\n");
    const nodes = parse(source).nodes;
    expect(nodes).toEqual([
      { code: "# Doc\n\n```futureos-choice\ntitle: T\n```", language: "markdown", type: "code" },
      { code: '$ strings $(which future) | grep -c "Offering Choices"\n0', type: "code" },
    ]);
  });

  it("keeps a tilde-fenced quote nested inside a backtick fence working", () => {
    // The mirror image needs no repair: a `~` run cannot close a backtick fence.
    const source = "~~~markdown\n```futureos-choice\ntitle: T\n```\n~~~\n";
    expect(parse(source).nodes).toEqual([
      { code: "```futureos-choice\ntitle: T\n```", language: "markdown", type: "code" },
    ]);
  });
});
