// @vitest-environment jsdom
import type { DeviceFilter } from "../../features/remote-peer/mergeConversations";
import type { RemotePeer } from "../../features/remote-peer/remotePeerClient";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { RailNewChatButton } from "./RailNewChatButton";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const peer: RemotePeer = {
  agentAvailable: true,
  bridgeInstanceId: "bridge_1",
  connected: true,
  stopped: false,
  desktopId: "desktop_a",
  error: null,
  features: [],
  icon: "laptop",
  name: "Studio iMac",
  pairId: "pair_1",
};

beforeEach(() => {
  document.body.innerHTML = "";
});

async function mount(
  filter: DeviceFilter,
  props: Partial<Parameters<typeof RailNewChatButton>[0]> = {},
) {
  const onNewChatLocal = vi.fn();
  const onNewChatRemote = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <RailNewChatButton
        filter={filter}
        onNewChatLocal={onNewChatLocal}
        onNewChatRemote={onNewChatRemote}
        peers={props.peers ?? []}
      />,
    );
  });
  return { container, onNewChatLocal, onNewChatRemote, root };
}

/** Click the trigger without letting the click reach a surrounding layer. */
async function openMenu(container: HTMLElement) {
  const trigger = container.querySelector<HTMLButtonElement>("button")!;
  await act(async () => {
    trigger.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  return trigger;
}

describe("railNewChatButton machine picker", () => {
  it("acts directly when one machine is already selected", async () => {
    const local = await mount({ kind: "device", desktopId: null });
    await act(async () => {
      local.container.querySelector<HTMLButtonElement>("button")!.click();
    });
    expect(local.onNewChatLocal).toHaveBeenCalledTimes(1);
    expect(local.onNewChatRemote).not.toHaveBeenCalled();
    expect(local.container.querySelector("[role='menu']")).toBeNull();
    await act(async () => local.root.unmount());

    const remote = await mount({ kind: "device", desktopId: "desktop_a" }, { peers: [peer] });
    await act(async () => {
      remote.container.querySelector<HTMLButtonElement>("button")!.click();
    });
    expect(remote.onNewChatRemote).toHaveBeenCalledWith("desktop_a", null);
    expect(remote.onNewChatLocal).not.toHaveBeenCalled();
    await act(async () => remote.root.unmount());
  });

  it("offers this machine and every paired host when the list shows all of them", async () => {
    const view = await mount({ kind: "all" }, { peers: [peer] });
    expect(view.container.querySelector("[role='menu']")).toBeNull();
    await openMenu(view.container);

    const items = [...view.container.querySelectorAll<HTMLButtonElement>("[role='menuitem']")];
    // This machine's own name first, then each paired host by its name.
    expect(items).toHaveLength(2);
    expect(items[1]!.textContent).toContain("Studio iMac");

    await act(async () => items[1]!.click());
    expect(view.onNewChatRemote).toHaveBeenCalledWith("desktop_a", null);
    expect(view.container.querySelector("[role='menu']")).toBeNull();
    await act(async () => view.root.unmount());
  });

  it("closes the picker when the click lands outside it", async () => {
    // The picker is a floating menu: without a dismiss path it would stay up
    // over the rail and swallow the next click's meaning.
    const view = await mount({ kind: "all" }, { peers: [peer] });
    await openMenu(view.container);
    expect(view.container.querySelector("[role='menu']")).not.toBeNull();

    await act(async () => {
      document.body.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
    });

    expect(view.container.querySelector("[role='menu']")).toBeNull();
    await act(async () => view.root.unmount());
  });
});
