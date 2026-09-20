// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { Composer } from "./Composer";

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }),
}));
vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: vi.fn(async (command: string) => command === "list_agent_providers" ? { builtin: [], custom: [] } : []),
}));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function deferred() {
  let resolve!: () => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<void>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

it("blocks button and Enter submission during external compaction, preserving the draft", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const onSend = vi.fn(async () => {});
  try {
    await act(async () => root.render(<Composer onSend={onSend} modelOptions={[]} compactionInProgress />));
    const editor = host.querySelector<HTMLElement>("[role=textbox]")!;
    // The disabled send button alone reads as "broken" — compaction runs for
    // minutes, so the composer must say what it is waiting for.
    expect(host.querySelector("[role=status]")?.textContent).toBe("Compacting context…");
    expect(host.querySelector<HTMLButtonElement>("button[type=submit]")!.title).toBe("Compacting context…");
    act(() => {
      editor.textContent = "keep my next message";
      editor.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(host.querySelector<HTMLButtonElement>("button[type=submit]")!.disabled).toBe(true);
    await act(async () => {
      host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
      editor.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    });
    expect(onSend).not.toHaveBeenCalled();
    expect(editor.textContent).toBe("keep my next message");
    await act(async () => root.render(<Composer onSend={onSend} modelOptions={[]} compactionInProgress={false} />));
    expect(host.querySelector("[role=status]")).toBeNull();
    expect(editor.textContent).toBe("keep my next message");
    expect(host.querySelector<HTMLButtonElement>("button[type=submit]")!.disabled).toBe(false);
    await act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(onSend).toHaveBeenCalledTimes(1);
  }
  finally {
    act(() => root.unmount());
    host.remove();
  }
});

it.each(["accepted", "rejected", "edited"] as const)("handles %s delivery without losing a draft", async (outcome) => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const delivery = deferred();
  const onSend = vi.fn(() => delivery.promise);
  try {
    await act(async () => root.render(<Composer onSend={onSend} modelOptions={[]} />));
    const editor = host.querySelector<HTMLElement>("[role=textbox]")!;
    act(() => {
      editor.textContent = "submitted prompt";
      editor.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(onSend).toHaveBeenCalledTimes(1);
    expect(editor.textContent).toBe("submitted prompt");
    if (outcome === "edited") {
      act(() => {
        editor.textContent = "my next draft";
        editor.dispatchEvent(new Event("input", { bubbles: true }));
      });
    }
    await act(async () => {
      if (outcome === "rejected")
        delivery.reject(new Error("delivery failed"));
      else
        delivery.resolve();
      await delivery.promise.catch(() => {});
    });
    expect(editor.textContent).toBe(outcome === "accepted" ? "" : outcome === "edited" ? "my next draft" : "submitted prompt");
  }
  finally {
    act(() => root.unmount());
    host.remove();
  }
});
