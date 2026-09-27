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
    // The box empties on submit, not on delivery: the conversation shows the
    // message optimistically, so a pending delivery must not leave a second
    // copy of it in the composer.
    expect(editor.textContent).toBe("");
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

it("empties the box before delivery settles, so the message never shows twice", async () => {
  // regression: the thread's optimistic bubble appears as soon as the send
  // pipeline starts, while clearing used to wait for the agent's acceptance
  // handshake — leaving the same message in the thread and in the composer for
  // as long as that handshake took.
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const delivery = deferred();
  const onSend = vi.fn(() => delivery.promise);
  try {
    await act(async () => root.render(<Composer onSend={onSend} modelOptions={[]} />));
    const editor = host.querySelector<HTMLElement>("[role=textbox]")!;
    act(() => {
      editor.textContent = "hello there";
      editor.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    // Same tick as the submit — the delivery promise is still pending here.
    expect(onSend).toHaveBeenCalledWith({ attachments: [], content: "hello there" });
    expect(editor.textContent).toBe("");
    await act(async () => {
      delivery.resolve();
      await delivery.promise;
    });
    expect(editor.textContent).toBe("");
  }
  finally {
    act(() => root.unmount());
    host.remove();
  }
});

it("restores the submitted draft, and its stored copy, when delivery is refused", async () => {
  // error-path: the hand-over is optimistic, so a rejection has to undo it —
  // the message must be editable again, and a reload of this conversation must
  // find it.
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<Composer draftKey="T1" onSend={() => Promise.reject(new Error("already running"))} modelOptions={[]} />));
    const editor = host.querySelector<HTMLElement>("[role=textbox]")!;
    act(() => {
      editor.textContent = "keep me";
      editor.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(editor.textContent).toBe("keep me");
    expect(sessionStorage.getItem("composer-draft:T1")).toContain("keep me");
  }
  finally {
    act(() => root.unmount());
    host.remove();
    sessionStorage.clear();
  }
});

it("keeps a draft typed after the send instead of restoring the refused message", async () => {
  // concurrency: the user started the next message while the delivery was in
  // flight, so their draft wins — the refused text stays readable in the thread.
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const delivery = deferred();
  try {
    await act(async () => root.render(<Composer onSend={() => delivery.promise} modelOptions={[]} />));
    const editor = host.querySelector<HTMLElement>("[role=textbox]")!;
    act(() => {
      editor.textContent = "first message";
      editor.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    act(() => {
      editor.textContent = "my new draft";
      editor.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {
      delivery.reject(new Error("delivery failed"));
      await delivery.promise.catch(() => {});
    });
    expect(editor.textContent).toBe("my new draft");
  }
  finally {
    act(() => root.unmount());
    host.remove();
  }
});

it("does not restore a refused message into the conversation the user switched to", async () => {
  // concurrency: the send belonged to T1 while the composer now holds T2, so
  // T1's refusal must not paste its message into T2.
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const delivery = deferred();
  try {
    await act(async () => root.render(<Composer draftKey="T1" onSend={() => delivery.promise} modelOptions={[]} />));
    const editor = host.querySelector<HTMLElement>("[role=textbox]")!;
    act(() => {
      editor.textContent = "T1 message";
      editor.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    await act(async () => root.render(<Composer draftKey="T2" onSend={() => delivery.promise} modelOptions={[]} />));
    await act(async () => {
      delivery.reject(new Error("delivery failed"));
      await delivery.promise.catch(() => {});
    });
    expect(editor.textContent).toBe("");
    expect(sessionStorage.getItem("composer-draft:T2")).toBeNull();
  }
  finally {
    act(() => root.unmount());
    host.remove();
    sessionStorage.clear();
  }
});
