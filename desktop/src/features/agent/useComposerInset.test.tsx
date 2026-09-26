// @vitest-environment jsdom
import { act, useRef } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../test/renderHook";
import { useComposerInset } from "./useComposerInset";
import { useStickyAutoScroll } from "./useStickyAutoScroll";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

function setup() {
  const host = document.createElement("div");
  document.body.append(host);
  let footerHeight = 180;
  let scrollTop = 0;
  const observers = new Map<Element, Set<() => void>>();
  const disconnect = vi.fn();
  vi.stubGlobal("ResizeObserver", class {
    targets = new Set<Element>();
    constructor(private callback: () => void) {}
    observe(target: Element) {
      this.targets.add(target);
      const callbacks = observers.get(target) ?? new Set();
      callbacks.add(this.callback);
      observers.set(target, callbacks);
    }

    disconnect() {
      disconnect();
      for (const target of this.targets)
        observers.get(target)?.delete(this.callback);
    }
  });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
    function (this: HTMLElement) {
      if (this.dataset.footer !== undefined)
        return { height: footerHeight } as DOMRect;
      if (this.dataset.messageId)
        return { top: -scrollTop, bottom: 1200 - scrollTop } as DOMRect;
      return { top: 0, bottom: 600, height: 600 } as DOMRect;
    },
  );
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(600);
  vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockImplementation(
    function (this: HTMLElement) {
      const spacer = this.querySelector<HTMLElement>("[data-spacer]");
      return 1200 + Number.parseFloat(spacer?.style.height ?? "0");
    },
  );
  vi.spyOn(HTMLElement.prototype, "scrollTop", "get").mockImplementation(() => scrollTop);
  vi.spyOn(HTMLElement.prototype, "scrollTop", "set").mockImplementation(
    function (this: HTMLElement, value: number) {
      scrollTop = Math.max(0, Math.min(value, this.scrollHeight - this.clientHeight));
    },
  );
  let current!: ReturnType<typeof useStickyAutoScroll>;
  function Harness() {
    const scrollRef = useRef<HTMLDivElement>(null);
    const { composerRef, composerHeight } = useComposerInset();
    current = useStickyAutoScroll({ scrollRef, contentKey: "history" });
    return (
      <>
        <div ref={scrollRef} data-scroll onScroll={current.handleScroll}>
          <div data-content>
            <div data-message-id="last">Historical response</div>
            <div data-spacer aria-hidden="true" style={{ height: composerHeight }} />
          </div>
        </div>
        <div ref={composerRef} data-footer />
      </>
    );
  }
  const root = createRoot(host);
  act(() => root.render(<Harness />));
  const content = host.querySelector<HTMLElement>("[data-content]")!;
  const footer = host.querySelector<HTMLElement>("[data-footer]")!;
  const scroll = host.querySelector<HTMLElement>("[data-scroll]")!;
  const spacer = host.querySelector<HTMLElement>("[data-spacer]")!;
  const notify = (target: Element) => act(() => {
    for (const callback of observers.get(target) ?? []) callback();
  });
  // Browsers notify the existing content observer after the spacer changes size.
  notify(content);
  return {
    scroll,
    spacer,
    disconnect,
    resize: (height: number) => {
      footerHeight = height;
      notify(footer);
      notify(content);
    },
    readHistory: (top: number) => act(() => {
      scroll.scrollTop = top;
      current.handleScroll();
    }),
    jumpToLatest: () => act(() => current.scrollToLatest()),
    dispose: () => {
      act(() => root.unmount());
      host.remove();
    },
  };
}

describe("floating composer inset", () => {
  it("reserves the measured footer on mount and follows long-paste growth and clearing", () => {
    const h = setup();
    expect(h.spacer.style.height).toBe("180px");
    expect(h.scroll.scrollTop).toBe(780);
    h.resize(420);
    expect(h.spacer.style.height).toBe("420px");
    expect(h.scroll.scrollTop).toBe(1020);
    // The final message ends above the footer, not behind it.
    expect(1200 - h.scroll.scrollTop).toBe(600 - 420);
    h.resize(180);
    expect(h.spacer.style.height).toBe("180px");
    expect(h.scroll.scrollTop).toBe(780);
    h.dispose();
  });

  it("preserves the history reading position while making the entire tail reachable", () => {
    const h = setup();
    h.readHistory(200);
    h.resize(480);
    expect(h.scroll.scrollTop).toBe(200);
    h.jumpToLatest();
    expect(h.scroll.scrollTop).toBe(1080);
    expect(1200 - h.scroll.scrollTop).toBe(600 - 480);
    h.dispose();
  });

  it("tracks footer changes beyond the editor and disconnects on unmount", () => {
    const h = setup();
    h.resize(550); // Approval/connection notice or attachment preview appears.
    expect(h.spacer.style.height).toBe("550px");
    h.resize(260); // Notice disappears or the window becomes wider.
    expect(h.spacer.style.height).toBe("260px");
    h.dispose();
    expect(h.disconnect).toHaveBeenCalled();
    h.resize(400);
    expect(h.spacer.style.height).toBe("260px");
  });

  it("stays inert when the composer ref was never attached to an element", () => {
    // boundary: the hook returns the ref for the caller to attach, so a caller
    // that renders before the element (or drops it) runs the layout effect with a
    // null ref. It must no-op rather than observe null, and must not construct a
    // ResizeObserver for nothing.
    const constructed: unknown[] = [];
    vi.stubGlobal("ResizeObserver", class {
      constructor() {
        constructed.push(this);
      }

      observe() {}
      disconnect() {}
    });
    const hook = renderHook(() => useComposerInset());

    expect(hook.current.composerRef.current).toBeNull();
    expect(hook.current.composerHeight).toBe(0);
    expect(constructed).toHaveLength(0);

    hook.unmount();
  });
});
