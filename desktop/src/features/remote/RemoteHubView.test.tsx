// @vitest-environment jsdom
import type { RemoteStatus } from "../remote/remoteClient";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { RemoteHubView } from "./RemoteHubView";

/**
 * The Remote page's own contract: that it puts both directions in one place,
 * that it says in words which is which, that each direction receives what it
 * needs, and that a machine's own pages (its settings, its tasks) replace this
 * page rather than opening a window of their own.
 *
 * The sections and the two sub-pages are stubbed: their internals are their own
 * concern. What belongs to *this* component is the page shell, the lead that
 * distinguishes local from remote, the wiring, and the navigation.
 */

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const sections = vi.hoisted(() => ({
  host: null as unknown,
  client: null as unknown,
}));

vi.mock("../remote/RemoteView", async () => {
  const { createElement } = await import("react");
  return {
    RemoteHostSection: (props: unknown) => {
      sections.host = props;
      return createElement("div", { "data-child": "host-section" });
    },
  };
});
vi.mock("../remote-peer/RemotePeersView", async () => {
  const { createElement } = await import("react");
  return {
    RemoteClientSection: (props: unknown) => {
      sections.client = props;
      return createElement("div", { "data-child": "client-section" });
    },
  };
});
vi.mock("../remote-peer/RemoteTasksView", async () => {
  const { createElement } = await import("react");
  return {
    RemoteTasksView: (props: { onBack: () => void; peer: { desktopId: string } }) =>
      createElement(
        "div",
        { "data-child": "tasks-view", "data-peer": props.peer.desktopId },
        createElement("button", { "data-back": "1", "onClick": props.onBack }),
      ),
  };
});
vi.mock("../remote-peer/RemotePeerSettings", async () => {
  const { createElement } = await import("react");
  return {
    RemotePeerSettings: (props: {
      onBack: () => void;
      onOpenTasks: () => void;
      onChanged: () => void;
      onUpdateLabel: (patch: { name?: string }) => void;
      peer: { desktopId: string };
    }) =>
      createElement(
        "div",
        { "data-child": "peer-settings", "data-peer": props.peer.desktopId },
        createElement("button", { "data-back": "1", "onClick": props.onBack }),
        createElement("button", { "data-tasks": "1", "onClick": props.onOpenTasks }),
        createElement("button", { "data-changed": "1", "onClick": props.onChanged }),
        createElement("button", {
          "data-rename": "1",
          "onClick": () => props.onUpdateLabel({ name: "Renamed" }),
        }),
      ),
  };
});

const setRemotePeerLabel = vi.fn<(desktopId: string, patch: Record<string, unknown>) => Promise<unknown>>();
vi.mock("../remote-peer/remotePeerClient", () => ({
  setRemotePeerLabel: (...args: Parameters<typeof setRemotePeerLabel>) => setRemotePeerLabel(...args),
}));

/** The page *this machine's* direction owns, stubbed the same way. */

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

const startWindowDrag = vi.fn();
vi.mock("../../lib/windowDrag", () => ({
  startWindowDrag: () => startWindowDrag(),
}));
vi.mock("../../lib/useIsFullscreen", () => ({ useIsFullscreen: () => false }));

function status(overrides: Partial<RemoteStatus> = {}): RemoteStatus {
  return {
    client: null,
    agentAvailable: true,
    desktopId: "desk_1",
    desktopPublicKey: "pk",
    natsUrl: "nats://example",
    pairId: "",
    pairingCode: null,
    pairingCodeExpiresAt: null,
    phase: "stopped",
    reason: null,
    recovery: null,
    warningCode: null,
    webLanUrl: null,
    webUrl: null,
    ...overrides,
  };
}

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

async function mount(overrides: Partial<Parameters<typeof RemoteHubView>[0]> = {}) {
  const props = {
    leftPanelExpanded: true,
    onRefreshRemote: vi.fn(async () => {}),
    onReconnectRemoteAll: vi.fn(async () => {}),
    onOpenRemoteSession: vi.fn(),
    onToggleLeftPanel: vi.fn(),
    remoteStatus: status(),
    ...overrides,
  };
  await act(async () => {
    root.render(createElement(RemoteHubView, props));
  });
  return props;
}

/** Open a machine's settings the way its row does. */
async function openSettings(desktopId = "desk_a") {
  await act(async () => (sections.client as { onOpenSettings: (peer: { desktopId: string }) => void })
    .onOpenSettings({ desktopId }));
}

beforeEach(() => {
  toasts.length = 0;
  setRemotePeerLabel.mockReset().mockResolvedValue(undefined);
  sections.host = null;
  sections.client = null;
  startWindowDrag.mockClear();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

/** The text of one labelled box, so an assertion cannot pass on the other's copy. */
function sectionText(testId: string): string {
  return container.querySelector(`[data-testid="${testId}"]`)?.textContent ?? "";
}

it("shows both directions on one page", async () => {
  await mount();

  expect(container.querySelector("[data-child='host-section']")).not.toBeNull();
  expect(container.querySelector("[data-child='client-section']")).not.toBeNull();
});

/**
 * The lead is the whole reason the two live together: the entries used to be two
 * rail labels a word apart, and nothing told the user which way each connected.
 */
it("says up front what is local and what is remote", async () => {
  await mount();

  const lead = container.querySelector("[data-testid='remote-hub-lead']")!.textContent ?? "";
  expect(lead).toContain("This computer's own conversations are in Chats");
  expect(lead).toContain("both directions");
});

it("names each direction, with its own explanation", async () => {
  await mount();

  const host = sectionText("remote-hub-host");
  expect(host).toContain("Let other devices connect here");
  // Each box carries the sentence that explains *that* direction, so a reader
  // who only scans headings still learns which side is which.
  expect(host).toContain("tools still run here");

  const client = sectionText("remote-hub-client");
  expect(client).toContain("Connect to another computer");
  expect(client).toContain("its tools run there");
});

/**
 * The headings, and only the headings.
 *
 * The explanatory sentences *should* mention phones — a phone is one of the
 * devices that can connect here, and leaving it out would be less clear, not
 * more neutral. What must not name a device is the label the user navigates by,
 * because either direction now works between two computers.
 */
it("names the directions by direction, not by the device on the other end", async () => {
  await mount();

  const headings = [...container.querySelectorAll("h2")].map(node => node.textContent ?? "");
  expect(headings).toEqual(["Let other devices connect here", "Connect to another computer"]);
  for (const heading of headings) {
    for (const device of ["Phone", "phone", "Mobile", "mobile"]) {
      expect(heading).not.toContain(device);
    }
  }
});

it("gives each direction what it needs, and nothing crossed over", async () => {
  const props = await mount({ remoteStatus: status({ pairId: "pair_1" }) });

  expect(sections.host).toMatchObject({
    remoteStatus: { pairId: "pair_1" },
  });
  // The host list opens nothing: a client's page repeated the row, and the
  // direction's own settings page is gone too (its switch is the preference).
  expect(sections.host).not.toHaveProperty("onOpenDevice");
  expect(sections.host).not.toHaveProperty("onOpenSettings");
  expect(sections.client).toMatchObject({ onReconnectAll: props.onReconnectRemoteAll });
  // The host half has no machines to reconnect, and the client half has no
  // pairing state: a prop on the wrong side would be invisible until it
  // misbehaved.
  expect(sections.host).not.toHaveProperty("onReconnectAll");
  expect(sections.client).not.toHaveProperty("remoteStatus");
});

it("carries the page shell: the title and the window drag", async () => {
  await mount();

  expect(container.querySelector("header")!.textContent).toContain("Remote");
  await act(async () => {
    container.querySelector("header")!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
  });
  expect(startWindowDrag).toHaveBeenCalledTimes(1);
});

/**
 * A machine's settings open as a page *of this window*.
 *
 * They belong to that machine, but the user is still working in this one:
 * leaving their window (its size, position and place) to change a setting is a
 * bigger errand than it deserves. The page replaces the hub's content, and its
 * own back control returns here.
 */
it("opens a machine's settings as a page in this window, and comes back", async () => {
  await mount();
  await openSettings();

  expect(container.querySelector("[data-child='peer-settings']")!.getAttribute("data-peer")).toBe("desk_a");
  // One page at a time: the hub's content is gone, not behind it.
  expect(container.querySelector("[data-testid='remote-hub-lead']")).toBeNull();
  expect(container.querySelector("[data-child='client-section']")).toBeNull();

  await act(async () => container.querySelector<HTMLButtonElement>("[data-back='1']")!.click());

  expect(container.querySelector("[data-child='peer-settings']")).toBeNull();
  expect(container.querySelector("[data-child='client-section']")).not.toBeNull();
});

/**
 * And that page opens the machine's tasks, which take the whole column — the
 * task editor is a list and a detail side by side, which does not fit inside a
 * settings column.
 */
it("hands the column to a machine's tasks, and gives it back to its settings", async () => {
  await mount();
  await openSettings();

  await act(async () => container.querySelector<HTMLButtonElement>("[data-tasks='1']")!.click());

  expect(container.querySelector("[data-child='tasks-view']")!.getAttribute("data-peer")).toBe("desk_a");
  expect(container.querySelector("[data-child='peer-settings']")).toBeNull();

  await act(async () => container.querySelector<HTMLButtonElement>("[data-back='1']")!.click());

  // Back to the settings page it came from, not to the hub: a task editor is one
  // level below the settings that offered it.
  expect(container.querySelector("[data-child='tasks-view']")).toBeNull();
  expect(container.querySelector("[data-child='peer-settings']")).not.toBeNull();
});

/**
 * A rename is *local*: the name lives in the credential book, so nothing else
 * would show it until the next catalogue read. What this test owns is that the
 * hub forwards the write and re-reads the machines.
 */
it("writes a local rename from the settings page", async () => {
  await mount();
  await openSettings();

  await act(async () => container.querySelector<HTMLButtonElement>("[data-rename='1']")!.click());

  expect(setRemotePeerLabel).toHaveBeenCalledWith("desk_a", { name: "Renamed" });
});

/** A rename the local store refuses is reported rather than swallowed. */
it("reports a rename that could not be stored", async () => {
  setRemotePeerLabel.mockRejectedValue(new Error("credential book is read-only"));
  await mount();
  await openSettings();

  await act(async () => container.querySelector<HTMLButtonElement>("[data-rename='1']")!.click());
  await act(async () => {
    for (let i = 0; i < 6; i += 1) await Promise.resolve();
  });

  expect(toasts.some(toast => toast.message.includes("credential book is read-only"))).toBe(true);
});

/**
 * A setting written on the page re-reads the machine list.
 *
 * The values live on the host, so nothing local would show what was just changed
 * until the list is read again — and the list is what the user returns to.
 */
it("re-reads the machines when the settings page reports a change", async () => {
  await mount();
  await openSettings();
  const before = (sections.client as { refreshNonce?: number }).refreshNonce;

  await act(async () => container.querySelector<HTMLButtonElement>("[data-changed='1']")!.click());
  await act(async () => container.querySelector<HTMLButtonElement>("[data-back='1']")!.click());

  expect((sections.client as { refreshNonce?: number }).refreshNonce).not.toBe(before);
});
