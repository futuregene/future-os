// @vitest-environment jsdom
import type { ReactElement } from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MarkdownContent } from "./MarkdownContent";

/**
 * A reply that drew several pictures is one drawing: the lightbox steps through
 * that reply's images in the order it drew them, wraps at both ends, and answers
 * the arrows, buttons and a horizontal trackpad swipe.
 */

vi.mock("../../integrations/storage/markdownReferences", () => ({
  resolveMarkdownReferences: () => Promise.resolve([]),
}));

vi.mock("../../integrations/storage/files", () => ({
  openExternalUrl: () => Promise.resolve(),
  openPath: () => Promise.resolve(),
  prepareImagePreviewUrl: (path: string) => Promise.resolve(`asset:${path}`),
  readTextFilePreview: () => Promise.resolve({ content: "", size: 0, truncated: false }),
  resolvePreviewLinkPath: () => Promise.resolve(null),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => {}),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const cleanups: Array<() => void> = [];
afterEach(() => cleanups.splice(0).forEach(cleanup => cleanup()));

function mount(node: ReactElement) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => root.render(node));
  cleanups.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return container;
}

function click(element: Element | null) {
  expect(element).not.toBeNull();
  act(() => element!.dispatchEvent(new MouseEvent("click", { bubbles: true })));
}

const TWO_IMAGES = "![first](https://example.com/first.png)\n\n![second](https://example.com/second.png)";

function openFirst(container: HTMLElement) {
  const triggers = container.querySelectorAll<HTMLButtonElement>("button[aria-haspopup=dialog]");
  expect(triggers).toHaveLength(2);
  click(triggers[0]!);
  return document.querySelector<HTMLElement>("[role=dialog]")!;
}

function shown(dialog: HTMLElement): string | null | undefined {
  return dialog.querySelector("img")?.getAttribute("src");
}

function step(dialog: HTMLElement, label: "Next image" | "Previous image") {
  click(dialog.querySelector(`button[aria-label="${label}"]`));
}

describe("stepping through a drawing's pictures", () => {
  it("opens on the picture that was tapped and walks to both neighbours", () => {
    const container = mount(<MarkdownContent content={TWO_IMAGES} />);
    const dialog = openFirst(container);
    expect(shown(dialog)).toBe("https://example.com/first.png");
    expect(dialog.textContent).toContain("1 / 2");

    step(dialog, "Next image");
    expect(shown(dialog)).toBe("https://example.com/second.png");
    expect(dialog.textContent).toContain("2 / 2");
    expect(dialog.getAttribute("aria-label")).toBe("second");

    step(dialog, "Previous image");
    expect(shown(dialog)).toBe("https://example.com/first.png");
  });

  it("wraps at both ends, so no arrow is ever a dead control", () => {
    const container = mount(<MarkdownContent content={TWO_IMAGES} />);
    const dialog = openFirst(container);

    // Left from the first picture lands on the last.
    step(dialog, "Previous image");
    expect(shown(dialog)).toBe("https://example.com/second.png");

    // And right from the last comes back to the first.
    step(dialog, "Next image");
    expect(shown(dialog)).toBe("https://example.com/first.png");
  });

  it("answers the Left and Right arrow keys the same way", () => {
    const container = mount(<MarkdownContent content={TWO_IMAGES} />);
    const dialog = openFirst(container);

    act(() => dialog.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "ArrowLeft" })));
    expect(shown(dialog)).toBe("https://example.com/second.png");

    act(() => dialog.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "ArrowRight" })));
    expect(shown(dialog)).toBe("https://example.com/first.png");
  });

  it("pages on a horizontal trackpad swipe and ignores a vertical scroll", () => {
    const container = mount(<MarkdownContent content={TWO_IMAGES} />);
    const dialog = openFirst(container);

    act(() => dialog.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaX: 80, deltaY: 2 })));
    expect(shown(dialog)).toBe("https://example.com/second.png");

    // A vertical scroll is the page's, not the drawing's.
    act(() => dialog.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaX: 0, deltaY: 120 })));
    expect(shown(dialog)).toBe("https://example.com/second.png");
  });

  it("keeps a single picture a single picture: no arrows, no counter", () => {
    const container = mount(<MarkdownContent content="![only](https://example.com/only.png)" />);
    const trigger = container.querySelector<HTMLButtonElement>("button[aria-haspopup=dialog]")!;
    click(trigger);
    const dialog = document.querySelector<HTMLElement>("[role=dialog]")!;
    expect(shown(dialog)).toBe("https://example.com/only.png");
    expect(dialog.querySelector(`button[aria-label="Next image"]`)).toBeNull();
    expect(dialog.querySelector(`button[aria-label="Previous image"]`)).toBeNull();
  });
});
