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
const unpairRemotePeer = vi.fn<(desktopId: string) => Promise<string | null>>();
vi.mock("./remotePeerClient", () => ({
  connectRemotePeer: (...args: Parameters<typeof connectRemotePeer>) => connectRemotePeer(...args),
  disconnectRemotePeer: (...args: Parameters<typeof disconnectRemotePeer>) => disconnectRemotePeer(...args),
  listRemotePeers: () => listRemotePeers(),
  pairRemotePeer: (...args: Parameters<typeof pairRemotePeer>) => pairRemotePeer(...args),
  unpairRemotePeer: (...args: Parameters<typeof unpairRemotePeer>) => unpairRemotePeer(...args),
}));
// A revoke the platform never took comes back as a warning string, which the
// section says out loud rather than swallowing: captured here so that claim can
// be asserted.
const toasts = vi.hoisted(() => [] as Array<{ message: string; tone?: string }>);
vi.mock("../../lib/futureEvents", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../lib/futureEvents")>();
  return {
    ...actual,
    emitFutureEvent: (name: string, detail: unknown) => {
      if (name === "toast")
        toasts.push(detail as { message: string; tone?: string });
      else
        actual.emitFutureEvent(name as never, detail as never);
    },
  };
});
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
  const onOpenSettings = vi.fn();
  const onStartConversation = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <RemoteClientSection onOpenSettings={onOpenSettings} onStartConversation={onStartConversation} />,
    );
  });
  await act(async () => {
    for (let i = 0; i < 4; i += 1) await Promise.resolve();
  });
  return {
    container,
    input: () => container.querySelector<HTMLInputElement>("input")!,
    onOpenSettings,
    onStartConversation,
    button: (label: string) => [...container.querySelectorAll("button")]
      .find(node => node.textContent === label),
    /**
     * A computer's own row control, on the first row that carries it.
     *
     * Scoped to the row because two different controls answer to "Connect" (the
     * pairing form and an offline computer) and to "Unpair" (a machine's row and
     * its confirmation dialog) — taking the first match would exercise the wrong
     * one and pass while doing nothing.
     */
    rowButton: (label: string) => [...container.querySelectorAll<HTMLButtonElement>("[data-testid='peer-row'] button")]
      .find(node => node.textContent === label),
    /** The same control on one numbered row: each machine has its own. */
    rowButtonIn: (row: number, label: string) => [...(container.querySelectorAll("[data-testid='peer-row']")[row]
      ?.querySelectorAll<HTMLButtonElement>("button") ?? [])]
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
  toasts.length = 0;
  connectRemotePeer.mockReset().mockResolvedValue(undefined);
  disconnectRemotePeer.mockReset().mockResolvedValue(undefined);
  listRemotePeers.mockReset().mockResolvedValue([peer()]);
  pairRemotePeer.mockReset().mockResolvedValue(undefined);
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
 * Settings live in a window of their own, so this section opens that window
 * rather than rendering the page inline: a page stacked in this window is lost
 * the moment the user navigates, and it reads as if it were this machine's.
 */
it("opens a machine's settings in a window instead of inline", async () => {
  const view = await mount();
  await act(async () => view.button("Settings")!.click());

  // The *section* only reports which machine; the window is opened by the shell
  // (see `openRemoteSettingsWindow`), so that the page never becomes a second
  // place that owns the window lifecycle.
  expect(view.onOpenSettings).toHaveBeenCalledWith(expect.objectContaining({ desktopId: "desktop_a" }));
  // Nothing is stacked here: no settings page, and the list is still the page.
  expect(view.container.querySelector("[data-testid='peer-settings']")).toBeNull();
  expect(view.input()).toBeTruthy();
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

// ── unpairing a paired computer ─────────────────────────────────────────────

/**
 * Unpairing is asked for from the row of the computer it removes.
 *
 * It used to be a button on that computer's own settings page — a page the user
 * has to go and open, about the machine they have decided to stop using, which is
 * not where the decision is made. On the list they are already looking at, it is
 * one button per machine.
 */
it("asks before unpairing, and removes the machine whose row asked", async () => {
  // The list answers with both machines, then with only the one that is left:
  // what is on screen after the confirmation has to be what survived it.
  listRemotePeers
    .mockResolvedValueOnce([
      peer({ desktopId: "desktop_a", name: "Studio iMac" }),
      peer({ desktopId: "desktop_b", name: "Mac mini" }),
    ])
    .mockResolvedValue([peer({ desktopId: "desktop_a", name: "Studio iMac" })]);
  const view = await mount();

  await act(async () => view.rowButtonIn(1, "Unpair")!.click());

  // The question names the machine the row named: every row offers "Unpair", and
  // asking about one while removing another is the failure that hides here.
  const dialog = view.container.querySelector("[role='dialog']")?.textContent ?? "";
  expect(dialog).toContain("Unpair this desktop?");
  expect(dialog).toContain("Mac mini");
  expect(unpairRemotePeer).not.toHaveBeenCalled();

  await act(async () => view.dialogButton("Unpair")!.click());
  await view.settle();

  expect(unpairRemotePeer).toHaveBeenCalledWith("desktop_b");
  expect(view.text()).not.toContain("Mac mini");
  await view.unmount();
});

/** Cancelling leaves the pairing alone. */
it("cancels an unpair without touching the pairing", async () => {
  const view = await mount();

  await act(async () => view.rowButton("Unpair")!.click());
  await act(async () => view.dialogButton("Cancel")!.click());

  expect(unpairRemotePeer).not.toHaveBeenCalled();
  expect(view.container.querySelector("[role='dialog']")).toBeNull();
  await view.unmount();
});

/**
 * The pairing is gone locally whatever the platform says, so a revoke the
 * platform never took is reported instead of being claimed as a clean removal.
 */
it("reports a revoke the platform never took", async () => {
  unpairRemotePeer.mockResolvedValue("pending");
  const view = await mount();

  await act(async () => view.rowButton("Unpair")!.click());
  await act(async () => view.dialogButton("Unpair")!.click());
  await view.settle();

  expect(unpairRemotePeer).toHaveBeenCalledWith("desktop_a");
  expect(toasts.some(toast => toast.message.includes("could not be reached to revoke"))).toBe(true);
  await view.unmount();
});

/** Escape dismisses the question and leaves the pairing alone. */
it("closes the confirmation on Escape without unpairing", async () => {
  const view = await mount();

  await act(async () => view.rowButton("Unpair")!.click());
  expect(view.container.querySelector("[role='dialog']")).not.toBeNull();

  await act(async () => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  });

  expect(unpairRemotePeer).not.toHaveBeenCalled();
  expect(view.container.querySelector("[role='dialog']")).toBeNull();
  await view.unmount();
});
