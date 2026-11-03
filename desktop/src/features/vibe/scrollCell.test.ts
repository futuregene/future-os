// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { SCROLL_STEP_RATIO, scrollToLiveEnd, scrollTranscript, transcriptOf } from "./scrollCell";

/**
 * Moving one cell's transcript from the keyboard.
 *
 * Two things here are worth a test of their own: the lookup is **scoped to the panel**
 * (a transcript elsewhere in the app must never be the one that moves, and a cell key
 * contains `:` which is a selector separator), and the engines without `scrollBy` /
 * `scrollTo` — the reason those two functions have a fallback branch at all — still
 * scroll, through the property.
 */
function panel(html: string): HTMLElement {
  const root = document.createElement("div");
  root.innerHTML = html;
  document.body.append(root);
  return root;
}

afterEach(() => {
  document.body.innerHTML = "";
});

describe("transcriptOf", () => {
  it("finds the transcript inside the cell it was asked for", () => {
    const root = panel(`
      <div data-vibe-transcript-for="local:1"><div data-chat-scroll="a"></div></div>
      <div data-vibe-transcript-for="local:2"><div data-chat-scroll="b"></div></div>
    `);
    expect(transcriptOf(root, "local:2")?.getAttribute("data-chat-scroll")).toBe("b");
  });

  it("escapes the cell key rather than treating `:` as a selector separator", () => {
    const root = panel(`<div data-vibe-transcript-for="remote:d1:s2"><div data-chat-scroll="r"></div></div>`);
    expect(transcriptOf(root, "remote:d1:s2")).not.toBeNull();
  });

  it("escapes a quote in the key instead of building a broken selector", () => {
    const root = document.createElement("div");
    root.innerHTML = `<div data-vibe-transcript-for='local:"x'><div data-chat-scroll="q"></div></div>`;
    document.body.append(root);
    expect(transcriptOf(root, "local:\"x")).not.toBeNull();
  });

  it("answers null with no panel, no key, or a cell that has no transcript", () => {
    const root = panel(`<div data-vibe-transcript-for="local:1"></div>`);
    expect(transcriptOf(null, "local:1")).toBeNull();
    expect(transcriptOf(root, null)).toBeNull();
    expect(transcriptOf(root, "local:1")).toBeNull();
    expect(transcriptOf(root, "local:missing")).toBeNull();
  });
});

describe("scrollTranscript", () => {
  it("moves one page in the pressed direction, keeping a line of context", () => {
    const element = document.createElement("div");
    Object.defineProperty(element, "clientHeight", { value: 100 });
    const scrollBy = vi.fn();
    element.scrollBy = scrollBy as unknown as HTMLElement["scrollBy"];

    expect(scrollTranscript(element, 1)).toBe(true);
    expect(scrollBy).toHaveBeenLastCalledWith({ behavior: "smooth", top: Math.round(100 * SCROLL_STEP_RATIO) });
    expect(scrollTranscript(element, -1)).toBe(true);
    expect(scrollBy).toHaveBeenLastCalledWith({ behavior: "smooth", top: -Math.round(100 * SCROLL_STEP_RATIO) });
  });

  it("falls back to the offset when the engine has no scrollBy", () => {
    const element = document.createElement("div");
    Object.defineProperty(element, "offsetHeight", { value: 200 });
    element.scrollTop = 500;
    // An engine that only has the property: `scrollBy` is absent, not a no-op.
    (element as unknown as { scrollBy?: unknown }).scrollBy = undefined;

    expect(scrollTranscript(element, -1)).toBe(true);
    expect(element.scrollTop).toBe(500 - Math.round(200 * SCROLL_STEP_RATIO));
  });

  it("does nothing without a transcript", () => {
    expect(scrollTranscript(null, 1)).toBe(false);
  });
});

describe("scrollToLiveEnd", () => {
  it("smooth-scrolls to the newest message", () => {
    const element = document.createElement("div");
    Object.defineProperty(element, "scrollHeight", { value: 1234 });
    const scrollTo = vi.fn();
    element.scrollTo = scrollTo as unknown as HTMLElement["scrollTo"];

    expect(scrollToLiveEnd(element)).toBe(true);
    expect(scrollTo).toHaveBeenCalledWith({ behavior: "smooth", top: 1234 });
  });

  it("falls back to the offset when the engine has no scrollTo", () => {
    const element = document.createElement("div");
    Object.defineProperty(element, "scrollHeight", { value: 900 });
    (element as unknown as { scrollTo?: unknown }).scrollTo = undefined;

    expect(scrollToLiveEnd(element)).toBe(true);
    expect(element.scrollTop).toBe(900);
  });

  it("does nothing without a transcript", () => {
    expect(scrollToLiveEnd(null)).toBe(false);
  });
});
