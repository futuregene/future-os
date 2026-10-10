// @vitest-environment jsdom
import type { MergedConversation } from "./mergeConversations";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { RemoteConversationMenu } from "./RemoteConversationMenu";

/**
 * The row menu's contract: which host command each item issues, and what it
 * does when the host refuses. A menu that reported success on a failed pin
 * would leave the user believing a conversation was pinned on a machine that
 * never heard about it.
 */

const pin = vi.fn<(desktopId: string, address: unknown, pinned: boolean) => Promise<unknown>>();
const remove = vi.fn<(desktopId: string, address: unknown) => Promise<unknown>>();
const rename = vi.fn<(desktopId: string, address: unknown, name: string) => Promise<unknown>>();
vi.mock("./remotePeerClient", () => ({
  deleteRemoteConversation: (...args: Parameters<typeof remove>) => remove(...args),
  pinRemoteConversation: (...args: Parameters<typeof pin>) => pin(...args),
  renameRemoteConversation: (...args: Parameters<typeof rename>) => rename(...args),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function conversation(overrides: Partial<MergedConversation> = {}): MergedConversation {
  return {
    key: "desktop_a::sess_1",
    desktopId: "desktop_a",
    id: "sess_1",
    threadId: "thread_1",
    title: "Fix the login bug",
    pinned: false,
    streaming: false,
    lastMessageAt: 1,
    mode: "chat",
    workspaceId: null,
    ...overrides,
  };
}

async function mount(row: MergedConversation, handlers: {
  onChanged?: () => void;
  onClose?: () => void;
  onOpenRename?: () => void;
} = {}) {
  const changed = handlers.onChanged ?? vi.fn();
  const closed = handlers.onClose ?? vi.fn();
  const openRename = handlers.onOpenRename ?? vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <RemoteConversationMenu
        conversation={row}
        onChanged={changed}
        onClose={closed}
        onOpenRename={openRename}
      />,
    );
  });
  return {
    changed,
    closed,
    container,
    openRename,
    root,
    items: () => [...container.querySelectorAll("button")],
    item: (label: string) => [...container.querySelectorAll("button")]
      .find(node => node.textContent === label),
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

/** Let the item's promise chain settle inside `act`. */
async function settle(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

beforeEach(() => {
  pin.mockReset().mockResolvedValue(undefined);
  remove.mockReset().mockResolvedValue(undefined);
  rename.mockReset().mockResolvedValue(undefined);
});

it("shows nothing for a local row", async () => {
  const view = await mount(conversation({ desktopId: null }));
  // The local row component owns its own menu; a second one here would issue
  // local-store actions from a remote component.
  expect(view.container.textContent).toBe("");
  await view.unmount();
});

it("pins with the host thread id and refreshes once the host accepts", async () => {
  const view = await mount(conversation());
  await act(async () => view.item("Pin")!.click());
  await settle();

  expect(pin).toHaveBeenCalledWith("desktop_a", { sessionId: "sess_1", threadId: "thread_1" }, true);
  expect(view.changed).toHaveBeenCalledTimes(1);
  expect(view.closed).toHaveBeenCalledTimes(1);
  await view.unmount();
});

it("offers unpin for an already pinned conversation", async () => {
  const view = await mount(conversation({ pinned: true }));
  await act(async () => view.item("Unpin")!.click());
  await settle();

  expect(pin).toHaveBeenCalledWith("desktop_a", { sessionId: "sess_1", threadId: "thread_1" }, false);
  await view.unmount();
});

it("deletes, then refreshes", async () => {
  const view = await mount(conversation());
  await act(async () => view.item("Delete")!.click());
  await settle();

  expect(remove).toHaveBeenCalledWith("desktop_a", { sessionId: "sess_1", threadId: "thread_1" });
  expect(view.changed).toHaveBeenCalledTimes(1);
  await view.unmount();
});

it("hands rename to the caller instead of doing it inline", async () => {
  const view = await mount(conversation());
  await act(async () => view.item("Rename")!.click());
  await settle();

  expect(view.openRename).toHaveBeenCalledTimes(1);
  // The dialog owns the command; the menu must not also fire one.
  expect(rename).not.toHaveBeenCalled();
  await view.unmount();
});

/**
 * A host that reported no thread id cannot be pinned: the item is disabled
 * rather than sending the session id in its place.
 */
it("disables pin and delete when the host gave no thread id", async () => {
  const view = await mount(conversation({ threadId: null }));

  expect(view.item("Pin")!.disabled).toBe(true);
  expect(view.item("Delete")!.disabled).toBe(true);
  await act(async () => view.item("Pin")!.click());
  await settle();
  expect(pin).not.toHaveBeenCalled();
  expect(view.changed).not.toHaveBeenCalled();
  await view.unmount();
});

/**
 * A refusal has to stay readable: the menu keeps the message and stays open, so
 * the user can see what the host said and try again.
 */
it("keeps a host's refusal visible and stays open", async () => {
  pin.mockRejectedValue(new Error("peer_not_connected"));
  const view = await mount(conversation());

  await act(async () => view.item("Pin")!.click());
  await settle();

  expect(view.container.textContent).toContain("peer_not_connected");
  expect(view.changed).not.toHaveBeenCalled();
  expect(view.closed).not.toHaveBeenCalled();
  await view.unmount();
});

/** A non-Error rejection still has to render. */
it("renders a non-Error refusal as text", async () => {
  remove.mockRejectedValue("offline");
  const view = await mount(conversation());

  await act(async () => view.item("Delete")!.click());
  await settle();

  expect(view.container.textContent).toContain("offline");
  await view.unmount();
});

/** A second click while the host is still answering must not queue another. */
it("ignores a second action while one is in flight", async () => {
  let release: (() => void) | null = null;
  pin.mockImplementation(() => new Promise((resolve) => {
    release = () => resolve(undefined);
  }));
  const view = await mount(conversation());

  await act(async () => view.item("Pin")!.click());
  await act(async () => view.item("Pin")!.click());
  expect(pin).toHaveBeenCalledTimes(1);

  await act(async () => {
    release?.();
  });
  await settle();
  await view.unmount();
});
