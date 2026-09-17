// @vitest-environment jsdom
import type { ReactElement } from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { MarkdownImageView } from "./renderers/MarkdownImageView";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const cleanups: Array<() => void> = [];
afterEach(() => cleanups.splice(0).forEach(cleanup => cleanup()));

function mount(node: ReactElement) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const render = (value: ReactElement) => act(() => root.render(value));
  render(node);
  cleanups.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return { container, render };
}

function click(element: Element | null) {
  expect(element).not.toBeNull();
  act(() => element!.dispatchEvent(new MouseEvent("click", { bubbles: true })));
}

describe("markdown image preview", () => {
  it.each(["https://example.com/chart.png", "asset:/workspace/chart.png"])("opens %s in a portal and restores focus after Escape", (src) => {
    const { container } = mount(<MarkdownImageView alt="chart" src={src} />);
    const trigger = container.querySelector<HTMLButtonElement>("button[aria-haspopup=dialog]")!;
    trigger.focus();
    click(container.querySelector("img"));
    const dialog = document.querySelector("[role=dialog]")!;
    expect(dialog).not.toBeNull();
    expect(container.contains(dialog)).toBe(false);
    expect(dialog.querySelector("img")?.getAttribute("src")).toBe(src);
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    expect(dialog.contains(document.activeElement)).toBe(true);
    act(() => document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true })));
    expect(dialog.contains(document.activeElement)).toBe(true);
    act(() => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })));
    expect(document.querySelector("[role=dialog]")).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it.each(["button.absolute", "button.fixed"])("closes using %s", (selector) => {
    const { container } = mount(<MarkdownImageView alt="chart" src="https://example.com/chart.png" />);
    click(container.querySelector("img"));
    click(document.querySelector(`[role=dialog] ${selector}`));
    expect(document.querySelector("[role=dialog]")).toBeNull();
  });

  it("keeps linked images free of nested buttons and preview actions", () => {
    const { container } = mount(<a href="https://example.com"><MarkdownImageView alt="chart" linked src="https://example.com/chart.png" /></a>);
    expect(container.querySelector("a img")).not.toBeNull();
    expect(container.querySelector("a button")).toBeNull();
    expect(document.querySelector("[role=dialog]")).toBeNull();
  });

  it("keeps long-image inline expansion independent from preview", () => {
    const { container } = mount(<MarkdownImageView alt="chart" src="https://example.com/chart.png" />);
    const image = container.querySelector("img")!;
    Object.defineProperty(image, "naturalHeight", { value: 1200 });
    act(() => image.dispatchEvent(new Event("load")));
    const expand = container.querySelector("button[aria-expanded]")!;
    click(expand);
    expect(expand.getAttribute("aria-expanded")).toBe("true");
    expect(image.className).toContain("max-h-none");
    click(image);
    expect(document.querySelector("[role=dialog]")).not.toBeNull();
    act(() => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })));
    click(expand);
    expect(image.className).toContain("max-h-80");
  });

  it("retries failed images and resets preview state when the source changes", () => {
    const { container, render } = mount(<MarkdownImageView alt="chart" src="https://example.com/first.png" />);
    act(() => container.querySelector("img")!.dispatchEvent(new Event("error")));
    expect(container.querySelector("img")).toBeNull();
    click(container.querySelector("button"));
    click(container.querySelector("img"));
    expect(document.querySelector("[role=dialog]")).not.toBeNull();
    render(<MarkdownImageView alt="chart" src="https://example.com/second.png" />);
    expect(document.querySelector("[role=dialog]")).toBeNull();
    expect(container.querySelector("img")?.getAttribute("src")).toBe("https://example.com/second.png");
  });

  it("dismisses a preview that fails to load without losing the inline image", () => {
    const { container } = mount(<MarkdownImageView alt="chart" src="https://example.com/chart.png" />);
    click(container.querySelector("img"));
    act(() => document.querySelector("[role=dialog] img")!.dispatchEvent(new Event("error")));
    expect(document.querySelector("[role=dialog]")).toBeNull();
    expect(container.querySelector("img")).not.toBeNull();
  });
});
