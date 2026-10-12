// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { RemoteWorkspaceMenu } from "./RemoteWorkspaceMenu";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/**
 * The actions a workspace on another machine supports.
 *
 * The same menu the local heading has, reduced to what the host accepts: pin and
 * remove. "Reveal in file manager" is absent because the folder is on that
 * machine, where an item that did nothing when clicked would be worse than its
 * absence.
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

async function mount() {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const onChanged = vi.fn();
  await act(async () => {
    root.render(<RemoteWorkspaceMenu desktopId="desktop_a" name="图表脚本" onChanged={onChanged} workspaceId="ws_9" />);
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

it("unpins it again", async () => {
  const view = await mount();
  await view.open();
  await act(async () => view.item("Unpin")!.click());
  expect(pin).toHaveBeenCalledWith("desktop_a", "ws_9", false);
  await view.unmount();
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
