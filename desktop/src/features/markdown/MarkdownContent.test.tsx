import { splitStreamingMarkdown } from "@future-os/markdown";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { MarkdownContent } from "./MarkdownContent";

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
