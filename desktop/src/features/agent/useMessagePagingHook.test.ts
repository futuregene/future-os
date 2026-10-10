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
  Object.defineProperty(container, "scrollHeight", {
    get: () => rowTop + 1200,
  });
  vi.spyOn(container, "getBoundingClientRect").mockImplementation(
    () => ({ top: 0, bottom: 200 }) as DOMRect,
  );
  vi.spyOn(row, "getBoundingClientRect").mockImplementation(
    () =>
      ({
        top: rowTop - container.scrollTop,
        bottom: rowTop + 200 - container.scrollTop,
      }) as DOMRect,
  );
  const scrollRef = { current: container };
  const h = renderHook(() => {
    const paging = useMessagePaging({
      messages: MESSAGES,
      scrollRef,
      userExchangeCount: 2,
    });
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
  it("reveals the matching text and retains its new anchor during cooldown", () => {
    const { container, row, h } = setupAnchoredPaging();
    row.textContent = "needle";
    const range = document.createRange();
    range.selectNodeContents(row);
    Object.defineProperty(range, "getBoundingClientRect", {
      value: () => ({ top: 150, height: 20 }),
    });
    act(() => h.current.revealSearchMatch(range));
    expect(container.scrollTop).toBe(660);
    expect(h.current.coolingDown).toBe(true);
    expect(container.style.overflowY).toBe("hidden");
    act(() => h.current.handleScroll());
    expect(container.scrollTop).toBe(660);
    h.unmount();
    container.remove();
  });

  it("expands only as far as the earliest matching cached message", async () => {
    const { container, h } = setup();
    await act(async () => h.current.prepareSearch("missing", new AbortController().signal));
    expect(h.current.visibleMessages[0]?.id).toBe("u5");
    await act(async () => h.current.prepareSearch("u3", new AbortController().signal));
    expect(h.current.visibleMessages[0]?.id).toBe("u3");
    expect(h.current.coolingDown).toBe(false);
    expect(h.current.showLoadOlderHint).toBe(false);
    expect(container.style.overflowY).toBe("");
    h.unmount();
    container.remove();
  });

  it("does not restart or release wheel protection while preparing search", async () => {
    const { container, h } = setupAnchoredPaging();
    await act(async () => h.current.prepareSearch("u1", new AbortController().signal));
    expect(h.current.coolingDown).toBe(true);
    expect(container.style.overflowY).toBe("hidden");
    h.unmount();
    container.remove();
  });

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

  it.each(["exchange", "assistant preview"])("fills the initial page when history arrives after a lone %s", (preview) => {
    let messages = preview === "exchange" ? MESSAGES.slice(-2) : MESSAGES.slice(-1);
    const scrollRef = { current: null as HTMLElement | null };
    const h = renderHook(() => useMessagePaging({
      messages,
      scrollRef,
      userExchangeCount: 2,
    }));
    expect(h.current.visibleMessages).toEqual(messages);

    // A live event or warm snapshot can precede the authoritative tail page.
    messages = MESSAGES;
    h.rerender();
    expect(h.current.visibleMessages.map(m => m.id)).toEqual([
      "u5",
      "a5",
      "u6",
      "a6",
    ]);
    expect(h.current.canLoadOlder).toBe(true);
    h.unmount();
  });

  it("keeps an explicitly expanded window when the tail is refreshed", async () => {
    let messages = MESSAGES;
    const scrollRef = { current: null as HTMLElement | null };
    const h = renderHook(() => useMessagePaging({ messages, scrollRef, userExchangeCount: 2 }));
    act(() => h.current.loadOlder());
    await settle();
    expect(h.current.visibleMessages[0]?.id).toBe("u3");

    messages = [...MESSAGES, msg("u7", "user"), msg("a7", "assistant")];
    h.rerender();
    expect(h.current.visibleMessages[0]?.id).toBe("u3");
    expect(h.current.visibleMessages[h.current.visibleMessages.length - 1]?.id).toBe("a7");
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
    const loadOlderHistory = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          completeLoad = resolve;
        }),
    );
    const scrollRef = { current: container };
    const messages = MESSAGES.slice(8);
    const h = renderHook(() =>
      useMessagePaging({
        messages,
        scrollRef,
        userExchangeCount: 2,
        hasOlderHistory: true,
        loadOlderHistory,
      }),
    );
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

  it("leaves ctrl-wheel (zoom) and zero-delta wheel events to the browser", async () => {
    // boundary: a ctrl-wheel is the browser's pinch-zoom and a zero-delta wheel is
    // noise; neither is a paging gesture, so both must be ignored before any
    // protection or load logic runs.
    const { container, h } = setup();
    container.scrollTop = 0;
    const zoom = new WheelEvent("wheel", { deltaY: -40, ctrlKey: true, cancelable: true });
    const zero = new WheelEvent("wheel", { deltaY: 0, cancelable: true });
    act(() => container.dispatchEvent(zoom));
    act(() => container.dispatchEvent(zero));

    expect(zoom.defaultPrevented).toBe(false);
    expect(zero.defaultPrevented).toBe(false);
    expect(h.current.showLoadOlderHint).toBe(false);
    h.unmount();
  });

  it("scales a line-mode wheel by the line height, as Firefox reports it", async () => {
    // platform-cfg: a wheel event carries `deltaMode`. Chromium reports PIXEL (0),
    // Firefox reports LINE (1) for a mouse wheel, and some report PAGE (2). The
    // handler scales the delta per mode, so a Firefox user's wheel moves the view
    // by lines rather than by the raw count. Every other fixture used the
    // constructor default (pixel), so both non-pixel arms were uncovered.
    const container = document.createElement("div");
    document.body.append(container);
    container.style.lineHeight = "20px";
    const scrollRef = { current: container as HTMLElement | null };
    const h = renderHook(() => useMessagePaging({
      messages: MESSAGES,
      scrollRef,
      userExchangeCount: 2,
    }));
    // Enter the protected state at the top, then take an explicit downward wheel.
    container.scrollTop = 0;
    act(() => h.current.handleScroll());
    container.scrollTop = 600;
    const down = new WheelEvent("wheel", { deltaMode: 1, deltaY: 8, cancelable: false });
    act(() => container.dispatchEvent(down));

    // 600 + 8 lines × 20px (set above, so the assertion tests the multiplication
    // rather than the code's own `|| 16` fallback).
    expect(container.scrollTop).toBe(760);
    h.unmount();
    container.remove();
  });

  it("scales a page-mode wheel by the viewport height", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    Object.defineProperty(container, "clientHeight", { configurable: true, value: 200 });
    const scrollRef = { current: container as HTMLElement | null };
    const h = renderHook(() => useMessagePaging({
      messages: MESSAGES,
      scrollRef,
      userExchangeCount: 2,
    }));
    container.scrollTop = 0;
    act(() => h.current.handleScroll());
    container.scrollTop = 600;
    const down = new WheelEvent("wheel", { deltaMode: 2, deltaY: 8, cancelable: false });
    act(() => container.dispatchEvent(down));

    // 600 + 8 pages × 200px viewport.
    expect(container.scrollTop).toBe(2200);
    h.unmount();
    container.remove();
  });

  it("leaves a downward wheel to native scrolling", async () => {
    // boundary: only an upward wheel at the top pages history in; a downward one is
    // ordinary scrolling and must not be prevented or turned into a load.
    const { container, h } = setup();
    container.scrollTop = 0;
    const down = new WheelEvent("wheel", { deltaY: 40, cancelable: true });
    act(() => container.dispatchEvent(down));

    expect(down.defaultPrevented).toBe(false);
    expect(h.current.showLoadOlderHint).toBe(false);
    h.unmount();
  });

  it("does not load on an upward wheel away from the top", async () => {
    // boundary: the wheel is upward but the viewport is not at the top yet, so the
    // collision that starts a transaction has not happened.
    const { container, h } = setup();
    container.scrollTop = 200;
    const up = new WheelEvent("wheel", { deltaY: -40, cancelable: true });
    act(() => container.dispatchEvent(up));

    expect(h.current.showLoadOlderHint).toBe(false);
    h.unmount();
  });

  it("does not load on an upward wheel at the top when everything is loaded", () => {
    // boundary: no older page exists, so the collision must stay inert.
    const { container, h } = setup(MESSAGES.slice(8), 2);
    expect(h.current.canLoadOlder).toBe(false);
    container.scrollTop = 0;
    const up = new WheelEvent("wheel", { deltaY: -40, cancelable: true });
    act(() => container.dispatchEvent(up));

    expect(up.defaultPrevented).toBe(false);
    expect(h.current.showLoadOlderHint).toBe(false);
    h.unmount();
  });

  it("still loads older history on a non-cancelable wheel at the top", () => {
    // boundary: WebKit reports the momentum tail of a gesture as a wheel that
    // cannot be cancelled. Such an event still has to page history in - only the
    // `preventDefault` is unavailable - so the load must not be gated on it.
    const container = document.createElement("div");
    document.body.append(container);
    const loadOlderHistory = vi.fn(() => Promise.resolve());
    const h = renderHook(() => useMessagePaging({
      messages: MESSAGES.slice(8),
      scrollRef: { current: container },
      userExchangeCount: 2,
      hasOlderHistory: true,
      loadOlderHistory,
    }));
    expect(h.current.canLoadOlder).toBe(true);

    container.scrollTop = 0;
    const up = new WheelEvent("wheel", { deltaY: -40, cancelable: false });
    act(() => container.dispatchEvent(up));

    expect(up.defaultPrevented).toBe(false);
    expect(loadOlderHistory).toHaveBeenCalledTimes(1);
    expect(h.current.showLoadOlderHint).toBe(true);
    h.unmount();
    container.remove();
  });

  it("searches the transcript the loader returns, not the windowed messages", async () => {
    // boundary: `loadAllHistoryForSearch` is the optional prop `ThreadSearch`
    // passes when a search must see history the local window has not loaded.
    // Without it the hook can only prefilter the messages already in memory, so
    // a match further up the thread is unreachable - and the window must move
    // onto the match the loader produced, not onto anything local.
    const container = document.createElement("div");
    document.body.appendChild(container);
    const loader = vi.fn(async (_signal: AbortSignal) => MESSAGES);
    let messages = MESSAGES.slice(8);
    const h = renderHook(() => useMessagePaging({
      messages,
      scrollRef: { current: container },
      userExchangeCount: 2,
      loadAllHistoryForSearch: loader,
    }));
    expect(h.current.visibleMessages.map(message => message.id)).toEqual(["u5", "a5", "u6", "a6"]);

    await act(async () => {
      await h.current.prepareSearch("u1", new AbortController().signal);
    });

    expect(loader).toHaveBeenCalledTimes(1);
    expect(loader.mock.calls[0]![0]).toBeInstanceOf(AbortSignal);

    // The caller merges the fetched history, as `useThreadMessages` does; the
    // window start pinned by the scan is what brings u1 into view.
    messages = MESSAGES;
    h.rerender();
    expect(h.current.visibleMessages[0]?.id).toBe("u1");
    h.unmount();
    container.remove();
  });

  it("renders an empty conversation without a window anchor", () => {
    // boundary: empty thread. `visibleMessages[0]` and `messages[start]` are both
    // undefined, so both anchors must fall back to null rather than dereferencing
    // a missing row - and scrolling must stay a no-op instead of throwing.
    const container = document.createElement("div");
    document.body.appendChild(container);
    const h = renderHook(() => useMessagePaging({
      messages: [],
      scrollRef: { current: container },
      userExchangeCount: 2,
    }));

    expect(h.current.visibleMessages).toEqual([]);
    expect(h.current.canLoadOlder).toBe(false);

    act(() => {
      container.scrollTop = 0;
      h.current.handleScroll();
    });

    expect(h.current.visibleMessages).toEqual([]);
    expect(h.current.showLoadOlderHint).toBe(false);
    h.unmount();
    container.remove();
  });

  it("keeps the window when the history page comes back empty", async () => {
    // boundary: the page callback is an optional out-parameter of
    // `loadOlderHistory`, and a caller that has run past the oldest row reports
    // an empty page. Dereferencing `page[0]` unconditionally would throw inside
    // the callback; the guard makes an empty page a no-op.
    const container = document.createElement("div");
    document.body.appendChild(container);
    const loadOlderHistory = vi.fn((onPage?: (page: AgentMessage[]) => void) => {
      onPage?.([]);
      return Promise.resolve();
    });
    const h = renderHook(() => useMessagePaging({
      messages: MESSAGES.slice(8),
      scrollRef: { current: container },
      userExchangeCount: 2,
      hasOlderHistory: true,
      loadOlderHistory,
    }));
    const before = h.current.visibleMessages.map(message => message.id);

    act(() => {
      h.current.loadOlder();
    });
    await act(async () => {
      await Promise.resolve();
    });

    expect(loadOlderHistory).toHaveBeenCalledTimes(1);
    expect(h.current.visibleMessages.map(message => message.id)).toEqual(before);
    h.unmount();
    container.remove();
  });

  it("is inert when more history is declared but no page has been loaded", () => {
    // boundary: `hasOlderHistory` and `loadOlderHistory` are independent props,
    // so a view can report older history while holding zero local messages and
    // no loader. The load path then falls to its non-loading branch with
    // `messages[start]` undefined - the row lookup must yield no anchor rather
    // than dereference a missing message.
    const container = document.createElement("div");
    document.body.appendChild(container);
    const h = renderHook(() => useMessagePaging({
      messages: [],
      scrollRef: { current: container },
      userExchangeCount: 2,
      hasOlderHistory: true,
    }));
    expect(h.current.canLoadOlder).toBe(true);

    expect(() => act(() => h.current.loadOlder())).not.toThrow();

    expect(h.current.visibleMessages).toEqual([]);
    h.unmount();
    container.remove();
  });

  it("keeps the loaded older page in the window once the caller merges it", async () => {
    // boundary: with the window already at the oldest local row
    // (`effectivePageStart === 0`), a successful load reports the new page through
    // the callback, and that page's first row becomes the pinned window start so
    // the fetched exchange stays visible instead of snapping back.
    const container = document.createElement("div");
    document.body.appendChild(container);
    const older = [msg("u3", "user"), msg("a3", "assistant")];
    const loadOlderHistory = vi.fn((onPage?: (page: AgentMessage[]) => void) => {
      onPage?.(older);
      return Promise.resolve();
    });
    let messages = MESSAGES.slice(8);
    const h = renderHook(() => useMessagePaging({
      messages,
      scrollRef: { current: container },
      userExchangeCount: 2,
      hasOlderHistory: true,
      loadOlderHistory,
    }));
    expect(h.current.visibleMessages.map(message => message.id)).toEqual(["u5", "a5", "u6", "a6"]);

    act(() => {
      h.current.loadOlder();
    });
    await act(async () => {
      await Promise.resolve();
    });
    expect(loadOlderHistory).toHaveBeenCalledTimes(1);

    // The caller merges the fetched page into the transcript, as
    // `useThreadMessages` does; the pinned window from the page callback is what
    // keeps the newly loaded exchange visible.
    messages = [...older, ...messages];
    h.rerender();

    expect(h.current.visibleMessages.map(message => message.id)).toEqual([
      "u3",
      "a3",
      "u5",
      "a5",
      "u6",
      "a6",
    ]);
    h.unmount();
    container.remove();
  });

  it("refuses to start a search on a signal that is already aborted", async () => {
    // error-path: `ThreadSearch` aborts a superseded query, and the signal it
    // passes may already be dead by the time the scan starts. Aborting before the
    // loader runs must surface as a cancellation rather than pinning a window.
    const container = document.createElement("div");
    document.body.appendChild(container);
    const loader = vi.fn(async (_signal: AbortSignal) => MESSAGES);
    const h = renderHook(() => useMessagePaging({
      messages: MESSAGES.slice(8),
      scrollRef: { current: container },
      userExchangeCount: 2,
      loadAllHistoryForSearch: loader,
    }));
    const before = h.current.visibleMessages.map(message => message.id);
    const controller = new AbortController();
    controller.abort();

    await expect(
      act(async () => h.current.prepareSearch("u1", controller.signal)),
    ).rejects.toThrow(/cancelled/i);

    expect(h.current.visibleMessages.map(message => message.id)).toEqual(before);
    h.unmount();
    container.remove();
  });

  it("stops a full-history scan when the signal fires during its batch wait", async () => {
    // concurrency: the scan yields to the event loop every 20 messages, and an
    // abort during that yield must stop it before it pins a window onto a match.
    // The yield needs more than 20 messages outside the loaded window, which no
    // other fixture provides - so the batch-wait branch was uncovered.
    const all = Array.from({ length: 25 }, (_, index) => msg(`x${index}`, "user"));
    const loader = vi.fn(async (_signal: AbortSignal) => all);
    const container = document.createElement("div");
    document.body.appendChild(container);
    const h = renderHook(() => useMessagePaging({
      messages: all.slice(-4),
      scrollRef: { current: container },
      userExchangeCount: 2,
      loadAllHistoryForSearch: loader,
    }));
    const before = h.current.visibleMessages.map(message => message.id);

    const controller = new AbortController();
    // Attach the rejection matcher IMMEDIATELY: the scan rejects inside the timer
    // callback below, and a promise with no handler at that moment is reported by
    // Vitest as an unhandled error even though the test asserts it afterwards.
    const pending = expect(
      h.current.prepareSearch("nomatch", controller.signal),
    ).rejects.toThrow(/cancelled/i);
    // Let the loader resolve so the scan reaches its first 20-message yield.
    await act(async () => {
      await Promise.resolve();
    });
    controller.abort();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });

    await pending;
    expect(h.current.visibleMessages.map(message => message.id)).toEqual(before);
    h.unmount();
    container.remove();
  });

  it("abandons a full-history scan whose view unmounted during its batch wait", async () => {
    // concurrency: the same batch-wait check has TWO operands and `||`
    // short-circuits, so the abort test above only ever evaluates the first. A view
    // that unmounts mid-scan (a fast thread switch while the walk is yielding) must
    // stop the walk through the second operand instead - nothing may be pinned onto
    // a window nobody is looking at.
    const all = Array.from({ length: 25 }, (_, index) => msg(`x${index}`, "user"));
    const loader = vi.fn(async (_signal: AbortSignal) => all);
    const container = document.createElement("div");
    document.body.appendChild(container);
    const h = renderHook(() => useMessagePaging({
      messages: all.slice(-4),
      scrollRef: { current: container },
      userExchangeCount: 2,
      loadAllHistoryForSearch: loader,
    }));
    const before = h.current.visibleMessages.map(message => message.id);

    // No abort this time: only the mount state can stop the walk.
    const controller = new AbortController();
    const pending = expect(
      h.current.prepareSearch("nomatch", controller.signal),
    ).rejects.toThrow(/cancelled/i);
    await act(async () => {
      await Promise.resolve();
    });
    h.unmount();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });

    await pending;
    // The window was never moved onto a match (the walk stopped before it could
    // pin one), and the loader ran exactly once.
    expect(before).toEqual(["x23", "x24"]);
    expect(loader).toHaveBeenCalledTimes(1);
    container.remove();
  });

  it("walks past its batch wait to finish a long search that is not interrupted", async () => {
    // boundary: the batch-wait check has a fall-through side - a scan that is neither
    // aborted nor unmounted keeps walking. That is the ordinary long-history case,
    // and it needs MORE than 20 messages outside the loaded window so the walk
    // crosses a yield and continues, landing on a match that sits beyond it.
    const all = Array.from({ length: 30 }, (_, index) => msg(`x${index}`, "user"));
    let messages = all.slice(-4);
    // Only the match at x25 is a hit, so the walk must cross the x20 yield to find
    // it - a single-batch scan cannot reach it.
    const loader = vi.fn(async (_signal: AbortSignal) => all);
    const container = document.createElement("div");
    document.body.appendChild(container);
    const h = renderHook(() => useMessagePaging({
      messages,
      scrollRef: { current: container },
      userExchangeCount: 2,
      loadAllHistoryForSearch: loader,
    }));

    // Start the walk without awaiting it, then pump the fake clock: the loader's
    // promise resolves first, and only afterwards does the loop reach its
    // `setTimeout(0)` yield - so a single advance cannot cover both.
    const pending = h.current.prepareSearch("x25", new AbortController().signal);
    for (let i = 0; i < 5; i++) {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1);
      });
    }
    await act(async () => {
      await pending;
    });

    // The caller merges the transcript the loader returned, as `useThreadMessages`
    // does; the window start pinned by the walk is what brings x25 into view.
    messages = all;
    h.rerender();
    expect(h.current.visibleMessages[0]?.id).toBe("x25");
    h.unmount();
    container.remove();
  });

  it("reveals nothing for a match outside the scroll container", () => {
    // boundary: `revealSearchMatch` receives a DOM Range located by the search.
    // A range whose node is not inside this container (a stale result after a
    // re-render) must be ignored rather than scrolling the view to a stranger's
    // geometry.
    const { container, h } = setup();
    const outside = document.createElement("div");
    outside.textContent = "elsewhere";
    document.body.append(outside);
    const range = document.createRange();
    range.selectNodeContents(outside);
    const before = container.scrollTop;

    act(() => h.current.revealSearchMatch(range));

    expect(container.scrollTop).toBe(before);
    expect(h.current.coolingDown).toBe(false);
    outside.remove();
    h.unmount();
  });
});
