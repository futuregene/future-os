// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { RemoteWorkspaceMenu } from "./RemoteWorkspaceMenu";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/**
 * The actions a workspace on another machine supports.
 *
 * The same menu the local heading has, reduced to what the host accepts: pin,
 * select its conversations, and remove. "Reveal in file manager" is absent
 * because the folder is on that machine, where an item that did nothing when
 * clicked would be worse than its absence.
 *
 * **One pin item**, showing the state the host is actually in. The menu used to
 * offer "pin" and "unpin" side by side, which asks the user to work out which of
 * the two is the no-op.
 */
const pin = vi.fn<(desktopId: string, workspaceId: string, pinned: boolean) => Promise<unknown>>();
const remove = vi.fn<(desktopId: string, workspaceId: string) => Promise<unknown>>();
vi.mock("./remotePeerClient", () => ({
  deleteRemoteWorkspace: (...args: Parameters<typeof remove>) => remove(...args),
  setRemoteWorkspacePinned: (...args: Parameters<typeof pin>) => pin(...args),
}));

beforeEach(() => {
  pin.mockReset().mockResolvedValue(undefined);
  remove.mockReset().mockResolvedValue(undefined);
});

async function mount(overrides: { onRename?: (workspaceId: string) => void; onSelect?: () => void; pinned?: boolean } = {}) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const onChanged = vi.fn();
  await act(async () => {
    root.render(
      <RemoteWorkspaceMenu
        desktopId="desktop_a"
        name="图表脚本"
        onChanged={onChanged}
        onRename={overrides.onRename}
        onSelect={overrides.onSelect}
        pinned={overrides.pinned ?? false}
        workspaceId="ws_9"
      />,
    );
  });
  const trigger = () => container.querySelector<HTMLButtonElement>("button[aria-label^='Workspace actions for']")!;
  const item = (label: string) => [...container.querySelectorAll("button")].find(node => node.textContent === label)!;
  return {
    container,
    item,
    onChanged,
    open: async () => {
      await act(async () => trigger().click());
    },
    trigger,
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

/**
 * A workspace on another machine is renamed on that machine.
 *
 * The menu only hands the request up: the dialog and its text are the shell's, as
 * they are for renaming one of this machine's own workspaces. The menu closes
 * first, because what comes next is a dialog rather than a host command.
 */
it("hands rename to the caller, and closes", async () => {
  const onRename = vi.fn();
  const view = await mount({ onRename });
  await view.open();
  await act(async () => view.item("Rename")!.click());

  expect(onRename).toHaveBeenCalledWith("ws_9");
  expect(view.container.querySelector("[role='menu']")).toBeNull();
  await view.unmount();
});

it("pins the workspace on its host, then re-reads that host's list", async () => {
  const view = await mount();
  await view.open();
  await act(async () => view.item("Pin")!.click());

  expect(pin).toHaveBeenCalledWith("desktop_a", "ws_9", true);
  expect(view.onChanged).toHaveBeenCalled();
  // The menu closes on success, so the change is not left pending on screen.
  expect(view.container.querySelector("[role='menu']")).toBeNull();
  await view.unmount();
});

/**
 * A workspace the host already pinned is offered "unpin", and only that.
 *
 * The two items cannot both be right: the host's state decides which is the
 * action and which is the no-op, and a menu that lists both makes the user guess.
 */
it("offers unpin, and not both pin and unpin, when the host says it is pinned", async () => {
  const view = await mount({ onRename: vi.fn(), pinned: true });
  await view.open();
  const labels = [...view.container.querySelectorAll("[role='menuitem']")].map(node => node.textContent);
  // Rename leads, as it does in the local menu; the pin item shows the host's own
  // state, and only that one of the two is offered.
  expect(labels).toEqual(["Rename", "Unpin", "Delete"]);
  await act(async () => view.item("Unpin")!.click());
  expect(pin).toHaveBeenCalledWith("desktop_a", "ws_9", false);
  await view.unmount();
});

/**
 * The local heading's third item, on a workspace that lives elsewhere.
 *
 * A batch is assembled from the rows under a heading, wherever they are, so a
 * remote workspace offers the same "select chats" the local one does.
 */
it("marks the workspace's conversations for a batch", async () => {
  const onSelect = vi.fn();
  const view = await mount({ onRename: vi.fn(), onSelect });
  await view.open();
  await act(async () => view.item("Select chats")!.click());
  expect(onSelect).toHaveBeenCalledTimes(1);
  await view.unmount();

  // Offered only outside selection mode, as the local heading's menu is: the item
  // is what *enters* it.
  const withoutSelect = await mount({ onRename: vi.fn() });
  await withoutSelect.open();
  expect(withoutSelect.container.textContent).not.toContain("Select chats");
  await withoutSelect.unmount();
});

it("removes the workspace from the host's list", async () => {
  const view = await mount();
  await view.open();
  await act(async () => view.item("Delete")!.click());
  expect(remove).toHaveBeenCalledWith("desktop_a", "ws_9");
  expect(view.onChanged).toHaveBeenCalled();
  await view.unmount();
});

/**
 * A failure is shown inside the panel.
 *
 * The menu stays open on failure for exactly this: a host command can fail on the
 * wire, and a message that disappeared with the menu that produced it is no
 * message at all.
 */
it("keeps the menu open and shows the reason when the host refuses", async () => {
  pin.mockRejectedValue(new Error("host refused to pin"));
  const view = await mount();
  await view.open();
  await act(async () => view.item("Pin")!.click());

  expect(view.container.textContent).toContain("host refused to pin");
  expect(view.container.querySelector("[role='menu']")).not.toBeNull();
  await view.unmount();
});

it("closes the menu on a second click of its trigger", async () => {
  const view = await mount();
  await view.open();
  expect(view.container.querySelector("[role='menu']")).not.toBeNull();
  await act(async () => view.trigger().click());
  expect(view.container.querySelector("[role='menu']")).toBeNull();
  await view.unmount();
});
