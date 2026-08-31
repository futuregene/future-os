// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { ThreadSearch } from "./ThreadSearch";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

describe("thread search", () => {
  it("keeps the previous count visible until the next query finishes", () => {
    Object.defineProperty(Element.prototype, "scrollIntoView", {
      configurable: true,
      value: vi.fn(),
    });
    let nextFrame = 0;
    const frames = new Map<number, FrameRequestCallback>();
    vi.stubGlobal("requestAnimationFrame", vi.fn((callback: FrameRequestCallback) => {
      nextFrame += 1;
      frames.set(nextFrame, callback);
      return nextFrame;
    }));
    vi.stubGlobal("cancelAnimationFrame", vi.fn((frame: number) => frames.delete(frame)));
    vi.stubGlobal("CSS", {
      highlights: {
        delete: vi.fn(),
        get: vi.fn(),
        set: vi.fn(),
      },
    });
    vi.stubGlobal("Highlight", class {
      constructor(..._ranges: Range[]) {}
    });

    const container = document.createElement("div");
    const thread = document.createElement("div");
    thread.textContent = "alpha beta";
    document.body.append(container, thread);
    const root = createRoot(container);
    act(() => {
      root.render(
        <ThreadSearch
          canLoadOlder={false}
          contentKey={null}
          onLoadOlder={vi.fn()}
          rootRef={{ current: thread }}
        />,
      );
    });
    act(() => window.dispatchEvent(new KeyboardEvent("keydown", {
      cancelable: true,
      ctrlKey: true,
      key: "f",
    })));

    const input = container.querySelector("input")!;
    const setNativeValue = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!;
    const flushFrame = () => {
      const callbacks = [...frames.values()];
      frames.clear();
      act(() => callbacks.forEach(callback => callback(0)));
    };
    act(() => {
      setNativeValue.call(input, "a");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    flushFrame();
    flushFrame();
    expect(container.textContent).toContain("1 / 3");

    act(() => {
      setNativeValue.call(input, "al");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });

    expect(container.textContent).toContain("1 / 3");
    expect([...container.querySelectorAll("button")].slice(0, 2).every(button => !button.disabled)).toBe(true);

    flushFrame();
    flushFrame();
    expect(container.textContent).toContain("1 / 1");

    act(() => root.unmount());
    container.remove();
    thread.remove();
    delete (Element.prototype as Partial<Element>).scrollIntoView;
    vi.unstubAllGlobals();
  });

  it("clears stale highlights synchronously when the query changes", () => {
    const clearHighlight = vi.fn();
    const deleteHighlight = vi.fn();
    vi.stubGlobal("CSS", {
      highlights: {
        delete: deleteHighlight,
        get: vi.fn(() => ({ clear: clearHighlight })),
        set: vi.fn(),
      },
    });
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    act(() => {
      root.render(
        <ThreadSearch
          canLoadOlder={false}
          contentKey={null}
          onLoadOlder={vi.fn()}
          rootRef={{ current: null }}
        />,
      );
    });
    act(() => window.dispatchEvent(new KeyboardEvent("keydown", {
      cancelable: true,
      ctrlKey: true,
      key: "f",
    })));

    const input = container.querySelector("input")!;
    const setNativeValue = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!;
    act(() => {
      setNativeValue.call(input, "s");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    clearHighlight.mockClear();
    deleteHighlight.mockClear();
    act(() => {
      setNativeValue.call(input, "");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });

    expect(deleteHighlight.mock.calls).toEqual([
      ["thread-search-match"],
      ["thread-search-current"],
    ]);
    expect(clearHighlight).toHaveBeenCalledTimes(2);
    act(() => root.unmount());
    container.remove();
    vi.unstubAllGlobals();
  });

  it.each([
    { ctrlKey: true, metaKey: false },
    { ctrlKey: false, metaKey: true },
  ])("opens the current thread search for the platform find shortcut", (modifiers) => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    const downstreamKeydown = vi.fn();
    document.addEventListener("keydown", downstreamKeydown);
    act(() => {
      root.render(
        <ThreadSearch
          canLoadOlder={false}
          contentKey={null}
          onLoadOlder={vi.fn()}
          rootRef={{ current: null }}
        />,
      );
    });

    const event = new KeyboardEvent("keydown", {
      ...modifiers,
      bubbles: true,
      cancelable: true,
      key: "f",
    });
    act(() => document.body.dispatchEvent(event));

    expect(event.defaultPrevented).toBe(true);
    expect(downstreamKeydown).not.toHaveBeenCalled();
    const input = container.querySelector("input");
    expect(input).not.toBeNull();
    expect(input?.getAttribute("spellcheck")).toBe("false");
    expect(input?.getAttribute("autocorrect")).toBe("off");
    expect(input?.getAttribute("autocapitalize")).toBe("none");
    expect(input?.getAttribute("autocomplete")).toBe("off");

    const close = container.querySelector("button:last-of-type")!;
    act(() => close.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(container.querySelector("input")).toBeNull();
    expect(container.querySelector("style")?.textContent).toContain("thread-search-match");
    act(() => root.unmount());
    document.removeEventListener("keydown", downstreamKeydown);
    container.remove();
  });
});
