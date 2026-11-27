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

  it("follows the newest message again when the conversation changes under one container", () => {
    // isolation: a caller can swap the transcript without remounting the view
    // (the remote view is drawn for whichever conversation is open, and some of
    // its frames reuse the instance). Stickiness and the reading anchor are refs,
    // so without `resetKey` the previous conversation's anchor outlives the
    // messages it names, `settleViewport` finds no target and keeps the current
    // position — which is how a long conversation opens on a historical message.
    const container = document.createElement("div");
    document.body.append(container);
    const first = document.createElement("div");
    first.dataset.messageId = "first";
    container.append(first);
    Object.defineProperty(container, "clientHeight", { configurable: true, value: 200 });
    Object.defineProperty(container, "scrollHeight", { configurable: true, value: 1_000 });
    const scrollRef = { current: container as HTMLElement | null };
    let resetKey = "desktop_a::sess_1";
    let contentKey: unknown = "first";
    const hook = renderHook(() => useStickyAutoScroll({ scrollRef, contentKey, resetKey }));

    // The reader scrolled up in the first conversation to read history.
    container.scrollTop = 300;
    act(() => hook.current.preserveViewport());

    // The same container now draws another conversation, whose rows carry other
    // ids than the one the reader was anchored to.
    first.remove();
    const second = document.createElement("div");
    second.dataset.messageId = "second";
    container.append(second);
    resetKey = "desktop_a::sess_2";
    contentKey = "second";
    hook.rerender();

    expect(container.scrollTop).toBe(1_000);

    // The same key, though, is the same conversation: a reader who scrolled up
    // there is not dragged to the bottom by an ordinary content change.
    container.scrollTop = 300;
    act(() => hook.current.preserveViewport());
    contentKey = "second-more";
    hook.rerender();
    expect(container.scrollTop).toBe(300);

    hook.unmount();
    container.remove();
  });

  /**
   * A transcript whose two messages end at 700px and 1_700px of a 2_000px
   * document: the last 300px is the spacer the real transcript reserves for the
   * composer, inside an 800px viewport. Geometry is in content coordinates, so
   * every row reports itself at `top - scrollTop`, as the browser would.
   */
  function mountTranscript() {
    const container = document.createElement("div");
    document.body.append(container);
    const first = document.createElement("div");
    first.dataset.messageId = "m1";
    const newest = document.createElement("div");
    newest.dataset.messageId = "m2";
    container.append(first, newest);
    Object.defineProperty(container, "clientHeight", { configurable: true, value: 800 });
    Object.defineProperty(container, "scrollHeight", { configurable: true, value: 2_000 });
    vi.spyOn(container, "getBoundingClientRect").mockImplementation(
      () => ({ top: 0, bottom: 800 }) as DOMRect,
    );
    const rectOf = (contentTop: number, height: number) => () =>
      ({
        top: contentTop - container.scrollTop,
        bottom: contentTop + height - container.scrollTop,
      }) as DOMRect;
    vi.spyOn(first, "getBoundingClientRect").mockImplementation(rectOf(0, 700));
    vi.spyOn(newest, "getBoundingClientRect").mockImplementation(rectOf(700, 1_000));
    const scrollRef = { current: container as HTMLElement | null };
    let contentKey: unknown = 1;
    const hook = renderHook(() => useStickyAutoScroll({ scrollRef, contentKey }));
    return {
      container,
      hook,
      touchContent: () => {
        contentKey = `k${Math.random()}`;
        hook.rerender();
      },
      dispose: () => {
        hook.unmount();
        container.remove();
      },
    };
  }

  it("keeps the jump button away while the newest message is already in view", () => {
    // regression: the transcript ends in a spacer as tall as the composer, plus the
    // pane's bottom padding, and no scroll closes that tail. Measured to the
    // content's end, a reader parked on the newest message reads 250px from the
    // bottom — past the 240px threshold — and is offered a jump to the message
    // filling the screen. Measured to the newest message, that distance is negative
    // there, and the button stays away. Reverting to the content-end distance turns
    // this test red on the first expectation (250px is what the button used to read).
    const { container, hook, touchContent, dispose } = mountTranscript();

    // The newest message's end sits 50px above the viewport's bottom edge, while the
    // document's end sits 250px below it.
    container.scrollTop = 950;
    act(() => hook.current.handleScroll());
    expect(container.scrollHeight - container.clientHeight - container.scrollTop).toBe(250);
    expect(hook.current.showJumpToLatest).toBe(false);

    // A later layout change settles through the same measure and adds nothing.
    touchContent();
    expect(container.scrollTop).toBe(950);
    expect(hook.current.showJumpToLatest).toBe(false);

    dispose();
  });

  it("offers the jump button once the newest message itself is out of view", () => {
    // The other side of the same measure: any fix that simply never shows the
    // button would pass the test above and still leave history hard to get back to.
    const { container, hook, dispose } = mountTranscript();

    // The newest message's end is 300px below the viewport's bottom edge.
    container.scrollTop = 600;
    act(() => hook.current.handleScroll());
    expect(hook.current.showJumpToLatest).toBe(true);

    // Coming back up to the newest message takes it away again.
    act(() => hook.current.scrollToLatest());
    expect(hook.current.showJumpToLatest).toBe(false);

    dispose();
  });
});
