// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { Composer } from "./Composer";

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }),
}));
vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: vi.fn(async (command: string) => command === "list_agent_providers" ? { builtin: [], custom: [] } : []),
}));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function type(editor: HTMLElement, value: string) {
  act(() => {
    editor.textContent = value;
    editor.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

it("keeps the send button as Stop while a reply streams and the box is empty", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const onAbort = vi.fn();
  try {
    await act(async () => root.render(
      <Composer modelOptions={[]} onAbort={onAbort} onEnqueue={() => true} onSend={vi.fn()} sending />,
    ));
    // No draft: the one thing this slot can offer is interrupting the run.
    const stop = host.querySelector<HTMLButtonElement>("button[aria-label='Stop']")!;
    expect(stop).not.toBeNull();
    expect(host.querySelector("button[aria-label='Choose what to do with this message']")).toBeNull();
    act(() => stop.click());
    expect(onAbort).toHaveBeenCalledTimes(1);
  }
  finally {
    act(() => root.unmount());
    host.remove();
  }
});

it("asks what a send means while a reply streams, and queues the draft", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const onSend = vi.fn();
  const onAbort = vi.fn();
  const onEnqueue = vi.fn(() => true);
  try {
    await act(async () => root.render(
      <Composer modelOptions={[]} onAbort={onAbort} onEnqueue={onEnqueue} onSend={onSend} sending />,
    ));
    const editor = host.querySelector<HTMLElement>("[role=textbox]")!;
    type(editor, "the next question");
    // The draft turns the button into a question instead of a Stop that would
    // end the run and discard what was typed.
    expect(host.querySelector("button[aria-label='Stop']")).toBeNull();
    const choose = host.querySelector<HTMLButtonElement>("button[aria-label='Choose what to do with this message']")!;
    act(() => choose.click());
    expect(host.textContent).toContain("Add to the send queue");
    expect(host.textContent).toContain("Interrupt and send");

    const queueRow = [...host.querySelectorAll("button")].find(button => button.textContent?.includes("Add to the send queue"))!;
    act(() => queueRow.click());
    expect(onEnqueue).toHaveBeenCalledWith({ attachments: [], content: "the next question" });
    expect(onAbort).not.toHaveBeenCalled();
    expect(onSend).not.toHaveBeenCalled();
    // Handed over: the box gives it up, exactly as a delivered message does.
    expect(editor.textContent).toBe("");
  }
  finally {
    act(() => root.unmount());
    host.remove();
  }
});

it("interrupts the run and hands the draft to the queue when that row is picked", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const onAbort = vi.fn();
  const onEnqueue = vi.fn(() => true);
  try {
    await act(async () => root.render(
      <Composer modelOptions={[]} onAbort={onAbort} onEnqueue={onEnqueue} onSend={vi.fn()} queueCount={1} sending />,
    ));
    const editor = host.querySelector<HTMLElement>("[role=textbox]")!;
    type(editor, "and this too");
    // A queue behind it changes the wording: the interrupt sends the queue and
    // this message as one.
    act(() => host.querySelector<HTMLButtonElement>("button[aria-label='Choose what to do with this message']")!.click());
    expect(host.textContent).toContain("Interrupt and send all");
    const interruptRow = [...host.querySelectorAll("button")].find(button => button.textContent?.includes("Interrupt and send all"))!;
    act(() => interruptRow.click());
    // Queued first, interrupted second: the flush that follows the run sends
    // what was on screen.
    expect(onEnqueue).toHaveBeenCalledWith({ attachments: [], content: "and this too" });
    expect(onAbort).toHaveBeenCalledTimes(1);
    expect(onEnqueue.mock.invocationCallOrder[0]!).toBeLessThan(onAbort.mock.invocationCallOrder[0]!);
    expect(editor.textContent).toBe("");
  }
  finally {
    act(() => root.unmount());
    host.remove();
  }
});

it("refuses to take a draft the queue cannot hold", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const onEnqueue = vi.fn(() => false);
  try {
    await act(async () => root.render(
      <Composer modelOptions={[]} onEnqueue={onEnqueue} onSend={vi.fn()} sending />,
    ));
    const editor = host.querySelector<HTMLElement>("[role=textbox]")!;
    type(editor, "keep me");
    act(() => host.querySelector<HTMLButtonElement>("button[aria-label='Choose what to do with this message']")!.click());
    const queueRow = [...host.querySelectorAll("button")].find(button => button.textContent?.includes("Add to the send queue"))!;
    act(() => queueRow.click());
    expect(onEnqueue).toHaveBeenCalledTimes(1);
    // Refused: the text stays where the reader can see it.
    expect(editor.textContent).toBe("keep me");
  }
  finally {
    act(() => root.unmount());
    host.remove();
  }
});

it("keeps sending directly when no queue is wired", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const onSend = vi.fn();
  try {
    await act(async () => root.render(<Composer modelOptions={[]} onSend={onSend} sending />));
    const editor = host.querySelector<HTMLElement>("[role=textbox]")!;
    type(editor, "draft during a run");
    // No queue here: the old rule stands — Stop, and the draft waits.
    expect(host.querySelector("button[aria-label='Stop']")).not.toBeNull();
    expect(host.querySelector("button[aria-label='Choose what to do with this message']")).toBeNull();
    await act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(onSend).not.toHaveBeenCalled();
    expect(editor.textContent).toBe("draft during a run");
  }
  finally {
    act(() => root.unmount());
    host.remove();
  }
});
