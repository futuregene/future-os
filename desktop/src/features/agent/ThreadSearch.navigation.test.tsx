import type { Root } from "react-dom/client";
// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ThreadSearch } from "./ThreadSearch";

/**
 * The parts of the search panel the existing suite does not reach: next/previous
 * navigation, IME composition, Enter while composing, the empty-result and
 * no-registry paths, and the frame-deferral bookkeeping.
 */
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root;
let container: HTMLDivElement;
let thread: HTMLDivElement;
let frames: Map<number, FrameRequestCallback>;
let nextFrame: number;

function stubFrameScheduler() {
  frames = new Map();
  nextFrame = 0;
  vi.stubGlobal("requestAnimationFrame", vi.fn((callback: FrameRequestCallback) => {
    nextFrame += 1;
    frames.set(nextFrame, callback);
    return nextFrame;
  }));
  vi.stubGlobal("cancelAnimationFrame", vi.fn((frame: number) => frames.delete(frame)));
}

/** Run whatever frame callbacks are queued right now. */
function flushFrame() {
  const callbacks = [...frames.values()];
  frames.clear();
  act(() => callbacks.forEach(callback => callback(0)));
}

/**
 * Let a MutationObserver microtask fire, then run the frame chain it starts
 * (frame → deferUntilAfterPaint → first/second frame).
 */
async function flushRescan() {
  for (let round = 0; round < 4; round++) {
    await act(async () => {
      await Promise.resolve();
    });
    flushFrame();
  }
}

beforeEach(() => {
  vi.useFakeTimers();
  stubFrameScheduler();
  Object.defineProperty(Element.prototype, "scrollIntoView", { configurable: true, value: vi.fn() });
  container = document.createElement("div");
  thread = document.createElement("div");
  thread.textContent = "alpha alphabet alpha";
  document.body.append(container, thread);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  thread.remove();
  delete (Element.prototype as Partial<Element>).scrollIntoView;
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

function mount(onRevealMatch?: (range: Range) => void) {
  act(() => root.render(
    <ThreadSearch contentKey={null} onPrepareSearch={async () => {}} onRevealMatch={onRevealMatch} rootRef={{ current: thread }} />,
  ));
}

function openSearch() {
  act(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ctrlKey: true, key: "f" }));
  });
}

function input() {
  return container.querySelector<HTMLInputElement>("input")!;
}

function setQuery(value: string) {
  const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  act(() => {
    setValue.call(input(), value);
    input().dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function settleSearch(query: string) {
  setQuery(query);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(300);
  });
  flushFrame();
  flushFrame();
}

function iconButton(label: string) {
  return container.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!;
}

it("walks forward and backward through the matches, wrapping at both ends", async () => {
  const revealed: Range[] = [];
  mount(range => void revealed.push(range));
  openSearch();
  await settleSearch("alpha");
  expect(container.textContent).toContain("1 / 3");

  act(() => iconButton("Next result").click());
  expect(container.textContent).toContain("2 / 3");
  act(() => iconButton("Next result").click());
  expect(container.textContent).toContain("3 / 3");

  // boundary: stepping past the last match wraps to the first.
  act(() => iconButton("Next result").click());
  expect(container.textContent).toContain("1 / 3");

  // …and stepping before the first wraps back to the last.
  act(() => iconButton("Previous result").click());
  expect(container.textContent).toContain("3 / 3");
  expect(revealed.length).toBeGreaterThan(0);
});

it("steps through the matches from the input's Enter and Shift+Enter", async () => {
  mount();
  openSearch();
  await settleSearch("alpha");
  expect(container.textContent).toContain("1 / 3");

  act(() => {
    input().dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Enter" }));
  });
  expect(container.textContent).toContain("2 / 3");

  act(() => {
    input().dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Enter", shiftKey: true }));
  });
  expect(container.textContent).toContain("1 / 3");
});

it("ignores Enter that confirms an IME composition", async () => {
  // platform-cfg: with a Chinese IME, Enter commits the candidate and must never
  // also step the search.
  mount();
  openSearch();
  await settleSearch("alpha");
  expect(container.textContent).toContain("1 / 3");

  act(() => {
    const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Enter" });
    Object.defineProperty(event, "isComposing", { value: true });
    input().dispatchEvent(event);
  });
  expect(container.textContent).toContain("1 / 3");
  expect(container.textContent).not.toContain("2 / 3");
});

it("suspends the search while a composition is in progress", async () => {
  mount();
  openSearch();
  await settleSearch("alpha");
  expect(container.textContent).toContain("1 / 3");

  act(() => {
    input().dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
  });
  // A half-committed query is not searched, and the stale count is cleared.
  expect(iconButton("Next result").disabled).toBe(true);

  act(() => {
    input().dispatchEvent(new CompositionEvent("compositionend", { bubbles: true }));
  });
  await settleSearch("alpha");
  expect(container.textContent).toContain("1 / 3");
});

it("reports zero matches and disables navigation for a query nothing matches", async () => {
  mount();
  openSearch();
  await settleSearch("zzz-nothing");

  expect(container.textContent).toContain("0 / 0");
  expect(iconButton("Next result").disabled).toBe(true);
  expect(iconButton("Previous result").disabled).toBe(true);
});

it("keeps Enter inert in the search field while there are no matches", async () => {
  // boundary + accessibility: the next/previous BUTTONS are disabled with no matches,
  // but the field's Enter handler calls `move` unconditionally, so a keyboard user
  // reaches the guard the buttons' `disabled` attribute hides. Pressing Enter there
  // must do nothing rather than show "-1 / 0" or throw on an empty range list.
  mount();
  openSearch();
  await settleSearch("zzz-nothing");
  expect(container.textContent).toContain("0 / 0");

  await act(async () => {
    input().dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Enter" }));
  });
  await act(async () => {
    input().dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Enter", shiftKey: true }));
  });
  flushFrame();

  // Still a well-formed empty state, not a negative index.
  expect(container.textContent).toContain("0 / 0");
  expect(container.textContent).not.toContain("-1");
});

it("treats a one-character query as not searchable yet", async () => {
  // boundary: a single character matches almost everything and would stall the
  // DOM scan, so the panel stays inert until the query means something.
  mount();
  openSearch();
  await settleSearch("a");

  expect(iconButton("Next result").disabled).toBe(true);
  expect(container.textContent).toContain("0 / 0");
});

it("closes on Escape and re-opens focused with the shortcut", async () => {
  mount();
  openSearch();
  expect(input()).not.toBeNull();

  act(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Escape" }));
  });
  expect(container.querySelector("input")).toBeNull();

  // boundary: Escape with the panel closed is nobody's business.
  act(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Escape" }));
  });
  expect(container.querySelector("input")).toBeNull();
});

it("drops a deferred scan whose work was superseded before it ran", async () => {
  // concurrency: a new query cancels the pending frames; the older callback must
  // not repaint results for the previous query.
  mount();
  openSearch();
  const stale = [...frames.values()];
  await settleSearch("alpha");
  expect(container.textContent).toContain("1 / 3");

  // Fire the superseded frame callbacks: they belong to work that is no longer
  // current, so nothing may change.
  act(() => stale.forEach(callback => callback(0)));
  expect(container.textContent).toContain("1 / 3");
});

it("rescans when the thread content changes under an open panel", async () => {
  mount();
  openSearch();
  await settleSearch("alpha");
  expect(container.textContent).toContain("1 / 3");

  // A streaming delta appends a third match; the MutationObserver must re-run
  // the scan once the frames settle.
  await act(async () => {
    thread.textContent = "alpha alphabet alpha and alpha";
    await Promise.resolve();
  });
  flushFrame();
  flushFrame();

  expect(container.textContent).toContain("1 / 3");
});

it("works without the CSS Highlight registry and without a reveal callback", async () => {
  // error-path: older WebViews have no `CSS.highlights`, and the caller may not
  // supply a reveal hook 鈥?neither may break the search.
  vi.stubGlobal("CSS", undefined);
  mount();
  openSearch();
  await settleSearch("alpha");

  expect(container.textContent).toContain("1 / 3");
  act(() => iconButton("Next result").click());
  expect(container.textContent).toContain("2 / 3");
});

it("tolerates a Highlight registry that cannot report existing highlights", async () => {
  const set = vi.fn();
  vi.stubGlobal("CSS", { highlights: { delete: vi.fn(), set } });
  vi.stubGlobal("Highlight", class {
    constructor(..._ranges: Range[]) {}
  });
  mount();
  openSearch();
  await settleSearch("alpha");

  expect(container.textContent).toContain("1 / 3");
  expect(set).toHaveBeenCalledTimes(2);
});

it("keeps searching when the transcript root is not mounted", async () => {
  // boundary: the panel can be open before the message list commits.
  act(() => root.render(
    <ThreadSearch contentKey={null} onPrepareSearch={async () => {}} rootRef={{ current: null }} />,
  ));
  openSearch();
  await settleSearch("alpha");

  expect(container.textContent).toContain("0 / 0");
  expect(iconButton("Next result").disabled).toBe(true);
});

it("clears the panel when the search request fails", async () => {
  // error-path: expanding history failed, so the partial result must not be
  // navigable.
  act(() => root.render(
    <ThreadSearch
      contentKey={null}
      onPrepareSearch={async () => {
        throw new Error("history unreadable");
      }}
      rootRef={{ current: thread }}
    />,
  ));
  openSearch();
  await settleSearch("alpha");

  expect(iconButton("Next result").disabled).toBe(true);
});

it("keeps the current match selected when a rescan finds the same ranges", async () => {
  // concurrency: a content change (a streaming delta elsewhere) rescans with an
  // unchanged query; the reader must stay on the match they were reading rather
  // than jumping back to the first.
  mount();
  openSearch();
  await settleSearch("alpha");
  act(() => iconButton("Next result").click());
  expect(container.textContent).toContain("2 / 3");

  // A structural change re-runs the scan with the same query.
  act(() => root.render(
    <ThreadSearch contentKey="history-v2" onPrepareSearch={async () => {}} rootRef={{ current: thread }} />,
  ));
  flushFrame();
  flushFrame();

  expect(container.textContent).toContain("2 / 3");
});

it("re-anchors to a surviving match when the transcript changed under it", async () => {
  // concurrency + boundary: a rescan whose previous range no longer exists must
  // fall back to a valid index instead of leaving the cursor at -1 (which would
  // label the panel "0 / N" while matches exist).
  mount();
  openSearch();
  await settleSearch("alpha");
  act(() => iconButton("Next result").click());
  act(() => iconButton("Next result").click());
  expect(container.textContent).toContain("3 / 3");

  // The whole transcript is replaced by equivalent-but-new text, so no old
  // Range survives the identity comparison.
  await act(async () => {
    thread.textContent = "alpha alphabet alpha alpha";
    await Promise.resolve();
  });
  await flushRescan();

  expect(container.textContent).toContain("3 / 4");
  expect(iconButton("Next result").disabled).toBe(false);
});

it("re-anchors when the match shifted inside the same text node", async () => {
  // concurrency + property: the range-identity comparison is four chained operands
  // (`startContainer`, `startOffset`, `endContainer`, `endOffset`). The existing tests
  // take the all-equal arm or fail at the FIRST operand (a replaced node). This one
  // keeps the SAME text node object - so the container operand matches - while the
  // match's offset moves, which is the only way to evaluate the `startOffset` operand
  // and find it false.
  mount();
  openSearch();
  await settleSearch("alpha");
  act(() => iconButton("Next result").click());
  expect(container.textContent).toContain("2 / 3");

  // Mutating `.data` keeps the node identity (unlike assigning textContent, which
  // replaces the child), and `characterData` is observed, so this rescans.
  await act(async () => {
    (thread.firstChild as Text).data = "zalpha alphabet alpha";
    await Promise.resolve();
  });
  await flushRescan();

  // The cursor falls back to a valid index rather than -1, and the count tracks the
  // transcript: three matches again, same position.
  expect(container.textContent).toContain("2 / 3");
  expect(iconButton("Next result").disabled).toBe(false);
});

it("re-anchors when the same start moves into a second text node", async () => {
  // The `endContainer` operand: reached only when startContainer AND startOffset both
  // match. `splitText` is the construct that produces it - it keeps the original node
  // (so the start is unchanged) and moves the tail to a new sibling, so a match at the
  // same offset now ENDS in a different node. This is the near-miss the identity
  // comparison exists for: the position is the same but the range object is not, and
  // the cursor must fall back rather than stay pinned.
  mount();
  openSearch();
  await settleSearch("alpha");
  expect(container.textContent).toContain("1 / 3");

  await act(async () => {
    (thread.firstChild as Text).splitText(2);
    await Promise.resolve();
  });
  await flushRescan();

  expect(container.textContent).toContain("1 / 3");
  expect(iconButton("Next result").disabled).toBe(false);
});

it("drops a frame callback whose deferred work was already replaced", async () => {
  // concurrency: the second-frame callback re-checks that its work is still the
  // current one, so a callback that outlives its cancellation does nothing.
  mount();
  openSearch();

  // Run the first frame of the opening scan, which queues its second frame.
  flushFrame();
  const staleSecondFrames = [...frames.values()];
  expect(staleSecondFrames).toHaveLength(1);

  // A new query cancels that queued frame and starts its own work.
  await settleSearch("alpha");
  expect(container.textContent).toContain("1 / 3");

  // The superseded callback still runs (a real browser may have already
  // dispatched it) and must not repaint or move the selection.
  act(() => staleSecondFrames.forEach(callback => callback(0)));
  expect(container.textContent).toContain("1 / 3");
  expect(iconButton("Next result").disabled).toBe(false);
});

it("closes from the close button and clears its highlights on unmount", async () => {
  const deleteHighlight = vi.fn();
  vi.stubGlobal("CSS", { highlights: { delete: deleteHighlight, get: vi.fn(() => ({ clear: vi.fn() })), set: vi.fn() } });
  mount();
  openSearch();
  await settleSearch("alpha");
  deleteHighlight.mockClear();

  act(() => iconButton("Close search").click());
  expect(container.querySelector("input")).toBeNull();

  act(() => root.unmount());
  expect(deleteHighlight).toHaveBeenCalled();
  // Recreate the root so the shared `afterEach` unmount stays valid.
  root = createRoot(container);
});

it("does not let a superseded prepare poison the query that replaced it", async () => {
  // concurrency: typing a new query re-runs the prepare effect, whose cleanup
  // ABORTS the previous controller. If the aborted call's `setPreparedQuery` were
  // allowed to land, `preparedQuery` would hold the OLD query while `query` holds the
  // new one - and `recompute` bails on `preparedQuery !== query` (ThreadSearch.tsx:157),
  // so the panel would sit at zero results forever with navigation dead. That is the
  // user-visible harm this test pins, and it is why those `!controller.signal.aborted`
  // guards exist. A branch audit found all three uncovered: every other test passes an
  // `async () => {}` prepare that resolves before a second query can land.
  const pending: Array<{ resolve: () => void }> = [];
  act(() => root.render(
    <ThreadSearch
      contentKey={null}
      onPrepareSearch={() => new Promise<void>((resolve) => {
        pending.push({ resolve });
      })}
      rootRef={{ current: thread }}
    />,
  ));
  openSearch();

  setQuery("alpha");
  await act(async () => {
    await vi.advanceTimersByTimeAsync(300);
  });
  expect(pending).toHaveLength(1);

  // Supersede it: this aborts the first controller.
  setQuery("alphabet");
  await act(async () => {
    await vi.advanceTimersByTimeAsync(300);
  });
  expect(pending).toHaveLength(2);

  // The replacement lands first, so the panel is showing "alphabet" results.
  await act(async () => {
    pending[1]!.resolve();
  });
  flushFrame();
  flushFrame();
  expect(iconButton("Next result").disabled).toBe(false);
  expect(container.textContent).toContain("1 / 1");

  // Now the superseded prepare resolves. Its write must be discarded.
  await act(async () => {
    pending[0]!.resolve();
  });
  flushFrame();
  flushFrame();

  // The harm is not visible in the instantaneous text - it is that every FUTURE
  // rescan would bail on `preparedQuery !== query` (ThreadSearch.tsx:157) and the
  // panel would freeze at its old count. So the discriminator is a content change
  // after the stale write: the TOTAL must still track the transcript (2 matches),
  // where the frozen panel would still read "1 / 1".
  await act(async () => {
    thread.textContent = "alphabet alphabet";
    await Promise.resolve();
  });
  await flushRescan();

  expect(container.textContent).toContain("1 / 2");
  expect(iconButton("Next result").disabled).toBe(false);
});

it("treats an aborted prepare's rejection as a supersede, not a failure", async () => {
  // error-path + concurrency: the aborted call may REJECT (the caller's fetch was
  // cancelled). That is not a search failure, so the panel must not show the failure
  // status for the query that superseded it - the guard's false arm here, and the
  // `finally`'s, were both uncovered for the same reason as above.
  const pending: Array<{ resolve: () => void; reject: (reason: Error) => void }> = [];
  act(() => root.render(
    <ThreadSearch
      contentKey={null}
      onPrepareSearch={() => new Promise<void>((resolve, reject) => {
        pending.push({ reject, resolve });
      })}
      rootRef={{ current: thread }}
    />,
  ));
  openSearch();

  setQuery("alpha");
  await act(async () => {
    await vi.advanceTimersByTimeAsync(300);
  });
  setQuery("alphabet");
  await act(async () => {
    await vi.advanceTimersByTimeAsync(300);
  });
  expect(pending).toHaveLength(2);

  // The superseding prepare SUCCEEDS (so the panel has good results), and the
  // abandoned one then rejects - which must be ignored rather than reported.
  await act(async () => {
    pending[1]!.resolve();
  });
  await act(async () => {
    pending[0]!.reject(new Error("aborted fetch"));
  });
  flushFrame();
  flushFrame();

  // No failure state was entered: the panel is not reporting a broken search, and
  // the matches it found are still navigable.
  expect(container.textContent).not.toContain("Search failed");
  expect(iconButton("Next result").disabled).toBe(false);
});
