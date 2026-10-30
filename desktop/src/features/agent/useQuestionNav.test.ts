// @vitest-environment jsdom
import type { TranscriptMessage } from "./questionNav";
import { act } from "react";
import { describe, expect, it, vi } from "vitest";
import { useKeyboardOwner } from "../../lib/keyboardOwners";
import { renderHook } from "../../test/renderHook";
import { useQuestionNav } from "./useQuestionNav";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function user(id: string): TranscriptMessage {
  return { id, role: "user" };
}

function assistant(id: string): TranscriptMessage {
  return { id, role: "assistant" };
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
 * Two turns, each a prompt and a reply. The reply heights are what make the
 * walk's geometry interesting: the first one is taller than the viewport.
 */
const MESSAGES: TranscriptMessage[] = [
  user("u1"),
  assistant("a1"),
  user("u2"),
  assistant("a2"),
];

/**
 * A transcript with rows at fixed heights. Rects are computed from `scrollTop`
 * so the hook sees a real viewport: this is the only way to exercise "the row
 * the reader is inside" without a browser.
 */
function transcript(
  messages: TranscriptMessage[] = MESSAGES,
  {
    heights = [100, 400, 80, 200],
    viewportHeight = 300,
  } = {},
) {
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
  const rows = messages.map((message, index) => {
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
  layout: { heights?: number[]; viewportHeight?: number } = {},
) {
  const world = transcript(messages, layout);
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
  it("offers ↑ from the top only when a prompt is above, and ↓ to the next one", () => {
    // At the top of the loaded window the first row is the reader's, so ↑ has
    // nowhere to go; ↓ walks to the second prompt.
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
    expect(hook.current.previous).toBe(2);
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

    expect(hook.current.previous).toBe(0);
    // The reader is at the newest prompt, so there is no later question below.
    expect(hook.current.hasNext).toBe(false);
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
    // prompt, so ↓ offers the prompt after it — not the one the sliver belongs
    // to, which would be a press that goes backwards.
    const { container, hook } = mount(
      [user("u1"), assistant("a1"), user("u2"), assistant("a2"), user("u3")],
      { heights: [100, 400, 80, 260, 120], viewportHeight: 300 },
    );
    scrollTo(container, 484);

    expect(hook.current.next).toBe(4);
    expect(hook.current.previous).toBe(0);
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
    expect(hook.current.previous).toBe(0);
    // …and ↓ the prompt below it.
    expect(hook.current.next).toBe(2);
    act(() => hook.current.goToNext());
    // The prompt below cannot reach the top edge (the content ends above it), so
    // the landing goes as far as it can: the reader gets that prompt in view.
    expect(container.scrollTop).toBe(664);
    hook.unmount();
    container.remove();
  });

  it("skips a whole answer in one press", () => {
    const { container, hook } = mount();
    // From the first prompt, ↓ lands on the second prompt — the 400px answer
    // between them is passed in this single press, never landed in.
    act(() => hook.current.goToNext());

    expect(container.scrollTop).toBe(500 - 16);
    hook.unmount();
    container.remove();
  });

  it("↑ skips the reply and reaches the prompt above it", () => {
    const { container, hook } = mount();
    scrollTo(container, 400);

    act(() => hook.current.goToPrevious());

    // Inside the second turn, ↑ reaches the first prompt: the reply between the
    // two is never a stop.
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

    expect(hook.current.previous).toBe(0);
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

describe("useQuestionNav page keys", () => {
  it("walks the transcript with Page Up / Page Down", () => {
    // The buttons and the keys are one walk, so a press has to land exactly
    // where its button would: ↓ the next prompt below, ↑ the previous one. The
    // reader starts at the top of the loaded window, where only ↓ has anywhere
    // to go.
    const { container, hook } = mount();

    press("PageDown");
    expect(container.scrollTop).toBe(500 - 16);
    expect(hook.current.hasPrevious).toBe(true);

    press("PageUp");
    expect(container.scrollTop).toBe(0);
    hook.unmount();
    container.remove();
  });

  it("leaves the bare arrows to the page", () => {
    // ↑/↓ on a scroll container is the reader's ordinary line scroll, and on a
    // long reply it is the finer control they reach for first. Answering it here
    // would turn every arrow press into a jump of a whole turn.
    const { container, hook } = mount();

    expect(press("ArrowDown").defaultPrevented).toBe(false);
    expect(container.scrollTop).toBe(0);
    hook.unmount();
    container.remove();
  });

  it("leaves a key the direction cannot answer to the page", () => {
    // Nothing above the first row: the ↑ button is inert there, and so is the
    // key. Swallowing it would take away the browser's own screenful scroll —
    // the transcript is a plain scroll container, and a short one still scrolls.
    const { container, hook } = mount();

    const event = press("PageUp");
    expect(event.defaultPrevented).toBe(false);
    expect(container.scrollTop).toBe(0);
    hook.unmount();
    container.remove();
  });

  it("leaves the page keys alone while the caret is in a text field", () => {
    // The composer is where the reader types, so a page key there belongs to the
    // field. jsdom's `isContentEditable` is undefined, which is why the
    // predicate walks the attribute instead.
    const { container, hook } = mount();
    const editor = document.createElement("div");
    editor.setAttribute("contenteditable", "true");
    container.append(editor);

    const event = press("PageDown", editor);
    expect(event.defaultPrevented).toBe(false);
    expect(container.scrollTop).toBe(0);
    hook.unmount();
    container.remove();
  });

  it("leaves the page keys alone while a menu is open", () => {
    // A floating menu can hold its own list of rows; a transcript jump
    // underneath one is the app fighting the user for the same keystroke.
    const { container, hook } = mount();
    const menu = renderHook(() => useKeyboardOwner(true));

    const event = press("PageDown");
    expect(event.defaultPrevented).toBe(false);
    expect(container.scrollTop).toBe(0);

    // …and closing it hands the key back.
    menu.unmount();
    press("PageDown");
    expect(container.scrollTop).toBe(500 - 16);
    hook.unmount();
    container.remove();
  });

  it("ignores a modified or composing page key", () => {
    // Ctrl+Page Up/Page Down switches tabs, Shift+Page Up extends a selection,
    // and a composing press belongs to the IME.
    const { container, hook } = mount();

    press("PageDown", undefined, { shiftKey: true });
    press("PageDown", undefined, { ctrlKey: true });
    press("PageDown", undefined, { isComposing: true });
    expect(container.scrollTop).toBe(0);
    hook.unmount();
    container.remove();
  });

  it("stops listening when the transcript is gone", () => {
    const { container, hook } = mount();
    hook.unmount();

    press("PageDown");
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
    assistant("a5"),
    user("u6"),
    assistant("a6"),
  ];
  const PAGE_MESSAGES: TranscriptMessage[] = [
    user("u1"),
    assistant("a1"),
    user("u2"),
    assistant("a2"),
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

  it("pages from the page key too, since the keys and the buttons are one walk", () => {
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

    expect(press("PageUp").defaultPrevented).toBe(true);
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

/**
 * A press with no transcript on screen neither throws nor spins.
 *
 * `pageUp` reads the rows *before* it looks at the container, and the unmounted
 * case is real: the hook outlives a closed conversation long enough for a key
 * press or a click to arrive. The read must answer "no rows" rather than throw,
 * and the frame loop that waits for the page's rows must stay bounded — a page
 * that never brings any (because there is nothing to draw it into) cannot leave
 * a requestAnimationFrame loop running behind the app.
 */
it("takes a press with no transcript mounted, and stops looking for rows", async () => {
  // Its own window: the paging suite's fixtures live inside its `describe`.
  const messages: TranscriptMessage[] = [user("u1"), assistant("a1")];
  const frames = queuedFrames();
  const loadOlder = vi.fn(() => Promise.resolve());
  const onTakeOver = vi.fn();
  const hook = renderHook(() => useQuestionNav({
    hasOlderHistory: true,
    loadOlder,
    messages,
    onTakeOver,
    scrollRef: { current: null },
  }));

  // Nothing is mounted, so the walk itself finds nothing...
  expect(hook.current.hasPrevious).toBe(true);
  expect(hook.current.previous).toBeNull();

  // ...and the press falls through to the page, which is handed the viewport
  // first (the transcript stops following its tail even while it is away).
  act(() => hook.current.goToPrevious());
  expect(loadOlder).toHaveBeenCalledTimes(1);
  expect(onTakeOver).toHaveBeenCalledTimes(1);

  // The page arrives with no rows to draw: the loop looks, gives up, and leaves
  // nothing behind.
  await act(async () => {
    await Promise.resolve();
  });
  frames.paint(12);
  expect(frames.waiting).toBe(0);

  hook.unmount();
  frames.restore();
});

/**
 * A `loadOlder` that throws *synchronously* still releases the press.
 *
 * `pageUp` holds `pagePendingRef` until the read settles, and every later press
 * is refused while it does. A callback that throws on the way in (a caller whose
 * page lookup is not async-safe) must therefore not leave the flag set: the next
 * press would be swallowed for the rest of the conversation, which the reader
 * experiences as a ↑ button that stopped working.
 */
it("releases the press when the page read throws on the way in", async () => {
  const messages: TranscriptMessage[] = [user("u1"), assistant("a1")];
  const world = pagedTranscript();
  world.layOut(["u1", "a1"]);
  const frames = queuedFrames();
  let attempts = 0;
  const loadOlder = vi.fn(() => {
    attempts += 1;
    if (attempts === 1)
      throw new Error("page lookup failed");
    return Promise.resolve();
  });
  const hook = renderHook(() => useQuestionNav({
    hasOlderHistory: true,
    loadOlder,
    messages,
    onTakeOver: vi.fn(),
    scrollRef: world.scrollRef,
  }));

  act(() => hook.current.goToPrevious());
  expect(loadOlder).toHaveBeenCalledTimes(1);

  // The failed press armed the frame loop that waits for rows that will never
  // come; let it finish.
  await act(async () => {
    await Promise.resolve();
  });
  frames.paint(12);
  expect(frames.waiting).toBe(0);

  // And the *pending* flag was released by the throw, so the next press asks
  // again — a press swallowed here would be a ↑ that stopped working.
  act(() => hook.current.goToPrevious());
  expect(loadOlder).toHaveBeenCalledTimes(2);
  hook.unmount();
  frames.restore();
  world.container.remove();
});

/**
 * What the reader is reading, when no row's middle is on screen.
 *
 * A row taller than two viewports (a long tool output) has neither its top nor
 * its middle in view, and the rows after it are below the fold: the walk has to
 * stop looking at that point and answer with the row the reader is *in*, rather
 * than walking on to a row nobody can see. Getting this wrong sends the first ↓
 * past the prompt of the turn on screen.
 */
describe("useQuestionNav inside a row taller than the viewport", () => {
  it("answers with the row the reader is in, and offers the prompt below it", () => {
    const { container, hook } = mount(MESSAGES, {
      heights: [900, 100, 100, 100],
      viewportHeight: 300,
    });

    // The reader is inside u1 (900px tall, its middle far below the fold)…
    expect(hook.current.previous).toBeNull();
    // …so the next stop below is the prompt that follows it.
    expect(hook.current.next).toBe(2);

    hook.unmount();
    container.remove();
  });
});
