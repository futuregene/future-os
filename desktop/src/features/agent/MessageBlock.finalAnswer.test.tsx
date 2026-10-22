// @vitest-environment jsdom
import type { AgentMessage, MessageSegment } from "@future-os/thread-projection";
import type { Root } from "react-dom/client";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { setReplyDetail } from "../../lib/replyDisplay";
import { MessageBlock } from "./MessageBlock";

/**
 * The reading setting that keeps a settled reply to its final answer: the work
 * folds into one row above the answer, the answer stays, and the fold opens to
 * the same blocks a reader sees with the setting off. While the reply streams
 * nothing folds — the reader is watching it happen.
 */
const mocks = vi.hoisted(() => ({
  copyText: vi.fn(async () => {}),
  openPath: vi.fn(async () => {}),
}));

vi.mock("@tauri-apps/api/core", () => ({ convertFileSrc: (path: string) => `asset://${path}` }));
vi.mock("../../integrations/storage/threadStore", () => ({ openPath: mocks.openPath }));
vi.mock("../../lib/clipboard", () => ({ copyText: mocks.copyText }));
vi.mock("../filepreview/FilePreviewOverlay", () => ({ FilePreviewOverlay: () => <div data-preview="" /> }));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.clearAllMocks();
  setReplyDetail("detailed");
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  setReplyDetail("detailed");
});

function note(id: string, text: string): MessageSegment {
  return { id, kind: "text", text };
}

function thought(id: string): MessageSegment {
  return { id, kind: "thinking", text: "weighing the options" };
}

function call(id: string): MessageSegment {
  return { id, kind: "activity", item: { id, kind: "read", status: "completed", target: "relay.rs" } };
}

function reply(segments: MessageSegment[], over: Partial<AgentMessage> = {}): AgentMessage {
  return {
    authorKey: "author.researchCopilot",
    content: "",
    createdAt: new Date("2026-01-01T00:00:00Z").toISOString(),
    id: "m1",
    role: "assistant",
    segments,
    status: "complete",
    ...over,
  } as AgentMessage;
}

// The shape the setting exists for: notes and calls through the task, the
// answer last.
const workedReply = reply([
  note("note", "I will look at the relay implementation first."),
  thought("thought"),
  call("call"),
  note("answer", "The plaintext relay works end to end."),
]);

function render(msg: AgentMessage, over: Partial<Parameters<typeof MessageBlock>[0]> = {}) {
  act(() => root.render(
    <MessageBlock
      dataMessageId={msg.id}
      hovered
      isLast
      message={msg}
      onHover={vi.fn()}
      onLeave={vi.fn()}
      {...over}
    />,
  ));
}

function processToggle() {
  return [...container.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => /process/i.test(button.textContent ?? "")) ?? null;
}

function text() {
  return container.textContent ?? "";
}

it("shows the whole reply when the setting is off", () => {
  render(workedReply);

  expect(text()).toContain("I will look at the relay implementation first.");
  expect(text()).toContain("The plaintext relay works end to end.");
  expect(processToggle()).toBeNull();
});

it("folds the work into one row above the final answer once the reply settles", () => {
  setReplyDetail("default");
  render(workedReply);

  expect(text()).not.toContain("I will look at the relay implementation first.");
  expect(text()).not.toContain("weighing the options");
  expect(text()).toContain("The plaintext relay works end to end.");
  const toggle = processToggle()!;
  expect(toggle.getAttribute("aria-expanded")).toBe("false");
  expect(toggle.textContent).toContain("Show process");
  expect(toggle.textContent).toContain("Tool calls 1×");
  expect(toggle.textContent).toContain("Thought 1×");
});

it("opens to the same blocks the setting hides, and closes again", () => {
  setReplyDetail("default");
  render(workedReply);

  const toggle = processToggle()!;
  act(() => toggle.click());

  expect(text()).toContain("I will look at the relay implementation first.");
  expect(text()).toContain("Thought");
  expect(processToggle()!.textContent).toContain("Hide process");

  act(() => processToggle()!.click());
  expect(text()).not.toContain("I will look at the relay implementation first.");
});

it("keeps the live reply whole while it streams", () => {
  setReplyDetail("default");
  render(reply(workedReply.segments!, { status: "streaming" }));

  expect(text()).toContain("I will look at the relay implementation first.");
  expect(processToggle()).toBeNull();
});

it("never folds a reply that has no answer to isolate", () => {
  const worked = reply([thought("thought"), call("call")]);
  render(worked);
  const unfolded = text();

  // Nothing to keep, so the reply stays exactly as the setting-off render left it.
  act(() => setReplyDetail("default"));
  render(worked);
  expect(processToggle()).toBeNull();
  expect(text()).toBe(unfolded);
});

it("copies the answer the row shows, not the prose the fold hides", async () => {
  setReplyDetail("default");
  render(workedReply);

  await act(async () => {
    container.querySelector<HTMLButtonElement>("button[title='Copy']")!.click();
    await Promise.resolve();
  });
  expect(mocks.copyText).toHaveBeenCalledWith("The plaintext relay works end to end.");
});
