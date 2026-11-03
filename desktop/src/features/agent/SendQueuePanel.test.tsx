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

it("offers a parked row a start, and only a parked one", () => {
  // The Agent runs a `queued` row by itself at the turn boundary; a `stale` one
  // waits for a person, so that row — and only that row — carries the control.
  const onRelease = vi.fn();
  const panel = renderPanel({
    items: [
      { attachmentCount: 0, content: "will run by itself", id: "a" },
      { attachmentCount: 0, content: "waited too long", id: "b", parked: true },
    ],
    onRelease,
  });
  try {
    const releases = panel.host.querySelectorAll("[data-queue-release]");
    expect(releases).toHaveLength(1);
    act(() => (releases[0] as HTMLButtonElement).click());
    expect(onRelease).toHaveBeenCalledWith("b");
  }
  finally {
    panel.cleanup();
  }
});

it("hides the start control when the queue cannot be started by hand", () => {
  // A client-side queue (a paired computer's, or the phone's) has no Agent row
  // behind a parked flag, and must not show a button that does nothing.
  const panel = renderPanel({
    items: [{ attachmentCount: 0, content: "waiting", id: "a", parked: true }],
  });
  try {
    expect(panel.host.querySelector("[data-queue-release]")).toBeNull();
  }
  finally {
    panel.cleanup();
  }
});

/**
 * A row read back from a session's queue carries text and an acceptance time,
 * never the files it was sent with. Claiming "0" there told the reader a message
 * was text-only when it may have carried files, and an edit — which is a removal
 * and a re-send, the only kind there is — then dropped them with nobody warned.
 */
it("says what an edit costs when the queue does not report a message's files", () => {
  const panel = renderPanel({
    attachmentsUnreported: true,
    items: [{ content: "look at this", id: "a" }],
  });
  try {
    // No invented count on the row…
    expect(panel.host.textContent).not.toContain("Attachments:");
    act(() => panel.host.querySelector<HTMLButtonElement>("[data-queue-edit]")!.click());
    // …and the trade is named where it is made, before the save.
    expect(panel.host.textContent).toContain("Saving sends the new text only");
    expect(panel.props.onUpdate).not.toHaveBeenCalled();
  }
  finally {
    panel.cleanup();
  }
});

it("does not offer an edit for a message that is only files", () => {
  // There is no text to correct, and an empty save is a removal: the bin beside
  // it already says that plainly.
  const panel = renderPanel({ items: [{ content: "", id: "files-only" }] });
  try {
    expect(panel.host.querySelector("[data-queue-edit]")).toBeNull();
    expect(panel.host.querySelector("[data-queue-remove]")).not.toBeNull();
    expect(panel.host.textContent).toContain("Attachments only");
  }
  finally {
    panel.cleanup();
  }
});
