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
    stopped: false,
    desktopId: "desktop_a",
    error: null,
    features: [],
    icon: "laptop",
    name: "Studio iMac",
    pairId: "pair_1",
    ...overrides,
  };
}

/**
 * The row that folds the pairing form, by what it says.
 *
 * It is the row itself that is found, not just any `aria-expanded` control: the
 * section is rendered on its own here, but a `Button` or a dialog could carry
 * the attribute too, and reading the wrong one would make the assertions below
 * pass without touching the form.
 */
function addRowButton(container: HTMLElement): HTMLButtonElement {
  return [...container.querySelectorAll<HTMLButtonElement>("button[aria-expanded]")]
    .find(node => (node.textContent || "").trim() === "Add a desktop")!;
}

async function mount() {
  const onOpenSettings = vi.fn();
  const onReconnectAll = vi.fn(async () => {});
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <RemoteClientSection
        onOpenSettings={onOpenSettings}
        onReconnectAll={onReconnectAll}
      />,
    );
  });
  await act(async () => {
    for (let i = 0; i < 4; i += 1) await Promise.resolve();
  });
  return {
    container,
    input: () => container.querySelector<HTMLInputElement>("input")!,
    onOpenSettings,
    onReconnectAll,
    button: (label: string) => [...container.querySelectorAll("button")]
      .find(node => node.textContent === label),
    /**
     * The row that folds the pairing form.
     *
     * The form is folded to start (the machines below are the card's content),
     * so every pairing test opens this first — which is also what a user does.
     * `aria-expanded` is the row's own statement of which way it is, and it is
     * what these assertions read rather than a class name.
     */
    addRow: () => addRowButton(container),
    /** Open the pairing form if it is folded, so the box is on screen. */
    openForm: async () => {
      const row = addRowButton(container);
      if (row.getAttribute("aria-expanded") === "false")
        await act(async () => row.click());
      return row;
    },
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
      const row = addRowButton(container);
      if (row.getAttribute("aria-expanded") === "false")
        await act(async () => row.click());
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

/**
 * A machine's row is about the machine, not about starting chats: conversations
 * from every host live in the shared conversation list, and a second entry point
 * here would make the user pick between two answers to "where do I start one".
 * The row therefore offers connect/disconnect, settings and unpair only.
 */
it("keeps a machine's row to machine actions", async () => {
  const view = await mount();
  const labels = [...view.container.querySelectorAll("[data-testid='peer-row'] button")]
    .map(node => node.textContent?.trim());

  expect(labels).toEqual(["Disconnect", "Settings", "Unpair"]);
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
  expect(view.rowButton("Settings")).toBeTruthy();
  await view.unmount();
});

/**
 * The pairing form is folded away until it is wanted.
 *
 * It sat open above the machines at 158px — a fifth of the page — with two
 * sentences about a text box, on a page whose content is the machines the user
 * already has. The row states what is behind it, and the box keeps its name when
 * the label that used to be beside it is gone.
 */
it("folds the pairing form away, and names the box when it is opened", async () => {
  const view = await mount();

  expect(view.addRow().getAttribute("aria-expanded")).toBe("false");
  expect(view.input()).toBeNull();

  await act(async () => view.addRow().click());
  expect(view.addRow().getAttribute("aria-expanded")).toBe("true");
  expect(view.input().getAttribute("aria-label")).toBe("Add a desktop");

  // And back: the row is the only way in and the only way out.
  await act(async () => view.addRow().click());
  expect(view.input()).toBeNull();
  await view.unmount();
});

// ── pairing a new computer ────────────────────────────────────────────────

it("pairs from a pasted link, and keeps the box empty after it", async () => {
  const view = await mount();
  await view.type("futureos://remote/pair?code=abc");

  await act(async () => view.button("Connect")!.click());
  await view.settle();

  expect(pairRemotePeer).toHaveBeenCalledWith("futureos://remote/pair?code=abc");
  // The rows are the page again, and the box behind the closed row is empty:
  // keeping a single-use link would invite a second press that cannot work.
  await act(async () => view.addRow().click());
  expect(view.input().value).toBe("");
  await view.unmount();
});

/** A failed pairing keeps the link: retyping an invitation is not the user's job. */
it("keeps the link and reports the reason when pairing fails", async () => {
  // The host's refusal detail, as the secure-channel prefix carries it. The
  // screen must explain it, not quote it.
  pairRemotePeer.mockRejectedValue(new Error("remote_secure_channel_invalid (invitation_expired)"));
  const view = await mount();
  await view.type("futureos://remote/pair?code=stale");

  await act(async () => view.button("Connect")!.click());
  await view.settle();

  expect(view.text()).toContain("That pairing has expired");
  expect(view.text()).not.toContain("remote_secure_channel_invalid");
  // The link survives the failure, ready for a corrected paste.
  expect(view.input().value).toBe("futureos://remote/pair?code=stale");
  await view.unmount();
});

it("will not pair an empty link", async () => {
  const view = await mount();
  await view.openForm();
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

// ── several machines at once ────────────────────────────────────────────────

/**
 * The count is the answer to "why is a machine's conversation missing". The app
 * holds every pairing open, so a merged list short of one machine is a fact
 * about this machine rather than about their settings — and this is where the
 * user is told which of them is not up.
 */
it("says how many of several machines are up", async () => {
  listRemotePeers.mockResolvedValue([
    peer({ desktopId: "desktop_a" }),
    peer({ desktopId: "desktop_b", connected: false, error: "gone" }),
  ]);
  const view = await mount();

  expect(view.container.querySelector("[data-testid='peers-summary']")?.textContent)
    .toBe("1 of 2 connected");
  await view.unmount();
});

/** One machine has no set to summarise: its own row already says everything. */
it("does not count a single machine", async () => {
  const view = await mount();

  expect(view.container.querySelector("[data-testid='peers-summary']")).toBeNull();
  expect(view.button("Reconnect all")).toBeUndefined();
  await view.unmount();
});

/**
 * "Reconnect all" is the one control allowed to overrule a deliberate stop, so
 * it must reach the shell's own liveness pass rather than reconnecting the rows
 * itself.
 */
it("asks the shell to reconnect every machine", async () => {
  listRemotePeers.mockResolvedValue([
    peer({ desktopId: "desktop_a" }),
    peer({ desktopId: "desktop_b", connected: false }),
  ]);
  const view = await mount();

  await act(async () => view.button("Reconnect all")!.click());
  await view.settle();

  expect(view.onReconnectAll).toHaveBeenCalledTimes(1);
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
 * A machine the user turned off is listed as switched off — not as a fault, and
 * not with the socket error its own disconnect left behind — and its row offers
 * the way back.
 */
it("shows a stopped machine as stopped, with the way back", async () => {
  listRemotePeers.mockResolvedValue([
    peer({ connected: false, error: "socket closed", stopped: true }),
  ]);
  const view = await mount();

  expect(view.rowButton("Connect")).toBeTruthy();
  expect(view.rowButton("Disconnect")).toBeUndefined();
  expect(view.text()).not.toContain("socket closed");
  await view.unmount();
});

/** The three "not yet" states must not read as the same thing. */
it("tells a machine still being reached from one that failed", async () => {
  listRemotePeers.mockResolvedValue([peer({ connected: false, error: null })]);
  const reaching = await mount();
  expect(reaching.text()).not.toContain("Offline");
  await reaching.unmount();

  listRemotePeers.mockResolvedValue([peer({ connected: false, error: "refused" })]);
  const failed = await mount();
  expect(failed.text()).toContain("Offline");
  await failed.unmount();
});

/**
 * The runtime's failure text is a diagnostic, not a sentence for the user: the
 * row says what they can do about a desktop that did not answer, and keeps the
 * original as the tooltip a support session needs.
 */
it("explains a failed connection instead of quoting the transport", async () => {
  const raw = "Remote handshake failed: no responders: no responders";
  listRemotePeers.mockResolvedValue([peer({ connected: false, error: raw })]);
  const view = await mount();

  expect(view.text()).toContain("Couldn't reach that desktop");
  expect(view.text()).not.toContain("no responders");
  expect(view.container.querySelector(`[title="${raw}"]`)).toBeTruthy();
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

/**
 * The three actions on this screen all reach another machine, and all three can
 * fail. Each failure has to be *said*: the machine's own row would otherwise look
 * unchanged, and a user who pressed "Reconnect all" or "Connect" would have no
 * way to tell a slow round trip from one that never left.
 *
 * The read is the third: the credential book is local, and a failure to read it
 * is the difference between "no machines are paired" and "I could not find out".
 */
it("reports a peer list it cannot read", async () => {
  listRemotePeers.mockRejectedValue(new Error("keystore unavailable"));
  const view = await mount();

  expect(view.text()).toContain("keystore unavailable");
  await view.unmount();
});

/**
 * "Nothing is paired" and "the answer has not arrived" used to render as the
 * same sentence, because the rows start empty and the read can take a while
 * (the backend's peer list shares a lock with every live command). A machine the
 * user has paired therefore looked *removed* until the read answered — and then
 * quietly reappeared, which is exactly what a dropped host used to do.
 */
it("says it is still reading rather than claiming nothing is paired", async () => {
  let answer: (peers: RemotePeer[]) => void = () => {};
  listRemotePeers.mockReturnValue(new Promise<RemotePeer[]>((resolve) => {
    answer = resolve;
  }));
  const view = await mount();

  expect(view.container.querySelector("[data-testid='peers-loading']")?.textContent)
    .toBe("Reading your desktops…");
  expect(view.text()).not.toContain("No remote desktops yet.");

  await act(async () => answer([peer()]));
  await view.settle();

  expect(view.container.querySelector("[data-testid='peers-loading']")).toBeNull();
  expect(view.text()).toContain("Studio iMac");
  await view.unmount();
});

it("reports a failed connect, and reports a failed disconnect the same way", async () => {
  connectRemotePeer.mockRejectedValue(new Error("bridge_unreachable"));
  // An offline machine is the one that offers "Connect".
  listRemotePeers.mockResolvedValue([peer({ connected: false })]);
  const offline = await mount();
  await act(async () => offline.rowButton("Connect")!.click());
  await offline.settle();
  expect(offline.text()).toContain("bridge_unreachable");
  await offline.unmount();

  disconnectRemotePeer.mockRejectedValue(new Error("already_gone"));
  // A fresh list: the offline machine above is not the one this half is about.
  listRemotePeers.mockResolvedValue([peer()]);
  const online = await mount();
  await act(async () => online.rowButton("Disconnect")!.click());
  await online.settle();
  expect(online.text()).toContain("already_gone");
  await online.unmount();
});

it("reports a reconnect-all the shell could not carry out", async () => {
  // The pass itself is the shell's, so its failure arrives as a rejection from
  // `onReconnectAll` — this screen's job is to say so rather than to look like
  // nothing happened.
  // The control appears once there is more than one machine (below that the
  // count says nothing), so the list answers with two.
  listRemotePeers.mockResolvedValue([
    peer({ desktopId: "desktop_a" }),
    peer({ desktopId: "desktop_b", connected: false }),
  ]);
  const view = await mount();
  view.onReconnectAll.mockRejectedValue(new Error("no bridge"));
  await act(async () => view.button("Reconnect all")!.click());
  await view.settle();

  expect(view.text()).toContain("no bridge");
  await view.unmount();
});
