// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { SendQueuePanel } from "./SendQueuePanel";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function renderPanel(overrides: Partial<Parameters<typeof SendQueuePanel>[0]> = {}) {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const props = {
    items: [
      { attachmentCount: 0, content: "first message", id: "a" },
      { attachmentCount: 2, content: "", id: "b" },
    ],
    onRemove: vi.fn(),
    onRetry: vi.fn(),
    onUpdate: vi.fn(),
    ...overrides,
  };
  act(() => root.render(<SendQueuePanel {...props} />));
  return {
    host,
    props,
    cleanup: () => {
      act(() => root.unmount());
      host.remove();
    },
  };
}

it("lists what is waiting, with an attachment-only row said in words", () => {
  const panel = renderPanel();
  try {
    expect(panel.host.querySelector("[data-send-queue]")?.textContent).toContain("Send queue");
    expect(panel.host.querySelectorAll("[data-queue-item]")).toHaveLength(2);
    expect(panel.host.textContent).toContain("first message");
    // An entry whose words are gone still travels while it has files.
    expect(panel.host.textContent).toContain("Attachments only");
    expect(panel.host.textContent).toContain("Attachments: 2");
  }
  finally {
    panel.cleanup();
  }
});

it("edits a queued message in place and saves the new text", () => {
  const panel = renderPanel();
  try {
    act(() => panel.host.querySelector<HTMLButtonElement>("[data-queue-edit]")!.click());
    const field = panel.host.querySelector<HTMLTextAreaElement>("textarea")!;
    expect(field.value).toBe("first message");
    // A controlled field only updates through React's own value setter.
    const setValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
    act(() => {
      setValue.call(field, "second thoughts");
      field.dispatchEvent(new Event("input", { bubbles: true }));
    });
    act(() => panel.host.querySelector<HTMLButtonElement>("[data-queue-save]")!.click());
    expect(panel.props.onUpdate).toHaveBeenCalledWith("a", "second thoughts");
    expect(panel.host.querySelector("textarea")).toBeNull();
  }
  finally {
    panel.cleanup();
  }
});

it("takes a message back out of the queue", () => {
  const panel = renderPanel();
  try {
    act(() => panel.host.querySelectorAll<HTMLButtonElement>("[data-queue-remove]")[1]!.click());
    expect(panel.props.onRemove).toHaveBeenCalledWith("b");
    expect(panel.props.onUpdate).not.toHaveBeenCalled();
  }
  finally {
    panel.cleanup();
  }
});

it("offers a retry instead of the hint after a failed automatic send", () => {
  const panel = renderPanel({ failed: true });
  try {
    expect(panel.host.textContent).not.toContain("Sent as one message when this reply ends");
    act(() => panel.host.querySelector<HTMLButtonElement>("[data-queue-retry]")!.click());
    expect(panel.props.onRetry).toHaveBeenCalledTimes(1);
  }
  finally {
    panel.cleanup();
  }
});
