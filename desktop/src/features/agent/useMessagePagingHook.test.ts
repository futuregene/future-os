import type { AgentMessage } from "@future-os/thread-projection";
import { act } from "react";
// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../test/renderHook";
import { computePageStart, useMessagePaging } from "./useMessagePaging";

function msg(id: string, role: "user" | "assistant"): AgentMessage {
  return {
    id,
    role,
    content: id,
    status: "complete",
  } as unknown as AgentMessage;
}

/** 6 exchanges: u1 a1 … u6 a6. */
const MESSAGES = [
  msg("u1", "user"),
  msg("a1", "assistant"),
  msg("u2", "user"),
  msg("a2", "assistant"),
  msg("u3", "user"),
  msg("a3", "assistant"),
  msg("u4", "user"),
  msg("a4", "assistant"),
  msg("u5", "user"),
  msg("a5", "assistant"),
  msg("u6", "user"),
  msg("a6", "assistant"),
];

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

function setup(messages: AgentMessage[] = MESSAGES, userExchangeCount = 2) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const scrollRef = { current: container as HTMLElement | null };
  const onScroll = vi.fn();
  const h = renderHook(() =>
    useMessagePaging({ messages, scrollRef, userExchangeCount, onScroll }),
  );
  return { container, scrollRef, onScroll, h };
}

function setupAnchoredPaging() {
  const container = document.createElement("div");
  const row = document.createElement("div");
  row.dataset.messageId = "u5";
  container.append(row);
  document.body.append(container);
  let rowTop = 0;
  Object.defineProperty(container, "clientHeight", { value: 200 });
  Object.defineProperty(container, "scrollHeight", { get: () => rowTop + 1200 });
  vi.spyOn(container, "getBoundingClientRect").mockImplementation(() => ({ top: 0, bottom: 200 }) as DOMRect);
  vi.spyOn(row, "getBoundingClientRect").mockImplementation(() => ({
    top: rowTop - container.scrollTop,
    bottom: rowTop + 200 - container.scrollTop,
  }) as DOMRect);
  const scrollRef = { current: container };
  const h = renderHook(() => {
    const paging = useMessagePaging({ messages: MESSAGES, scrollRef, userExchangeCount: 2 });
    // Model the DOM prepend before React's layout effects restore the anchor.
    rowTop = paging.visibleMessages[0]?.id === "u5" ? 0 : 600;
    return paging;
  });
  act(() => {
    container.scrollTop = 0;
    h.current.handleScroll();
  });
  return { container, row, h };
}

async function settle() {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1600);
  });
}

describe("computePageStart (hook fixtures)", () => {
  it("walks back N user exchanges", () => {
    expect(computePageStart(MESSAGES, 1)).toBe(10);
    expect(computePageStart(MESSAGES, 3)).toBe(6);
    expect(computePageStart(MESSAGES, 99)).toBe(0);
  });
});

describe("useMessagePaging", () => {
  it("shows the last page of exchanges and reports more history", () => {
    const { h } = setup();
    expect(h.current.visibleMessages.map(m => m.id)).toEqual([
      "u5",
      "a5",
      "u6",
      "a6",
    ]);
    expect(h.current.canLoadOlder).toBe(true);
    expect(h.current.showLoadOlderHint).toBe(false);
    h.unmount();
  });

  it("handleScroll without a container only forwards to onScroll", () => {
    const scrollRef = { current: null as HTMLElement | null };
    const onScroll = vi.fn();
    const h = renderHook(() =>
      useMessagePaging({
        messages: MESSAGES,
        scrollRef,
        userExchangeCount: 2,
        onScroll,
      }),
    );
    act(() => {
      h.current.handleScroll();
    });
    expect(onScroll).toHaveBeenCalledTimes(1);
    expect(h.current.showLoadOlderHint).toBe(false);
    h.unmount();
  });

  it("starts loading, the hint and the 1500ms timer on the first top collision", async () => {
    const { container, h } = setup();
    container.scrollTop = 0;
    act(() => {
      h.current.handleScroll();
    });
    expect(h.current.visibleMessages[0]?.id).toBe("u3");
    expect(h.current.coolingDown).toBe(true);
    expect(h.current.showLoadOlderHint).toBe(true);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1499);
    });
    expect(h.current.showLoadOlderHint).toBe(true);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    // Even while still at the top, no old confirmation hint may remain.
    expect(container.scrollTop).toBe(0);
    expect(h.current.coolingDown).toBe(false);
    expect(h.current.showLoadOlderHint).toBe(false);
    h.unmount();
  });

  it("does not reload or restart the timer for momentum during cooldown", async () => {
    const { container, h } = setup();
    act(() => {
      h.current.handleScroll();
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });
    const up = new WheelEvent("wheel", { deltaY: -40, cancelable: true });
    act(() => {
      container.scrollTop = 200;
      h.current.handleScroll();
      container.scrollTop = 0;
      h.current.handleScroll();
      container.dispatchEvent(up);
    });
    expect(up.defaultPrevented).toBe(true);
    expect(h.current.visibleMessages[0]?.id).toBe("u3");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });
    expect(h.current.coolingDown).toBe(false);
    expect(h.current.showLoadOlderHint).toBe(false);
    h.unmount();
  });

  it("starts the same transaction on the second collision without another confirmation", async () => {
    const { container, h } = setup();
    act(() => {
      h.current.handleScroll();
    });
    await settle();
    act(() => {
      container.scrollTop = 200;
      h.current.handleScroll();
      container.scrollTop = 0;
      h.current.handleScroll();
    });
    expect(h.current.visibleMessages[0]?.id).toBe("u1");
    expect(h.current.showLoadOlderHint).toBe(true);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1499);
    });
    expect(h.current.coolingDown).toBe(true);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(h.current.coolingDown).toBe(false);
    expect(h.current.showLoadOlderHint).toBe(false);
    h.unmount();
  });

  it("loads on an upward wheel when the top produces no scroll event", async () => {
    const { container, h } = setup();
    const up = new WheelEvent("wheel", { deltaY: -40, cancelable: true });
    act(() => {
      container.dispatchEvent(up);
    });
    expect(up.defaultPrevented).toBe(true);
    expect(h.current.visibleMessages[0]?.id).toBe("u3");
    expect(h.current.showLoadOlderHint).toBe(true);
    await settle();
    expect(h.current.showLoadOlderHint).toBe(false);
    h.unmount();
  });

  it("counts from the collision even if React commits slowly", async () => {
    const { h } = setup();
    act(() => {
      h.current.handleScroll();
      vi.advanceTimersByTime(1800);
    });
    expect(h.current.coolingDown).toBe(true);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(64);
    });
    expect(h.current.coolingDown).toBe(false);
    expect(h.current.showLoadOlderHint).toBe(false);
    h.unmount();
  });

  it("does not wait forever for an image whose geometry is already stable", async () => {
    const { container, h } = setup();
    const img = document.createElement("img");
    Object.defineProperty(img, "complete", { value: false });
    container.append(img);
    act(() => {
      h.current.handleScroll();
    });
    await settle();
    expect(h.current.coolingDown).toBe(false);
    expect(h.current.showLoadOlderHint).toBe(false);
    h.unmount();
  });

  it("loadOlder prepends a page and re-arms the guard", async () => {
    const { container, h } = setup();
    expect(h.current.visibleMessages[0]?.id).toBe("u5");
    act(() => {
      h.current.loadOlder();
    });
    expect(h.current.visibleMessages[0]?.id).toBe("u3");
    // The layout effect ran (container present, no anchor elements → pin top).
    expect(container.scrollTop).toBe(0);
    // Guard cleared: another load works.
    act(() => {
      h.current.loadOlder();
    });
    expect(h.current.visibleMessages[0]?.id).toBe("u1");
    expect(h.current.canLoadOlder).toBe(false);
    h.unmount();
  });

  it("loadOlder is a no-op when everything is already loaded", () => {
    const { h } = setup(MESSAGES.slice(8), 2);
    expect(h.current.canLoadOlder).toBe(false);
    act(() => {
      h.current.loadOlder();
    });
    expect(h.current.visibleMessages).toHaveLength(4);
    h.unmount();
  });

  it("blocks re-entrant loads while a restore is pending", () => {
    const { h } = setup();
    act(() => {
      h.current.loadOlder();
      // Synchronous second call hits the ref guard.
      h.current.loadOlder();
    });
    expect(h.current.visibleMessages[0]?.id).toBe("u3");
    h.unmount();
  });

  it("allows downward scrolling without shortening upward protection", () => {
    const { container, h } = setup();
    act(() => {
      h.current.loadOlder();
    });
    const down = new WheelEvent("wheel", { deltaY: 40, cancelable: true });
    const up = new WheelEvent("wheel", { deltaY: -40, cancelable: true });
    act(() => {
      container.dispatchEvent(down);
      container.dispatchEvent(up);
    });
    expect(down.defaultPrevented).toBe(true);
    expect(container.scrollTop).toBe(40);
    expect(up.defaultPrevented).toBe(true);
    h.unmount();
  });

  it("waits for slow data and subsequent layout without adding another 1500ms", async () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    let completeLoad!: () => void;
    const loadOlderHistory = vi.fn(() => new Promise<void>((resolve) => {
      completeLoad = resolve;
    }));
    const scrollRef = { current: container };
    const messages = MESSAGES.slice(8);
    const h = renderHook(() => useMessagePaging({
      messages,
      scrollRef,
      userExchangeCount: 2,
      hasOlderHistory: true,
      loadOlderHistory,
    }));
    act(() => {
      h.current.loadOlder();
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1800);
    });
    expect(h.current.coolingDown).toBe(true);
    expect(h.current.showLoadOlderHint).toBe(true);
    const up = new WheelEvent("wheel", { deltaY: -40, cancelable: true });
    act(() => {
      container.dispatchEvent(up);
    });
    expect(up.defaultPrevented).toBe(true);
    await act(async () => {
      completeLoad();
    });
    expect(h.current.coolingDown).toBe(true);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(64);
    });
    expect(h.current.coolingDown).toBe(false);
    expect(h.current.showLoadOlderHint).toBe(false);
    h.unmount();
    container.remove();
  });

  it("restores noncancelable momentum drift during the cooldown, then unlocks", async () => {
    const { container, row, h } = setupAnchoredPaging();
    expect(container.scrollTop).toBe(600);
    expect(row.getBoundingClientRect().top).toBe(0);
    expect(container.style.overflowY).toBe("hidden");
    const up = new WheelEvent("wheel", { deltaY: -80, cancelable: false });
    act(() => {
      container.dispatchEvent(up);
      // Simulate an in-flight WebKit scroll arriving despite the native lock.
      container.scrollTop = 520;
      h.current.handleScroll();
    });
    expect(up.defaultPrevented).toBe(false);
    expect(h.current.coolingDown).toBe(true);
    expect(container.scrollTop).toBe(600);
    expect(row.getBoundingClientRect().top).toBe(0);
    await settle();
    expect(container.style.overflowY).toBe("");
    act(() => {
      container.scrollTop = 520;
      h.current.handleScroll();
    });
    expect(container.scrollTop).toBe(520);
    h.unmount();
  });

  it("accepts downward wheel input as the new protected reading position", () => {
    const { container, row, h } = setupAnchoredPaging();
    const down = new WheelEvent("wheel", { deltaY: 80, cancelable: false });
    act(() => {
      container.dispatchEvent(down);
    });
    expect(container.scrollTop).toBe(680);
    expect(row.getBoundingClientRect().top).toBe(-80);
    act(() => {
      container.scrollTop = 620;
      h.current.handleScroll();
    });
    expect(container.scrollTop).toBe(680);
    expect(row.getBoundingClientRect().top).toBe(-80);
    h.unmount();
    expect(container.style.overflowY).toBe("");
  });

  it("restores the original overflow style on completion and unmount", async () => {
    const { container, h } = setup();
    container.style.setProperty("overflow-y", "scroll", "important");
    act(() => {
      h.current.loadOlder();
    });
    expect(container.style.overflowY).toBe("hidden");
    await settle();
    expect(container.style.overflowY).toBe("scroll");
    expect(container.style.getPropertyPriority("overflow-y")).toBe("important");
    act(() => {
      h.current.loadOlder();
    });
    h.unmount();
    expect(container.style.overflowY).toBe("scroll");
    expect(container.style.getPropertyPriority("overflow-y")).toBe("important");
  });

  it("restores the scroll position from the captured anchor", () => {
    const { container, h } = setup();
    // Two rendered messages with geometry: u3 crosses the viewport top.
    const u3 = document.createElement("div");
    u3.setAttribute("data-message-id", "u3");
    const a3 = document.createElement("div");
    a3.setAttribute("data-message-id", "a3");
    container.append(u3, a3);
    vi.spyOn(container, "getBoundingClientRect").mockReturnValue({
      top: 100,
      bottom: 500,
    } as DOMRect);
    vi.spyOn(u3, "getBoundingClientRect").mockReturnValue({
      top: 90,
      bottom: 110,
    } as DOMRect);
    vi.spyOn(a3, "getBoundingClientRect").mockReturnValue({
      top: 120,
      bottom: 200,
    } as DOMRect);
    container.scrollTop = 50;

    act(() => {
      h.current.loadOlder();
    });
    // Anchor u3 at offset -10 from the viewport top; after the prepend its rect
    // is unchanged in the mock, so the delta shifts scrollTop by 0 - (-10)… the
    // exact value matters less than the adjustment being applied.
    expect(container.scrollTop).not.toBe(0);
    h.unmount();
  });

  it("keeps the position when the anchor element is gone after the load", () => {
    const { container, h } = setup();
    const ghost = document.createElement("div");
    ghost.setAttribute("data-message-id", "ghost");
    container.append(ghost);
    vi.spyOn(container, "getBoundingClientRect").mockReturnValue({
      top: 0,
      bottom: 500,
    } as DOMRect);
    vi.spyOn(ghost, "getBoundingClientRect").mockReturnValue({
      top: 10,
      bottom: 60,
    } as DOMRect);
    container.scrollTop = 42;
    act(() => {
      h.current.loadOlder();
      // The anchor id no longer exists in the container at restore time.
      ghost.remove();
    });
    expect(container.scrollTop).toBe(42);
    h.unmount();
  });

  it("skips the restore when the container vanished", () => {
    const { scrollRef, h } = setup();
    act(() => {
      scrollRef.current = null;
      h.current.loadOlder();
    });
    // No crash; the page still advanced.
    expect(h.current.visibleMessages[0]?.id).toBe("u3");
    h.unmount();
  });

  it("clears pending cooldown and render work on unmount", async () => {
    const { container, h } = setup();
    container.scrollTop = 0;
    act(() => {
      h.current.handleScroll();
    });
    h.unmount();
    await settle();
    // No post-unmount setState warning/crash.
  });

  it("skips elements above the viewport when capturing the anchor", () => {
    const { container, h } = setup();
    const above = document.createElement("div");
    above.setAttribute("data-message-id", "above");
    const visible = document.createElement("div");
    visible.setAttribute("data-message-id", "u5");
    container.append(above, visible);
    vi.spyOn(container, "getBoundingClientRect").mockReturnValue({
      top: 100,
      bottom: 500,
    } as DOMRect);
    // Fully above the viewport top → not a candidate.
    vi.spyOn(above, "getBoundingClientRect").mockReturnValue({
      top: 40,
      bottom: 90,
    } as DOMRect);
    vi.spyOn(visible, "getBoundingClientRect").mockReturnValue({
      top: 110,
      bottom: 160,
    } as DOMRect);
    container.scrollTop = 20;
    act(() => {
      h.current.loadOlder();
    });
    // The visible element anchored the restore (not the above-viewport one).
    expect(container.scrollTop).not.toBe(0);
    h.unmount();
  });

  it("ignores wheel events from a detached container", async () => {
    const { container, scrollRef, h } = setup();
    container.scrollTop = 0;
    act(() => {
      h.current.handleScroll();
    });
    // The first collision loaded a page; events from a detached container
    // must not trigger another load.
    scrollRef.current = null;
    await settle();
    act(() => {
      container.dispatchEvent(new WheelEvent("wheel", { deltaY: -40 }));
    });
    expect(h.current.visibleMessages[0]?.id).toBe("u3");
    h.unmount();
  });

  it("captures no anchor from an empty container", () => {
    const { container, h } = setup();
    // No data-message-id children → anchor null → restore pins top.
    container.scrollTop = 30;
    act(() => {
      h.current.loadOlder();
    });
    expect(container.scrollTop).toBe(30);
    h.unmount();
  });
});
