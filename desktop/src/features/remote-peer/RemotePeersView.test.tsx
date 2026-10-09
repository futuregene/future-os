// @vitest-environment jsdom
import type { RemotePeer } from "../../features/remote-peer/remotePeerClient";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { RemotePeersView } from "./RemotePeersView";

/**
 * The desktop-management screen. Only its new responsibility is asserted here —
 * starting a conversation on a host — because that button's *gating* is the
 * part that can lie: a bridge can be reachable while its agent is not, and a
 * composer opened there would take messages that can never be answered.
 */

const listRemotePeers = vi.fn<() => Promise<RemotePeer[]>>();
vi.mock("./remotePeerClient", () => ({
  connectRemotePeer: vi.fn(async () => {}),
  disconnectRemotePeer: vi.fn(async () => {}),
  listRemotePeers: () => listRemotePeers(),
  pairRemotePeer: vi.fn(async () => {}),
  setRemotePeerLabel: vi.fn(async () => {}),
  unpairRemotePeer: vi.fn(async () => null),
}));
vi.mock("./RemotePeerSettings", () => ({ RemotePeerSettings: () => <div /> }));
// LeftPanelTitlebarToggle → useIsFullscreen reads the Tauri window API, which is
// irrelevant (and absent) under jsdom.
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isFullscreen: () => Promise.resolve(false),
    onResized: () => Promise.resolve(() => {}),
  }),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function peer(overrides: Partial<RemotePeer> = {}): RemotePeer {
  return {
    agentAvailable: true,
    bridgeInstanceId: "bridge_1",
    connected: true,
    desktopId: "desktop_a",
    error: null,
    features: [],
    icon: "laptop",
    name: "Studio iMac",
    pairId: "pair_1",
    ...overrides,
  };
}

async function mount() {
  const onStartConversation = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <RemotePeersView
        leftPanelExpanded
        onStartConversation={onStartConversation}
        onToggleLeftPanel={() => {}}
      />,
    );
  });
  await act(async () => {
    for (let i = 0; i < 4; i += 1) await Promise.resolve();
  });
  return {
    container,
    onStartConversation,
    button: (label: string) => [...container.querySelectorAll("button")]
      .find(node => node.textContent === label),
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

beforeEach(() => {
  document.body.innerHTML = "";
  listRemotePeers.mockReset().mockResolvedValue([peer()]);
});

it("offers a new conversation on a reachable host, and names it", async () => {
  const view = await mount();
  expect(view.button("New conversation")).toBeTruthy();

  await act(async () => view.button("New conversation")!.click());
  expect(view.onStartConversation).toHaveBeenCalledWith("desktop_a");
  await view.unmount();
});

/**
 * The link is up but the agent is not: every prompt would fail, so the button is
 * withheld rather than handing the user a composer that cannot deliver.
 */
it("withholds the new conversation button when the host's agent is unreachable", async () => {
  listRemotePeers.mockResolvedValue([peer({ agentAvailable: false })]);
  const view = await mount();

  expect(view.button("New conversation")).toBeUndefined();
  await view.unmount();
});

it("withholds the new conversation button while disconnected", async () => {
  listRemotePeers.mockResolvedValue([peer({ connected: false })]);
  const view = await mount();

  expect(view.button("New conversation")).toBeUndefined();
  await view.unmount();
});
