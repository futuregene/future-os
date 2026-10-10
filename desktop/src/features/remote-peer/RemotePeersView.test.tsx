// @vitest-environment jsdom
import type { RemotePeer } from "../../features/remote-peer/remotePeerClient";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { RemoteClientSection } from "./RemotePeersView";

/**
 * The client direction: the computers *this* machine connects out to.
 *
 * Every action here reaches another machine, so the assertions are about which
 * command went to which host — a disconnect sent to the wrong desktop, or a
 * pairing link that never left the input box, are the failures a user cannot
 * see until something on the other end is wrong.
 */

const connectRemotePeer = vi.fn<(desktopId: string) => Promise<unknown>>();
const disconnectRemotePeer = vi.fn<(desktopId: string) => Promise<unknown>>();
const listRemotePeers = vi.fn<() => Promise<RemotePeer[]>>();
const pairRemotePeer = vi.fn<(invitation: string) => Promise<unknown>>();
const setRemotePeerLabel = vi.fn<(desktopId: string, patch: Record<string, unknown>) => Promise<unknown>>();
const unpairRemotePeer = vi.fn<(desktopId: string) => Promise<string | null>>();
vi.mock("./remotePeerClient", () => ({
  connectRemotePeer: (...args: Parameters<typeof connectRemotePeer>) => connectRemotePeer(...args),
  disconnectRemotePeer: (...args: Parameters<typeof disconnectRemotePeer>) => disconnectRemotePeer(...args),
  listRemotePeers: () => listRemotePeers(),
  pairRemotePeer: (...args: Parameters<typeof pairRemotePeer>) => pairRemotePeer(...args),
  setRemotePeerLabel: (...args: Parameters<typeof setRemotePeerLabel>) => setRemotePeerLabel(...args),
  unpairRemotePeer: (...args: Parameters<typeof unpairRemotePeer>) => unpairRemotePeer(...args),
}));
// The real settings panel reads the host for models/skills/tasks; its own
// behaviour is tested where it lives, and `onUpdateLabel` is what this section
// is responsible for wiring.
vi.mock("./RemotePeerSettings", () => ({
  RemotePeerSettings: (props: {
    onBack: () => void;
    onChanged: () => void;
    onUnpair: () => void;
    onUpdateLabel: (patch: Record<string, unknown>) => void;
  }) => (
    <div data-testid="peer-settings">
      <button onClick={props.onBack} type="button">settings-back</button>
      <button onClick={() => props.onUpdateLabel({ name: "Renamed" })} type="button">settings-rename</button>
      {/* Unpairing lives behind the computer's settings, so the confirmation
          this section owns only appears from there. */}
      <button onClick={props.onUnpair} type="button">settings-unpair</button>
      <button onClick={props.onChanged} type="button">settings-changed</button>
    </div>
  ),
}));
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
      <RemoteClientSection onStartConversation={onStartConversation} />,
    );
  });
  await act(async () => {
    for (let i = 0; i < 4; i += 1) await Promise.resolve();
  });
  return {
    container,
    input: () => container.querySelector<HTMLInputElement>("input")!,
    onStartConversation,
    button: (label: string) => [...container.querySelectorAll("button")]
      .find(node => node.textContent === label),
    /**
     * A computer's own row control.
     *
     * Scoped to the row because two different controls answer to "Connect" (the
     * pairing form and an offline computer) and to "Unpair" (the row and its
     * confirmation dialog) — taking the first match would exercise the wrong one
     * and pass while doing nothing.
     */
    rowButton: (label: string) => [...container.querySelectorAll<HTMLButtonElement>("[data-testid='peer-row'] button")]
      .find(node => node.textContent === label),
    /** A control inside the open dialog. */
    dialogButton: (label: string) => [...container.querySelectorAll<HTMLButtonElement>("[role=\'dialog\'] button")]
      .find(node => node.textContent === label),
    text: () => container.textContent ?? "",
    type: async (value: string) => {
      const input = container.querySelector<HTMLInputElement>("input")!;
      await act(async () => {
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
        setter.call(input, value);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
    },
    settle,
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

async function settle(): Promise<void> {
  await act(async () => {
    for (let i = 0; i < 6; i += 1) await Promise.resolve();
  });
}

beforeEach(() => {
  document.body.innerHTML = "";
  connectRemotePeer.mockReset().mockResolvedValue(undefined);
  disconnectRemotePeer.mockReset().mockResolvedValue(undefined);
  listRemotePeers.mockReset().mockResolvedValue([peer()]);
  pairRemotePeer.mockReset().mockResolvedValue(undefined);
  setRemotePeerLabel.mockReset().mockResolvedValue(undefined);
  unpairRemotePeer.mockReset().mockResolvedValue(null);
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

// ── pairing a new computer ──────────────────────────────────────────────────

it("pairs from a pasted link, then clears the box", async () => {
  const view = await mount();
  await view.type("futureos://remote/pair?code=abc");

  await act(async () => view.button("Connect")!.click());
  await view.settle();

  expect(pairRemotePeer).toHaveBeenCalledWith("futureos://remote/pair?code=abc");
  expect(view.input().value).toBe("");
  await view.unmount();
});

/** A failed pairing keeps the link: retyping an invitation is not the user's job. */
it("keeps the link and reports the reason when pairing fails", async () => {
  pairRemotePeer.mockRejectedValue(new Error("invitation_expired"));
  const view = await mount();
  await view.type("futureos://remote/pair?code=stale");

  await act(async () => view.button("Connect")!.click());
  await view.settle();

  expect(view.text()).toContain("invitation_expired");
  await view.unmount();
});

it("will not pair an empty link", async () => {
  const view = await mount();
  expect(view.button("Connect")!.hasAttribute("disabled")).toBe(true);
  await view.unmount();
});

// ── acting on a paired computer ─────────────────────────────────────────────

it("disconnects the connected computer, by its own id", async () => {
  const view = await mount();
  await act(async () => view.rowButton("Disconnect")!.click());
  await view.settle();
  expect(disconnectRemotePeer).toHaveBeenCalledWith("desktop_a");
  await view.unmount();
});

it("reconnects an offline computer, by its own id", async () => {
  listRemotePeers.mockResolvedValue([peer({ connected: false })]);
  const view = await mount();

  // The row's button, not the pairing form's — both read "Connect", and the
  // pairing one is disabled with an empty link, so clicking the wrong one would
  // pass by doing nothing.
  await act(async () => view.rowButton("Connect")!.click());
  await view.settle();
  expect(connectRemotePeer).toHaveBeenCalledWith("desktop_a");
  await view.unmount();
});

/** Unpairing is destructive enough to confirm, and the confirmation is the thing that acts. */
it("unpairs only after the confirmation", async () => {
  const view = await mount();
  await act(async () => view.rowButton("Settings")!.click());
  await act(async () => view.button("settings-unpair")!.click());
  expect(unpairRemotePeer).not.toHaveBeenCalled();

  await act(async () => view.dialogButton("Unpair")!.click());
  await view.settle();
  expect(unpairRemotePeer).toHaveBeenCalledWith("desktop_a");
  await view.unmount();
});

it("cancels an unpair without touching the pairing", async () => {
  const view = await mount();
  await act(async () => view.rowButton("Settings")!.click());
  await act(async () => view.button("settings-unpair")!.click());
  await act(async () => view.dialogButton("Cancel")!.click());
  await view.settle();

  expect(unpairRemotePeer).not.toHaveBeenCalled();
  expect(view.container.querySelector("[role='dialog']")).toBeNull();
  await view.unmount();
});

/**
 * A revoke the platform never accepted is reported: the local pairing is gone
 * either way, and saying so is more honest than claiming a clean removal.
 */
it("says so when the platform-side revoke could not be delivered", async () => {
  unpairRemotePeer.mockResolvedValue("pending");
  const view = await mount();
  await act(async () => view.rowButton("Settings")!.click());
  await act(async () => view.button("settings-unpair")!.click());

  // The pairing is gone locally either way, so the list comes back — and the
  // warning about the undelivered revoke belongs there, not on a settings page
  // for a computer that is no longer paired.
  listRemotePeers.mockResolvedValue([]);
  await act(async () => view.dialogButton("Unpair")!.click());
  await view.settle();

  expect(view.text()).toContain("could not be reached to revoke");
  await view.unmount();
});

it("opens that computer's settings, and writes a rename to it", async () => {
  const view = await mount();
  await act(async () => view.rowButton("Settings")!.click());
  expect(view.container.querySelector("[data-testid='peer-settings']")).not.toBeNull();

  await act(async () => view.button("settings-rename")!.click());
  await view.settle();
  expect(setRemotePeerLabel).toHaveBeenCalledWith("desktop_a", { name: "Renamed" });

  await act(async () => view.button("settings-back")!.click());
  expect(view.container.querySelector("[data-testid='peer-settings']")).toBeNull();
  await view.unmount();
});

it("lists every paired computer with its own name", async () => {
  listRemotePeers.mockResolvedValue([
    peer({ desktopId: "desktop_a", name: "Studio iMac" }),
    peer({ desktopId: "desktop_b", name: "Mac mini" }),
  ]);
  const view = await mount();

  expect(view.text()).toContain("Studio iMac");
  expect(view.text()).toContain("Mac mini");
  await view.unmount();
});

/** Dismissing the confirmation with Escape must leave the pairing alone. */
it("closes the unpair confirmation on Escape without unpairing", async () => {
  const view = await mount();
  await act(async () => view.rowButton("Settings")!.click());
  await act(async () => view.button("settings-unpair")!.click());
  expect(view.container.querySelector("[role='dialog']")).not.toBeNull();

  await act(async () => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  });
  await view.settle();

  expect(unpairRemotePeer).not.toHaveBeenCalled();
  expect(view.container.querySelector("[role='dialog']")).toBeNull();
  await view.unmount();
});

/**
 * A change made on that computer's settings page re-reads the computers.
 *
 * The title or icon the panel just wrote lives on the host, so nothing local
 * would show it until the next catalogue read.
 */
it("re-reads the computers when a settings page reports a change", async () => {
  const view = await mount();
  await act(async () => view.rowButton("Settings")!.click());
  const readsBefore = listRemotePeers.mock.calls.length;

  await act(async () => view.button("settings-changed")!.click());
  await view.settle();

  expect(listRemotePeers.mock.calls.length).toBeGreaterThan(readsBefore);
  await view.unmount();
});
