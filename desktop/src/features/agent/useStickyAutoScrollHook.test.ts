// @vitest-environment jsdom
import { act } from "react";
import { describe, expect, it, vi } from "vitest";
import { renderHook } from "../../test/renderHook";
import { useStickyAutoScroll } from "./useStickyAutoScroll";

describe("useStickyAutoScroll", () => {
  it("waits for temporarily hidden message content before following to the bottom", () => {
    const container = document.createElement("div");
    Object.defineProperty(container, "clientHeight", { configurable: true, value: 200 });
    Object.defineProperty(container, "scrollHeight", { configurable: true, value: 1_000 });
    const scrollRef = { current: container as HTMLElement | null };
    const settled = vi.fn();
    let followEnabled = false;
    let contentKey: unknown = "loading";
    const hook = renderHook(() => useStickyAutoScroll({
      scrollRef,
      contentKey,
      followEnabled,
      onContentSettled: settled,
    }));

    contentKey = "messages";
    hook.rerender();
    expect(container.scrollTop).toBe(0);
    expect(settled).not.toHaveBeenCalled();

    followEnabled = true;
    hook.rerender();
    expect(container.scrollTop).toBe(1_000);
    expect(settled).toHaveBeenCalledTimes(1);

    act(() => hook.current.scrollToLatest());
    expect(container.scrollTop).toBe(1_000);
    hook.unmount();
  });

  it("returns without throwing when there is no scroll container", () => {
    // boundary: `scrollToLatest` is wired to a "jump to latest" control, and the
    // ref can be empty when the control is rendered before the container (or after
    // it is torn down). It must no-op rather than dereference null.
    const scrollRef = { current: null as HTMLElement | null };
    const hook = renderHook(() => useStickyAutoScroll({ scrollRef, contentKey: 1 }));

    expect(() => act(() => hook.current.scrollToLatest())).not.toThrow();
    expect(hook.current.showJumpToLatest).toBe(false);
    hook.unmount();
  });

  it("ignores the echo of its own anchor correction instead of resuming auto-follow", () => {
    // concurrency: `settleViewport` corrects the scroll position to preserve the
    // reader's anchor, and the browser echoes that programmatic write back as a
    // scroll event. Re-deriving stickiness from the echo would read a position
    // inside the follow threshold and resume auto-follow, silently dragging a
    // reader who had scrolled up back to the bottom on the next layout change.
    const container = document.createElement("div");
    const row = document.createElement("div");
    row.dataset.messageId = "m1";
    container.append(row);
    document.body.append(container);
    let rowTop = 250;
    Object.defineProperty(container, "clientHeight", { configurable: true, value: 200 });
    Object.defineProperty(container, "scrollHeight", { configurable: true, value: 1_000 });
    vi.spyOn(container, "getBoundingClientRect").mockImplementation(
      () => ({ top: 0, bottom: 200 }) as DOMRect,
    );
    vi.spyOn(row, "getBoundingClientRect").mockImplementation(
      () => ({
        top: rowTop - container.scrollTop,
        bottom: rowTop + 100 - container.scrollTop,
      }) as DOMRect,
    );
    const scrollRef = { current: container as HTMLElement | null };
    let contentKey = 1;
    const hook = renderHook(() => useStickyAutoScroll({ scrollRef, contentKey }));

    // The reader is mid-history and stops following.
    container.scrollTop = 300;
    act(() => hook.current.preserveViewport());

    // Content above grows, so the hook corrects the position to keep the anchor —
    // and this correction happens to land inside the follow threshold of the bottom.
    contentKey = 2;
    rowTop = 750;
    hook.rerender();
    expect(container.scrollTop).toBe(800);

    // The browser echoes that correction back as a scroll event.
    act(() => hook.current.handleScroll());

    // A later layout change must not drag the reader to the bottom: with the echo
    // mistaken for a user scroll, stickiness flips back on and this rerender jumps
    // to `scrollHeight` (1_000) instead.
    contentKey = 3;
    hook.rerender();
    expect(container.scrollTop).toBe(800);
    hook.unmount();
    container.remove();
  });
});
