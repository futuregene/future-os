import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { MarkdownContent, StreamingMarkdownContent } from "./MarkdownContent";
import { splitStreamingMarkdown } from "./streamingMarkdownBlocks";

describe("markdown content", () => {
  it("keeps completed top-level blocks stable and only marks the tail live", () => {
    const blocks = splitStreamingMarkdown("First paragraph.\n\nSecond paragraph", true);

    expect(blocks).toEqual([
      { content: "First paragraph.\n\n", live: false, start: 0 },
      { content: "Second paragraph", live: true, start: 18 },
    ]);
  });

  it("keeps reference-definition documents whole while streaming", () => {
    const content = "Read [the guide][docs].\n\n[docs]: https://example.com";
    expect(splitStreamingMarkdown(content, true)).toEqual([
      { content, live: true, start: 0 },
    ]);
  });

  it("preserves complete Unicode graphemes across streaming block boundaries", () => {
    const content = "Family 👨‍👩‍👧‍👦 and café\u0301.\n\n第二段 👍🏽";
    const blocks = splitStreamingMarkdown(content, true);

    expect(blocks.map(block => block.content).join("")).toBe(content);
    expect(blocks[0]?.content).toContain("👨‍👩‍👧‍👦");
    expect(blocks[blocks.length - 1]?.content).toContain("👍🏽");
  });

  it("renders GFM table and inline formatting through the markdown runtime", () => {
    const html = renderToStaticMarkup(
      <MarkdownContent
        content={[
          "| Name | Link |",
          "| --- | --- |",
          "| ~~Old~~ | [**Docs**](https://example.com/docs) |",
        ].join("\n")}
      />,
    );

    expect(html).toContain("<table");
    expect(html).toContain("<del");
    expect(html).toContain("<strong");
    expect(html).toContain("href=\"https://example.com/docs\"");
  });

  it("does not render raw HTML as executable markup", () => {
    const html = renderToStaticMarkup(<MarkdownContent content="<script>alert(1)</script>" />);

    expect(html).not.toContain("<script>");
    // Non-whitelisted markup is kept verbatim as inert text (escaped), never executed.
    expect(html).toContain("&lt;script&gt;alert(1)&lt;/script&gt;");
  });

  it("renders unresolved references as neutral placeholders, not the red missing badge", () => {
    const html = renderToStaticMarkup(
      <MarkdownContent content="[artifact:Report](futureos://artifact/artifact_123)" />,
    );

    // Without a workspace the resolve IPC can't run, so `resolved` is undefined —
    // a pending state, not a failure. Show the label neutrally; the red badge is
    // reserved for genuinely missing / failed targets.
    expect(html).not.toContain("Missing");
    expect(html).toContain("artifact:Report");
  });

  it("renders a disabled app-object embed as a plain code block (minimal link mode)", () => {
    // App-object embeds are disabled at parse level: the fence is shown
    // verbatim as code, never resolved into a run card.
    const html = renderToStaticMarkup(
      <MarkdownContent
        content={[
          "```futureos-run",
          "id: run_store",
          "view: card",
          "```",
        ].join("\n")}
        workspaceId="workspace_test"
      />,
    );

    expect(html).toContain("run_store");
    expect(html).not.toContain("running");
  });
});

describe("splitStreamingMarkdown empty input", () => {
  it("returns no blocks for an empty string", () => {
    expect(splitStreamingMarkdown("", true)).toEqual([]);
  });
});

describe("streamingMarkdownContent", () => {
  it("splits top-level blocks and renders each through MarkdownContent", () => {
    const html = renderToStaticMarkup(
      <StreamingMarkdownContent content="First paragraph.\n\nSecond paragraph" live />,
    );

    expect(html).toContain("First paragraph");
    expect(html).toContain("Second paragraph");
    expect(html).toContain("space-y-3");
  });

  it("marks the growing tail for layout containment and leaves closed blocks untouched", () => {
    // While a reply streams, only the still-growing tail gets `data-streamed-block`
    // (MessageBlock scopes `contain: layout style` to it, so a delta's relayout
    // stays inside the tail instead of re-walking the whole message).
    const live = renderToStaticMarkup(
      <StreamingMarkdownContent content="Done.\n\nGrowing" live />,
    );
    expect(live.match(/data-streamed-block=""/g)).toHaveLength(1);

    // Settled content is never marked: the browser measures it exactly once.
    const settled = renderToStaticMarkup(
      <StreamingMarkdownContent content="Done.\n\nGrowing" live={false} />,
    );
    expect(settled).not.toContain("data-streamed-block");
  });
});

describe("choice cards in a reply", () => {
  const fence = [
    "先做这一步。",
    "",
    "```futureos-choice",
    "title: 接下来做哪一件？",
    "option: forest | 重画森林图 | 按 Dreher 2025 的样本量重算",
    "option: flow | 补一张实验流程图",
    "```",
  ].join("\n");

  it("renders the fence as a card, not as a code block", () => {
    const html = renderToStaticMarkup(<MarkdownContent content={fence} />);
    expect(html).toContain("data-choice-card");
    expect(html).toContain("重画森林图");
    expect(html).not.toContain("futureos-choice");
  });

  it("leaves every card read-only when no answer surface is passed", () => {
    const html = renderToStaticMarkup(<MarkdownContent content={fence} />);
    expect(html).toContain("data-choice-state=\"readonly\"");
    expect(html).not.toContain("data-choice-state=\"active\"");
  });

  it("makes only the last card of the document interactive", () => {
    const twoCards = [
      "```futureos-choice",
      "title: 第一问",
      "option: a | A",
      "option: b | B",
      "```",
      "",
      "```futureos-choice",
      "title: 第二问",
      "option: c | C",
      "option: d | D",
      "```",
    ].join("\n");
    const html = renderToStaticMarkup(
      <MarkdownContent choiceAnswer={{ onSelect: () => {}, selectedIds: ["c"] }} content={twoCards} />,
    );
    // Two cards, exactly one of them live, and the answer marks only that one.
    expect(html.match(/data-choice-card/g)).toHaveLength(2);
    expect(html.match(/data-choice-state="active"/g)).toHaveLength(1);
    expect(html.match(/data-choice-selected="true"/g)).toHaveLength(1);
  });
});
