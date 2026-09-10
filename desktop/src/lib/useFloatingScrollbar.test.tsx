// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useFloatingScrollbar } from "./useFloatingScrollbar";

const observed: Element[] = [];

class ResizeObserverStub {
  observe(element: Element) {
    observed.push(element);
  }

  disconnect() {}
}

afterEach(() => {
  observed.length = 0;
  vi.unstubAllGlobals();
});

describe("floating scrollbar", () => {
  it("observes the explicit content wrapper instead of an arbitrary first child", () => {
    vi.stubGlobal("ResizeObserver", ResizeObserverStub);
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    function Probe() {
      const scrollbar = useFloatingScrollbar();
      return (
        <div ref={scrollbar.scrollRef} data-testid="viewport">
          <div data-testid="unrelated" />
          <div ref={scrollbar.contentRef} data-testid="content" />
        </div>
      );
    }

    act(() => root.render(<Probe />));
    expect(observed).toContain(container.querySelector("[data-testid=\"viewport\"]"));
    expect(observed).toContain(container.querySelector("[data-testid=\"content\"]"));
    expect(observed).not.toContain(container.querySelector("[data-testid=\"unrelated\"]"));

    act(() => root.unmount());
    container.remove();
  });
});
