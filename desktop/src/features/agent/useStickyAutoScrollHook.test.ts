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
});
