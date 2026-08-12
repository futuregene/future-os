import type { AgentMessage } from "./agentThreadTypes";
import { act } from "react";
// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../test/renderHook";
import { computePageStart, useMessagePaging } from "./useMessagePaging";

function msg(id: string, role: "user" | "assistant"): AgentMessage {
  return { id, role, content: id, status: "complete" } as unknown as AgentMessage;
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
  const h = renderHook(() => useMessagePaging({ messages, scrollRef, userExchangeCount, onScroll }));
  return { container, scrollRef, onScroll, h };
}

async function settle() {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(400);
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
    expect(h.current.visibleMessages.map(m => m.id)).toEqual(["u5", "a5", "u6", "a6"]);
    expect(h.current.canLoadOlder).toBe(true);
    expect(h.current.showLoadOlderHint).toBe(false);
    h.unmount();
  });

  it("handleScroll without a container only forwards to onScroll", () => {
    const scrollRef = { current: null as HTMLElement | null };
    const onScroll = vi.fn();
    const h = renderHook(() => useMessagePaging({ messages: MESSAGES, scrollRef, userExchangeCount: 2, onScroll }));
    act(() => {
      h.current.handleScroll();
    });
    expect(onScroll).toHaveBeenCalledTimes(1);
    expect(h.current.showLoadOlderHint).toBe(false);
    h.unmount();
  });

  it("settles the load hint after resting at the top", async () => {
    const { container, h } = setup();
    container.scrollTop = 0;
    act(() => {
      h.current.handleScroll();
    });
    expect(h.current.showLoadOlderHint).toBe(false);
    // A second scroll while the settle timer is pending is a no-op.
    act(() => {
      h.current.handleScroll();
    });
    await settle();
    expect(h.current.showLoadOlderHint).toBe(true);
    h.unmount();
  });

  it("cancels the settle when scrolling away from the top", async () => {
    const { container, h } = setup();
    container.scrollTop = 0;
    act(() => {
      h.current.handleScroll();
    });
    container.scrollTop = 100;
    act(() => {
      h.current.handleScroll();
    });
    await settle();
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

  it("clears a pending settle timer when a load starts", async () => {
    const { container, h } = setup();
    container.scrollTop = 0;
    act(() => {
      h.current.handleScroll();
    });
    act(() => {
      h.current.loadOlder();
    });
    // The pre-load settle must not fire afterwards.
    await settle();
    expect(h.current.showLoadOlderHint).toBe(false);
    h.unmount();
  });

  it("loads a page via the wheel gesture once settled, with a cooldown", async () => {
    const { container, h } = setup();
    container.scrollTop = 0;
    act(() => {
      h.current.handleScroll();
    });
    await settle();
    expect(h.current.showLoadOlderHint).toBe(true);

    // A downward wheel does nothing.
    act(() => {
      container.dispatchEvent(new WheelEvent("wheel", { deltaY: 40 }));
    });
    expect(h.current.visibleMessages[0]?.id).toBe("u5");

    // An upward wheel loads a page…
    act(() => {
      container.dispatchEvent(new WheelEvent("wheel", { deltaY: -40 }));
    });
    expect(h.current.visibleMessages[0]?.id).toBe("u3");

    // …but the restore dropped the hint; re-settle, then the cooldown swallows
    // the trailing wheel of the same gesture.
    container.scrollTop = 0;
    act(() => {
      h.current.handleScroll();
    });
    await settle();
    act(() => {
      container.dispatchEvent(new WheelEvent("wheel", { deltaY: -40 }));
      container.dispatchEvent(new WheelEvent("wheel", { deltaY: -40 }));
    });
    expect(h.current.visibleMessages[0]?.id).toBe("u1");
    h.unmount();
  });

  it("restores the scroll position from the captured anchor", () => {
    const { container, h } = setup();
    // Two rendered messages with geometry: u3 crosses the viewport top.
    const u3 = document.createElement("div");
    u3.setAttribute("data-message-id", "u3");
    const a3 = document.createElement("div");
    a3.setAttribute("data-message-id", "a3");
    container.append(u3, a3);
    vi.spyOn(container, "getBoundingClientRect").mockReturnValue({ top: 100, bottom: 500 } as DOMRect);
    vi.spyOn(u3, "getBoundingClientRect").mockReturnValue({ top: 90, bottom: 110 } as DOMRect);
    vi.spyOn(a3, "getBoundingClientRect").mockReturnValue({ top: 120, bottom: 200 } as DOMRect);
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

  it("pins the top when the anchor element is gone after the load", () => {
    const { container, h } = setup();
    const ghost = document.createElement("div");
    ghost.setAttribute("data-message-id", "ghost");
    container.append(ghost);
    vi.spyOn(container, "getBoundingClientRect").mockReturnValue({ top: 0, bottom: 500 } as DOMRect);
    vi.spyOn(ghost, "getBoundingClientRect").mockReturnValue({ top: 10, bottom: 60 } as DOMRect);
    container.scrollTop = 42;
    act(() => {
      h.current.loadOlder();
      // The anchor id no longer exists in the container at restore time.
      ghost.remove();
    });
    expect(container.scrollTop).toBe(0);
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

  it("clears a pending settle timer on unmount", async () => {
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
    vi.spyOn(container, "getBoundingClientRect").mockReturnValue({ top: 100, bottom: 500 } as DOMRect);
    // Fully above the viewport top → not a candidate.
    vi.spyOn(above, "getBoundingClientRect").mockReturnValue({ top: 40, bottom: 90 } as DOMRect);
    vi.spyOn(visible, "getBoundingClientRect").mockReturnValue({ top: 110, bottom: 160 } as DOMRect);
    container.scrollTop = 20;
    act(() => {
      h.current.loadOlder();
    });
    // The visible element anchored the restore (not the above-viewport one).
    expect(container.scrollTop).not.toBe(0);
    h.unmount();
  });

  it("does not attach the wheel listener when the container is gone", async () => {
    const { container, scrollRef, h } = setup();
    container.scrollTop = 0;
    act(() => {
      h.current.handleScroll();
    });
    // Container vanishes before the settle completes: the wheel effect runs
    // with no container and attaches nothing.
    scrollRef.current = null;
    await settle();
    act(() => {
      container.dispatchEvent(new WheelEvent("wheel", { deltaY: -40 }));
    });
    expect(h.current.visibleMessages[0]?.id).toBe("u5");
    h.unmount();
  });

  it("captures no anchor from an empty container", () => {
    const { container, h } = setup();
    // No data-message-id children → anchor null → restore pins top.
    container.scrollTop = 30;
    act(() => {
      h.current.loadOlder();
    });
    expect(container.scrollTop).toBe(0);
    h.unmount();
  });
});
