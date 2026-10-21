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

async function mount(options: { onEmptySubmit?: () => void; onSend?: (payload: unknown) => void } = {}) {
  await act(async () => root.render(
    <Composer
      modelOptions={[]}
      onEmptySubmit={options.onEmptySubmit}
      onSend={options.onSend ?? (() => {})}
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
    // Once the composition ends, the same key sends.
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
