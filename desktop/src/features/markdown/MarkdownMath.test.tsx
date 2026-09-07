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
  prepareImagePreviewUrl: (path: string) => Promise.resolve(`asset:${path}`),
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
      <MarkdownContent content="The formula $E=mc^2$ is famous." />,
    );
    expect(html).toContain("katex");
    expect(html).toContain("katex-mathml");
    // Inline math should NOT use the display wrapper
    expect(html).not.toContain("katex-display");
    expect(html).not.toContain("$E=mc^2$");
  });

  it("renders LaTeX parentheses within bold prose as inline KaTeX", () => {
    const html = renderToStaticMarkup(
      <MarkdownContent content={String.raw`最少需要 **\(\boxed{21}\)** 块瓷砖。`} />,
    );
    expect(html).toContain("katex-mathml");
    expect(html).not.toContain("katex-display");
    expect(html).not.toContain("katex-error");
    expect(html).toContain("最少需要");
    expect(html).toContain("块瓷砖。");
  });

  it("renders bracket-delimited aligned equations from the tiling answer", () => {
    const html = renderToStaticMarkup(
      <MarkdownContent content={String.raw`\[
\begin{aligned}
&\underbrace{(16-a-b+1)}_{\text{不在两条链上}}
+\underbrace{2(a+b-2)}_{\text{只在一条链上}}+4\\
&=16+a+b+1.
\end{aligned}
\]`}
      />,
    );
    expect(html).toContain("katex-display");
    expect(html).toContain("katex-mathml");
    expect(html).not.toContain("katex-error");
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
