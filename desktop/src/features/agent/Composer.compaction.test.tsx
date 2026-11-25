/**
 * A manual compaction owns the session for as long as the summary takes — minutes
 * on a long conversation — and the Agent refuses the mutating commands that would
 * change the transcript it is summarising. The send button stays live for that
 * whole window anyway, and a click on it is taken rather than dropped: the draft
 * stays in the box, the button reads busy, and the words that were there when it
 * was pressed leave the moment the fence opens. The four endings that are NOT a
 * send (edited, switched, another compaction, window closed) all keep the draft.
 *
 * The phone ships the same behaviour (`useComposerSubmission`), and these tests
 * pin the two endings the ticket calls out as the regression risk: an edited
 * draft and a conversation switch must not be delivered by a click the reader
 * made about different text, in a different conversation.
 */
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

function submit(host: HTMLElement) {
  act(() => {
    host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  });
}

/** The one control-row line that explains what the composer is waiting for. */
function statusLine(host: HTMLElement) {
  return host.querySelector("[role=status]")?.textContent ?? null;
}

it("takes a click during a compaction, and sends the same words when it ends", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const onSend = vi.fn(async () => {});
  try {
    await act(async () => root.render(<Composer compactionInProgress modelOptions={[]} onSend={onSend} />));
    const editor = host.querySelector<HTMLElement>("[role=textbox]")!;
    // Compaction alone is not a reason to disable the button: the click is the
    // one thing that can hand the message over, and it is held instead.
    expect(host.querySelector<HTMLButtonElement>("button[type=submit]")!.disabled).toBe(true);
    type(editor, "what I want to ask next");
    expect(host.querySelector<HTMLButtonElement>("button[type=submit]")!.disabled).toBe(false);
    expect(statusLine(host)).toBe("Compacting context…");

    submit(host);
    // Taken, not refused: nothing has left, the words are still in the box, and
    // the button now reads busy with the reason beside it.
    expect(onSend).not.toHaveBeenCalled();
    expect(editor.textContent).toBe("what I want to ask next");
    expect(statusLine(host)).toBe("Sending when compaction finishes…");
    const held = host.querySelector<HTMLButtonElement>("button[type=submit]")!;
    expect(held.disabled).toBe(true);
    expect(held.getAttribute("aria-busy")).toBe("true");

    // The summary ends: the click goes out, with exactly the words it carried.
    await act(async () => root.render(<Composer compactionInProgress={false} modelOptions={[]} onSend={onSend} />));
    expect(onSend).toHaveBeenCalledTimes(1);
    expect(onSend).toHaveBeenCalledWith({ attachments: [], content: "what I want to ask next" });
    expect(editor.textContent).toBe("");
    expect(statusLine(host)).toBeNull();
  }
  finally {
    act(() => root.unmount());
    host.remove();
  }
});

it("does not send a draft the reader edited while the compaction ran", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const onSend = vi.fn(async () => {});
  try {
    await act(async () => root.render(<Composer compactionInProgress modelOptions={[]} onSend={onSend} />));
    const editor = host.querySelector<HTMLElement>("[role=textbox]")!;
    type(editor, "the first version");
    submit(host);
    expect(onSend).not.toHaveBeenCalled();

    // The reader keeps writing — this is their draft now, and nobody asked for
    // the earlier sentence any more.
    type(editor, "the first version, revised");
    await act(async () => root.render(<Composer compactionInProgress={false} modelOptions={[]} onSend={onSend} />));
    expect(onSend).not.toHaveBeenCalled();
    expect(editor.textContent).toBe("the first version, revised");
    expect(statusLine(host)).toBeNull();
  }
  finally {
    act(() => root.unmount());
    host.remove();
  }
});

it("does not send a held click into the conversation the reader switched to", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const onSend = vi.fn(async () => {});
  try {
    await act(async () => root.render(<Composer compactionInProgress draftKey="T1" modelOptions={[]} onSend={onSend} />));
    const editor = host.querySelector<HTMLElement>("[role=textbox]")!;
    type(editor, "T1's next question");
    submit(host);
    expect(onSend).not.toHaveBeenCalled();
    expect(sessionStorage.getItem("composer-draft:T1")).toContain("T1's next question");

    // Another conversation: its box is empty, and the wait for T1's compaction
    // does not follow the reader here.
    await act(async () => root.render(<Composer compactionInProgress draftKey="T2" modelOptions={[]} onSend={onSend} />));
    await act(async () => root.render(<Composer compactionInProgress={false} draftKey="T2" modelOptions={[]} onSend={onSend} />));
    expect(onSend).not.toHaveBeenCalled();
    expect(editor.textContent).toBe("");
    expect(sessionStorage.getItem("composer-draft:T2")).toBeNull();
    // T1's message still waits where it was written, in its own draft.
    expect(sessionStorage.getItem("composer-draft:T1")).toContain("T1's next question");
  }
  finally {
    act(() => root.unmount());
    host.remove();
    sessionStorage.clear();
  }
});

it("keeps waiting through a second click and a longer compaction", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const onSend = vi.fn(async () => {});
  try {
    await act(async () => root.render(<Composer compactionInProgress modelOptions={[]} onSend={onSend} />));
    const editor = host.querySelector<HTMLElement>("[role=textbox]")!;
    type(editor, "one message only");
    submit(host);
    // A click on the busy button, and a compaction that runs on: neither is a
    // second message nor an early release.
    submit(host);
    await act(async () => root.render(<Composer compactionInProgress modelOptions={[]} onSend={onSend} />));
    expect(onSend).not.toHaveBeenCalled();
    expect(statusLine(host)).toBe("Sending when compaction finishes…");

    await act(async () => root.render(<Composer compactionInProgress={false} modelOptions={[]} onSend={onSend} />));
    expect(onSend).toHaveBeenCalledTimes(1);
  }
  finally {
    act(() => root.unmount());
    host.remove();
  }
});

it("sends nothing when the window closes during the wait, and keeps the draft", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const onSend = vi.fn(async () => {});
  try {
    await act(async () => root.render(<Composer compactionInProgress draftKey="T1" modelOptions={[]} onSend={onSend} />));
    const editor = host.querySelector<HTMLElement>("[role=textbox]")!;
    type(editor, "written while it compacted");
    submit(host);
    expect(onSend).not.toHaveBeenCalled();
    // The wait is local, not the Agent's queue: closing the window ends it
    // without handing anything over, and the draft survives for the next visit.
    act(() => root.unmount());
    expect(onSend).not.toHaveBeenCalled();
    expect(sessionStorage.getItem("composer-draft:T1")).toContain("written while it compacted");
  }
  finally {
    host.remove();
    sessionStorage.clear();
  }
});
