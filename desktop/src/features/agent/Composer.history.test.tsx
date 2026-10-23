// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { Composer } from "./Composer";

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }),
}));
vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: vi.fn(async (command: string) => command === "list_agent_providers" ? { builtin: [], custom: [] } : []),
}));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

beforeEach(() => {
  sessionStorage.clear();
});

/** Type into the editor the way a user does (the editor is uncontrolled DOM). */
function type(editor: HTMLElement, text: string) {
  act(() => {
    editor.textContent = text;
    editor.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

/** Press a bare key in the editor. */
function key(editor: HTMLElement, name: string, over: KeyboardEventInit = {}) {
  const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: name, ...over });
  act(() => {
    editor.dispatchEvent(event);
  });
  return event;
}

async function send(host: HTMLElement) {
  await act(async () => {
    host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    await Promise.resolve();
  });
}

it("puts the conversation's sent messages back on ↑/↓", async () => {
  // The Codex/OpenCode `history_previous` behaviour: after sending a correction,
  // the paragraph it corrected is one key away.
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const onSend = vi.fn();
  try {
    await act(async () => root.render(<Composer draftKey="thread-1" modelOptions={[]} onSend={onSend} />));
    const editor = host.querySelector<HTMLElement>("[role=textbox]")!;

    type(editor, "the first prompt");
    await send(host);
    type(editor, "the second prompt");
    await send(host);
    expect(editor.textContent).toBe("");

    key(editor, "ArrowUp");
    expect(editor.textContent).toBe("the second prompt");
    key(editor, "ArrowUp");
    expect(editor.textContent).toBe("the first prompt");
    key(editor, "ArrowDown");
    expect(editor.textContent).toBe("the second prompt");
    // Past the newest: the box empties, and the arrows are the editor's again.
    key(editor, "ArrowDown");
    expect(editor.textContent).toBe("");
  }
  finally {
    act(() => root.unmount());
    host.remove();
  }
});

it("leaves the arrows to the caret while the box has text", () => {
  // A press in a box the reader is typing in is a caret move, not a recall —
  // taking it would make the arrow keys unusable for editing.
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    act(() => root.render(<Composer draftKey="thread-1" modelOptions={[]} onSend={() => {}} />));
    const editor = host.querySelector<HTMLElement>("[role=textbox]")!;
    type(editor, "typed by hand");
    const event = key(editor, "ArrowUp");
    expect(event.defaultPrevented).toBe(false);
    expect(editor.textContent).toBe("typed by hand");
  }
  finally {
    act(() => root.unmount());
    host.remove();
  }
});

it("ends the walk as soon as the reader types", () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    act(() => root.render(<Composer draftKey="thread-1" modelOptions={[]} onSend={() => {}} />));
    const editor = host.querySelector<HTMLElement>("[role=textbox]")!;
    type(editor, "sent earlier");
    act(() => {
      host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    });
    key(editor, "ArrowUp");
    expect(editor.textContent).toBe("sent earlier");

    // Editing the recalled text is the reader taking it over: ↑ now moves the
    // caret instead of yanking the text away.
    type(editor, "sent earlier, plus a correction");
    const event = key(editor, "ArrowUp");
    expect(event.defaultPrevented).toBe(false);
  }
  finally {
    act(() => root.unmount());
    host.remove();
  }
});

it("a draft typed before ↑ is not restored by ↓", () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    act(() => root.render(<Composer draftKey="thread-1" modelOptions={[]} onSend={() => {}} />));
    const editor = host.querySelector<HTMLElement>("[role=textbox]")!;
    type(editor, "hello");
    act(() => {
      host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    });
    key(editor, "ArrowUp");
    expect(editor.textContent).toBe("hello");
    key(editor, "ArrowDown");
    expect(editor.textContent).toBe("");
  }
  finally {
    act(() => root.unmount());
    host.remove();
  }
});
