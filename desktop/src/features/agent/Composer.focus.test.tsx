/**
 * Collapsing the terminal panel hands the caret back to the composer.
 *
 * Regression: the panel hides itself but focus stayed on the (now unmounted)
 * terminal, so the next keystrokes went nowhere.
 */
// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { emitFutureEvent } from "../../lib/futureEvents";
import { Composer } from "./Composer";

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }),
}));
vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: vi.fn(async (command: string) => command === "list_agent_providers" ? { builtin: [], custom: [] } : []),
}));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function render(onSend: () => void, disabled: boolean) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  return {
    container,
    render: async () => {
      await act(async () => root.render(
        <Composer disabled={disabled} modelOptions={[]} onSend={onSend} />,
      ));
    },
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

/** The editor is the only contentEditable element the composer renders. */
function editorOf(container: HTMLElement): HTMLElement {
  const editor = container.querySelector<HTMLElement>("[contenteditable]");
  expect(editor).not.toBeNull();
  return editor!;
}

it("focuses the editor when the terminal panel collapses", async () => {
  const harness = render(vi.fn(), false);
  try {
    await harness.render();
    const editor = editorOf(harness.container);
    editor.blur();
    expect(document.activeElement).not.toBe(editor);

    act(() => emitFutureEvent("focus-composer", undefined));
    expect(document.activeElement).toBe(editor);
  }
  finally {
    harness.unmount();
  }
});

it("declines while the composer cannot hold a caret", async () => {
  // `disabled` makes the editor contentEditable=false; focusing it would put a
  // caret nowhere, so the event is ignored rather than stealing scroll.
  const harness = render(vi.fn(), true);
  try {
    await harness.render();
    const editor = editorOf(harness.container);
    editor.blur();

    act(() => emitFutureEvent("focus-composer", undefined));
    expect(document.activeElement).not.toBe(editor);
  }
  finally {
    harness.unmount();
  }
});
