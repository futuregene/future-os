// @vitest-environment jsdom
import type { MergedConversation } from "./mergeConversations";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { RemoteRenameDialog } from "./RemoteRenameDialog";

/**
 * The remote rename dialog. Its contract: the new name is sent to the host that
 * owns the conversation, addressed with the session id the host uses, and a
 * refusal keeps the dialog open with the user's text intact.
 */

const rename = vi.fn<(desktopId: string, address: unknown, name: string) => Promise<unknown>>();
vi.mock("./remotePeerClient", () => ({
  renameRemoteConversation: (...args: Parameters<typeof rename>) => rename(...args),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function conversation(overrides: Partial<MergedConversation> = {}): MergedConversation {
  return {
    key: "desktop_a::sess_1",
    desktopId: "desktop_a",
    id: "sess_1",
    threadId: "thread_1",
    title: "Old name",
    pinned: false,
    streaming: false,
    lastMessageAt: 1,
    mode: "chat",
    workspaceId: null,
    ...overrides,
  };
}

async function mount(row = conversation()) {
  const onClose = vi.fn();
  const onRenamed = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(<RemoteRenameDialog conversation={row} onClose={onClose} onRenamed={onRenamed} />);
  });
  const input = container.querySelector("input")!;
  return {
    container,
    input,
    onClose,
    onRenamed,
    button: (label: string) => [...container.querySelectorAll("button")]
      .find(node => node.textContent === label),
    setValue: async (value: string) => {
      await act(async () => {
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
        setter.call(input, value);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
    },
    pressEnter: async () => {
      await act(async () => {
        input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Enter" }));
      });
    },
    text: () => container.textContent ?? "",
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

async function settle(): Promise<void> {
  await act(async () => {
    for (let i = 0; i < 4; i += 1) await Promise.resolve();
  });
}

beforeEach(() => {
  rename.mockReset().mockResolvedValue(undefined);
});

it("starts from the current title and renames on the owning host", async () => {
  const view = await mount();
  expect(view.input.value).toBe("Old name");

  await view.setValue("Better name");
  await act(async () => view.button("Save")!.click());
  await settle();

  expect(rename).toHaveBeenCalledWith(
    "desktop_a",
    { sessionId: "sess_1", threadId: "thread_1" },
    "Better name",
  );
  expect(view.onRenamed).toHaveBeenCalledTimes(1);
  expect(view.onClose).toHaveBeenCalledTimes(1);
  await view.unmount();
});

it("trims the name it sends", async () => {
  const view = await mount();
  await view.setValue("  padded  ");
  await act(async () => view.button("Save")!.click());
  await settle();

  expect(rename).toHaveBeenCalledWith("desktop_a", expect.anything(), "padded");
  await view.unmount();
});

it("submits on Enter", async () => {
  const view = await mount();
  await view.setValue("Keyboard name");
  await view.pressEnter();
  await settle();

  expect(rename).toHaveBeenCalledTimes(1);
  await view.unmount();
});

it("refuses an empty name", async () => {
  const view = await mount();
  await view.setValue("   ");
  expect(view.button("Save")!.disabled).toBe(true);
  await view.pressEnter();
  await settle();

  expect(rename).not.toHaveBeenCalled();
  await view.unmount();
});

/** A refusal keeps the dialog open so the name can be corrected and retried. */
it("shows a refusal and stays open", async () => {
  rename.mockRejectedValue(new Error("peer_not_connected"));
  const view = await mount();
  await view.setValue("New name");
  await act(async () => view.button("Save")!.click());
  await settle();

  expect(view.text()).toContain("peer_not_connected");
  expect(view.onClose).not.toHaveBeenCalled();
  expect(view.onRenamed).not.toHaveBeenCalled();
  await view.unmount();
});

it("renders a non-Error refusal as text", async () => {
  rename.mockRejectedValue("offline");
  const view = await mount();
  await view.setValue("New name");
  await act(async () => view.button("Save")!.click());
  await settle();

  expect(view.text()).toContain("offline");
  await view.unmount();
});

it("cancels without sending anything", async () => {
  const view = await mount();
  await act(async () => view.button("Cancel")!.click());

  expect(rename).not.toHaveBeenCalled();
  expect(view.onClose).toHaveBeenCalledTimes(1);
  await view.unmount();
});
