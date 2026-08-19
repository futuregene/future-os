import { renderToStaticMarkup } from "react-dom/server";
// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { MarkdownContent } from "./MarkdownContent";

vi.mock("../../integrations/storage/markdownReferences", () => ({
  resolveMarkdownReferences: () => Promise.resolve([]),
}));

vi.mock("../../integrations/storage/files", () => ({
  openPath: () => Promise.resolve(),
  openExternalUrl: () => Promise.resolve(),
  readFileBase64: () => Promise.resolve("QUJD"),
  readTextFilePreview: () => Promise.resolve({ content: "", size: 0, truncated: false }),
  resolvePreviewLinkPath: () => Promise.resolve(null),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => {}),
}));

describe("math rendering in MarkdownContent", () => {
  it("renders block-level $$...$$ math as a KaTeX display element", () => {
    const html = renderToStaticMarkup(
      <MarkdownContent content={"$$L = -\\left[y \\log p + (1-y)\\log(1-p)\\right]$$"} />,
    );
    expect(html).toContain("katex-display");
    expect(html).toContain("katex-mathml");
    expect(html).not.toContain("$$");
  });

  it("renders inline $...$ math as a KaTeX inline element", () => {
    const html = renderToStaticMarkup(
      <MarkdownContent content={"The formula $E=mc^2$ is famous."} />,
    );
    expect(html).toContain("katex");
    expect(html).toContain("katex-mathml");
    // Inline math should NOT use the display wrapper
    expect(html).not.toContain("katex-display");
    expect(html).not.toContain("$E=mc^2$");
  });

  it("keeps prose text around inline math intact", () => {
    const html = renderToStaticMarkup(
      <MarkdownContent content={"Loss is $L(y, \\hat{y})$ where $y$ is the label."} />,
    );
    expect(html).toContain("Loss is");
    expect(html).toContain("where");
    expect(html).toContain("is the label.");
  });

  it("falls back gracefully on malformed math instead of crashing", () => {
    const html = renderToStaticMarkup(
      <MarkdownContent content={"Bad formula $\\frac{unclosed$ here"} />,
    );
    // No exception thrown; renders some HTML (fallback path shows raw code or error span)
    expect(html.length).toBeGreaterThan(0);
  });
});
