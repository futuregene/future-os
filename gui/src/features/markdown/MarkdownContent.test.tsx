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

  it("keeps future references as safe unresolved chips without a workspace", () => {
    const html = renderToStaticMarkup(
      <MarkdownContent content="[artifact:Report](futureos://artifact/artifact_123)" />,
    );

    expect(html).toContain("Missing");
    expect(html).toContain("artifact");
  });

  it("renders references from the shared future reference store", () => {
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

    expect(html).toContain("running");
    expect(html).toContain("run_store");
  });
});
