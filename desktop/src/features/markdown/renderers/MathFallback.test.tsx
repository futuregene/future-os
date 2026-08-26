import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { MathBlock } from "./MathBlock";
import { MathInline } from "./MathInline";

vi.mock("katex", () => ({
  default: {
    renderToString: vi.fn(() => {
      throw new Error("malformed LaTeX");
    }),
  },
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function mount(node: React.ReactElement) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => {
    root.render(node);
  });
  return {
    container,
    cleanup: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

describe("math fallback on KaTeX failure", () => {
  it("mathBlock falls back to escaped raw code when KaTeX throws", () => {
    const { container, cleanup } = mount(createElement(MathBlock, { code: "<b>bold & amp</b>" }));
    expect(container.querySelector(".text-red-500")).toBeTruthy();
    expect(container.textContent).toContain("<b>bold & amp</b>");
    cleanup();
  });

  it("mathInline falls back to escaped raw code when KaTeX throws", () => {
    const { container, cleanup } = mount(createElement(MathInline, { code: "<i>x</i>" }));
    expect(container.querySelector(".text-red-500")).toBeTruthy();
    expect(container.textContent).toContain("<i>x</i>");
    cleanup();
  });
});
