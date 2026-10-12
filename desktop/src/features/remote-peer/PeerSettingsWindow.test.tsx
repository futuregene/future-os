// @vitest-environment jsdom
import type { RemotePeer } from "./remotePeerClient";
import { emit } from "@tauri-apps/api/event";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { OPEN_REMOTE_SESSION_EVENT, PeerSettingsWindow } from "./PeerSettingsWindow";

/**
 * The settings window's own contract.
 *
 * Two things are easy to get wrong here and neither shows up as an error: which
 * machine the page is *about* (it is addressed by a query parameter, so a wrong
 * read edits another machine's settings), and what the page does when that
 * machine is gone (unpaired in the main window, or from this very page).
 */

const listRemotePeers = vi.fn<() => Promise<RemotePeer[]>>();
const setRemotePeerLabel = vi.fn<(desktopId: string, patch: Record<string, unknown>) => Promise<void>>();
const unpairRemotePeer = vi.fn<(desktopId: string) => Promise<string | null>>();
vi.mock("./remotePeerClient", () => ({
  listRemotePeers: () => listRemotePeers(),
  setRemotePeerLabel: (...args: Parameters<typeof setRemotePeerLabel>) => setRemotePeerLabel(...args),
  unpairRemotePeer: (...args: Parameters<typeof unpairRemotePeer>) => unpairRemotePeer(...args),
}));

const closeWindow = vi.fn(() => Promise.resolve());

vi.mock("@tauri-apps/api/event", () => ({
  emit: vi.fn(() => Promise.resolve()),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    close: () => closeWindow(),
    startDragging: () => Promise.resolve(),
  }),
}));

/**
 * The settings panel, reduced to the two things this window wires: label writes
 *  and the request to open that machine's tasks.
 */
vi.mock("./RemotePeerSettings", () => ({
  RemotePeerSettings: (props: {
    onBack: () => void;
    onOpenTasks: () => void;
    onUnpair: () => void;
    onUpdateLabel: (patch: { name?: string }) => void;
  }) => (
    <div data-testid="peer-settings">
      <button onClick={props.onUpdateLabel.bind(null, { name: "Renamed" })} type="button">rename</button>
      <button onClick={props.onOpenTasks} type="button">open-tasks</button>
      <button onClick={props.onUnpair} type="button">ask-unpair</button>
    </div>
  ),
}));

vi.mock("./RemoteTasksView", () => ({
  RemoteTasksView: (props: {
    onBack: () => void;
    onOpenSession: (desktopId: string, sessionId: string) => void;
    peer: RemotePeer;
  }) => (
    <div data-testid="tasks">
      <span>{props.peer.desktopId}</span>
      <button onClick={() => props.onOpenSession(props.peer.desktopId, "sess_9")} type="button">open-session</button>
      <button onClick={props.onBack} type="button">tasks-back</button>
    </div>
  ),
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

interface View {
  button: (label: string) => HTMLButtonElement | undefined;
  container: HTMLElement;
  dialogButton: (label: string) => HTMLButtonElement | undefined;
  refresh: () => Promise<void>;
  text: () => string;
  unmount: () => Promise<void>;
}

async function mount(desktopId = "desktop_a"): Promise<View> {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(<PeerSettingsWindow desktopId={desktopId} />);
  });
  const settle = async () => {
    await act(async () => {
      for (let i = 0; i < 6; i += 1) await Promise.resolve();
    });
  };
  await settle();
  return {
    button: label => [...container.querySelectorAll("button")].find(node => node.textContent === label),
    container,
    dialogButton: label => [...container.querySelectorAll("[role='dialog'] button")]
      .find(node => node.textContent === label) as HTMLButtonElement | undefined,
    refresh: settle,
    text: () => container.textContent ?? "",
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

beforeEach(() => {
  document.body.innerHTML = "";
  listRemotePeers.mockReset().mockResolvedValue([peer()]);
  setRemotePeerLabel.mockReset().mockResolvedValue(undefined);
  unpairRemotePeer.mockReset().mockResolvedValue(null);
  closeWindow.mockReset().mockResolvedValue(undefined);
  vi.mocked(emit).mockClear();
});

/**
 * The page is addressed by the query parameter the backend put in the URL, so it
 * has to read *that* machine — a window showing another machine's settings would
 * write every change to the wrong computer.
 */
it("shows the settings of the machine it was opened for", async () => {
  listRemotePeers.mockResolvedValue([
    peer({ desktopId: "desktop_a", name: "Studio iMac" }),
    peer({ desktopId: "desktop_b", name: "Mac mini" }),
  ]);
  const view = await mount("desktop_b");

  await act(async () => view.button("rename")!.click());
  await view.refresh();

  expect(setRemotePeerLabel).toHaveBeenCalledWith("desktop_b", { name: "Renamed" });
  await view.unmount();
});

/**
 * Polled, because the other side of this page changes without telling us: the
 * link can drop, and the main window can rename or unpair that machine while
 * this window is open.
 */
it("re-reads the machine while open", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  try {
    const view = await mount();
    const before = listRemotePeers.mock.calls.length;

    await act(async () => {
      vi.advanceTimersByTime(5_000);
    });
    await view.refresh();

    expect(listRemotePeers.mock.calls.length).toBeGreaterThan(before);
    await view.unmount();
  }
  finally {
    vi.useRealTimers();
  }
});

/**
 * Unpairing from another window, or a pairing revoked elsewhere: there is
 * nothing left for this page to show, and an empty settings page reads as a bug.
 */
it("says the machine is gone rather than showing empty settings", async () => {
  listRemotePeers.mockResolvedValue([]);
  const view = await mount();

  expect(view.text()).toContain("no longer paired");
  expect(view.container.querySelector("[data-testid='peer-settings']")).toBeNull();
  await view.unmount();
});

/** Unpairing removes the pairing, so it asks first — and acts only then. */
it("unpairs after the confirmation, and closes when it is done", async () => {
  const view = await mount();

  await act(async () => view.button("ask-unpair")!.click());
  expect(unpairRemotePeer).not.toHaveBeenCalled();

  listRemotePeers.mockResolvedValue([]);
  await act(async () => view.dialogButton("Unpair")!.click());
  await view.refresh();

  expect(unpairRemotePeer).toHaveBeenCalledWith("desktop_a");
  expect(closeWindow).toHaveBeenCalled();
  await view.unmount();
});

/**
 * A revoke the platform never received is the one case where the window stays
 * open: there is something to tell the user, and it would otherwise be lost with
 * the window that was about to close.
 */
it("keeps the window open to report a revoke that could not be delivered", async () => {
  unpairRemotePeer.mockResolvedValue("pending");
  const view = await mount();

  await act(async () => view.button("ask-unpair")!.click());
  listRemotePeers.mockResolvedValue([]);
  await act(async () => view.dialogButton("Unpair")!.click());
  await view.refresh();

  expect(view.text()).toContain("could not be reached to revoke");
  expect(closeWindow).not.toHaveBeenCalled();
  await view.unmount();
});

/**
 * A task run's conversation is shown by the main window, which is a *different*
 * window: this is the one hand-off that has to leave the page, and it uses a
 * backend event because the app's own bus is per-window.
 */
it("asks the main window to show a conversation, and goes back", async () => {
  const view = await mount();

  await act(async () => view.button("open-tasks")!.click());
  expect(view.container.querySelector("[data-testid='tasks']")).toBeTruthy();

  await act(async () => view.button("open-session")!.click());
  expect(emit).toHaveBeenCalledWith(OPEN_REMOTE_SESSION_EVENT, {
    desktopId: "desktop_a",
    sessionId: "sess_9",
  });

  await act(async () => view.button("tasks-back")!.click());
  expect(view.container.querySelector("[data-testid='peer-settings']")).toBeTruthy();
  await view.unmount();
});
