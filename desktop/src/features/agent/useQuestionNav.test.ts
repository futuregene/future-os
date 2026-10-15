// @vitest-environment jsdom
import type { MessageSegment } from "@future-os/thread-projection";
import type { TranscriptMessage } from "./questionNav";
import { act } from "react";
import { describe, expect, it, vi } from "vitest";
import { useKeyboardOwner } from "../../lib/keyboardOwners";
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

/** The viewport's own top edge, in viewport coordinates. */
const VIEWPORT_TOP = 24;
const VIEWPORT_HEIGHT = 300;

/**
 * A DOMRect for a content offset, as the container's own scroll puts it on
 * screen: the hook reads real geometry, so the fixture has to report it.
 */
function rectAt(contentTop: number, height: number, container: HTMLElement): DOMRect {
  const top = VIEWPORT_TOP + contentTop - container.scrollTop;
  return {
    bottom: top + height,
    height,
    left: 0,
    right: 600,
    toJSON: () => ({}),
    top,
    width: 600,
    x: 0,
    y: top,
  };
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
  const top = VIEWPORT_TOP;
  const rect = (contentTop: number, height: number): DOMRect =>
    rectAt(contentTop, height, container);
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

/** A real keydown on the window (or on `target`), as a keyboard user sends it. */
function press(key: string, target?: EventTarget, init: KeyboardEventInit = {}) {
  const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key, ...init });
  act(() => {
    (target ?? window).dispatchEvent(event);
  });
  return event;
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

describe("useQuestionNav arrow keys", () => {
  it("walks the transcript with the bare arrow keys", () => {
    // The buttons and the keys are one walk, so a press has to land exactly
    // where its button would: ↓ takes the next stop below, ↑ the previous
    // prompt. The reader starts at the top of the loaded window, where only ↓
    // has anywhere to go.
    const { container, hook } = mount();

    press("ArrowDown");
    expect(container.scrollTop).toBe(100 + 120 - 16);
    expect(hook.current.hasPrevious).toBe(true);

    press("ArrowUp");
    expect(container.scrollTop).toBe(0);
    hook.unmount();
    container.remove();
  });

  it("leaves a key the direction cannot answer to the page", () => {
    // Nothing above the first row: the ↑ button is inert there, and so is the
    // key. Swallowing it would take away the browser's ordinary line scroll —
    // the transcript is a plain scroll container, and a short one still scrolls.
    const { container, hook } = mount();

    const event = press("ArrowUp");
    expect(event.defaultPrevented).toBe(false);
    expect(container.scrollTop).toBe(0);
    hook.unmount();
    container.remove();
  });

  it("leaves the arrows alone while the caret is in a text field", () => {
    // The composer is where the reader types, so an arrow there belongs to the
    // caret. jsdom's `isContentEditable` is undefined, which is why the
    // predicate walks the attribute instead.
    const { container, hook } = mount();
    const editor = document.createElement("div");
    editor.setAttribute("contenteditable", "true");
    container.append(editor);

    const event = press("ArrowDown", editor);
    expect(event.defaultPrevented).toBe(false);
    expect(container.scrollTop).toBe(0);
    hook.unmount();
    container.remove();
  });

  it("leaves the arrows alone while a menu is open", () => {
    // The activity rail's menus move focus with the arrows; a transcript jump
    // underneath one is the app fighting the user for the same keystroke.
    const { container, hook } = mount();
    const menu = renderHook(() => useKeyboardOwner(true));

    const event = press("ArrowDown");
    expect(event.defaultPrevented).toBe(false);
    expect(container.scrollTop).toBe(0);

    // …and closing it hands the key back.
    menu.unmount();
    press("ArrowDown");
    expect(container.scrollTop).toBe(100 + 120 - 16);
    hook.unmount();
    container.remove();
  });

  it("ignores a modified or composing arrow", () => {
    // Shift/Alt/Cmd+Arrow are selection, word-motion and the platform's line
    // motion; a composing arrow picks an IME candidate.
    const { container, hook } = mount();

    press("ArrowDown", undefined, { shiftKey: true });
    press("ArrowDown", undefined, { isComposing: true });
    expect(container.scrollTop).toBe(0);
    hook.unmount();
    container.remove();
  });

  it("stops listening when the transcript is gone", () => {
    const { container, hook } = mount();
    hook.unmount();

    press("ArrowDown");
    expect(container.scrollTop).toBe(0);
    container.remove();
  });
});

/** One row's height in the paging fixture, so a landing can be arithmetic. */
const ROW_HEIGHT = 100;

/**
 * A transcript whose rows the test can re-draw.
 *
 * That is what a fetched page does to one: the rows it brings go *above* the
 * reader, so every offset below them moves — and the press that fetched them has
 * to land on the rows it brought, measured where they ended up.
 */
function pagedTranscript() {
  const container = document.createElement("div");
  container.style.height = `${VIEWPORT_HEIGHT}px`;
  container.style.overflow = "auto";
  document.body.appendChild(container);
  vi.spyOn(container, "getBoundingClientRect").mockReturnValue({
    bottom: VIEWPORT_TOP + VIEWPORT_HEIGHT,
    height: VIEWPORT_HEIGHT,
    left: 0,
    right: 600,
    toJSON: () => ({}),
    top: VIEWPORT_TOP,
    width: 600,
    x: 0,
    y: VIEWPORT_TOP,
  });
  Object.defineProperty(container, "clientHeight", { configurable: true, value: VIEWPORT_HEIGHT });
  /** Draw `ids` as the transcript, in order, one row each. */
  const layOut = (ids: string[]) => {
    container.replaceChildren();
    ids.forEach((id, index) => {
      const row = document.createElement("div");
      row.dataset.messageId = id;
      vi.spyOn(row, "getBoundingClientRect").mockImplementation(() =>
        rectAt(index * ROW_HEIGHT, ROW_HEIGHT, container));
      container.append(row);
    });
    Object.defineProperty(container, "scrollHeight", {
      configurable: true,
      value: ids.length * ROW_HEIGHT + VIEWPORT_TOP,
    });
  };
  return { container, layOut, scrollRef: { current: container as HTMLElement | null } };
}

/** The page read is the test's to finish, so it can draw the rows first. */
function deferred() {
  let release: () => void = () => {};
  const promise = new Promise<void>((resolve) => {
    release = resolve;
  });
  return { promise, release };
}

/**
 * Frames on demand.
 *
 * A real `requestAnimationFrame` arrives after the commit that drew a page's
 * rows, which is the moment the landing is resolved at — so the test, not the
 * timer, decides when the browser paints.
 */
function queuedFrames() {
  const queued: FrameRequestCallback[] = [];
  const raf = vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
    queued.push(callback);
    return queued.length;
  });
  return {
    /** Run every frame queued so far, for `rounds` batches. */
    paint(rounds = 1) {
      for (let round = 0; round < rounds && queued.length > 0; round += 1) {
        const batch = queued.splice(0);
        act(() => batch.forEach(callback => callback(0)));
      }
    },
    /** Whether a frame loop is still waiting for its rows. */
    get waiting(): number {
      return queued.length;
    },
    restore: () => raf.mockRestore(),
  };
}

/**
 * Paging from the loaded window's top.
 *
 * A remote conversation is read a page at a time, and a window can be shorter
 * than its viewport — so the top-edge scroll that asks the host for more cannot
 * be reached by scrolling at all. ↑ is the walk's own way in: it asks for the
 * page, and the press lands on the prompt it was reaching for once the rows the
 * page brought have been drawn.
 */
describe("useQuestionNav paging from the window's top", () => {
  const WINDOW_MESSAGES: TranscriptMessage[] = [
    user("u5"),
    assistant("a5", [text("t5")]),
    user("u6"),
    assistant("a6", [text("t6")]),
  ];
  const PAGE_MESSAGES: TranscriptMessage[] = [
    user("u1"),
    assistant("a1", [text("t1")]),
    user("u2"),
    assistant("a2", [text("t2")]),
  ];
  const WINDOW_IDS = ["u5", "a5", "u6", "a6"];
  const PAGE_IDS = ["u1", "a1", "u2", "a2"];

  it("fetches the page above, and lands on the prompt the press was reaching for", async () => {
    const world = pagedTranscript();
    world.layOut(WINDOW_IDS);
    const frames = queuedFrames();
    const gate = deferred();
    const loadOlder = vi.fn(() => gate.promise);
    const onTakeOver = vi.fn();
    let messages: TranscriptMessage[] = WINDOW_MESSAGES;
    const hook = renderHook(() => useQuestionNav({
      hasOlderHistory: true,
      loadOlder,
      messages,
      onTakeOver,
      scrollRef: world.scrollRef,
    }));

    // Nothing above the loaded window, and the host has more: ↑ is offered and
    // its press is a request, not a scroll.
    expect(hook.current.previous).toBeNull();
    expect(hook.current.hasPrevious).toBe(true);

    act(() => hook.current.goToPrevious());
    expect(loadOlder).toHaveBeenCalledTimes(1);
    // …and the viewport is handed over before the page lands, so a reply
    // streaming in cannot pull the reader back to the tail.
    expect(onTakeOver).toHaveBeenCalledTimes(1);

    // The page arrives: the transcript's own array grows, the rows are drawn
    // above the reader, and the reader stays on their row — the anchor the view
    // keeps when older history is prepended (`preserveViewport`).
    messages = [...PAGE_MESSAGES, ...WINDOW_MESSAGES];
    act(() => {
      world.layOut([...PAGE_IDS, ...WINDOW_IDS]);
      world.container.scrollTop = PAGE_IDS.length * ROW_HEIGHT;
      hook.rerender();
    });
    await act(async () => {
      gate.release();
    });
    frames.paint(2);

    // The press comes to rest on u2 — the closest prompt above the reader, which
    // arrived with the page — 16px under the container's own top edge. (Landing
    // "at the top" would read 0 here, and not moving at all 400.)
    expect(world.container.scrollTop).toBe(2 * ROW_HEIGHT - 16);

    // The walk then goes on *inside* the page: one step per press, not one page.
    act(() => hook.current.goToPrevious());
    expect(loadOlder).toHaveBeenCalledTimes(1);
    expect(world.container.scrollTop).toBe(0);
    hook.unmount();
    frames.restore();
    world.container.remove();
  });

  it("waits for the frame that draws the page, when the rows follow the read", async () => {
    const world = pagedTranscript();
    world.layOut(WINDOW_IDS);
    const frames = queuedFrames();
    const gate = deferred();
    const loadOlder = vi.fn(() => gate.promise);
    let messages: TranscriptMessage[] = WINDOW_MESSAGES;
    const hook = renderHook(() => useQuestionNav({
      hasOlderHistory: true,
      loadOlder,
      messages,
      onTakeOver: vi.fn(),
      scrollRef: world.scrollRef,
    }));

    act(() => hook.current.goToPrevious());
    // The read finishes before React has drawn what it brought. The landing has
    // to watch the frames that follow it, not the one it was asked on — and it
    // leaves the reader where they are until the page is actually there.
    await act(async () => {
      gate.release();
    });
    expect(frames.waiting).toBe(1);
    expect(world.container.scrollTop).toBe(0);

    messages = [...PAGE_MESSAGES, ...WINDOW_MESSAGES];
    act(() => {
      world.layOut([...PAGE_IDS, ...WINDOW_IDS]);
      world.container.scrollTop = PAGE_IDS.length * ROW_HEIGHT;
      hook.rerender();
    });
    frames.paint(1);

    expect(world.container.scrollTop).toBe(2 * ROW_HEIGHT - 16);
    hook.unmount();
    frames.restore();
    world.container.remove();
  });

  it("asks for one page per press, however the press is repeated", async () => {
    const world = pagedTranscript();
    world.layOut(WINDOW_IDS);
    const frames = queuedFrames();
    const gate = deferred();
    const loadOlder = vi.fn(() => gate.promise);
    let loadingOlder = false;
    const hook = renderHook(() => useQuestionNav({
      hasOlderHistory: true,
      loadOlder,
      loadingOlder,
      messages: WINDOW_MESSAGES,
      onTakeOver: vi.fn(),
      scrollRef: world.scrollRef,
    }));

    act(() => {
      hook.current.goToPrevious();
      hook.current.goToPrevious();
    });
    expect(loadOlder).toHaveBeenCalledTimes(1);

    // …and while the host is reading it, the walk is busy: the button is inert
    // until the page has landed (the same rule the mobile walk applies).
    loadingOlder = true;
    act(() => {
      hook.rerender();
      hook.current.goToPrevious();
    });
    expect(loadOlder).toHaveBeenCalledTimes(1);
    expect(hook.current.hasPrevious).toBe(false);
    await act(async () => {
      gate.release();
    });
    hook.unmount();
    frames.restore();
    world.container.remove();
  });

  it("pages from the arrow key too, since the keys and the buttons are one walk", () => {
    const world = pagedTranscript();
    world.layOut(WINDOW_IDS);
    const loadOlder = vi.fn(() => Promise.resolve());
    const hook = renderHook(() => useQuestionNav({
      hasOlderHistory: true,
      loadOlder,
      messages: WINDOW_MESSAGES,
      onTakeOver: vi.fn(),
      scrollRef: world.scrollRef,
    }));

    expect(press("ArrowUp").defaultPrevented).toBe(true);
    expect(loadOlder).toHaveBeenCalledTimes(1);
    hook.unmount();
    world.container.remove();
  });

  it("leaves the reader where they were when the page never arrives", async () => {
    const world = pagedTranscript();
    world.layOut(WINDOW_IDS);
    const frames = queuedFrames();
    const gate = deferred();
    const loadOlder = vi.fn(() => gate.promise);
    const hook = renderHook(() => useQuestionNav({
      hasOlderHistory: true,
      loadOlder,
      messages: WINDOW_MESSAGES,
      onTakeOver: vi.fn(),
      scrollRef: world.scrollRef,
    }));

    act(() => hook.current.goToPrevious());
    // The read fails: nothing is ever drawn above the reader.
    await act(async () => {
      gate.release();
    });
    frames.paint(20);

    expect(world.container.scrollTop).toBe(0);
    // The wait is bounded, so nothing is left looking for rows that are not
    // coming — and the next press is free to ask again.
    expect(frames.waiting).toBe(0);
    hook.unmount();
    frames.restore();
    world.container.remove();
  });
});
