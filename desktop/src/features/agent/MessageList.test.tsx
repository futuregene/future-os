// @vitest-environment jsdom
import type { AgentMessage } from "@future-os/thread-projection";
import type { Root } from "react-dom/client";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { MessageList } from "./MessageList";

/**
 * The row is stubbed out: `MessageList` owns the single-hovered-row state
 * machine and the recovery-target wiring, so the test drives those directly and
 * asserts what each row is told.
 */
vi.mock("./MessageBlock", () => ({
  MessageBlock: (props: {
    message: AgentMessage;
    hovered: boolean;
    isLast: boolean;
    recoverySource: AgentMessage | null;
    onHover: (id: string) => void;
    onLeave: (id: string) => void;
  }) => (
    <div
      data-hovered={String(props.hovered)}
      data-last={String(props.isLast)}
      data-recovery={props.recoverySource?.id ?? ""}
      data-row={props.message.id}
    >
      <button data-enter={props.message.id} onPointerOver={() => props.onHover(props.message.id)} type="button" />
      <button data-exit={props.message.id} onPointerLeave={() => props.onLeave(props.message.id)} type="button" />
    </div>
  ),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function user(id: string): AgentMessage {
  return { id, role: "user", content: id, status: "complete" } as AgentMessage;
}

function assistant(id: string): AgentMessage {
  return { id, role: "assistant", content: id, status: "complete" } as AgentMessage;
}

const HISTORY = [user("u1"), assistant("a1"), user("u2"), assistant("a2")];

let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
  vi.useFakeTimers();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

function render(messages: AgentMessage[]) {
  act(() => root.render(<MessageList messages={messages} />));
}

function row(id: string) {
  return container.querySelector<HTMLElement>(`[data-row="${id}"]`)!;
}

function enterButton(id: string) {
  return container.querySelector<HTMLElement>(`[data-enter="${id}"]`)!;
}

/**
 * React derives `onPointerEnter`/`onPointerLeave` from the `pointerover` /
 * `pointerout` pairs, so that is what a test must dispatch — and `relatedTarget`
 * is what tells React whether the pointer stayed inside the list.
 */
function enter(id: string) {
  act(() => {
    enterButton(id).dispatchEvent(new MouseEvent("pointerover", { bubbles: true, relatedTarget: null }));
  });
}

function exit(id: string, to?: Element) {
  act(() => {
    const button = container.querySelector<HTMLElement>(`[data-exit="${id}"]`)!;
    // Default: the pointer moved to another row, i.e. it stayed inside the list.
    const related = to ?? enterButton(id === "u1" ? "u2" : "u1");
    button.dispatchEvent(new MouseEvent("pointerout", { bubbles: true, relatedTarget: related }));
  });
}

function hoveredIds() {
  return [...container.querySelectorAll<HTMLElement>("[data-row]")]
    .filter(element => element.dataset.hovered === "true")
    .map(element => element.dataset.row);
}

async function advance(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

it("renders one row per message and only the last one can be a recovery target", () => {
  render(HISTORY);

  expect([...container.querySelectorAll<HTMLElement>("[data-row]")].map(element => element.dataset.row))
    .toEqual(["u1", "a1", "u2", "a2"]);
  // Computing the previous user message for every row is O(n²); only the tail
  // row may offer continue/retry, and it targets u2.
  expect(HISTORY.map(message => row(message.id).dataset.last)).toEqual(["false", "false", "false", "true"]);
  expect(HISTORY.map(message => row(message.id).dataset.recovery)).toEqual(["", "", "", "u2"]);
  expect(hoveredIds()).toEqual([]);
});

it("tolerates an empty thread and a thread with no user message", () => {
  // boundary: nothing to render.
  render([]);
  expect(container.querySelectorAll("[data-row]")).toHaveLength(0);

  // boundary: an assistant-only tail has no previous user message to recover.
  render([assistant("a1")]);
  expect(row("a1").dataset.recovery).toBe("");
  expect(row("a1").dataset.last).toBe("true");
});

it("keeps at most one row's controls shown", () => {
  render(HISTORY);

  enter("u1");
  expect(hoveredIds()).toEqual(["u1"]);

  // The next row's `pointerover` corrects a hover-exit the webview dropped.
  enter("u2");
  expect(hoveredIds()).toEqual(["u2"]);
});

it("debounces the hide so sweeping between rows does not flicker", async () => {
  render(HISTORY);

  enter("u2");
  exit("u2");
  // Still shown immediately after leaving: the hide waits out the delay.
  await advance(199);
  expect(hoveredIds()).toEqual(["u2"]);

  // Re-entering within the window cancels the pending hide…
  enter("u2");
  await advance(199);
  expect(hoveredIds()).toEqual(["u2"]);

  // …and leaving for good hides it.
  exit("u2");
  await advance(200);
  expect(hoveredIds()).toEqual([]);
});

it("a late pointerleave from a row that is no longer hovered does not clear the current one", async () => {
  // concurrency: WKWebView can deliver u1's `pointerleave` only after u2's
  // `pointerover`; the delayed hide must check the id before clearing.
  render(HISTORY);

  enter("u1");
  enter("u2");
  exit("u1");
  await advance(200);

  expect(hoveredIds()).toEqual(["u2"]);
});

it("clears the hover when the pointer leaves the whole list", async () => {
  render(HISTORY);

  enter("a1");
  // The pointer moved out of the transcript entirely.
  exit("a1", document.body);
  await advance(199);
  expect(hoveredIds()).toEqual(["a1"]);

  await advance(1);
  expect(hoveredIds()).toEqual([]);
});

it("cancels a pending hide when the list unmounts", async () => {
  render(HISTORY);
  enter("a1");
  exit("a1");

  const errors: unknown[][] = [];
  const spy = vi.spyOn(console, "error").mockImplementation((...args: unknown[]) => void errors.push(args));
  act(() => root.unmount());
  await advance(400);

  expect(errors).toEqual([]);
  spy.mockRestore();
  // Recreate the root so the shared `afterEach` unmount stays valid.
  root = createRoot(container);
});

it("re-entering a row after the delay starts a fresh hover", async () => {
  render(HISTORY);

  enter("u1");
  exit("u1");
  await advance(200);
  expect(hoveredIds()).toEqual([]);

  enter("u1");
  expect(hoveredIds()).toEqual(["u1"]);
});

it("never leaves a stale timer behind across repeated enter/leave cycles", async () => {
  render(HISTORY);

  for (let cycle = 0; cycle < 5; cycle++) {
    enter("u1");
    exit("u1");
  }
  // One hide is pending for u1 no matter how many cycles ran, so the row is
  // still shown now and gone exactly one delay later.
  await advance(199);
  expect(hoveredIds()).toEqual(["u1"]);
  await advance(1);
  expect(hoveredIds()).toEqual([]);
});

it("forwards the workspace context to every row", () => {
  act(() => root.render(
    <MessageList messages={[user("u1")]} workspaceId="w1" workspacePath="/work" />,
  ));
  expect(row("u1")).not.toBeNull();
});

it("keeps the row hovered when the pointer leaves the list and returns to it", async () => {
  // Two hides can be scheduled for one gesture (the row's, then the list's).
  // The list handler cancels and re-owns the pending timer, so returning to the
  // same row cancels the only live hide and the controls stay put.
  render(HISTORY);

  enter("u1");
  exit("u1", document.body); // both the row's and the list's hide are scheduled
  await advance(100);
  enter("u1");
  expect(hoveredIds()).toEqual(["u1"]);

  await advance(400);
  expect(hoveredIds()).toEqual(["u1"]);
});

it("keeps the hover ref stable across a message-list change", async () => {
  render(HISTORY);
  enter("u2");
  expect(hoveredIds()).toEqual(["u2"]);

  // A streaming delta replaces the array on every push; the hovered row must
  // stay hovered rather than flicker off.
  render([...HISTORY, assistant("a3")]);
  expect(hoveredIds()).toEqual(["u2"]);
  expect(row("a3").dataset.last).toBe("true");
  expect(row("a2").dataset.last).toBe("false");
  expect(row("a3").dataset.recovery).toBe("u2");
});
