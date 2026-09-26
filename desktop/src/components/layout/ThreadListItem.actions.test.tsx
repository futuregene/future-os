// @vitest-environment jsdom
import type { ComponentProps } from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ThreadListItem } from "./ThreadListItem";

vi.mock("../../integrations/agent/agentStateCache", () => ({ useCachedAgentState: () => undefined }));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function props(): ComponentProps<typeof ThreadListItem> {
  return {
    active: false,
    menuOpen: false,
    thread: {
      id: "thread",
      agentSessionId: "thread",
      title: "Thread title",
      mode: "chat",
      workspaceId: "workspace",
      status: "active",
      pinned: false,
      readonly: false,
      createdAt: 0,
      updatedAt: 0,
    },
    onDeleteThread: vi.fn(),
    onMenuOpenChange: vi.fn(),
    onRenameThread: vi.fn(),
    onRestoreThread: vi.fn(),
    onSelectThread: vi.fn(),
    onTogglePinThread: vi.fn(),
    onToggleSelection: vi.fn(),
    onToggleExpanded: vi.fn(),
  };
}

function mount(overrides: Partial<ComponentProps<typeof ThreadListItem>> = {}) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const all = { ...props(), ...overrides };
  act(() => root.render(<ThreadListItem {...all} />));
  return {
    all,
    container,
    row: () => container.firstElementChild as HTMLElement,
    rerender: (next: Partial<ComponentProps<typeof ThreadListItem>>) => act(() => root.render(<ThreadListItem {...all} {...next} />)),
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

function menuItem(container: HTMLElement, label: string) {
  return [...container.querySelectorAll<HTMLButtonElement>("[role=\"menuitem\"]")].find(button => button.textContent === label);
}

afterEach(() => {
  document.body.innerHTML = "";
});

describe("thread row interactions", () => {
  it("opens the actions menu on right-click without selecting the thread", () => {
    const view = mount();
    act(() => {
      view.row().dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    });
    expect(view.all.onMenuOpenChange).toHaveBeenCalledWith(view.all.thread, true);
    expect(view.all.onSelectThread).not.toHaveBeenCalled();
    view.unmount();
  });

  it("toggles the menu from the actions trigger and stops the row click", () => {
    const view = mount();
    const trigger = view.container.querySelector<HTMLButtonElement>("button[aria-haspopup=\"menu\"]")!;
    expect(trigger.getAttribute("aria-expanded")).toBe("false");
    act(() => trigger.click());
    expect(view.all.onMenuOpenChange).toHaveBeenCalledWith(view.all.thread, true);
    expect(view.all.onSelectThread).not.toHaveBeenCalled();

    view.rerender({ menuOpen: true });
    const openTrigger = view.container.querySelector<HTMLButtonElement>("button[aria-haspopup=\"menu\"]")!;
    expect(openTrigger.getAttribute("aria-expanded")).toBe("true");
    act(() => openTrigger.click());
    expect(view.all.onMenuOpenChange).toHaveBeenLastCalledWith(view.all.thread, false);
    view.unmount();
  });

  it("runs every action offered by the open menu against the row's thread", () => {
    const view = mount({ menuOpen: true });
    act(() => menuItem(view.container, "Rename")!.click());
    expect(view.all.onRenameThread).toHaveBeenCalledWith(view.all.thread);
    expect(view.all.onMenuOpenChange).toHaveBeenLastCalledWith(view.all.thread, false);

    act(() => menuItem(view.container, "Pin")!.click());
    expect(view.all.onTogglePinThread).toHaveBeenCalledWith(view.all.thread);

    act(() => menuItem(view.container, "Delete")!.click());
    expect(view.all.onDeleteThread).toHaveBeenCalledWith(view.all.thread);
    view.unmount();
  });

  it("offers Restore instead of rename/pin for an archived thread", () => {
    const view = mount({ archived: true, menuOpen: true });
    expect(view.container.textContent).toContain("Archived");
    expect(menuItem(view.container, "Rename")).toBeUndefined();
    act(() => menuItem(view.container, "Restore")!.click());
    expect(view.all.onRestoreThread).toHaveBeenCalledWith(view.all.thread);
    view.unmount();
  });

  it("closes the menu on Escape and returns focus to the trigger", () => {
    const view = mount({ menuOpen: true });
    const trigger = view.container.querySelector<HTMLButtonElement>("button[aria-haspopup=\"menu\"]")!;
    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    expect(view.all.onMenuOpenChange).toHaveBeenCalledWith(view.all.thread, false);
    expect(document.activeElement).toBe(trigger);
    view.unmount();
  });

  it("closes the menu when the pointer goes down outside the row", () => {
    const view = mount({ menuOpen: true });
    act(() => {
      document.body.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    });
    expect(view.all.onMenuOpenChange).toHaveBeenCalledWith(view.all.thread, false);
    view.unmount();
  });

  it("toggles selection from the checkbox without selecting the row", () => {
    const view = mount({ selected: false, selectionMode: true });
    const checkbox = view.container.querySelector<HTMLInputElement>("input[type=\"checkbox\"]")!;
    expect(checkbox.checked).toBe(false);
    expect(checkbox.getAttribute("aria-label")).toBe("Select Thread title");
    // No run indicator competes with the checkbox in selection mode.
    expect(view.container.querySelector("button[aria-haspopup=\"menu\"]")).toBeNull();

    act(() => checkbox.click());
    expect(view.all.onToggleSelection).toHaveBeenCalledWith(view.all.thread);
    expect(view.all.onSelectThread).not.toHaveBeenCalled();
    view.unmount();
  });

  it("leaves the checkbox inert when the row has no selection handler", () => {
    const view = mount({ onToggleSelection: undefined, selectionMode: true });
    const checkbox = view.container.querySelector<HTMLInputElement>("input[type=\"checkbox\"]")!;
    expect(() => act(() => checkbox.click())).not.toThrow();
    expect(view.all.onSelectThread).not.toHaveBeenCalled();
    view.unmount();
  });
});

describe("thread row run status", () => {
  it("spins while the response is streaming from another client", () => {
    const view = mount({ isStreaming: true });
    const running = view.container.querySelector("[aria-label=\"Running\"]");
    expect(running).not.toBeNull();
    expect(running!.querySelector(".animate-spin")).not.toBeNull();
    view.unmount();
  });

  it("shows a red unread dot for a failed run and a green one for a completed run", () => {
    for (const [status, label, colour] of [
      ["failed", "Failed", "bg-danger"],
      ["completed", "Completed", "bg-success"],
    ] as const) {
      const view = mount({ runStatus: { status } as never, unread: true });
      const badge = view.container.querySelector(`[aria-label="${label}"]`)!;
      expect(badge).not.toBeNull();
      expect(badge.querySelector(`.${colour}`)).not.toBeNull();
      view.unmount();
    }
  });

  it("hides the finished-run dot once the thread has been read", () => {
    const view = mount({ runStatus: { status: "completed" } as never, unread: false });
    expect(view.container.querySelector("[aria-label=\"Completed\"]")).toBeNull();
    expect(view.container.querySelector("[aria-label=\"Running\"]")).toBeNull();
    view.unmount();
  });

  it("never surfaces a cancelled run as unread", () => {
    const view = mount({ runStatus: { status: "cancelled" } as never, unread: true });
    expect(view.container.querySelector("[aria-label=\"Failed\"]")).toBeNull();
    expect(view.container.querySelector("[aria-label=\"Completed\"]")).toBeNull();
    view.unmount();
  });

  it("prefers a local run status over the streaming flag", () => {
    const view = mount({ isStreaming: true, runStatus: { status: "queued" } as never });
    expect(view.container.querySelector("[aria-label=\"Running\"]")).not.toBeNull();
    view.unmount();
  });

  it("badges pending approvals only while the thread is not the open one", () => {
    const view = mount({ active: false, pendingApprovalCount: 2 });
    expect(view.container.querySelector("[aria-label=\"2 pending approval(s)\"]")).not.toBeNull();
    view.unmount();

    const active = mount({ active: true, pendingApprovalCount: 2 });
    expect(active.container.querySelector("[aria-label=\"2 pending approval(s)\"]")).toBeNull();
    active.unmount();
  });
});
