/**
 * The panel's send key, end to end through the real composer and editor.
 *
 * The panel puts "send" on `⏎` — a remote control has no other key for it — and
 * this test pins down the three properties that make it safe to put it there:
 *
 * 1. `⏎` over the editor sends the dictated sentence;
 * 2. a **composing** `⏎` belongs to the input method, so a dictation's `⏎`
 *    commits a candidate instead of sending half a sentence;
 * 3. an **empty** draft closes the layer, because a key that silently does
 *    nothing is a key the reader stops trusting.
 *
 * The key is the editor's own (`MentionEditor.onSubmit`), not a command in the
 * table — that is deliberate, and the reason is in `catalog.ts` beside
 * `vibe.input.close`: two owners on one press is how a message gets sent twice.
 */
// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { resetShortcutScopes } from "../keyboard/scopes";
import { resetAllCommandBindings } from "../keyboard/shortcuts";
import { Composer } from "./Composer";

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }),
}));
vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: vi.fn(async (command: string) => (command === "list_agent_providers" ? { builtin: [], custom: [] } : [])),
}));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLElement;
let root: ReturnType<typeof createRoot>;

async function mount(options: {
  controls?: "full" | "none";
  draftKey?: string;
  onEmptySubmit?: () => void;
  onQueue?: (payload: { content: string }) => boolean;
  onSend?: (payload: unknown) => void;
  sending?: boolean;
} = {}) {
  await act(async () => root.render(
    <Composer
      controls={options.controls}
      draftKey={options.draftKey}
      modelOptions={[]}
      onEmptySubmit={options.onEmptySubmit}
      onQueue={options.onQueue}
      onSend={options.onSend ?? (() => {})}
      sending={options.sending}
    />,
  ));
}

function editor(): HTMLElement {
  const element = container.querySelector<HTMLElement>("[contenteditable=\"true\"]");
  expect(element, "the composer's editor").not.toBeNull();
  return element!;
}

/** Type into the editor the way a dictation would: one inserted run of text. */
function dictate(text: string) {
  const element = editor();
  act(() => {
    element.focus();
    element.textContent = text;
    element.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: text }));
  });
}

function press(init: KeyboardEventInit) {
  act(() => {
    editor().dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
  });
}

beforeEach(() => {
  resetAllCommandBindings();
  resetShortcutScopes();
  container = document.body.appendChild(document.createElement("div"));
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  localStorage.clear();
  sessionStorage.clear();
});

describe("the input layer's control row", () => {
  /**
   * The composer's interactive controls, as the page names them: the approval
   * dropdown is a button titled "Approval", the model and thinking controls are
   * buttons in the same row, and the attach button has its own label.
   */
  function controlLabels(): string[] {
    return [...container.querySelectorAll("button")]
      .map(button => button.getAttribute("aria-label") ?? button.getAttribute("title") ?? "")
      .filter(Boolean);
  }

  it("draws them by default, which is every other composer in the app", async () => {
    await mount();
    // The attach button is the one control that exists whatever the model catalogue
    // says, so its absence would mean the row is gone rather than unpopulated.
    expect(container.textContent).not.toBe("");
    expect(container.querySelector("form")).not.toBeNull();
    expect(controlLabels().length).toBeGreaterThan(0);
  });

  it("draws none of them when asked, and keeps the editor", async () => {
    // The panel's input layer (§5.6): the values move into the menu and the layer
    // keeps one read-only line, so the row must be gone — not empty, gone.
    await mount({ controls: "none" });
    expect(editor()).not.toBeUndefined();
    expect(controlLabels()).toEqual([]);
  });
});

describe("a sentence said while the run is still going", () => {
  // The queue is the panel's (`vibeQueue.ts`); what the *composer* has to get right is
  // the two halves it owns: hand the text over and clear the box, or hand nothing over
  // and leave the text exactly where the reader can see it.
  it("hands the sentence over and clears the box when the queue accepts it", async () => {
    const onQueue = vi.fn(() => true);
    const onSend = vi.fn();
    await mount({ onQueue, onSend, sending: true });
    dictate("顺便把测试补一下");
    press({ code: "Enter", key: "Enter" });
    expect(onQueue).toHaveBeenCalledWith(expect.objectContaining({ content: "顺便把测试补一下" }));
    // Not sent: the whole point of holding it is that it is not on its way yet.
    expect(onSend).not.toHaveBeenCalled();
    // …and the box lets it go, because the text now lives in the queue (visible on the
    // cell), not in two places.
    expect(editor().textContent).toBe("");
  });

  it("keeps the text when one is already queued", async () => {
    // A refusal is "there is already one": the message must stay in front of the reader
    // rather than being swallowed by a second hand-over.
    const onQueue = vi.fn(() => false);
    const onSend = vi.fn();
    await mount({ onQueue, onSend, sending: true });
    dictate("第二句");
    press({ code: "Enter", key: "Enter" });
    expect(onQueue).toHaveBeenCalledTimes(1);
    expect(onSend).not.toHaveBeenCalled();
    expect(editor().textContent).toBe("第二句");
  });

  it("clears the stored draft once the queue has taken the whole sentence", async () => {
    // The box and the stored draft are two copies of the same sentence. Handing it to
    // the queue empties the box, so the stored copy has to go with it — otherwise a
    // relaunch (or another view reading the draft) brings back a sentence that is
    // already held, and the reader types it twice.
    sessionStorage.setItem("composer-draft:T9", JSON.stringify({ attachments: [], text: "held", version: 1 }));
    const onQueue = vi.fn(() => true);
    await mount({ draftKey: "T9", onQueue, sending: true });
    dictate("整句都交给队列");

    press({ code: "Enter", key: "Enter" });

    expect(onQueue).toHaveBeenCalledTimes(1);
    expect(editor().textContent).toBe("");
    expect(sessionStorage.getItem("composer-draft:T9")).toBeNull();
  });

  it("does not queue while the run is idle — it sends", async () => {
    const onQueue = vi.fn(() => true);
    const onSend = vi.fn();
    await mount({ onQueue, onSend, sending: false });
    dictate("正常发送");
    press({ code: "Enter", key: "Enter" });
    expect(onQueue).not.toHaveBeenCalled();
    expect(onSend).toHaveBeenCalledWith(expect.objectContaining({ content: "正常发送" }));
  });
});

describe("the input layer's send key", () => {
  it("sends a dictated draft with ⏎", async () => {
    const onSend = vi.fn();
    await mount({ onSend });
    dictate("把登录页的跳转修一下");
    press({ code: "Enter", key: "Enter" });
    expect(onSend).toHaveBeenCalledWith(expect.objectContaining({ content: "把登录页的跳转修一下" }));
  });

  it("leaves a composing ⏎ to the input method", async () => {
    // The key that commits a candidate (or ends a phrase) must never be the key
    // that sends half a sentence.
    const onSend = vi.fn();
    await mount({ onSend });
    dictate("半句");
    press({ code: "Enter", key: "Enter", isComposing: true });
    expect(onSend).not.toHaveBeenCalled();
    // Chromium's way of saying "this key belongs to the IME".
    press({ code: "Enter", key: "Enter", keyCode: 229 } as KeyboardEventInit);
    expect(onSend).not.toHaveBeenCalled();
    // Ending composition does not release the physical Enter.
    press({ code: "Enter", key: "Enter", repeat: true });
    expect(onSend).not.toHaveBeenCalled();
    act(() => {
      editor().dispatchEvent(new KeyboardEvent("keyup", { bubbles: true, code: "Enter", key: "Enter" }));
    });
    // Only a fresh press after release sends.
    press({ code: "Enter", key: "Enter" });
    expect(onSend).toHaveBeenCalledTimes(1);
  });

  it("keeps Shift+⏎ a newline rather than a send", async () => {
    // Still true in the input layer: a dictation that needs two paragraphs must
    // be able to make one, and this is the key everyone tries.
    const onSend = vi.fn();
    await mount({ onSend });
    dictate("第一段");
    press({ code: "Enter", key: "Enter", shiftKey: true });
    expect(onSend).not.toHaveBeenCalled();
  });

  it("closes the layer on an empty draft instead of doing nothing", async () => {
    const onEmptySubmit = vi.fn();
    const onSend = vi.fn();
    await mount({ onEmptySubmit, onSend });
    press({ code: "Enter", key: "Enter" });
    expect(onEmptySubmit).toHaveBeenCalledTimes(1);
    expect(onSend).not.toHaveBeenCalled();
  });

  it("does not send and does not close while disabled", async () => {
    // A conversation whose history cannot continue (`canContinue === false`) has a
    // disabled editor: `⏎` must not report a layer change it cannot honour.
    const onEmptySubmit = vi.fn();
    const onSend = vi.fn();
    await act(async () => root.render(
      <Composer disabled modelOptions={[]} onEmptySubmit={onEmptySubmit} onSend={onSend} />,
    ));
    act(() => {
      container.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, code: "Enter", key: "Enter" }));
    });
    expect(onEmptySubmit).not.toHaveBeenCalled();
    expect(onSend).not.toHaveBeenCalled();
  });
});
