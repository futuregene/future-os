import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
// @vitest-environment jsdom
import { expect, it } from "vitest";
import { loadKatex } from "../lazyKatex";
import { MathInline } from "./MathInline";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/**
 * The first formula is what fetches KaTeX: it is a chunk of its own, roughly a
 * quarter of the main bundle, and a conversation without maths should not pay
 * for it. The cost of that split is one paint without the engine, which this
 * test holds to the behaviour it promises — the source is visible rather than a
 * blank gap, and the formula upgrades in place once the chunk lands.
 */
it("shows the formula's source until KaTeX arrives, then the rendered formula", async () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);

  // No flush: the import is still in flight, so this is the first paint.
  act(() => {
    root.render(createElement(MathInline, { code: "E=mc^2" }));
  });
  expect(container.textContent).toBe("E=mc^2");
  expect(container.querySelector(".katex")).toBeNull();

  // The chunk lands, and the formula renders without remounting the line.
  await act(async () => {
    await loadKatex();
  });
  expect(container.querySelector(".katex")).toBeTruthy();
  expect(container.querySelector(".katex-mathml")).toBeTruthy();
  // The placeholder `<code>` is gone: the source text is no longer what renders.
  expect(container.querySelector("code")).toBeNull();

  act(() => root.unmount());
  container.remove();
});
