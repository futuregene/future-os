import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { upsertFutureReferenceData } from "./futureReferenceStore";
import { MarkdownContent } from "./MarkdownContent";

describe("markdown content", () => {
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
    // Even with resolvable run data in the store, app-object embeds are disabled:
    // the fence is shown verbatim as code, never resolved into a run card.
    upsertFutureReferenceData("workspace_test", "run", "run_store", {
      createdAt: 1,
      id: "run_store",
      status: "running",
      threadId: "thread_1",
      updatedAt: 1,
    });

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
