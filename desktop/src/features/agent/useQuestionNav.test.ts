// @vitest-environment jsdom
import type { MessageSegment } from "@future-os/thread-projection";
import type { TranscriptMessage } from "./questionNav";
import { act } from "react";
import { describe, expect, it, vi } from "vitest";
import { renderHook } from "../../test/renderHook";
import { useQuestionNav } from "./useQuestionNav";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function user(id: string): TranscriptMessage {
  return { id, role: "user", content: `prompt ${id}` };
}

function assistant(id: string, segments: MessageSegment[]): TranscriptMessage {
  return { id, role: "assistant", content: "", segments };
}

const text = (id: string): MessageSegment => ({ id, kind: "text", text: "…" }) as MessageSegment;
const reasoning = (id: string): MessageSegment => ({ id, kind: "thinking", text: "…" }) as MessageSegment;
function call(id: string): MessageSegment {
  return ({ id, kind: "activity", item: { id, kind: "read", status: "completed" } }) as unknown as MessageSegment;
}

/**
 * Two turns. The first reply opens with reasoning and a call before its prose
 * (the shape that made the landing matter), the second is plain prose.
 */
const MESSAGES: TranscriptMessage[] = [
  user("u1"),
  assistant("a1", [reasoning("h1"), call("w1"), text("t1")]),
  user("u2"),
  assistant("a2", [text("t2")]),
];

/**
 * A transcript with rows at fixed heights and — for the reply a1 — a marked
 * anchor block inside it, laid out the way `MessageBlock` marks it. Rects are
 * computed from `scrollTop` so the hook sees a real viewport: this is the only
 * way to exercise "the row the reader is inside" without a browser.
 */
function transcript({
  heights = [100, 400, 80, 200],
  anchors = { a1: 120 } as Record<string, number>,
  viewportHeight = 300,
} = {}) {
  const container = document.createElement("div");
  container.style.height = `${viewportHeight}px`;
  container.style.overflow = "auto";
  document.body.appendChild(container);
  const offsets: number[] = [];
  let running = 0;
  for (const height of heights) {
    offsets.push(running);
    running += height;
  }
  const total = running;
  const rows = MESSAGES.map((message, index) => {
    const row = document.createElement("div");
    row.dataset.messageId = message.id;
    row.style.height = `${heights[index]}px`;
    container.appendChild(row);
    return row;
  });
  const top = 24;
  const rect = (contentTop: number, height: number): DOMRect => ({
    bottom: top + contentTop - container.scrollTop + height,
    height,
    left: 0,
    right: 600,
    toJSON: () => ({}),
    top: top + contentTop - container.scrollTop,
    width: 600,
    x: 0,
    y: top + contentTop - container.scrollTop,
  });
  vi.spyOn(container, "getBoundingClientRect").mockReturnValue({
    bottom: top + viewportHeight,
    height: viewportHeight,
    left: 0,
    right: 600,
    toJSON: () => ({}),
    top,
    width: 600,
    x: 0,
    y: top,
  });
  Object.defineProperty(container, "clientHeight", { configurable: true, value: viewportHeight });
  Object.defineProperty(container, "scrollHeight", { configurable: true, value: total + top });
  rows.forEach((row, index) => {
    vi.spyOn(row, "getBoundingClientRect").mockImplementation(() =>
      rect(offsets[index]!, heights[index]!));
    const anchorOffset = anchors[MESSAGES[index]!.id];
    if (anchorOffset === undefined)
      return;
    const anchor = document.createElement("div");
    anchor.dataset.replyTail = "";
    row.appendChild(anchor);
    vi.spyOn(anchor, "getBoundingClientRect").mockImplementation(() =>
      rect(offsets[index]! + anchorOffset, 280));
  });
  const scrollRef = { current: container as HTMLElement | null };
  return { container, rows, scrollRef, offsets, total, top };
}

/** Put the viewport's top edge at a content offset (as a scroll would). */
function scrollTo(container: HTMLElement, offset: number) {
  act(() => {
    container.scrollTop = offset;
    // jsdom stores the value but never fires the event a real scroll would;
    // the hook's own listener is what keeps the offer honest, so fire it.
    container.dispatchEvent(new Event("scroll"));
  });
}

function mount(
  messages: TranscriptMessage[] = MESSAGES,
  layout: Parameters<typeof transcript>[0] = {},
) {
  const world = transcript(layout);
  const onTakeOver = vi.fn();
  const hook = renderHook(() =>
    useQuestionNav({ messages, onTakeOver, scrollRef: world.scrollRef }),
  );
  return { ...world, hook, onTakeOver };
}

describe("useQuestionNav", () => {
  it("offers ↑ from the tail before the reader has scrolled, and no ↓", () => {
    // At the top of the loaded window the first row is the reader's, so ↑ has
    // nowhere to go; ↓ walks the conversation from there.
    const { container, hook } = mount();
    expect(hook.current.previous).toBeNull();
    expect(hook.current.hasNext).toBe(true);
    expect(hook.current.hasPrevious).toBe(false);
    hook.unmount();
    container.remove();
  });

  it("lands a question with its top edge just under the viewport's", () => {
    const { container, hook, onTakeOver } = mount();
    // Inside the first reply: its middle is on screen, so the reply is the row
    // the reader is reading, and the next stop below is the prompt after it.
    scrollTo(container, 300);

    act(() => hook.current.goToNext());

    // Row u2 sits at content offset 500. The jump puts it 16px below the
    // container's own top edge, so the container scrolls to 500 - 16.
    expect(container.scrollTop).toBe(484);
    expect(onTakeOver).toHaveBeenCalledTimes(1);
    hook.unmount();
    container.remove();
  });

  it("offers the newest turn's prompt at the end of the transcript", () => {
    // The end of a conversation: the previous prompt's last line sits under the
    // top edge while the reply the reader is reading fills the rest. Reading that
    // sliver as "where I am" would send the first ↑ past the prompt of the turn
    // on screen — the one press that must land on it.
    const { container, hook } = mount(MESSAGES, {
      heights: [100, 400, 80, 260],
      viewportHeight: 300,
    });
    scrollTo(container, 564);

    // The reader is at the newest row, so ↑ offers that turn's prompt…
    expect(hook.current.previous).toEqual({ kind: "question", index: 2, messageId: "u2" });
    // …and there is nothing newer below the newest row.
    expect(hook.current.hasNext).toBe(false);
    hook.unmount();
    container.remove();
  });

  it("reads the row it is looking at when it has scrolled above the end", () => {
    // One line down from the end, the prompt above is fully in view: that is
    // what the reader is looking at, so ↑ steps past it — the sliver rule only
    // decides a row the reader is *not* reading, and here it is not a sliver.
    const { container, hook } = mount(MESSAGES, {
      heights: [100, 400, 80, 260],
      viewportHeight: 300,
    });
    scrollTo(container, 400);

    expect(hook.current.previous).toEqual({ kind: "question", index: 0, messageId: "u1" });
    expect(hook.current.hasNext).toBe(true);
    hook.unmount();
    container.remove();
  });

  it("still moves at the end of a conversation whose last exchange is short", () => {
    // A short last exchange: the newest prompt is on screen below the top edge
    // and the content ends just under it, so it cannot be lifted to the top —
    // the press would be a no-op. The walk goes to the prompt it can reach, and
    // ↓ has nothing newer to offer.
    const { container, hook } = mount(MESSAGES, {
      heights: [100, 400, 80, 60],
      viewportHeight: 300,
    });
    scrollTo(container, 364);

    // The press reaches the prompt it *can* bring to the top edge, leaving the
    // one it cannot (already in view) behind it.
    act(() => hook.current.goToPrevious());
    expect(container.scrollTop).toBe(0);
    expect(hook.current.previous).toBeNull();
    hook.unmount();
    container.remove();
  });

  it("does not read a sliver of the row above as the reader's row", () => {
    // Above the end of the transcript, where the rows themselves decide: the
    // previous reply's last 40px are under the top edge and the prompt the
    // reader is looking at is fully in view below it. The reader is in that
    // prompt, so ↓ offers the reply that answers it.
    const { container, hook } = mount(MESSAGES, {
      heights: [100, 400, 80, 260],
      viewportHeight: 300,
    });
    scrollTo(container, 484);

    expect(hook.current.next).toEqual({ kind: "replyTail", index: 3, messageId: "a2" });
    expect(hook.current.previous).toEqual({ kind: "question", index: 0, messageId: "u1" });
    hook.unmount();
    container.remove();
  });

  it("keeps a row taller than the viewport as the row being read", () => {
    // The other side of the same rule: a reply the reader has scrolled into is
    // still theirs to step off, even though its start is long gone.
    const { container, hook } = mount(MESSAGES, {
      heights: [100, 700, 80, 60],
      viewportHeight: 300,
    });
    scrollTo(container, 200);

    // ↑ reaches its own prompt…
    expect(hook.current.previous).toEqual({ kind: "question", index: 0, messageId: "u1" });
    // …and ↓ the prompt below it.
    expect(hook.current.next).toEqual({ kind: "question", index: 2, messageId: "u2" });
    act(() => hook.current.goToNext());
    // The prompt below cannot reach the top edge (the content ends above it), so
    // the landing goes as far as it can: the reader gets that prompt in view.
    expect(container.scrollTop).toBe(664);
    hook.unmount();
    container.remove();
  });

  it("lands a reply on its last output, not on the row's start", () => {
    const { container, hook } = mount();
    // The reader is inside the first prompt, so the next stop below is the reply
    // that answers it.
    act(() => hook.current.goToNext());

    // The row starts at 100, but its prose sits 120px into it: the press was for
    // the answer, so that is what takes the reading edge.
    expect(container.scrollTop).toBe(100 + 120 - 16);
    hook.unmount();
    container.remove();
  });

  it("↑ skips the reply and reaches the prompt above it", () => {
    const { container, hook } = mount();
    scrollTo(container, 400);

    act(() => hook.current.goToPrevious());

    // Inside the second turn, ↑ reaches the first prompt: the reply between the
    // two is what ↓ is for.
    expect(container.scrollTop).toBe(0);
    hook.unmount();
    container.remove();
  });

  it("never scrolls above the loaded window's first row", () => {
    const { container, hook } = mount();
    scrollTo(container, 120);

    act(() => hook.current.goToPrevious());

    // Row u1 is already the top of what is loaded; the jump clamps at the
    // window's own start instead of inventing a negative offset.
    expect(container.scrollTop).toBe(0);
    hook.unmount();
    container.remove();
  });

  it("keeps the offer current as the reader scrolls", () => {
    const { container, hook } = mount();
    expect(hook.current.hasPrevious).toBe(false);

    scrollTo(container, 400);
    expect(hook.current.hasPrevious).toBe(true);

    // …and back: the offer is a fact about the viewport, not about the last
    // press, so a scroll of their own changes what the buttons do.
    scrollTo(container, 0);
    expect(hook.current.hasPrevious).toBe(false);
    expect(hook.current.hasNext).toBe(true);
    hook.unmount();
    container.remove();
  });

  it("retracts ↓ at the newest row", () => {
    const { container, hook } = mount();
    scrollTo(container, 600);

    expect(hook.current.next).toBeNull();
    expect(hook.current.hasNext).toBe(false);
    hook.unmount();
    container.remove();
  });

  it("steps past a stop the viewport cannot reach", () => {
    // At the very end of a conversation whose last exchange is short, the last
    // prompt is already on screen: landing it at the top edge would need an
    // offset past the end of the transcript, so that press would move nothing.
    // The walk steps on to the prompt above, which it can reach.
    const { container, hook } = mount(MESSAGES, {
      heights: [100, 400, 80, 40],
      viewportHeight: 300,
    });
    scrollTo(container, 344);

    act(() => hook.current.goToPrevious());

    expect(container.scrollTop).toBe(0);
    // …and the offer agrees with what the press does.
    expect(hook.current.previous).toBeNull();
    hook.unmount();
    container.remove();
  });

  it("offers the reachable stop rather than the one it would skip", () => {
    // The same position without pressing: the button says what pressing it
    // would do, so ↓ is not offered as a jump that could not move the viewport.
    const { container, hook } = mount(MESSAGES, {
      heights: [100, 400, 80, 40],
      viewportHeight: 300,
    });
    scrollTo(container, 344);

    expect(hook.current.previous).toEqual({ kind: "question", index: 0, messageId: "u1" });
    expect(hook.current.hasNext).toBe(false);
    hook.unmount();
    container.remove();
  });

  it("does nothing when a direction has nowhere to go", () => {
    const { container, hook, onTakeOver } = mount();
    act(() => hook.current.goToPrevious());
    expect(onTakeOver).not.toHaveBeenCalled();
    expect(container.scrollTop).toBe(0);

    scrollTo(container, 600);
    act(() => hook.current.goToNext());
    expect(onTakeOver).not.toHaveBeenCalled();
    hook.unmount();
    container.remove();
  });

  it("lands a reply row at its start when the row marked no anchor", () => {
    // A reply whose output is plain content, or one the renderer found no
    // output block in: the row's own top is all there is to aim at.
    const world = transcript({ anchors: {} });
    const hook = renderHook(() =>
      useQuestionNav({ messages: MESSAGES, onTakeOver: () => {}, scrollRef: world.scrollRef }),
    );
    act(() => hook.current.goToNext());
    expect(world.container.scrollTop).toBe(100 - 16);
    hook.unmount();
    world.container.remove();
  });

  it("goes inert when there is no transcript to read", () => {
    const scrollRef = { current: null as HTMLElement | null };
    const hook = renderHook(() =>
      useQuestionNav({ messages: MESSAGES, onTakeOver: () => {}, scrollRef }),
    );
    expect(hook.current.hasPrevious).toBe(false);
    expect(hook.current.hasNext).toBe(false);
    act(() => hook.current.goToNext());
    hook.unmount();
  });
});
