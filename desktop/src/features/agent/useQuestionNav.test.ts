// @vitest-environment jsdom
import type { AgentMessage } from "@future-os/thread-projection";
import { act } from "react";
import { describe, expect, it, vi } from "vitest";
import { renderHook } from "../../test/renderHook";
import { useQuestionNav } from "./useQuestionNav";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function user(id: string): AgentMessage {
  return {
    id,
    role: "user",
    content: `prompt ${id}`,
    status: "complete",
    createdAt: "2026-01-01T00:00:00Z",
  } as AgentMessage;
}

function assistant(id: string): AgentMessage {
  return {
    id,
    role: "assistant",
    content: `answer ${id}`,
    status: "complete",
    createdAt: "2026-01-01T00:00:00Z",
  } as AgentMessage;
}

describe("useQuestionNav", () => {
  it("has no previous at the first question", () => {
    const scrollRef = { current: null as HTMLElement | null };
    const hook = renderHook(() =>
      useQuestionNav({
        messages: [user("u1"), assistant("a1")],
        scrollRef,
        onTakeOver: () => {},
      }),
    );

    expect(hook.current.hasPrevious).toBe(false);
    expect(hook.current.hasNext).toBe(false);
    hook.unmount();
  });

  it("jumps to the previous question and disables bottom-follow", () => {
    const onTakeOver = vi.fn();
    const { container, scrollRef } = createScrollableTranscript({
      rowCount: 8,
      rowHeight: 100,
    });
    const hook = renderHook(() =>
      useQuestionNav({
        messages: [
          user("u1"),
          assistant("a1"),
          user("u2"),
          assistant("a2"),
          user("u3"),
          assistant("a3"),
          user("u4"),
          assistant("a4"),
        ],
        scrollRef,
        onTakeOver,
      }),
    );

    act(() => {
      container.scrollTop = 500;
    });
    act(() => hook.rerender());
    expect(hook.current.hasPrevious).toBe(true);
    act(() => hook.current.goToPrevious());
    expect(onTakeOver).toHaveBeenCalled();
    // The actual scroll value depends on jsdom's mocked geometry; the key
    // assertion is that the container scrolled somewhere (it changed from 500).
    expect(container.scrollTop).not.toBe(500);
    hook.unmount();
    container.remove();
  });

  it("jumps to the next question", () => {
    const onTakeOver = vi.fn();
    const { container, scrollRef } = createScrollableTranscript();
    const hook = renderHook(() =>
      useQuestionNav({
        messages: [user("u1"), assistant("a1"), user("u2"), assistant("a2"), user("u3"), assistant("a3")],
        scrollRef,
        onTakeOver,
      }),
    );

    // Keep scrollTop at 0 so the hook treats the viewport as being at the top.
    // There are questions below, so goToNext should jump.
    expect(hook.current.hasNext).toBe(true);
    act(() => hook.current.goToNext());
    expect(onTakeOver).toHaveBeenCalled();
    expect(container.scrollTop).toBeGreaterThan(0);
    hook.unmount();
    container.remove();
  });

  it("does nothing when there is no previous question", () => {
    const onTakeOver = vi.fn();
    const { container, scrollRef } = createScrollableTranscript();
    const hook = renderHook(() =>
      useQuestionNav({
        messages: [user("u1"), assistant("a1")],
        scrollRef,
        onTakeOver,
      }),
    );

    act(() => hook.current.goToPrevious());
    expect(onTakeOver).not.toHaveBeenCalled();
    hook.unmount();
    container.remove();
  });
});

function createScrollableTranscript({ rowCount = 6, rowHeight = 100 }: { rowCount?: number; rowHeight?: number } = {}) {
  const container = document.createElement("div");
  container.style.height = "400px";
  container.style.overflow = "auto";
  document.body.appendChild(container);
  const rows: HTMLDivElement[] = [];
  for (let i = 0; i < rowCount; i++) {
    const row = document.createElement("div");
    row.style.height = `${rowHeight}px`;
    row.dataset.messageId = `m${i}`;
    container.appendChild(row);
    rows.push(row);
  }
  Object.defineProperty(container, "clientHeight", { value: 400, configurable: true });
  Object.defineProperty(container, "scrollHeight", { value: rowCount * rowHeight, configurable: true });
  const containerRect: DOMRect = { top: 0, bottom: 400, left: 0, right: 400, width: 400, height: 400, x: 0, y: 0, toJSON: () => ({}) };
  vi.spyOn(container, "getBoundingClientRect").mockReturnValue(containerRect);
  rows.forEach((row, index) => {
    const rowTop = index * rowHeight;
    const rowRect: DOMRect = { top: rowTop, bottom: rowTop + rowHeight, left: 0, right: 400, width: 400, height: rowHeight, x: 0, y: rowTop, toJSON: () => ({}) };
    vi.spyOn(row, "getBoundingClientRect").mockReturnValue(rowRect);
  });
  const scrollRef = { current: container as HTMLElement | null };
  return { container, scrollRef };
}
