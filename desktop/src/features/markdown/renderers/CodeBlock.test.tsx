// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { LiveMarkdownProvider } from "../LiveMarkdownContext";

let highlighterState: {
  highlight: (code: string, language?: string) => unknown;
  isLoaded: boolean;
};

vi.mock("../useCodeHighlighter", () => ({
  useCodeHighlighter: () => highlighterState,
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: () => Promise.resolve(null),
}));

import { CodeBlock } from "./CodeBlock";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

beforeEach(() => {
  highlighterState = { highlight: () => null, isLoaded: false };
});

const highlighted = {
  bgColor: "#fff",
  fgColor: "#000",
  lines: [
    {
      tokens: [
        { content: "const", color: "#d73a49", fontStyle: 1 },
        { content: " x", color: undefined, fontStyle: 0 },
      ],
    },
  ],
};

describe("CodeBlock", () => {
  it("renders plain text with the language label while the highlighter loads", () => {
    const html = renderToStaticMarkup(createElement(CodeBlock, { code: "x = 1", language: "python" }));
    expect(html).toContain("x = 1");
    expect(html).toContain("python");
    expect(html).toContain("<pre");
  });

  it("omits the language label when no language is given", () => {
    const html = renderToStaticMarkup(createElement(CodeBlock, { code: "x" }));
    expect(html).not.toContain("mb-2");
  });

  it("renders plain text for the live streaming tail even when loaded", () => {
    highlighterState = { highlight: () => highlighted, isLoaded: true };
    const html = renderToStaticMarkup(createElement(
      LiveMarkdownProvider,
      { value: true },
      createElement(CodeBlock, { code: "x", language: "ts" }),
    ));
    expect(html).not.toContain("#d73a49");
  });

  it("renders plain text when highlight returns null", () => {
    highlighterState = { highlight: () => null, isLoaded: true };
    const html = renderToStaticMarkup(createElement(CodeBlock, { code: "x", language: "ts" }));
    expect(html).not.toContain("#d73a49");
  });

  it("renders highlighted tokens with colors and italic font style", () => {
    highlighterState = { highlight: () => highlighted, isLoaded: true };
    const html = renderToStaticMarkup(createElement(CodeBlock, { code: "const x", language: "ts" }));
    expect(html).toContain("#d73a49");
    expect(html).toContain("italic");
    expect(html).toContain("background-color:#fff");
  });

  it("copy button copies the code", async () => {
    const exec = vi.fn().mockReturnValue(true);
    Object.defineProperty(document, "execCommand", { value: exec, configurable: true });
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    act(() => {
      root.render(createElement(CodeBlock, { code: "const x = 1", language: "ts" }));
    });
    const button = container.querySelector("button")!;
    await act(async () => {
      button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });
    expect(exec).toHaveBeenCalledWith("copy");
    act(() => root.unmount());
    container.remove();
  });
});
